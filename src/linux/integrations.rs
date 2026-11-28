use super::daemon::Shared;
use super::*;
use futures_util::StreamExt;
use ksni::TrayMethods;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
static SLEEPING: AtomicBool = AtomicBool::new(false);
static LOCKED: AtomicBool = AtomicBool::new(false);
static SESSION_KNOWN: AtomicBool = AtomicBool::new(false);
pub fn interactive() -> Option<bool> {
    SESSION_KNOWN
        .load(Ordering::Relaxed)
        .then(|| !SLEEPING.load(Ordering::Relaxed) && !LOCKED.load(Ordering::Relaxed))
}
fn status(shared: &Shared, key: &str, value: &str) {
    shared
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .integrations
        .insert(key.into(), value.into());
}

pub fn start(config: PathBuf, shared: Shared, running: Arc<AtomicBool>) {
    let desktop = std::env::var_os("WAYLAND_DISPLAY").is_some();
    let xconfig = config.clone();
    let xshared = shared.clone();
    let xrunning = running.clone();
    if !desktop {
        std::thread::spawn(move || {
            while xrunning.load(Ordering::Relaxed) {
                let enabled = xshared
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .config
                    .linux
                    .global_shortcut;
                if enabled {
                    if x11_hotkey(&xconfig, &xshared, &xrunning).is_err() {
                        status(&xshared, "hotkey", "linux_shortcut_unavailable");
                    }
                } else {
                    status(&xshared, "hotkey", "disabled");
                }
                std::thread::sleep(Duration::from_secs(2));
            }
        });
    }
    std::thread::spawn(move || {
        let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        else {
            return;
        };
        runtime.block_on(async move {
            let session_state = shared.clone();
            tokio::spawn(async move {
                let _ = session(session_state).await;
            });
            if desktop {
                let path = config.clone();
                let state = shared.clone();
                let active = running.clone();
                tokio::spawn(async move {
                    while active.load(Ordering::Relaxed) {
                        let enabled = state
                            .lock()
                            .unwrap_or_else(|p| p.into_inner())
                            .config
                            .linux
                            .global_shortcut;
                        if enabled {
                            if portal_hotkey(&path, &state, &active).await.is_err() {
                                status(&state, "hotkey", "linux_shortcut_unavailable");
                            }
                        } else {
                            status(&state, "hotkey", "disabled");
                        }
                        tokio::time::sleep(Duration::from_secs(5)).await;
                    }
                });
            }
            while running.load(Ordering::Relaxed) {
                let enabled = shared.lock().unwrap_or_else(|p| p.into_inner()).config.tray;
                if !enabled {
                    status(&shared, "tray", "disabled");
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    continue;
                }
                match (Tray {
                    config: config.clone(),
                    state: shared.clone(),
                })
                .spawn()
                .await
                {
                    Ok(handle) => {
                        status(&shared, "tray", "active");
                        while running.load(Ordering::Relaxed)
                            && shared.lock().unwrap_or_else(|p| p.into_inner()).config.tray
                        {
                            handle.update(|_| {}).await;
                            tokio::time::sleep(Duration::from_secs(1)).await;
                        }
                        let _ = handle.shutdown().await;
                    }
                    Err(_) => {
                        status(&shared, "tray", "linux_tray_unavailable");
                        tokio::time::sleep(Duration::from_secs(5)).await;
                    }
                }
            }
        });
    });
}
struct Tray {
    config: PathBuf,
    state: Shared,
}
impl ksni::Tray for Tray {
    fn id(&self) -> String {
        "io.github.flochiii.StutterWatch".into()
    }
    fn title(&self) -> String {
        stutter_watch::branding::NAME.into()
    }
    fn icon_name(&self) -> String {
        "io.github.flochiii.StutterWatch".into()
    }
    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        let mut pixels = vec![0u8; 16 * 16 * 4];
        for (left, height) in [(2, 5), (7, 9), (12, 13)] {
            for x in left..left + 3 {
                for y in 15 - height..15 {
                    let i = (y * 16 + x) * 4;
                    pixels[i..i + 4].copy_from_slice(&[255, 106, 158, 216]);
                }
            }
        }
        vec![ksni::Icon {
            width: 16,
            height: 16,
            data: pixels,
        }]
    }
    fn tool_tip(&self) -> ksni::ToolTip {
        let state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        let text = stutter_watch::i18n::Text::new(&state.config.language);
        ksni::ToolTip {
            title: stutter_watch::branding::NAME.into(),
            description: stutter_watch::tray_display::tooltip(
                &state.config,
                &text,
                state.latest.as_ref(),
                state.paused,
                !state.recording.error.is_empty(),
            ),
            ..Default::default()
        }
    }
    fn activate(&mut self, _x: i32, _y: i32) {
        let _ = open_panel(&self.config);
    }
    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        let state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        let t = stutter_watch::i18n::Text::new(&state.config.language);
        let pause = t
            .get(if state.paused { "resume" } else { "pause" })
            .to_owned();
        drop(state);
        vec![
            ksni::menu::StandardItem {
                label: t.get("live").into(),
                activate: Box::new(|s| {
                    let _ = open_panel(&s.config);
                }),
                ..Default::default()
            }
            .into(),
            ksni::menu::StandardItem {
                label: t.get("mark").into(),
                activate: Box::new(|s| {
                    let _ = rpc(&s.config, "mark");
                }),
                ..Default::default()
            }
            .into(),
            ksni::menu::StandardItem {
                label: pause,
                activate: Box::new(|s| {
                    let _ = rpc(&s.config, "pause");
                }),
                ..Default::default()
            }
            .into(),
            ksni::MenuItem::Separator,
            ksni::menu::StandardItem {
                label: t.get("quit").into(),
                activate: Box::new(|s| {
                    let _ = rpc(&s.config, "shutdown");
                }),
                ..Default::default()
            }
            .into(),
        ]
    }
}
fn x11_hotkey(path: &Path, shared: &Shared, running: &AtomicBool) -> Result<()> {
    use x11rb::{
        connection::Connection,
        protocol::{
            Event,
            xproto::{ConnectionExt, GrabMode, ModMask},
        },
    };
    let config = shared
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .config
        .clone();
    let (mods, key) = stutter_watch::hotkey::parse_hotkey(&config.hotkey)?;
    let (conn, screen) = x11rb::connect(None)?;
    let root = conn.setup().roots[screen].root;
    let setup = conn.setup();
    let keys = conn
        .get_keyboard_mapping(setup.min_keycode, setup.max_keycode - setup.min_keycode + 1)?
        .reply()?;
    let symbol = if (0x70..=0x87).contains(&key) {
        0xffbe + key - 0x70
    } else {
        key
    };
    let code = keys
        .keysyms
        .chunks(keys.keysyms_per_keycode as usize)
        .position(|values| values.contains(&symbol))
        .map(|i| setup.min_keycode + i as u8)
        .context("Hotkey key is unavailable")?;
    let mut mask = ModMask::default();
    if mods & 1 != 0 {
        mask |= ModMask::M1;
    }
    if mods & 2 != 0 {
        mask |= ModMask::CONTROL;
    }
    if mods & 4 != 0 {
        mask |= ModMask::SHIFT;
    }
    if mods & 8 != 0 {
        mask |= ModMask::M4;
    }
    for extra in [
        ModMask::default(),
        ModMask::LOCK,
        ModMask::M2,
        ModMask::LOCK | ModMask::M2,
    ] {
        conn.grab_key(
            false,
            root,
            mask | extra,
            code,
            GrabMode::ASYNC,
            GrabMode::ASYNC,
        )?
        .check()?;
    }
    conn.flush()?;
    status(shared, "hotkey", "active");
    while running.load(Ordering::Relaxed) {
        let c = shared
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .config
            .clone();
        if !c.linux.global_shortcut || c.hotkey != config.hotkey {
            break;
        }
        if let Some(Event::KeyPress(_)) = conn.poll_for_event()? {
            let _ = rpc(path, "mark");
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    Ok(())
}
async fn portal_hotkey(path: &Path, shared: &Shared, running: &AtomicBool) -> Result<()> {
    use ashpd::desktop::global_shortcuts::{GlobalShortcuts, NewShortcut};
    let config = shared
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .config
        .clone();
    let portal = GlobalShortcuts::new().await?;
    let session = portal.create_session(Default::default()).await?;
    let shortcut = NewShortcut::new("mark", "Stutter Watch: mark stutter")
        .preferred_trigger(Some(config.hotkey.to_uppercase().as_str()));
    portal
        .bind_shortcuts(&session, &[shortcut], None, Default::default())
        .await?
        .response()?;
    let mut events = portal.receive_activated().await?;
    status(shared, "hotkey", "active");
    loop {
        tokio::select! {event=events.next()=>{if let Some(event)=event {if event.shortcut_id()=="mark"{let _=rpc(path,"mark");}}else{break;}},_=tokio::time::sleep(Duration::from_secs(1))=>{let c=shared.lock().unwrap_or_else(|p|p.into_inner()).config.clone();if !running.load(Ordering::Relaxed)||!c.linux.global_shortcut||c.hotkey!=config.hotkey{break;}}}
    }
    let _ = session.close().await;
    Ok(())
}
async fn session(shared: Shared) -> Result<()> {
    let connection = zbus::Connection::system().await?;
    let manager = zbus::Proxy::new(
        &connection,
        "org.freedesktop.login1",
        "/org/freedesktop/login1",
        "org.freedesktop.login1.Manager",
    )
    .await?;
    let path: zbus::zvariant::OwnedObjectPath = manager
        .call("GetSessionByPID", &(std::process::id(),))
        .await?;
    let proxy = zbus::Proxy::new(
        &connection,
        "org.freedesktop.login1",
        path,
        "org.freedesktop.login1.Session",
    )
    .await?;
    let mut sleep = manager.receive_signal("PrepareForSleep").await?;
    status(&shared, "session", "active");
    loop {
        if let Ok(locked) = proxy.get_property::<bool>("LockedHint").await {
            LOCKED.store(locked, Ordering::Relaxed);
            SESSION_KNOWN.store(true, Ordering::Relaxed);
        }
        tokio::select! {message=sleep.next()=>{if let Some(message)=message {if let Ok((sleeping,))=message.body().deserialize::<(bool,)>(){SLEEPING.store(sleeping,Ordering::Relaxed);}}else{break;}},_=tokio::time::sleep(Duration::from_secs(1))=>{}}
    }
    Ok(())
}
