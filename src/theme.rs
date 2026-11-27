//! A small native dark palette. Controls retain their Win32 input/accessibility behavior.
use windows::{
    Win32::{
        Foundation::*,
        Graphics::{Dwm::*, Gdi::*},
        UI::{Controls::*, Input::KeyboardAndMouse::*, Shell::*, WindowsAndMessaging::*},
    },
    core::w,
};

pub const fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    COLORREF(r as u32 | ((g as u32) << 8) | ((b as u32) << 16))
}
pub const BACKGROUND: COLORREF = rgb(25, 28, 33);
pub const FIELD: COLORREF = rgb(31, 35, 41);
pub const BUTTON: COLORREF = rgb(43, 48, 56);
pub const ACTIVE: COLORREF = rgb(14, 18, 24);
pub const HOVER: COLORREF = rgb(54, 61, 71);
pub const BORDER: COLORREF = rgb(64, 72, 84);
pub const FOREGROUND: COLORREF = rgb(226, 231, 239);
pub const MUTED: COLORREF = rgb(156, 167, 183);
pub const BLUE: COLORREF = rgb(125, 173, 245);
pub const AMBER: COLORREF = rgb(223, 176, 105);
pub const GRID: COLORREF = rgb(49, 55, 65);
pub const DANGER: COLORREF = rgb(244, 115, 122);
pub const DANGER_BG: COLORREF = rgb(58, 30, 36);
pub const WARNING_BG: COLORREF = rgb(51, 43, 29);
pub const fn series_color(index: usize) -> COLORREF {
    if matches!(index, 1 | 4 | 6) {
        AMBER
    } else {
        BLUE
    }
}

pub unsafe fn window_icon(hwnd: HWND) {
    unsafe {
        let instance = HINSTANCE(
            windows::Win32::System::LibraryLoader::GetModuleHandleW(None)
                .unwrap_or_default()
                .0,
        );
        let dpi = windows::Win32::UI::HiDpi::GetDpiForWindow(hwnd).max(96) as i32;
        for (kind, size) in [(ICON_SMALL, 16), (ICON_BIG, 32)] {
            if let Ok(icon) = LoadImageW(
                Some(instance),
                windows::core::PCWSTR(std::ptr::without_provenance(1)),
                IMAGE_ICON,
                size * dpi / 96,
                size * dpi / 96,
                LR_SHARED,
            ) {
                SendMessageW(
                    hwnd,
                    WM_SETICON,
                    Some(WPARAM(kind as usize)),
                    Some(LPARAM(icon.0 as isize)),
                );
            }
        }
    }
}

