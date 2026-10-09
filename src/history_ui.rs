//! Pure Session-history apply helpers (Windows GUI and tests).
//!
//! Family-aware: polish of an earlier line updates that row, not only the last row.

use crate::buffer::{
    extends_by_words, normalize_for_cmp, number_skeleton, prefer_polish, same_or_refinement,
    skeleton_overlaps,
};

/// Default scan depth for same-family matches (matches transcript ring K).
pub const HISTORY_FAMILY_K: usize = crate::buffer::RECENT_FAMILY_K;
/// Further back than `HISTORY_FAMILY_K`, only a long unmistakable repeat matches
/// (Live Captions re-shows an older sentence after rewriting its numbers).
pub const STRONG_REPEAT_K: usize = 20;
/// Letters a far-back repeat must share (a short "Yeah, that makes sense." stays new).
const STRONG_REPEAT_MIN: usize = 40;
/// A merged polish only absorbs earlier lines at least this long (normalized chars),
/// unless it starts with that line (a stub like "years and" right before it).
const ABSORB_MIN: usize = 12;
/// An identical line this many words or longer, within `STRONG_REPEAT_K`, is Live
/// Captions re-showing it (seen ~20 s later, 9 lines on); shorter replies stay new.
const SAME_LINE_MIN_WORDS: usize = 5;

/// How a caption changes the recent lines.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FamilyPlan {
    /// New sentence: add it at the end.
    Append,
    /// Same as (or worse than) a line already there.
    NoOp,
    /// Put the text at line `at` and delete the lines in `remove` (ascending, never `at`).
    /// `remove` is non-empty when Live Captions merged earlier sentences into this one
    /// ("A." + "B." → "A B."); the merged line keeps the earliest position and time.
    Replace { at: usize, remove: Vec<usize> },
    /// Set line `at` to `text` (the caption joined onto that line). Live Captions split
    /// one sentence across two lines: "it will help with the" + "will help with the
    /// supply of that.", or "…besides gaming." + "on a long-term trend.".
    Join { at: usize, text: String },
}

/// Decide how `text` applies to `lines` (oldest → newest). Shared by the transcript
/// file and the window so both show the same lines.
pub fn plan_family<S: AsRef<str>>(lines: &[S], text: &str) -> FamilyPlan {
    let n = lines.len();
    let near = n.saturating_sub(HISTORY_FAMILY_K);
    let far = n.saturating_sub(STRONG_REPEAT_K);
    // Most recent match wins.
    let idx = (near..n)
        .rev()
        .find(|&i| same_or_refinement(lines[i].as_ref(), text))
        .or_else(|| {
            let key = normalize_for_cmp(text);
            let long_enough = key.split(' ').count() >= SAME_LINE_MIN_WORDS;
            (far..near).rev().find(|&i| {
                let line = lines[i].as_ref();
                skeleton_overlaps(line, text, STRONG_REPEAT_MIN)
                    || (long_enough && normalize_for_cmp(line) == key)
            })
        });
    let Some(idx) = idx else {
        return match lines.last().and_then(|last| join_split(last.as_ref(), text)) {
            Some(joined) => FamilyPlan::Join { at: n - 1, text: joined },
            None => FamilyPlan::Append,
        };
    };
    let existing = lines[idx].as_ref();
    if existing == text || !prefer_saved(existing, text) {
        return FamilyPlan::NoOp;
    }
    // Lines directly before the match that the text repeats in full.
    let mut keep = idx;
    while keep > far
        && (contains_line(text, lines[keep - 1].as_ref(), ABSORB_MIN)
            || starts_with_line(text, lines[keep - 1].as_ref()))
    {
        keep -= 1;
    }
    let mut absorbed: Vec<usize> = (keep..idx).collect();
    // A long earlier version with another line in between ("A", "X", "A + more").
    for j in (near.min(keep)..keep).rev() {
        if contains_line(text, lines[j].as_ref(), STRONG_REPEAT_MIN) {
            absorbed.push(j);
        }
    }
    absorbed.push(idx);
    absorbed.sort_unstable();
    let at = absorbed.remove(0);
    FamilyPlan::Replace { at, remove: absorbed }
}

