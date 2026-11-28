use crate::{
    capacity,
    collectors::pdh::wide,
    config::{Config, HistoryMode},
    i18n::{LANGUAGE_NAMES, LANGUAGES, Text},
    model::*,
    recorder::Status,
    theme::{self, Kind, Theme},
    tray::Command,
};
use anyhow::Result;
use std::{
    path::PathBuf,
    sync::{Arc, Mutex, mpsc::SyncSender},
};
use windows::Win32::UI::{Input::KeyboardAndMouse::EnableWindow, Shell::Common::COMDLG_FILTERSPEC};
use windows::{
    Win32::{
        Foundation::*,
        Graphics::Gdi::*,
        System::LibraryLoader::GetModuleHandleW,
        UI::{Controls::*, HiDpi::GetDpiForWindow, WindowsAndMessaging::*},
    },
    core::{PCWSTR, PWSTR, w},
};

unsafe fn send(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe { SendMessageW(hwnd, msg, Some(wp), Some(lp)) }
}
pub const SOURCE_KEYS: [&str; 14] = [
    "cpu_total",
    "cpu_cores",
    "memory",
    "disks",
    "process_cpu",
    "process_memory",
    "process_io",
    "window_response",
    "frames",
    "gpu",
    "gpu_details",
    "dpc",
    "dpc_events",
    "boot",
];
pub const TRAY_KEYS: [&str; 7] = [
    "tray_status",
    "cpu_total",
    "memory",
    "gpu",
    "disks",
    "dpc",
    "window_response",
];
pub const CONFIG_CHANGED: u32 = WM_APP + 10;
struct Surface {
    dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
    width: i32,
    height: i32,
}
impl Surface {
    unsafe fn new(target: HDC, width: i32, height: i32) -> Option<Self> {
        unsafe {
            let dc = CreateCompatibleDC(Some(target));
            if dc.is_invalid() {
                return None;
            }
            let bitmap = CreateCompatibleBitmap(target, width.max(1), height.max(1));
            if bitmap.is_invalid() {
                let _ = DeleteDC(dc);
                return None;
            }
            let previous = SelectObject(dc, HGDIOBJ(bitmap.0));
            Some(Self {
                dc,
                bitmap,
                previous,
                width,
                height,
            })
        }
    }
}
impl Drop for Surface {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.previous);
            let _ = DeleteObject(HGDIOBJ(self.bitmap.0));
            let _ = DeleteDC(self.dc);
        }
    }
}
pub use crate::dashboard::{Dashboard, Point, envelope};

struct State {
    shared: Arc<Mutex<Dashboard>>,
    sender: SyncSender<Command>,
    text: Text,
    page: usize,
    font: HFONT,
    controls: Vec<(HWND, usize)>,
    checks: Vec<HWND>,
    tray_checks: Vec<HWND>,
    tray_preview: HWND,
    mode: HWND,
    budget: HWND,
    retention: HWND,
    language: HWND,
    footer: HWND,
    list: HWND,
    paths: Vec<PathBuf>,
    rows: Vec<String>,
    incident_rows: std::collections::HashMap<PathBuf, (std::time::SystemTime, u64, String)>,
    tooltip: HWND,
    help_texts: Vec<Vec<u16>>,
    incident: Vec<Point>,
    selected_header: Option<Incident>,
    mango_view: bool,
    importing: Option<std::sync::mpsc::Receiver<Result<Option<PathBuf>>>>,
    incident_message: Option<&'static str>,
    dpi: i32,
    last_incident_refresh: std::time::Instant,
    shown: [bool; 7],
    surface: Option<Surface>,
    theme: Theme,
    details_window: HWND,
    details_edit: HWND,
}
impl Drop for State {
    fn drop(&mut self) {
        unsafe {
            let _ = DeleteObject(HGDIOBJ(self.font.0));
        }
        let mut d = self.shared.lock().unwrap_or_else(|p| p.into_inner());
        d.visible = false;
    }
}

