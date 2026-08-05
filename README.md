# Interpres

**Save what Live Captions say — so you can read it again later.**

Interpres is a free, open-source helper for **Deaf and hard of hearing** people on **Windows** and **Mac**. It does not replace your captions. It works **with** the Live Captions your computer already has.

This project was **rebuilt from the ground up**. After real meetings, interviews, and everyday conversations, it became clear that **Windows Live Captions** and **macOS Live Captions** already work well for *reading* speech live. What was missing was a simple, local way to **keep a transcript** — one file per session, with the date and time — so you can remember what was said.

No subscription. No cloud account. No paid product. Public open source for anyone to use or fork.

---

## What it does

| You already have | Interpres adds |
|------------------|----------------|
| Live Captions on screen | Optional **saved transcript** |
| Great for the moment | Great for **later** (notes, follow-up, “what did they say?”) |

- Works with **Windows 11 Live Captions** and **Mac Live Captions**
- **You choose the folder** for transcripts (it remembers that folder)
- **One new file per session**, named with **date and time** (for example `2026-08-05_14-22-01.txt`)
- Saving is **off until you turn it on** (your privacy, your choice)
- Everything stays **on your computer**

---

## Easy start (non-technical) — double-click, no terminal

You do **not** need the command line for day-to-day use.

### Download a ready build (Mac)

**GitHub Releases** (compiled app folder + bare binary):

**https://github.com/IronAdamant/Interpres/releases**

Latest: **[v0.2.0](https://github.com/IronAdamant/Interpres/releases/tag/v0.2.0)** — unzip **Interpres-portable-macos-aarch64.zip**, open **START HERE.txt**, double-click **Interpres.app**.

Builds are **not obfuscated**. Curious people can rebuild from source and compare checksums — see [docs/VERIFY.md](docs/VERIFY.md). Windows users: build from the same tag with `cargo build --release` until a Windows `.exe` is attached to a future release.

### Build a clickable app folder yourself (any Mac with Rust)

Someone technical (or you, once) runs:

```text
./packaging/make-double-click.sh
```

That creates **`dist/Interpres/`** with:

| Double-click this | What it does |
|-------------------|--------------|
| **Interpres.app** (Mac) | Opens Terminal and starts Interpres |
| **Open Interpres.command** (Mac) | Same idea without a full .app |
| **Open Interpres.bat** (Windows) | After you place `interpres.exe` in the folder |
| **Try demo…** | Sample transcript — no Live Captions needed |
| **Turn saving ON** | Start saving sessions to disk |
| **START HERE.txt** | Short plain-language guide |

Open the folder:

```text
open dist/Interpres
```

Then **double-click `Interpres.app`** (or `Open Interpres.command`).  
First time on a Mac: if Gatekeeper blocks it, **right-click → Open**.

A zip is also made: `dist/Interpres-portable-macos.zip` — you can copy that folder to another Mac of the same kind (Apple silicon vs Intel must match the build).

### 1. Turn on Live Captions

**Windows 11**

- Press **Win + Ctrl + L**, or  
- Settings → Accessibility → Captions → **Live captions**

**Mac**

- System Settings → Accessibility → **Live Captions** → turn on  
- If capture is empty: System Settings → Privacy & Security → **Accessibility** → allow **Interpres** and/or **Terminal**

### 2. Start Interpres

- **Preferred:** double-click **Interpres.app** or **Open Interpres.command** in `dist/Interpres`  
- **Windows:** double-click **Open Interpres.bat** (with `interpres.exe` in the same folder)

### 3. Saving (optional)

Double-click **Turn saving ON.command**, or on Windows run `interpres.exe remember on`.  
Files go under **Documents → Interpres Transcripts** by default (one new file per session, date and time in the name).

### For people who like the terminal

```text
cargo run --release
# or after make-double-click.sh:
./dist/Interpres/interpres probe
./dist/Interpres/interpres set-folder "/Users/you/Documents/My Meeting Notes"
./dist/Interpres/interpres remember on
./dist/Interpres/interpres run
```

---

## Common commands

| Command | Meaning |
|---------|---------|
| `interpres run` | Capture while Live Captions is on (default) |
| `interpres probe` | Self-check (is Live Captions running? any permission issue?) |
| `interpres set-folder PATH` | Sticky transcript folder |
| `interpres remember on` / `off` | Save to disk or not |
| `interpres show-config` | Show current settings |
| `interpres demo` | Create a sample transcript **without** Live Captions |
| `interpres watch` | Print when Live Captions starts or stops |
| `interpres help` | Short help |

---

## Privacy

- Captions and transcripts stay **local** unless **you** copy files somewhere else  
- Disk saving defaults to **off**  
- Interpres is **not** a speech service in the cloud  

Windows and Mac do not officially “export” Live Captions. Interpres reads the caption UI the accessibility way so *you* can keep a personal record.

---

## For developers

- **Strict zero third-party Rust crates** in the core (`Cargo.toml` has an empty `[dependencies]`)  
- Platform access: process checks + hand-written OS APIs / small helpers under `helpers/`  
- Plan of record: [`MD/LIVE-CAPTIONS-REBUILD-PLAN.md`](MD/LIVE-CAPTIONS-REBUILD-PLAN.md)  
- Protocol / signal notes: [`docs/PROTOCOL.md`](docs/PROTOCOL.md), [`docs/SIGNALS.md`](docs/SIGNALS.md)  
- License: **MIT OR Apache-2.0** — fork freely  

```text
cargo test
cargo run -- probe
cargo run -- demo
```

---

## Accessibility keywords (plain language)

If you are searching for tools around **live captions**, **real-time captions**, **offline transcripts**, **Deaf accessibility**, **hard of hearing**, **Windows 11 Live Captions**, **Mac Live Captions**, or **meeting caption history**, Interpres is a small local companion: it does not try to be another AI meeting bot — it helps you **keep the words** your system already shows.

---

## Status

Ground-up rebuild (v0.2): Live Captions companion, dated session files, sticky folder, probe, strict zero-dependency core. Optional polish (installers, tray watcher packaging) can grow on top without changing that core idea.
