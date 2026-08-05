# Interpres Complete Rebuild Plan — OS Live Captions Companion

**Status:** Implemented as Interpres v0.2 (ground-up rebuild). This file remains the architecture plan of record.  
**Date:** 2026-08-05  
**Audience:** Maintainers / contributors  
**Supersedes (product direction):** mic-first STT pipeline as the primary path. The rebuild centers on **native OS Live Captions** (Windows 11 + macOS).

---

## 1. Product intent (north star)

For Deaf / hard-of-hearing users on **Windows PC/laptops** and **macOS laptops**:

1. **Hook** the Live Captions experience they already use and that already works well in real meetings, interviews, and friend calls.
2. **Capture** caption text so it can be **saved and remembered** (the OS does not export transcripts).
3. **Lifecycle:** automatically open the companion when Live Captions is running; stop / close when Live Captions is not (both OSes).
4. Stay **100% local**, **open source**, and **strict zero-dependency** in the Interpres core.
5. Later (out of scope for rebuild v1 code, but not forbidden by architecture): act on saved text (search, lookup). Rebuild v1 only needs a clean text stream + durable sessions to enable that.

**Non-negotiable product frame:** Interpres is a **companion to OS Live Captions**, not a replacement speech engine. Local STT is a **fallback**, never the default primary source in v1.

---

## 2. Strict zero-dependency definition (hard constraint)

### 2.1 What “strict zero-dep” means for this rebuild

| Rule | Required |
|------|----------|
| **crates.io / Cargo third-party crates in the main Interpres binary** | **Zero.** `Cargo.toml` `[dependencies]` for the core package must be empty (or only path-internal crates that are also zero third-party). |
| **`std` only for portable logic** | Yes: threads, channels, fs, process, net, time, sync. |
| **Hand-written platform FFI** | Allowed: `extern "system"` / `#[link]` to **OS-provided** libraries only (e.g. `user32`, `ole32`, `UIAutomationCore` on Windows; `ApplicationServices`, `CoreFoundation` on macOS). **Not** crates.io wrappers (`windows`, `objc`, `accessibility`, etc.) in core. |
| **Process plugins / helpers** | Allowed. Spawned with `std::process::Command`. May be written in Rust (zero crates + FFI), C/C++, Swift, C#, or Python **using only OS SDKs / stdlib**. Prefer no third-party package manager deps in helpers either. |
| **Historical exceptions (`cpal`, `egui`/`eframe`)** | **Not assumed. Not permitted in strict rebuild v1 core.** Audio I/O and third-party GUI frameworks are out of the critical path for captions-first v1. |
| **Optional GUI later** | If a GUI is needed in v1, use **hand-written** Win32 / AppKit (or pure-std headless + thin-client HTML first). Adding any crates.io GUI is a **budget violation** and requires an explicit plan amendment, not silent drift. |

### 2.2 What pure `std` cannot do (and how we still ship)

| Need | Pure `std` alone? | Allowed path under strict zero-dep |
|------|-------------------|-------------------------------------|
| Read Windows Live Captions text | No | Process plugin **or** hand-written UIA COM FFI |
| Read macOS Live Captions text | No | Process plugin **or** hand-written AX FFI |
| Detect Live Captions process | No (portable API) | FFI process enumeration **or** tiny helper; polling OK |
| Save JSONL/TXT/SRT/VTT | **Yes** | `std::fs` only |
| HTTP+SSE thin client | **Yes** | Existing pure-std pattern |
| Auto open/close | Needs OS signals | Light watcher via FFI / helper + `Command` to launch main |

**Contradiction resolution (must not be reopened as “impossible”):**  
Strict zero-dep = **zero third-party crates**, **not** “zero platform APIs.” Platform scrape lives in **plugins or in-tree FFI**, never as “add a crate to make it easy.”

### 2.3 Cargo / tree policy for the rebuild

```text
interpres/                 # main package: [dependencies] = empty
  src/                     # pure std + optional cfg FFI modules
  helpers/                 # optional native helpers (no crates.io)
  plugins/captions/        # line-protocol capturers (Win / Mac)
  tools/                   # build scripts, smoke tests (may use system tools)
```

- No `Cargo.lock` third-party entries for the main binary.
- CI / local check: `cargo tree -p interpres` shows only std / build-std (and no external crates).
- Document this check in the plan’s verification section; implement when `/goal` runs.

---

## 3. Research baseline (do not re-litigate)

Research package (2026-08-05) established:

