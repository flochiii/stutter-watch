#![cfg_attr(all(windows, not(test)), windows_subsystem = "windows")]

pub use stutter_watch::{
    assessment, branding, capacity, config, dashboard, detector, history, i18n, model, recorder,
    report, startup, storage,
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
mod tray_display;
#[cfg(windows)]
mod tray_popup;
#[cfg(windows)]
mod windows_app;

fn main() -> anyhow::Result<()> {
    #[cfg(windows)]
    return windows_app::main();
    #[cfg(not(windows))]
    anyhow::bail!("The desktop backend is not available for this target")
}