pub struct Theme {
    pub background: HBRUSH,
    pub field: HBRUSH,
    button: HBRUSH,
    active: HBRUSH,
    hover: HBRUSH,
    border: HBRUSH,
}
impl Theme {
    pub fn new() -> Self {
        unsafe {
            Self {
                background: CreateSolidBrush(BACKGROUND),
                field: CreateSolidBrush(FIELD),
                button: CreateSolidBrush(BUTTON),
                active: CreateSolidBrush(ACTIVE),
                hover: CreateSolidBrush(HOVER),
                border: CreateSolidBrush(BORDER),
            }
        }
    }
    pub unsafe fn titlebar(&self, hwnd: HWND) {
        unsafe {
            let dark = 1i32;
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_USE_IMMERSIVE_DARK_MODE,
                (&dark as *const i32).cast(),
                4,
            );
            for (attribute, color) in [
                (DWMWA_CAPTION_COLOR, BACKGROUND),
                (DWMWA_TEXT_COLOR, FOREGROUND),
                (DWMWA_BORDER_COLOR, BORDER),
            ] {
                let _ =
                    DwmSetWindowAttribute(hwnd, attribute, (&color as *const COLORREF).cast(), 4);
            }
        }
    }
    pub unsafe fn attach(&self, hwnd: HWND, kind: Kind, font: HFONT) {
        unsafe {
            let heading = if matches!(kind, Kind::Preview) {
                let mut lf = LOGFONTW::default();
                GetObjectW(
                    HGDIOBJ(font.0),
                    std::mem::size_of::<LOGFONTW>() as i32,
                    Some((&mut lf as *mut LOGFONTW).cast()),
                );
                lf.lfWeight = 600;
                lf.lfHeight = lf.lfHeight * 16 / 14;
                CreateFontIndirectW(&lf)
            } else {
                HFONT::default()
            };
            let style = Box::new(ControlStyle {
                theme: self as *const _,
                kind,
                font,
                heading,
                buffer: None,
                active: false,
                hover: false,
            });
            let ptr = Box::into_raw(style);
            if !SetWindowSubclass(hwnd, Some(control_proc), 1, ptr as usize).as_bool() {
                drop(Box::from_raw(ptr));
            }
            let _ = SetWindowTheme(hwnd, w!(""), w!(""));
        }
    }
    pub unsafe fn combo_item(&self, item: &DRAWITEMSTRUCT, font: HFONT, dpi: i32) {
        unsafe {
            if item.itemID == u32::MAX {
                return;
            }
            let saved = SaveDC(item.hDC);
            SelectObject(item.hDC, HGDIOBJ(font.0));
            SetBkMode(item.hDC, TRANSPARENT);
            let selected = item.itemState.0 & ODS_SELECTED.0 != 0;
            FillRect(
                item.hDC,
                &item.rcItem,
                if selected { self.hover } else { self.field },
            );
            let mut r = item.rcItem;
            r.left += 10 * dpi / 96;
            r.right -= 8 * dpi / 96;
            let length = SendMessageW(
                item.hwndItem,
                CB_GETLBTEXTLEN,
                Some(WPARAM(item.itemID as usize)),
                None,
            )
            .0;
            if (0..4096).contains(&length) {
                let mut buffer = vec![0u16; length as usize + 1];
                SendMessageW(
                    item.hwndItem,
                    CB_GETLBTEXT,
                    Some(WPARAM(item.itemID as usize)),
                    Some(LPARAM(buffer.as_mut_ptr() as isize)),
                );
                SetTextColor(item.hDC, FOREGROUND);
                label(
                    item.hDC,
                    r,
                    &String::from_utf16_lossy(&buffer[..length as usize]),
                    DT_LEFT | DT_VCENTER,
                );
            }
            let _ = RestoreDC(item.hDC, saved);
        }
    }
}
impl Drop for Theme {
    fn drop(&mut self) {
        unsafe {
            for b in [
                self.background,
                self.field,
                self.button,
                self.active,
                self.hover,
                self.border,
            ] {
                let _ = DeleteObject(HGDIOBJ(b.0));
            }
        }
    }
}

#[derive(Clone, Copy)]
pub enum Kind {
    Button,
    Checkbox,
    Combo,
    Preview,
}
struct ControlStyle {
    theme: *const Theme,
    kind: Kind,
    font: HFONT,
    heading: HFONT,
    buffer: Option<Buffer>,
    active: bool,
    hover: bool,
}
struct Buffer {
    dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
    width: i32,
    height: i32,
}
impl Buffer {
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
impl Drop for Buffer {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.previous);
            let _ = DeleteObject(HGDIOBJ(self.bitmap.0));
            let _ = DeleteDC(self.dc);
        }
    }
}
impl Drop for ControlStyle {
    fn drop(&mut self) {
        if !self.heading.is_invalid() {
            unsafe {
                let _ = DeleteObject(HGDIOBJ(self.heading.0));
            }
        }
    }
}
pub fn preview_height(text: &str) -> i32 {
    64 + text.lines().count().saturating_sub(1) as i32 * 22
}
pub unsafe fn set_active(hwnd: HWND, active: bool) {
    unsafe {
        let mut data = 0;
        if GetWindowSubclass(hwnd, Some(control_proc), 1, Some(&mut data)).as_bool() {
            let style = &mut *(data as *mut ControlStyle);
            if style.active != active {
                style.active = active;
                let _ = InvalidateRect(Some(hwnd), None, false);
            }
        }
    }
}
pub unsafe fn label(dc: HDC, mut rect: RECT, text: &str, format: DRAW_TEXT_FORMAT) {
    unsafe {
        let mut units: Vec<_> = text.encode_utf16().collect();
        DrawTextW(
            dc,
            &mut units,
            &mut rect,
            format | DT_SINGLELINE | DT_NOPREFIX | DT_END_ELLIPSIS,
        );
    }
}

