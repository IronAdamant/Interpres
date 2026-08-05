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

    /// Public check: is this line (or the last line of a surface) already finalized?
    pub fn is_covered(&self, text: &str) -> bool {
        let last = last_line(text);
        if last.is_empty() {
            return false;
        }
        self.already_covered(&last)
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
            | "correct spelling automatically"
            | "correct spelling"
            | "capitalise words automatically"
            | "capitalize words automatically"
            | "add period with double-space"
            | "smart quotes and dashes"
            | "touch bar typing suggestions"
            | "check spelling while typing"
            | "correct spelling automatically."
            | "use smart quotes and dashes"
            | "show spelling and grammar"
            | "force quit live captions"
            | "force quit"
            | "quit live captions"
            | "return to previous size"
            | "enter full screen"
            | "exit full screen"
            | "minimize"
            | "zoom"
            | "move"
            | "fill"
            | "center"
            | "arrange"
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
        || l.contains("spelling automatically")
        || l.contains("correct spelling")
        || l.contains("check spelling")
        || l.contains("auto-correct")
        || l.contains("autocorrect")
        || l.contains("text replacement")
        || l.contains("smart quotes")
        || l.contains("double-space")
        || l.contains("force quit")
        || l.contains("return to previous")
        || l.contains("previous size")
        || l.contains("full screen")
        || l.contains("use selection")
        || l.contains("for find")
        || l.contains("find and replace")
        || l.starts_with("edit ")
        || l.starts_with("view ")
        || l.starts_with("window ")
        || l.starts_with("file ")
        || (l.contains("quit") && l.contains("live captions"))
        || (l.contains("spelling") && l.contains("grammar"))
        || (l.contains("spelling") && l.contains("automatically"))
        || (l.contains("capitali") && l.contains("automatically"))
    {
        return true;
    }
    // Short menu/button chrome: few words, no sentence punctuation, mostly Capitalized.
    let words = t.split_whitespace().count();
    if words >= 2
        && words <= 5
        && !t.contains('.')
        && !t.contains('?')
        && !t.contains('!')
    {
        let caps = t
            .split_whitespace()
            .filter(|w| {
                w.chars()
                    .next()
                    .map(|c| c.is_ascii_uppercase())
                    .unwrap_or(false)
            })
            .count();
        // e.g. "Use Selection for Find", "Return to Previous Size"
        if caps >= words.saturating_sub(1) {
            return true;
        }
        if words <= 3 && caps >= 2 {
            return true;
        }
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

/// Pure surface pick used by AX scrape: drop junk-only input; prefer speech-like lines.
/// Returns `None` when every candidate is chrome/empty (never promote junk as caption).
pub fn pick_caption_surface<'a, I>(raw: I) -> Option<String>
where
    I: IntoIterator<Item = &'a str>,
{
    let mut best: Option<(i64, String)> = None;
    for s in raw {
        let trimmed = clean_surface(s);
        if trimmed.is_empty() {
            continue;
        }
        // clean_surface already drops junk lines; whole-string junk never scores.
        if is_junk_line(&trimmed) {
            continue;
        }
        let sc = score_caption_candidate(&trimmed);
        if sc <= 0 {
            continue;
        }
        match &best {
            None => best = Some((sc, trimmed)),
            Some((bsc, btxt)) => {
                let bw = btxt.split_whitespace().count();
                let tw = trimmed.split_whitespace().count();
                // Prefer higher score; within 15% prefer more words (fuller live line).
                if sc > *bsc
                    || (sc * 100 >= *bsc * 85 && (tw > bw || (tw == bw && trimmed.len() > btxt.len())))
                {
                    best = Some((sc, trimmed));
                }
            }
        }
    }
    best.map(|(_, t)| t)
}

fn score_caption_candidate(s: &str) -> i64 {
    if is_junk_line(s) {
        return -1_000_000;
    }
    let mut best_line = 0i64;
    for line in s.lines() {
        let t = line.trim();
        if t.is_empty() || is_junk_line(t) {
            continue;
        }
        best_line = best_line.max(score_one_caption_line(t));
    }
    if best_line > 0 {
        return best_line + s.lines().count() as i64;
    }
    score_one_caption_line(s)
}

fn score_one_caption_line(s: &str) -> i64 {
    if is_junk_line(s) {
        return -1_000_000;
    }
    let chars = s.chars().count() as i64;
    let words = s.split_whitespace().count() as i64;
    let mut score = chars + words * 4;
    if words >= 5 {
        score += 80;
    }
    if words >= 10 {
        score += 40;
    }
    if s.contains('.') || s.contains('?') || s.contains('!') {
        score += 30;
    }
    let lower = s.to_ascii_lowercase();
    if lower.contains(" in finder")
        || lower.starts_with("show ")
        || lower.contains("spelling")
        || lower.contains("correct spelling")
    {
        score -= 800;
    }
    if words < 4 {
        score -= 100;
    }
    score
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
    let longer = ta.len().max(tb.len());
    let mut shared_prefix = 0usize;
    for i in 0..shorter {
        if ta[i] == tb[i] {
            shared_prefix += 1;
        } else {
            break;
        }
    }
    // First 4+ words match (ASR often rewrites the tail only)
    if shared_prefix >= 4 {
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
    let jaccard = inter as f32 / union as f32;
    if jaccard >= 0.72 {
        return true;
    }

    // Allow one mid-sentence token edit when most tokens still overlap
    // (e.g. numeral spoken form vs digit form with shared frame).
    if inter >= 5 && longer > 0 && (inter as f32 / longer as f32) >= 0.62 {
        return true;
    }
    false
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

    /// Real chrome strings from user sessions / KNOWN-ISSUES (must all be junk).
    const KNOWN_JUNK: &[&str] = &[
        "Correct Spelling Automatically",
        "correct spelling automatically",
        "Correct Spelling",
        "Capitalise Words Automatically",
        "Capitalize Words Automatically",
        "Add Period with Double-Space",
        "Smart Quotes and Dashes",
        "Touch Bar Typing Suggestions",
        "Check Spelling While Typing",
        "Show Spelling and Grammar",
        "system floating window",
        "System Floating Window",
        "floating window",
        "Show “Fully-remote-115k.md” in Finder",
        "Show \"notes.md\" in Finder",
        "Show 'report.pdf' in Finder",
        "AU Dual Cable Straight Bar Research.md",
        "live captions",
        "Live Captions",
        "Computer Audio",
        "Type to Speak",
        "auto-correct",
        "Text Replacement",
        "Force Quit Live Captions",
        "Force Quit",
        "Quit Live Captions",
        "Return to Previous Size",
        "Enter Full Screen",
        "Exit Full Screen",
        "Use Selection for Find",
        "Find and Replace",
    ];

    const REAL_CAPTION: &str = "The fuel is free, which sounds wonderful until you remember free fuel arrives on the weather schedule, not on yours.";

    #[test]
    fn all_known_chrome_is_junk() {
        for s in KNOWN_JUNK {
            assert!(is_junk_line(s), "expected junk: {s:?}");
        }
    }

    #[test]
    fn real_captions_are_not_junk() {
        assert!(!is_junk_line(REAL_CAPTION));
        assert!(!is_junk_line(
            "Last year, California threw away roughly 3.4 million megawatt hours of clean electricity."
        ));
        assert!(!is_junk_line("Hello how are you doing today my friend"));
    }

    #[test]
    fn junk_only_surface_never_picked() {
        let only_junk = [
            "Correct Spelling Automatically",
            "system floating window",
            "Show “x.md” in Finder",
        ];
        assert_eq!(
            pick_caption_surface(only_junk.iter().copied()),
            None,
            "junk-only AX must not become caption surface"
        );
    }

    #[test]
    fn junk_mixed_with_caption_prefers_caption() {
        let mixed = [
            "Correct Spelling Automatically",
            REAL_CAPTION,
            "system floating window",
        ];
        let picked = pick_caption_surface(mixed.iter().copied()).expect("caption");
        assert!(picked.contains("fuel is free"), "got {picked:?}");
        assert!(!picked.to_ascii_lowercase().contains("spelling"));
    }

    #[test]
    fn junk_only_observe_never_emits_final() {
        let mut b = CaptionBuffer::new();
        for _ in 0..5 {
            match b.observe("Correct Spelling Automatically") {
                BufferEmit::Final(_) | BufferEmit::Finals(_) => {
                    panic!("must not finalize chrome")
                }
                BufferEmit::Partial(t) => {
                    assert!(!t.to_ascii_lowercase().contains("spelling"), "{t}");
                }
                BufferEmit::None => {}
            }
        }
        assert!(b.committed.is_empty());
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
    fn near_duplicate_family_one_final_in_history() {
        // Real transcript-style draft → polish pairs (numeral rewrite + tiny wording).
        let pairs = [
            (
                "Last year, California threw away roughly 3.400 megawatt hours of clean",
                "Last year, California threw away roughly 3.4 million megawatt hours of clean electricity.",
            ),
            (
                "They said it would take about 2.500 hours of work on the grid",
                "They said it would take about 2.5 thousand hours of work on the grid.",
            ),
            (
                "Wind turbines produce power when the weather cooperates with demand",
                "Wind turbines produce power when the weather cooperates with the grid demand.",
            ),
        ];
        for (draft, polished) in pairs {
            assert!(
                same_or_refinement(draft, polished),
                "family match failed:\n  {draft}\n  {polished}"
            );
            let mut b = CaptionBuffer::new();
            b.stable_needed = 1;
            let _ = b.observe(draft);
            let _ = b.observe(draft); // force final draft if any
            let _ = b.observe(polished);
            let _ = b.observe(polished);
            let family = b
                .committed
                .iter()
                .filter(|c| same_or_refinement(c, polished) || same_or_refinement(c, draft))
                .count();
            assert_eq!(family, 1, "committed={:?}", b.committed);
        }
    }

    #[test]
    fn is_covered_after_final() {
        let mut b = CaptionBuffer::new();
        b.stable_needed = 1;
        let line = "Wind power works when the weather cooperates with the grid.";
        let _ = b.observe(line);
        let _ = b.observe(line);
        assert!(b.is_covered(line));
        assert!(b.is_covered(&format!("{line}\n{line}")));
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
