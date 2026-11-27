//! Reproducible, synthetic, privacy-safe format probe; no production logs are read.
use std::{
    io::{Read, Write},
    time::Instant,
};
fn main() -> anyhow::Result<()> {
    let mut input = Vec::new();
    for i in 0..120u64 {
        let processes: Vec<_> = (0..8).map(|p| serde_json::json!({"pid":100+p,"created_ms":1000,"name":format!("app-{p}.exe"),"cpu_pct":((i*17+p*13)%1000) as f64/10.0,"working_set_bytes":100_000_000+(i+p)*4096,"io_read_bytes_sec":(i*71+p*11)%50000})).collect();
        let value = serde_json::json!({"type":"sample","ts_ms":1_000_000+i*500,"cpu":{"usage_pct":(i*37%1000) as f64/10.0,"top":processes},"memory":{"available_bytes":5_000_000_000-i*4096,"commit_bytes":11_000_000_000+i*8192},"frames":{"frame_time_ms":16.67,"max_frame_time_ms":if i==73 {210.13} else {17.2}},"disk":{"read_mb_sec":(i*31%100) as f64},"dpc":{"time_pct":0.12},"gpu":{"state":"sleeping"}});
        serde_json::to_writer(&mut input, &value)?;
        input.push(b'\n');
    }
    for codec in ["lz4-64k", "zstd-1-64k", "gzip-1"] {
        let mut sizes = Vec::new();
        let mut times = Vec::new();
        for _ in 0..21 {
            let start = Instant::now();
            let output = match codec {
                "lz4-64k" => {
                    let info = lz4_flex::frame::FrameInfo::new()
                        .block_size(lz4_flex::frame::BlockSize::Max64KB);
                    let mut w = lz4_flex::frame::FrameEncoder::with_frame_info(info, Vec::new());
                    w.write_all(&input)?;
                    w.finish()?
                }
                "zstd-1-64k" => {
                    let mut w = zstd::stream::Encoder::new(Vec::new(), 1)?;
                    w.window_log(16)?;
                    w.write_all(&input)?;
                    w.finish()?
                }
                _ => {
                    let mut w =
                        flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
                    w.write_all(&input)?;
                    w.finish()?
                }
            };
            times.push(start.elapsed().as_secs_f64() * 1000.0);
            sizes.push(output.len());
            let mut decoded = Vec::new();
            match codec {
                "lz4-64k" => {
                    lz4_flex::frame::FrameDecoder::new(output.as_slice())
                        .read_to_end(&mut decoded)?;
                }
                "zstd-1-64k" => {
                    zstd::stream::Decoder::new(output.as_slice())?.read_to_end(&mut decoded)?;
                }
                _ => {
                    flate2::read::GzDecoder::new(output.as_slice()).read_to_end(&mut decoded)?;
                }
            }
            assert_eq!(decoded, input);
        }
        times.remove(0);
        times.sort_by(f64::total_cmp);
        println!(
            "{}",
            serde_json::json!({"codec":codec,"input_bytes":input.len(),"output_bytes":sizes[0],"ratio":input.len() as f64/sizes[0] as f64,"median_encode_ms":times[times.len()/2],"repeats":20,"synthetic":true})
        );
    }
    Ok(())
}
