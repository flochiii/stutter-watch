use super::pdh::{Query, Readings, Values};
use crate::{config::Config, model::*};
use anyhow::Result;
use nvml_wrapper::{
    Nvml,
    bitmasks::InitFlags,
    enum_wrappers::device::{Clock, TemperatureSensor},
};
use std::{
    mem::size_of,
    path::PathBuf,
    time::{Duration, Instant},
};
use windows::{
    Win32::{
        Devices::DeviceAndDriverInstallation::*,
        Graphics::Dxgi::*,
        System::{Power::*, SystemInformation::GetSystemDirectoryW},
    },
    core::{GUID, PCWSTR},
};

const DISPLAY_CLASS: GUID = GUID::from_u128(0x4d36e968_e325_11ce_bfc1_08002be10318);

struct Pnp {
    info: SP_DEVINFO_DATA,
    hardware: String,
    pci: Option<String>,
}
struct DeviceSet {
    handle: HDEVINFO,
    devices: Vec<Pnp>,
}
impl DeviceSet {
    fn new() -> Result<Self> {
        // Enumeration and registry properties do not open a GPU device or create a D3D context.
        let handle = unsafe {
            SetupDiGetClassDevsW(Some(&DISPLAY_CLASS), PCWSTR::null(), None, DIGCF_PRESENT)
        }?;
        let mut set = Self {
            handle,
            devices: Vec::new(),
        };
        for index in 0..64 {
            let mut info = SP_DEVINFO_DATA {
                cbSize: size_of::<SP_DEVINFO_DATA>() as u32,
                ..Default::default()
            };
            if unsafe { SetupDiEnumDeviceInfo(handle, index, &mut info) }.is_err() {
                break;
            }
            let mut bytes = [0u8; 4096];
            let hardware = if unsafe {
                SetupDiGetDeviceRegistryPropertyW(
                    handle,
                    &info,
                    SPDRP_HARDWAREID,
                    None,
                    Some(&mut bytes),
                    None,
                )
            }
            .is_ok()
            {
                let units: Vec<_> = bytes
                    .chunks_exact(2)
                    .map(|b| u16::from_le_bytes([b[0], b[1]]))
                    .take_while(|v| *v != 0)
                    .collect();
                String::from_utf16_lossy(&units).to_ascii_lowercase()
            } else {
                String::new()
            };
            let property = |kind| {
                let mut bytes = [0u8; 4];
                unsafe {
                    SetupDiGetDeviceRegistryPropertyW(
                        handle,
                        &info,
                        kind,
                        None,
                        Some(&mut bytes),
                        None,
                    )
                }
                .ok()
                .map(|_| u32::from_le_bytes(bytes))
            };
            let pci =
                property(SPDRP_BUSNUMBER)
                    .zip(property(SPDRP_ADDRESS))
                    .map(|(bus, address)| {
                        format!(
                            "00000000:{bus:02X}:{:02X}.{:X}",
                            address >> 16,
                            address & 0xffff
                        )
                    });
            set.devices.push(Pnp {
                info,
                hardware,
                pci,
            });
        }
        Ok(set)
    }
    fn matching(&self, vendor: u32, device: u32) -> Option<&Pnp> {
        let needle = format!("ven_{vendor:04x}&dev_{device:04x}");
        let mut matches = self.devices.iter().filter(|p| p.hardware.contains(&needle));
        let first = matches.next()?;
        // Do not guess between two identical cards and accidentally attach to a sleeping one.
        if matches.next().is_some() {
            None
        } else {
            Some(first)
        }
    }
    fn power(&self, pnp: &Pnp) -> Option<u32> {
        let mut buffer = [0u8; size_of::<CM_POWER_DATA>()];
        unsafe {
            SetupDiGetDeviceRegistryPropertyW(
                self.handle,
                &pnp.info,
                SPDRP_DEVICE_POWER_DATA,
                None,
                Some(&mut buffer),
                None,
            )
        }
        .ok()?;
        // Registry property is a packed byte buffer, so use an unaligned read.
        let data = unsafe { buffer.as_ptr().cast::<CM_POWER_DATA>().read_unaligned() };
        (data.PD_Size as usize >= size_of::<CM_POWER_DATA>())
            .then_some(data.PD_MostRecentPowerState.0 as u32)
    }
}
impl Drop for DeviceSet {
    fn drop(&mut self) {
        unsafe {
            let _ = SetupDiDestroyDeviceInfoList(self.handle);
        }
    }
}

struct Identity {
    name: String,
    vendor: u32,
    device: u32,
    luid: String,
    state: String,
}
pub struct Collector {
    adapters: Vec<Identity>,
    pnp: Option<DeviceSet>,
    nvml: Option<Nvml>,
    retry_after: Instant,
    enabled: bool,
    tgp: f64,
    pub warnings: Vec<String>,
    status: String,
}

