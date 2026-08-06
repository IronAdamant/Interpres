//! Pure UI label / status strings (testable without AppKit).
//!
//! Source of truth for folder, Save, and session footer copy so cold-launch and
//! mid-session toggles cannot drift from engine/config state.

use std::path::Path;

/// Folder row: always absolute path when set; never pretend unset when path is set.
pub fn folder_label(path: &Path) -> String {
    let s = path.to_string_lossy();
    if s.is_empty() {
        "Folder: (not set)".to_string()
    } else {
        format!("Folder: {s}")
    }
}

/// Footer under the window. Non-empty **only** when a session writer path is active.
pub fn session_footer(active_session_file: Option<&Path>) -> String {
    match active_session_file {
        Some(p) if !p.as_os_str().is_empty() => {
            format!("Saving to file: {}", p.display())
        }
        _ => String::new(),
    }
}

/// Status after the user toggles Save to disk.
pub fn remember_toggle_status(save_on: bool, folder: &Path) -> String {
    if save_on {
        format!(
            "Save to disk is ON. Next session will save under {}.",
            folder.display()
        )
    } else {
        "Save to disk is OFF for the next session. (A session already open may still finish writing.)"
            .to_string()
    }
}

/// Status when a new Live Captions session opens (writer created or deliberately not).
pub fn session_open_status(save_on: bool, session_txt: Option<&Path>) -> String {
    if save_on {
        if let Some(p) = session_txt {
            format!(
                "Saving what Live Captions shows to {} (Save is ON)",
                p.display()
            )
        } else {
            "Listening to Live Captions… Save is ON but no session file was created.".to_string()
        }
    } else {
        "Listening to Live Captions (not saving — turn on Save to disk).".to_string()
    }
}

/// Status while capture is running (companion to OS Live Captions — not a standalone captioner).
pub fn listening_status() -> &'static str {
    #[cfg(windows)]
    {
        "Listening to Live Captions… Keep system Live Captions open (Win+Ctrl+L) and play audio. Interpres only saves what captions already show."
    }
    #[cfg(target_os = "macos")]
    {
        "Listening to Live Captions… Keep system Live Captions on. Interpres only saves what captions already show."
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        "Listening… Live Captions companion (requires OS Live Captions)."
    }
}

/// Cold-start / idle guidance before Start listening.
pub fn idle_setup_status() -> &'static str {
    #[cfg(windows)]
    {
        "Turn on Windows Live Captions first (Win+Ctrl+L), then press Start listening. Interpres cannot caption by itself."
    }
    #[cfg(target_os = "macos")]
    {
        "Turn on Mac Live Captions first (System Settings → Accessibility), then press Start listening. Interpres cannot caption by itself."
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        "Live Captions companion — turn on OS Live Captions, then Start listening."
    }
}

/// Honesty for a **new** session: Save OFF must not show a saving footer.
///
/// Returns `(folder_label, session_footer, save_button_on)`.
pub fn new_session_labels(
    save_on: bool,
    session_txt: Option<&Path>,
    folder: &Path,
) -> (String, String, bool) {
    let folder_l = folder_label(folder);
    if !save_on {
        // New session with Save OFF: no writer → empty footer, button OFF.
        (folder_l, String::new(), false)
    } else {
        (folder_l, session_footer(session_txt), true)
    }
}

/// Short tip when live text may be frozen on a stale AX surface.
pub const LAG_TIP: &str =
    "If text freezes, Check setup / restart Live Captions";

/// When to surface the lag tip (poll ticks with unchanged or empty surface).
pub const LAG_TIP_AFTER_STALE_TICKS: u64 = 40;

/// When to clear the live box after junk-only / empty surfaces.
pub const CLEAR_LIVE_AFTER_EMPTY_TICKS: u64 = 12;

/// Stale-surface skip: unchanged surface already finalized → skip most re-process ticks.
pub fn should_skip_stale_surface(stale_ticks: u64, already_covered: bool) -> bool {
    stale_ticks > 6 && already_covered && (stale_ticks % 5 != 0)
}

/// Clear live after N consecutive **empty/junk-only** polls (dedicated counter, not shared stale).
pub fn should_clear_live_after_empty(empty_ticks: u64) -> bool {
    empty_ticks == CLEAR_LIVE_AFTER_EMPTY_TICKS
}

