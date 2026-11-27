use super::{
    etw_frames::{property, property_u64},
    pdh::{Query, Readings, Values},
};
use crate::model::{BootProcess, Dpc, DriverSpike};
use std::{
    mem::size_of,
    sync::{Arc, Mutex},
};
use windows::{
    Win32::{
        Foundation::*,
        Security::*,
        System::{Diagnostics::Etw::*, Threading::*},
    },
    core::GUID,
};

const PERFINFO: GUID = GUID::from_u128(0xce1dbfb4_137e_4da6_87b0_3f59aa102cbc);
const IMAGE: GUID = GUID::from_u128(0x2cb15d1d_5fc1_11d2_abe1_00a0c911f518);
const PROCESS: GUID = GUID::from_u128(0x3d6fa8d0_fe05_11d0_9dda_00c04fd7ba7c);

#[derive(Clone)]
struct Module {
    base: u64,
    size: u64,
    name: String,
}
#[derive(Default)]
pub struct KernelData {
    pub spikes: Vec<DriverSpike>,
    pub starts: Vec<BootProcess>,
    modules: Vec<Module>,
    pub discarded: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn decodes_dpc_duration_and_resolves_driver_without_admin() {
        let state = Arc::new(Mutex::new(KernelData::default()));
        state.lock().unwrap().modules.push(Module {
            base: 0x1000,
            size: 0x1000,
            name: "test.sys".into(),
        });
        let mut payload = Vec::new();
        payload.extend_from_slice(&20_000i64.to_le_bytes());
        payload.extend_from_slice(&0x1234u64.to_le_bytes());
        let mut event = EVENT_RECORD::default();
        event.EventHeader.ProviderId = PERFINFO;
        event.EventHeader.EventDescriptor.Opcode = 68;
        event.EventHeader.TimeStamp = 50_000;
        event.UserData = payload.as_mut_ptr().cast();
        event.UserDataLength = payload.len() as u16;
        handle_event(&event, &state, 10_000_000, 0, 1000, 2.0);
        let mut data = state.lock().unwrap();
        assert_eq!(data.spikes.len(), 1);
        let spike = &data.spikes[0];
        assert_eq!(spike.duration_ms, 3.0);
        assert_eq!(spike.module.as_deref(), Some("test.sys"));
        assert_eq!(spike.ts_ms, 1005);
        data.spikes.clear();
        drop(data);
        event.UserDataLength = 4;
        handle_event(&event, &state, 10_000_000, 0, 1000, 2.0);
        assert!(state.lock().unwrap().spikes.is_empty());
    }
}
impl KernelData {
    pub fn new() -> Self {
        Self {
            spikes: Vec::new(),
            starts: Vec::new(),
            modules: modules(),
            discarded: 0,
        }
    }
    pub fn drain_spikes(&mut self) -> Vec<DriverSpike> {
        std::mem::take(&mut self.spikes)
    }
    pub fn drain_starts(&mut self) -> Vec<BootProcess> {
        std::mem::take(&mut self.starts)
    }
}

pub fn register(query: &mut Query) {
    for (key, path) in [
        ("dpc", r"\Processor(_Total)\% DPC Time"),
        ("interrupt", r"\Processor(_Total)\% Interrupt Time"),
        ("dpc_queued", r"\Processor(_Total)\DPCs Queued/sec"),
    ] {
        query.add(key, path);
    }
}
pub fn collect(values: &Values, status: &str, kernel: &Arc<Mutex<KernelData>>) -> Dpc {
    Dpc {
        time_pct: values.value("dpc", "_Total"),
        interrupt_pct: values.value("interrupt", "_Total"),
        queued_sec: values.value("dpc_queued", "_Total"),
        kernel_status: status.into(),
        spikes: kernel
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .drain_spikes(),
    }
}

