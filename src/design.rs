//! Shared visual specification for the Windows and Linux desktop renderers.
pub const BACKGROUND: [u8; 3] = [25, 28, 33];
pub const FIELD: [u8; 3] = [31, 35, 41];
pub const BUTTON: [u8; 3] = [43, 48, 56];
pub const ACTIVE: [u8; 3] = [14, 18, 24];
pub const HOVER: [u8; 3] = [54, 61, 71];
pub const BORDER: [u8; 3] = [64, 72, 84];
pub const FOREGROUND: [u8; 3] = [226, 231, 239];
pub const MUTED: [u8; 3] = [156, 167, 183];
pub const BLUE: [u8; 3] = [125, 173, 245];
pub const AMBER: [u8; 3] = [223, 176, 105];
pub const GREEN: [u8; 3] = [114, 188, 150];
pub const GRID: [u8; 3] = [49, 55, 65];
pub const DANGER: [u8; 3] = [244, 115, 122];
pub const DANGER_BG: [u8; 3] = [58, 30, 36];
pub const WARNING_BG: [u8; 3] = [51, 43, 29];
pub const MARGIN: i32 = 18;
pub const GAP: i32 = 8;
pub const NAV_WIDTH: i32 = 112;
pub const NAV_HEIGHT: i32 = 32;
pub const CHART_GAP: f32 = 12.0;
pub const CHART_HEADER: i32 = 58;
pub const LIVE_TOP: i32 = 76;
pub const BODY_SIZE: i32 = 14;
pub const SMALL_SIZE: i32 = 10;
pub const FOOTER_SPACE: f32 = 54.0;
pub const LATIN_FONT: &[u8] = include_bytes!("../assets/fonts/NotoSans-Regular.ttf");
pub const CJK_SC_FONT: &[u8] = include_bytes!("../assets/fonts/ui-cjk-sc.otf");
pub const CJK_JP_FONT: &[u8] = include_bytes!("../assets/fonts/ui-cjk-jp.otf");

pub fn chart_size(width: f32, height: f32, rows: usize) -> (f32, f32) {
    (
        (width - CHART_GAP) / 2.0,
        (height - FOOTER_SPACE - CHART_GAP * rows.saturating_sub(1) as f32) / rows.max(1) as f32,
    )
}
