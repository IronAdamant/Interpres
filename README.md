# Interpres — Privacy-First Local AI Communication Appliance

**For Deaf and Hard of Hearing users. 100% local. No cloud. No human operators.**

Interpres is a fully on-device AI relay that helps with sensitive calls (banking, medical, legal, personal) using real-time transcription and three reply modes — all on your machine.

## Core Promise (Non-Negotiables)

- **100% local** — No STT, TTS, LLM, or audio leaves your device in production.
- **No audio retention** unless you explicitly opt in.
- **Deaf/HoH first** — Large high-contrast text, minimal clicks, big obvious controls.
- **Simple for non-technical users** — One installer (planned), model setup guide, drag-and-drop plugins.
- **Open source core** (MIT/Apache-2.0).
- English primary in the free tier.

## Three Reply Modes (Per Session)

| Mode | Description | What the other party hears |
|------|-------------|----------------------------|
| **Trusted Calls** | Live subtitles; you speak in your own voice. | Your real voice |
| **Voice Protection** | Subtitles + voice changer (stub labels in v1). | Changed voice (full changer planned) |
| **AI on My Behalf** | You type; TTS speaks with required announcement prefix. | "This is an AI speaking on behalf of …" |

## Two Ways to See Subtitles

1. **Laptop — floating always-on-top window** (recommended)  
   `cargo run --features gui` — giant high-contrast subtitles stay on top of Zoom, Teams, browsers.

2. **Phone / tablet — thin client**  
   Open `http://<your-pc-ip>:43123/` on the same Wi‑Fi (pure HTTP+SSE, no install).

## Current Status (2026)

**Working today (zero-dep core + optional GUI):**

- Real microphone capture + VAD + utterance pipeline (`cpal` only mandatory crate)
- Python dev transcriber plugin (or demo mode without faster-whisper)
- Pure-std HTTP+SSE thin-client server
- Native GUI: control panel + **always-on-top** subtitle overlay (`--overlay` mode)
- AI-on-behalf composer + dev TTS stub
- Settings persistence (`~/.config/interpres/settings.conf`)
- Easy standalone Windows test .exe via `packaging/windows-build.ps1` (includes `--features "gui,debug-logs"` with full local logging to `interpres-test-debug.log.txt` next to the exe)

**Not production-certified for critical calls yet** — complete QA on your OS after Phase 5.

## Quick Start (from source)

```bash
# Prerequisites: Rust 1.80+, Python 3 for dev plugins
git clone <repo> && cd interpres

# Headless (console + phone subtitles)
cargo run

# Laptop GUI + floating subtitles
cargo run --features gui

# Pure floating subtitles only (giant text, almost no UI surface; one window)
cargo run --features gui -- --subtitles-only  # or --pure / --floating-only / --overlay (alias)
```

For a ready-to-run Windows .exe (with debug logs to .txt beside it for test investigation):

```powershell
powershell -File packaging\windows-build.ps1
```

See **[BUILD.md](BUILD.md)** for platform audio routing (VB-CABLE, BlackHole, PipeWire).

Models: run `./tools/setup_models.sh` for download paths.

## Building an Easy Standalone Test .exe (with Full Debug Logging)

For laptop testing (no install, just copy and run the .exe):

On Windows (with `rustup` + Python 3 on PATH):

```powershell
powershell -File packaging\windows-build.ps1
```

This builds using `cargo build --release --features "gui,debug-logs"` and stages:

- `dist/interpres-test/interpres.exe` (the portable test binary)
- `tools/` (dev plugins needed at runtime for now)
- `TEST-RUN.txt` (instructions + test checklist)

Key debug feature:
- `interpres-test-debug.log.txt` is automatically created **right next to the .exe** on every run.
- Captures *everything* locally: plugin protocol (all SPEAK/PARTIAL/FINAL/AUDIO_OUT/ERROR/LOG lines), device selection & reliability events, capture supervisor, TTS/AI label roundtrips, startup URLs, panics with backtraces (set `RUST_BACKTRACE=1`), etc.
- For the debug/test build, a console window pops showing the log file path + live output.
- Perfect for investigation on real hardware (Mac/Win/Linux notes still apply via the exe).

See the generated `TEST-RUN.txt` inside the bundle, [packaging/README.md](packaging/README.md), and [MD/DEPLOY-READINESS.md](MD/DEPLOY-READINESS.md) for the full pre-test list.

(The `debug-logs` feature is opt-in; normal builds stay lean and quiet.)

## Tech Stack (Zero-Dep Pivot)

| Layer | Implementation |
|-------|----------------|
| **Core** | Pure Rust `std` — VAD, plugin host, HTTP+SSE server, config |
| **Audio** | `cpal` (single required dependency) |
| **GUI** | `egui` / `eframe` (optional `--features gui` only) |
| **Plugins** | Process-based text protocol; `plugin.yaml` discovery |
| **Legacy UI** | Old Tauri+Svelte scaffold in `legacy/` (reference only) |

Direct dependencies: **1** (headless) or **3** (with GUI). See [ZERO-DEP-PLAN.md](ZERO-DEP-PLAN.md).

## Repository Layout

```
interpres/
├── src/                 # Zero-dep core + engine + optional gui.rs
├── plugins/             # Example plugin.yaml trees
├── tools/               # dev_transcriber.py, dev_tts.py, setup_models.sh
├── packaging/           # Release build scripts + installer notes
├── MD/                  # Phase plans & architecture synthesis
├── legacy/              # Abandoned Tauri stack (UX reference)
├── BUILD.md
├── ARCHITECTURE.md      # High-level design (being updated)
└── PHASE1-STATUS.md … PHASE5-STATUS.md
```

## Documentation

- [BUILD.md](BUILD.md) — Build & run (Windows / macOS / Linux)
- [MD/PHASES-OVERVIEW.md](MD/PHASES-OVERVIEW.md) — Phase roadmap
- [MD/ALWAYS-ON-TOP-SUBTITLES-ARCHITECTURE.md](MD/ALWAYS-ON-TOP-SUBTITLES-ARCHITECTURE.md) — Overlay design
- [docs/PLUGIN-PROTOCOL.md](docs/PLUGIN-PROTOCOL.md) — Plugin wire format
- [packaging/README.md](packaging/README.md) — Release & installers

## License

MIT OR Apache-2.0. Model weights follow their upstream licenses.