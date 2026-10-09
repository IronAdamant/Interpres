//! Durable session transcripts: sticky folder, one dated file per session.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::history_ui::{plan_family, FamilyPlan, STRONG_REPEAT_K};
use crate::session::{format_session_stamp, unique_session_stem};

/// Writes one session's captions to a user-chosen folder.
pub struct TranscriptWriter {
    folder: PathBuf,
    stem: String,
    txt_path: PathBuf,
    txt: File,
    /// Every line of the `.txt` file (no newlines), so a polish rewrites only the tail
    /// from the changed line instead of re-reading and rewriting the whole file.
    lines: Vec<String>,
    jsonl: Option<File>,
    source_label: String,
    line_count: u64,
    /// Physical last caption body on disk (after any write/rewrite/NoOp).
    last_final_text: Option<String>,
}

impl TranscriptWriter {
    /// Begin a new session file in `folder`. Creates the folder if needed.
    /// When `remember` is false, returns `Ok(None)` and writes nothing.
    pub fn begin_session(
        folder: &Path,
        remember: bool,
        write_jsonl: bool,
        source_label: &str,
        now: SystemTime,
    ) -> io::Result<Option<Self>> {
        if !remember {
            return Ok(None);
        }
        fs::create_dir_all(folder)?;
        let stamp = format_session_stamp(now);
        let stem = unique_session_stem(folder, &stamp);
        let txt_path = folder.join(format!("{stem}.txt"));
        let mut txt = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&txt_path)?;

        let local_stamp = stamp.replace('_', " ");
        let lines = vec![
            format!("# Interpres session started {local_stamp}"),
            format!("# Source: {source_label}"),
            format!("# Folder: {}", folder.display()),
            String::new(),
        ];
        for l in &lines {
            writeln!(txt, "{l}")?;
        }
        txt.flush()?;

        let jsonl = if write_jsonl {
            let p = folder.join(format!("{stem}.jsonl"));
            Some(OpenOptions::new().create_new(true).write(true).open(&p)?)
        } else {
            None
        };

