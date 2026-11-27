//! Explicit Shell identity so notifications do not inherit cached EXE branding.
use crate::collectors::pdh::wide;
use anyhow::{Result, ensure};
use std::path::Path;
use windows::{
    Win32::{
        Foundation::{HWND, PROPERTYKEY},
        System::Com::{
            CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
            CoUninitialize, IPersistFile, StructuredStorage::PROPVARIANT,
        },
        UI::Shell::{
            IShellLinkW,
            PropertiesSystem::{
                IPropertyStore, PSCoerceToCanonicalValue, SHGetPropertyStoreForWindow,
            },
            SHCNE_UPDATEITEM, SHCNF_PATHW, SHChangeNotify, SetCurrentProcessExplicitAppUserModelID,
            ShellLink,
        },
    },
    core::{BSTR, GUID, Interface, PCWSTR, w},
};

const APP_ID: PCWSTR = w!("StutterWatch.Desktop");
const APP_ID_TEXT: &str = "StutterWatch.Desktop";
const APP_ID_KEY: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0x9f4c2855_9f79_4b39_a8d0_e1d42de1d5f3),
    pid: 5,
};
const ICON_KEY: PROPERTYKEY = PROPERTYKEY {
    pid: 3,
    ..APP_ID_KEY
};

struct Apartment;
impl Apartment {
    fn new() -> Result<Self> {
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
        }
        Ok(Self)
    }
}

impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe {
            CoUninitialize();
        }
    }
}

pub fn set_process_id() {
    unsafe {
        let _ = SetCurrentProcessExplicitAppUserModelID(APP_ID);
    }
}

unsafe fn set_string(store: &IPropertyStore, key: &PROPERTYKEY, text: &str) -> Result<()> {
    let mut value = PROPVARIANT::from(text);
    unsafe {
        PSCoerceToCanonicalValue(key, &mut value)?;
        store.SetValue(key, &value)?;
    }
    Ok(())
}

pub fn set_window_id(hwnd: HWND) -> Result<()> {
    let _apartment = Apartment::new()?;
    let exe = std::env::current_exe()?;
    let icon = format!("{},-1", exe.display());
    unsafe {
        let properties: IPropertyStore = SHGetPropertyStoreForWindow(hwnd)?;
        set_string(&properties, &ICON_KEY, &icon)?;
        set_string(&properties, &APP_ID_KEY, APP_ID_TEXT)?;
    }
    Ok(())
}

pub fn install_shortcut(config: &Path) -> Result<()> {
    let _apartment = Apartment::new()?;
    let exe = std::env::current_exe()?;
    let config = config.canonicalize()?;
    let normalize = |p: &Path| p.to_string_lossy().trim_start_matches(r"\\?\").to_owned();
    let icon = config
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Missing configuration directory"))?
        .join("stutter-watch-bars-v1.ico");
    // A dedicated icon path also avoids the old EXE-path icon cache entry.
    std::fs::write(&icon, include_bytes!("../assets/app.ico"))?;
    let programs = std::path::PathBuf::from(
        std::env::var_os("APPDATA").ok_or_else(|| anyhow::anyhow!("APPDATA is unavailable"))?,
    )
    .join("Microsoft/Windows/Start Menu/Programs/Stutter Watch");
    std::fs::create_dir_all(&programs)?;
    let path = programs.join("Stutter Watch.lnk");
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
        if path.exists() {
            let existing: IPersistFile = link.cast()?;
            existing.Load(
                PCWSTR(wide(&normalize(&path)).as_ptr()),
                windows::Win32::System::Com::STGM_READ,
            )?;
            let store: IPropertyStore = link.cast()?;
            let existing_id = store.GetValue(&APP_ID_KEY)?;
            ensure!(
                BSTR::try_from(&existing_id).unwrap_or_default() == APP_ID_TEXT,
                "Existing Start menu shortcut belongs to another application"
            );
        }
        link.SetPath(PCWSTR(wide(&normalize(&exe)).as_ptr()))?;
        link.SetArguments(PCWSTR(
            wide(&format!("--config \"{}\" --panel", normalize(&config))).as_ptr(),
        ))?;
        link.SetWorkingDirectory(PCWSTR(wide(&normalize(config.parent().unwrap())).as_ptr()))?;
        link.SetDescription(PCWSTR(wide(crate::branding::NAME).as_ptr()))?;
        link.SetIconLocation(PCWSTR(wide(&normalize(&icon)).as_ptr()), 0)?;
        let store: IPropertyStore = link.cast()?;
        set_string(&store, &ICON_KEY, &format!("{},0", normalize(&icon)))?;
        set_string(&store, &APP_ID_KEY, APP_ID_TEXT)?;
        store.Commit()?;
        let file: IPersistFile = link.cast()?;
        file.Save(PCWSTR(wide(&normalize(&path)).as_ptr()), true)?;
        SHChangeNotify(
            SHCNE_UPDATEITEM,
            SHCNF_PATHW,
            Some(wide(&normalize(&path)).as_ptr().cast()),
            None,
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::UI::WindowsAndMessaging::*;

    #[test]
    fn notification_window_has_explicit_identity_and_embedded_product_icon() {
        let _apartment = Apartment::new().unwrap();
        unsafe {
            let hwnd = CreateWindowExW(
                WS_EX_TOOLWINDOW,
                w!("STATIC"),
                w!("identity-test"),
                WS_POPUP,
                0,
                0,
                1,
                1,
                None,
                None,
                None,
                None,
            )
            .unwrap();
            set_window_id(hwnd).unwrap();
            let store: IPropertyStore = SHGetPropertyStoreForWindow(hwnd).unwrap();
            let id = BSTR::try_from(&store.GetValue(&APP_ID_KEY).unwrap())
                .unwrap()
                .to_string();
            let icon = BSTR::try_from(&store.GetValue(&ICON_KEY).unwrap())
                .unwrap()
                .to_string();
            assert_eq!(id, APP_ID_TEXT);
            assert!(icon.ends_with(",-1"));
            DestroyWindow(hwnd).unwrap();
        }
    }
}
