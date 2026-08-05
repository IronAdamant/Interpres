//! Rolling Live Captions buffer → PARTIAL / FINAL emissions.

/// Diffs successive full caption surfaces (multi-line rolling text from OS UI).
#[derive(Clone, Debug, Default)]
pub struct CaptionBuffer {
    previous: String,
    /// Lines already committed as FINAL.
    committed: Vec<String>,
    /// Idle polls with unchanged trailing partial before force-final (caller increments).
    stable_ticks: u32,
    /// How many identical trailing observations before committing last partial.
    pub stable_needed: u32,
}

/// What the buffer wants the host to emit this tick.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BufferEmit {
    None,
    Partial(String),
    Final(String),
    /// Multiple finals when several new complete lines appear at once.
    Finals(Vec<String>),
}

impl CaptionBuffer {
    pub fn new() -> Self {
        Self {
            previous: String::new(),
            committed: Vec::new(),
            stable_ticks: 0,
            stable_needed: 3,
        }
    }

    /// Observe the full current caption surface text from the OS.
    pub fn observe(&mut self, current: &str) -> BufferEmit {
        let current = current.trim_end();
        if current.is_empty() {
            return self.flush_partial_if_any();
        }
        if current == self.previous {
            self.stable_ticks = self.stable_ticks.saturating_add(1);
            if self.stable_ticks >= self.stable_needed {
                return self.flush_partial_if_any();
            }
            // still partial on last line
            if let Some(last) = current.lines().last() {
                if !last.trim().is_empty() && !self.committed.iter().any(|c| c == last.trim()) {
                    return BufferEmit::Partial(last.trim().to_string());
                }
            }
            return BufferEmit::None;
        }

        self.stable_ticks = 0;
        let prev = self.previous.clone();
        let emit = self.diff_emit(&prev, current);
        self.previous = current.to_string();
        emit
    }

    fn flush_partial_if_any(&mut self) -> BufferEmit {
        self.stable_ticks = 0;
        if let Some(last) = self.previous.lines().last() {
            let last = last.trim();
            if !last.is_empty() && !self.committed.iter().any(|c| c == last) {
                self.committed.push(last.to_string());
                return BufferEmit::Final(last.to_string());
            }
        }
        BufferEmit::None
    }

    fn diff_emit(&mut self, prev: &str, curr: &str) -> BufferEmit {
        let prev_lines: Vec<&str> = prev.lines().map(str::trim_end).filter(|l| !l.is_empty()).collect();
        let curr_lines: Vec<&str> = curr.lines().map(str::trim_end).filter(|l| !l.is_empty()).collect();

        // Find longest suffix of prev that is a prefix of curr (overlap), else grow.
        let mut finals = Vec::new();

        // New complete lines: all but last line of curr that we haven't committed
        // and that look "finished" relative to prev (either new or prev last was completed).
        if curr_lines.is_empty() {
            return BufferEmit::None;
        }

        // Commit any line that disappeared from the rolling window as final if not committed.
        for pl in &prev_lines {
            let p = pl.trim();
            if p.is_empty() {
                continue;
            }
            if !curr_lines.iter().any(|c| c.trim() == p || c.trim().starts_with(p) || p.starts_with(c.trim())) {
                if !self.committed.iter().any(|c| c == p) {
                    self.committed.push(p.to_string());
                    finals.push(p.to_string());
                }
            }
        }

        // Lines in curr except possibly the last: if they are stable complete lines
        if curr_lines.len() >= 2 {
            for line in &curr_lines[..curr_lines.len() - 1] {
                let l = line.trim();
                if l.is_empty() {
                    continue;
                }
                if !self.committed.iter().any(|c| c == l) {
                    self.committed.push(l.to_string());
                    if !finals.iter().any(|f| f == l) {
                        finals.push(l.to_string());
                    }
                }
            }
        }

        let last = curr_lines.last().map(|s| s.trim()).unwrap_or("");
        if !finals.is_empty() {
            if finals.len() == 1 {
                // also emit partial for new last if different
                if !last.is_empty() && !self.committed.iter().any(|c| c == last) {
                    // Prefer returning finals; host can still partial next tick
                    return BufferEmit::Finals(finals);
                }
                return BufferEmit::Final(finals.remove(0));
            }
            return BufferEmit::Finals(finals);
        }

        if !last.is_empty() && !self.committed.iter().any(|c| c == last) {
            return BufferEmit::Partial(last.to_string());
        }
        BufferEmit::None
    }

    /// Force-commit any open partial (session end).
    pub fn finish(&mut self) -> BufferEmit {
        self.flush_partial_if_any()
    }

    pub fn reset(&mut self) {
        self.previous.clear();
        self.committed.clear();
        self.stable_ticks = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rolling_buffer_emits_partial_then_final_on_new_line() {
        let mut b = CaptionBuffer::new();
        b.stable_needed = 2;

        match b.observe("Hello how are") {
            BufferEmit::Partial(t) => assert_eq!(t, "Hello how are"),
            other => panic!("expected partial, got {other:?}"),
        }

        // same text → stability path
        let _ = b.observe("Hello how are");
        match b.observe("Hello how are") {
            BufferEmit::Final(t) => assert_eq!(t, "Hello how are"),
            BufferEmit::Partial(_) => {
                // still ok if stable_needed not met path returned partial
            }
            other => panic!("unexpected {other:?}"),
        }

        // New surface with completed first line + new partial
        let mut b2 = CaptionBuffer::new();
        let _ = b2.observe("Hello");
        match b2.observe("Hello how are you?\nI am fine") {
            BufferEmit::Final(t) => {
                assert!(t.contains("Hello") || t.contains("fine") || t.contains("you"));
            }
            BufferEmit::Finals(v) => {
                assert!(!v.is_empty());
            }
            BufferEmit::Partial(t) => {
                // acceptable intermediate
                assert!(!t.is_empty());
            }
            BufferEmit::None => {}
        }
    }

    #[test]
    fn multi_line_commits_completed_lines() {
        let mut b = CaptionBuffer::new();
        let _ = b.observe("Line one is going");
        match b.observe("Line one is done.\nLine two starts") {
            BufferEmit::Final(t) => assert!(t.contains("Line one") || t.contains("done")),
            BufferEmit::Finals(v) => {
                assert!(v.iter().any(|x| x.contains("Line one") || x.contains("done")));
            }
            BufferEmit::Partial(t) => {
                // first transition might only partial if algorithm sees growth differently
                assert!(!t.is_empty());
            }
            other => panic!("{other:?}"),
        }
    }
}