/// Show lag tip after long stuck surface or empty runs.
pub fn should_show_lag_tip(ticks: u64) -> bool {
    ticks == LAG_TIP_AFTER_STALE_TICKS
}

/// Consecutive failed capture polls before the UI shows a hard scrape error.
/// Transient PowerShell/UIA failures must not pin Status while captions still flow.
pub const CAPTURE_ERROR_SHOW_AFTER: u32 = 4;

/// Pure hysteresis for capture/scrape errors (testable without engine threads).
///
/// - A good surface clears the failure streak and hides hard error.
/// - Failures increment the streak; hard error only after `CAPTURE_ERROR_SHOW_AFTER`.
/// - Optional `error_message` is returned only when the hard error should be shown.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CaptureErrorHysteresis {
    consecutive_failures: u32,
    showing_hard_error: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaptureErrorTick {
    /// Show a hard scrape/UIA error in Status (sticky until a good surface).
    pub show_hard_error: bool,
    /// Message to display when `show_hard_error` (last failure text).
    pub message: Option<String>,
    /// Clear prior error UI after a successful surface.
    pub clear_error: bool,
}

impl CaptureErrorHysteresis {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn consecutive_failures(&self) -> u32 {
        self.consecutive_failures
    }

    pub fn showing_hard_error(&self) -> bool {
        self.showing_hard_error
    }

    /// One poll outcome: `surface_ok` true when caption surface text was obtained.
    pub fn on_poll(&mut self, surface_ok: bool, error: Option<&str>) -> CaptureErrorTick {
        if surface_ok {
            let clear = self.showing_hard_error || self.consecutive_failures > 0;
            self.consecutive_failures = 0;
            self.showing_hard_error = false;
            return CaptureErrorTick {
                show_hard_error: false,
                message: None,
                clear_error: clear,
            };
        }
        // No surface this tick.
        if error.is_some() {
            self.consecutive_failures = self.consecutive_failures.saturating_add(1);
        } else {
            // Empty surface without error (LC idle / junk filtered) — do not accumulate scrape errors.
            return CaptureErrorTick {
                show_hard_error: self.showing_hard_error,
                message: None,
                clear_error: false,
            };
        }
        if self.consecutive_failures >= CAPTURE_ERROR_SHOW_AFTER {
            self.showing_hard_error = true;
            CaptureErrorTick {
                show_hard_error: true,
                message: error.map(|s| s.to_string()),
                clear_error: false,
            }
        } else {
            CaptureErrorTick {
                show_hard_error: false,
                message: None,
                clear_error: false,
            }
        }
    }
}

/// Pure live-surface poll tracker used by the capture engine (testable without AX/threads).
///
/// `empty_ticks` is **not** shared with `stale_ticks`: after a long covered surface
/// (stale_ticks ≫ 12), transitioning to junk-only/None still clears Live after
/// exactly `CLEAR_LIVE_AFTER_EMPTY_TICKS` empty polls.
#[derive(Clone, Debug, Default)]
pub struct LiveSurfaceTracker {
    pub last_surface: String,
    pub stale_ticks: u64,
    /// Consecutive polls with no caption surface (junk filtered or empty AX).
    pub empty_ticks: u64,
}

/// Outcome of one poll tick for the live UI / buffer path.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LiveSurfaceTick {
    /// Process this surface with the caption buffer (false when skip_stale).
    pub process_surface: bool,
    /// Skip re-processing an unchanged covered surface.
    pub skip_stale: bool,
    /// Clear the live caption box (empty path only).
    pub clear_live: bool,
    /// Show lag tip under status.
    pub show_lag_tip: bool,
}