pub fn register(query: &mut Query) {
    for (key, path) in [
        ("gpu_engine", r"\GPU Engine(*)\Utilization Percentage"),
        ("gpu_dedicated", r"\GPU Process Memory(*)\Dedicated Usage"),
        ("gpu_shared", r"\GPU Process Memory(*)\Shared Usage"),
    ] {
        query.add(key, path);
    }
}

pub fn nvml_allowed(power: Option<u32>, busy: bool, pci_known: bool) -> bool {
    power == Some(1) && busy && pci_known
}

impl Collector {
    pub fn pause(&mut self) {
        self.nvml = None;
    }
    pub fn new(config: &Config) -> Self {
        let mut this = Self {
            adapters: Vec::new(),
            pnp: None,
            nvml: None,
            retry_after: Instant::now(),
            enabled: config.collection.gpu_details,
            tgp: config.nvidia_tgp_w,
            warnings: Vec::new(),
            status: "passive_guard".into(),
        };
        let inventory = (|| -> Result<()> {
            let factory: IDXGIFactory1 = unsafe { CreateDXGIFactory1() }?;
            for index in 0..32 {
                let Ok(adapter) = (unsafe { factory.EnumAdapters1(index) }) else {
                    break;
                };
                let desc = unsafe { adapter.GetDesc1() }?;
                if desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0 {
                    continue;
                }
                let length = desc
                    .Description
                    .iter()
                    .position(|u| *u == 0)
                    .unwrap_or(desc.Description.len());
                this.adapters.push(Identity {
                    name: String::from_utf16_lossy(&desc.Description[..length]),
                    vendor: desc.VendorId,
                    device: desc.DeviceId,
                    luid: format!(
                        "0x{:08x}_0x{:08x}",
                        desc.AdapterLuid.HighPart as u32, desc.AdapterLuid.LowPart
                    ),
                    state: "unknown".into(),
                });
            }
            this.pnp = Some(DeviceSet::new()?);
            Ok(())
        })();
        if let Err(e) = inventory {
            this.warnings.push(format!("GPU inventory degraded: {e:#}"));
        }
        this
    }