/// A joined line never grows past this many characters (a run of continuations would
/// otherwise become one paragraph).
const JOIN_MAX_CHARS: usize = 320;

/// Words of `s` as (original, normalized) pairs; tokens with no letters or digits are
/// kept with the word before them so the original text is rebuilt faithfully.
fn words(s: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for tok in s.split_whitespace() {
        let norm: String = tok
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '\'')
            .flat_map(char::to_lowercase)
            .collect();
        match out.last_mut() {
            Some(last) if norm.is_empty() => {
                last.0.push(' ');
                last.0.push_str(tok);
            }
            _ => out.push((tok.to_string(), norm)),
        }
    }
    out
}

/// Line words `a` and caption words `b` mean the same word; the line's last word may
/// have been cut mid-word ("penetra" → "penetrated").
fn same_word(a: &str, b: &str, line_end: bool) -> bool {
    a == b || (line_end && a.len() >= 3 && b.starts_with(a))
}

/// `text` starts by repeating the end of `line`: index of the line word where the
/// repeat begins. The first two words must match exactly, and at least 80% of the
/// rest (Live Captions polishes a word or two while re-showing).
fn tail_overlap(line: &[(String, String)], text: &[(String, String)]) -> Option<usize> {
    let n = line.len();
    (0..n.saturating_sub(1)).find(|&p| {
        let len = n - p;
        if len < 2 || len > text.len() || line[p].1 != text[0].1 || line[p + 1].1 != text[1].1 {
            return false;
        }
        let same = (0..len)
            .filter(|&i| same_word(&line[p + i].1, &text[i].1, p + i == n - 1))
            .count();
        same * 5 >= len * 4
    })
}

/// Join a caption onto the line before it when Live Captions split one sentence.
/// Returns the joined line, or None when `text` is a new line.
fn join_split(line: &str, text: &str) -> Option<String> {
    // Only a lowercase start continues the line before: Live Captions starts each
    // sentence with a capital, and a capitalised repeat ("There a lot of great" →
    // "A lot of great engineers.") is it restarting the sentence, which the family
    // match handles. "iPhone"-style words are not continuations.
    let mut chars = text.chars();
    let first = chars.next()?;
    let second = chars.next().unwrap_or(' ');
    if !first.is_lowercase() || second.is_uppercase() {
        return None;
    }
    let (lw, tw) = (words(line), words(text));
    if lw.is_empty() || tw.is_empty() {
        return None;
    }
    // "it will help with the" + "will help with the supply of that.": drop the repeat.
    if let Some(p) = tail_overlap(&lw, &tw) {
        let head: Vec<&str> = lw[..p].iter().map(|w| w.0.as_str()).collect();
        return Some(if head.is_empty() {
            text.to_string()
        } else {
            format!("{} {text}", head.join(" "))
        });
    }
    if line.len() + text.len() + 1 > JOIN_MAX_CHARS {
        return None;
    }
    // The full stop (or "?") before it was Live Captions guessing a sentence end.
    let head = line.trim_end().trim_end_matches(['.', '?', '!']);
    Some(format!("{head} {text}"))
}

/// For lines already saved: a version that repeats every word and adds two or more wins
/// even without a full stop. Live Captions saved "We'll see though." then sent "We'll see
/// though there's always repercussions"; the caption buffer's full-stop bonus dropped the
/// extra words. (The buffer itself keeps the stricter rule: mid-stream, those extra words
/// are often the next sentence glued on.)
fn prefer_saved(existing: &str, candidate: &str) -> bool {
    extends_by_words(existing, candidate, 2) || prefer_polish(existing, candidate)
}

/// `text` begins with every word of `line` ("years and" → "years and I am still…").
fn starts_with_line(text: &str, line: &str) -> bool {
    let (t, l) = (normalize_for_cmp(text), normalize_for_cmp(line));
    !l.is_empty() && (t == l || t.starts_with(&format!("{l} ")))
}

/// `text` repeats all of `line` (ignoring case, punctuation and number style), and the
/// line has at least `min` characters to compare.
fn contains_line(text: &str, line: &str, min: usize) -> bool {
    let (t, l) = (normalize_for_cmp(text), normalize_for_cmp(line));
    if l.len() >= min && t.contains(&l) {
        return true;
    }
    let (ts, ls) = (number_skeleton(text), number_skeleton(line));
    ls.len() >= min && ts.contains(&ls)
}

