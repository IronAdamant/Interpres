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

    /// Guard against reintroducing a light Grok-style watermark in the bottom-right.
    #[test]
    fn logo_bottom_right_is_dark_no_watermark() {
        let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/logo.png");
        let bytes = std::fs::read(&p).expect("read logo");
        assert!(bytes.starts_with(&[0x89, b'P', b'N', b'G']));
        // Parse IHDR for dimensions (big-endian at offset 16 after 8-byte signature + 8-byte chunk hdr)
        // Minimal PNG: signature(8) + length(4) + "IHDR"(4) + width(4) + height(4)
        assert!(bytes.len() > 24);
        let w = u32::from_be_bytes(bytes[16..20].try_into().unwrap()) as usize;
        let h = u32::from_be_bytes(bytes[20..24].try_into().unwrap()) as usize;
        assert_eq!((w, h), (1024, 1024), "expected 1024 logo master");

        // Decode with a tiny pure-std approach is heavy; use `sips`/`python` is external.
        // Instead: ensure shipped logo-clean was promoted — re-scan RGB via optional python in CI
        // For unit test without crates: shell out is flaky. Check file is smaller after watermark
        // strip (watermarked was ~464k, clean ~310k) and PNG valid.
        let len = bytes.len() as u64;
        assert!(
            len < 420_000,
            "logo.png unexpectedly large ({len}); may still contain watermark baggage"
        );

        // Pixel-level check via `sips` export is unavailable in pure rust without deps.
        // Drive real decode through Python available on Mac dev machines:
        let status = std::process::Command::new("python3")
            .args([
                "-c",
                r#"
from PIL import Image
im = Image.open("assets/logo.png").convert("RGB")
w, h = im.size
bright = 0
for y in range(h - 40, h):
    for x in range(w - 100, w):
        if sum(im.getpixel((x, y))) > 100:
            bright += 1
assert bright == 0, f"bright_pixels={bright}"
print("ok", bright)
"#,
            ])
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .status()
            .expect("spawn python3");
        assert!(
            status.success(),
            "bottom-right corner still has bright watermark pixels (python check failed)"
        );
    }
}
