# Interpres

<p align="center">
  <img src="assets/logo.png" alt="Interpres logo" width="128" height="128" />
</p>

**Save what Windows Live Captions already shows — so you can read it again later.**

Interpres is a free helper for people who use **Windows Live Captions**.  
It does **not** replace Live Captions and does **not** listen to your microphone by itself.  
It works **with** Live Captions and can **save** those words into a simple text file on your PC.

No account. No cloud. Everything stays on your computer unless **you** move the files.

---

## What it is / isn’t

| Interpres **is** | Interpres **is not** |
|------------------|----------------------|
| A local helper that **records** Live Captions | A speech-to-text engine by itself |
| Able to **save** captions when they work | A guarantee of perfect words |
| Meant to work **with** Windows Live Captions | A replacement for Live Captions |

You turn on **Live Captions first**. Interpres cannot caption audio alone.

---

## Easy start (Windows)

### 1. Download

Open **[Releases](https://github.com/IronAdamant/Interpres/releases)** and download the **Windows** zip (for example `Interpres-portable-windows.zip`).

### 2. Unzip

Extract the folder anywhere (Desktop, Downloads, etc.).

### 3. Turn on Live Captions

Press **Win + Ctrl + L**  
(or: Settings → Accessibility → Captions → Live captions)

Play a video or call so captions appear on screen.

### 4. Run Interpres

1. Double-click **`interpres.exe`** (or **Open Interpres.bat**).  
2. Press **Start listening**.  
3. Optional: turn **Save to disk: ON** and pick a folder (default is often Documents → Interpres Transcripts).  
4. When you finish, press **Stop**.

You should **not** see terminal windows flashing while listening. If captions show on Windows but Interpres stays empty, try **Check Live Captions (probe).bat** or **Diagnose.bat** in the same folder.

### 5. Your files

When saving is **ON**, each session creates a dated text file, for example:

`2026-08-09_14-22-01.txt`

---

## Privacy

- Captions and transcripts stay **local** unless **you** move them.  
- Saving to disk can stay **off** until you turn it on.  
- Interpres is **not** a cloud speech service.

---

## Common questions

**Why is a word wrong in the file?**  
Live Captions guessed wrong. Interpres saved what captions showed.

**Why is Live empty?**  
Turn Live Captions on (**Win+Ctrl+L**), play audio, then press **Start listening** again.

**Do I need the PowerShell helper file?**  
The release zip includes it. Newer builds can also recreate it automatically if the file is missing.

**Mac?**  
A Mac build may be available on Releases. This guide focuses on Windows.

**Linux?**  
Not supported officially.

---

## For developers

Zero crates.io dependencies. Native Win32 UI on Windows.

```text
cargo test
cargo build --release
powershell -NoProfile -ExecutionPolicy Bypass -File packaging\make-windows-release.ps1
```

Optional CLI: `interpres run`, `probe`, `diagnose`, `remember on|off`, `set-folder`, `demo`, `help`.

License: **MIT OR Apache-2.0**.

---

## Status

**v0.2.x** — Windows Live Captions companion: native window, opt-in dated session files, local only. Best-effort capture of the OS caption surface.