| Fact | Implication |
|------|-------------|
| No official MS/Apple Live Captions **export/subscribe API** | Scrape is required |
| Windows: UIA `CaptionsTextBlock` under `LiveCaptionsDesktopWindow` / `LiveCaptions.exe` is **proven** by multiple OSS projects | Primary Win path |
| macOS: agent `com.apple.accessibility.LiveTranscriptionAgent` + **AX** tree | Primary Mac path; Accessibility TCC required |
| Auto open/close is **yes (conditional)** on both OSes | Requires **light always-on watcher** |
| Save is pure-std | JSONL canonical + TXT human |

Primary external anchors (implementers must re-verify AutomationIds / bundle IDs on target OS versions):

- Microsoft Support: Live Captions on-device, not stored — no export.
- Microsoft UI Automation (Win32) docs.
- Apple Support Live Captions; Apple AXUIElement docs.
- Precedents: SakiRinn/LiveCaptions-Translator; corbamico/get-livecaptions-rs/cpp; SaveLiveCaptions.

---

## 4. Target architecture

### 4.1 Process model

```text
┌─────────────────────────────────────────────────────────────┐
│ interpres-watcher  (login item / Startup / tray-light)      │
│  - Poll process presence of OS Live Captions                │
│  - Debounced on/off edges                                   │
│  - Launch / signal / quit main companion                    │
│  - Strict zero-dep (std + minimal FFI or tiny OS helper)    │
└──────────────────────────┬──────────────────────────────────┘
                           │ start / stop / already-running
┌──────────────────────────▼──────────────────────────────────┐
│ interpres  (main; 0 crates)                                 │
│  - Session manager, privacy gates                           │
│  - CaptionSource trait → plugin host                        │
│  - TranscriptWriter (JSONL/TXT; optional SRT/VTT export)    │
│  - Optional pure-std HTTP+SSE for phone thin client         │
│  - Optional headless or hand-written overlay later          │
└───────────────┬─────────────────────────────┬───────────────┘
                │ stdin/stdout line protocol  │
                ▼                             ▼
   ┌────────────────────────┐    ┌────────────────────────────┐
   │ captions-win helper    │    │ captions-mac helper        │
   │ UIA poll + line-diff   │    │ AX walk/observer + diff    │
   └───────────┬────────────┘    └─────────────┬──────────────┘
               ▼                               ▼
        LiveCaptions.exe              Live Captions.app agent
```

### 4.2 Core modules (logical; names may adjust at implement time)

| Module | Responsibility | Deps |
|--------|----------------|------|
| `config` | Hand-written settings (remember on/off, paths, debounce ms) | std |
| `session` | Session id, start/end, active source tag | std |
| `plugin_host` | Spawn, supervise, restart helpers; line protocol | std |
| `protocol` | PARTIAL / FINAL / STATUS / ERROR / READY (text lines, no serde) | std |
| `transcript` | Append JSONL; optional TXT mirror; export SRT/VTT | std |
| `privacy` | Default no disk; opt-in Remember; delete session | std |
| `lifecycle` | Shared logic: edges, debounce (used by watcher + main) | std |
| `ffi_win` / `ffi_mac` | **Only if** process-detection or scrape is in-process | system libs |
| `server` | Optional thin-client HTTP+SSE | std |
| **Out of v1 core** | cpal audio, VAD, STT plugins as primary, egui, voice modes | — |

### 4.3 Caption source protocol (stable contract)

Helpers print one line per event (human-debuggable, no JSON crates):

```text
READY
STATUS lc=running|stopped reason=...
PARTIAL text=...
FINAL text=...
ERROR message=...
LOG level=info|warn|error message=...
SHUTDOWN   # core → helper
```

- Escape rules for `text=`: define in `docs/PLUGIN-PROTOCOL.md` rewrite (percent-encoding or length-prefix). Prefer simple length-prefix for binary-safe UTF-8: `FINAL len=N` then N bytes — still no crates.
- Core never parses UI trees; only protocol lines.

### 4.4 Detection signals (versioned tables)

Ship as data files (or constants with version tags) so OS updates can be patched without redesign:

**Windows (`signals_windows.toml` or hand-parsed `.conf`):**

| Key | Primary values (research baseline) |
|-----|-------------------------------------|
| process_names | `LiveCaptions`, `LiveCaptions.exe` |
| window_classes | `LiveCaptionsDesktopWindow` |
| text_automation_ids | `CaptionsTextBlock`, fallback `CaptionsScrollViewer` |
| ignore_automation_ids | `ReadyToCaptionTextBlock` |
| min_os | Windows 11 22H2+ |

**macOS (`signals_macos.conf`):**

