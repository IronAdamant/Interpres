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
    /// Live edge only — caption still being rewritten by the OS.
    Partial(String),
    /// New settled sentence (not a polish of something already committed).
    Final(String),
    Finals(Vec<String>),
    /// OS updated an already-committed sentence family (draft → polish).
    /// UI should replace the last same-family history line; disk may rewrite last final.
    Revised(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum CommitOutcome {
    None,
    New(String),
    Revised(String),
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
            // Surface cleared — leave-window finalize everything we still hold.
            if self.previous.is_empty() {
                return BufferEmit::None;
            }
            let prev = self.previous.clone();
            let emit = self.diff_emit(&prev, "");
            self.previous.clear();
            self.stable_ticks = 0;
            return emit;
        }
        if current == self.previous {
            self.stable_ticks = self.stable_ticks.saturating_add(1);
            let last = last_segment(&current);
            let looks_done = looks_sentence_complete(&last);
            // Incomplete tails need more stability; never force-final tiny fragments.
            let need = if looks_done {
                1
            } else if word_count(&last) < 6 {
                self.stable_needed.saturating_add(3)
            } else {
                self.stable_needed
            };
            if self.stable_ticks >= need {
                // Only force-final incomplete text when it is substantial.
                let force = looks_done
                    || (self.stable_ticks >= need.saturating_add(2) && word_count(&last) >= 8);
                return self.flush_partial_if_any(force);
            }
            if !last.is_empty() && !self.already_covered(&last) {
                return BufferEmit::Partial(last);
            }
            // Covered but OS may still be polishing — surface live edge anyway.
            if !last.is_empty() {
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

    /// Public check: is this line (or the last segment of a surface) already finalized?
    pub fn is_covered(&self, text: &str) -> bool {
        let last = last_segment(text);
        if last.is_empty() {
            return false;
        }
        self.already_covered(&last)
    }

    /// Record a final line. Polished rewrites of a committed family → Revised.
    ///
    /// `leave_window`: phrase left the rolling surface — allow shorter complete-ish
    /// utterances ("Wait a second") while still rejecting open stubs ("see the").
    fn try_commit(&mut self, line: &str, leave_window: bool) -> CommitOutcome {
        let line = line.trim();
        if line.is_empty() || is_junk_line(line) {
            return CommitOutcome::None;
        }
        if !looks_sentence_complete(line) {
            let wc = word_count(line);
            if leave_window {
                // Left the surface: commit short real phrases, not open stubs.
                if wc < 2 || is_open_phrase_stub(line) {
                    return CommitOutcome::None;
                }
            } else if wc < 5 {
                // Mid-stream: never finalize tiny incomplete scraps.
                return CommitOutcome::None;
            }
        }
        for c in self.committed.iter_mut() {
            if same_or_refinement(c, line) {
                if line_quality(line) > line_quality(c) {
                    *c = line.to_string();
                    return CommitOutcome::Revised(line.to_string());
                }
                return CommitOutcome::None;
            }
        }
        self.committed.push(line.to_string());
        CommitOutcome::New(line.to_string())
    }

    fn flush_partial_if_any(&mut self, force: bool) -> BufferEmit {
        self.stable_ticks = 0;
        let last = last_segment(&self.previous);
        if last.is_empty() {
            return BufferEmit::None;
        }
        if !force {
            // Mid-session soft flush: keep incomplete live edge as PARTIAL only.
            if !looks_sentence_complete(&last) {
                return BufferEmit::Partial(last);
            }
            return match self.try_commit(&last, false) {
                CommitOutcome::New(t) => BufferEmit::Final(t),
                CommitOutcome::Revised(t) => BufferEmit::Revised(t),
                CommitOutcome::None => BufferEmit::Partial(last),
            };
        }
        // Session end / forced settle: leave-window rules (short real phrases commit;
        // open stubs like "see the" still rejected).
        match self.try_commit(&last, true) {
            CommitOutcome::New(t) => BufferEmit::Final(t),
            CommitOutcome::Revised(t) => BufferEmit::Revised(t),
            CommitOutcome::None => BufferEmit::None,
        }
    }

    fn diff_emit(&mut self, prev: &str, curr: &str) -> BufferEmit {
        let prev_lines: Vec<String> = segment_captions(prev);
        let curr_lines: Vec<String> = segment_captions(curr);

        if curr_lines.is_empty() {
            // Entire surface cleared — leave-window finalize previous segments.
            let mut finals = Vec::new();
            for p in &prev_lines {
                match self.try_commit(p, true) {
                    CommitOutcome::New(t) => finals.push(t),
                    CommitOutcome::Revised(t) => finals.push(t),
                    CommitOutcome::None => {}
                }
            }
            return match finals.len() {
                0 => BufferEmit::None,
                1 => BufferEmit::Final(finals.remove(0)),
                _ => BufferEmit::Finals(finals),
            };
        }

        let mut finals = Vec::new();
        let mut revised: Option<String> = None;

        // Segments that left the rolling window → finalize once (short phrases OK).
        for p in &prev_lines {
            let still = curr_lines.iter().any(|c| same_or_refinement(c, p));
            if !still {
                match self.try_commit(p, true) {
                    CommitOutcome::New(t) => finals.push(t),
                    CommitOutcome::Revised(t) => revised = Some(t),
                    CommitOutcome::None => {}
                }
            }
        }

        // Settled segments (all but last) in current surface — including
        // complete sentences inside one Windows UIA blob.
        if curr_lines.len() >= 2 {
            for line in &curr_lines[..curr_lines.len() - 1] {
                // Settled rows should be complete enough; incomplete mid-segments wait.
                if !looks_sentence_complete(line) && word_count(line) < 8 {
                    continue;
                }
                match self.try_commit(line, false) {
                    CommitOutcome::New(t) => {
                        if !finals.iter().any(|f| same_or_refinement(f, &t)) {
                            finals.push(t);
                        }
                    }
                    CommitOutcome::Revised(t) => revised = Some(t),
                    CommitOutcome::None => {}
                }
            }
        }

        let last = curr_lines.last().cloned().unwrap_or_default();

        // Live edge growing: if last is a polish of the previous last segment, surface Partial
        // (or Revised if we already committed that family). Never New-final mid-growth.
        if let Some(prev_last) = prev_lines.last() {
            if same_or_refinement(prev_last, &last) && last != *prev_last {
                match self.try_commit(&last, false) {
                    CommitOutcome::Revised(t) => {
                        return BufferEmit::Revised(t);
                    }
                    CommitOutcome::New(_) | CommitOutcome::None => {
                        return BufferEmit::Partial(last);
                    }
                }
            }
        }

        if let Some(r) = revised {
            if finals.is_empty() {
                return BufferEmit::Revised(r);
            }
        }

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

    /// End of session / LC stop: leave-window commit every held segment (not only last).
    pub fn finish(&mut self) -> BufferEmit {
        self.stable_ticks = 0;
        if self.previous.is_empty() {
            return BufferEmit::None;
        }
        let segs = segment_captions(&self.previous);
        self.previous.clear();
        let mut finals = Vec::new();
        for p in &segs {
            match self.try_commit(p, true) {
                CommitOutcome::New(t) | CommitOutcome::Revised(t) => {
                    if !finals.iter().any(|f: &String| same_or_refinement(f, &t)) {
                        finals.push(t);
                    }
                }
                CommitOutcome::None => {}
            }
        }
        match finals.len() {
            0 => BufferEmit::None,
            1 => BufferEmit::Final(finals.remove(0)),
            _ => BufferEmit::Finals(finals),
        }
    }

    pub fn reset(&mut self) {
        self.previous.clear();
        self.committed.clear();
        self.stable_ticks = 0;
    }
}

fn partial_or_none(_buf: &CaptionBuffer, last: &str) -> BufferEmit {
    if last.is_empty() {
        BufferEmit::None
    } else {
        // Always surface the live edge while the OS is still rewriting text.
        BufferEmit::Partial(last.to_string())
    }
}

fn word_count(s: &str) -> usize {
    s.split_whitespace().count()
}

/// Open LC stubs that must not become FINAL mid-phrase or as tiny leave-window scraps
/// when a longer same-family line is about to follow.
fn is_open_phrase_stub(s: &str) -> bool {
    let l = s.trim().to_ascii_lowercase();
    if looks_sentence_complete(s) {
        return false;
    }
    matches!(
        l.as_str(),
        "see the"
            | "is that"
            | "and then"
            | "so this"
            | "so this is"
            | "i think"
            | "and"
            | "the"
            | "so"
            | "but"
            | "wait"
            | "oh"
            | "yeah"
            | "yes"
            | "no"
            | "um"
            | "uh"
    ) || (word_count(s) <= 2
        && (l.ends_with(" the")
            || l.ends_with(" a")
            || l.ends_with(" an")
            || l.starts_with("and ")
            || l.starts_with("so ")
            || l.starts_with("but ")))
}

fn split_caption_lines(s: &str) -> Vec<String> {
    s.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !is_junk_line(l))
        .map(|s| s.to_string())
        .collect()
}

/// Split a continuous Live Captions blob into sentence-like segments.
/// Windows often returns one growing string without newlines; we still need
/// draft→polish and “left the window” behaviour per spoken sentence.
fn split_sentences(s: &str) -> Vec<String> {
    let s = s.trim();
    if s.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut start = 0usize;
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len();
    let mut i = 0usize;
    while i < n {
        let c = chars[i];
        let is_end = c == '.' || c == '?' || c == '!' || c == '…';
        if is_end {
            // Avoid splitting decimals / ellipsis mid-number (3.4, 88%).
            let prev_digit = i > 0 && chars[i - 1].is_ascii_digit();
            let next_digit = i + 1 < n && chars[i + 1].is_ascii_digit();
            if c == '.' && prev_digit && next_digit {
                i += 1;
                continue;
            }
            // Include closing quotes after punct.
            let mut end = i + 1;
            while end < n && (chars[end] == '"' || chars[end] == '\u{201d}' || chars[end] == '\'')
            {
                end += 1;
            }
            let seg: String = chars[start..end].iter().collect::<String>().trim().to_string();
            if !seg.is_empty() && !is_junk_line(&seg) {
                out.push(seg);
            }
            // Skip spaces after sentence end.
            i = end;
            while i < n && chars[i].is_whitespace() {
                i += 1;
            }
            start = i;
            continue;
        }
        i += 1;
    }
    if start < n {
        let seg: String = chars[start..].iter().collect::<String>().trim().to_string();
        if !seg.is_empty() && !is_junk_line(&seg) {
            out.push(seg);
        }
    }
    if out.is_empty() && !is_junk_line(s) {
        out.push(s.to_string());
    }
    out
}

/// Newline rows, then sentence segments (Windows rolling UIA).
fn segment_captions(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in split_caption_lines(s) {
        for seg in split_sentences(&line) {
            out.push(seg);
        }
    }
    out
}

fn last_segment(s: &str) -> String {
    segment_captions(s).into_iter().last().unwrap_or_default()
}

fn clean_surface(s: &str) -> String {
    // Prefer a short tail so we track the live edge, not a giant sticky history blob.
    let segs = segment_captions(s);
    if segs.is_empty() {
        return String::new();
    }
    // Keep last 4 segments for diffing (live + recent settled context).
    let start = segs.len().saturating_sub(4);
    segs[start..].join("\n")
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
        || l.contains("remove window")
        || l.contains("window from")
        || l.contains("from set")
        || l.contains("move & resize")
        || l.contains("move and resize")
        || l.contains("merge all windows")
        || l.contains("bring all to front")
        || l.starts_with("edit ")
        || l.starts_with("view ")
        || l.starts_with("window ")
        || l.starts_with("file ")
        || (l.contains("window")
            && (l.contains("remove")
                || l.contains("move")
                || l.contains("tile")
                || l.contains("arrange")
                || l.contains("fill")
                || l.contains("center")
                || l.contains("zoom")
                || l.contains("minimize")
                || l.contains("set")))
        || (l.contains("quit") && l.contains("live captions"))
        || (l.contains("spelling") && l.contains("grammar"))
        || (l.contains("spelling") && l.contains("automatically"))
        || (l.contains("capitali") && l.contains("automatically"))
    {
        return true;
    }
    // Short menu-bar commands only (not free speech that happens to start with open/help/save).
    // Pass original casing so Title-Case UI labels can be distinguished from speech.
    if words_look_like_menu_command(t) {
        return true;
    }
    // Filename-like single tokens (not spoken captions).
    if !t.contains(' ') && (t.contains('.') || t.contains('_') || t.contains('-')) {
        return true;
    }
    if !t.contains(' ') && t.chars().count() < 16 {
        return true;
    }
    false
}

/// Edit/Window/app menu commands — not free speech that starts with the same verbs.
///
/// "Open the door" / "Help me understand this" / "Save our jobs at the plant" are speech
/// (lowercase content words). "Paste and Match Style" / "Select All" / "Open File" are chrome.
fn words_look_like_menu_command(s: &str) -> bool {
    let lower = s.to_ascii_lowercase();
    let words: Vec<&str> = lower.split_whitespace().collect();
    if words.is_empty() || words.len() > 6 {
        return false;
    }
    // Explicit multi-word chrome (always junk regardless of casing).
    if lower.contains("match style")
        || lower.contains("dictation")
        || lower.contains("emoji")
        || lower.contains("special character")
        || lower.contains("default position")
        || lower.contains("default size")
        || lower.contains("to selection")
        || lower.ends_with(" selection")
        || lower.contains("left to right")
        || lower.contains("right to left")
        || lower.contains("force quit")
        || lower.contains("full screen")
        || lower.contains("previous size")
        || lower.contains("quarters")
        || lower.contains("halves")
        || lower.contains("bottom &")
        || lower.contains("top &")
        || lower.contains("left &")
        || lower.contains("right &")
        || lower.contains("on screen")
        || lower.contains("on top")
        || lower.contains("log out")
        || lower.contains("log off")
        || lower.starts_with("sign out")
        || lower.starts_with("sign in")
        || lower.starts_with("lock screen")
        || (s.contains('&') && looks_like_title_case_chrome(s))
        // Short Title-Case labels ending in ellipsis are almost always menus.
        || ((s.ends_with('…') || s.ends_with("..."))
            && words_count(s) <= 5
            && looks_like_title_case_chrome(s.trim_end_matches(['…', '.'])))
        // "KeyboardAccessAgent Help" style AX chrome (CamelCase process + Help).
        || looks_like_app_help_label(s)
    {
        return true;
    }
    // Sentence punctuation → spoken line, not a menu label.
    if lower.contains('.') || lower.contains('?') || lower.contains('!') {
        return false;
    }
    let first = words[0];
    let is_menu_verb = matches!(
        first,
        "paste"
            | "copy"
            | "cut"
            | "undo"
            | "redo"
            | "select"
            | "hide"
            | "show"
            | "bring"
            | "send"
            | "make"
            | "create"
            | "add"
            | "remove"
            | "merge"
            | "arrange"
            | "tile"
            | "fill"
            | "center"
            | "zoom"
            | "minimize"
            | "close"
            | "save"
            | "open"
            | "print"
            | "find"
            | "replace"
            | "delete"
            | "duplicate"
            | "rename"
            | "export"
            | "import"
            | "share"
            | "start"
            | "stop"
            | "toggle"
            | "use"
            | "return"
            | "enter"
            | "exit"
            | "force"
            | "check"
            | "correct"
            | "capitalize"
            | "capitalise"
            | "move"
            | "resize"
            | "format"
            | "insert"
            | "services"
            | "preferences"
            | "settings"
            | "about"
            | "quit"
            | "help"
            | "window"
            | "file"
            | "edit"
            | "view"
            | "go"
            | "restore"
            | "reset"
            | "clear"
            | "empty"
            | "turn"
            | "enable"
            | "disable"
            | "jump"
            | "scroll"
            | "navigate"
            | "keep"
            | "pin"
            | "float"
    );
    if !is_menu_verb {
        return false;
    }
    // Long lowercase content words after the verb ⇒ spoken sentence, not a menu label.
    // e.g. "Open the door", "Help me understand this", "Check your email when you can".
    if has_long_lowercase_content(s) {
        return false;
    }
    // 1–2 token menu items ("Copy", "Select All", "Open File") or Title-Case menu phrases.
    words.len() <= 2 || looks_like_title_case_chrome(s)
}

/// True when a token is lowercase and length ≥ 4 (speech glue: said, door, email, …).
fn has_long_lowercase_content(s: &str) -> bool {
    s.split_whitespace().any(|w| {
        w.chars()
            .next()
            .map(|c| c.is_ascii_lowercase())
            .unwrap_or(false)
            && w.chars().count() >= 4
    })
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
    // Only penalize Finder/spelling chrome — not ordinary speech starting with "show ".
    if lower.contains(" in finder")
        || lower.contains("spelling")
        || lower.contains("correct spelling")
    {
        score -= 800;
    }
    // Mild penalty for very short lines — keep 3-word speech partials pickable
    // ("President Biden said", "New York Times", "Good Morning America").
    if words < 2 {
        score -= 100;
    } else if words < 3 {
        score -= 30;
    }
    // Do **not** score-down all Title-Case short phrases: proper-noun live edges
    // ("New York Times") must remain pickable. Menu Title-Case is already is_junk_line.
    score
}

fn words_count(s: &str) -> usize {
    s.split_whitespace().count()
}

/// e.g. "KeyboardAccessAgent Help" — not spoken captions.
fn looks_like_app_help_label(s: &str) -> bool {
    let words: Vec<&str> = s.split_whitespace().collect();
    if words.len() != 2 {
        return false;
    }
    if !words[1].eq_ignore_ascii_case("help") {
        return false;
    }
    let name = words[0];
    // CamelCase / PascalCase process or app id
    let has_upper = name.chars().any(|c| c.is_ascii_uppercase());
    let has_lower = name.chars().any(|c| c.is_ascii_lowercase());
    has_upper && has_lower && name.len() >= 6
}

/// True for short menu/layout labels: mostly Capitalized, no long lowercase content words.
fn looks_like_title_case_chrome(s: &str) -> bool {
    let s = s.trim().trim_end_matches(['…', '.']);
    if s.contains('?') || s.contains('!') {
        return false;
    }
    // Allow a single trailing period only if not a multi-sentence line.
    if s.matches('.').count() > 0 {
        return false;
    }
    let words: Vec<&str> = s.split_whitespace().collect();
    if words.len() < 2 || words.len() > 5 {
        return false;
    }
    let caps = words
        .iter()
        .filter(|w| {
            w.chars()
                .next()
                .map(|c| c.is_ascii_uppercase())
                .unwrap_or(false)
        })
        .count();
    // Long lowercase token (≥4) usually means speech ("said", "about"), not menu chrome.
    let lower_content = words.iter().any(|w| {
        w.chars()
            .next()
            .map(|c| c.is_ascii_lowercase())
            .unwrap_or(false)
            && w.chars().count() >= 4
    });
    if lower_content {
        return false;
    }
    // All or all-but-one words Capitalized (allows "to"/"and"/"from" in the middle).
    caps >= words.len().saturating_sub(1)
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

/// Same spoken line or live rewrite (draft → polish / growing LC phrase).
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
    // Growing Live Captions phrase: "see the" → "see the problem with…"
    if na.len() >= 4 && (nb.starts_with(&na) || na.starts_with(&nb)) {
        return true;
    }
    if na.len() >= 8 && nb.len() >= 8 && (na.contains(&nb) || nb.contains(&na)) {
        return true;
    }

    let ta: Vec<&str> = na.split_whitespace().collect();
    let tb: Vec<&str> = nb.split_whitespace().collect();
    // Word-prefix growth (2+ shared leading words, one side extends).
    if ta.len() >= 2 && tb.len() >= 2 {
        let pre = ta.len().min(tb.len());
        let mut shared = 0usize;
        for i in 0..pre {
            if ta[i] == tb[i] {
                shared += 1;
            } else {
                break;
            }
        }
        if shared >= 2 && shared == pre && ta.len() != tb.len() {
            return true;
        }
    }
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
        "Remove Window from Set",
        "Move & Resize",
        "Bring All to Front",
        "Merge All Windows",
        "Paste and Match Style",
        "Copy",
        "Select All",
        "Start Dictation…",
        "Start Dictation",
        "Restore Default Position",
        "Create Key Points",
    ];

    const REAL_CAPTION: &str = "The fuel is free, which sounds wonderful until you remember free fuel arrives on the weather schedule, not on yours.";

    /// Short proper-noun / partial speech must NOT be treated as menu chrome.
    const REAL_SHORT_PARTIALS: &[&str] = &[
        "President Biden said",
        "New York Times",
        "Good Morning America",
        "Senator Warren argued",
        "Los Angeles traffic",
        "United Nations meeting",
    ];

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
    fn short_proper_noun_partials_are_not_junk() {
        for s in REAL_SHORT_PARTIALS {
            assert!(
                !is_junk_line(s),
                "short speech must pass junk filter: {s:?}"
            );
            // Sole live-edge partials must remain pickable (not score-killed).
            assert!(
                pick_caption_surface(std::iter::once(*s)).is_some(),
                "pick_caption_surface must keep real partial: {s:?}"
            );
        }
        // Menu chrome must not win as caption surface.
        assert_eq!(
            pick_caption_surface(std::iter::once("Restore Default Position")),
            None
        );
        assert_eq!(
            pick_caption_surface(std::iter::once("Paste and Match Style")),
            None
        );
    }

    /// Imperative speech that shares a first word with menu verbs must stay speech.
    #[test]
    fn imperative_speech_not_menu_junk() {
        const SPEECH: &[&str] = &[
            "Open the door",
            "Help me understand this",
            "Check your email when you can",
            "Save our jobs at the plant",
            "Show me the way home",
            "Start the meeting after lunch",
            "Close your eyes and listen carefully",
        ];
        for s in SPEECH {
            assert!(
                !is_junk_line(s),
                "imperative speech must not be menu junk: {s:?}"
            );
            assert!(
                pick_caption_surface(std::iter::once(*s)).is_some(),
                "imperative speech must be pickable: {s:?}"
            );
        }
        // True 1–2 word menu items still junk.
        assert!(is_junk_line("Copy"));
        assert!(is_junk_line("Select All"));
        assert!(is_junk_line("Open File"));
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
                BufferEmit::Final(_) | BufferEmit::Finals(_) | BufferEmit::Revised(_) => {
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

    #[test]
    fn growing_phrase_is_refinement_not_new_finals() {
        // Windows LC often extends one incomplete phrase in place.
        assert!(same_or_refinement("see the", "see the problem with being the most valuable"));
        assert!(same_or_refinement(
            "is that",
            "is that to stay there? You need more customers"
        ));
        let mut b = CaptionBuffer::new();
        b.stable_needed = 2;
        let _ = b.observe("see the");
        let _ = b.observe("see the");
        // Must not finalize tiny incomplete scraps.
        match b.observe("see the") {
            BufferEmit::Final(t) => panic!("premature final: {t}"),
            BufferEmit::Finals(v) => panic!("premature finals: {v:?}"),
            _ => {}
        }
        let mut saw_partial = false;
        for s in [
            "see the problem with being the most valuable company on the planet",
            "see the problem with being the most valuable company on the planet.",
        ] {
            match b.observe(s) {
                BufferEmit::Partial(t) => {
                    saw_partial = true;
                    assert!(t.contains("problem") || t.contains("see the"));
                }
                BufferEmit::Final(t) => {
                    assert!(t.contains("problem") || t.contains("planet"), "{t}");
                }
                BufferEmit::Revised(t) => {
                    assert!(t.contains("problem") || t.contains("planet"), "{t}");
                }
                BufferEmit::Finals(_) => {}
                BufferEmit::None => {}
            }
        }
        // Complete sentence with period should settle without leaving "see the" alone.
        let _ = b.observe("see the problem with being the most valuable company on the planet.");
        let _ = b.observe("see the problem with being the most valuable company on the planet.");
        assert!(
            !b.committed.iter().any(|c| c.trim() == "see the"),
            "committed={:?}",
            b.committed
        );
        let _ = saw_partial;
    }

    #[test]
    fn windows_blob_segments_and_revises_polish() {
        let mut b = CaptionBuffer::new();
        b.stable_needed = 1;
        let draft = "Apple needs you to buy it. I don't know if you noticed";
        let polished =
            "Apple needs you to buy it. I don't know if you've noticed or not, but everyone has an iPhone.";
        let _ = b.observe(draft);
        let _ = b.observe(draft);
        // Grow / polish second sentence while first stays.
        match b.observe(polished) {
            BufferEmit::Partial(t) => assert!(t.contains("iPhone") || t.contains("noticed")),
            BufferEmit::Revised(t) => assert!(t.contains("noticed") || t.contains("iPhone")),
            BufferEmit::Final(t) => assert!(!t.is_empty()),
            BufferEmit::Finals(v) => assert!(!v.is_empty()),
            BufferEmit::None => {}
        }
        // First sentence should be one family at most in committed when settled.
        let first_family = b
            .committed
            .iter()
            .filter(|c| c.contains("Apple needs you to buy it"))
            .count();
        assert!(first_family <= 1, "committed={:?}", b.committed);
    }

    /// Field pack 2026-08-06_07-00-16: growing first line, then short "Wait a second", then next sentence.
    #[test]
    fn field_session_wait_a_second_not_lost_and_no_stub_finals() {
        let mut b = CaptionBuffer::new();
        b.stable_needed = 2;

        // Growing first utterance (Windows-style single rolling string with commas).
        let s1 = "so this is so this just arrived in the post today, the snap maker you won and it has a hole in the side so I hope that's not broken.";
        let _ = b.observe("so this is");
        let _ = b.observe("so this is so this just arrived in the post today");
        let _ = b.observe(s1);
        let _ = b.observe(s1);
        let _ = b.observe(s1);

        // Short phrase appears as live edge (appended after period → new segment).
        let with_wait = format!("{s1} Wait a second");
        match b.observe(&with_wait) {
            BufferEmit::Partial(t) => assert!(
                t.to_ascii_lowercase().contains("wait"),
                "live edge should be Wait… got {t}"
            ),
            other => panic!("expected Partial for Wait a second, got {other:?}"),
        }

        // Next unrelated phrase replaces Wait — leave-window should commit "Wait a second".
        let next = format!("{s1} And apparently this has to be done differently for tool heads.");
        let mut got_wait_final = false;
        match b.observe(&next) {
            BufferEmit::Final(t) | BufferEmit::Revised(t) => {
                if t.to_ascii_lowercase().contains("wait") {
                    got_wait_final = true;
                }
            }
            BufferEmit::Finals(v) => {
                got_wait_final = v.iter().any(|t| t.to_ascii_lowercase().contains("wait"));
            }
            BufferEmit::Partial(_) | BufferEmit::None => {}
        }
        // Either committed now or still as partial until finish; must not leave silently with no trace.
        let wait_in_committed = b
            .committed
            .iter()
            .any(|c| c.to_ascii_lowercase().contains("wait a second"));
        assert!(
            got_wait_final || wait_in_committed,
            "Wait a second must be finalized on leave-window; committed={:?} got_wait={got_wait_final}",
            b.committed
        );

        // Growing stub "see the" → longer phrase must not leave a lone "see the" FINAL.
        let mut b2 = CaptionBuffer::new();
        b2.stable_needed = 2;
        for _ in 0..4 {
            let _ = b2.observe("see the");
        }
        assert!(
            !b2.committed.iter().any(|c| c.trim() == "see the"),
            "must not final stub see the; committed={:?}",
            b2.committed
        );
        let full = "see the problem with being the most valuable company on the planet.";
        for _ in 0..3 {
            let _ = b2.observe(full);
        }
        assert!(
            !b2.committed.iter().any(|c| c.trim() == "see the"),
            "after growth still no lone stub; committed={:?}",
            b2.committed
        );
    }

    #[test]
    fn open_phrase_stubs_detected() {
        assert!(is_open_phrase_stub("see the"));
        assert!(is_open_phrase_stub("is that"));
        assert!(!is_open_phrase_stub("Wait a second"));
        assert!(!is_open_phrase_stub("I think that's right."));
    }

    /// Session end must commit short real live-edge phrases (not drop as Partial).
    #[test]
    fn finish_commits_wait_a_second_not_stub() {
        let mut b = CaptionBuffer::new();
        b.stable_needed = 2;
        let _ = b.observe("Wait a second");
        let _ = b.observe("Wait a second");
        match b.finish() {
            BufferEmit::Final(t) => assert!(
                t.to_ascii_lowercase().contains("wait a second"),
                "got {t}"
            ),
            other => panic!("finish must Final short real phrase, got {other:?}"),
        }
        assert!(
            b.committed
                .iter()
                .any(|c| c.to_ascii_lowercase().contains("wait a second"))
        );

        // Open stubs still rejected on finish.
        let mut b2 = CaptionBuffer::new();
        let _ = b2.observe("see the");
        let _ = b2.observe("see the");
        match b2.finish() {
            BufferEmit::None => {}
            BufferEmit::Final(t) | BufferEmit::Revised(t) | BufferEmit::Partial(t) => {
                panic!("must not commit stub on finish: {t}")
            }
            BufferEmit::Finals(v) => panic!("must not commit stubs: {v:?}"),
        }
        assert!(
            !b2.committed.iter().any(|c| c.trim() == "see the"),
            "committed={:?}",
            b2.committed
        );
    }

    /// Empty surface observe must leave-window finalize short real phrases in previous.
    #[test]
    fn empty_surface_leave_window_commits_short_phrase() {
        let mut b = CaptionBuffer::new();
        let _ = b.observe("Wait a second");
        match b.observe("") {
            BufferEmit::Final(t) => assert!(
                t.to_ascii_lowercase().contains("wait"),
                "got {t}"
            ),
            BufferEmit::Finals(v) => assert!(
                v.iter().any(|t| t.to_ascii_lowercase().contains("wait")),
                "{v:?}"
            ),
            other => panic!("empty surface must leave-window Final, got {other:?}"),
        }
        assert!(b.previous.is_empty());
        assert!(
            b.committed
                .iter()
                .any(|c| c.to_ascii_lowercase().contains("wait a second"))
        );
    }
}
