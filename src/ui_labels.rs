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
            format!("Saving session to {} (Save is ON)", p.display())
        } else {
            "Listening… Save is ON but no session file was created.".to_string()
        }
    } else {
        "Listening (not saving — turn on Save to disk).".to_string()
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

/// Clear live caption box after this many empty/junk-only surface polls.
pub fn should_clear_live_after_empty(stale_ticks: u64) -> bool {
    stale_ticks == CLEAR_LIVE_AFTER_EMPTY_TICKS
}

/// Show lag tip after long stuck surface runs.
pub fn should_show_lag_tip(stale_ticks: u64) -> bool {
    stale_ticks == LAG_TIP_AFTER_STALE_TICKS
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

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
        assert!(should_show_lag_tip(LAG_TIP_AFTER_STALE_TICKS));
        assert!(!should_show_lag_tip(5));
        assert!(LAG_TIP.contains("Check setup"));
    }
}
