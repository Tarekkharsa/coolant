//! Colors. One dark palette; overhead (heat the OS adds) is always orange.

use gpui::Rgba;

pub const BG: Rgba = rgba(0x0f1115);
pub const CARD: Rgba = rgba(0x171a21);
pub const BORDER: Rgba = rgba(0x262a33);
pub const TRACK: Rgba = rgba(0x222630);
pub const TEXT: Rgba = rgba(0xe6e8ee);
pub const MUTED: Rgba = rgba(0x8a91a0);
pub const ACCENT: Rgba = rgba(0x5b9cff);
pub const GOOD: Rgba = rgba(0x4cc38a);
pub const WARN: Rgba = rgba(0xf5a524);
pub const BAD: Rgba = rgba(0xf2555a);

const fn rgba(hex: u32) -> Rgba {
    Rgba {
        r: ((hex >> 16) & 0xff) as f32 / 255.0,
        g: ((hex >> 8) & 0xff) as f32 / 255.0,
        b: (hex & 0xff) as f32 / 255.0,
        a: 1.0,
    }
}

/// Blue when cool, orange when warm, red when hot.
pub fn temp_color(c: f64) -> Rgba {
    match c {
        c if c >= 85.0 => BAD,
        c if c >= 65.0 => WARN,
        _ => ACCENT,
    }
}
