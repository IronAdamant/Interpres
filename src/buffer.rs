//! Rolling Live Captions buffer → PARTIAL / FINAL emissions.
//!
//! Handles OS rolling windows that rewrite the same sentence (draft → polished)
//! without writing near-duplicate finals to disk/UI.

/// Diffs successive caption text from the OS.
#[derive(Clone, Debug, Default)]
pub struct CaptionBuffer {
    previous: String,
    /// Lines already committed as FINAL (logical sentences).
    committed: Vec<String>,
    stable_ticks: u32,
    /// Identical observations before force-final on open partial.
    pub stable_needed: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BufferEmit {
    None,
    Partial(String),
    Final(String),
    Finals(Vec<String>),
}

impl CaptionBuffer {
    pub fn new() -> Self {
        Self {
            previous: String::new(),
            committed: Vec::new(),
            stable_ticks: 0,
            // Faster catch-up (~2–3 polls) while still reducing mid-word finals.
            stable_needed: 2,
        }
    }

    /// Observe caption text (prefer last spoken line or short recent surface).
    pub fn observe(&mut self, current: &str) -> BufferEmit {
        let current = clean_surface(current);
        if current.is_empty() {
            return self.flush_partial_if_any(false);
        }
        if current == self.previous {
            self.stable_ticks = self.stable_ticks.saturating_add(1);
            let last = last_line(&current);
            let looks_done = looks_sentence_complete(&last);
            let need = if looks_done { 1 } else { self.stable_needed };
            if self.stable_ticks >= need {
                return self.flush_partial_if_any(looks_done || self.stable_ticks >= 3);
            }
            if !last.is_empty() && !self.already_covered(&last) {
                return BufferEmit::Partial(last);
            }
            return BufferEmit::None;
        }

        self.stable_ticks = 0;
        let prev = self.previous.clone();
        let emit = self.diff_emit(&prev, &current);
        self.previous = current;
        emit
    }

    fn already_covered(&self, line: &str) -> bool {
        self.committed.iter().any(|c| same_or_refinement(c, line))
    }

    /// Record a final line. Never re-emits polished rewrites of an already-committed family.
    fn try_commit(&mut self, line: &str) -> Option<String> {
        let line = line.trim();
        if line.is_empty() || is_junk_line(line) {
            return None;
        }
        for c in self.committed.iter_mut() {
            if same_or_refinement(c, line) {
                if line_quality(line) >= line_quality(c) {
                    *c = line.to_string();
                }
                return None;
            }
        }
        self.committed.push(line.to_string());
        Some(line.to_string())
    }

    fn flush_partial_if_any(&mut self, force: bool) -> BufferEmit {
        self.stable_ticks = 0;
        let last = last_line(&self.previous);
        if last.is_empty() {
            return BufferEmit::None;
        }
        if !force && !looks_sentence_complete(&last) && last.split_whitespace().count() < 5 {
            return BufferEmit::Partial(last);
        }
        match self.try_commit(&last) {
            Some(t) => BufferEmit::Final(t),
            None => BufferEmit::None,
        }
    }

    fn diff_emit(&mut self, prev: &str, curr: &str) -> BufferEmit {
        let prev_lines: Vec<String> = split_caption_lines(prev);
        let curr_lines: Vec<String> = split_caption_lines(curr);

        if curr_lines.is_empty() {
            return BufferEmit::None;
        }

        let mut finals = Vec::new();

        // Lines that left the rolling window → finalize once.
        for p in &prev_lines {
            let still = curr_lines.iter().any(|c| same_or_refinement(c, p));
            if !still {
                if let Some(t) = self.try_commit(p) {
                    finals.push(t);
                }
            }
        }

        // Settled rows (all but last) in current surface.
        if curr_lines.len() >= 2 {
            for line in &curr_lines[..curr_lines.len() - 1] {
                if let Some(t) = self.try_commit(line) {
                    if !finals.iter().any(|f| same_or_refinement(f, &t)) {
                        finals.push(t);
                    }
                }
            }
        }

        let last = curr_lines.last().cloned().unwrap_or_default();

        if !finals.is_empty() {
            let mut out = Vec::new();
            for f in finals {
                if !out.iter().any(|x: &String| same_or_refinement(x, &f)) {
                    out.push(f);
                }
            }
            return match out.len() {
                0 => partial_or_none(self, &last),
                1 => BufferEmit::Final(out.remove(0)),
                _ => BufferEmit::Finals(out),
            };
        }

        partial_or_none(self, &last)
    }

    pub fn finish(&mut self) -> BufferEmit {
        self.flush_partial_if_any(true)
    }

    pub fn reset(&mut self) {
        self.previous.clear();
        self.committed.clear();
        self.stable_ticks = 0;
    }
}

fn partial_or_none(buf: &CaptionBuffer, last: &str) -> BufferEmit {
    if !last.is_empty() && !buf.already_covered(last) {
        BufferEmit::Partial(last.to_string())
    } else {
        BufferEmit::None
    }
}

fn split_caption_lines(s: &str) -> Vec<String> {
    s.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !is_junk_line(l))
        .map(|s| s.to_string())
        .collect()
}

fn last_line(s: &str) -> String {
    split_caption_lines(s).into_iter().last().unwrap_or_default()
}

fn clean_surface(s: &str) -> String {
    // Prefer a short tail so we track the live edge, not a giant sticky history blob.
    let lines = split_caption_lines(s);
    if lines.is_empty() {
        return String::new();
    }
    // Keep last 3 non-junk lines max for diffing (live + recent context).
    let start = lines.len().saturating_sub(3);
    lines[start..].join("\n")
}

