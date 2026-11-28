use crate::collectors::pdh::wide;
use anyhow::{Result, bail, ensure};
use std::{
    mem::size_of,
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread::{self, JoinHandle},
};
use windows::{
    Win32::{
        Foundation::*,
        Graphics::Gdi::*,
        System::LibraryLoader::GetModuleHandleW,
        UI::{Input::KeyboardAndMouse::*, Shell::*, WindowsAndMessaging::*},
    },
    core::{BOOL, PCWSTR, w},
};

const NOTIFY: u32 = WM_APP + 1;
const UPDATE: u32 = WM_APP + 2;
const OPEN_PANEL: u32 = WM_APP + 3;
const STOP_RECORDER: u32 = WM_APP + 4;
#[derive(serde::Serialize, serde::Deserialize)]
struct Endpoint {
    pid: u32,
    hwnd: usize,
}
fn endpoint_path(config: &Path) -> std::path::PathBuf {
    let mut name = config.file_name().unwrap_or_default().to_os_string();
    name.push(".instance.json");
    config.with_file_name(name)
}
pub fn signal(config: &Path, shutdown: bool) -> Result<()> {
    let state: Endpoint = serde_json::from_slice(&std::fs::read(endpoint_path(config))?)?;
    let hwnd = HWND(state.hwnd as *mut _);
    unsafe {
        ensure!(
            IsWindow(Some(hwnd)).as_bool(),
            "Recorder window no longer exists"
        );
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        ensure!(
            pid == state.pid,
            "Recorder endpoint belongs to another process"
        );
        let mut class = [0u16; 128];
        let length = GetClassNameW(hwnd, &mut class).max(0) as usize;
        ensure!(
            String::from_utf16_lossy(&class[..length]) == format!("stutter-watch-{}", state.pid),
            "Invalid recorder endpoint"
        );
        PostMessageW(
            Some(hwnd),
            if shutdown { STOP_RECORDER } else { OPEN_PANEL },
            WPARAM(0),
            LPARAM(0),
        )?;
    }
    Ok(())
}
#[derive(Debug)]
pub enum Command {
    Apply(Box<crate::config::Config>),
    ApplyTray(crate::config::TrayDisplay),
    Pause,
    Mark,
    Report,
    OpenLogs,
    Autostart,
    Quit,
    Warning(String),
}

struct Context {
    dashboard: Arc<Mutex<crate::panel::Dashboard>>,
    display_language: String,
    display_text: crate::i18n::Text,
    panel: HWND,
    sender: SyncSender<Command>,
    paused: Arc<AtomicBool>,
    autostart: Arc<AtomicBool>,
    notification: Arc<Mutex<String>>,
    warning: Arc<AtomicBool>,
    icons: Icons,
    icon: NOTIFYICONDATAW,
    tooltip_text: String,
    popup: Option<crate::tray_popup::Popup>,
    visible: bool,
    hotkey: bool,
    icon_error_reported: bool,
    registered: bool,
    taskbar_created: u32,
}