pub fn handle_event(
    event: &EVENT_RECORD,
    kernel: &Arc<Mutex<KernelData>>,
    freq: i64,
    qpc_origin: i64,
    utc_origin: u64,
    threshold: f64,
) {
    let h = &event.EventHeader;
    let opcode = h.EventDescriptor.Opcode;
    let ts_ms = utc_origin
        .saturating_add(((h.TimeStamp - qpc_origin).max(0) as f64 * 1000.0 / freq as f64) as u64);
    if h.ProviderId == PERFINFO && matches!(opcode, 66..=69) {
        if event.UserData.is_null() || event.UserDataLength < 12 {
            return;
        }
        let bytes = unsafe {
            std::slice::from_raw_parts(event.UserData.cast::<u8>(), event.UserDataLength as usize)
        };
        let initial = i64::from_le_bytes(bytes[..8].try_into().expect("eight byte slice"));
        let duration = (h.TimeStamp - initial) as f64 * 1000.0 / freq as f64;
        if !(threshold..60_000.0).contains(&duration) {
            return;
        }
        let pointer32 = h.Flags & EVENT_HEADER_FLAG_32_BIT_HEADER as u16 != 0;
        let routine = if pointer32 {
            u32::from_le_bytes(bytes[8..12].try_into().expect("four byte slice")) as u64
        } else if bytes.len() >= 16 {
            u64::from_le_bytes(bytes[8..16].try_into().expect("eight byte slice"))
        } else {
            return;
        };
        let mut state = kernel.lock().unwrap_or_else(|p| p.into_inner());
        let module = state
            .modules
            .iter()
            .find(|m| routine >= m.base && routine < m.base.saturating_add(m.size))
            .map(|m| m.name.clone());
        if state.spikes.len() < 128 {
            state.spikes.push(DriverSpike {
                ts_ms,
                kind: if opcode == 67 { "ISR" } else { "DPC" }.into(),
                duration_ms: duration,
                routine: format!("0x{routine:x}"),
                module,
            });
        } else {
            state.discarded += 1;
        }
    } else if h.ProviderId == IMAGE && matches!(opcode, 1 | 3 | 4) {
        let Some(bytes) = property(event, "FileName") else {
            return;
        };
        let words: Vec<_> = bytes
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .take_while(|u| *u != 0)
            .collect();
        let name = String::from_utf16_lossy(&words)
            .rsplit('\\')
            .next()
            .unwrap_or("")
            .to_owned();
        if !name.to_ascii_lowercase().ends_with(".sys") {
            return;
        }
        if let Some((base, size)) =
            property_u64(event, "ImageBase").zip(property_u64(event, "ImageSize"))
        {
            let mut state = kernel.lock().unwrap_or_else(|p| p.into_inner());
            state.modules.retain(|m| m.base != base);
            if state.modules.len() < 2048 {
                state.modules.push(Module { base, size, name });
            }
        }
    } else if h.ProviderId == PROCESS && matches!(opcode, 1 | 3) {
        let pid = property_u64(event, "ProcessId").unwrap_or(h.ProcessId as u64) as u32;
        let Some(bytes) = property(event, "ImageFileName") else {
            return;
        };
        let name = String::from_utf8_lossy(&bytes)
            .trim_end_matches('\0')
            .rsplit('\\')
            .next()
            .unwrap_or("")
            .to_string();
        let mut state = kernel.lock().unwrap_or_else(|p| p.into_inner());
        if state.starts.len() < 2048 {
            state.starts.push(BootProcess {
                pid,
                name,
                created_ms: ts_ms,
                first_seen_ms: ts_ms,
                already_running: opcode == 3,
                ..Default::default()
            });
        } else {
            state.discarded += 1;
        }
    }
}

#[repr(C)]
struct NtModule {
    section: usize,
    mapped: usize,
    base: usize,
    size: u32,
    flags: u32,
    load_order: u16,
    init_order: u16,
    load_count: u16,
    name_offset: u16,
    path: [u8; 256],
}
#[link(name = "ntdll")]
unsafe extern "system" {
    fn NtQuerySystemInformation(
        class: u32,
        buffer: *mut std::ffi::c_void,
        length: u32,
        needed: *mut u32,
    ) -> i32;
}

struct DebugPrivilege {
    token: HANDLE,
    previous: TOKEN_PRIVILEGES,
}
impl DebugPrivilege {
    fn acquire() -> Option<Self> {
        // Only an already-held privilege can be enabled. Restore it after enumeration.
        unsafe {
            let mut token = HANDLE::default();
            if OpenProcessToken(
                GetCurrentProcess(),
                TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY,
                &mut token,
            )
            .is_err()
            {
                return None;
            }
            let mut guard = Self {
                token,
                previous: TOKEN_PRIVILEGES::default(),
            };
            let mut luid = LUID::default();
            if LookupPrivilegeValueW(None, SE_DEBUG_NAME, &mut luid).is_ok() {
                let privileges = TOKEN_PRIVILEGES {
                    PrivilegeCount: 1,
                    Privileges: [LUID_AND_ATTRIBUTES {
                        Luid: luid,
                        Attributes: SE_PRIVILEGE_ENABLED,
                    }],
                };
                let mut length = 0;
                let _ = AdjustTokenPrivileges(
                    token,
                    false,
                    Some(&privileges),
                    size_of::<TOKEN_PRIVILEGES>() as u32,
                    Some(&mut guard.previous),
                    Some(&mut length),
                );
            }
            Some(guard)
        }
    }
}
impl Drop for DebugPrivilege {
    fn drop(&mut self) {
        unsafe {
            if self.previous.PrivilegeCount > 0 {
                let _ =
                    AdjustTokenPrivileges(self.token, false, Some(&self.previous), 0, None, None);
            }
            let _ = CloseHandle(self.token);
        }
    }
}

fn modules() -> Vec<Module> {
    // Windows 11 24H2 restricts kernel addresses without SeDebugPrivilege.
    let _privilege = DebugPrivilege::acquire();
    let mut length = 0;
    unsafe {
        NtQuerySystemInformation(11, std::ptr::null_mut(), 0, &mut length);
    }
    if length == 0 || length > 4 * 1024 * 1024 {
        return Vec::new();
    }
    let mut buffer = vec![0u64; (length as usize + 8192).div_ceil(8)];
    if unsafe {
        NtQuerySystemInformation(
            11,
            buffer.as_mut_ptr().cast(),
            (buffer.len() * 8) as u32,
            &mut length,
        )
    } != 0
    {
        return Vec::new();
    }
    let count = buffer[0] as u32 as usize;
    let offset = size_of::<usize>();
    if count > 2048 || offset + count * size_of::<NtModule>() > length as usize {
        return Vec::new();
    }
    let records = unsafe {
        std::slice::from_raw_parts(
            buffer.as_ptr().cast::<u8>().add(offset).cast::<NtModule>(),
            count,
        )
    };
    records
        .iter()
        .filter(|m| m.base != 0)
        .map(|m| {
            let start = (m.name_offset as usize).min(m.path.len());
            let name = &m.path[start..];
            let end = name.iter().position(|b| *b == 0).unwrap_or(name.len());
            Module {
                base: m.base as u64,
                size: m.size as u64,
                name: String::from_utf8_lossy(&name[..end]).into(),
            }
        })
        .collect()
}
