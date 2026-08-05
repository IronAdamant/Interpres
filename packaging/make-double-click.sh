#!/usr/bin/env bash
# Build a portable folder non-technical users can double-click.
# No installers, no app stores — just a folder with an app / .command / .bat.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

echo "==> Building release binary (this may take a minute)…"
cargo build --release

DIST="$ROOT/dist/Interpres"
rm -rf "$DIST"
mkdir -p "$DIST/helpers/macos" "$DIST/helpers/windows"

cp "$ROOT/target/release/interpres" "$DIST/interpres"
chmod +x "$DIST/interpres"
cp "$ROOT/helpers/macos/captions_loop.sh" "$DIST/helpers/macos/" 2>/dev/null || true
cp "$ROOT/helpers/windows/Get-LiveCaptionsText.ps1" "$DIST/helpers/windows/" 2>/dev/null || true
cp "$ROOT/README.md" "$DIST/README.md"

# --- Double-click on Mac: .command opens Terminal ---
cat > "$DIST/Open Interpres.command" << 'EOF'
#!/bin/bash
cd "$(dirname "$0")"
# Opens the native Interpres window (not a webpage, not a long terminal session).
export INTERPRES_GUI=1
./interpres
EOF
chmod +x "$DIST/Open Interpres.command"

# Demo double-click
cat > "$DIST/Try demo (no Live Captions needed).command" << 'EOF'
#!/bin/bash
cd "$(dirname "$0")"
clear
echo "Demo — writes a sample transcript file so you can see the shape."
echo ""
export INTERPRES_FRIENDLY=1
./interpres demo
echo ""
echo "Press Enter to close."
read -r _
EOF
chmod +x "$DIST/Try demo (no Live Captions needed).command"

# Settings helpers
cat > "$DIST/Turn saving ON.command" << 'EOF'
#!/bin/bash
cd "$(dirname "$0")"
./interpres remember on
echo ""
echo "Press Enter to close."
read -r _
EOF
chmod +x "$DIST/Turn saving ON.command"

cat > "$DIST/Turn saving OFF.command" << 'EOF'
#!/bin/bash
cd "$(dirname "$0")"
./interpres remember off
echo ""
echo "Press Enter to close."
read -r _
EOF
chmod +x "$DIST/Turn saving OFF.command"

cat > "$DIST/Check Live Captions (probe).command" << 'EOF'
#!/bin/bash
cd "$(dirname "$0")"
./interpres probe
echo ""
echo "Press Enter to close."
read -r _
EOF
chmod +x "$DIST/Check Live Captions (probe).command"

# --- macOS .app (double-click in Finder) ---
APP="$DIST/Interpres.app"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$DIST/interpres" "$APP/Contents/Resources/interpres"
chmod +x "$APP/Contents/Resources/interpres"
cp -R "$DIST/helpers" "$APP/Contents/Resources/helpers" 2>/dev/null || true
# App icon (zero crates — prebuilt .icns from assets/)
if [ -f "$ROOT/assets/Interpres.icns" ]; then
  cp "$ROOT/assets/Interpres.icns" "$APP/Contents/Resources/Interpres.icns"
fi
if [ -f "$ROOT/assets/logo.png" ]; then
  cp "$ROOT/assets/logo.png" "$APP/Contents/Resources/logo.png"
  cp "$ROOT/assets/logo.png" "$DIST/logo.png"
fi

# Launcher: run the real binary (native AppKit window — no Terminal, no webpage)
cat > "$APP/Contents/MacOS/Interpres" << 'EOF'
#!/bin/bash
RES="$(cd "$(dirname "$0")/../Resources" && pwd)"
cd "$RES" || exit 1
export INTERPRES_GUI=1
# No args → native window UI (Rust core + AppKit)
exec ./interpres
EOF
chmod +x "$APP/Contents/MacOS/Interpres"

cat > "$APP/Contents/Info.plist" << 'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key>
  <string>Interpres</string>
  <key>CFBundleDisplayName</key>
  <string>Interpres</string>
  <key>CFBundleIdentifier</key>
  <string>org.interpres.app</string>
  <key>CFBundleVersion</key>
  <string>0.2.0</string>
  <key>CFBundleShortVersionString</key>
  <string>0.2.0</string>
  <key>CFBundleExecutable</key>
  <string>Interpres</string>
  <key>CFBundleIconFile</key>
  <string>Interpres</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>LSMinimumSystemVersion</key>
  <string>13.0</string>
  <key>NSHighResolutionCapable</key>
  <true/>
  <key>NSHumanReadableCopyright</key>
  <string>Free open source. MIT OR Apache-2.0.</string>
</dict>
</plist>
EOF

# --- Windows double-click templates (use after building .exe on Windows) ---
cat > "$DIST/Open Interpres.bat" << 'EOF'
@echo off
cd /d "%~dp0"
title Interpres
echo ========================================
echo   Interpres - Live Captions companion
echo ========================================
echo.
echo   1) Turn on Live Captions: Win+Ctrl+L
echo   2) This window will start Interpres
echo.
pause
set INTERPRES_FRIENDLY=1
if exist interpres.exe (
  interpres.exe run
) else if exist interpres (
  interpres run
) else (
  echo interpres.exe not found. On Windows, build with:
  echo   cargo build --release
  echo then copy target\release\interpres.exe into this folder.
  pause
  exit /b 1
)
echo.
pause
EOF

cat > "$DIST/Try demo.bat" << 'EOF'
@echo off
cd /d "%~dp0"
if exist interpres.exe (interpres.exe demo) else (interpres demo)
pause
EOF

# START HERE for humans
cat > "$DIST/START HERE.txt" << 'EOF'
Interpres — for people who do not want to use a terminal

WHAT THIS IS
  A free helper that can save Windows / Mac Live Captions as text files.
  It is open source and not a paid product.

ON A MAC (this computer)
  Easiest:  double-click  Interpres.app
            → a normal Mac window opens (buttons, big text — not a website)
  Or:       double-click  “Open Interpres.command”
  First time Mac may ask to allow the app (right-click → Open if needed).

  In the window:
    1. Press “Check setup” if unsure
    2. Press “Start listening”
    3. Optional: “Save to disk: ON” and “Choose folder…”

  Also try:
    “Try demo (no Live Captions needed).command”  — sample transcript file
    “Check Live Captions (probe).command”         — is Live Captions on?

BEFORE YOU CAPTURE REAL CONVERSATIONS
  1. Turn on Live Captions
       Mac: System Settings → Accessibility → Live Captions
       Windows: Win + Ctrl + L
  2. On Mac, allow Accessibility for Interpres (and Terminal if you use CLI):
       System Settings → Privacy & Security → Accessibility
  3. Optional: turn on Save to disk in the window

ON WINDOWS
  1. On a Windows PC with Rust:  cargo build --release
  2. Copy target\release\interpres.exe into this same folder (or a copy of it)
  3. Double-click  Open Interpres.bat

WHERE FILES GO
  When saving is ON, transcripts land in:
    Documents → Interpres Transcripts
  One new file per session, named with date and time.

NEED HELP?
  Read README.md in this folder.
EOF

# Zip for sharing
ZIP="$ROOT/dist/Interpres-portable-macos.zip"
rm -f "$ZIP"
(cd "$ROOT/dist" && zip -r -q "Interpres-portable-macos.zip" "Interpres")

echo ""
echo "Done."
echo "  Folder: $DIST"
echo "  Zip:    $ZIP"
echo ""
echo "Double-click now:"
echo "  open \"$DIST\""
echo "  or open \"$DIST/Interpres.app\""