pub struct Ui {
    endpoint: Option<std::path::PathBuf>,
    hwnd: usize,
    thread: Option<JoinHandle<()>>,
    notification: Arc<Mutex<String>>,
    warning: Arc<AtomicBool>,
    pub paused: Arc<AtomicBool>,
    pub autostart: Arc<AtomicBool>,
}
impl Ui {
    pub fn start(
        dashboard: Arc<Mutex<crate::panel::Dashboard>>,
        visible: bool,
        hotkey: &str,
        autostart: bool,
        warning: bool,
    ) -> Result<(Self, Receiver<Command>)> {
        let (mods, key) = parse_hotkey(hotkey)?;
        let (sender, receiver) = mpsc::sync_channel(64);
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let paused = Arc::new(AtomicBool::new(false));
        let autostart = Arc::new(AtomicBool::new(autostart));
        let notification = Arc::new(Mutex::new(String::new()));
        let warning = Arc::new(AtomicBool::new(warning));
        let (thread_notification, thread_warning) =
            (Arc::clone(&notification), Arc::clone(&warning));
        let (thread_paused, thread_auto) = (Arc::clone(&paused), Arc::clone(&autostart));
        let thread = thread::Builder::new()
            .name("tray".into())
            .stack_size(256 * 1024)
            .spawn(move || {
                let result = (|| -> Result<()> {
                    unsafe {
                        let instance = HINSTANCE(GetModuleHandleW(None)?.0);
                        let icons = Icons::new()?;
                        let class = wide(&format!("stutter-watch-{}", std::process::id()));
                        let wc = WNDCLASSW {
                            lpfnWndProc: Some(window_proc),
                            hInstance: instance,
                            hIcon: icons.active.0,
                            lpszClassName: PCWSTR(class.as_ptr()),
                            ..Default::default()
                        };
                        ensure!(RegisterClassW(&wc) != 0, "RegisterClassW failed");
                        let mut context = Box::new(Context {
                            dashboard,
                            display_language: String::new(),
                            display_text: crate::i18n::Text::new("en"),
                            panel: HWND::default(),
                            sender,
                            paused: thread_paused,
                            autostart: thread_auto,
                            notification: thread_notification,
                            warning: thread_warning,
                            icons,
                            icon: NOTIFYICONDATAW::default(),
                            tooltip_text: String::new(),
                            popup: None,
                            visible,
                            hotkey: false,
                            icon_error_reported: false,
                            registered: false,
                            taskbar_created: RegisterWindowMessageW(w!("TaskbarCreated")),
                        });
                        let hwnd = CreateWindowExW(
                            WINDOW_EX_STYLE::default(),
                            PCWSTR(class.as_ptr()),
                            PCWSTR(wide(crate::branding::NAME).as_ptr()),
                            WINDOW_STYLE::default(),
                            0,
                            0,
                            0,
                            0,
                            None,
                            None,
                            Some(instance),
                            Some((&mut *context as *mut Context).cast()),
                        )?;
                        if let Err(error) = crate::identity::set_window_id(hwnd) {
                            crate::startup::phase(&format!("tray.identity_error: {error}"));
                        }
                        if visible {
                            let _ = SetTimer(Some(hwnd), 1, 1000, None);
                        }
                        // Accept Explorer's tray callbacks plus two argument-free
                        // launcher commands. WM_COMMAND stays filtered.
                        let _ = ChangeWindowMessageFilterEx(hwnd, NOTIFY, MSGFLT_ALLOW, None);
                        let _ = ChangeWindowMessageFilterEx(
                            hwnd,
                            context.taskbar_created,
                            MSGFLT_ALLOW,
                            None,
                        );
                        let _ = ChangeWindowMessageFilterEx(hwnd, OPEN_PANEL, MSGFLT_ALLOW, None);
                        let _ =
                            ChangeWindowMessageFilterEx(hwnd, STOP_RECORDER, MSGFLT_ALLOW, None);
                        context.icon = NOTIFYICONDATAW {
                            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
                            hWnd: hwnd,
                            uID: 1,
                            uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
                            uCallbackMessage: NOTIFY,
                            hIcon: context.icons.active.0,
                            ..Default::default()
                        };
                        // Updating an EXE at the same path can leave Shell's
                        // notification header using a cached, older app icon.
                        if let Ok(exe) = std::env::current_exe() {
                            let path = wide(&exe.to_string_lossy());
                            SHChangeNotify(
                                SHCNE_UPDATEITEM,
                                SHCNF_PATHW,
                                Some(path.as_ptr().cast()),
                                None,
                            );
                        }
                        update_icon(&mut context, NIM_ADD);
                        match RegisterHotKey(
                            Some(hwnd),
                            1,
                            HOT_KEY_MODIFIERS(mods) | MOD_NOREPEAT,
                            key,
                        ) {
                            Ok(()) => context.hotkey = true,
                            Err(e) => {
                                let _ = context.sender.try_send(Command::Warning(format!(
                                    "Global hotkey unavailable: {e}"
                                )));
                            }
                        }
                        let _ = ready_tx.send(Ok(hwnd.0 as usize));
                        let mut message = MSG::default();
                        while GetMessageW(&mut message, None, 0, 0).0 > 0 {
                            if IsWindow(Some(context.panel)).as_bool()
                                && (message.hwnd == context.panel
                                    || IsChild(context.panel, message.hwnd).as_bool())
                                && IsDialogMessageW(context.panel, &message).as_bool()
                            {
                                continue;
                            }
                            let _ = TranslateMessage(&message);
                            DispatchMessageW(&message);
                        }
                        let _ = UnregisterClassW(PCWSTR(class.as_ptr()), Some(instance));
                    }
                    Ok(())
                })();
                if let Err(error) = result {
                    let _ = ready_tx.send(Err(format!("{error:#}")));
                }
            })?;
        let hwnd = ready_rx
            .recv()
            .map_err(|_| anyhow::anyhow!("Tray thread exited during startup"))?
            .map_err(anyhow::Error::msg)?;
        Ok((
            Self {
                endpoint: None,
                hwnd,
                thread: Some(thread),
                notification,
                warning,
                paused,
                autostart,
            },
            receiver,
        ))
    }
    pub fn notify(&self, message: &str) {
        *self.notification.lock().unwrap_or_else(|p| p.into_inner()) = message.to_string();
        unsafe {
            let _ = PostMessageW(
                Some(HWND(self.hwnd as *mut _)),
                UPDATE,
                WPARAM(1),
                LPARAM(0),
            );
        }
    }
    pub fn publish_endpoint(&mut self, config: &Path) -> Result<()> {
        let path = endpoint_path(config);
        let temp = path.with_extension("tmp");
        std::fs::write(
            &temp,
            serde_json::to_vec(&Endpoint {
                pid: std::process::id(),
                hwnd: self.hwnd,
            })?,
        )?;
        std::fs::rename(temp, &path)?;
        self.endpoint = Some(path);
        Ok(())
    }
    pub fn open_panel(&self) {
        unsafe {
            let _ = PostMessageW(
                Some(HWND(self.hwnd as *mut _)),
                OPEN_PANEL,
                WPARAM(0),
                LPARAM(0),
            );
        }
    }
    pub fn rebuild_panel(&self) {
        unsafe {
            let _ = PostMessageW(
                Some(HWND(self.hwnd as *mut _)),
                OPEN_PANEL,
                WPARAM(1),
                LPARAM(0),
            );
        }
    }
    pub fn set_warning(&self, warning: bool) {
        if self.warning.swap(warning, Ordering::Relaxed) == warning {
            return;
        }
        unsafe {
            let _ = PostMessageW(
                Some(HWND(self.hwnd as *mut _)),
                UPDATE,
                WPARAM(0),
                LPARAM(0),
            );
        }
    }
}
impl Drop for Ui {
    fn drop(&mut self) {
        unsafe {
            let _ = PostMessageW(
                Some(HWND(self.hwnd as *mut _)),
                WM_CLOSE,
                WPARAM(0),
                LPARAM(0),
            );
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        if let Some(path) = self.endpoint.take() {
            let _ = std::fs::remove_file(path);
        }
    }
}

fn copy_wide<const N: usize>(target: &mut [u16; N], text: &str) {
    target.fill(0);
    for (slot, unit) in target.iter_mut().take(N - 1).zip(text.encode_utf16()) {
        *slot = unit;
    }
}
fn update_icon(context: &mut Context, action: NOTIFY_ICON_MESSAGE) {
    // Explorer may not exist yet at sign-in. A failed ADD must be retried as
    // ADD, rather than silently attempting MODIFY on an icon that never existed.
    let action = if action == NIM_MODIFY && !context.registered {
        NIM_ADD
    } else {
        action
    };
    if !context.visible {
        return;
    }
    let paused = context.paused.load(Ordering::Relaxed);
    let warning = context.warning.load(Ordering::Relaxed);
    let next_icon = if paused {
        context.icons.paused.0
    } else if warning {
        context.icons.warning.0
    } else {
        context.icons.active.0
    };
    let tooltip = {
        let dashboard = context.dashboard.lock().unwrap_or_else(|p| p.into_inner());
        if context.display_language != dashboard.config.language {
            context
                .display_language
                .clone_from(&dashboard.config.language);
            context.display_text = crate::i18n::Text::new(&context.display_language);
        }
        crate::tray_display::tooltip(
            &dashboard.config,
            &context.display_text,
            dashboard.latest.as_deref(),
            paused,
            warning,
        )
    };
    let mut next_tip = [0u16; 128];
    context.tooltip_text = tooltip;
    if let Some(popup) = context.popup.as_mut() {
        popup.update(&context.tooltip_text);
    }
    // Keep Shell's accessible icon name stable. Live numbers belong to our
    // persistent popup; modifying szTip repeatedly closes Shell's native tooltip.
    copy_wide(&mut next_tip, crate::branding::NAME);
    if action == NIM_MODIFY
        && context.icon.hIcon == next_icon
        && context.icon.szTip == next_tip
        && context.icon.uFlags.0 & NIF_INFO.0 == 0
    {
        return;
    }
    context.icon.hIcon = next_icon;
    context.icon.szTip = next_tip;
    unsafe {
        let success = Shell_NotifyIconW(action, &context.icon).as_bool();
        context.registered = success;
        if success && action == NIM_ADD {
            crate::startup::phase("tray.icon_added");
            context.icon.Anonymous.uVersion = NOTIFYICON_VERSION_4;
            let _ = Shell_NotifyIconW(NIM_SETVERSION, &context.icon);
            context.icon_error_reported = false;
        }
        if !success && !context.icon_error_reported {
            crate::startup::phase("tray.registration_pending");
            context.icon_error_reported = true;
            let _ = context
                .sender
                .try_send(Command::Warning("System tray icon unavailable".into()));
        }
    }
}
fn menu(hwnd: HWND, context: &Context) {
    unsafe {
        let language = context
            .dashboard
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .config
            .language
            .clone();
        let text = crate::i18n::Text::new(&language);
        let Ok(menu) = CreatePopupMenu() else {
            return;
        };
        let name = wide(crate::branding::NAME);
        let _ = AppendMenuW(menu, MF_STRING | MF_DISABLED, 0, PCWSTR(name.as_ptr()));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        for (id, label, flags) in [
            (7, text.get("show"), MF_STRING),
            (
                1,
                if context.paused.load(Ordering::Relaxed) {
                    text.get("resume")
                } else {
                    text.get("pause")
                },
                MF_STRING,
            ),
            (2, text.get("mark"), MF_STRING),
            (3, text.get("report"), MF_STRING),
            (4, text.get("logs"), MF_STRING),
            (
                5,
                text.get("autostart"),
                if context.autostart.load(Ordering::Relaxed) {
                    MF_STRING | MF_CHECKED
                } else {
                    MF_STRING
                },
            ),
            (6, text.get("quit"), MF_STRING),
        ] {
            let text = wide(label);
            let _ = AppendMenuW(menu, flags, id, PCWSTR(text.as_ptr()));
        }
        let mut point = POINT::default();
        let _ = GetCursorPos(&mut point);
        let _ = SetForegroundWindow(hwnd);
        let selected = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON,
            point.x,
            point.y,
            None,
            hwnd,
            None,
        )
        .0;
        let command = match selected {
            7 => {
                let _ = PostMessageW(Some(hwnd), OPEN_PANEL, WPARAM(0), LPARAM(0));
                None
            }
            1 => Some(Command::Pause),
            2 => Some(Command::Mark),
            3 => Some(Command::Report),
            4 => Some(Command::OpenLogs),
            5 => Some(Command::Autostart),
            6 => Some(Command::Quit),
            _ => None,
        };
        if let Some(command) = command {
            let _ = context.sender.try_send(command);
        }
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
    }
}
unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // SAFETY: Context is boxed for the whole message loop; WM_NCCREATE installs its pointer.
    unsafe {
        if message == WM_NCCREATE {
            let create = &*(lparam.0 as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
        }
        let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Context;
        if let Some(context) = ptr.as_mut() {
            if message == context.taskbar_created {
                context.popup = None;
                context.registered = false;
                update_icon(context, NIM_ADD);
                return LRESULT(0);
            }
            match message {
                WM_TIMER => {
                    update_icon(context, NIM_MODIFY);
                    return LRESULT(0);
                }
                STOP_RECORDER => {
                    let _ = context.sender.try_send(Command::Quit);
                    return LRESULT(0);
                }
                WM_QUERYENDSESSION => {
                    return LRESULT(1);
                }
                WM_ENDSESSION if wparam.0 != 0 => {
                    let _ = context.sender.try_send(Command::Quit);
                    return LRESULT(0);
                }
                WM_COMMAND => {
                    if wparam.0 & 0xffff == 7 {
                        let _ = PostMessageW(Some(hwnd), OPEN_PANEL, WPARAM(0), LPARAM(0));
                        return LRESULT(0);
                    }
                    let command = match wparam.0 & 0xffff {
                        1 => Some(Command::Pause),
                        2 => Some(Command::Mark),
                        3 => Some(Command::Report),
                        4 => Some(Command::OpenLogs),
                        5 => Some(Command::Autostart),
                        6 => Some(Command::Quit),
                        _ => None,
                    };
                    if let Some(command) = command {
                        let _ = context.sender.try_send(command);
                    }
                    return LRESULT(0);
                }
                WM_HOTKEY => {
                    let _ = context.sender.try_send(Command::Mark);
                    return LRESULT(0);
                }
                NOTIFY => {
                    if (lparam.0 as u32 >> 16) != context.icon.uID {
                        return LRESULT(0);
                    }
                    let event = lparam.0 as u32 & 0xffff;
                    if event == NIN_POPUPOPEN {
                        let identifier = NOTIFYICONIDENTIFIER {
                            cbSize: size_of::<NOTIFYICONIDENTIFIER>() as u32,
                            hWnd: hwnd,
                            uID: 1,
                            ..Default::default()
                        };
                        let anchor = Shell_NotifyIconGetRect(&identifier).unwrap_or_else(|_| {
                            let x = wparam.0 as u16 as i16 as i32;
                            let y = (wparam.0 >> 16) as u16 as i16 as i32;
                            RECT {
                                left: x,
                                top: y,
                                right: x + 1,
                                bottom: y + 1,
                            }
                        });
                        if context.popup.is_none() {
                            context.popup = crate::tray_popup::Popup::new(
                                Some(hwnd),
                                anchor,
                                &context.tooltip_text,
                            )
                            .ok();
                        }
                        if let Some(popup) = &context.popup {
                            popup.show();
                        }
                    } else if event == NIN_POPUPCLOSE {
                        context.popup = None;
                    } else if matches!(event, WM_RBUTTONUP | WM_CONTEXTMENU) {
                        context.popup = None;
                        menu(hwnd, context);
                    } else if matches!(event, WM_LBUTTONDBLCLK | NIN_SELECT)
                        || event == (NIN_SELECT | 1)
                    {
                        context.popup = None;
                        let _ = PostMessageW(Some(hwnd), OPEN_PANEL, WPARAM(0), LPARAM(0));
                    }
                    return LRESULT(0);
                }
                UPDATE => {
                    let message = context
                        .notification
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .clone();
                    copy_wide(&mut context.icon.szInfo, &message);
                    copy_wide(&mut context.icon.szInfoTitle, crate::branding::NAME);
                    // A notification has its own icon, independently of hIcon.
                    // Use the product mark instead of Windows' stock blue "i".
                    context.icon.hBalloonIcon = context.icons.active.0;
                    context.icon.dwInfoFlags = NIIF_USER | NIIF_LARGE_ICON;
                    if wparam.0 == 1 {
                        context.icon.uFlags |= NIF_INFO;
                    }
                    update_icon(context, NIM_MODIFY);
                    context.icon.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
                    return LRESULT(0);
                }
                OPEN_PANEL => {
                    if wparam.0 == 1 && IsWindow(Some(context.panel)).as_bool() {
                        let _ = PostMessageW(
                            Some(context.panel),
                            crate::panel::CONFIG_CHANGED,
                            WPARAM(0),
                            LPARAM(0),
                        );
                        return LRESULT(0);
                    }
                    if IsWindow(Some(context.panel)).as_bool() {
                        let _ = ShowWindow(context.panel, SW_RESTORE);
                        let _ = SetForegroundWindow(context.panel);
                    } else {
                        match crate::panel::open(
                            Arc::clone(&context.dashboard),
                            context.sender.clone(),
                        ) {
                            Ok(panel) => context.panel = panel,
                            Err(e) => {
                                let _ = context
                                    .sender
                                    .try_send(Command::Warning(format!("Panel: {e:#}")));
                            }
                        }
                    }
                    return LRESULT(0);
                }
                WM_CLOSE => {
                    let _ = DestroyWindow(hwnd);
                    return LRESULT(0);
                }
                WM_DESTROY => {
                    context.popup = None;
                    if IsWindow(Some(context.panel)).as_bool() {
                        let _ = DestroyWindow(context.panel);
                    }
                    if context.visible {
                        let _ = Shell_NotifyIconW(NIM_DELETE, &context.icon);
                    }
                    if context.hotkey {
                        let _ = UnregisterHotKey(Some(hwnd), 1);
                    }
                    PostQuitMessage(0);
                    return LRESULT(0);
                }
                _ => {}
            }
        }
        DefWindowProcW(hwnd, message, wparam, lparam)
    }
}