pub fn open(shared: Arc<Mutex<Dashboard>>, sender: SyncSender<Command>) -> Result<HWND> {
    unsafe {
        let instance = HINSTANCE(GetModuleHandleW(None)?.0);
        let class = w!("stutter-watch-panel");
        let _ = RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            lpszClassName: class,
            hIcon: LoadIconW(Some(instance), PCWSTR(std::ptr::without_provenance(1)))
                .unwrap_or_default(),
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            ..Default::default()
        });
        let config = shared
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .config
            .clone();
        let state = Box::new(State {
            shared: Arc::clone(&shared),
            sender,
            text: Text::new(&config.language),
            page: 0,
            font: HFONT::default(),
            controls: Vec::new(),
            checks: Vec::new(),
            tray_checks: Vec::new(),
            tray_preview: HWND::default(),
            mode: HWND::default(),
            budget: HWND::default(),
            retention: HWND::default(),
            language: HWND::default(),
            footer: HWND::default(),
            list: HWND::default(),
            paths: Vec::new(),
            rows: Vec::new(),
            incident_rows: std::collections::HashMap::new(),
            tooltip: HWND::default(),
            help_texts: Vec::new(),
            incident: Vec::new(),
            selected_header: None,
            mango_view: false,
            importing: None,
            incident_message: None,
            dpi: 96,
            last_incident_refresh: std::time::Instant::now(),
            shown: [true; 7],
            surface: None,
            theme: Theme::new(),
            details_window: HWND::default(),
            details_edit: HWND::default(),
        });
        let raw = Box::into_raw(state);
        let result = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            class,
            PCWSTR(wide(crate::branding::NAME).as_ptr()),
            WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            1120,
            800,
            None,
            None,
            Some(instance),
            Some(raw.cast()),
        );
        let hwnd = match result {
            Ok(hwnd) => hwnd,
            Err(e) => {
                drop(Box::from_raw(raw));
                return Err(e.into());
            }
        };
        (*raw).theme.titlebar(hwnd);
        theme::window_icon(hwnd);
        (*raw).dpi = GetDpiForWindow(hwnd) as i32;
        let mut monitor = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if GetMonitorInfoW(
            MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST),
            &mut monitor,
        )
        .as_bool()
        {
            let k = (*raw).dpi as f64 / 96.0;
            let width = (1120.0 * k) as i32;
            let height = (800.0 * k) as i32;
            let _ = SetWindowPos(
                hwnd,
                None,
                monitor.rcWork.left
                    + (monitor.rcWork.right
                        - monitor.rcWork.left
                        - width.min(monitor.rcWork.right - monitor.rcWork.left))
                        / 2,
                monitor.rcWork.top
                    + (monitor.rcWork.bottom
                        - monitor.rcWork.top
                        - height.min(monitor.rcWork.bottom - monitor.rcWork.top))
                        / 2,
                width.min(monitor.rcWork.right - monitor.rcWork.left),
                height.min(monitor.rcWork.bottom - monitor.rcWork.top),
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
        (*raw).font = CreateFontW(
            -14 * (*raw).dpi / 96,
            0,
            0,
            0,
            400,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            CLEARTYPE_QUALITY,
            DEFAULT_PITCH.0 as u32,
            w!("Segoe UI"),
        );
        setup(hwnd, &mut *raw);
        layout(hwnd, &mut *raw);
        refresh(&mut *raw);
        (*raw)
            .shared
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .visible = true;
        let _ = SetTimer(Some(hwnd), 1, 500, None);
        let _ = ShowWindow(hwnd, SW_SHOW);
        // A launcher may set STARTF_USESHOWWINDOW/SW_HIDE for the console.
        // Explicitly show this window independently of that first-call override.
        let _ = SetWindowPos(
            hwnd,
            None,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_SHOWWINDOW,
        );
        let _ = SetForegroundWindow(hwnd);
        Ok(hwnd)
    }
}
unsafe fn control(
    hwnd: HWND,
    s: &mut State,
    class: PCWSTR,
    label: &str,
    id: usize,
    style: WINDOW_STYLE,
    page: usize,
) -> HWND {
    unsafe {
        let text = wide(label);
        let c = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            class,
            PCWSTR(text.as_ptr()),
            WS_CHILD | WS_VISIBLE | style,
            0,
            0,
            0,
            0,
            Some(hwnd),
            Some(HMENU(id as *mut _)),
            None,
            None,
        )
        .unwrap_or_default();
        let _ = send(c, WM_SETFONT, WPARAM(s.font.0 as usize), LPARAM(1));
        if class.to_string().is_ok_and(|name| name == "BUTTON") {
            s.theme.attach(
                c,
                if style.0 & 15 == BS_AUTOCHECKBOX as u32 {
                    Kind::Checkbox
                } else {
                    Kind::Button
                },
                s.font,
            );
        } else if class.to_string().is_ok_and(|name| name == "COMBOBOX") {
            s.theme.attach(c, Kind::Combo, s.font);
        }
        s.controls.push((c, page));
        c
    }
}
unsafe fn setup(hwnd: HWND, s: &mut State) {
    unsafe {
        let language = s
            .shared
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .config
            .language
            .clone();
        let labels = Text::new(&language);
        for (i, key) in ["live", "data", "incidents", "tray_settings"]
            .iter()
            .enumerate()
        {
            control(
                hwnd,
                s,
                w!("BUTTON"),
                labels.get(key),
                10 + i,
                WS_TABSTOP,
                usize::MAX,
            );
        }
        control(
            hwnd,
            s,
            w!("BUTTON"),
            labels.get("mark"),
            20,
            WS_TABSTOP,
            usize::MAX,
        );
        control(
            hwnd,
            s,
            w!("BUTTON"),
            labels.get("pause"),
            21,
            WS_TABSTOP,
            usize::MAX,
        );
        s.footer = control(hwnd, s, w!("STATIC"), "", 31, WINDOW_STYLE(0), usize::MAX);
        s.tooltip = CreateWindowExW(
            WS_EX_TOPMOST,
            TOOLTIPS_CLASSW,
            None,
            WINDOW_STYLE(TTS_ALWAYSTIP | TTS_NOPREFIX),
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            Some(hwnd),
            None,
            None,
            None,
        )
        .unwrap_or_default();
        send(
            s.tooltip,
            TTM_SETMAXTIPWIDTH,
            WPARAM(0),
            LPARAM(360 * s.dpi as isize / 96),
        );
        send(
            s.tooltip,
            TTM_SETDELAYTIME,
            WPARAM(TTDT_INITIAL as usize),
            LPARAM(350),
        );
        for (i, label) in ["CPU", "RAM", "I/O", "Present API", "UI", "DPC", "ISR"]
            .iter()
            .enumerate()
        {
            let c = control(
                hwnd,
                s,
                w!("BUTTON"),
                label,
                400 + i,
                WS_TABSTOP | WINDOW_STYLE(BS_AUTOCHECKBOX as u32),
                4,
            );
            send(c, BM_SETCHECK, WPARAM(1), LPARAM(0));
        }
        let config = s
            .shared
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .config
            .clone();
        control(
            hwnd,
            s,
            w!("STATIC"),
            labels.get("collect_hint"),
            40,
            WINDOW_STYLE(0),
            1,
        );
        for (i, key) in SOURCE_KEYS.iter().enumerate() {
            let c = control(
                hwnd,
                s,
                w!("BUTTON"),
                labels.get(key),
                100 + i,
                WINDOW_STYLE(BS_AUTOCHECKBOX as u32) | WS_TABSTOP,
                1,
            );
            let _ = send(
                c,
                BM_SETCHECK,
                WPARAM(usize::from(config.collection.flags()[i])),
                LPARAM(0),
            );
            s.checks.push(c);
            let help = control(
                hwnd,
                s,
                w!("STATIC"),
                "ⓘ",
                500 + i,
                WINDOW_STYLE(windows::Win32::System::SystemServices::SS_NOTIFY.0),
                1,
            );
            s.help_texts.push(wide(labels.get(&format!("help_{key}"))));
            let info = TTTOOLINFOW {
                cbSize: std::mem::size_of::<TTTOOLINFOW>() as u32,
                uFlags: TTF_IDISHWND | TTF_SUBCLASS,
                hwnd,
                uId: help.0 as usize,
                lpszText: PWSTR(s.help_texts[i].as_mut_ptr()),
                ..Default::default()
            };
            send(
                s.tooltip,
                TTM_ADDTOOLW,
                WPARAM(0),
                LPARAM((&info as *const TTTOOLINFOW) as isize),
            );
        }
        control(
            hwnd,
            s,
            w!("STATIC"),
            labels.get("history_mode"),
            46,
            WINDOW_STYLE(0),
            1,
        );
        s.mode = control(
            hwnd,
            s,
            w!("COMBOBOX"),
            "",
            200,
            WINDOW_STYLE((CBS_DROPDOWNLIST | CBS_OWNERDRAWFIXED | CBS_HASSTRINGS) as u32)
                | WS_TABSTOP,
            1,
        );
        for key in ["lossless", "summary"] {
            let text = wide(s.text.get(key));
            send(
                s.mode,
                CB_ADDSTRING,
                WPARAM(0),
                LPARAM(text.as_ptr() as isize),
            );
        }
        send(
            s.mode,
            CB_SETCURSEL,
            WPARAM(usize::from(config.history.mode == HistoryMode::Summary)),
            LPARAM(0),
        );
        control(
            hwnd,
            s,
            w!("STATIC"),
            labels.get("budget"),
            42,
            WINDOW_STYLE(0),
            1,
        );
        s.budget = control(
            hwnd,
            s,
            w!("EDIT"),
            &budget_text(config.history.ordinary_budget_bytes),
            201,
            WS_BORDER | WS_TABSTOP,
            1,
        );
        control(
            hwnd,
            s,
            w!("STATIC"),
            labels.get("retention"),
            43,
            WINDOW_STYLE(0),
            1,
        );
        s.retention = control(
            hwnd,
            s,
            w!("EDIT"),
            &config.history.retention_hours.to_string(),
            202,
            WS_BORDER | WS_TABSTOP | WINDOW_STYLE(ES_NUMBER as u32),
            1,
        );
        control(
            hwnd,
            s,
            w!("STATIC"),
            labels.get("language"),
            47,
            WINDOW_STYLE(0),
            1,
        );
        s.language = control(
            hwnd,
            s,
            w!("COMBOBOX"),
            "",
            203,
            WINDOW_STYLE(
                (CBS_DROPDOWNLIST | CBS_OWNERDRAWFIXED | CBS_HASSTRINGS | CBS_NOINTEGRALHEIGHT)
                    as u32,
            ) | WS_TABSTOP
                | WS_VSCROLL,
            1,
        );
        for name in LANGUAGE_NAMES {
            let text = wide(name);
            send(
                s.language,
                CB_ADDSTRING,
                WPARAM(0),
                LPARAM(text.as_ptr() as isize),
            );
        }
        send(
            s.language,
            CB_SETCURSEL,
            WPARAM(
                LANGUAGES
                    .iter()
                    .position(|&l| l == config.language)
                    .unwrap_or(0),
            ),
            LPARAM(0),
        );
        control(
            hwnd,
            s,
            w!("BUTTON"),
            labels.get("apply"),
            204,
            WS_TABSTOP | WINDOW_STYLE(BS_DEFPUSHBUTTON as u32),
            1,
        );
        control(
            hwnd,
            s,
            w!("BUTTON"),
            labels.get("details"),
            205,
            WS_TABSTOP,
            1,
        );
        control(
            hwnd,
            s,
            w!("STATIC"),
            labels.get("incident_hint"),
            45,
            WINDOW_STYLE(0),
            2,
        );
        control(
            hwnd,
            s,
            w!("BUTTON"),
            labels.get("import_mangohud"),
            301,
            WS_TABSTOP,
            2,
        );
        s.list = control(
            hwnd,
            s,
            w!("LISTBOX"),
            "",
            300,
            WS_BORDER
                | WS_VSCROLL
                | WS_TABSTOP
                | WINDOW_STYLE((LBS_NOTIFY | LBS_NOINTEGRALHEIGHT) as u32),
            2,
        );
        send(
            s.mode,
            CB_SETITEMHEIGHT,
            WPARAM(usize::MAX),
            LPARAM((26 * s.dpi / 96) as isize),
        );
        send(
            s.language,
            CB_SETITEMHEIGHT,
            WPARAM(usize::MAX),
            LPARAM((26 * s.dpi / 96) as isize),
        );
        control(
            hwnd,
            s,
            w!("STATIC"),
            labels.get("tray_choose"),
            610,
            WINDOW_STYLE(0),
            3,
        );
        for (i, key) in TRAY_KEYS.iter().enumerate() {
            let c = control(
                hwnd,
                s,
                w!("BUTTON"),
                labels.get(key),
                600 + i,
                WINDOW_STYLE(BS_AUTOCHECKBOX as u32) | WS_TABSTOP,
                3,
            );
            send(
                c,
                BM_SETCHECK,
                WPARAM(usize::from(config.tray_display.flags()[i])),
                LPARAM(0),
            );
            s.tray_checks.push(c);
        }
        control(
            hwnd,
            s,
            w!("BUTTON"),
            labels.get("apply"),
            612,
            WS_TABSTOP,
            3,
        );
        control(
            hwnd,
            s,
            w!("STATIC"),
            labels.get("tray_preview"),
            613,
            WINDOW_STYLE(0),
            3,
        );
        s.tray_preview = control(hwnd, s, w!("STATIC"), "", 614, WINDOW_STYLE(0), 3);
        s.theme.attach(s.tray_preview, Kind::Preview, s.font);
        load_incidents(s);
    }
}
unsafe fn layout(hwnd: HWND, s: &mut State) {
    unsafe {
        let mut r = RECT::default();
        let _ = GetClientRect(hwnd, &mut r);
        let k = s.dpi as f64 / 96.0;
        let width = (r.right as f64 / k) as i32;
        let height = (r.bottom as f64 / k) as i32;
        for &(c, page) in &s.controls {
            let id = GetDlgCtrlID(c) as usize;
            if (10..=13).contains(&id) {
                theme::set_active(c, id - 10 == s.page);
            }
            let (x, y, w, h) = match id {
                10..=13 => (18 + (id as i32 - 10) * 120, 14, 112, 32),
                20 => (width - 336, 14, 180, 30),
                21 => (width - 148, 14, 132, 30),
                31 => (18, height - 38, width - 36, 24),
                40 => (18, 76, width - 36, 24),
                100..=113 => {
                    let i = id - 100;
                    (
                        18 + (i / 7) as i32 * (width / 2),
                        110 + (i % 7) as i32 * 28,
                        width / 2 - 74,
                        26,
                    )
                }
                500..=513 => {
                    let i = id - 500;
                    (
                        width / 2 - 34 + (i / 7) as i32 * (width / 2),
                        110 + (i % 7) as i32 * 28,
                        24,
                        26,
                    )
                }
                46 => (18, 334, width / 2 - 36, 20),
                47 => (width / 2 + 18, 334, width / 2 - 36, 20),
                200 => (18, 358, width / 2 - 36, 100),
                42 => (18, 402, 156, 22),
                201 => (180, 398, 130, 30),
                43 => (330, 402, 160, 22),
                202 => (500, 398, 80, 30),
                203 => (width / 2 + 18, 358, width / 2 - 36, 202),
                204 => (width - 160, 402, 140, 32),
                205 => (18, 444, 200, 28),
                44 => (18, 546, width - 36, (height - 632).max(35)),
                45 => (18, 76, width - 264, 62),
                301 => (width - 230, 76, 212, 32),
                300 => (18, 146, width - 36, 130),
                610 => (18, 76, width - 36, 24),
                600..=606 => {
                    let i = id - 600;
                    (
                        18 + (i / 4) as i32 * (width / 2),
                        112 + (i % 4) as i32 * 36,
                        width / 2 - 42,
                        30,
                    )
                }
                612 => (18, 276, 140, 32),
                613 => (18, 338, width - 36, 24),
                614 => (
                    18,
                    374,
                    (width - 36).min(460),
                    theme::preview_height(&window_text(c)),
                ),
                400..=406 => (
                    18 + (id as i32 - 400) * (width - 36) / 7,
                    if s.page == 2 { 284 } else { 68 },
                    (width - 36) / 7 - 6,
                    25,
                ),
                _ => (0, 0, 0, 0),
            };
            let _ = MoveWindow(
                c,
                (x as f64 * k) as i32,
                (y as f64 * k) as i32,
                (w as f64 * k) as i32,
                (h as f64 * k) as i32,
                true,
            );
            let _ = ShowWindow(
                c,
                if (page == usize::MAX || page == s.page || (page == 4 && matches!(s.page, 0 | 2)))
                    && !(s.page == 2 && s.mango_view && (400..=406).contains(&id))
                {
                    SW_SHOW
                } else {
                    SW_HIDE
                },
            );
        }
        let _ = InvalidateRect(Some(hwnd), None, true);
    }
}
fn set_text(hwnd: HWND, text: &str) {
    unsafe {
        let mut old = vec![0u16; GetWindowTextLengthW(hwnd).max(0) as usize + 1];
        let count = GetWindowTextW(hwnd, &mut old).max(0) as usize;
        if old[..count].iter().copied().eq(text.encode_utf16()) {
            return;
        }
        let text = wide(text);
        let _ = SetWindowTextW(hwnd, PCWSTR(text.as_ptr()));
    }
}
fn window_text(hwnd: HWND) -> String {
    let mut text = [0u16; 128];
    let n = unsafe { GetWindowTextW(hwnd, &mut text) }.max(0) as usize;
    String::from_utf16_lossy(&text[..n])
}
unsafe fn details_text(edit: HWND, text: &str) {
    unsafe {
        let mut previous = vec![0u16; GetWindowTextLengthW(edit).max(0) as usize + 1];
        let n = GetWindowTextW(edit, &mut previous).max(0) as usize;
        if previous[..n].iter().copied().eq(text.encode_utf16()) {
            return;
        }
        let top = send(edit, EM_GETFIRSTVISIBLELINE, WPARAM(0), LPARAM(0)).0;
        let (mut start, mut end) = (0u32, 0u32);
        send(
            edit,
            EM_GETSEL,
            WPARAM((&mut start as *mut u32) as usize),
            LPARAM((&mut end as *mut u32) as isize),
        );
        send(edit, WM_SETREDRAW, WPARAM(0), LPARAM(0));
        set_text(edit, text);
        send(
            edit,
            EM_SETSEL,
            WPARAM(start as usize),
            LPARAM(end as isize),
        );
        let now = send(edit, EM_GETFIRSTVISIBLELINE, WPARAM(0), LPARAM(0)).0;
        send(edit, EM_LINESCROLL, WPARAM(0), LPARAM(top - now));
        details_scrollbar(edit);
        send(edit, WM_SETREDRAW, WPARAM(1), LPARAM(0));
        let _ = InvalidateRect(Some(edit), None, false);
    }
}
unsafe fn details_scrollbar(edit: HWND) {
    unsafe {
        let mut rect = RECT::default();
        let _ = GetClientRect(edit, &mut rect);
        let dc = GetDC(Some(edit));
        let old = SelectObject(
            dc,
            HGDIOBJ(send(edit, WM_GETFONT, WPARAM(0), LPARAM(0)).0 as *mut _),
        );
        let mut metrics = TEXTMETRICW::default();
        let _ = GetTextMetricsW(dc, &mut metrics);
        SelectObject(dc, old);
        ReleaseDC(Some(edit), dc);
        let lines = send(edit, EM_GETLINECOUNT, WPARAM(0), LPARAM(0)).0 as i32;
        let needed = lines * (metrics.tmHeight + metrics.tmExternalLeading) > rect.bottom - 4;
        let visible = GetWindowLongPtrW(edit, GWL_STYLE) as u32 & WS_VSCROLL.0 != 0;
        if needed != visible {
            let _ = ShowScrollBar(edit, SB_VERT, needed);
        }
    }
}
struct Details {
    edit: HWND,
    font: HFONT,
    theme: Theme,
}
impl Drop for Details {
    fn drop(&mut self) {
        unsafe {
            let _ = DeleteObject(HGDIOBJ(self.font.0));
        }
    }
}
unsafe fn open_details(owner: HWND, s: &mut State) {
    unsafe {
        if IsWindow(Some(s.details_window)).as_bool() {
            let _ = ShowWindow(s.details_window, SW_RESTORE);
            let _ = SetForegroundWindow(s.details_window);
            return;
        }
        let instance = HINSTANCE(GetModuleHandleW(None).unwrap_or_default().0);
        let class = w!("stutter-watch-details");
        let _ = RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(details_proc),
            hInstance: instance,
            lpszClassName: class,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            ..Default::default()
        });
        let mut lf = LOGFONTW::default();
        GetObjectW(
            HGDIOBJ(s.font.0),
            std::mem::size_of::<LOGFONTW>() as i32,
            Some((&mut lf as *mut LOGFONTW).cast()),
        );
        let details = Box::new(Details {
            edit: HWND::default(),
            font: CreateFontIndirectW(&lf),
            theme: Theme::new(),
        });
        let raw = Box::into_raw(details);
        let title = wide(s.text.get("details"));
        let Ok(hwnd) = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            class,
            PCWSTR(title.as_ptr()),
            WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            820 * s.dpi / 96,
            520 * s.dpi / 96,
            Some(owner),
            None,
            Some(instance),
            Some(raw.cast()),
        ) else {
            drop(Box::from_raw(raw));
            return;
        };
        (*raw).theme.titlebar(hwnd);
        theme::window_icon(hwnd);
        if let Ok(edit) = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("EDIT"),
            w!(""),
            WS_CHILD
                | WS_VISIBLE
                | WS_VSCROLL
                | WS_TABSTOP
                | WINDOW_STYLE((ES_MULTILINE | ES_READONLY | ES_AUTOVSCROLL) as u32),
            12,
            12,
            100,
            100,
            Some(hwnd),
            None,
            Some(instance),
            None,
        ) {
            (*raw).edit = edit;
            send(edit, WM_SETFONT, WPARAM((*raw).font.0 as usize), LPARAM(1));
            s.details_edit = edit;
        }
        s.details_window = hwnd;
        let mut r = RECT::default();
        let _ = GetClientRect(hwnd, &mut r);
        let _ = MoveWindow((*raw).edit, 14, 14, r.right - 28, r.bottom - 28, true);
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetWindowPos(
            hwnd,
            None,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_SHOWWINDOW,
        );
        let _ = SetForegroundWindow(hwnd);
    }
}
unsafe extern "system" fn details_proc(
    hwnd: HWND,
    message: u32,
    wp: WPARAM,
    lp: LPARAM,
) -> LRESULT {
    unsafe {
        if message == WM_NCCREATE {
            SetWindowLongPtrW(
                hwnd,
                GWLP_USERDATA,
                (*(lp.0 as *const CREATESTRUCTW)).lpCreateParams as isize,
            );
        }
        let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Details;
        if let Some(details) = ptr.as_mut() {
            match message {
                WM_SIZE => {
                    let mut r = RECT::default();
                    let _ = GetClientRect(hwnd, &mut r);
                    let _ = MoveWindow(
                        details.edit,
                        14,
                        14,
                        (r.right - 28).max(1),
                        (r.bottom - 28).max(1),
                        true,
                    );
                    details_scrollbar(details.edit);
                    return LRESULT(0);
                }
                WM_GETMINMAXINFO => {
                    let m = &mut *(lp.0 as *mut MINMAXINFO);
                    m.ptMinTrackSize = POINT { x: 480, y: 300 };
                    return LRESULT(0);
                }
                WM_CTLCOLORSTATIC | WM_CTLCOLOREDIT => {
                    let dc = HDC(wp.0 as *mut _);
                    SetTextColor(dc, theme::FOREGROUND);
                    SetBkColor(dc, theme::FIELD);
                    return LRESULT(details.theme.field.0 as isize);
                }
                WM_ERASEBKGND => return LRESULT(1),
                WM_PAINT => {
                    let mut p = PAINTSTRUCT::default();
                    let dc = BeginPaint(hwnd, &mut p);
                    let mut r = RECT::default();
                    let _ = GetClientRect(hwnd, &mut r);
                    FillRect(dc, &r, details.theme.background);
                    let _ = EndPaint(hwnd, &p);
                    return LRESULT(0);
                }
                WM_CLOSE => {
                    let _ = DestroyWindow(hwnd);
                    return LRESULT(0);
                }
                WM_NCDESTROY => {
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                    drop(Box::from_raw(ptr));
                    return DefWindowProcW(hwnd, message, wp, lp);
                }
                _ => {}
            }
        }
        DefWindowProcW(hwnd, message, wp, lp)
    }
}
fn budget_text(bytes: u64) -> String {
    let fraction = format!("{:06}", bytes % 1_000_000);
    let fraction = fraction.trim_end_matches('0');
    if fraction.is_empty() {
        (bytes / 1_000_000).to_string()
    } else {
        format!("{}.{fraction}", bytes / 1_000_000)
    }
}
struct Snapshot {
    config: Config,
    paused: bool,
    latest: Option<Arc<Sample>>,
    recording: Status,
    observed_seconds: f64,
    warnings: Vec<String>,
    samples: u64,
}
fn refresh(s: &mut State) {
    // Copy only status data. The graph's 10-minute point buffer stays in place.
    let d = {
        let d = s.shared.lock().unwrap_or_else(|p| p.into_inner());
        Snapshot {
            config: d.config.clone(),
            paused: d.paused,
            latest: d.latest.clone(),
            recording: d.recording.clone(),
            observed_seconds: d.observed_seconds,
            warnings: d.warnings.clone(),
            samples: d.samples,
        }
    };
    if let Some((button, _)) = s
        .controls
        .iter()
        .find(|(c, _)| unsafe { GetDlgCtrlID(*c) } == 21)
    {
        set_text(
            *button,
            s.text.get(if d.paused { "resume" } else { "pause" }),
        );
    }
    let e = capacity::estimate(d.recording.usage.written_bytes, d.observed_seconds);
    let number = |v: Option<f64>| v.map(|v| format!("{v:.1}")).unwrap_or_else(|| "—".into());
    let mut bottom = format!(
        "{}: {} / {} MB · {}: {} MB · {} MB/{}",
        s.text.get("ordinary"),
        number(Some(d.recording.usage.ordinary_bytes as f64 / 1e6)),
        budget_text(d.config.history.ordinary_budget_bytes),
        s.text.get("protected"),
        number(Some(d.recording.usage.protected_bytes as f64 / 1e6)),
        number(e.bytes_per_hour.map(|v| v / 1e6)),
        s.text.get("unit_hour"),
    );
    if !d.recording.error.is_empty() {
        bottom = format!(
            "{} · {}",
            s.text.get("recording_error"),
            s.text.get("details_hint")
        );
    }
    set_text(s.footer, &bottom);
    if s.page == 3 {
        let mut preview_config = d.config.clone();
        preview_config.tray_display = unsafe { draft_tray(s) };
        let preview = crate::tray_display::tooltip(
            &preview_config,
            &s.text,
            d.latest.as_deref(),
            d.paused,
            !d.warnings.is_empty() || !d.recording.error.is_empty(),
        )
        .replace('\n', "\r\n");
        let old_height = theme::preview_height(&window_text(s.tray_preview));
        let new_height = theme::preview_height(&preview);
        set_text(s.tray_preview, &preview);
        if old_height != new_height {
            unsafe {
                let mut rect = RECT::default();
                let _ = GetClientRect(s.tray_preview, &mut rect);
                let _ = SetWindowPos(
                    s.tray_preview,
                    None,
                    0,
                    0,
                    rect.right,
                    new_height * s.dpi / 96,
                    SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
                );
                let _ = InvalidateRect(GetParent(s.tray_preview).ok(), None, false);
            }
        }
    }
    if s.page == 1 {
        let mut lines = Vec::new();
        for (i, key) in SOURCE_KEYS.iter().enumerate() {
            let selected = d.config.collection.flags()[i];
            let available = d.latest.as_ref().is_some_and(|v| match i {
                0 => v.cpu.usage_pct.is_some(),
                1 => !v.cpu.cores.is_empty(),
                2 => v.memory.total_bytes > 0,
                3 => !v.disk.devices.is_empty(),
                4 => !v.cpu.top.is_empty(),
                5 => !v.memory.top.is_empty(),
                6 => !v.disk.top.is_empty(),
                7 => v.foreground.responsive.is_some(),
                8 => v.frames.status.starts_with("active"),
                9 => !v.gpu.adapters.is_empty(),
                10 => v.gpu.adapters.iter().any(|a| a.nvml.is_some()),
                11 => v.dpc.time_pct.is_some(),
                12 => v.dpc.kernel_status == "active",
                13 => v.boot.active,
                _ => false,
            });
            let status = if !selected {
                "disabled"
            } else if available {
                "active"
            } else if i == 10 || i == 13 {
                "waiting"
            } else {
                "unknown"
            };
            set_text(
                s.checks[i],
                &if selected && !available {
                    format!("{} · {}", s.text.get(key), s.text.get(status))
                } else {
                    s.text.get(key).to_owned()
                },
            );
            let help = format!(
                "{}{}",
                s.text.get(&format!("help_{key}")),
                if selected && !available {
                    format!(
                        "\r\n\r\n{}: {}",
                        s.text.get(status),
                        source_reason(s, &d, i)
                    )
                } else {
                    String::new()
                }
            );
            unsafe {
                update_help(s, i, &help);
            }
            if unsafe { IsWindow(Some(s.details_window)).as_bool() } {
                lines.push(format!(
                    "{}: {}{}",
                    s.text.get(key),
                    s.text.get(status),
                    if selected && !available {
                        format!(" — {}", source_reason(s, &d, i))
                    } else {
                        String::new()
                    }
                ));
            }
        }
        if unsafe { IsWindow(Some(s.details_window)).as_bool() } {
            lines.push(format!(
                "\r\n{} {} · {} {} · {}: {:.0} MB",
                d.config.sample_ms,
                s.text.get("unit_ms"),
                d.samples,
                s.text.get("samples"),
                s.text.get("free"),
                d.recording.usage.free_bytes as f64 / 1e6
            ));
            lines.extend(
                d.warnings
                    .iter()
                    .map(|w| s.text.diagnostic(w))
                    .collect::<std::collections::BTreeSet<_>>(),
            );
            if !d.recording.error.is_empty() {
                lines.push(s.text.recording_issue(&d.recording.error));
            }
            unsafe {
                details_text(s.details_edit, &lines.join("\r\n"));
            }
        }
    }
}
fn source_reason<'a>(s: &'a State, d: &Snapshot, index: usize) -> &'a str {
    let access_denied = d.warnings.iter().any(|w| {
        w.contains("WIN32_ERROR(5)")
            && w.starts_with(if index == 8 {
                "Frame ETW"
            } else {
                "Kernel ETW"
            })
    });
    s.text.get(match index {
        8 | 12 if access_denied => "reason_etw_permissions",
        8 | 12 => "reason_etw",
        9 => "reason_gpu",
        10 => "reason_nvml",
        13 => "reason_boot",
        _ => "reason_missing",
    })
}
unsafe fn update_help(s: &mut State, index: usize, text: &str) {
    unsafe {
        let value = wide(text);
        if s.help_texts[index] == value {
            return;
        }
        s.help_texts[index] = value;
        if let Some(&(help, _)) = s
            .controls
            .iter()
            .find(|(c, _)| GetDlgCtrlID(*c) == 500 + index as i32)
        {
            let info = TTTOOLINFOW {
                cbSize: std::mem::size_of::<TTTOOLINFOW>() as u32,
                hwnd: GetParent(help).unwrap_or_default(),
                uId: help.0 as usize,
                lpszText: PWSTR(s.help_texts[index].as_mut_ptr()),
                ..Default::default()
            };
            send(
                s.tooltip,
                TTM_UPDATETIPTEXTW,
                WPARAM(0),
                LPARAM((&info as *const TTTOOLINFOW) as isize),
            );
        }
    }
}
unsafe fn draft_tray(s: &State) -> crate::config::TrayDisplay {
    let mut flags = [false; 7];
    for (i, &c) in s.tray_checks.iter().enumerate() {
        flags[i] = unsafe { send(c, BM_GETCHECK, WPARAM(0), LPARAM(0)).0 == 1 };
    }
    let mut display = crate::config::TrayDisplay::default();
    display.set_flags(flags);
    display
}
unsafe fn apply(s: &mut State) -> Result<()> {
    unsafe {
        let mut config = s
            .shared
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .config
            .clone();
        let mut flags = [false; 14];
        for (i, c) in s.checks.iter().enumerate() {
            flags[i] = send(*c, BM_GETCHECK, WPARAM(0), LPARAM(0)).0 == 1;
        }
        config.collection.set_flags(flags);
        config.history.mode = if send(s.mode, CB_GETCURSEL, WPARAM(0), LPARAM(0)).0 == 0 {
            HistoryMode::Lossless
        } else {
            HistoryMode::Summary
        };
        let mb = window_text(s.budget)
            .replace(',', ".")
            .parse::<f64>()
            .map_err(|_| anyhow::anyhow!(s.text.get("invalid_budget").to_owned()))?;
        anyhow::ensure!(
            mb.is_finite() && (1.0..=100_000.0).contains(&mb),
            "{}",
            s.text.get("invalid_budget")
        );
        config.history.ordinary_budget_bytes = (mb * 1_000_000.0).round() as u64;
        config.history.retention_hours = window_text(s.retention)
            .parse()
            .map_err(|_| anyhow::anyhow!(s.text.get("invalid_retention").to_owned()))?;
        let language = send(s.language, CB_GETCURSEL, WPARAM(0), LPARAM(0)).0 as usize;
        config.language = LANGUAGES.get(language).unwrap_or(&"en").to_string();
        config
            .validate()
            .map_err(|_| anyhow::anyhow!(s.text.get("invalid_settings").to_owned()))?;
        s.sender
            .try_send(Command::Apply(Box::new(config)))
            .map_err(|_| anyhow::anyhow!(s.text.get("operation_failed").to_owned()))?;
        Ok(())
    }
}
unsafe fn config_changed(hwnd: HWND, s: &mut State) {
    unsafe {
        let config = s
            .shared
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .config
            .clone();
        s.text = Text::new(&config.language);
        s.incident_rows.clear();
        let visible: Vec<_> = s
            .controls
            .iter()
            .filter(|(control, _)| IsWindowVisible(*control).as_bool())
            .map(|(control, _)| *control)
            .collect();
        send(hwnd, WM_SETREDRAW, WPARAM(0), LPARAM(0));
        for &control in &visible {
            send(control, WM_SETREDRAW, WPARAM(0), LPARAM(0));
        }
        for &(control, _) in &s.controls {
            let id = GetDlgCtrlID(control) as usize;
            let key = match id {
                10 => Some("live"),
                11 => Some("data"),
                12 => Some("incidents"),
                13 => Some("tray_settings"),
                610 => Some("tray_choose"),
                612 => Some("apply"),
                613 => Some("tray_preview"),
                600..=606 => Some(TRAY_KEYS[id - 600]),
                20 => Some("mark"),
                21 => Some("pause"),
                40 => Some("collect_hint"),
                42 => Some("budget"),
                43 => Some("retention"),
                45 => Some("incident_hint"),
                46 => Some("history_mode"),
                47 => Some("language"),
                205 => Some("details"),
                301 => Some("import_mangohud"),
                100..=113 => Some(SOURCE_KEYS[id - 100]),
                204 => Some("apply"),
                _ => None,
            };
            if let Some(key) = key {
                set_text(control, s.text.get(key));
            }
            if (600..=606).contains(&id) {
                send(
                    control,
                    BM_SETCHECK,
                    WPARAM(usize::from(config.tray_display.flags()[id - 600])),
                    LPARAM(0),
                );
            }
            if (100..=113).contains(&id) {
                send(
                    control,
                    BM_SETCHECK,
                    WPARAM(usize::from(config.collection.flags()[id - 100])),
                    LPARAM(0),
                );
            }
        }
        send(s.mode, CB_RESETCONTENT, WPARAM(0), LPARAM(0));
        for key in ["lossless", "summary"] {
            let text = wide(s.text.get(key));
            send(
                s.mode,
                CB_ADDSTRING,
                WPARAM(0),
                LPARAM(text.as_ptr() as isize),
            );
        }
        send(
            s.mode,
            CB_SETCURSEL,
            WPARAM(usize::from(config.history.mode == HistoryMode::Summary)),
            LPARAM(0),
        );
        set_text(s.budget, &budget_text(config.history.ordinary_budget_bytes));
        set_text(s.retention, &config.history.retention_hours.to_string());
        send(
            s.language,
            CB_SETCURSEL,
            WPARAM(
                LANGUAGES
                    .iter()
                    .position(|&l| l == config.language)
                    .unwrap_or(0),
            ),
            LPARAM(0),
        );
        refresh(s);
        update_incident_caption(s);
        for &control in &visible {
            send(control, WM_SETREDRAW, WPARAM(1), LPARAM(0));
        }
        send(hwnd, WM_SETREDRAW, WPARAM(1), LPARAM(0));
        let _ = RedrawWindow(Some(hwnd), None, None, RDW_INVALIDATE | RDW_ALLCHILDREN);
    }
}
unsafe fn load_incidents(s: &mut State) {
    unsafe {
        let root = s
            .shared
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .config
            .logs_dir
            .clone();
        let mut paths = crate::storage::managed_files(&root).unwrap_or_default();
        paths.retain(|(p, _, _)| {
            p.file_name().is_some_and(|n| {
                let n = n.to_string_lossy();
                n.starts_with("incident-") || n.starts_with("mangohud-")
            })
        });
        paths.sort_by_key(|(_, m, _)| std::cmp::Reverse(*m));
        paths.truncate(200);
        let present: std::collections::HashSet<_> =
            paths.iter().map(|(p, _, _)| p.clone()).collect();
        s.incident_rows.retain(|p, _| present.contains(p));
        let mut next_paths = Vec::new();
        let mut next_rows = Vec::new();
        for (path, modified, bytes) in paths {
            if let Some((old_modified, old_bytes, row)) = s.incident_rows.get(&path)
                && *old_modified == modified
                && *old_bytes == bytes
            {
                next_rows.push(row.clone());
                next_paths.push(path);
                continue;
            }
            if let Ok((name, timestamp)) = stutter_watch::mangohud::header(&path) {
                let time = chrono::DateTime::from_timestamp_millis(timestamp as i64)
                    .unwrap_or_default()
                    .with_timezone(&chrono::Local)
                    .format("%m-%d %H:%M:%S");
                let row = format!(
                    "{time}   MangoHud   {}",
                    name.chars().take(180).collect::<String>()
                );
                s.incident_rows
                    .insert(path.clone(), (modified, bytes, row.clone()));
                next_rows.push(row);
                next_paths.push(path);
            } else if let Ok(header) = crate::storage::incident_header(&path) {
                let time = chrono::DateTime::from_timestamp_millis(header.start_ms as i64)
                    .unwrap_or_default()
                    .with_timezone(&chrono::Local)
                    .format("%m-%d %H:%M:%S")
                    .to_string();
                let text = wide(&format!(
                    "{}   {}   {}   {}",
                    time,
                    header
                        .triggers
                        .iter()
                        .map(|t| s.text.get(crate::assessment::reason_key(&t.reason)))
                        .collect::<Vec<_>>()
                        .join(", "),
                    if header.protected {
                        s.text.get("protected")
                    } else {
                        ""
                    },
                    if !header.complete {
                        s.text.get("provisional")
                    } else {
                        ""
                    }
                ));
                let row: String = String::from_utf16_lossy(&text[..text.len() - 1])
                    .chars()
                    .take(2048)
                    .collect();
                s.incident_rows
                    .insert(path.clone(), (modified, bytes, row.clone()));
                next_rows.push(row);
                next_paths.push(path);
            }
        }
        if s.paths == next_paths && s.rows == next_rows {
            if !s.paths.is_empty() && send(s.list, LB_GETCURSEL, WPARAM(0), LPARAM(0)).0 < 0 {
                send(s.list, LB_SETCURSEL, WPARAM(0), LPARAM(0));
                select_incident(s);
            } else if s.incident_message.is_some() {
                select_incident(s);
            }
            return;
        }
        let selected = send(s.list, LB_GETCURSEL, WPARAM(0), LPARAM(0)).0;
        let top = send(s.list, LB_GETTOPINDEX, WPARAM(0), LPARAM(0)).0.max(0) as usize;
        let next_selected = incident_selection(&s.paths, &next_paths, selected);
        let next_top = s
            .paths
            .get(top)
            .and_then(|p| next_paths.iter().position(|n| n == p))
            .unwrap_or(top.min(next_paths.len().saturating_sub(1)));
        let next_top = if selected < 0 { 0 } else { next_top };
        let visible = IsWindowVisible(s.list).as_bool();
        if visible {
            send(s.list, WM_SETREDRAW, WPARAM(0), LPARAM(0));
        }
        send(s.list, LB_RESETCONTENT, WPARAM(0), LPARAM(0));
        for row in &next_rows {
            let row = wide(row);
            send(
                s.list,
                LB_ADDSTRING,
                WPARAM(0),
                LPARAM(row.as_ptr() as isize),
            );
        }
        s.paths = next_paths;
        s.rows = next_rows;
        if let Some(selected) = next_selected {
            send(s.list, LB_SETCURSEL, WPARAM(selected), LPARAM(0));
            select_incident(s);
        } else {
            s.incident.clear();
            s.selected_header = None;
            s.incident_message = None;
            if let Some(&(caption, _)) = s.controls.iter().find(|(c, _)| GetDlgCtrlID(*c) == 45) {
                set_text(caption, s.text.get("incident_hint"));
            }
        }
        send(s.list, LB_SETTOPINDEX, WPARAM(next_top), LPARAM(0));
        if visible {
            send(s.list, WM_SETREDRAW, WPARAM(1), LPARAM(0));
            let _ = InvalidateRect(Some(s.list), None, false);
        }
    }
}
fn import_mangohud(config: Config, owner: usize) -> Result<Option<PathBuf>> {
    use windows::Win32::{System::Com::*, UI::Shell::*};
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
        struct Apartment;
        impl Drop for Apartment {
            fn drop(&mut self) {
                unsafe {
                    CoUninitialize();
                }
            }
        }
        let _apartment = Apartment;
        let dialog: IFileOpenDialog =
            CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)?;
        dialog.SetOptions(FOS_FILEMUSTEXIST | FOS_FORCEFILESYSTEM | FOS_NOCHANGEDIR)?;
        dialog.SetFileTypes(&[COMDLG_FILTERSPEC {
            pszName: w!("MangoHud CSV"),
            pszSpec: w!("*.csv"),
        }])?;
        if let Err(error) = dialog.Show(Some(HWND(owner as *mut _))) {
            if error.code().0 as u32 == 0x800704c7 {
                return Ok(None);
            }
            return Err(error.into());
        }
        let raw = dialog.GetResult()?.GetDisplayName(SIGDN_FILESYSPATH)?;
        let path = raw.to_string();
        CoTaskMemFree(Some(raw.0.cast()));
        let path = PathBuf::from(path?);
        Ok(Some(stutter_watch::mangohud::import(
            &path,
            &config.logs_dir,
            config.history.reserve_free_bytes,
        )?))
    }
}
fn incident_selection(previous: &[PathBuf], next: &[PathBuf], selected: isize) -> Option<usize> {
    usize::try_from(selected)
        .ok()
        .and_then(|i| previous.get(i))
        .and_then(|p| next.iter().position(|n| n == p))
        .or_else(|| (!next.is_empty()).then_some(0))
}
unsafe fn update_incident_caption(s: &State) {
    unsafe {
        if let Some(&(caption, _)) = s.controls.iter().find(|(c, _)| GetDlgCtrlID(*c) == 45) {
            let text = if s.mango_view {
                s.text.get("relative_time").to_owned()
            } else {
                s.incident_message
                    .map(|key| s.text.get(key).to_owned())
                    .unwrap_or_else(|| {
                        incident_summary(&s.incident, s.selected_header.as_ref(), &s.text)
                    })
            };
            set_text(caption, &text);
        }
    }
}
unsafe fn select_incident(s: &mut State) {
    unsafe {
        let config = s
            .shared
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .config
            .clone();
        let selected = send(s.list, LB_GETCURSEL, WPARAM(0), LPARAM(0)).0 as usize;
        s.incident.clear();
        s.selected_header = None;
        s.mango_view = false;
        s.incident_message = None;
        if let Some(path) = s.paths.get(selected) {
            let result = crate::report::read_records(path, |r| match r {
                Record::Sample(sample) if s.incident.len() < 6002 => {
                    s.incident.push(Point::from_sample(&sample, &config))
                }
                Record::Incident(h) => s.selected_header = Some(h),
                Record::ExternalLog { .. } => s.mango_view = true,
                Record::ExternalFrame {
                    elapsed_ns,
                    fps,
                    frametime_ms,
                } => {
                    let mut values = [None; 7];
                    values[3] = Some(frametime_ms);
                    values[4] = Some(fps);
                    s.incident.push(Point {
                        ts: elapsed_ns / 1_000_000,
                        values,
                        alerts: if frametime_ms > config.frame_ms {
                            crate::assessment::PRESENT
                        } else {
                            0
                        },
                        span_ms: 100,
                    });
                    stutter_watch::mangohud::reduce(&mut s.incident);
                }
                _ => {}
            });
            s.incident_message = if result.is_err() {
                Some("incident_unreadable")
            } else if s.incident.is_empty() {
                Some("incident_no_samples")
            } else {
                None
            };
        }
        update_incident_caption(s);
        let parent = GetParent(s.list).unwrap_or_default();
        if !parent.is_invalid() {
            layout(parent, s);
        }
    }
}
fn incident_summary(points: &[Point], header: Option<&Incident>, text: &Text) -> String {
    if points.is_empty() {
        return text.get("incident_hint").into();
    }
    let flags = points.iter().fold(0, |mask, p| mask | p.alerts)
        | header
            .into_iter()
            .flat_map(|h| &h.triggers)
            .fold(0, |mask, t| {
                mask | crate::assessment::reason_flag(&t.reason)
            });
    let conclusion = text.get(if flags & crate::assessment::DELAYS != 0 {
        "analysis_delay_detected"
    } else if flags
        & (crate::assessment::RAM_LOW
            | crate::assessment::DISK
            | crate::assessment::DPC
            | crate::assessment::DRIVER)
        != 0
    {
        "analysis_resource_alert"
    } else if flags & (crate::assessment::CPU_BUSY | crate::assessment::RAM_BUSY) != 0 {
        "analysis_pressure_only"
    } else {
        "analysis_no_delay"
    });
    let events = header
        .into_iter()
        .flat_map(|h| &h.triggers)
        .map(|t| text.get(crate::assessment::reason_key(&t.reason)))
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let events = format!(
        "{}{}",
        events
            .iter()
            .take(3)
            .copied()
            .collect::<Vec<_>>()
            .join(", "),
        if events.len() > 3 {
            format!(" +{}", events.len() - 3)
        } else {
            String::new()
        }
    );
    let mut peaks = Vec::new();
    let milliseconds = format!(" {}", text.get("unit_ms"));
    for (i, name, unit) in [
        (0, "CPU", "%"),
        (1, "RAM", "%"),
        (2, "I/O", milliseconds.as_str()),
        (3, "Present", milliseconds.as_str()),
        (4, "UI", milliseconds.as_str()),
        (5, "DPC", "%"),
        (6, "ISR", "%"),
    ] {
        if let Some(value) = points
            .iter()
            .filter_map(|p| p.values[i])
            .filter(|v| v.is_finite())
            .max_by(f64::total_cmp)
        {
            let number = format!("{value:.1}");
            let number = number.trim_end_matches('0').trim_end_matches('.');
            peaks.push(format!("{name}: {number}{unit}"));
        }
    }
    format!(
        "{conclusion}\r\n{}: {events}\r\n{}: {}",
        text.get("analysis_events"),
        text.get("analysis_peaks"),
        peaks.join(" · ")
    )
}
/// Screen-space bins preserve short events without bridging unmeasured gaps.
fn alert_bands(
    points: &[Point],
    series: &[usize],
    start: u64,
    end: u64,
    width: usize,
) -> Vec<(usize, usize, u8)> {
    let width = width.max(1);
    let mut bins = vec![0u8; width];
    for point in points.iter().filter(|p| p.ts >= start && p.ts <= end) {
        let severity = crate::assessment::severity(point.alerts, series);
        if severity == 0 {
            continue;
        }
        let left = ((point.ts.saturating_sub(point.span_ms as u64).max(start) - start) as f64
            / (end - start).max(1) as f64
            * (width - 1) as f64) as usize;
        let right = (((point.ts - start) as f64 / (end - start).max(1) as f64 * (width - 1) as f64)
            .ceil() as usize
            + 1)
        .max(left + 2)
        .min(width);
        for bin in &mut bins[left..right] {
            *bin = (*bin).max(severity);
        }
    }
    let mut bands = Vec::new();
    let mut x = 0;
    while x < width {
        let severity = bins[x];
        let start = x;
        while x < width && bins[x] == severity {
            x += 1;
        }
        if severity != 0 {
            bands.push((start, x, severity));
        }
    }
    bands
}
unsafe fn draw_text(dc: HDC, x: i32, y: i32, text: &str) {
    let text: Vec<_> = text.encode_utf16().collect();
    unsafe {
        let _ = TextOutW(dc, x, y, &text);
    }
}
// For each horizontal pixel retain both extremes; narrow peaks never become an average.

