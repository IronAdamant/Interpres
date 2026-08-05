//! Process presence via pure `std` + OS process listing commands / FFI-free tools.

#[cfg(windows)]
use super::signals::windows_signals;
#[cfg(target_os = "macos")]
use super::signals::macos_signals;

#[derive(Clone, Debug)]
pub struct LiveCaptionsPresence {
    pub running: bool,
    pub detail: String,
}

/// Detect whether OS Live Captions process is running.
pub fn live_captions_present() -> LiveCaptionsPresence {
    #[cfg(windows)]
    {
        return detect_windows();
    }
    #[cfg(target_os = "macos")]
    {
        return detect_macos();
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        LiveCaptionsPresence {
            running: false,
            detail: "unsupported OS for Live Captions detection".into(),
        }
    }
}

#[cfg(windows)]
fn detect_windows() -> LiveCaptionsPresence {
    let signals = windows_signals();
    // tasklist is always available on Windows interactive sessions.
    let output = std::process::Command::new("tasklist")
        .args(["/FO", "CSV", "/NH"])
        .output();
    match output {
        Ok(out) => {
            let text = String::from_utf8_lossy(&out.stdout).to_ascii_lowercase();
            for name in signals.process_names {
                let n = name.to_ascii_lowercase();
                if text.contains(&n) {
                    return LiveCaptionsPresence {
                        running: true,
                        detail: format!("process matched: {name}"),
                    };
                }
            }
            LiveCaptionsPresence {
                running: false,
                detail: "LiveCaptions.exe not in tasklist".into(),
            }
        }
        Err(e) => LiveCaptionsPresence {
            running: false,
            detail: format!("tasklist failed: {e}"),
        },
    }
}

#[cfg(target_os = "macos")]
fn detect_macos() -> LiveCaptionsPresence {
    let signals = macos_signals();
    // pgrep by bundle id path / process name
    // Try pgrep -fl for full command line
    let output = std::process::Command::new("pgrep")
        .args(["-fl", "Live"])
        .output();
    match output {
        Ok(out) => {
            let text = String::from_utf8_lossy(&out.stdout);
            for sub in signals.process_substrings {
                if text.lines().any(|l| l.contains(sub)) {
                    return LiveCaptionsPresence {
                        running: true,
                        detail: format!("process matched: {sub}"),
                    };
                }
            }
            // Also try exact pgrep for Live Captions
            if let Ok(out2) = std::process::Command::new("pgrep")
                .args(["-f", "Live Captions"])
                .output()
            {
                if out2.status.success() && !out2.stdout.is_empty() {
                    return LiveCaptionsPresence {
                        running: true,
                        detail: "pgrep -f 'Live Captions' matched".into(),
                    };
                }
            }
            LiveCaptionsPresence {
                running: false,
                detail: "Live Captions agent not running".into(),
            }
        }
        Err(e) => {
            // Fallback: ps
            if let Ok(out) = std::process::Command::new("ps")
                .args(["-ax", "-o", "command="])
                .output()
            {
                let text = String::from_utf8_lossy(&out.stdout);
                for sub in signals.process_substrings {
                    if text.contains(sub) {
                        return LiveCaptionsPresence {
                            running: true,
                            detail: format!("ps matched: {sub}"),
                        };
                    }
                }
                return LiveCaptionsPresence {
                    running: false,
                    detail: format!("ps scan: Live Captions not found (pgrep err: {e})"),
                };
            }
            LiveCaptionsPresence {
                running: false,
                detail: format!("process scan failed: {e}"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detection_returns_structured_result() {
        let p = live_captions_present();
        // Must not panic; detail non-empty
        assert!(!p.detail.is_empty());
    }
}
