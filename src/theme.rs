//! Shared light / dark appearance tokens for native UIs.
//!
//! Palette values are mirrored in:
//! - `native/macos/interpres_gui.m` (AppKit `NSColor`)
//! - `src/gui_win.rs` (GDI `0x00BBGGRR`)
//!
//! This module is the single source of truth for **mode names**, config values,
//! button labels, and resolution of `system` → concrete light/dark. It does not
//! touch capture, engine, or transcript logic.

/// User-facing appearance preference (sticky in settings).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ThemeMode {
    /// Follow OS light/dark preference.
    #[default]
    System,
    Light,
    Dark,
}

impl ThemeMode {
    pub fn as_str(self) -> &'static str {
        match self {
            ThemeMode::System => "system",
            ThemeMode::Light => "light",
            ThemeMode::Dark => "dark",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "light" | "day" => ThemeMode::Light,
            "dark" | "night" => ThemeMode::Dark,
            "system" | "auto" | "" => ThemeMode::System,
            _ => ThemeMode::System,
        }
    }

    /// Cycle System → Light → Dark → System (Theme button).
    pub fn cycle(self) -> Self {
        match self {
            ThemeMode::System => ThemeMode::Light,
            ThemeMode::Light => ThemeMode::Dark,
            ThemeMode::Dark => ThemeMode::System,
        }
    }

    /// Wire value for C / Win32 callbacks: 0 = system, 1 = light, 2 = dark.
    pub fn as_int(self) -> i32 {
        match self {
            ThemeMode::System => 0,
            ThemeMode::Light => 1,
            ThemeMode::Dark => 2,
        }
    }

    pub fn from_int(v: i32) -> Self {
        match v {
            1 => ThemeMode::Light,
            2 => ThemeMode::Dark,
            _ => ThemeMode::System,
        }
    }

    /// Button caption shared by Mac and Windows.
    pub fn button_label(self) -> &'static str {
        match self {
            ThemeMode::System => "Theme: System",
            ThemeMode::Light => "Theme: Light",
            ThemeMode::Dark => "Theme: Dark",
        }
    }

    /// Resolve to concrete dark/light using the current OS preference.
    pub fn resolve_dark(self, system_is_dark: bool) -> bool {
        match self {
            ThemeMode::System => system_is_dark,
            ThemeMode::Light => false,
            ThemeMode::Dark => true,
        }
    }
}

/// Shared sRGB 0–1 components (document + tests). Platforms convert to native types.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rgb {
    pub r: f32,
    pub g: f32,
    pub b: f32,
}

impl Rgb {
    pub const fn new(r: f32, g: f32, b: f32) -> Self {
        Self { r, g, b }
    }

    /// GDI COLORREF `0x00BBGGRR`.
    pub fn to_gdi(self) -> u32 {
        let r = (self.r.clamp(0.0, 1.0) * 255.0).round() as u32;
        let g = (self.g.clamp(0.0, 1.0) * 255.0).round() as u32;
        let b = (self.b.clamp(0.0, 1.0) * 255.0).round() as u32;
        (b << 16) | (g << 8) | r
    }
}

/// Concrete palette for one appearance (dark or light).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    pub bg: Rgb,
    pub panel: Rgb,
    pub text: Rgb,
    pub muted: Rgb,
    pub accent: Rgb,
    pub button: Rgb,
    pub border: Rgb,
}

/// Dark palette — matches the shipped companion chrome (both platforms).
pub const PALETTE_DARK: Palette = Palette {
    bg: Rgb::new(0.07, 0.08, 0.10),
    panel: Rgb::new(0.12, 0.13, 0.16),
    text: Rgb::new(0.95, 0.95, 0.95),
    muted: Rgb::new(0.65, 0.65, 0.65),
    accent: Rgb::new(0.20, 0.75, 0.55),
    button: Rgb::new(0.18, 0.19, 0.22),
    border: Rgb::new(0.28, 0.30, 0.34),
};

/// Light palette — same structure, high-contrast for Deaf / hard-of-hearing UI.
pub const PALETTE_LIGHT: Palette = Palette {
    bg: Rgb::new(0.96, 0.96, 0.97),
    panel: Rgb::new(1.0, 1.0, 1.0),
    text: Rgb::new(0.10, 0.11, 0.13),
    muted: Rgb::new(0.36, 0.39, 0.44),
    accent: Rgb::new(0.10, 0.61, 0.43),
    button: Rgb::new(0.90, 0.91, 0.93),
    border: Rgb::new(0.78, 0.80, 0.83),
};

pub fn palette_for_dark(is_dark: bool) -> Palette {
    if is_dark {
        PALETTE_DARK
    } else {
        PALETTE_LIGHT
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_cycle_theme_modes() {
        assert_eq!(ThemeMode::parse("system"), ThemeMode::System);
        assert_eq!(ThemeMode::parse("LIGHT"), ThemeMode::Light);
        assert_eq!(ThemeMode::parse("dark"), ThemeMode::Dark);
        assert_eq!(ThemeMode::parse("weird"), ThemeMode::System);
        assert_eq!(ThemeMode::System.cycle(), ThemeMode::Light);
        assert_eq!(ThemeMode::Light.cycle(), ThemeMode::Dark);
        assert_eq!(ThemeMode::Dark.cycle(), ThemeMode::System);
        assert_eq!(ThemeMode::from_int(ThemeMode::Dark.as_int()), ThemeMode::Dark);
    }

    #[test]
    fn resolve_system_follows_os() {
        assert!(ThemeMode::System.resolve_dark(true));
        assert!(!ThemeMode::System.resolve_dark(false));
        assert!(!ThemeMode::Light.resolve_dark(true));
        assert!(ThemeMode::Dark.resolve_dark(false));
    }

    #[test]
    fn button_labels_stable() {
        assert_eq!(ThemeMode::System.button_label(), "Theme: System");
        assert_eq!(ThemeMode::Light.button_label(), "Theme: Light");
        assert_eq!(ThemeMode::Dark.button_label(), "Theme: Dark");
    }

    #[test]
    fn dark_gdi_matches_existing_shipped_bg() {
        // Historical constant in gui_win: COL_BG = 0x001A_1412 ≈ (0.07,0.08,0.10)
        let gdi = PALETTE_DARK.bg.to_gdi();
        assert_eq!(gdi, 0x001A_1412);
        let text = PALETTE_DARK.text.to_gdi();
        assert_eq!(text, 0x00F2_F2F2);
    }

    #[test]
    fn light_palette_is_high_contrast() {
        let p = PALETTE_LIGHT;
        assert!(p.text.r < 0.2);
        assert!(p.bg.r > 0.9);
        assert!(p.muted.r > p.text.r);
        assert!(p.muted.r < p.bg.r);
    }
}
