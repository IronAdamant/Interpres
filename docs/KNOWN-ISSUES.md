# Known issues (Mac Live Captions companion)

**Last updated:** 2026-08-05  
**Evidence:** Real YouTube + Live Captions sessions, `*.txt` / `*.debug.log` under Documents/Interpres Transcripts, phone photos, and AgentVideoParse frame packs (e.g. `~/Movies/AgentVideoParse/IMG_6490-…`).

These were **confirmed product issues**. Mitigations for items 1–5 landed in code with unit tests; items stay annotated until a clean user re-test on the latest `dist/Interpres.app` fully closes them in the field.

**Fix plan:** [MD/FIX-KNOWN-ISSUES-PLAN.md](../MD/FIX-KNOWN-ISSUES-PLAN.md)

---

## Confirmed bugs / UX problems

### 1. macOS chrome junk in session history and files

**Symptom:** Non-speech strings appear as captions, e.g.:

- `Correct Spelling Automatically`
- Finder / document titles (e.g. `Show "….md" in Finder`)
- Other short UI labels (`Force Quit Live Captions`, floating window chrome)

**Why:** Accessibility scrape of Live Captions can also see system/app chrome; filters were incomplete.

**Status:** **Closed in code (2026-08-05)** — expanded `is_junk_line`, `pick_caption_surface` never returns junk-only surfaces; menu-verb rule does not drop real speech (“Open the door”); short proper-noun partials (e.g. New York Times) remain pickable. Unit tests: known chrome + `short_proper_noun_partials_are_not_junk` + `imperative_speech_not_menu_junk`. **User re-test** on latest `dist/Interpres.app` recommended.

---

### 2. Save / folder labels out of sync with reality

**Symptom (observed on screen):**

- Button shows **Save to disk: OFF** while status/footer still says **Saving to file: …/….txt**
- **Folder: (not set)** while files land in Documents/Interpres Transcripts

**Why:** UI labels were sometimes applied **before** the native window existed (updates dropped), and mid-session Save toggle did not clearly describe “current session vs next session.”

**Status:** **Closed in code (2026-08-05)** — `on_ready` re-applies folder/save/debug; pure `ui_labels` helpers for footer/status; Save OFF mid-session status is explicit; new session with Save OFF cannot show a saving footer (unit-tested). **User re-test** on cold launch + Save toggle still recommended.

---

### 3. Live captions lag / “not catching up” with speech

**Symptom:** YouTube / Live Captions move on; Interpres live box or session list holds an older line for a long time.

**Why (from debug logs):**

- AX surface string **stuck** unchanged for many poll cycles (dozens of identical `surface_chars` / preview lines)
- Poll + “stable before FINAL” logic adds delay
- After FINAL, stuck surface re-processed until filters/skip logic improved

**Status:** **Closed in code (2026-08-05)** — stale-surface skip when already covered, clear live after N empty/junk ticks, live-edge multi-candidate pick, lag tip after long stale runs. **User re-test** with Debug ON + short clip still recommended.

---

### 4. Near-duplicate lines (draft then polish)

**Symptom:** Same sentence twice in `.txt` / session list a few seconds apart (small ASR rewrites, e.g. `3.400` → `3.4 million`).

**Why:** Draft and polished forms both treated as FINAL before stronger “same family” matching.

**Status:** **Closed in code (2026-08-05)** — strengthened `same_or_refinement` (prefix + Jaccard + mid-token overlap); buffer commits one family line; unit tests drive real near-duplicate pairs. **User re-test** on multi-minute session recommended.

---

### 5. Live box and session list both show the same line

**Symptom:** Feels like “duplication” when the current FINAL is also the latest session line.

**Why:** By design, FINAL updates live **and** appends history (with refinement skip for true rewrites).

**Status:** **Closed in code (2026-08-05)** — UI labels **Live (now)** vs **Session (saved lines)**; history still skips same-family rewrites. Copy polish only; behavior is intentional.

---

### 6. System Live Captions invisible in screenshots

**Symptom:** OS Live Captions bar missing from screenshots/recordings.

**Why:** Apple privacy behavior — not an Interpres bug.

**Workaround:** Phone photo of the screen, or AgentVideoParse frames of a short recording; trust Interpres live box + debug log for what we scraped.

**Status:** **Out of scope** (Apple platform limitation).

---

## What works (do not regress)

- Live Captions process detection on Mac  
- Accessibility-based text scrape when trusted  
- Native AppKit window (no crates.io GUI)  
- Opt-in save, dated session files, debug log in transcript folder  
- Zero crates.io dependency graph  

---

## Evidence pack pattern (for future bugs)

1. Interpres **Debug: ON** → `….debug.log` next to `….txt`  
2. Optional short screen recording ≤30s → AgentVideoParse frames  
3. Note build path (e.g. `dist/Interpres.app` commit hash)

---

## Fix tracking

See **[MD/FIX-KNOWN-ISSUES-PLAN.md](../MD/FIX-KNOWN-ISSUES-PLAN.md)** for the ordered fix plan (phases A–E implemented 2026-08-05).