unsafe fn paint(hwnd: HWND, s: &mut State) {
    unsafe {
        let mut ps = PAINTSTRUCT::default();
        let target = BeginPaint(hwnd, &mut ps);
        let mut client = RECT::default();
        let _ = GetClientRect(hwnd, &mut client);
        if matches!(s.page, 1 | 3) {
            FillRect(target, &client, s.theme.background);
            let _ = EndPaint(hwnd, &ps);
            return;
        }
        let points: Vec<_> = if s.page == 2 {
            s.incident.clone()
        } else {
            s.shared
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .points
                .iter()
                .cloned()
                .collect()
        };
        let k = s.dpi as f64 / 96.0;
        let top = if s.page == 2 { 318.0 } else { 102.0 };
        let bottom = client.bottom as f64 / k - 86.0;
        let height = (((bottom - top - 24.0)
            / if s.page == 2 && s.mango_view {
                1.0
            } else {
                2.0
            })
        .max(64.0)
            * k)
            .ceil() as i32;
        let width = (((client.right as f64 / k - 52.0) / 2.0) * k).ceil() as i32;
        let mut charts = Vec::new();
        let mango = s.page == 2 && s.mango_view;
        let definitions = if mango {
            vec![("frame_time", vec![3]), ("fps", vec![4])]
        } else {
            vec![
                ("graph_cpu", vec![0, 1]),
                ("graph_disk", vec![2]),
                ("graph_response", vec![3, 4]),
                ("graph_dpc", vec![5, 6]),
            ]
        };
        for (i, (key, series)) in definitions.into_iter().enumerate() {
            let left = (18.0 * k) as i32 + (i % 2) as i32 * (width + (16.0 * k) as i32);
            let y = (top * k) as i32 + (i / 2) as i32 * (height + (24.0 * k) as i32);
            charts.push((
                RECT {
                    left,
                    top: y,
                    right: left + width,
                    bottom: y + height,
                },
                key,
                series
                    .into_iter()
                    .filter(|&i| mango || s.shown[i])
                    .collect::<Vec<_>>(),
            ));
        }
        // Clear only the margins on screen. Graph rectangles are replaced by a
        // completed image, never by an intermediate white background.
        let saved = SaveDC(target);
        for (r, _, _) in &charts {
            ExcludeClipRect(target, r.left, r.top, r.right, r.bottom);
        }
        FillRect(target, &client, s.theme.background);
        let _ = RestoreDC(target, saved);
        if s.surface
            .as_ref()
            .is_none_or(|surface| surface.width != width || surface.height != height)
        {
            s.surface = Surface::new(target, width, height);
        }
        if let Some(surface) = s.surface.as_ref() {
            let dc = surface.dc;
            for (r, key, series) in charts {
                let local = RECT {
                    left: 0,
                    top: 0,
                    right: width,
                    bottom: height,
                };
                FillRect(dc, &local, s.theme.background);
                let _ = SetViewportOrgEx(dc, -r.left, -r.top, None);
                let previous = SelectObject(dc, HGDIOBJ(s.font.0));
                SetBkMode(dc, TRANSPARENT);
                SetTextColor(dc, theme::FOREGROUND);
                if !series.is_empty() {
                    chart(
                        dc,
                        r,
                        s.text.get(key),
                        &points,
                        &series,
                        s.selected_header.as_ref().filter(|_| s.page == 2),
                        s.page == 0,
                        mango,
                        &s.text,
                        k,
                    );
                } else {
                    draw_text(dc, r.left, r.top, s.text.get(key));
                }
                SelectObject(dc, previous);
                let _ = SetViewportOrgEx(dc, 0, 0, None);
                let _ = BitBlt(
                    target,
                    r.left,
                    r.top,
                    width,
                    height,
                    Some(dc),
                    0,
                    0,
                    SRCCOPY,
                );
            }
        }
        let _ = EndPaint(hwnd, &ps);
    }
}
#[allow(clippy::too_many_arguments)]
unsafe fn chart(
    dc: HDC,
    r: RECT,
    title: &str,
    points: &[Point],
    series: &[usize],
    header: Option<&Incident>,
    live: bool,
    relative: bool,
    text: &Text,
    k: f64,
) {
    unsafe {
        draw_text(dc, r.left, r.top, title);
        if points.is_empty() {
            return;
        }
        let plot = RECT {
            left: r.left + (42.0 * k) as i32,
            top: r.top + (28.0 * k) as i32,
            right: r.right - 8,
            bottom: r.bottom - (22.0 * k) as i32,
        };
        if plot.right <= plot.left || plot.bottom <= plot.top {
            return;
        }
        let end = points.last().map(|p| p.ts).unwrap_or(1);
        let start = if live {
            end.saturating_sub(600_000)
                .max(points.first().map(|p| p.ts).unwrap_or(0))
        } else {
            points.first().map(|p| p.ts).unwrap_or(0)
        };
        let max = if !relative && series.iter().any(|&metric| metric < 2) {
            100.0
        } else {
            points
                .iter()
                .flat_map(|p| series.iter().filter_map(|&i| p.values[i]))
                .fold(
                    if series[0] == 3 && !relative {
                        250.0f64
                    } else if series[0] == 2 {
                        10.0
                    } else {
                        5.0
                    },
                    f64::max,
                )
                * 1.15
        };
        let bands = alert_bands(
            points,
            series,
            start,
            end,
            (plot.right - plot.left) as usize,
        );
        for &(left, right, severity) in &bands {
            let area = RECT {
                left: plot.left + left as i32,
                top: plot.top,
                right: plot.left + right as i32,
                bottom: plot.bottom,
            };
            let previous = SetDCBrushColor(
                dc,
                if severity == 2 {
                    theme::DANGER_BG
                } else {
                    theme::WARNING_BG
                },
            );
            FillRect(dc, &area, HBRUSH(GetStockObject(DC_BRUSH).0));
            SetDCBrushColor(
                dc,
                if severity == 2 {
                    theme::DANGER
                } else {
                    theme::AMBER
                },
            );
            FillRect(
                dc,
                &RECT {
                    bottom: area.top + (2.0 * k) as i32,
                    ..area
                },
                HBRUSH(GetStockObject(DC_BRUSH).0),
            );
            SetDCBrushColor(dc, previous);
        }
        if let Some(severity) = bands.iter().map(|b| b.2).max() {
            let key = if severity == 1 {
                "analysis_load"
            } else if series.contains(&1) {
                "event_memory"
            } else if series.contains(&2) {
                "event_disk"
            } else if series.contains(&3) || series.contains(&4) {
                "analysis_delay"
            } else {
                "event_dpc"
            };
            SetTextColor(
                dc,
                if severity == 2 {
                    theme::DANGER
                } else {
                    theme::AMBER
                },
            );
            theme::label(
                dc,
                RECT {
                    left: r.left + (220.0 * k) as i32,
                    top: r.top,
                    right: r.right - 8,
                    bottom: r.top + (20.0 * k) as i32,
                },
                text.get(key),
                DT_RIGHT | DT_VCENTER,
            );
            SetTextColor(dc, theme::FOREGROUND);
        }
        let grid = CreatePen(PS_SOLID, 1, theme::GRID);
        let old = SelectObject(dc, HGDIOBJ(grid.0));
        for i in 0..=4 {
            let y = plot.top + (plot.bottom - plot.top) * i / 4;
            let _ = MoveToEx(dc, plot.left, y, None);
            let _ = LineTo(dc, plot.right, y);
            draw_text(
                dc,
                r.left,
                y - 7,
                &format!("{:.0}", max * (1.0 - i as f64 / 4.0)),
            );
        }
        SelectObject(dc, old);
        let _ = DeleteObject(HGDIOBJ(grid.0));
        for &metric in series {
            let color = if matches!(metric, 1 | 4 | 6) {
                theme::AMBER
            } else {
                theme::BLUE
            };
            let pen = CreatePen(PS_SOLID, (1.5 * k).max(1.0) as i32, color);
            let old = SelectObject(dc, HGDIOBJ(pen.0));
            let max_gap = ((1500.0 * (plot.right - plot.left) as f64 / (end - start).max(1) as f64)
                .ceil() as i32)
                .max(3);
            let mut path: Vec<POINT> = Vec::new();
            let draw = |path: &mut Vec<POINT>| {
                if path.len() > 1 {
                    let _ = Polyline(dc, path);
                } else if let Some(p) = path.first() {
                    let _ = SetPixelV(dc, p.x, p.y, color);
                }
                path.clear();
            };
            for (x, lo, hi) in envelope(
                points,
                metric,
                start,
                end,
                (plot.right - plot.left) as usize,
            ) {
                let x = plot.left + x as i32;
                let y = |v: f64| {
                    plot.bottom
                        - ((v / max).clamp(0.0, 1.0) * (plot.bottom - plot.top) as f64) as i32
                };
                if path.last().is_some_and(|p| x - p.x > max_gap) {
                    draw(&mut path);
                }
                path.push(POINT { x, y: y(lo) });
                if y(lo) != y(hi) {
                    path.push(POINT { x, y: y(hi) });
                }
            }
            draw(&mut path);
            SelectObject(dc, old);
            let _ = DeleteObject(HGDIOBJ(pen.0));
        }
        if let Some(header) = header {
            for trigger in &header.triggers {
                let flags = crate::assessment::reason_flag(&trigger.reason);
                if trigger.ts_ms < start
                    || trigger.ts_ms > end
                    || (flags != 0 && crate::assessment::severity(flags, series) == 0)
                {
                    continue;
                }
                let pen = CreatePen(
                    PS_DOT,
                    1,
                    if flags == 0 {
                        theme::AMBER
                    } else {
                        theme::DANGER
                    },
                );
                let old = SelectObject(dc, HGDIOBJ(pen.0));
                let x = plot.left
                    + ((trigger.ts_ms - start) as f64 / (end - start).max(1) as f64
                        * (plot.right - plot.left) as f64) as i32;
                let _ = MoveToEx(dc, x, plot.top, None);
                let _ = LineTo(dc, x, plot.bottom);
                SelectObject(dc, old);
                let _ = DeleteObject(HGDIOBJ(pen.0));
            }
        }
        let time = |ts| {
            if relative {
                return format!("{:.1} s", ts as f64 / 1000.0);
            }
            chrono::DateTime::from_timestamp_millis(ts as i64)
                .unwrap_or_default()
                .with_timezone(&chrono::Local)
                .format("%H:%M:%S")
                .to_string()
        };
        draw_text(dc, plot.left, plot.bottom + 3, &time(start));
        draw_text(
            dc,
            plot.right - (64.0 * k) as i32,
            plot.bottom + 3,
            &time(end),
        );
    }
}
unsafe extern "system" fn window_proc(hwnd: HWND, message: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        if message == WM_NCCREATE {
            let create = &*(lp.0 as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
        }
        let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State;
        if let Some(s) = ptr.as_mut() {
            match message {
                WM_DRAWITEM => {
                    let item = &*(lp.0 as *const DRAWITEMSTRUCT);
                    if item.CtlType == ODT_COMBOBOX {
                        s.theme.combo_item(item, s.font, s.dpi);
                        return LRESULT(1);
                    }
                }
                WM_MEASUREITEM => {
                    let item = &mut *(lp.0 as *mut MEASUREITEMSTRUCT);
                    if item.CtlType == ODT_COMBOBOX {
                        item.itemHeight = (28 * s.dpi / 96) as u32;
                        return LRESULT(1);
                    }
                }
                CONFIG_CHANGED => {
                    config_changed(hwnd, s);
                    return LRESULT(0);
                }
                WM_SIZE => {
                    layout(hwnd, s);
                    return LRESULT(0);
                }
                WM_GETMINMAXINFO => {
                    let info = &mut *(lp.0 as *mut MINMAXINFO);
                    info.ptMinTrackSize = POINT {
                        x: 940 * s.dpi / 96,
                        y: 780 * s.dpi / 96,
                    };
                    return LRESULT(0);
                }
                WM_DPICHANGED => {
                    s.dpi = (wp.0 & 0xffff) as i32;
                    let r = &*(lp.0 as *const RECT);
                    let _ = SetWindowPos(
                        hwnd,
                        None,
                        r.left,
                        r.top,
                        r.right - r.left,
                        r.bottom - r.top,
                        SWP_NOZORDER | SWP_NOACTIVATE,
                    );
                    layout(hwnd, s);
                    return LRESULT(0);
                }
                WM_TIMER => {
                    if let Some(receiver) = &s.importing
                        && let Ok(result) = receiver.try_recv()
                    {
                        s.importing = None;
                        let _ = EnableWindow(GetDlgItem(Some(hwnd), 301).unwrap_or_default(), true);
                        match result {
                            Ok(Some(path)) => {
                                load_incidents(s);
                                if let Some(index) = s.paths.iter().position(|p| *p == path) {
                                    send(s.list, LB_SETCURSEL, WPARAM(index), LPARAM(0));
                                    select_incident(s);
                                }
                            }
                            Ok(None) => {}
                            Err(e) => {
                                let message =
                                    wide(&format!("{}\n\n{e:#}", s.text.get("operation_failed")));
                                let _ = MessageBoxW(
                                    Some(hwnd),
                                    PCWSTR(message.as_ptr()),
                                    PCWSTR(wide(crate::branding::NAME).as_ptr()),
                                    MB_OK | MB_ICONERROR,
                                );
                            }
                        }
                    }
                    refresh(s);
                    if s.page == 2
                        && s.last_incident_refresh.elapsed() >= std::time::Duration::from_secs(5)
                    {
                        load_incidents(s);
                        s.last_incident_refresh = std::time::Instant::now();
                    }
                    let _ = InvalidateRect(Some(hwnd), None, false);
                    return LRESULT(0);
                }
                WM_ERASEBKGND => return LRESULT(1),
                WM_CTLCOLORSTATIC | WM_CTLCOLORBTN | WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX => {
                    let dc = HDC(wp.0 as *mut _);
                    SetBkMode(dc, TRANSPARENT);
                    SetTextColor(dc, theme::FOREGROUND);
                    let field = message == WM_CTLCOLOREDIT || message == WM_CTLCOLORLISTBOX;
                    SetBkColor(
                        dc,
                        if field {
                            theme::FIELD
                        } else {
                            theme::BACKGROUND
                        },
                    );
                    return LRESULT(if field {
                        s.theme.field.0
                    } else {
                        s.theme.background.0
                    } as isize);
                }
                WM_PAINT => {
                    paint(hwnd, s);
                    return LRESULT(0);
                }
                WM_COMMAND => {
                    match wp.0 & 0xffff {
                        2 => {
                            let _ = DestroyWindow(hwnd);
                        }
                        10..=13 => {
                            s.page = (wp.0 & 0xffff) - 10;
                            if s.page == 2 {
                                load_incidents(s);
                            }
                            refresh(s);
                            layout(hwnd, s);
                        }
                        20 => {
                            let _ = s.sender.try_send(Command::Mark);
                        }
                        21 => {
                            let _ = s.sender.try_send(Command::Pause);
                        }
                        301 if s.importing.is_none() => {
                            let config = s
                                .shared
                                .lock()
                                .unwrap_or_else(|p| p.into_inner())
                                .config
                                .clone();
                            let (tx, rx) = std::sync::mpsc::channel();
                            s.importing = Some(rx);
                            let owner = hwnd.0 as usize;
                            let _ = EnableWindow(HWND(lp.0 as *mut _), false);
                            std::thread::spawn(move || {
                                let _ = tx.send(import_mangohud(config, owner));
                            });
                        }
                        600..=606 => {
                            refresh(s);
                        }
                        612 => {
                            let display = draft_tray(s);
                            let _ = s.sender.try_send(Command::ApplyTray(display));
                        }
                        205 => {
                            open_details(hwnd, s);
                            refresh(s);
                        }
                        204 => {
                            if let Err(e) = apply(s) {
                                let text = wide(&format!("{e:#}"));
                                let _ = MessageBoxW(
                                    Some(hwnd),
                                    PCWSTR(text.as_ptr()),
                                    PCWSTR(wide(crate::branding::NAME).as_ptr()),
                                    MB_OK | MB_ICONERROR,
                                );
                            }
                        }
                        300 if (wp.0 >> 16) as u32 == LBN_SELCHANGE => {
                            select_incident(s);
                            let _ = InvalidateRect(Some(hwnd), None, false);
                        }
                        400..=406 => {
                            let id = wp.0 & 0xffff;
                            s.shown[id - 400] =
                                send(HWND(lp.0 as *mut _), BM_GETCHECK, WPARAM(0), LPARAM(0)).0
                                    == 1;
                            let _ = InvalidateRect(Some(hwnd), None, false);
                        }
                        _ => {}
                    }
                    return LRESULT(0);
                }
                WM_CLOSE => {
                    let _ = DestroyWindow(hwnd);
                    return LRESULT(0);
                }
                WM_NCDESTROY => {
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                    let _ = KillTimer(Some(hwnd), 1);
                    if IsWindow(Some(s.details_window)).as_bool() {
                        let _ = DestroyWindow(s.details_window);
                    }
                    drop(Box::from_raw(ptr));
                    return DefWindowProcW(hwnd, message, wp, lp);
                }
                _ => {}
            }
        }
        DefWindowProcW(hwnd, message, wp, lp)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn incidents_open_with_a_selection_and_keep_it_when_new_rows_arrive() {
        let a = PathBuf::from("incident-a.jsonl");
        let b = PathBuf::from("incident-b.jsonl");
        let new = PathBuf::from("incident-new.jsonl");
        assert_eq!(
            incident_selection(&[], &[a.clone(), b.clone()], -1),
            Some(0)
        );
        assert_eq!(
            incident_selection(
                &[a.clone(), b.clone()],
                &[new.clone(), a.clone(), b.clone()],
                1
            ),
            Some(2)
        );
        assert_eq!(incident_selection(&[a, b], &[new], 1), Some(0));
        assert_eq!(incident_selection(&[], &[], -1), None);
    }
    #[test]
    fn short_alerts_keep_their_color_and_do_not_bridge_quiet_time() {
        let points = vec![
            Point {
                ts: 1000,
                values: [None; 7],
                alerts: crate::assessment::WINDOW,
                span_ms: 50,
            },
            Point {
                ts: 9000,
                values: [None; 7],
                alerts: crate::assessment::CPU_BUSY,
                span_ms: 500,
            },
        ];
        let delay = alert_bands(&points, &[4], 0, 10_000, 100);
        assert_eq!(delay.len(), 1);
        assert_eq!(delay[0].2, 2);
        assert!(delay[0].1 - delay[0].0 >= 2);
        let load = alert_bands(&points, &[0], 0, 10_000, 100);
        assert_eq!(load.len(), 1);
        assert_eq!(load[0].2, 1);
        assert!(delay[0].1 < load[0].0);
        assert!(alert_bands(&points, &[1], 0, 10_000, 100).is_empty());
    }
    #[test]
    fn envelope_preserves_a_single_short_peak() {
        let points: Vec<_> = (0..1000)
            .map(|i| Point {
                ts: i,
                values: [Some(if i == 513 { 250.0 } else { 1.0 }); 7],
                alerts: 0,
                span_ms: 1,
            })
            .collect();
        let e = envelope(&points, 0, 0, 999, 10);
        assert_eq!(e.len(), 10);
        assert_eq!(e.iter().map(|v| v.2).max_by(f64::total_cmp), Some(250.0));
    }
}
