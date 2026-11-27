use super::{
    cpu_mem::top_n,
    pdh::{Query, Readings, Values},
};
use crate::model::{Disk, DiskDevice, Process};

pub fn register(query: &mut Query) {
    for (key, path) in [
        ("disk_read", r"\PhysicalDisk(*)\Disk Read Bytes/sec"),
        ("disk_write", r"\PhysicalDisk(*)\Disk Write Bytes/sec"),
        ("disk_read_latency", r"\PhysicalDisk(*)\Avg. Disk sec/Read"),
        (
            "disk_write_latency",
            r"\PhysicalDisk(*)\Avg. Disk sec/Write",
        ),
        ("disk_queue", r"\PhysicalDisk(*)\Current Disk Queue Length"),
    ] {
        query.add(key, path);
    }
}

pub fn collect(values: &Values, processes: &[Process], limit: usize) -> Disk {
    Disk {
        devices: values
            .get("disk_queue")
            .into_iter()
            .flatten()
            .filter(|(name, _)| name != "_Total")
            .map(|(name, queue)| DiskDevice {
                name: name.clone(),
                read_mb_sec: values.value("disk_read", name).map(|v| v / 1_048_576.0),
                write_mb_sec: values.value("disk_write", name).map(|v| v / 1_048_576.0),
                read_latency_ms: values.value("disk_read_latency", name).map(|v| v * 1000.0),
                write_latency_ms: values.value("disk_write_latency", name).map(|v| v * 1000.0),
                queue: Some(*queue),
            })
            .collect(),
        top: top_n(
            processes,
            |p| p.io_read_bytes_sec + p.io_write_bytes_sec,
            limit,
        ),
    }
}