/// UI chrome / Finder / control labels that are not spoken captions.
pub fn is_junk_line(s: &str) -> bool {
    let t = s.trim();
    if t.chars().count() < 4 {
        return true;
    }
    let l = t.to_ascii_lowercase();
    if matches!(
        l.as_str(),
        "live captions"
            | "close"
            | "settings"
            | "pause"
            | "resume"
            | "microphone"
            | "computer audio"
            | "type to speak"
            | "ok"
            | "cancel"
            | "finder"
            | "system floating window"
            | "floating window"
    ) {
        return true;
    }
    if l.contains(" in finder")
        || l.starts_with("show “")
        || l.starts_with("show \"")
        || l.starts_with("show '")
        || l.contains(".md")
        || l.ends_with(".txt")
        || l.ends_with(".pdf")
        || l.contains("notification")
        || l.contains("screenshot")
        || l.contains("floating window")
        || l.starts_with("system ")
    {
        return true;
    }
    // Filename-like single tokens
    if !t.contains(' ') && (t.contains('.') || t.contains('_') || t.contains('-')) {
        return true;
    }
    if !t.contains(' ') && t.chars().count() < 16 {
        return true;
    }
    false
}

fn looks_sentence_complete(s: &str) -> bool {
    let s = s.trim();
    if s.ends_with('.') || s.ends_with('?') || s.ends_with('!') || s.ends_with('…') {
        return true;
    }
    if (s.ends_with('"') || s.ends_with('\u{201d}'))
        && (s.contains('.') || s.contains('?') || s.contains('!'))
    {
        return true;
    }
    false
}

fn normalize_for_cmp(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Same spoken line or live rewrite (draft → polish).
pub fn same_or_refinement(a: &str, b: &str) -> bool {
    if a.trim() == b.trim() {
        return true;
    }
    let na = normalize_for_cmp(a);
    let nb = normalize_for_cmp(b);
    if na.is_empty() || nb.is_empty() {
        return false;
    }
    if na == nb {
        return true;
    }
    if na.len() >= 10 && nb.len() >= 10 && (na.contains(&nb) || nb.contains(&na)) {
        return true;
    }

    let ta: Vec<&str> = na.split_whitespace().collect();
    let tb: Vec<&str> = nb.split_whitespace().collect();
    if ta.len() < 3 || tb.len() < 3 {
        return false;
    }

    // Shared leading tokens (handles "3.400" → "3.4 million" mid-sentence)
    let shorter = ta.len().min(tb.len());
    let mut shared_prefix = 0usize;
    for i in 0..shorter {
        if ta[i] == tb[i] {
            shared_prefix += 1;
        } else {
            break;
        }
    }
    // First 5+ words match, or 60%+ of shorter is shared prefix
    if shared_prefix >= 5 {
        return true;
    }
    if shared_prefix >= 3 && (shared_prefix as f32 / shorter as f32) >= 0.55 {
        return true;
    }

    // Soft Jaccard for near-rewrites
    let set_a: std::collections::HashSet<&str> = ta.iter().copied().collect();
    let set_b: std::collections::HashSet<&str> = tb.iter().copied().collect();
    let inter = set_a.intersection(&set_b).count();
    let union = set_a.union(&set_b).count().max(1);
    (inter as f32 / union as f32) >= 0.78
}

fn line_quality(s: &str) -> usize {
    let mut q = s.len();
    if looks_sentence_complete(s) {
        q += 40;
    }
    q
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn junk_finder_and_md() {
        assert!(is_junk_line(
            "Show “Fully-remote-115k.md” in Finder"
        ));
        assert!(is_junk_line("AU Dual Cable Straight Bar Research.md"));
        assert!(is_junk_line("system floating window"));
        assert!(!is_junk_line(
            "The fuel is free, which sounds wonderful until you remember free fuel arrives on the weather schedule, not on yours."
        ));
    }

    #[test]
    fn polish_3_4_million_is_same_family() {
        let a = "Last year, California threw away roughly 3.400 megawatt hours of clean";
        let b = "Last year, California threw away roughly 3.4 million megawatt hours of clean electricity.";
        assert!(same_or_refinement(a, b));
    }

    #[test]
    fn refinement_single_final() {
        let mut b = CaptionBuffer::new();
        b.stable_needed = 1;
        let draft = "Last year, California threw away roughly 3.400 megawatt hours of clean";
        let polished =
            "Last year, California threw away roughly 3.4 million megawatt hours of clean electricity.";
        assert!(matches!(b.observe(draft), BufferEmit::Partial(_)));
        let e1 = b.observe(draft); // stable → final draft
        let mut n = 0;
        if matches!(e1, BufferEmit::Final(_)) {
            n += 1;
        }
        match b.observe(polished) {
            BufferEmit::Final(_) | BufferEmit::Finals(_) => n += 1,
            _ => {}
        }
        assert!(n <= 1, "duplicate finals n={n}");
        assert_eq!(
            b.committed
                .iter()
                .filter(|c| same_or_refinement(c, polished))
                .count(),
            1
        );
    }

    #[test]
    fn rolling_emits_partial() {
        let mut b = CaptionBuffer::new();
        match b.observe("Hello how are you doing today") {
            BufferEmit::Partial(t) => assert!(t.contains("Hello")),
            other => panic!("{other:?}"),
        }
    }
}
