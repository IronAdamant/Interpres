# Known issues (Mac Live Captions companion)

**Last updated:** 2026-08-05  
**Evidence:** Real YouTube + Live Captions sessions, `*.txt` / `*.debug.log` under Documents/Interpres Transcripts, phone photos, and AgentVideoParse frame packs (e.g. `~/Movies/AgentVideoParse/IMG_6490-…`).

These are **confirmed product issues**, not guesses. Some mitigations landed in code already; items stay here until a clean re-test on the latest build closes them.

---

## Confirmed bugs / UX problems

### 1. macOS chrome junk in session history and files

**Symptom:** Non-speech strings appear as captions, e.g.:

- `Correct Spelling Automatically`
- Finder / document titles (e.g. `Show "….md" in Finder`)
- Other short UI labels

**Why:** Accessibility scrape of Live Captions can also see system/app chrome; filters were incomplete.

**Status:** Filters expanded (spelling, Finder, floating window, etc.). **Re-test required** on latest `dist/Interpres.app`.

---

### 2. Save / folder labels out of sync with reality

**Symptom (observed on screen):**

- Button shows **Save to disk: OFF** while status/footer still says **Saving to file: …/….txt**
- **Folder: (not set)** while files land in Documents/Interpres Transcripts

**Why:** UI labels were sometimes applied **before** the native window existed (updates dropped), and mid-session Save toggle did not clearly describe “current session vs next session.”

**Status:** `on_ready` re-applies folder/save/debug; session-file events refresh folder label. **Re-test required.**

---

### 3. Live captions lag / “not catching up” with speech

**Symptom:** YouTube / Live Captions move on; Interpres live box or session list holds an older line for a long time.

**Why (from debug logs):**

- AX surface string **stuck** unchanged for many poll cycles (dozens of identical `surface_chars` / preview lines)
- Poll + “stable before FINAL” logic adds delay
- After FINAL, stuck surface re-processed until filters/skip logic improved

**Status:** Faster poll floor, stale-surface skip when already finalized, live-edge surface preference. **Re-test required** with Debug ON + AgentVideoParse frames.

---

### 4. Near-duplicate lines (draft then polish)

**Symptom:** Same sentence twice in `.txt` / session list a few seconds apart (small ASR rewrites, e.g. `3.400` → `3.4 million`).

**Why:** Draft and polished forms both treated as FINAL before stronger “same family” matching.

**Status:** Dedup / refinement logic improved. **Re-test required.**

---

### 5. Live box and session list both show the same line

**Symptom:** Feels like “duplication” when the current FINAL is also the latest session line.

**Why:** By design, FINAL updates live **and** appends history (with refinement skip for true rewrites).

**Status:** Partial — history skips same/refinement of last line; live still shows current text. May need clearer UI copy (“live = now, session = saved lines”) rather than only code.

---

### 6. System Live Captions invisible in screenshots

**Symptom:** OS Live Captions bar missing from screenshots/recordings.

**Why:** Apple privacy behavior — not an Interpres bug.

**Workaround:** Phone photo of the screen, or AgentVideoParse frames of a short recording; trust Interpres live box + debug log for what we scraped.

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

See **[MD/FIX-KNOWN-ISSUES-PLAN.md](../MD/FIX-KNOWN-ISSUES-PLAN.md)** for the ordered fix plan.
