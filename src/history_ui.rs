//! Pure Session-history apply helpers (Windows GUI and tests).
//!
//! Family-aware: polish of an earlier line updates that row, not only the last row.

use crate::buffer::{prefer_polish, same_or_refinement};

/// Default scan depth for same-family matches (matches transcript ring K).
pub const HISTORY_FAMILY_K: usize = crate::buffer::RECENT_FAMILY_K;

/// Apply a new Final caption to the in-memory Session history list.
/// Returns (new_history, body_of_last_nonempty_row).
pub fn history_apply_final(history: &[String], text: &str) -> (Vec<String>, String) {
    let text = text.trim();
    if text.is_empty() {
        return (history.to_vec(), last_body(history));
    }
    apply_inner(history, text, false)
}

/// Apply a Revised (polish) caption: replace matching family row if preferred.
/// Returns (new_history, body_of_last_nonempty_row) — last row body always, even when a
/// non-last row was rewritten (UI mirror of transcript `last_final_text` invariant).
pub fn history_apply_revised(history: &[String], text: &str) -> (Vec<String>, String) {
    let text = text.trim();
    if text.is_empty() {
        return (history.to_vec(), last_body(history));
    }
    apply_inner(history, text, true)
}

fn apply_inner(history: &[String], text: &str, revised_only: bool) -> (Vec<String>, String) {
    let mut out = history.to_vec();
    if let Some(idx) = find_family_index(&out, text) {
        let existing = out[idx].clone();
        if existing == text {
            let last = last_body(&out);
            return (out, last);
        }
        if prefer_polish(&existing, text) {
            out[idx] = text.to_string();
        }
        // Same family but worse/equal scrap → NoOp (never append a twin).
        let last = last_body(&out);
        return (out, last);
    }
    out.push(text.to_string());
    let last = last_body(&out);
    let _ = revised_only;
    (out, last)
}

fn find_family_index(history: &[String], text: &str) -> Option<usize> {
    let start = history.len().saturating_sub(HISTORY_FAMILY_K);
    // Most recent match wins.
    history
        .iter()
        .enumerate()
        .rev()
        .take(history.len() - start)
        .find(|(_, line)| same_or_refinement(line, text))
        .map(|(i, _)| i)
}

fn last_body(history: &[String]) -> String {
    history
        .iter()
        .rev()
        .find(|s| !s.trim().is_empty())
        .cloned()
        .unwrap_or_default()
}

/// Format history lines for a multi-line Win32 EDIT control.
pub fn history_to_edit_text(history: &[String]) -> String {
    let mut out = String::new();
    for line in history {
        out.push_str(line);
        out.push('\r');
        out.push('\n');
    }
    out
}

/// Parse lines from an EDIT control (tolerates `\r\n` / `\n`).
pub fn history_from_edit_text(s: &str) -> Vec<String> {
    s.lines()
        .map(|l| l.trim_end_matches('\r').to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revised_updates_non_last_family_row() {
        let hist = vec![
            "I am the owner of Elite Software Automation.".into(),
            "Second different sentence about inventory.".into(),
        ];
        let polish =
            "I am the owner of Elite Software Automation, and I started this business.";
        let (new_h, last) = history_apply_revised(&hist, polish);
        assert_eq!(new_h.len(), 2);
        assert!(new_h[0].contains("started this business"));
        assert_eq!(new_h[1], "Second different sentence about inventory.");
        // last_hist is physical last row, not the polish.
        assert_eq!(last, "Second different sentence about inventory.");
    }

    #[test]
    fn final_same_family_prefers_polish_not_append() {
        let hist = vec!["We can meet on Thursday".into()];
        let (new_h, _) = history_apply_final(&hist, "We can meet on Thursday.");
        assert_eq!(new_h.len(), 1);
        assert_eq!(new_h[0], "We can meet on Thursday.");
    }

    #[test]
    fn worse_scrap_is_noop() {
        let hist = vec![
            "And your purpose and your role, if you do get this job, will be to actually figure this out, to figure out what is actually going on there.".into(),
        ];
        let scrap = "And your purpose and your role";
        let (new_h, _) = history_apply_revised(&hist, scrap);
        assert_eq!(new_h.len(), 1);
        assert!(new_h[0].contains("going on there"));
    }

    #[test]
    fn unrelated_final_appends() {
        let hist = vec!["Hello there friend.".into()];
        let (new_h, last) = history_apply_final(&hist, "Completely different topic now.");
        assert_eq!(new_h.len(), 2);
        assert_eq!(last, "Completely different topic now.");
    }
}