struct OwnedIcon(HICON);
impl Drop for OwnedIcon {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyIcon(self.0);
        }
    }
}
struct Icons {
    active: OwnedIcon,
    paused: OwnedIcon,
    warning: OwnedIcon,
}
impl Icons {
    fn new() -> Result<Self> {
        Ok(Self {
            active: make_icon([106, 158, 216])?,
            paused: make_icon([148, 163, 184])?,
            warning: make_icon([251, 191, 36])?,
        })
    }
}

fn make_icon(badge: [u8; 3]) -> Result<OwnedIcon> {
    const SIZE: usize = 32;
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: SIZE as i32,
            biHeight: -(SIZE as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut pixels = std::ptr::null_mut();
    let bitmap = unsafe { CreateDIBSection(None, &info, DIB_RGB_COLORS, &mut pixels, None, 0) }?;
    let mask_bits = [0xffu8; SIZE * SIZE / 8];
    let mask = unsafe {
        CreateBitmap(
            SIZE as i32,
            SIZE as i32,
            1,
            1,
            Some(mask_bits.as_ptr().cast()),
        )
    };
    if mask.is_invalid() {
        unsafe {
            let _ = DeleteObject(HGDIOBJ(bitmap.0));
        }
        bail!("Cannot create tray icon mask");
    }
    // Three bars, one flat color: the same small silhouette as assets/app.svg.
    let output = unsafe { std::slice::from_raw_parts_mut(pixels.cast::<u8>(), SIZE * SIZE * 4) };
    for y in 0..SIZE {
        for x in 0..SIZE {
            let (px, py) = ((x as f64 + 0.5) * 1.5, (y as f64 + 0.5) * 1.5);
            let visible = [(9.0, 16.0, 24.0), (21.0, 28.0, 9.0), (33.0, 40.0, 18.0)]
                .iter()
                .any(|&(left, right, top)| px >= left && px < right && py >= top && py < 39.0);
            let offset = (y * SIZE + x) * 4;
            if visible {
                output[offset..offset + 4].copy_from_slice(&[badge[2], badge[1], badge[0], 255]);
            } else {
                output[offset..offset + 4].fill(0);
            }
        }
    }
    let icon = unsafe {
        CreateIconIndirect(&ICONINFO {
            fIcon: BOOL(1),
            hbmMask: mask,
            hbmColor: bitmap,
            ..Default::default()
        })
    };
    unsafe {
        let _ = DeleteObject(HGDIOBJ(mask.0));
        let _ = DeleteObject(HGDIOBJ(bitmap.0));
    }
    Ok(OwnedIcon(icon?))
}

pub fn open(path: &Path) -> Result<()> {
    let path = wide(&path.canonicalize()?.to_string_lossy());
    let result = unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            PCWSTR(path.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    ensure!(result.0 as usize > 32, "Could not open file/folder");
    Ok(())
}

pub use stutter_watch::hotkey::parse_hotkey;
