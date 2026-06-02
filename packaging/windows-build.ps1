# Build an easy, standalone Windows test .exe for Interpres (no installer needed).
# Run this from PowerShell in the repo root on your Windows test machine.
#
# This produces a binary with the "debug-logs" feature loaded:
#   - All important events, plugin protocol chatter (stdout/stderr), device changes,
#     capture status, errors, and panics are saved to `interpres-test-debug.log.txt`
#     placed right next to the .exe (dead simple to find, zip, and send for investigation).
#   - Verbose mode is forced.
#   - A console window pops on launch (for the debug/test build) so you see the log path
#     immediately + live output.
#
# After build you can zip the whole "dist\interpres-test\" folder and run on any Windows
# laptop without installing anything (except Python 3 for the current dev plugins).
#
# Requirements on the Windows box: Rust (rustup), Python 3 on PATH.

$ErrorActionPreference = "Stop"
Set-Location (Split-Path $PSScriptRoot -Parent)

Write-Host "==> Building EASY TEST .exe with full debug logging to .txt (gui + debug-logs)..."
# Use release for a smaller/faster test exe, but with debug logging feature enabled.
# (If you want full debug symbols for crash investigation, remove --release.)
cargo build --release --features "gui,debug-logs"

$bin = "target\release\interpres.exe"
if (-not (Test-Path $bin)) {
    Write-Error "Binary not found at $bin — build failed?"
}

# Prepare a self-contained test bundle (exe + the dev tools it needs at runtime + instructions).
$bundle = "dist\interpres-test"
New-Item -ItemType Directory -Force -Path $bundle | Out-Null
New-Item -ItemType Directory -Force -Path "$bundle\tools" | Out-Null

Copy-Item $bin $bundle\
Copy-Item tools\dev_transcriber.py "$bundle\tools\"
Copy-Item tools\dev_tts.py "$bundle\tools\"
# Also copy requirements if user wants real faster-whisper
if (Test-Path tools\requirements-dev.txt) {
    Copy-Item tools\requirements-dev.txt "$bundle\tools\"
}

# A tiny readme for the tester (double-click friendly).
$readme = @"
INTERPRES — Windows Test .exe (debug logs enabled)

This is a standalone test build. No install required.

HOW TO RUN:
1. Make sure this folder contains:
     interpres.exe
     tools\
       dev_transcriber.py
       dev_tts.py
   (Python 3 must be installed and on PATH for the dev plugins.)

2. (Recommended for calls) Install VB-CABLE: https://vb-audio.com/Cable/
   Then in the app, set Input to the CABLE Output, Output to your real speakers or another CABLE.

3. Double-click interpres.exe  (or run from cmd/PowerShell).
   - A console window will appear (because this is the debug-logs test build).
   - It will print the path to the debug log file.
   - The giant floating subtitles window (or control + overlay) will open.

4. The file  interpres-test-debug.log.txt  will be created RIGHT NEXT TO interpres.exe .
   It contains *everything* (startup, device choices, every PARTIAL/FINAL/AUDIO/ERROR from plugins,
   AI speak roundtrips, capture restarts, panics with backtrace if RUST_BACKTRACE=1, etc.).
   After a test run, zip that .txt (and the exe if you want) and send it for analysis.

LAUNCH MODES (from the console or shortcuts):
  interpres.exe
  interpres.exe --subtitles-only     (pure giant always-on-top subs, minimal UI — the "star")

TIPS:
- First run will show the Welcome wizard. Enter your name (used in "AI on my behalf" announcements).
- Click START SESSION before expecting live captions.
- Use the composer in "AI on My Behalf" mode to test the full TTS roundtrip + labeled subs.
- To get richer panic backtraces: set the env var RUST_BACKTRACE=1 before running, or
  run from cmd:  set RUST_BACKTRACE=1 && interpres.exe --subtitles-only

This build was produced with the "debug-logs" Cargo feature + the supporting code in
src/util/logging.rs (file next to exe, panic hook, forced verbose, key events logged).

See MD/DEPLOY-READINESS.md and packaging/README.md for more.
"@

$readme | Out-File -Encoding UTF8 -FilePath "$bundle\TEST-RUN.txt"

Write-Host ""
Write-Host "============================================================"
Write-Host "SUCCESS: Easy test bundle ready at: $bundle"
Write-Host "  interpres.exe   (with debug-logs: writes interpres-test-debug.log.txt beside it)"
Write-Host "  tools\          (dev plugins — Python required on this machine for now)"
Write-Host "  TEST-RUN.txt    (open this)"
Write-Host ""
Write-Host "Just zip the interpres-test folder and copy to any Windows laptop for testing."
Write-Host "VB-CABLE: https://vb-audio.com/Cable/"
Write-Host "============================================================"

# Also drop a copy of the plain exe in dist for convenience
New-Item -ItemType Directory -Force -Path dist | Out-Null
Copy-Item $bin dist\interpres-gui-debug-logs.exe -Force
Write-Host "Also available: dist\interpres-gui-debug-logs.exe (raw)"