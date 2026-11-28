//! Local-only MangoHud CSV conversion. Relative times never become system timestamps.
use crate::model::{Record, now_ms};
use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
};
const LIMIT: u64 = 64 * 1024 * 1024;

pub fn header(path: &Path) -> Result<(String, u64)> {
    let mut line = String::new();
    let input = File::open(path)?;
    let mut reader = BufReader::new(lz4_flex::frame::FrameDecoder::new(input).take(16384));
    reader.read_line(&mut line)?;
    match serde_json::from_str(&line)? {
        Record::ExternalLog {
            basename,
            imported_ms,
            ..
        } => Ok((basename, imported_ms)),
        _ => anyhow::bail!("Not an imported recording"),
    }
}

/// Bound viewer memory while retaining short frame-time peaks in long logs.
pub fn reduce(points: &mut Vec<crate::dashboard::Point>) {
    if points.len() <= 6002 {
        return;
    }
    let mut reduced = Vec::with_capacity(points.len() / 2 + 2);
    for chunk in points.chunks(4) {
        let min = chunk
            .iter()
            .min_by(|a, b| {
                a.values[3]
                    .unwrap_or_default()
                    .total_cmp(&b.values[3].unwrap_or_default())
            })
            .unwrap();
        let max = chunk
            .iter()
            .max_by(|a, b| {
                a.values[3]
                    .unwrap_or_default()
                    .total_cmp(&b.values[3].unwrap_or_default())
            })
            .unwrap();
        let (first, second) = if min.ts <= max.ts {
            (min, max)
        } else {
            (max, min)
        };
        reduced.push(first.clone());
        if first.ts != second.ts {
            reduced.push(second.clone());
        }
    }
    *points = reduced;
}

pub fn import(source: &Path, root: &Path, reserve: u64) -> Result<PathBuf> {
    let metadata = source.metadata()?;
    let length = metadata.len();
    ensure!(
        length > 0 && length <= LIMIT,
        "MangoHud input must be 1 byte to 64 MiB"
    );
    let mut file = File::open(source)?.take(LIMIT + 1);
    let mut digest = Sha256::new();
    let mut bytes = [0u8; 16384];
    loop {
        let count = file.read(&mut bytes)?;
        if count == 0 {
            break;
        }
        digest.update(&bytes[..count]);
    }
    let hash = format!("{:x}", digest.finalize());
    fs::create_dir_all(root.join("protected"))?;
    ensure!(
        crate::storage::free_bytes(root)? > reserve.saturating_add(length.saturating_mul(3)),
        "Insufficient disk space to import this recording"
    );
    let destination = root
        .join("protected")
        .join(format!("mangohud-{}.jsonl.lz4", &hash[..32]));
    if destination.exists() {
        return Ok(destination);
    }
    let temporary = destination.with_extension("tmp");
    let output = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)?;
    let result = (|| -> Result<()> {
        let mut encoder = lz4_flex::frame::FrameEncoder::with_frame_info(
            lz4_flex::frame::FrameInfo::new().content_checksum(true),
            output,
        );
        let header = Record::ExternalLog {
            source: "mangohud".into(),
            basename: source
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            sha256: hash,
            imported_ms: now_ms(),
            time_basis: "relative".into(),
        };
        serde_json::to_writer(&mut encoder, &header)?;
        encoder.write_all(b"\n")?;
        convert(
            BufReader::new(File::open(source)?.take(LIMIT + 1)),
            |elapsed_ns, fps, frametime_ms| {
                serde_json::to_writer(
                    &mut encoder,
                    &Record::ExternalFrame {
                        elapsed_ns,
                        fps,
                        frametime_ms,
                    },
                )?;
                encoder.write_all(b"\n")?;
                Ok(())
            },
        )?;
        let after = source.metadata()?;
        ensure!(
            after.len() == length && after.modified()? == metadata.modified()?,
            "MangoHud log changed during import; stop logging and try again"
        );
        let output = encoder.finish()?;
        output.sync_all()?;
        Ok(())
    })();
    if let Err(error) = result {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    fs::rename(&temporary, &destination)?;
    Ok(destination)
}

