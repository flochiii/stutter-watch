//! Per-user installer lifecycle. Never deletes configuration or recordings.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};
use windows::Win32::{
    Foundation::STILL_ACTIVE,
    System::Threading::{GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION},
};

#[derive(Serialize, Deserialize)]
struct Receipt {
    directory: PathBuf,
    was_running: bool,
    scheduled: bool,
    had_binary: bool,
}

fn canonical(path: &Path) -> Result<PathBuf> {
    Ok(path.canonicalize()?)
}
fn stop(config: &Path) -> Result<bool> {
    let endpoint = config.with_file_name("config.toml.instance.json");
    let Some(pid) = std::fs::read(&endpoint)
        .ok()
        .and_then(|v| serde_json::from_slice::<serde_json::Value>(&v).ok())
        .and_then(|v| v.get("pid")?.as_u64())
    else {
        return Ok(false);
    };
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid as u32) }
        .context("Wait for the existing recorder")?;
    let deadline = Instant::now() + Duration::from_secs(30);
    let result = (|| {
        // Hold the process handle before signaling: a quick clean exit must not
        // race OpenProcess. signal validates HWND, PID and window class.
        if crate::tray::signal(config, true).is_err() {
            return Ok(false);
        }
        loop {
            let mut code = 0;
            unsafe { GetExitCodeProcess(handle, &mut code) }?;
            if code != STILL_ACTIVE.0 as u32 {
                return Ok(true);
            }
            ensure!(
                Instant::now() < deadline,
                "Close Stutter Watch before continuing; the recorder has not finished saving"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
    })();
    unsafe {
        let _ = windows::Win32::Foundation::CloseHandle(handle);
    }
    result
}
fn wait_ready(config: &Path) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let ready = std::fs::read(config.with_file_name("config.toml.startup.json"))
            .ok()
            .and_then(|v| serde_json::from_slice::<serde_json::Value>(&v).ok());
        if let Some(value) = ready
            && value["events"]
                .as_array()
                .is_some_and(|events| events.iter().any(|e| e["phase"] == "recorder.running"))
            && let Some(pid) = value["pid"].as_u64()
            && let Ok(handle) =
                unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid as u32) }
        {
            let mut code = 0;
            let active = unsafe { GetExitCodeProcess(handle, &mut code) }.is_ok()
                && code == STILL_ACTIVE.0 as u32;
            unsafe {
                let _ = windows::Win32::Foundation::CloseHandle(handle);
            }
            if active {
                return Ok(());
            }
        }
        ensure!(
            Instant::now() < deadline,
            "Recorder did not restart after the update"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}
pub fn execute(operation: &str, directory: &Path, receipt_path: &Path) -> Result<()> {
    ensure!(
        directory.is_absolute(),
        "Installation directory must be absolute"
    );
    std::fs::create_dir_all(directory)?;
    let directory = canonical(directory)?;
    let exe = directory.join("stutter-watch.exe");
    let config = directory.join("config.toml");
    match operation {
        "prepare" => {
            ensure!(!receipt_path.exists(), "Update receipt already exists");
            let scheduled = crate::autostart::owned(&exe, &config);
            let was_running = stop(&config)?;
            let had_binary = exe.exists();
            if had_binary {
                std::fs::copy(&exe, receipt_path.with_extension("previous.exe"))?;
            }
            // The installer owns a fresh private temporary directory.
            let state = Receipt {
                directory,
                was_running,
                scheduled,
                had_binary,
            };
            std::fs::write(receipt_path, serde_json::to_vec(&state)?)?;
        }
        "resume" | "rollback" => {
            let state: Receipt = serde_json::from_slice(&std::fs::read(receipt_path)?)?;
            ensure!(
                state.directory == directory,
                "Receipt belongs to another installation"
            );
            if operation == "rollback" && state.had_binary {
                stop(&config)?;
                std::fs::copy(receipt_path.with_extension("previous.exe"), &exe)?;
            }
            if state.was_running {
                // Do not accept a diagnostic left by a previous process as proof
                // that the replacement successfully initialized.
                let _ = std::fs::remove_file(config.with_file_name("config.toml.startup.json"));
                if state.scheduled && crate::autostart::owned(&exe, &config) {
                    crate::autostart::start()?;
                } else {
                    use std::os::windows::process::CommandExt;
                    Command::new(&exe)
                        .arg("--config")
                        .arg(&config)
                        .arg("run")
                        .creation_flags(0x08000000)
                        .stdin(Stdio::null())
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .spawn()?;
                }
                wait_ready(&config)?;
            }
        }
        "uninstall" => {
            stop(&config)?;
            if crate::autostart::owned(&exe, &config) {
                crate::autostart::remove()?;
            }
        }
        _ => anyhow::bail!("Unknown maintenance operation"),
    }
    Ok(())
}
