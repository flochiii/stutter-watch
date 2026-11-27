//! Persistent, non-activating tray tooltip. Telemetry updates never replace the window.
use crate::{
    collectors::pdh::wide,
    theme::{Kind, Theme},
};
use anyhow::Result;
use windows::{
    Win32::{
        Foundation::*,
        Graphics::Gdi::*,
        UI::{HiDpi::GetDpiForWindow, WindowsAndMessaging::*},
    },
    core::{PCWSTR, w},
};

pub struct Popup {
    hwnd: HWND,
    font: HFONT,
    _theme: Box<Theme>,
    text: String,
    anchor: RECT,
    width: i32,
    dpi: i32,
    bounds: RECT,
}
impl Popup {
    pub fn new(owner: Option<HWND>, anchor: RECT, text: &str) -> Result<Self> {
        unsafe {
            let hwnd = CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                w!("STATIC"),
                PCWSTR(wide(text).as_ptr()),
                WS_POPUP,
                anchor.left,
                anchor.top,
                1,
                1,
                owner,
                None,
                None,
                None,
            )?;
            let dpi = GetDpiForWindow(hwnd).max(96) as i32;
            let font = CreateFontW(
                -14 * dpi / 96,
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
            let theme = Box::new(Theme::new());
            theme.attach(hwnd, Kind::Preview, font);
            let mut popup = Self {
                hwnd,
                font,
                _theme: theme,
                text: String::new(),
                anchor,
                width: 0,
                dpi,
                bounds: RECT::default(),
            };
            popup.update(text);
            Ok(popup)
        }
    }
    pub fn update(&mut self, text: &str) {
        if self.text == text {
            return;
        }
        self.text = text.into();
        unsafe {
            let _ = SetWindowTextW(self.hwnd, PCWSTR(wide(text).as_ptr()));
            let dc = GetDC(Some(self.hwnd));
            let old = SelectObject(dc, HGDIOBJ(self.font.0));
            let mut measured = 0;
            for (i, line) in text.lines().enumerate() {
                let units: Vec<_> = line.encode_utf16().collect();
                let mut size = SIZE::default();
                let _ = GetTextExtentPoint32W(dc, &units, &mut size);
                measured = measured.max(if i == 0 {
                    size.cx * 16 / 14 + 70 * self.dpi / 96
                } else {
                    size.cx + 36 * self.dpi / 96
                });
            }
            SelectObject(dc, old);
            ReleaseDC(Some(self.hwnd), dc);
            // A small reserve keeps changing digit counts from shifting the popup.
            let width = ((measured * 96 / self.dpi + 47) / 32 * 32).clamp(220, 460) * self.dpi / 96;
            self.width = self.width.max(width);
            let mut monitor = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            let _ = GetMonitorInfoW(
                MonitorFromRect(&self.anchor, MONITOR_DEFAULTTONEAREST),
                &mut monitor,
            );
            let height = crate::theme::preview_height(text) * self.dpi / 96;
            let rect = placement(
                self.anchor,
                monitor.rcWork,
                self.width,
                height,
                8 * self.dpi / 96,
            );
            if self.bounds != rect {
                let _ = SetWindowPos(
                    self.hwnd,
                    Some(HWND_TOPMOST),
                    rect.left,
                    rect.top,
                    rect.right - rect.left,
                    rect.bottom - rect.top,
                    SWP_NOACTIVATE,
                );
                let region = CreateRoundRectRgn(
                    0,
                    0,
                    rect.right - rect.left,
                    rect.bottom - rect.top,
                    8 * self.dpi / 96,
                    8 * self.dpi / 96,
                );
                if SetWindowRgn(self.hwnd, Some(region), true) == 0 {
                    let _ = DeleteObject(HGDIOBJ(region.0));
                }
                self.bounds = rect;
            }
            let _ = InvalidateRect(Some(self.hwnd), None, false);
        }
    }
    pub fn show(&self) {
        unsafe {
            if !IsWindowVisible(self.hwnd).as_bool() {
                let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
            }
        }
    }
}
impl Drop for Popup {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.hwnd);
            let _ = DeleteObject(HGDIOBJ(self.font.0));
        }
    }
}
fn placement(anchor: RECT, work: RECT, width: i32, height: i32, gap: i32) -> RECT {
    let width = width.min((work.right - work.left - 2 * gap).max(1));
    let height = height.min((work.bottom - work.top - 2 * gap).max(1));
    let left = ((anchor.left + anchor.right - width) / 2).clamp(
        work.left + gap,
        (work.right - width - gap).max(work.left + gap),
    );
    let above = anchor.top - height - gap;
    let top = if above >= work.top {
        above
    } else {
        anchor.bottom + gap
    }
    .clamp(
        work.top + gap,
        (work.bottom - height - gap).max(work.top + gap),
    );
    RECT {
        left,
        top,
        right: left + width,
        bottom: top + height,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn popup_fits_work_area_at_top_bottom_and_negative_monitor_edges() {
        let work = RECT {
            left: -1920,
            top: 0,
            right: 0,
            bottom: 1040,
        };
        for anchor in [
            RECT {
                left: -25,
                top: 1040,
                right: 0,
                bottom: 1080,
            },
            RECT {
                left: -1920,
                top: 0,
                right: -1890,
                bottom: 40,
            },
        ] {
            let r = placement(anchor, work, 400, 130, 8);
            assert!(
                r.left >= work.left
                    && r.right <= work.right
                    && r.top >= work.top
                    && r.bottom <= work.bottom
            );
        }
    }
    #[test]
    fn telemetry_updates_keep_the_same_window_and_release_it_on_close() {
        let mut popup = Popup::new(
            None,
            RECT {
                left: 100,
                top: 100,
                right: 130,
                bottom: 130,
            },
            "Stutter Watch\nCPU: 1%",
        )
        .unwrap();
        let hwnd = popup.hwnd;
        for value in [10, 100, 4] {
            let text = format!("Stutter Watch\nCPU: {value}%");
            popup.update(&text);
            assert_eq!(popup.hwnd, hwnd);
            let mut stored = [0u16; 128];
            let length = unsafe { GetWindowTextW(hwnd, &mut stored) } as usize;
            assert_eq!(String::from_utf16_lossy(&stored[..length]), text);
        }
        assert!(unsafe { IsWindow(Some(hwnd)) }.as_bool());
        drop(popup);
        assert!(!unsafe { IsWindow(Some(hwnd)) }.as_bool());
    }
}