/// Apply a plan to `lines` and a parallel per-line list (e.g. times); `new_side` makes
/// the entry for an appended line.
pub fn apply_plan<T>(
    lines: &mut Vec<String>,
    side: &mut Vec<T>,
    plan: FamilyPlan,
    text: &str,
    new_side: impl FnOnce() -> T,
) {
    match plan {
        FamilyPlan::Append => {
            lines.push(text.to_string());
            side.push(new_side());
        }
        FamilyPlan::NoOp => {}
        FamilyPlan::Join { at, text } => lines[at] = text,
        FamilyPlan::Replace { at, remove } => {
            lines[at] = text.to_string();
            for &i in remove.iter().rev() {
                lines.remove(i);
                if i < side.len() {
                    side.remove(i);
                }
            }
        }
    }
}

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

fn apply_inner(history: &[String], text: &str, _revised_only: bool) -> (Vec<String>, String) {
    let mut out = history.to_vec();
    let plan = plan_family(&out, text);
    apply_plan(&mut out, &mut Vec::<()>::new(), plan, text, || ());
    let last = last_body(&out);
    (out, last)
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

    #[test]
    fn merged_sentences_replace_their_parts() {
        // Live run: A, B, C saved separately, then re-sent merged as "A B", "A B C".
        let a = "I was told this last month it was originally planned for the end of twenty six.";
        let b = "That's obviously not happening, but just like I've talked about the PS six has to come out at a certain point.";
        let c = "This does as well you if you delay a product half a year that's easily doable.";
        let mut h = Vec::new();
        for t in [a, b, c] {
            h = history_apply_final(&h, t).0;
        }
        let ab = "I was told this last month it was originally planned for the end of twenty six that's obviously not happening but just like I've talked about the PS six has to come out at a certain point.";
        h = history_apply_revised(&h, ab).0;
        assert_eq!(h, vec![ab.to_string(), c.to_string()]);
        let abc = "I was told this last month it was originally planned for the end of twenty six that's obviously not happening but just like I've talked about the PS six has to come out at a certain point this does as well you if you delay a product half a year that's easily doable.";
        h = history_apply_revised(&h, abc).0;
        assert_eq!(h, vec![abc.to_string()]);
    }

    #[test]
    fn merge_keeps_unrelated_earlier_lines_and_times() {
        let mut lines: Vec<String> = ["Does anyone have questions?", "Second, the launch moved to the end of the quarter.", "I think that makes sense."]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let mut times = vec![1, 2, 3];
        let merged = "Second, the launch moved to the end of the quarter, I think that makes sense.";
        let plan = plan_family(&lines, merged);
        assert_eq!(plan, FamilyPlan::Replace { at: 1, remove: vec![2] });
        apply_plan(&mut lines, &mut times, plan, merged, || 0);
        assert_eq!(lines, vec!["Does anyone have questions?".to_string(), merged.to_string()]);
        assert_eq!(times, vec![1, 2], "merged line keeps the earliest time");
    }

    #[test]
    fn number_rewrite_of_an_older_line_is_not_a_new_line() {
        let old = "then screen, then and it comes with sixteen gigabytes of RAM and a two hundred dollar board.";
        let mut h = vec![old.to_string()];
        for i in 0..10 {
            h.push(format!("Unrelated sentence number {i} about something else entirely."));
        }
        let redo = "then screen, then and it comes with 16 gigabytes of RAM and a $200 board.";
        let (out, _) = history_apply_final(&h, redo);
        assert_eq!(out.len(), h.len(), "re-shown line is not appended again");
        // A short repeat that far back is a real new line.
        let (out, _) = history_apply_final(&h, "Yeah, that makes sense.");
        assert_eq!(out.len(), h.len() + 1);
    }

    #[test]
    fn longer_version_absorbs_earlier_one_across_a_line() {
        // Live run: "A", "X", then a longer "A" arrived as its own line.
        let a = "If you were to delay this more than a year from when it was supposed to come out, I mean then you have to start asking yourself, well should we keep this node?";
        let x = "OK let's say we do want to keep this node, OK.";
        let a2 = "If you were to delay this more than a year from when it was supposed to come out, I mean then you have to start asking yourself, well should we keep this node if we don't keep this node then we're going to spend money porting it to a new node.";
        let mut h = vec![a.to_string(), x.to_string()];
        h = history_apply_revised(&h, a2).0;
        assert_eq!(h, vec![a2.to_string(), x.to_string()], "keeps speaking order and the line between");
        // A short line between two copies is never pulled in.
        let mut h = vec!["OK.".to_string(), x.to_string()];
        h = history_apply_final(&h, "OK. Fine, let's go.").0;
        assert_eq!(h.len(), 3);
    }

    #[test]
    fn longer_version_of_a_finished_line_keeps_its_extra_words() {
        // From the 3-hour run: these words were lost.
        let h = vec!["We'll see though.".to_string()];
        let (out, _) = history_apply_revised(&h, "We'll see though there's always repercussions");
        assert_eq!(out, vec!["We'll see though there's always repercussions".to_string()]);
        // One extra word is still a draft: the finished line stays.
        let h = vec!["We can meet on Thursday.".to_string()];
        assert_eq!(history_apply_revised(&h, "We can meet on Thursday if").0, h);
    }

    #[test]
    fn stub_right_before_its_full_line_is_absorbed() {
        // 3-hour run: "years and" saved, then the sentence arrived via another line.
        let mut h = vec!["years and".to_string(), "And I am still not to my next goal on patreon once I get".to_string()];
        h = history_apply_revised(&h, "years and I am still not to my next goal on Patreon.").0;
        assert_eq!(h, vec!["years and I am still not to my next goal on Patreon.".to_string()]);
    }

    #[test]
    fn identical_line_reshown_a_little_later_is_not_repeated() {
        let mut h = vec!["Australia is up there too.".to_string()];
        for i in 0..8 {
            h.push(format!("Some other sentence number {i} here."));
        }
        assert_eq!(history_apply_final(&h, "Australia is up there too.").0.len(), h.len());
        // A short reply repeated that far on is a new line.
        let mut h = vec!["Yeah, that makes sense.".to_string()];
        for i in 0..8 {
            h.push(format!("Some other sentence number {i} here."));
        }
        assert_eq!(history_apply_final(&h, "Yeah, that makes sense.").0.len(), h.len() + 1);
    }

    // Field run 2026-10-09 (Mac, 3 h): Live Captions split sentences across lines.
    #[test]
    fn split_sentence_is_joined() {
        let join = |line: &str, text: &str| match plan_family(&[line], text) {
            FamilyPlan::Join { at: 0, text } => Some(text),
            _ => None,
        };
        // The new line repeats the end of the last one.
        assert_eq!(
            join("it will help with the", "will help with the supply of that.").as_deref(),
            Some("it will help with the supply of that.")
        );
        assert_eq!(
            join("It's obvious you", "obvious you should choose that.").as_deref(),
            Some("It's obvious you should choose that.")
        );
        // The rest of a sentence Live Captions closed too early.
        assert_eq!(
            join(
                "And it's like, everything's going down pretty much besides gaming.",
                "on a long-term trend."
            )
            .as_deref(),
            Some("And it's like, everything's going down pretty much besides gaming on a long-term trend.")
        );
        assert_eq!(join("Any questions?", "so far?").as_deref(), Some("Any questions so far?"));
    }

    #[test]
    fn capitalised_line_is_not_joined() {
        // Live Captions restarting the sentence (dropping a misheard word) stays a
        // family rewrite, never "There A lot of great engineers.".
        assert!(!matches!(
            plan_family(&["There a lot of great"], "A lot of great engineers."),
            FamilyPlan::Join { .. }
        ));
        assert_eq!(plan_family(&["We can meet on Thursday."], "I'll send the invite."), FamilyPlan::Append);
        assert_eq!(plan_family(&["Let me check."], "iPhone sales were up."), FamilyPlan::Append);
    }

    #[test]
    fn joined_lines_stay_readable_length() {
        let long = "word ".repeat(70);
        assert_eq!(plan_family(&[long.trim()], "and then some more words here."), FamilyPlan::Append);
    }
}