pub fn convert(
    mut reader: impl BufRead,
    mut visit: impl FnMut(u64, f64, f64) -> Result<()>,
) -> Result<usize> {
    let mut line = String::new();
    let mut header = None;
    for _ in 0..128 {
        line.clear();
        ensure!(
            reader.read_line(&mut line)? > 0,
            "MangoHud frame header is missing"
        );
        ensure!(line.len() <= 65536, "MangoHud metadata line is too long");
        let columns: Vec<_> = line.trim().split(',').map(str::trim).collect();
        if columns.contains(&"frametime")
            && columns.contains(&"fps")
            && columns.contains(&"elapsed")
        {
            header = Some((
                columns.iter().position(|v| *v == "elapsed").unwrap(),
                columns.iter().position(|v| *v == "fps").unwrap(),
                columns.iter().position(|v| *v == "frametime").unwrap(),
            ));
            break;
        }
    }
    let (time, fps, frame) = header.context("Unsupported MangoHud CSV header")?;
    let mut csv = csv::ReaderBuilder::new()
        .has_headers(false)
        .trim(csv::Trim::All)
        .from_reader(reader);
    let mut count = 0;
    let mut previous = None;
    for row in csv.records() {
        let row = row?;
        ensure!(row.as_slice().len() <= 65536, "MangoHud record is too long");
        let elapsed: u64 = row.get(time).context("Missing elapsed field")?.parse()?;
        let fps: f64 = row.get(fps).context("Missing FPS field")?.parse()?;
        let frame: f64 = row.get(frame).context("Missing frametime field")?.parse()?;
        ensure!(
            fps.is_finite()
                && frame.is_finite()
                && (0.0..=1_000_000.0).contains(&fps)
                && (0.0..=3_600_000.0).contains(&frame),
            "Invalid MangoHud measurement"
        );
        ensure!(
            previous.is_none_or(|p| elapsed >= p),
            "MangoHud elapsed time moved backwards"
        );
        previous = Some(elapsed);
        visit(elapsed, fps, frame)?;
        count += 1;
    }
    ensure!(count > 0, "MangoHud recording is empty");
    Ok(count)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn file_import_is_lossless_deduplicated_and_does_not_modify_input() {
        let temp = tempfile::tempdir().unwrap();
        let input = temp.path().join("frames.csv");
        let data = b"fps,frametime,elapsed\n60,16.67,0\n5,200,1000000000\n";
        fs::write(&input, data).unwrap();
        let root = temp.path().join("logs");
        let output = import(&input, &root, 0).unwrap();
        assert_eq!(header(&output).unwrap().0, "frames.csv");
        assert_eq!(import(&input, &root, 0).unwrap(), output);
        assert!(
            crate::storage::managed_files(&root)
                .unwrap()
                .iter()
                .any(|(p, _, _)| *p == output)
        );
        assert_eq!(fs::read(&input).unwrap(), data);
        let mut frames = Vec::new();
        crate::report::read_records(&output, |r| {
            if let Record::ExternalFrame {
                elapsed_ns,
                fps,
                frametime_ms,
            } = r
            {
                frames.push((elapsed_ns, fps, frametime_ms));
            }
        })
        .unwrap();
        assert_eq!(frames, vec![(0, 60.0, 16.67), (1_000_000_000, 5.0, 200.0)]);
        fs::write(&input, "bad header\n").unwrap();
        assert!(import(&input, &root, 0).is_err());
        assert_eq!(fs::read_dir(root.join("protected")).unwrap().count(), 1);
    }
    #[test]
    fn imports_named_columns_and_keeps_relative_nanoseconds() {
        let data =
            b"os,cpu\nLinux,CPU\nfps,frametime,elapsed\n60,16.67,1000000000\n5,200,2000000000\n";
        let mut rows = Vec::new();
        assert_eq!(
            convert(&data[..], |t, f, m| {
                rows.push((t, f, m));
                Ok(())
            })
            .unwrap(),
            2
        );
        assert_eq!(rows[1], (2000000000, 5.0, 200.0));
    }
    #[test]
    fn rejects_missing_header_nonfinite_and_reversed_time() {
        for text in [
            "fps,other\n1,2\n",
            "fps,frametime,elapsed\nNaN,2,1\n",
            "fps,frametime,elapsed\n1,2,4\n1,2,3\n",
        ] {
            assert!(convert(text.as_bytes(), |_, _, _| Ok(())).is_err());
        }
    }
}
