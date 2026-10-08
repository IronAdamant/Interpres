# Interpres

<p align="center">
  <img src="assets/logo.png" alt="Interpres logo" width="128" height="128" />
</p>

**Save what Windows Live Captions shows, so you have the words after the meeting.**

Interpres is a free helper for people who use **Windows Live Captions**.
It does **not** listen to your microphone or caption anything itself.
It reads the text Live Captions already puts on screen and saves it to a plain text file on your PC.

No account. No cloud. Everything stays on your computer unless **you** move the files.

<p align="center">
  <img src="assets/screenshot-windows.png" alt="Interpres recording a meeting: green Recording banner, Stop button, checklist, and a transcript with times" width="720" />
</p>

---

## What it is / isn’t

| Interpres **is** | Interpres **is not** |
|------------------|----------------------|
| A local helper that **records** what Live Captions shows | A speech-to-text engine |
| Able to **save** captions while Live Captions works | A guarantee of perfect words |
| Loud when it **can’t** read captions, so you know | A replacement for Live Captions |

---

## Quick start (Windows 11)

1. **Download** the Windows zip from **[Releases](https://github.com/IronAdamant/Interpres/releases)** and unzip it anywhere.
2. **Open** `interpres.exe`. There is nothing else to install.
3. **Live Captions:** if the banner says *Live Captions is off*, press **Turn on Live Captions** (or press **Win + Ctrl + L**).
4. Press **Start recording** before your meeting.
5. Press **Stop recording** when you’re done. Interpres saves the sentence still being spoken before it closes the file.

Then use **Open transcript** or **Copy all** to move the text into your notes.

---

## Reading the banner

The coloured banner tells you whether your words are being saved. The window title and the taskbar button show the same state.

| Banner | Meaning | What to do |
|--------|---------|------------|
| 🟩 **Recording · 12:40 · 48 lines** | Captions are being read and saved | Nothing |
| 🟦 **Ready — waiting for speech** | Live Captions is on but nobody is talking yet | Nothing |
| 🟥 **Live Captions is off** | Nothing can be saved | Press **Turn on Live Captions** |
| 🟥 **Not capturing — can’t read Live Captions** | Live Captions is stuck; captions are being missed | Press **Restart Live Captions** |
| 🟧 **No new captions for 3 min — are you done?** | It has gone quiet | **Stop & save**, or **Keep recording** |

When the banner turns red, the taskbar button flashes and Windows plays a warning sound, so you notice even when another window is in front.

**Interpres never stops recording by itself.** When it goes quiet, it asks and waits for your answer. You can change the wait (or turn the question off) with `idle_prompt_minutes` in the settings file. See [Settings file](#settings-file).

If Live Captions closes or restarts during a meeting, recording carries on in **the same file** and a note marks the gap.

---

## Your files

Each recording creates one dated text file in your transcripts folder (by default **Documents → Interpres Transcripts**):

```text
2026-10-08_17-59-54.txt
```

```text
# Interpres session started 2026-10-08 17-59-54
# Source: Windows Live Captions
# Folder: C:\Users\you\Documents\Interpres Transcripts

[18:04:07] If I unmute myself.
[18:04:10] How are you?
# [18:09:12] Live Captions turned off — nothing captured until it is back on
# [18:09:20] Live Captions back on
[18:09:24] So that's the premise.

# Session ended (user)
```

Times are your local time and show when each line was first heard.

---

## Settings ▾

| Item | What it does |
|------|--------------|
| Save transcripts to disk | On/off. When off, captions show in the window but are not kept |
| Change / Open transcripts folder | Choose or open where files go |
| Captions from | Windows Live Captions, or your own engine (see below) |
| Edit settings file… | Opens `settings.conf` in Notepad |
| Check Live Captions setup | Checks whether Interpres can read Live Captions right now |
| Restart Live Captions | Closes and reopens Live Captions (fixes a frozen captions window) |
| Theme | Match Windows, light, or dark |
| Write debug log | Writes a troubleshooting log next to your transcripts |

---

## Use your own speech engine (advanced)

Live Captions is the easy option and needs no setup. If you run your own speech-to-text model (for example **Phonon-2**, **Parakeet** or **Whisper**), Interpres can save its output instead, with the same banner, prompts and files.

Interpres doesn’t bundle or download any model. You point it at your engine program in the settings file, and the engine prints caption lines that Interpres saves. Interpres itself stays zero-dependency; your engine brings its own. This works on Windows and macOS.

```ini
source=engine
helper_path=C:\Users\you\AppData\Local\Programs\Python\Python313\python.exe
helper_args=-u "C:\engines\my_engine.py"
```

See **[docs/ENGINES.md](docs/ENGINES.md)** for the line format and a standard-library Python example engine ([`examples/engines/example_engine.py`](examples/engines/example_engine.py)). Switch between Live Captions and your engine under **Settings ▾ → Captions from**.

---

## Privacy

- Captions and transcripts stay **on your PC** unless **you** move them.
- Saving is **on** by default (that’s the point of the app). Turn it off under **Settings ▾ → Save transcripts to disk** to only watch captions in the window.
- Interpres makes no network connections.

---

## Common questions

**A word in the file is wrong.**
Live Captions misheard it. Interpres saves exactly what Live Captions showed.

**The banner is red and says “can’t read Live Captions”.**
Press **Restart Live Captions**. Recording carries on in the same file. If it keeps happening, turn on **Write debug log** in Settings and keep the `.debug.log` file that appears next to the transcript.

**Do I need PowerShell or any helper files?**
No. Interpres reads Live Captions directly through Windows UI Automation. `interpres.exe` is all you need.

**Mac?**
A macOS build exists but has not had this redesign yet. This guide covers Windows.

**Linux?**
Not supported.

---

## Known issues

- Live Captions sometimes changes earlier words after the fact. Interpres keeps the most polished version it sees, but occasionally a line is saved twice with small differences, or slightly out of order.
- Interpres can only save what Live Captions shows. If Live Captions isn’t showing anything, the banner says **Ready — waiting for speech** and nothing is being saved.

---

## For developers

Rust, zero crates.io dependencies. Native Win32 UI; captions are read in-process through UI Automation COM (`src/platform/windows_uia.rs`) with 1.5 s connect and 2.5 s per-call timeouts and a watchdog.

```text
cargo test
cargo build --release
powershell -NoProfile -ExecutionPolicy Bypass -File packaging\make-windows-release.ps1
```

The `x86_64-pc-windows-gnu` toolchain needs a MinGW with `libgcc` (for example WinLibs) first on `PATH` to link tests. llvm-mingw alone fails with `unable to find library -lgcc_eh`.

| File | Purpose |
|------|---------|
| `src/platform/windows_uia.rs` | Hand-written UI Automation COM bindings |
| `src/platform/windows.rs` | Reader thread, watchdog, turn on / restart Live Captions |
| `src/health.rs` | Banner states and the “are you done?” prompt (shared with macOS) |
| `src/engine.rs` | Capture loop, external-engine loop, session files, end-of-recording drain |
| `src/plugin_host.rs` | Runs an external engine and reads its protocol lines |
| `src/buffer.rs` | Turns the rolling caption text into finished lines |
| `src/gui_win.rs` | Windows UI |

Diagnostics: `interpres diagnose` prints what Interpres can read right now. With Live Captions open, `cargo test --lib dump_live_surface -- --ignored --nocapture` dumps the raw caption text line by line.

### Settings file

`%APPDATA%\Interpres\settings.conf`, `key=value` lines:

| Key | Default | Meaning |
|-----|---------|---------|
| `remember` | `true` | Save transcripts to disk |
| `transcript_folder` | Documents\Interpres Transcripts | Where files go |
| `idle_prompt_minutes` | `3` | Ask “are you done?” after this many quiet minutes (`0` = never) |
| `debug` | `false` | Write `interpres-debug.log` and per-session `.debug.log` |
| `theme` | `system` | `system`, `light`, or `dark` |
| `write_jsonl` | `false` | Also write a machine-readable `.jsonl` next to each transcript |
| `source` | `os` | `os` = Live Captions, `engine` = your own engine ([docs/ENGINES.md](docs/ENGINES.md)) |
| `helper_path` | (empty) | Engine program, as a full path |
| `helper_args` | (empty) | Engine arguments; double quotes group paths with spaces |

Optional CLI: `interpres run`, `probe`, `diagnose`, `remember on|off`, `set-folder`, `demo`, `help`.

License: **MIT OR Apache-2.0**.