impl LiveSurfaceTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Poll returned caption text.
    pub fn on_surface(&mut self, surface: &str, already_covered: bool) -> LiveSurfaceTick {
        self.empty_ticks = 0;
        if surface == self.last_surface {
            self.stale_ticks = self.stale_ticks.saturating_add(1);
        } else {
            self.stale_ticks = 0;
            self.last_surface = surface.to_string();
        }
        let skip = should_skip_stale_surface(self.stale_ticks, already_covered);
        LiveSurfaceTick {
            process_surface: !skip,
            skip_stale: skip,
            clear_live: false,
            show_lag_tip: should_show_lag_tip(self.stale_ticks),
        }
    }

    /// Poll returned no caption surface (junk-only or empty AX).
    pub fn on_empty(&mut self) -> LiveSurfaceTick {
        self.empty_ticks = self.empty_ticks.saturating_add(1);
        LiveSurfaceTick {
            process_surface: false,
            skip_stale: false,
            clear_live: should_clear_live_after_empty(self.empty_ticks),
            show_lag_tip: should_show_lag_tip(self.empty_ticks),
        }
    }

    /// Buffer emitted a FINAL — surface may still be sticky; reset stale so we re-arm skip.
    pub fn note_final(&mut self) {
        self.stale_ticks = 0;
    }

    pub fn reset(&mut self) {
        self.last_surface.clear();
        self.stale_ticks = 0;
        self.empty_ticks = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn listening_and_idle_status_mention_live_captions_companion() {
        let listen = listening_status();
        assert!(
            listen.to_ascii_lowercase().contains("live captions"),
            "{listen}"
        );
        assert!(
            !listen.to_ascii_lowercase().contains("speech engine"),
            "{listen}"
        );
        let idle = idle_setup_status();
        assert!(
            idle.to_ascii_lowercase().contains("live captions"),
            "{idle}"
        );
        assert!(
            idle.to_ascii_lowercase().contains("cannot")
                || idle.to_ascii_lowercase().contains("by itself"),
            "idle must say Interpres is not standalone: {idle}"
        );
        let open = session_open_status(true, Some(Path::new("/tmp/s.txt")));
        assert!(open.contains("Live Captions") || open.contains("Saving"));
    }

    #[test]
    fn capture_error_hysteresis_clears_on_success_and_needs_streak() {
        let mut h = CaptureErrorHysteresis::new();
        // Single failure must not show hard error.
        let t1 = h.on_poll(false, Some("UIA text scrape failed (helper…)"));
        assert!(!t1.show_hard_error);
        assert!(!t1.clear_error);
        assert_eq!(h.consecutive_failures(), 1);

        // Still under threshold
        for _ in 0..2 {
            let t = h.on_poll(false, Some("UIA text scrape failed"));
            assert!(!t.show_hard_error);
        }
        assert_eq!(h.consecutive_failures(), 3);

        // Cross threshold
        let t_hard = h.on_poll(false, Some("UIA text scrape failed (helper at path)"));
        assert!(t_hard.show_hard_error);
        assert!(t_hard.message.as_ref().unwrap().contains("UIA"));
        assert!(h.showing_hard_error());

        // Success surface clears sticky error
        let t_ok = h.on_poll(true, None);
        assert!(!t_ok.show_hard_error);
        assert!(t_ok.clear_error);
        assert_eq!(h.consecutive_failures(), 0);
        assert!(!h.showing_hard_error());

        // Empty without error does not accumulate
        let t_empty = h.on_poll(false, None);
        assert!(!t_empty.show_hard_error);
        assert_eq!(h.consecutive_failures(), 0);
    }

    #[test]
    fn folder_label_shows_absolute_path_when_set() {
        let p = PathBuf::from("/Users/aron/Documents/Interpres Transcripts");
        let label = folder_label(&p);
        assert!(label.starts_with("Folder: /Users/aron/Documents/Interpres Transcripts"));
        assert!(!label.contains("(not set)"));
    }

    #[test]
    fn folder_label_not_set_only_when_empty() {
        assert_eq!(folder_label(Path::new("")), "Folder: (not set)");
    }

    #[test]
    fn session_footer_only_when_writer_path_active() {
        let p = PathBuf::from("/tmp/session.txt");
        assert_eq!(
            session_footer(Some(&p)),
            "Saving to file: /tmp/session.txt"
        );
        assert_eq!(session_footer(None), "");
        assert_eq!(session_footer(Some(Path::new(""))), "");
    }

    #[test]
    fn save_off_new_session_cannot_show_saving_footer() {
        let folder = PathBuf::from("/Users/aron/Documents/Interpres Transcripts");
        let (fl, footer, btn_on) = new_session_labels(false, None, &folder);
        assert!(fl.contains("Interpres Transcripts"));
        assert_eq!(footer, "", "Save OFF must not show Saving to file…");
        assert!(!btn_on);
        let status = session_open_status(false, None);
        assert!(status.contains("not saving"));
        assert!(!status.to_ascii_lowercase().contains("saving session to"));
    }

    #[test]
    fn save_on_new_session_shows_footer_and_status() {
        let folder = PathBuf::from("/Users/aron/Documents/Interpres Transcripts");
        let file = folder.join("2026-08-05_120000.txt");
        let (fl, footer, btn_on) = new_session_labels(true, Some(&file), &folder);
        assert!(fl.contains("Interpres Transcripts"));
        assert!(footer.starts_with("Saving to file:"));
        assert!(footer.contains("2026-08-05_120000.txt"));
        assert!(btn_on);
        let status = session_open_status(true, Some(&file));
        assert!(status.contains("Save is ON"));
    }

    #[test]
    fn mid_session_save_off_status_is_explicit() {
        let folder = PathBuf::from("/Users/x/Documents/Interpres Transcripts");
        let s = remember_toggle_status(false, &folder);
        assert!(s.contains("OFF"));
        assert!(s.contains("next session") || s.contains("may still finish"));
        let s_on = remember_toggle_status(true, &folder);
        assert!(s_on.contains("ON"));
        assert!(s_on.contains(folder.to_string_lossy().as_ref()));
    }

    #[test]
    fn stale_and_clear_rules() {
        assert!(!should_skip_stale_surface(3, true));
        assert!(should_skip_stale_surface(7, true));
        assert!(!should_skip_stale_surface(10, true)); // % 5 == 0 → re-check
        assert!(!should_skip_stale_surface(7, false));
        assert!(should_clear_live_after_empty(CLEAR_LIVE_AFTER_EMPTY_TICKS));
        assert!(!should_clear_live_after_empty(1));
        // High shared stale must NOT clear live (old bug: reused stale_ticks == 12)
        assert!(!should_clear_live_after_empty(50));
        assert!(!should_clear_live_after_empty(13));
        assert!(should_show_lag_tip(LAG_TIP_AFTER_STALE_TICKS));
        assert!(!should_show_lag_tip(5));
        assert!(LAG_TIP.contains("Check setup"));
    }

    /// Phase C integration: long covered sticky surface, then empty ticks → clear Live.
    /// Drives the shipped `LiveSurfaceTracker` used by the engine (not a reimplementation).
    #[test]
    fn covered_then_empty_ticks_clears_live() {
        let mut tr = LiveSurfaceTracker::new();
        let line = "Wind power works when the weather cooperates with the grid.";

        // Many polls on the same covered surface (stale_ticks climbs well past 12).
        for i in 0..30 {
            let tick = tr.on_surface(line, true);
            assert!(!tick.clear_live, "surface path never clears live");
            if i > 6 && i % 5 != 0 {
                assert!(tick.skip_stale, "stale covered surface should skip i={i}");
                assert!(!tick.process_surface);
            }
        }
        assert!(tr.stale_ticks >= 20, "stale_ticks={}", tr.stale_ticks);
        assert_eq!(tr.empty_ticks, 0);

        // Transition to junk-only / empty: empty_ticks starts at 0, not stale_ticks.
        let mut cleared_at = None;
        for i in 1..=CLEAR_LIVE_AFTER_EMPTY_TICKS + 3 {
            let tick = tr.on_empty();
            assert!(!tick.process_surface);
            if tick.clear_live {
                cleared_at = Some(i);
                break;
            }
        }
        assert_eq!(
            cleared_at,
            Some(CLEAR_LIVE_AFTER_EMPTY_TICKS),
            "must clear exactly after {CLEAR_LIVE_AFTER_EMPTY_TICKS} empty ticks; \
             empty_ticks={} stale_ticks={}",
            tr.empty_ticks,
            tr.stale_ticks
        );

        // New surface resets empty counter.
        let tick = tr.on_surface("Fresh caption about solar farms on the coast today", false);
        assert_eq!(tr.empty_ticks, 0);
        assert!(tick.process_surface);
        assert!(!tick.clear_live);
    }
}
