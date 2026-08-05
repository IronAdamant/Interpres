//! Durable session transcripts: sticky folder, one dated file per session.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::session::{format_session_stamp, unique_session_stem};

/// Writes one session's captions to a user-chosen folder.
pub struct TranscriptWriter {
    folder: PathBuf,
    stem: String,
    txt_path: PathBuf,
    txt: File,
    jsonl: Option<File>,
    source_label: String,
    line_count: u64,
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
        writeln!(txt, "# Interpres session started {local_stamp}")?;
        writeln!(txt, "# Source: {source_label}")?;
        writeln!(txt, "# Folder: {}", folder.display())?;
        writeln!(txt)?;
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
            jsonl,
            source_label: source_label.to_string(),
            line_count: 0,
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

    /// Append a finalized caption line (human TXT + optional JSONL).
    pub fn write_final(&mut self, clock_hhmmss: &str, text: &str) -> io::Result<()> {
        let text = text.trim();
        if text.is_empty() {
            return Ok(());
        }
        writeln!(self.txt, "[{clock_hhmmss}] {text}")?;
        self.txt.flush()?;
        if let Some(ref mut j) = self.jsonl {
            let esc = json_escape(text);
            let src = json_escape(&self.source_label);
            writeln!(
                j,
                "{{\"v\":1,\"t\":\"{clock_hhmmss}\",\"kind\":\"final\",\"src\":\"{src}\",\"text\":\"{esc}\"}}"
            )?;
            j.flush()?;
        }
        self.line_count += 1;
        Ok(())
    }

    pub fn end_session(&mut self, reason: &str) -> io::Result<()> {
        writeln!(self.txt)?;
        writeln!(self.txt, "# Session ended ({reason})")?;
        self.txt.flush()?;
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
    // stamp is YYYY-MM-DD_HH-MM-SS → take time part and use colons
    if let Some(t) = stamp.split('_').nth(1) {
        return t.replace('-', ":");
    }
    "00:00:00".into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, UNIX_EPOCH};

    #[test]
    fn remember_off_writes_nothing() {
        let dir = std::env::temp_dir().join(format!(
            "interpres-tr-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
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
        let dir = std::env::temp_dir().join(format!(
            "interpres-tr-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let t0 = UNIX_EPOCH + Duration::from_secs(1_700_000_100);
        let mut w = TranscriptWriter::begin_session(&dir, true, true, "os-lc-test", t0)
            .unwrap()
            .expect("writer");
        let path1 = w.txt_path().to_path_buf();
        assert!(path1.starts_with(&dir));
        let name = path1.file_name().unwrap().to_string_lossy();
        assert!(name.ends_with(".txt"));
        // date-time shape in name
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

        // Second session → different file
        let t1 = t0 + Duration::from_secs(60);
        let w2 = TranscriptWriter::begin_session(&dir, true, false, "os-lc-test", t1)
            .unwrap()
            .unwrap();
        assert_ne!(w2.txt_path(), path1);
        assert!(w2.txt_path().starts_with(&dir));

        let _ = fs::remove_dir_all(dir);
    }
}
