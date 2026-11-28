#![cfg_attr(all(windows, not(test)), windows_subsystem = "windows")]

pub use stutter_watch::{
    assessment, branding, capacity, config, dashboard, detector, history, i18n, model, recorder,
    report, startup, storage, tray_display,
};
#[cfg(windows)]
mod autostart;
#[cfg(windows)]
mod collectors;
#[cfg(windows)]
mod identity;
#[cfg(windows)]
mod maintenance;
#[cfg(windows)]
mod panel;
#[cfg(windows)]
mod sources;
#[cfg(windows)]
mod theme;
#[cfg(windows)]
mod tray;
#[cfg(windows)]
mod tray_popup;
#[cfg(windows)]
mod windows_app;

#[cfg(target_os = "linux")]
mod linux;

fn main() -> anyhow::Result<()> {
    #[cfg(windows)]
    return windows_app::main();
    #[cfg(target_os = "linux")]
    return linux::main();
}