unsafe fn paint_control(hwnd: HWND, style: &mut ControlStyle) {
    unsafe {
        let theme = &*style.theme;
        let mut ps = PAINTSTRUCT::default();
        let target = BeginPaint(hwnd, &mut ps);
        let mut r = RECT::default();
        let _ = GetClientRect(hwnd, &mut r);
        if matches!(style.kind, Kind::Preview)
            && style
                .buffer
                .as_ref()
                .is_none_or(|b| b.width != r.right || b.height != r.bottom)
        {
            style.buffer = Buffer::new(target, r.right, r.bottom);
        }
        let dc = style.buffer.as_ref().map(|b| b.dc).unwrap_or(target);
        let saved = SaveDC(dc);
        SelectObject(dc, HGDIOBJ(style.font.0));
        SetBkMode(dc, TRANSPARENT);
        SetTextColor(dc, FOREGROUND);
        let dpi = windows::Win32::UI::HiDpi::GetDpiForWindow(hwnd).max(96) as i32;
        let pad = 10 * dpi / 96;
        let count = GetWindowTextLengthW(hwnd).max(0) as usize;
        let mut title = vec![0u16; count + 1];
        let n = GetWindowTextW(hwnd, &mut title).max(0) as usize;
        let title = String::from_utf16_lossy(&title[..n]);
        match style.kind {
            Kind::Preview => {
                FillRect(dc, &r, theme.background);
                let pen = CreatePen(PS_SOLID, 1, BORDER);
                let old_pen = SelectObject(dc, HGDIOBJ(pen.0));
                let old_brush = SelectObject(dc, HGDIOBJ(theme.field.0));
                let _ = RoundRect(
                    dc,
                    1,
                    1,
                    r.right - 1,
                    r.bottom - 1,
                    8 * dpi / 96,
                    8 * dpi / 96,
                );
                SelectObject(dc, old_brush);
                SelectObject(dc, old_pen);
                let _ = DeleteObject(HGDIOBJ(pen.0));
                let scale = |v: i32| v * dpi / 96;
                let old_color = SetDCBrushColor(dc, BLUE);
                for (x, top) in [(18, 28), (25, 20), (32, 24)] {
                    FillRect(
                        dc,
                        &RECT {
                            left: scale(x),
                            top: scale(top),
                            right: scale(x + 4),
                            bottom: scale(36),
                        },
                        HBRUSH(GetStockObject(DC_BRUSH).0),
                    );
                }
                SetDCBrushColor(dc, old_color);
                let mut lines = title.lines();
                if !style.heading.is_invalid() {
                    SelectObject(dc, HGDIOBJ(style.heading.0));
                }
                label(
                    dc,
                    RECT {
                        left: scale(52),
                        top: scale(14),
                        right: r.right - scale(18),
                        bottom: scale(42),
                    },
                    lines.next().unwrap_or(""),
                    DT_LEFT | DT_VCENTER,
                );
                SelectObject(dc, HGDIOBJ(style.font.0));
                for (i, line) in lines.enumerate() {
                    SetTextColor(
                        dc,
                        if line.contains("ms/") {
                            MUTED
                        } else {
                            FOREGROUND
                        },
                    );
                    let top = 50 + i as i32 * 22;
                    label(
                        dc,
                        RECT {
                            left: scale(18),
                            top: scale(top),
                            right: r.right - scale(18),
                            bottom: scale(top + 22),
                        },
                        line,
                        DT_LEFT | DT_VCENTER,
                    );
                }
            }
            Kind::Button => {
                let pressed =
                    SendMessageW(hwnd, BM_GETSTATE, None, None).0 as u32 & BST_PUSHED != 0;
                FillRect(
                    dc,
                    &r,
                    if style.active || pressed {
                        theme.active
                    } else if style.hover {
                        theme.hover
                    } else {
                        theme.button
                    },
                );
                FrameRect(dc, &r, theme.border);
                if style.active {
                    let line = RECT {
                        left: r.left + 1,
                        top: r.bottom - 3 * dpi / 96,
                        right: r.right - 1,
                        bottom: r.bottom - 1,
                    };
                    let old = SetDCBrushColor(dc, BLUE);
                    FillRect(dc, &line, HBRUSH(GetStockObject(DC_BRUSH).0));
                    SetDCBrushColor(dc, old);
                }
                let mut text = r;
                text.left += pad;
                text.right -= pad;
                label(dc, text, &title, DT_CENTER | DT_VCENTER);
            }
            Kind::Checkbox => {
                FillRect(dc, &r, theme.background);
                let size = 14 * dpi / 96;
                let y = (r.bottom - size) / 2;
                let box_rect = RECT {
                    left: 4 * dpi / 96,
                    top: y,
                    right: 4 * dpi / 96 + size,
                    bottom: y + size,
                };
                let checked = SendMessageW(hwnd, BM_GETCHECK, None, None).0 == 1;
                FillRect(
                    dc,
                    &box_rect,
                    if checked { theme.hover } else { theme.field },
                );
                FrameRect(dc, &box_rect, theme.border);
                if checked {
                    let pen = CreatePen(PS_SOLID, (2 * dpi / 96).max(1), BLUE);
                    let old = SelectObject(dc, HGDIOBJ(pen.0));
                    let _ = MoveToEx(dc, box_rect.left + size / 5, y + size / 2, None);
                    let _ = LineTo(dc, box_rect.left + size * 2 / 5, y + size * 3 / 4);
                    let _ = LineTo(dc, box_rect.left + size * 4 / 5, y + size / 4);
                    SelectObject(dc, old);
                    let _ = DeleteObject(HGDIOBJ(pen.0));
                }
                let mut text = r;
                text.left = box_rect.right + 9 * dpi / 96;
                let id = GetDlgCtrlID(hwnd);
                if (400..=406).contains(&id) {
                    let key = RECT {
                        left: text.left,
                        top: r.bottom / 2 - dpi / 96,
                        right: text.left + 13 * dpi / 96,
                        bottom: r.bottom / 2 + 2 * dpi / 96,
                    };
                    let color = SetDCBrushColor(dc, series_color((id - 400) as usize));
                    FillRect(dc, &key, HBRUSH(GetStockObject(DC_BRUSH).0));
                    SetDCBrushColor(dc, color);
                    text.left += 21 * dpi / 96;
                }
                label(dc, text, &title, DT_LEFT | DT_VCENTER);
            }
            Kind::Combo => {
                FillRect(dc, &r, theme.field);
                FrameRect(dc, &r, theme.border);
                let mut text = r;
                text.left += pad;
                text.right -= 28 * dpi / 96;
                label(dc, text, &title, DT_LEFT | DT_VCENTER);
                let x = r.right - 15 * dpi / 96;
                let y = r.bottom / 2;
                let pen = CreatePen(PS_SOLID, 1, MUTED);
                let old = SelectObject(dc, HGDIOBJ(pen.0));
                let _ = MoveToEx(dc, x - 4, y - 2, None);
                let _ = LineTo(dc, x, y + 2);
                let _ = LineTo(dc, x + 4, y - 2);
                SelectObject(dc, old);
                let _ = DeleteObject(HGDIOBJ(pen.0));
            }
        }
        if GetFocus() == hwnd {
            let mut focus = r;
            let inset = if matches!(style.kind, Kind::Checkbox) {
                (dpi / 96).max(1)
            } else {
                (3 * dpi / 96).max(1)
            };
            let _ = InflateRect(&mut focus, -inset, -inset);
            SetTextColor(dc, MUTED);
            let _ = DrawFocusRect(dc, &focus);
        }
        let _ = RestoreDC(dc, saved);
        if dc != target {
            let _ = BitBlt(target, 0, 0, r.right, r.bottom, Some(dc), 0, 0, SRCCOPY);
        }
        let _ = EndPaint(hwnd, &ps);
    }
}
unsafe extern "system" fn control_proc(
    hwnd: HWND,
    message: u32,
    wp: WPARAM,
    lp: LPARAM,
    _id: usize,
    data: usize,
) -> LRESULT {
    unsafe {
        let ptr = data as *mut ControlStyle;
        match message {
            WM_SETTEXT if matches!((*ptr).kind, Kind::Preview) => {
                // Store the caption without STATIC's immediate unbuffered repaint.
                let result = DefWindowProcW(hwnd, message, wp, lp);
                let _ = InvalidateRect(Some(hwnd), None, false);
                return result;
            }
            WM_NCDESTROY => {
                let _ = RemoveWindowSubclass(hwnd, Some(control_proc), 1);
                drop(Box::from_raw(ptr));
                return DefSubclassProc(hwnd, message, wp, lp);
            }
            WM_PAINT => {
                paint_control(hwnd, &mut *ptr);
                return LRESULT(0);
            }
            WM_SHOWWINDOW if wp.0 == 0 => {
                (*ptr).buffer = None;
            }
            WM_ERASEBKGND => return LRESULT(1),
            WM_MOUSEMOVE => {
                if !(*ptr).hover {
                    (*ptr).hover = true;
                    let mut tracking = TRACKMOUSEEVENT {
                        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: hwnd,
                        ..Default::default()
                    };
                    let _ = TrackMouseEvent(&mut tracking);
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
            }
            WM_MOUSELEAVE => {
                (*ptr).hover = false;
                let _ = InvalidateRect(Some(hwnd), None, false);
            }
            _ => {}
        }
        let result = DefSubclassProc(hwnd, message, wp, lp);
        if matches!(
            message,
            WM_SETFOCUS
                | WM_KILLFOCUS
                | WM_LBUTTONDOWN
                | WM_LBUTTONUP
                | BM_SETCHECK
                | BM_SETSTATE
                | CB_SETCURSEL
                | WM_SETTEXT
                | WM_ENABLE
        ) {
            let _ = InvalidateRect(Some(hwnd), None, false);
        }
        result
    }
}