        Ok(Some(Self {
            folder: folder.to_path_buf(),
            stem,
            txt_path,
            txt,
            lines,
            jsonl,
            source_label: source_label.to_string(),
            line_count: 0,
            last_final_text: None,
        }))
    }

    pub fn txt_path(&self) -> &Path {
        &self.txt_path
    }

    pub fn folder(&self) -> &Path {
        &self.folder
    }

    pub fn stem(&self) -> &str {
        &self.stem
    }

    pub fn line_count(&self) -> u64 {
        self.line_count
    }

    pub fn last_final_text(&self) -> Option<&str> {
        self.last_final_text.as_deref()
    }

    /// Append a finalized caption, or family-rewrite if it polishes an earlier line.
    pub fn write_final(&mut self, clock_hhmmss: &str, text: &str) -> io::Result<()> {
        self.write_caption(clock_hhmmss, text, false)
    }

    /// Append a caption exactly as given (external engines: each FINAL is authoritative,
    /// so no same-family merging — a repeated "Yes." is a real second line).
    pub fn append_final(&mut self, clock_hhmmss: &str, text: &str) -> io::Result<()> {
        let text = text.trim();
        if text.is_empty() {
            return Ok(());
        }
        self.append_caption(clock_hhmmss, text)
    }

    /// Polish path: same family rewrite rules; still appends only on NoMatch.
    pub fn write_revised(&mut self, clock_hhmmss: &str, text: &str) -> io::Result<()> {
        self.write_caption(clock_hhmmss, text, true)
    }

    fn write_caption(&mut self, clock_hhmmss: &str, text: &str, _from_revised: bool) -> io::Result<()> {
        let text = one_line(text);
        if text.is_empty() {
            return Ok(());
        }
        // Recent caption lines (oldest → newest) as file line indexes.
        let mut tail: Vec<usize> = self
            .lines
            .iter()
            .enumerate()
            .rev()
            .filter(|(_, l)| is_caption_line(l))
            .take(STRONG_REPEAT_K)
            .map(|(i, _)| i)
            .collect();
        tail.reverse();
        let bodies: Vec<String> = tail.iter().map(|&i| caption_body(&self.lines[i])).collect();
        match plan_family(&bodies, &text) {
            FamilyPlan::Append => self.append_caption(clock_hhmmss, &text),
            FamilyPlan::NoOp => Ok(()),
            FamilyPlan::Replace { at, remove } => {
                let at = tail[at];
                // Keep the time the line was first heard; a later polish must not move it.
                let clock = caption_clock(&self.lines[at]).unwrap_or(clock_hhmmss).to_string();
                self.lines[at] = format!("[{clock}] {text}");
                // Merged sentences: drop the lines now contained in this one (all after `at`).
                for &r in remove.iter().rev() {
                    self.lines.remove(tail[r]);
                }
                self.rewrite_from(at)?;
                if let Some(ref mut j) = self.jsonl {
                    let esc = json_escape(&text);
                    let src = json_escape(&self.source_label);
                    writeln!(
                        j,
                        "{{\"v\":1,\"t\":\"{clock_hhmmss}\",\"kind\":\"revised\",\"src\":\"{src}\",\"text\":\"{esc}\"}}"
                    )?;
                    j.flush()?;
                }
                self.refresh_last_final();
                Ok(())
            }
        }
    }

    /// Rewrite the file from line `from` to the end; earlier bytes are untouched, so a
    /// crash mid-write can only affect the last few lines.
    fn rewrite_from(&mut self, from: usize) -> io::Result<()> {
        use std::io::{Seek, SeekFrom};
        let offset: u64 = self.lines[..from].iter().map(|l| l.len() as u64 + 1).sum();
        let mut tail = String::new();
        for l in &self.lines[from..] {
            tail.push_str(l);
            tail.push('\n');
        }
        self.txt.set_len(offset)?;
        self.txt.seek(SeekFrom::Start(offset))?;
        self.txt.write_all(tail.as_bytes())?;
        self.txt.flush()
    }

    fn refresh_last_final(&mut self) {
        self.last_final_text = self
            .lines
            .iter()
            .rev()
            .find(|l| is_caption_line(l))
            .map(|l| caption_body(l));
    }

    /// Append a line to the file and the in-memory copy.
    fn push_line(&mut self, line: String) -> io::Result<()> {
        writeln!(self.txt, "{line}")?;
        self.txt.flush()?;
        self.lines.push(line);
        Ok(())
    }

    fn append_caption(&mut self, clock_hhmmss: &str, text: &str) -> io::Result<()> {
        let text = one_line(text);
        self.push_line(format!("[{clock_hhmmss}] {text}"))?;
        if let Some(ref mut j) = self.jsonl {
            let esc = json_escape(&text);
            let src = json_escape(&self.source_label);
            writeln!(
                j,
                "{{\"v\":1,\"t\":\"{clock_hhmmss}\",\"kind\":\"final\",\"src\":\"{src}\",\"text\":\"{esc}\"}}"
            )?;
            j.flush()?;
        }
        self.line_count += 1;
        self.last_final_text = Some(text);
        Ok(())
    }

    /// Write a `# …` note line (e.g. Live Captions turned off / back on mid-session).
    pub fn write_note(&mut self, clock_hhmmss: &str, note: &str) -> io::Result<()> {
        self.push_line(format!("# [{clock_hhmmss}] {}", one_line(note)))?;
        if let Some(ref mut j) = self.jsonl {
            let esc = json_escape(note);
            writeln!(
                j,
                "{{\"v\":1,\"t\":\"{clock_hhmmss}\",\"kind\":\"note\",\"text\":\"{esc}\"}}"
            )?;
            j.flush()?;
        }
        Ok(())
    }

    pub fn end_session(&mut self, reason: &str) -> io::Result<()> {
        self.push_line(String::new())?;
        self.push_line(format!("# Session ended ({})", one_line(reason)))?;
        if let Some(ref mut j) = self.jsonl {
            let r = json_escape(reason);
            writeln!(
                j,
                "{{\"v\":1,\"kind\":\"session_end\",\"reason\":\"{r}\"}}"
            )?;
            j.flush()?;
        }
        Ok(())
    }
}

/// A caption is one file line: fold any embedded line breaks.
fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn is_caption_line(l: &str) -> bool {
    let t = l.trim_start();
    t.starts_with('[') && t.contains(']') && !t.starts_with("#")
}

/// `HH:MM:SS` from a `[HH:MM:SS] text` caption line.
fn caption_clock(line: &str) -> Option<&str> {
    let rest = line.trim_start().strip_prefix('[')?;
    let end = rest.find(']')?;
    Some(&rest[..end])
}

fn caption_body(line: &str) -> String {
    let t = line.trim();
    if let Some(rest) = t.strip_prefix('[') {
        if let Some(idx) = rest.find(']') {
            return rest[idx + 1..].trim().to_string();
        }
    }
    t.to_string()
}

fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// Format HH:MM:SS from SystemTime for line prefixes.
pub fn format_clock(now: SystemTime) -> String {
    let stamp = format_session_stamp(now);
    if let Some(t) = stamp.split('_').nth(1) {
        return t.replace('-', ":");
    }
    "00:00:00".into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, UNIX_EPOCH};

    fn temp_dir(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "interpres-tr-{tag}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    fn open_writer(dir: &Path) -> TranscriptWriter {
        TranscriptWriter::begin_session(
            dir,
            true,
            true,
            "os-lc-test",
            UNIX_EPOCH + Duration::from_secs(1_700_000_100),
        )
        .unwrap()
        .expect("writer")
    }

    fn caption_bodies(path: &Path) -> Vec<String> {
        fs::read_to_string(path)
            .unwrap()
            .lines()
            .filter(|l| is_caption_line(l))
            .map(caption_body)
            .collect()
    }

    #[test]
    fn remember_off_writes_nothing() {
        let dir = temp_dir("off");
        let w = TranscriptWriter::begin_session(
            &dir,
            false,
            false,
            "test",
            UNIX_EPOCH + Duration::from_secs(1_700_000_000),
        )
        .unwrap();
        assert!(w.is_none());
        assert!(!dir.exists() || fs::read_dir(&dir).map(|d| d.count()).unwrap_or(0) == 0);
    }

    #[test]
    fn one_dated_file_per_session_sticky_folder() {
        let dir = temp_dir("sess");
        let mut w = open_writer(&dir);
        let path1 = w.txt_path().to_path_buf();
        assert!(path1.starts_with(&dir));
        let name = path1.file_name().unwrap().to_string_lossy();
        assert!(name.ends_with(".txt"));
        assert!(name.contains('-') && name.contains('_'));

        w.write_final("12:00:01", "We can meet on Thursday.")
            .unwrap();
        w.write_final("12:00:04", "I'll send the invite.").unwrap();
        w.end_session("user").unwrap();

        let body = fs::read_to_string(&path1).unwrap();
        assert!(body.contains("We can meet on Thursday."));
        assert!(body.contains("I'll send the invite."));
        assert!(body.contains("# Source: os-lc-test"));

        let jsonl = path1.with_extension("jsonl");
        assert!(jsonl.exists());
        let j = fs::read_to_string(&jsonl).unwrap();
        assert!(j.contains("\"kind\":\"final\""));
        assert!(j.contains("We can meet on Thursday."));

        let t1 = UNIX_EPOCH + Duration::from_secs(1_700_000_160);
        let w2 = TranscriptWriter::begin_session(&dir, true, false, "os-lc-test", t1)
            .unwrap()
            .unwrap();
        assert_ne!(w2.txt_path(), path1);
        assert!(w2.txt_path().starts_with(&dir));

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn adjacent_rewrite_one_disk_line() {
        let dir = temp_dir("adj");
        let mut w = open_writer(&dir);
        let path = w.txt_path().to_path_buf();
        w.write_final("12:00:01", "We can meet on Thursday").unwrap();
        assert_eq!(w.line_count(), 1);
        w.write_final("12:00:02", "We can meet on Thursday.").unwrap();
        assert_eq!(w.line_count(), 1, "rewrite must not increment line_count");
        let bodies = caption_bodies(&path);
        assert_eq!(bodies.len(), 1);
        assert_eq!(bodies[0], "We can meet on Thursday.");
        assert_eq!(w.last_final_text(), Some("We can meet on Thursday."));
        let j = fs::read_to_string(path.with_extension("jsonl")).unwrap();
        assert!(j.contains("\"kind\":\"revised\""));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn intervening_family_rewrite_f_g_f_polish_g_polish() {
        let dir = temp_dir("fg");
        let mut w = open_writer(&dir);
        let path = w.txt_path().to_path_buf();

        let f = "I am the owner of Elite Software Automation.";
        let g = "Second sentence about inventory management.";
        let f_polish =
            "I am the owner of Elite Software Automation, and I have been building it for years.";
        let g_polish =
            "Second sentence about inventory management and the whole delivery cycle.";

        w.write_final("12:00:01", f).unwrap();
        w.write_final("12:00:02", g).unwrap();
        assert_eq!(w.line_count(), 2);
        assert_eq!(w.last_final_text(), Some(g));

        w.write_revised("12:00:03", f_polish).unwrap();
        let bodies = caption_bodies(&path);
        assert_eq!(bodies.len(), 2, "must not append a third line for F polish");
        assert!(bodies[0].contains("building it for years"));
        assert_eq!(bodies[1], g);
        // Physical last caption body is still G (not F polish).
        assert_eq!(w.last_final_text(), Some(g));
        assert_eq!(w.line_count(), 2);

        w.write_revised("12:00:04", g_polish).unwrap();
        let bodies = caption_bodies(&path);
        assert_eq!(bodies.len(), 2);
        assert!(bodies[0].contains("building it for years"));
        assert!(bodies[1].contains("delivery cycle"));
        assert_eq!(w.last_final_text(), Some(g_polish));
        assert_eq!(w.line_count(), 2);

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn shorter_same_family_scrap_is_noop() {
        let dir = temp_dir("scrap");
        let mut w = open_writer(&dir);
        let path = w.txt_path().to_path_buf();
        let long = "And your purpose and your role, if you do get this job, will be to actually figure this out, to figure out what is actually going on there.";
        w.write_final("12:00:01", long).unwrap();
        w.write_final("12:00:02", "And your purpose and your role").unwrap();
        let bodies = caption_bodies(&path);
        assert_eq!(bodies.len(), 1);
        assert!(bodies[0].contains("going on there"));
        assert_eq!(w.line_count(), 1);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn polish_keeps_time_line_was_first_heard() {
        // Field log: "[07:04:19] good good yeah." landed above "[07:04:16] is that better,".
        let dir = temp_dir("clock");
        let mut w = open_writer(&dir);
        let path = w.txt_path().to_path_buf();
        w.write_final("07:04:10", "Good good yeah").unwrap();
        w.write_final("07:04:16", "Is that better?").unwrap();
        w.write_revised("07:04:25", "Good, good yeah, had a read up on you").unwrap();
        let raw = fs::read_to_string(&path).unwrap();
        let clocks: Vec<&str> = raw.lines().filter_map(caption_clock).collect();
        assert_eq!(clocks, ["07:04:10", "07:04:16"], "{raw}");
        assert!(raw.contains("[07:04:10] Good, good yeah, had a read up on you"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn note_lines_are_not_captions() {
        let dir = temp_dir("note");
        let mut w = open_writer(&dir);
        let path = w.txt_path().to_path_buf();
        w.write_final("10:00:00", "First caption line here").unwrap();
        w.write_note("10:00:05", "Live Captions turned off").unwrap();
        w.write_final("10:01:00", "Second caption line here").unwrap();
        assert_eq!(caption_bodies(&path).len(), 2);
        let raw = fs::read_to_string(&path).unwrap();
        assert!(raw.contains("# [10:00:05] Live Captions turned off"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn write_revised_on_shipped_path_rewrites() {
        // Drive the public write_revised API (engine uses this for BufferEmit::Revised).
        let dir = temp_dir("rev");
        let mut w = open_writer(&dir);
        let path = w.txt_path().to_path_buf();
        w.write_final("12:00:01", "Thank you for watching this video").unwrap();
        w.write_revised(
            "12:00:02",
            "Thank you for watching this video. If you feel like this is for you, continue.",
        )
        .unwrap();
        let bodies = caption_bodies(&path);
        assert_eq!(bodies.len(), 1);
        assert!(bodies[0].contains("continue"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn merged_polish_rewrites_only_the_tail_and_file_matches_memory() {
        let dir = temp_dir("merge");
        let mut w = open_writer(&dir);
        w.write_final("10:00:01", "Does anyone have questions?").unwrap();
        w.write_note("10:00:02", "Live Captions back on").unwrap();
        w.write_final("10:00:03", "Second, the launch moved to the end of the quarter.").unwrap();
        w.write_final("10:00:05", "I think that makes sense.").unwrap();
        w.write_revised(
            "10:00:06",
            "Second, the launch moved to the end of the quarter, I think that makes sense.",
        )
        .unwrap();
        w.write_final("10:00:09", "Next item.").unwrap();
        w.end_session("user").unwrap();
        let on_disk = fs::read_to_string(w.txt_path()).unwrap();
        let expected: String = w.lines.iter().map(|l| format!("{l}\n")).collect();
        assert_eq!(on_disk, expected);
        let captions: Vec<&str> = on_disk.lines().filter(|l| is_caption_line(l)).collect();
        assert_eq!(
            captions,
            vec![
                "[10:00:01] Does anyone have questions?",
                "[10:00:03] Second, the launch moved to the end of the quarter, I think that makes sense.",
                "[10:00:09] Next item.",
            ]
        );
        assert!(on_disk.contains("# [10:00:02] Live Captions back on"));
        let _ = fs::remove_dir_all(dir);
    }
}
