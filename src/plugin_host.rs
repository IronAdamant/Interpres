//! Spawn caption helpers and read protocol lines (pure std).

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use crate::protocol::CaptionEvent;

pub struct PluginHost {
    child: Option<Child>,
    rx: Option<Receiver<CaptionEvent>>,
}

impl PluginHost {
    pub fn idle() -> Self {
        Self {
            child: None,
            rx: None,
        }
    }

    /// Start a helper process; stdout lines become CaptionEvents.
    pub fn start(helper: &Path, args: &[&str]) -> std::io::Result<Self> {
        let mut child = Command::new(helper)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        let stdout = child.stdout.take().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::Other, "helper missing stdout")
        })?;
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines().flatten() {
                let ev = CaptionEvent::parse_line(&line);
                if tx.send(ev).is_err() {
                    break;
                }
            }
        });
        // Drain stderr to avoid blocking
        if let Some(stderr) = child.stderr.take() {
            thread::spawn(move || {
                let reader = BufReader::new(stderr);
                for _ in reader.lines() {}
            });
        }

        Ok(Self {
            child: Some(child),
            rx: Some(rx),
        })
    }

    pub fn try_recv(&self) -> Option<CaptionEvent> {
        self.rx.as_ref()?.try_recv().ok()
    }

    pub fn shutdown(&mut self) {
        if let Some(child) = self.child.as_mut() {
            if let Some(mut stdin) = child.stdin.take() {
                let _ = writeln!(stdin, "SHUTDOWN");
            }
            let _ = child.kill();
            let _ = child.wait();
        }
        self.child = None;
        self.rx = None;
    }
}

impl Drop for PluginHost {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Resolve helper path: config override, then platform default under helpers/.
pub fn default_helper_path() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        let p = PathBuf::from("helpers/windows/Get-LiveCaptionsText.ps1");
        if p.exists() {
            return Some(p);
        }
    }
    #[cfg(target_os = "macos")]
    {
        let p = PathBuf::from("helpers/macos/captions_loop.sh");
        if p.exists() {
            return Some(p);
        }
    }
    None
}

/// Demo helper: emits canned FINAL lines (for tests / no-LC environments).
pub fn run_demo_source(tx: Sender<CaptionEvent>) {
    let _ = tx.send(CaptionEvent::Ready);
    let _ = tx.send(CaptionEvent::Status {
        lc: crate::protocol::LcState::Running,
        reason: "demo".into(),
    });
    let _ = tx.send(CaptionEvent::Partial {
        text: "Hello from demo".into(),
    });
    let _ = tx.send(CaptionEvent::Final {
        text: "Hello from demo mode.".into(),
    });
    let _ = tx.send(CaptionEvent::Final {
        text: "This is a second transcript line.".into(),
    });
    let _ = tx.send(CaptionEvent::Status {
        lc: crate::protocol::LcState::Stopped,
        reason: "demo_done".into(),
    });
}
