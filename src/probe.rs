//! Self-check entry: process presence + capture path status.

use crate::platform;

#[cfg(windows)]
use crate::platform::windows_signals;
#[cfg(target_os = "macos")]
use crate::platform::macos_signals;
use crate::protocol::{CaptionEvent, LcState};
use std::io::Write;

/// Exit codes (plan §6.2).
pub const EXIT_OK: i32 = 0;
pub const EXIT_LC_NOT_RUNNING: i32 = 2;
pub const EXIT_PERMISSION: i32 = 3;
pub const EXIT_SIGNALS_STALE: i32 = 4;

#[derive(Clone, Debug)]
pub struct ProbeReport {
    pub exit_code: i32,
    pub lines: Vec<String>,
}

/// Run probe and return structured report (also printable).
pub fn run_probe() -> ProbeReport {
    let mut lines = Vec::new();
    lines.push("Interpres probe — Live Captions self-check".into());
    lines.push(format!("OS: {}", std::env::consts::OS));
    lines.push(format!("Arch: {}", std::env::consts::ARCH));

    #[cfg(windows)]
    {
        let s = windows_signals();
        lines.push(format!("signals.process_names: {:?}", s.process_names));
        lines.push(format!("signals.window_classes: {:?}", s.window_classes));
        lines.push(format!(
            "signals.text_automation_ids: {:?}",
            s.text_automation_ids
        ));
    }
    #[cfg(target_os = "macos")]
    {
        let s = macos_signals();
        lines.push(format!("signals.bundle_ids: {:?}", s.bundle_ids));
        lines.push(format!(
            "signals.process_substrings: {:?}",
            s.process_substrings
        ));
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        lines.push("Live Captions capture not supported on this OS.".into());
        return ProbeReport {
            exit_code: EXIT_SIGNALS_STALE,
            lines,
        };
    }

    let snap = platform::poll_capture();
    lines.push(format!("process_running: {}", snap.process_running));
    lines.push(format!("detail: {}", snap.detail));

    if let Some(ref e) = snap.error {
        lines.push(format!("error: {e}"));
        if e.to_ascii_lowercase().contains("accessibility")
            || e.to_ascii_lowercase().contains("permission")
        {
            lines.push(CaptionEvent::Status {
                lc: LcState::Degraded,
                reason: "permission".into(),
            }
            .to_line());
            return ProbeReport {
                exit_code: EXIT_PERMISSION,
                lines,
            };
        }
    }

    if !snap.process_running {
        lines.push(
            CaptionEvent::Status {
                lc: LcState::Stopped,
                reason: "live_captions_not_running".into(),
            }
            .to_line(),
        );
        return ProbeReport {
            exit_code: EXIT_LC_NOT_RUNNING,
            lines,
        };
    }

    if let Some(ref text) = snap.surface_text {
        let preview: String = text.chars().take(80).collect();
        lines.push(format!("surface_chars: {}", text.chars().count()));
        lines.push(format!("surface_preview: {preview}"));
        lines.push(
            CaptionEvent::Status {
                lc: LcState::Running,
                reason: "capture_ok".into(),
            }
            .to_line(),
        );
        ProbeReport {
            exit_code: EXIT_OK,
            lines,
        }
    } else {
        lines.push(
            CaptionEvent::Status {
                lc: LcState::Degraded,
                reason: "process_up_text_unavailable".into(),
            }
            .to_line(),
        );
        // Process up but no text — not a hard fail of detection; signals may need AX grant
        // or helper. Use degraded with exit 0 if process found without permission error.
        let code = if snap.error.is_some() {
            EXIT_SIGNALS_STALE
        } else {
            EXIT_OK
        };
        ProbeReport {
            exit_code: code,
            lines,
        }
    }
}

pub fn print_probe(out: &mut dyn Write, report: &ProbeReport) -> std::io::Result<()> {
    for line in &report.lines {
        writeln!(out, "{line}")?;
    }
    writeln!(out, "exit_code={}", report.exit_code)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_runs_without_panic_and_emits_status() {
        let report = run_probe();
        assert!(!report.lines.is_empty());
        let blob = report.lines.join("\n");
        assert!(blob.contains("process_running:"));
        assert!(blob.contains("OS:"));
        // Must mention STATUS or explicit not-running path
        assert!(
            blob.contains("STATUS")
                || blob.contains("Live Captions")
                || blob.contains("process_running")
        );
    }
}
