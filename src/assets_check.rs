//! Structural checks for shipped branding assets.

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    #[test]
    fn logo_png_exists_and_is_nontrivial() {
        let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/logo.png");
        assert!(
            p.is_file(),
            "expected logo at assets/logo.png — generate via Imagine and place under assets/"
        );
        let len = std::fs::metadata(&p).expect("stat logo").len();
        assert!(
            len > 8_000,
            "logo.png too small ({len} bytes) — likely empty stub"
        );
        // PNG magic
        let bytes = std::fs::read(&p).expect("read logo");
        assert!(
            bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]),
            "assets/logo.png is not a PNG"
        );
    }

    #[test]
    fn mac_icns_exists_for_app_packaging() {
        let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/Interpres.icns");
        assert!(p.is_file(), "expected assets/Interpres.icns for Mac .app icon");
        let len = std::fs::metadata(&p).expect("stat icns").len();
        assert!(len > 1_000, "Interpres.icns too small ({len} bytes)");
    }
}
