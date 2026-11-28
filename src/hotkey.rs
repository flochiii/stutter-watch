use anyhow::{Result, bail, ensure};

pub fn parse_hotkey(text: &str) -> Result<(u32, u32)> {
    let mut mods = 0u32;
    let mut key = None;
    for part in text.split('+').map(str::trim) {
        match part.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => mods |= 2,
            "shift" => mods |= 4,
            "alt" => mods |= 1,
            "win" => mods |= 8,
            function if function.starts_with('f') => {
                let number: u32 = function[1..].parse()?;
                ensure!((1..=24).contains(&number), "F-key must be F1..F24");
                ensure!(key.is_none(), "Only one key allowed");
                key = Some(0x70 + number - 1);
            }
            letter if letter.len() == 1 && letter.as_bytes()[0].is_ascii_alphanumeric() => {
                ensure!(key.is_none(), "Only one key allowed");
                key = Some(letter.as_bytes()[0].to_ascii_uppercase() as u32);
            }
            _ => bail!("Invalid hotkey component: {part}"),
        }
    }
    ensure!(mods > 0, "Hotkey requires a modifier");
    Ok((
        mods,
        key.ok_or_else(|| anyhow::anyhow!("Hotkey requires a key"))?,
    ))
}
