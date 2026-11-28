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

pub fn import(source: &Path, root: &Path, reserve: u64) -> Result<PathBuf> {
    let length = source.metadata()?.len();
    ensure!(
        length > 0 && length <= LIMIT,
        "MangoHud input must be 1 byte to 64 MiB"
    );
    let mut file = File::open(source)?;
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
        let mut encoder = lz4_flex::frame::FrameEncoder::new(output);
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
            BufReader::new(File::open(source)?),
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
