use anyhow::{Context, Result, bail};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};
use windows::Win32::System::SystemInformation::GetSystemDirectoryW;

fn system_exe(name: &str) -> PathBuf {
    let mut buffer = [0u16; 512];
    let count = unsafe { GetSystemDirectoryW(Some(&mut buffer)) } as usize;
    PathBuf::from(String::from_utf16_lossy(&buffer[..count.min(buffer.len())])).join(name)
}
fn command(name: &str) -> Command {
    use std::os::windows::process::CommandExt;
    let mut command = Command::new(system_exe(name));
    command.creation_flags(0x08000000);
    command.stdin(Stdio::null());
    command
}
fn sid() -> Result<String> {
    let result = command("whoami.exe")
        .args(["/user", "/fo", "csv", "/nh"])
        .output()?;
    let output = String::from_utf8_lossy(&result.stdout);
    let value = output
        .trim()
        .split(',')
        .next_back()
        .unwrap_or("")
        .trim_matches('"')
        .to_string();
    if !result.status.success() || !value.starts_with("S-1-") {
        bail!("Unable to determine current user SID");
    }
    Ok(value)
}
fn task_name() -> Result<String> {
    Ok(format!("stutter-watch-{}", sid()?))
}
pub fn enabled() -> bool {
    task_name()
        .ok()
        .and_then(|task| {
            command("schtasks.exe")
                .args(["/Query", "/TN", &task])
                .output()
                .ok()
        })
        .is_some_and(|r| r.status.success())
}
fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
fn task_xml(exe: &Path, config: &Path, user: &str, elevated: bool) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.4" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
<RegistrationInfo><Description>stutter-watch performance flight recorder</Description></RegistrationInfo>
<Triggers><LogonTrigger><Enabled>true</Enabled><UserId>{user}</UserId></LogonTrigger></Triggers>
<Principals><Principal id="User"><UserId>{user}</UserId><LogonType>InteractiveToken</LogonType><RunLevel>{level}</RunLevel></Principal></Principals>
<Settings><MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy><DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries><StopIfGoingOnBatteries>false</StopIfGoingOnBatteries><StartWhenAvailable>true</StartWhenAvailable><RunOnlyIfNetworkAvailable>false</RunOnlyIfNetworkAvailable><AllowStartOnDemand>true</AllowStartOnDemand><Enabled>true</Enabled><Hidden>true</Hidden><ExecutionTimeLimit>PT0S</ExecutionTimeLimit><Priority>7</Priority><RestartOnFailure><Interval>PT1M</Interval><Count>3</Count></RestartOnFailure></Settings>
<Actions Context="User"><Exec><Command>{exe}</Command><Arguments>--config &quot;{config}&quot; run</Arguments><WorkingDirectory>{workdir}</WorkingDirectory></Exec></Actions>
</Task>"#,
        user = xml_escape(user),
        level = if elevated {
            "HighestAvailable"
        } else {
            "LeastPrivilege"
        },
        exe = xml_escape(&exe.to_string_lossy()),
        config = xml_escape(&config.to_string_lossy()),
        workdir = xml_escape(&config.parent().unwrap_or(Path::new(".")).to_string_lossy())
    )
}
pub fn install(config: &Path, elevated: bool) -> Result<()> {
    let exe = std::env::current_exe()?.canonicalize()?;
    let config = config.canonicalize()?;
    // Extended-length prefixes are understood by Win32 but not all Task Scheduler versions.
    let normalize =
        |p: PathBuf| PathBuf::from(p.to_string_lossy().trim_start_matches(r"\\?\").to_string());
    let xml = task_xml(&normalize(exe), &normalize(config), &sid()?, elevated);
    let path = std::env::temp_dir().join(format!(
        "stutter-watch-task-{}-{}.xml",
        std::process::id(),
        crate::model::now_ms()
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?;
    file.write_all(&task_xml_bytes(&xml))?;
    drop(file);
    let result = command("schtasks.exe")
        .args(["/Create", "/TN", &task_name()?, "/XML"])
        .arg(&path)
        .arg("/F")
        .output();
    let _ = fs::remove_file(&path);
    let result = result.context("Start Task Scheduler")?;
    if !result.status.success() {
        bail!(
            "Task Scheduler: {} {}. For --elevated, run this command once as administrator.",
            command_output(&result.stdout),
            command_output(&result.stderr)
        );
    }
    Ok(())
}
pub fn remove() -> Result<()> {
    let result = command("schtasks.exe")
        .args(["/Delete", "/TN", &task_name()?, "/F"])
        .output()?;
    if !result.status.success() {
        bail!(
            "Task Scheduler: {} {}",
            command_output(&result.stdout),
            command_output(&result.stderr)
        );
    }
    Ok(())
}

pub fn start() -> Result<()> {
    let result = command("schtasks.exe")
        .args(["/Run", "/TN", &task_name()?])
        .output()?;
    if !result.status.success() {
        bail!(
            "Start scheduled task: {} {}",
            command_output(&result.stdout),
            command_output(&result.stderr)
        );
    }
    Ok(())
}

fn task_xml_bytes(xml: &str) -> Vec<u8> {
    // schtasks reads task definitions as Unicode. Match the declaration and BOM.
    std::iter::once(0xfeffu16)
        .chain(xml.encode_utf16())
        .flat_map(u16::to_le_bytes)
        .collect()
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetOEMCP() -> u32;
    fn MultiByteToWideChar(
        codepage: u32,
        flags: u32,
        input: *const u8,
        input_length: i32,
        output: *mut u16,
        output_length: i32,
    ) -> i32;
}
fn command_output(bytes: &[u8]) -> String {
    if let Ok(text) = std::str::from_utf8(bytes) {
        return text.into();
    }
    if bytes.len() > i32::MAX as usize {
        return String::from_utf8_lossy(bytes).into();
    }
    unsafe {
        let codepage = GetOEMCP();
        let length = MultiByteToWideChar(
            codepage,
            0,
            bytes.as_ptr(),
            bytes.len() as i32,
            std::ptr::null_mut(),
            0,
        );
        if length <= 0 {
            return String::from_utf8_lossy(bytes).into();
        }
        let mut output = vec![0u16; length as usize];
        let written = MultiByteToWideChar(
            codepage,
            0,
            bytes.as_ptr(),
            bytes.len() as i32,
            output.as_mut_ptr(),
            length,
        );
        String::from_utf16_lossy(&output[..written.max(0) as usize])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scheduled_task_quotes_unicode_and_spaces_and_has_no_runtime_limit() {
        let xml = task_xml(
            Path::new(r"C:\Тест & tools\watch.exe"),
            Path::new(r"C:\My config\config.toml"),
            "S-1-5-21-test",
            true,
        );
        assert!(xml.contains("Тест &amp; tools"));
        assert!(xml.contains("&quot;C:\\My config\\config.toml&quot;"));
        assert!(xml.contains("PT0S"));
        assert!(xml.contains("HighestAvailable"));
        assert!(xml.contains("InteractiveToken"));
        assert!(xml.contains("encoding=\"UTF-16\""));
        let bytes = task_xml_bytes(&xml);
        assert_eq!(&bytes[..2], &[0xff, 0xfe]);
        let words: Vec<_> = bytes[2..]
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        assert_eq!(String::from_utf16(&words).unwrap(), xml);
    }
}
