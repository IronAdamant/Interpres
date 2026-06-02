# Interpres Packaging (Phase 5)

## Quick release build

| Platform | Command |
|----------|---------|
| Linux / macOS | `./packaging/build-release.sh` |
| Windows | `powershell -File packaging/windows-build.ps1` |

Outputs: `target/release/interpres` (headless). GUI uses the same binary with `--features gui` at compile time — build with `cargo build --release --features gui`.

## Easy "no-install" test .exe with full debug logging (what you asked for)

For laptop testing (especially on Windows):

```powershell
# On Windows, in repo root:
powershell -File packaging\windows-build.ps1
```

This builds with `--features "gui,debug-logs"` (see Cargo.toml) and produces a ready-to-zip folder:

```
dist/interpres-test/
  interpres.exe
  tools/dev_transcriber.py
  tools/dev_tts.py
  windows-build.ps1   (in case you want to re-run)
  TEST-RUN.txt        (open this — explains the magic log file)
```

Key debug feature:
- When you run the exe, `interpres-test-debug.log.txt` is created **right next to the .exe**.
- Contains startup, every plugin message (PARTIAL/FINAL/SPEAK/AUDIO_OUT/ERROR/LOG/...), device changes, capture supervisor decisions, AI label roundtrips, panics + backtraces, thin-client URLs, etc.
- Console window pops automatically for the test build (you see the log path immediately).
- Zero dependencies for the binary itself (Python only needed while we still use the dev_*.py plugins).

See `dist/interpres-test/TEST-RUN.txt` (after you run the ps1) and `MD/DEPLOY-READINESS.md` for the full checklist.

Normal users will later get a clean release without the forced console + verbose file (the feature is opt-in for test builds).

## Install layout (recommended)

```
~/.local/share/interpres/     # or %LOCALAPPDATA%\Interpres
  models/                     # STT/TTS model files
  plugins/                    # optional user plugins
~/.config/interpres/
  settings.conf               # user settings (auto-created)
```

## Platform notes

### Windows (primary target)

- Install [VB-CABLE](https://vb-audio.com/Cable/) for routing Zoom/Teams audio.
- Grant microphone permission when prompted.
- **Installer (future)**: WiX Toolset or `cargo install cargo-wix` → bundle `interpres.exe`, `tools/`, default plugin folders.
- Code signing: Authenticode for SmartScreen trust.

### macOS

- Build: `cargo build --release --features gui`
- Bundle as `.app`: wrap binary + `Info.plist` (mic usage string required).
- Call routing: [BlackHole](https://existential.audio/blackhole/) or Loopback.
- Notarization required for distribution outside dev machines.

### Linux

- PipeWire/ALSA via `cpal`.
- Always-on-top: best on **X11** (`WINIT_UNIX_BACKEND=x11` if Wayland limits stacking).
- Optional `.desktop` file for launcher (see `packaging/interpres.desktop`).

## Models

Run `tools/setup_models.sh` for download instructions (Parakeet ONNX, Piper, etc.). Production builds should ship or download models into `models_dir()` — not bundled in git.

## Dependencies in shipped binary

- **Always**: `cpal` (audio)
- **GUI build**: `egui` + `eframe` (optional feature)
- **Dev plugins**: Python 3 + `tools/dev_transcriber.py` / `dev_tts.py` (production uses native plugin binaries)