| Key | Primary values (research baseline) |
|-----|-------------------------------------|
| bundle_ids | `com.apple.accessibility.LiveTranscriptionAgent` |
| process_name_substrings | `Live Captions` |
| app_path_suffixes | `Live Captions.app` |
| ax_notes | walk windows; prefer longest changing text value |

**Update discipline:** each OS major version bump runs a **signal probe** tool (see §6) that records what it finds; if primary IDs fail, fall through the ordered fallback list.

---

## 5. Feature workstreams

### 5.1 Windows Live Captions capture

1. Ensure Live Captions is user-started (or document Win+Ctrl+L); companion follows user by default.
2. Helper: COM init → UIA → find window/process → `CaptionsTextBlock` → poll Name.
3. Line-diff rolling buffer → emit PARTIAL/FINAL.
4. Re-bind on COM/element failure (process restart).
5. No special Accessibility toggle expected; same-session integrity.

### 5.2 macOS Live Captions capture

1. Require Accessibility trust (`AXIsProcessTrustedWithOptions` prompt path).
2. Resolve PID for `com.apple.accessibility.LiveTranscriptionAgent`.
3. AX walk / observer → text attributes.
4. Same PARTIAL/FINAL protocol as Windows.
5. Fail closed with clear STATUS if TCC denied.
6. Respect personal-use framing (Apple SLA language on Live Captions output).

### 5.3 Auto open / close (both OSes)

| Component | Behavior |
|-----------|----------|
| Watcher install | User enables “Start with login” once |
| On edge (LC appears) | Launch main if not running; cancel off-timer |
| Off edge (LC gone) | Start **off-delay** (default 2–3 s, config 1–5 s); then stop capture, flush session, quit or idle main |
| Cold start | If LC already running when watcher starts → treat as on |
| Detection primary | **Process presence** (not full UIA/AX) |
| Detection secondary | Window class (Win) / bundle confirm (Mac) |

**Watcher must stay tiny** (CPU-cheap poll 0.5–2 s or OS process events via FFI). Prefer **idle main** vs hard kill if a session write is in flight (graceful IPC).

### 5.4 Durable save / remember

| Setting | Default |
|---------|---------|
| Live display / in-memory ring | On when main runs |
| Disk transcript | **Off** until user opts in (“Remember this session” or global Always remember) |
| Audio recording | **Never** in captions-first v1 (out of scope) |
| **Transcript folder** | **User-chosen.** Pick once (settings / first Remember); **persist path** in config and keep writing there until the user changes it. Default fallback only if never chosen: e.g. `~/Documents/Interpres Transcripts/` or `%USERPROFILE%\Documents\Interpres Transcripts\` (user-visible, not a hidden app cache). |
| **One session → one file** | **Yes.** Each Live Captions session creates **one new transcript file** in that folder (not one giant forever-log). |
| **Filename includes date and time** | **Yes.** Local wall-clock at session start, filesystem-safe, sortable. Example: `2026-08-05_14-22-01.txt` (and optional twin `2026-08-05_14-22-01.jsonl` if dual format is enabled). Optional short suffix if two sessions start in the same second: `_2`. |
| Primary human file | `.txt` (timestamped lines) — easiest to open later and re-read |
| Optional machine log | `.jsonl` alongside or instead (config); append-only during the session |
| Export | On demand: SRT, WebVTT, copy-all (from that session file) |
| Delete | Delete that one file (or the whole folder in Finder/Explorer) = forget |

**Folder picker (UX):** Settings control **“Transcript folder…”** (and “Open folder”). On Windows/macOS, v1 may use a native folder dialog via hand-written FFI **or** accept a path typed/pasted into config if dialog work is deferred — path must still be **user-owned and sticky**. Changing the folder does not move old files; old sessions stay where they were written.

**Session file rules:**

1. Session starts (main up + Remember on + LC running) → create **new** file named with **date + time**.
2. All FINAL lines for that session append only to that file.
3. Session ends (LC stop / user stop / app quit) → flush and close; next session gets a **new** dated file.
4. Never overwrite a previous session’s file.

JSONL event sketch (hand-emitted, fixed keys; only if jsonl companion enabled):

```text
{"v":1,"t":"<iso-ish>","kind":"final","src":"os-lc-win|os-lc-mac","text":"..."}
{"v":1,"t":"...","kind":"session_end","reason":"lc_stopped|user"}
```

TXT layout example (`2026-08-05_14-22-01.txt`):

```text
# Interpres session started 2026-08-05 14:22:01
# Source: Windows Live Captions
# Folder: /Users/you/Documents/Interpres Transcripts

