use crate::model::Foreground;
use crate::model::{WindowDelay, now_ms};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};
use windows::Win32::System::{StationsAndDesktops::*, Threading::*};
use windows::Win32::{Foundation::*, Graphics::Gdi::*, UI::WindowsAndMessaging::*};
use windows::core::PWSTR;

pub fn interactive() -> bool {
    // The secure lock screen is inaccessible to this process; never probe its UI.
    unsafe {
        if let Ok(desktop) = OpenInputDesktop(DESKTOP_CONTROL_FLAGS(0), false, DESKTOP_READOBJECTS)
        {
            let _ = CloseDesktop(desktop);
            true
        } else {
            false
        }
    }
}

pub struct Collector {
    last_hwnd: u64,
    hung_since: Option<Instant>,
    probe: Option<Probe>,
}
#[derive(Default)]
struct ProbeState {
    hwnd: u64,
    responsive: Option<bool>,
    current_ms: u64,
    peak_ms: u64,
    events: Vec<WindowDelay>,
}
struct Probe {
    state: Arc<Mutex<ProbeState>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

fn response_result(valid: bool, sent: LRESULT, error: WIN32_ERROR) -> Option<bool> {
    if !valid {
        None
    } else if sent.0 != 0 {
        Some(true)
    } else if error == ERROR_TIMEOUT {
        Some(false)
    } else {
        None
    }
}
impl Probe {
    fn new() -> Self {
        let state = Arc::new(Mutex::new(ProbeState::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let shared = Arc::clone(&state);
        let done = Arc::clone(&stop);
        let thread = std::thread::Builder::new()
            .name("window-response".into())
            .stack_size(128 * 1024)
            .spawn(move || {
                let mut last = 0u64;
                let mut since = None;
                while !done.load(Ordering::Relaxed) {
                    let tick = Instant::now();
                    unsafe {
                        let hwnd = if interactive() {
                            GetForegroundWindow()
                        } else {
                            HWND::default()
                        };
                        let raw = hwnd.0 as u64;
                        let changed = last != raw;
                        if last != raw {
                            since = None;
                            last = raw;
                        }
                        let mut result = 0;
                        SetLastError(ERROR_SUCCESS);
                        let sent = if !hwnd.is_invalid() {
                            SendMessageTimeoutW(
                                hwnd,
                                WM_NULL,
                                WPARAM(0),
                                LPARAM(0),
                                SMTO_ABORTIFHUNG | SMTO_BLOCK,
                                20,
                                Some(&mut result),
                            )
                        } else {
                            LRESULT(0)
                        };
                        let response = response_result(!hwnd.is_invalid(), sent, GetLastError());
                        let mut s = shared.lock().unwrap_or_else(|p| p.into_inner());
                        if changed {
                            s.peak_ms = 0;
                            s.current_ms = 0;
                        }
                        s.hwnd = raw;
                        s.responsive = response;
                        if response == Some(false) {
                            let start = since.get_or_insert(tick);
                            s.current_ms = start.elapsed().as_millis() as u64;
                            s.peak_ms = s.peak_ms.max(s.current_ms);
                        } else {
                            if let Some(start) = since.take().filter(|_| response == Some(true)) {
                                let duration = start.elapsed().as_millis() as u64;
                                if duration >= 50 {
                                    let mut pid = 0;
                                    GetWindowThreadProcessId(hwnd, Some(&mut pid));
                                    if s.events.len() < 128 {
                                        s.events.push(WindowDelay {
                                            ts_ms: now_ms(),
                                            pid,
                                            duration_ms: duration,
                                            probe_interval_ms: 50,
                                        });
                                    }
                                    s.peak_ms = s.peak_ms.max(duration);
                                }
                            }
                            s.current_ms = 0;
                        }
                    }
                    std::thread::sleep(Duration::from_millis(50).saturating_sub(tick.elapsed()));
                }
            })
            .ok();
        Self {
            state,
            stop,
            thread,
        }
    }
}
impl Drop for Probe {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
impl Collector {
    pub fn new(response: bool) -> Self {
        Self {
            last_hwnd: 0,
            hung_since: None,
            probe: response.then(Probe::new),
        }
    }
    pub fn collect(&mut self, response: bool, titles: bool) -> Foreground {
        // SAFETY: HWND is borrowed; every API handles a window disappearing between calls.
        unsafe {
            let hwnd = GetForegroundWindow();
            if hwnd.is_invalid() {
                self.hung_since = None;
                return Foreground::default();
            }
            let raw = hwnd.0 as u64;
            if self.last_hwnd != raw {
                self.hung_since = None;
                self.last_hwnd = raw;
            }
            let mut pid = 0;
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
            let mut title = [0u16; 512];
            let length = if titles {
                GetWindowTextW(hwnd, &mut title).max(0) as usize
            } else {
                0
            };
            let mut program = String::new();
            if let Ok(process) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
                let mut path = [0u16; 1024];
                let mut length = path.len() as u32;
                if QueryFullProcessImageNameW(
                    process,
                    PROCESS_NAME_FORMAT(0),
                    PWSTR(path.as_mut_ptr()),
                    &mut length,
                )
                .is_ok()
                {
                    // The API returns a path; retain only the executable basename.
                    program = String::from_utf16_lossy(&path[..length as usize])
                        .rsplit('\\')
                        .next()
                        .unwrap_or("")
                        .into();
                }
                let _ = CloseHandle(process);
            }
            let mut result = 0;
            SetLastError(ERROR_SUCCESS);
            let sent = if response && self.probe.is_none() {
                SendMessageTimeoutW(
                    hwnd,
                    WM_NULL,
                    WPARAM(0),
                    LPARAM(0),
                    SMTO_ABORTIFHUNG | SMTO_BLOCK,
                    20,
                    Some(&mut result),
                )
            } else {
                LRESULT(0)
            };
            let mut responsive = if response && self.probe.is_none() {
                response_result(true, sent, GetLastError())
            } else {
                None
            };
            let mut response_events = Vec::new();
            let mut peak = None;
            let mut current = None;
            if let Some(probe) = &self.probe {
                let mut state = probe.state.lock().unwrap_or_else(|p| p.into_inner());
                if state.hwnd == raw {
                    responsive = state.responsive;
                    peak =
                        (state.peak_ms > 0 || state.responsive.is_some()).then_some(state.peak_ms);
                    current = state.responsive.map(|_| state.current_ms);
                }
                response_events = std::mem::take(&mut state.events);
                state.peak_ms = state.current_ms;
            }
            if responsive == Some(false) {
                self.hung_since.get_or_insert_with(Instant::now);
            } else {
                self.hung_since = None;
            }
            let mut rect = RECT::default();
            let mut monitor = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            let fullscreen = GetWindowRect(hwnd, &mut rect).is_ok()
                && GetMonitorInfoW(
                    MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST),
                    &mut monitor,
                )
                .as_bool()
                && rect.left <= monitor.rcMonitor.left
                && rect.top <= monitor.rcMonitor.top
                && rect.right >= monitor.rcMonitor.right
                && rect.bottom >= monitor.rcMonitor.bottom;
            Foreground {
                hwnd: raw,
                pid,
                title: String::from_utf16_lossy(&title[..length]),
                program,
                responsive,
                unresponsive_ms: current.unwrap_or_else(|| {
                    self.hung_since
                        .map(|t| t.elapsed().as_millis() as u64 + 20)
                        .unwrap_or(0)
                }),
                fullscreen,
                response_peak_ms: peak,
                response_events,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn generic_failures_are_unknown_instead_of_window_hangs() {
        assert_eq!(response_result(true, LRESULT(1), ERROR_SUCCESS), Some(true));
        assert_eq!(
            response_result(true, LRESULT(0), ERROR_TIMEOUT),
            Some(false)
        );
        assert_eq!(response_result(true, LRESULT(0), ERROR_SUCCESS), None);
        assert_eq!(response_result(true, LRESULT(0), ERROR_ACCESS_DENIED), None);
        assert_eq!(
            response_result(true, LRESULT(0), ERROR_INVALID_WINDOW_HANDLE),
            None
        );
        assert_eq!(response_result(false, LRESULT(0), ERROR_TIMEOUT), None);
    }
}
