//! Line protocol between core and caption helpers (no serde).

use std::fmt;

/// One event from a caption source / helper.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CaptionEvent {
    Ready,
    Status {
        lc: LcState,
        reason: String,
    },
    Partial {
        text: String,
    },
    Final {
        text: String,
    },
    Error {
        message: String,
    },
    Log {
        level: String,
        message: String,
    },
    /// Core → helper
    Shutdown,
    /// Unrecognized or empty (ignored by host).
    Unknown(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LcState {
    Running,
    Stopped,
    Degraded,
    Unknown,
}

impl LcState {
    pub fn as_str(self) -> &'static str {
        match self {
            LcState::Running => "running",
            LcState::Stopped => "stopped",
            LcState::Degraded => "degraded",
            LcState::Unknown => "unknown",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "running" | "on" | "yes" => LcState::Running,
            "stopped" | "off" | "no" => LcState::Stopped,
            "degraded" => LcState::Degraded,
            _ => LcState::Unknown,
        }
    }
}

impl CaptionEvent {
    /// Parse one protocol line from a helper.
    pub fn parse_line(line: &str) -> Self {
        let line = line.trim();
        if line.is_empty() {
            return CaptionEvent::Unknown(String::new());
        }
        let (cmd, rest) = match line.split_once(char::is_whitespace) {
            Some((c, r)) => (c, r.trim()),
            None => (line, ""),
        };
        match cmd.to_ascii_uppercase().as_str() {
            "READY" => CaptionEvent::Ready,
            "SHUTDOWN" => CaptionEvent::Shutdown,
            "PARTIAL" => CaptionEvent::Partial {
                text: parse_text_field(rest),
            },
            "FINAL" => CaptionEvent::Final {
                text: parse_text_field(rest),
            },
            "ERROR" => CaptionEvent::Error {
                message: parse_message_field(rest),
            },
            "STATUS" => parse_status(rest),
            "LOG" => parse_log(rest),
            _ => CaptionEvent::Unknown(line.to_string()),
        }
    }

    /// Format as a protocol line (for helpers / tests).
    pub fn to_line(&self) -> String {
        match self {
            CaptionEvent::Ready => "READY".into(),
            CaptionEvent::Shutdown => "SHUTDOWN".into(),
            CaptionEvent::Partial { text } => format!("PARTIAL text={}", escape_text(text)),
            CaptionEvent::Final { text } => format!("FINAL text={}", escape_text(text)),
            CaptionEvent::Error { message } => {
                format!("ERROR message={}", escape_text(message))
            }
            CaptionEvent::Status { lc, reason } => {
                if reason.is_empty() {
                    format!("STATUS lc={}", lc.as_str())
                } else {
                    format!("STATUS lc={} reason={}", lc.as_str(), escape_text(reason))
                }
            }
            CaptionEvent::Log { level, message } => {
                format!("LOG level={} message={}", level, escape_text(message))
            }
            CaptionEvent::Unknown(s) => s.clone(),
        }
    }
}

fn parse_status(rest: &str) -> CaptionEvent {
    let mut lc = LcState::Unknown;
    let mut reason = String::new();
    for part in split_fields(rest) {
        if let Some((k, v)) = part.split_once('=') {
            match k {
                "lc" => lc = LcState::parse(v),
                "reason" => reason = unescape_text(v),
                _ => {}
            }
        }
    }
    CaptionEvent::Status { lc, reason }
}

fn parse_log(rest: &str) -> CaptionEvent {
    let mut level = "info".to_string();
    let mut message = String::new();
    for part in split_fields(rest) {
        if let Some((k, v)) = part.split_once('=') {
            match k {
                "level" => level = v.to_string(),
                "message" => message = unescape_text(v),
                _ => {}
            }
        }
    }
    CaptionEvent::Log { level, message }
}

fn parse_text_field(rest: &str) -> String {
    if let Some(v) = rest.strip_prefix("text=") {
        return unescape_text(v);
    }
    // bare text after command
    unescape_text(rest)
}

fn parse_message_field(rest: &str) -> String {
    if let Some(v) = rest.strip_prefix("message=") {
        return unescape_text(v);
    }
    unescape_text(rest)
}

/// Split on spaces that are not inside percent-encoding (simple: split on ` key=` boundaries is hard;
/// we use: first known keys only — fields separated by space when key= form).
fn split_fields(rest: &str) -> Vec<&str> {
    // "lc=running reason=foo%20bar" — reason may contain encoded spaces only.
    let mut out = Vec::new();
    let mut start = 0;
    let bytes = rest.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b' ' {
            // look ahead for key=
            let next = &rest[i + 1..];
            if next.contains('=') {
                let key = next.split('=').next().unwrap_or("");
                if matches!(key, "lc" | "reason" | "level" | "message" | "text") {
                    if start < i {
                        out.push(&rest[start..i]);
                    }
                    start = i + 1;
                }
            }
        }
        i += 1;
    }
    if start < rest.len() {
        out.push(&rest[start..]);
    }
    out
}

/// Escape spaces and specials for single-line fields.
pub fn escape_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '%' => out.push_str("%25"),
            ' ' => out.push_str("%20"),
            '\n' => out.push_str("%0A"),
            '\r' => out.push_str("%0D"),
            '=' => out.push_str("%3D"),
            _ => out.push(c),
        }
    }
    out
}

/// Decode `%XX` escapes. Decoded bytes are reassembled as UTF-8 so `%C3%A9` → `é`;
/// malformed escapes are kept literally.
pub fn unescape_text(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = |b: u8| (b as char).to_digit(16);
            if let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

impl fmt::Display for CaptionEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_line())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_final_and_partial_roundtrip() {
        let line = CaptionEvent::Final {
            text: "We can meet on Thursday.".into(),
        }
        .to_line();
        match CaptionEvent::parse_line(&line) {
            CaptionEvent::Final { text } => assert_eq!(text, "We can meet on Thursday."),
            other => panic!("unexpected {other:?}"),
        }

        let p = CaptionEvent::Partial {
            text: "hello world".into(),
        }
        .to_line();
        match CaptionEvent::parse_line(&p) {
            CaptionEvent::Partial { text } => assert_eq!(text, "hello world"),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn parse_status_running_with_reason() {
        let e = CaptionEvent::parse_line("STATUS lc=running reason=LiveCaptions%20process");
        match e {
            CaptionEvent::Status { lc, reason } => {
                assert_eq!(lc, LcState::Running);
                assert_eq!(reason, "LiveCaptions process");
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn unescape_keeps_utf8_and_plain_text() {
        assert_eq!(unescape_text("caf%C3%A9%20au%20lait"), "café au lait");
        assert_eq!(unescape_text("Plain text works too"), "Plain text works too");
        assert_eq!(unescape_text("100%25 sure"), "100% sure");
        assert_eq!(unescape_text("bad %zz and tail %"), "bad %zz and tail %");
        match CaptionEvent::parse_line("FINAL text=Résumé ready, see you Tuesday") {
            CaptionEvent::Final { text } => assert_eq!(text, "Résumé ready, see you Tuesday"),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn parse_error() {
        let e = CaptionEvent::parse_line("ERROR message=Accessibility%20denied");
        match e {
            CaptionEvent::Error { message } => assert_eq!(message, "Accessibility denied"),
            other => panic!("unexpected {other:?}"),
        }
    }
}