[14:22:01] We can meet on Thursday.
[14:22:04] I'll send the calendar invite.
```

### 5.5 UX minimum for rebuild v1 (strict zero-dep)

Priority order:

1. **Headless / CLI main** that captures + saves (always ship).
2. **Thin-client HTML** over pure-std HTTP+SSE (large text on phone/laptop browser) — zero crates.
3. **Native overlay** only if time allows: hand-written always-on-top window via Win32/AppKit FFI — still zero crates.
4. **Do not** reintroduce egui/cpal for v1 success criteria.

Deaf/HoH UX still applies: high contrast, large type in HTML and any native window; big Remember / Stop / Open folder controls.

---

## 6. Future-proofing and fallbacks (required design)

### 6.1 Layers of defense

```text
L0  Versioned signal tables (AutomationIds, bundle IDs, window classes)
L1  Ordered fallbacks within same method (TextBlock → ScrollViewer → heuristic longest text node)
L2  Alternate method on same OS (Win: UIA → optional OCR helper; Mac: AX → optional parallel STT plugin)
L3  Cross-source fallback (explicit user opt-in “Use local STT plugin instead”)
L4  Degraded mode (lifecycle-only: open note that capture is broken; still offer manual paste/file)
```

### 6.2 Signal probe & self-test (ship with product)

Build `interpres probe` (or `helpers/probe-*`) that:

1. Detects whether Live Captions process is running.
2. Attempts to resolve primary text node and prints sample (redacted length only in logs if needed).
3. Writes `probe-report.txt` with OS version, which signal keys matched, which failed.
4. Exit codes: 0 = capture path OK; 2 = LC not running; 3 = permission; 4 = signals stale.

**On every OS update / user “Captions look empty” support path:** run probe → if L0 fails, try L1 → report which layer works.

### 6.3 Diff algorithm resilience

- Treat caption surface as a **rolling multi-line buffer**, not append-only.
- Emit FINAL when: new line committed, punctuation + idle stability, or buffer truncation of old lines with new content retained.
- Unit-test pure-std diff with **fixtures** captured from real sessions (checked-in text before/after pairs) — no UI needed in CI.

### 6.4 Helper hot-swap

- Core discovers helpers via config path + `plugin.yaml`-style hand-parsed manifest (no YAML crate: use minimal line format or existing simple conf).
- Multiple helpers can register: `captions-win-uia`, `captions-win-ocr`, `captions-mac-ax`, `captions-fallback-stt`.
- User/config selects order; core tries next on repeated ERROR / empty FINAL for N seconds.

### 6.5 What we explicitly do **not** promise

- Microsoft/Apple will not break UI trees.
- Pixel-perfect match to every partial recognition token.
- Capture while Accessibility is denied (macOS) or LC feature absent (Win10, non-Apple-silicon, locale).

---

## 7. Rebuild execution strategy (for future `/goal`)

### 7.1 Relationship to current tree

This is a **complete rebuild** of product direction and preferably of the **main application crate surface**:

| Keep (ideas / assets) | Leave behind / archive |
|----------------------|-------------------------|
| Pure-std HTTP+SSE patterns | Mic→VAD→STT as primary |
| Plugin host *idea* + line protocol discipline | cpal / egui as assumed deps |
| Privacy non-negotiables | Tauri/legacy UI as active path |
| Packaging scripts as reference | “Zero-dep but 2–4 crates” soft budget |

**Suggested tree move when implementing:** archive current `src/` audio-centric paths under `legacy/` or `archive/pre-lc-rebuild/`; new `src/` is captions-first. Do **not** do this until `/goal` implementation starts.

### 7.2 Phased delivery (implementation order)

| Phase | Outcome | Strict zero-dep check |
|-------|---------|------------------------|
| **P0 — Skeleton** | New core binary: config, session, protocol, plugin host, transcript writer; empty deps | `cargo tree` clean |
| **P1 — Windows capture** | Working helper + FINAL into JSONL/TXT on opt-in | Helper uses only OS APIs |
| **P2 — macOS capture** | Working AX helper + TCC messaging | Same |
| **P3 — Watcher lifecycle** | Auto open/close with debounce on both OSes | Watcher 0 crates |
| **P4 — Thin client + probe** | SSE/HTML large text; `interpres probe` | std only |
| **P5 — Fallbacks + polish** | Signal tables, L1–L3 fallbacks, docs, installers notes | No crate creep |
| **P6 — Optional native overlay** | Hand-written always-on-top (if still zero crates) | FFI only |

Each phase ends with: build + unit tests for pure-std pieces + manual smoke on target OS for scrape.

### 7.3 Acceptance criteria (rebuild v1 “done”)

1. **Windows:** With Live Captions on, companion produces PARTIAL/FINAL text matching the caption window closely enough to be useful in a meeting; opt-in save creates `session.jsonl` + `session.txt`.
2. **macOS:** Same with Accessibility granted; clear failure if not granted.
3. **Lifecycle:** With watcher installed, starting Live Captions starts companion capture; stopping Live Captions stops capture and closes/idles companion after debounce.
4. **Zero-dep:** Main package has **zero** third-party Cargo dependencies; documented in README; CI or script asserts it.
5. **Privacy:** Disk remember default off; user can choose and keep a transcript folder; **one dated/time-stamped file per session**; easy delete of a single file or the whole folder.
6. **Fallback proof:** Probe tool + at least one alternate path documented and stubbed or implemented (e.g. second AutomationId or STATUS degraded mode).
7. **No primary STT:** Default config source is `os-live-captions`, not local model.

### 7.4 Non-goals for rebuild v1

- AI-on-behalf / TTS / voice protection modes
- Replacing OS Live Captions quality with local models as default
- Linux Live Captions
- Meeting “lookup” / search UI (architecture must not block it; v1 may only save text)
- crates.io GUI frameworks
- Cloud sync

---

## 8. Privacy, ethics, open source

- **Local only.** No network upload of transcripts.
- **Opt-in remember** for durable text (other people’s speech).
- Honest UI copy: this app reads the Live Captions window/agent text via accessibility APIs; it is not an official Microsoft/Apple export API.
- Open source is load-bearing: users must be able to audit scrape + storage.
- Windows: community scrapers already normalize this assistive use case.
- macOS: personal accessibility tooling; respect SLA personal-use framing for Live Captions output.

---

## 9. Documentation deliverables (when implementing)

| Doc | Purpose |
|-----|---------|
| `README.md` rewrite | Captions-first quick start; zero-dep claim; Win/Mac setup |
| `docs/PLUGIN-PROTOCOL.md` | Line protocol + helper contract |
| `docs/signals.md` | Versioned detection tables + how to update after OS upgrade |
| `docs/privacy.md` | Remember defaults, delete paths |
| `BUILD.md` | Build with empty deps; helper build steps per OS |
| This file | Remains the product/architecture plan of record |

---

## 10. Verification plan (for implementer `/goal` — not run in this planning step)

1. **Gating — zero-dep:** `Cargo.toml` has no third-party deps; automated check fails the build if any appear.
2. **Gating — protocol:** unit tests feed fixture PARTIAL/FINAL lines into `TranscriptWriter`; assert JSONL/TXT on disk.
3. **Gating — diff:** unit tests with rolling-buffer fixtures (no UI).
4. **Gating — lifecycle logic:** unit tests for debounce state machine (process present/absent sequences).
5. **Manual / OS:** Windows smoke with real Live Captions; macOS smoke with Accessibility on/off.
6. **Probe:** `interpres probe` returns documented exit codes.
7. **Regression:** after signal table change, probe still green on reference machines.

---

## 11. Risks and mitigations

| Risk | Mitigation |
|------|------------|
| OS renames AutomationId / AX tree | Versioned signals + L1 fallbacks + probe + open-source quick patch |
| macOS Accessibility denied | Fail closed; one-click open Settings; no silent empty sessions |
| Watcher killed by user/OS | Document re-enable Login Items / Startup; optional KeepAlive LaunchAgent |
| Strict zero-dep slows native UI | Ship HTML thin client first; overlay is P6 |
| Scope creep back to full STT appliance | Enforce acceptance criterion “default source = os-live-captions” |
| Helper written with crates “for convenience” | Reject in review; helpers also prefer OS-only deps |

---

## 12. One-paragraph summary for `/goal` kickoff

Rebuild Interpres as a **strict zero-dependency** (zero crates.io) open-source companion that **scrapes Windows and macOS Live Captions** via process helpers (UIA / AX), **saves opt-in JSONL+TXT session transcripts**, and uses a **light login watcher** to **auto open/close with Live Captions process presence**, with **versioned signal tables, probe tooling, and layered fallbacks** so OS updates and permission failures degrade safely—**without** making local STT the primary path and **without** reintroducing cpal/egui into the core.

---

## 13. Implementation note

v0.2 core shipped: pure-`std` binary, sticky folder + dated one-file-per-session transcripts, lifecycle debounce, `probe` / `run` / `watch` / `demo`, Windows helper + macOS AX path, zero crates.io dependencies. Further packaging (login-item tray installers) may follow without changing the captions-first model.