    pub fn collect(&mut self, values: &Values, processes: &[Process], ts_ms: u64) -> Gpu {
        let mut adapters = Vec::new();
        let mut transitions = Vec::new();
        let mut any_nvml = false;
        for identity in &mut self.adapters {
            let processes_gpu: Vec<_> = values
                .get("gpu_engine")
                .into_iter()
                .flatten()
                .filter(|(name, value)| {
                    name.to_ascii_lowercase().contains(&identity.luid) && *value > 0.01
                })
                .filter_map(|(name, value)| {
                    let pid = name.strip_prefix("pid_")?.split('_').next()?.parse().ok()?;
                    let engine = name
                        .split("engtype_")
                        .nth(1)
                        .unwrap_or("unknown")
                        .to_string();
                    let memory_name = format!("pid_{pid}_luid_{}_phys_0", identity.luid);
                    Some(GpuProcess {
                        pid,
                        name: processes
                            .iter()
                            .find(|p| p.pid == pid)
                            .map(|p| p.name.clone()),
                        engine,
                        usage_pct: (*value).min(100.0),
                        dedicated_bytes: values.value("gpu_dedicated", &memory_name),
                        shared_bytes: values.value("gpu_shared", &memory_name),
                    })
                })
                .collect();
            let busy = !processes_gpu.is_empty();
            let pnp = self
                .pnp
                .as_ref()
                .and_then(|set| set.matching(identity.vendor, identity.device));
            let power = self.pnp.as_ref().zip(pnp).and_then(|(set, p)| set.power(p));
            let state = match power {
                Some(1) if busy => "active",
                Some(1) => "idle_d0",
                Some(2..=4) if !busy => "sleeping",
                _ if busy => "active_unconfirmed",
                _ => "unknown",
            };
            if identity.vendor == 0x10de && state != identity.state {
                transitions.push(GpuTransition{ts_ms,adapter:identity.name.clone(),from:identity.state.clone(),to:state.into(),
                    process_candidates:processes_gpu.iter().map(|p|p.pid).collect(),attribution:"PDH activity candidates; not proof of wake initiator; PnP reports most recent D-state".into()});
            }
            identity.state = state.into();
            let mut metrics = None;
            if identity.vendor == 0x10de
                && self.enabled
                && Instant::now() >= self.retry_after
                && nvml_allowed(power, busy, pnp.and_then(|p| p.pci.as_ref()).is_some())
            {
                // Recheck immediately before *initialization* and attachment, never use NVML
                // itself as the power probe. NO_ATTACH prevents global eager attachment.
                let rechecked = self.pnp.as_ref().zip(pnp).and_then(|(set, p)| set.power(p));
                if nvml_allowed(rechecked, busy, true) {
                    any_nvml = true;
                    if self.nvml.is_none() && Instant::now() >= self.retry_after {
                        let mut system = [0u16; 512];
                        let length = unsafe { GetSystemDirectoryW(Some(&mut system)) } as usize;
                        let path = PathBuf::from(String::from_utf16_lossy(
                            &system[..length.min(system.len())],
                        ))
                        .join("nvml.dll");
                        match Nvml::builder()
                            .lib_path(path.as_os_str())
                            .flags(InitFlags::NO_GPUS | InitFlags::NO_ATTACH)
                            .init()
                        {
                            Ok(nvml) => self.nvml = Some(nvml),
                            Err(e) => {
                                self.status = format!("NVML unavailable: {e}");
                                self.retry_after = Instant::now() + Duration::from_secs(60);
                            }
                        }
                    }
                    if let Some(nvml) = &self.nvml {
                        match nvml.device_by_pci_bus_id(
                            pnp.and_then(|p| p.pci.as_deref()).expect("PCI guard"),
                        ) {
                            Ok(device) => {
                                let power_w = device.power_usage().ok().map(|v| v as f64 / 1000.0);
                                let memory = device.memory_info().ok();
                                let throttle = device.current_throttle_reasons().ok();
                                metrics = Some(NvmlMetrics {
                                    core_mhz: device.clock_info(Clock::Graphics).ok(),
                                    memory_mhz: device.clock_info(Clock::Memory).ok(),
                                    temperature_c: device.temperature(TemperatureSensor::Gpu).ok(),
                                    power_w,
                                    tgp_w: self.tgp,
                                    power_tgp_pct: power_w.map(|v| v / self.tgp * 100.0),
                                    vram_used_bytes: memory.as_ref().map(|m| m.used),
                                    vram_total_bytes: memory.map(|m| m.total),
                                    clocks_throttle_reasons: throttle.map(|t| t.bits()),
                                    throttle_reasons: throttle
                                        .map(|t| {
                                            t.iter_names().map(|(name, _)| name.into()).collect()
                                        })
                                        .unwrap_or_default(),
                                });
                                self.status = "guarded_active_only".into();
                            }
                            Err(e) => {
                                self.status = format!("NVML attach unavailable: {e}");
                                self.retry_after = Instant::now() + Duration::from_secs(60);
                            }
                        }
                    }
                }
            }
            adapters.push(Adapter {
                sensors: None,
                name: identity.name.clone(),
                vendor_id: identity.vendor,
                luid: identity.luid.clone(),
                usage_pct: values
                    .get("gpu_engine")
                    .map(|engines| engine_usage(engines, &identity.luid)),
                state: state.into(),
                most_recent_power_state: power,
                processes: processes_gpu,
                nvml: metrics,
            });
        }
        if !any_nvml {
            self.nvml = None;
            self.status = if self.enabled {
                "passive_guard: no confirmed D0 activity"
            } else {
                "disabled"
            }
            .into();
        }
        Gpu {
            adapters,
            transitions,
            nvml_status: self.status.clone(),
        }
    }
}

// Sum processes sharing a physical engine, then take the busiest engine.
fn engine_usage(values: &[(String, f64)], luid: &str) -> f64 {
    let mut totals = std::collections::BTreeMap::<&str, f64>::new();
    for (name, value) in values
        .iter()
        .filter(|(name, value)| name.to_ascii_lowercase().contains(luid) && value.is_finite())
    {
        if let Some(engine) = name
            .split("_luid_")
            .nth(1)
            .and_then(|s| s.split("_engtype_").next())
        {
            *totals.entry(engine).or_default() += value.max(0.0);
        }
    }
    totals
        .values()
        .copied()
        .max_by(f64::total_cmp)
        .unwrap_or(0.0)
        .min(100.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn adapter_usage_combines_processes_without_summing_independent_engines() {
        let mut counters = vec![
            ("pid_1_luid_abc_phys_0_eng_0_engtype_3D".into(), 30.0),
            ("pid_2_luid_abc_phys_0_eng_0_engtype_3D".into(), 20.0),
            ("pid_1_luid_abc_phys_0_eng_1_engtype_Copy".into(), 80.0),
            ("pid_3_luid_xyz_phys_0_eng_0_engtype_3D".into(), 99.0),
            ("pid_4_luid_abc_phys_0_eng_1_engtype_Copy".into(), f64::NAN),
        ];
        assert_eq!(engine_usage(&counters, "abc"), 80.0);
        counters.push(("pid_5_luid_abc_phys_0_eng_0_engtype_3D".into(), 90.0));
        assert_eq!(engine_usage(&counters, "abc"), 100.0);
    }
    #[test]
    fn optimus_guard_denies_unknown_sleeping_or_idle() {
        for state in [None, Some(0), Some(2), Some(3), Some(4)] {
            assert!(!nvml_allowed(state, true, true));
        }
        assert!(!nvml_allowed(Some(1), false, true));
        assert!(!nvml_allowed(Some(1), true, false));
        assert!(nvml_allowed(Some(1), true, true));
    }
}
