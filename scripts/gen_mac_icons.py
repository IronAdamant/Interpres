#!/usr/bin/env python3
"""Generate Mac logo masters + Interpres.icns from assets/logo-256.png.

Uses the cleaned square symbol (same mark as Windows). Writes:
  - assets/logo.png (1024 RGB master)
  - assets/logo-1024.png (same)
  - assets/Interpres.icns (multi-size via iconutil)

iconset is built under /tmp to avoid leaving assets/Interpres.iconset around
(.gitignore). On case-insensitive APFS, all ten iconset names are distinct.
"""
from __future__ import annotations

import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

from PIL import Image

ROOT = Path(__file__).resolve().parents[1]
SRC = ROOT / "assets" / "logo-256.png"
# iconset entry name, pixel edge — names built without embedding '@' in this file
# as plain string literals that some transports mangle; use chr(64) for '@'.
AT = chr(64)


def icon_name(base: str, retina: bool = False) -> str:
    """e.g. icon_name('16x16') -> icon_16x16.png; retina -> icon_16x16@2x.png"""
    if retina:
        return f"icon_{base}{AT}2x.png"
    return f"icon_{base}.png"


def main() -> int:
    if not SRC.is_file():
        print(f"missing source {SRC}", file=sys.stderr)
        return 1

    src = Image.open(SRC).convert("RGBA")
    if src.size != (256, 256):
        print(f"expected 256x256 source, got {src.size}", file=sys.stderr)
        return 1

    master = src.resize((1024, 1024), Image.Resampling.LANCZOS)
    master_rgb = master.convert("RGB")

    logo = ROOT / "assets" / "logo.png"
    logo1024 = ROOT / "assets" / "logo-1024.png"
    master_rgb.save(logo, "PNG", optimize=True)
    master_rgb.save(logo1024, "PNG", optimize=True)
    print(f"wrote {logo.relative_to(ROOT)} ({logo.stat().st_size} bytes)")
    print(f"wrote {logo1024.relative_to(ROOT)} ({logo1024.stat().st_size} bytes)")

    # (filename, edge)
    entries = [
        (icon_name("16x16"), 16),
        (icon_name("16x16", retina=True), 32),
        (icon_name("32x32"), 32),
        (icon_name("32x32", retina=True), 64),
        (icon_name("128x128"), 128),
        (icon_name("128x128", retina=True), 256),
        (icon_name("256x256"), 256),
        (icon_name("256x256", retina=True), 512),
        (icon_name("512x512"), 512),
        (icon_name("512x512", retina=True), 1024),
    ]

    def raster(edge: int) -> Image.Image:
        if edge == 256:
            return src.copy()
        if edge == 1024:
            return master.copy()
        if edge < 256:
            return src.resize((edge, edge), Image.Resampling.LANCZOS)
        return master.resize((edge, edge), Image.Resampling.LANCZOS)

    with tempfile.TemporaryDirectory(prefix="interpres-iconset-") as tmp:
        iconset = Path(tmp) / "Interpres.iconset"
        iconset.mkdir()
        for name, edge in entries:
            path = iconset / name
            im = raster(edge)
            im.save(path, "PNG")
            got = Image.open(path).size
            if got != (edge, edge):
                print(f"size mismatch {name}: {got} != {edge}", file=sys.stderr)
                return 1
            print(f"  {name:24} {edge}x{edge}")

        names = sorted(p.name for p in iconset.iterdir())
        if len(names) != 10:
            print(f"expected 10 iconset files, got {len(names)}: {names}", file=sys.stderr)
            return 1

        out_icns = ROOT / "assets" / "Interpres.icns"
        # iconutil writes next to iconset if -o omitted; use --output
        subprocess.check_call(
            ["iconutil", "--convert", "icns", "--output", str(out_icns), str(iconset)]
        )
        print(f"wrote {out_icns.relative_to(ROOT)} ({out_icns.stat().st_size} bytes)")

    # Guard: bottom-right dark (no watermark) on master
    bright = 0
    w, h = master_rgb.size
    for y in range(h - 40, h):
        for x in range(w - 100, w):
            if sum(master_rgb.getpixel((x, y))) > 100:
                bright += 1
    if bright != 0:
        print(f"watermark check failed bright_pixels={bright}", file=sys.stderr)
        return 1
    print("watermark check ok")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
