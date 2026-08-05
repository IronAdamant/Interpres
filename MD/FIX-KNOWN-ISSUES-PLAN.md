# Plan: Fix confirmed Mac Live Captions known issues

**Date:** 2026-08-05  
**Source of issues:** `docs/KNOWN-ISSUES.md` + user YT sessions + AgentVideoParse frames  
**Constraint:** Keep **zero crates.io** dependencies; Mac-first.

This is a **fix plan** for remaining work after partial mitigations. Execute in order; re-test after each phase with Debug ON.

---

## Goals

1. No macOS spelling/Finder chrome in live, session, or `.txt`  
2. Folder + Save buttons always match engine/config and active session  
3. Live text tracks speech with less multi-second freeze on old AX strings  
4. At most one FINAL per spoken sentence family (draft ≠ second file line)  
5. Clearer UI about live vs session (optional copy polish)

---

## Phase A — Close the loop on junk (highest confidence)

| Task | Detail |
|------|--------|
| A1 | Expand/verify `is_junk_line` for all observed chrome (`Correct Spelling Automatically`, related Settings strings) |
| A2 | Unit tests for each junk string from real sessions |
| A3 | If junk still wins AX scoring, **never** return junk-only surface (surface_text = None) |
| A4 | Manual re-test: YT + Live Captions + Debug; confirm no junk in `.txt` / session list |

**Done when:** New session file has zero chrome lines in a 2–3 minute YT clip.

---

## Phase B — Label / save state honesty

| Task | Detail |
|------|--------|
| B1 | Ensure `on_ready` always paints folder, remember, debug (already added — verify on cold launch) |
| B2 | Single source of truth: footer **Saving to file** only if `writer` active; button Save = config for *next* / current policy |
| B3 | If Save turned OFF mid-session: status text explicit (“current file may still finish; next session won’t save”) |
| B4 | Folder label always absolute path from config/engine, never “(not set)” when path is set |
| B5 | Manual re-test: toggle Save before/during session; labels match behavior |

**Done when:** Screenshot cannot show Save OFF + “Saving to file…” for a new session started with Save OFF.

---

## Phase C — Stale surface / catch-up

| Task | Detail |
|------|--------|
| C1 | Keep stale-skip when surface unchanged **and** last line already covered |
| C2 | When surface is junk-only/empty, clear live after N ticks (avoid frozen old caption) |
| C3 | Prefer live-edge lines from AX; re-walk / multi-candidate pick if top string is stale junk |
| C4 | Optional: lower default `poll_ms` further only if CPU OK |
| C5 | Re-test with AgentVideoParse ≤30s clip + debug log: fewer long identical `surface_chars` runs on chrome |

**Done when:** Debug log does not show 100+ consecutive polls on `Correct Spelling…` or similar; live updates within ~1s of LC change in normal conditions.

---

## Phase D — Dedup polish

| Task | Detail |
|------|--------|
| D1 | Keep/strengthen `same_or_refinement` (shared prefix + Jaccard) for ASR rewrites |
| D2 | File + UI history: one line per family |
| D3 | Unit tests from real near-duplicate pairs in user transcripts |

**Done when:** No twin lines 1–5s apart for same sentence in a 3-minute session.

---

## Phase E — UX copy (optional, small)

| Task | Detail |
|------|--------|
| E1 | Labels: “Live (now)” vs “Session (saved lines)” if users still report “duplication” |
| E2 | Short tip under status when lag suspected: “If text freezes, Check setup / restart Live Captions” |

---

## Verification matrix (every phase)

| Check | How |
|-------|-----|
| Unit tests | `cargo test` (junk, refinement, assets) |
| Zero-dep | `cargo tree -p interpres` → only interpres |
| Manual | Live Captions + YT, Save ON, Debug ON |
| Artifacts | New `….txt` + `….debug.log` |
| Optional | AgentVideoParse frames of ≤30s screen recording |

---

## Out of scope for this plan

- Windows Live Captions GUI  
- Changing zero-dep policy  
- Full legal/accessibility certification  
- Fixing Apple screenshot exclusion of Live Captions  

---

## Suggested execution order

**A → B → C → D → E**, shipping a rebuild of `dist/Interpres.app` after A+B+C at minimum for user re-test.

---

## Handoff note

Partial work already in tree (junk filter expansions, on_ready, stale skip, refinement). This plan is the **checklist to close known issues** with evidence, not a greenfield rewrite.

---

## Implementation status (2026-08-05)

| Phase | Status | Notes |
|-------|--------|--------|
| A | **Done in code** | Expanded junk list + `pick_caption_surface`; tests for all known chrome; never junk-only surface |
| B | **Done in code** | `ui_labels` pure helpers; `on_ready` paint; mid-session Save OFF explicit; honesty tests |
| C | **Done in code** | Stale skip via `should_skip_stale_surface`; clear live after N empty ticks; lag tip; multi-candidate pick |
| D | **Done in code** | Stronger `same_or_refinement`; near-duplicate family unit tests |
| E | **Done in code** | Labels **Live (now)** / **Session (saved lines)**; lag tip string |
| Package | **Done** | `dist/Interpres/Interpres.app` rebuilt via `packaging/make-double-click.sh` |
| Field re-test | **Pending user** | 2–3 min YT + LC with Debug ON on latest dist app |

Zero-dep preserved (`cargo tree -p interpres` → only `interpres`).
