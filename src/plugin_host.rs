//! Spawn caption helpers and read protocol lines (pure std).

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

/// After SHUTDOWN, give the helper this long to print its last lines and exit.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(2);

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
        let mut cmd = Command::new(helper);
        cmd.args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // GUI PE on Windows: avoid a console window for console-subsystem helpers.
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let mut child = cmd.spawn()?;

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
        // Drain stderr (a full pipe would block the helper); keep it for troubleshooting.
        if let Some(stderr) = child.stderr.take() {
            thread::spawn(move || {
                let reader = BufReader::new(stderr);
                for line in reader.lines().map_while(Result::ok) {
                    if !line.trim().is_empty() {
                        crate::debuglog::log(&format!("engine stderr: {line}"));
                    }
                }
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

    /// `Some(description)` once the helper process has exited.
    pub fn exit_status(&mut self) -> Option<String> {
        let child = self.child.as_mut()?;
        match child.try_wait() {
            Ok(Some(status)) => Some(status.to_string()),
            Ok(None) => None,
            Err(e) => Some(format!("unknown ({e})")),
        }
    }

    /// Ask the helper to stop (SHUTDOWN + stdin close), wait briefly so it can print its
    /// last lines, then kill it. Returns any events it printed while finishing.
    pub fn shutdown_collect(&mut self) -> Vec<CaptionEvent> {
        let mut tail = Vec::new();
        if let Some(child) = self.child.as_mut() {
            if let Some(mut stdin) = child.stdin.take() {
                let _ = writeln!(stdin, "SHUTDOWN");
                let _ = stdin.flush();
                // Dropping stdin closes it: helpers may also stop on EOF.
            }
            let deadline = Instant::now() + SHUTDOWN_GRACE;
            while Instant::now() < deadline {
                if let Ok(Some(_)) = child.try_wait() {
                    break;
                }
                thread::sleep(Duration::from_millis(50));
            }
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Some(rx) = self.rx.as_ref() {
            // Reader thread ends at EOF once the process is gone.
            let deadline = Instant::now() + Duration::from_millis(500);
            loop {
                match rx.recv_timeout(Duration::from_millis(50)) {
                    Ok(ev) => tail.push(ev),
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    Err(mpsc::RecvTimeoutError::Timeout) if Instant::now() >= deadline => break,
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                }
            }
        }
        self.child = None;
        self.rx = None;
        tail
    }

    pub fn shutdown(&mut self) {
        let _ = self.shutdown_collect();
    }
}

impl Drop for PluginHost {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Resolve helper path: config override, then platform default under helpers/.
/// On Windows also searches next to `interpres.exe` (portable pack layout).
pub fn default_helper_path() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        // Windows reads Live Captions in-process (UI Automation); no default helper.
        return None;
    }
    #[cfg(target_os = "macos")]
    {
        let name = "captions_loop.sh";
        let rel = Path::new("helpers").join("macos").join(name);
        let mut candidates = Vec::new();
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                candidates.push(dir.join(&rel));
                candidates.push(dir.join(name));
            }
        }
        if let Ok(cwd) = std::env::current_dir() {
            candidates.push(cwd.join(&rel));
        }
        candidates.push(rel);
        for p in candidates {
            if p.is_file() {
                return Some(p);
            }
        }
        return None;
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        None
    }
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

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    fn cmd() -> PathBuf {
        let root = std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
        PathBuf::from(root).join("System32").join("cmd.exe")
    }

    fn wait_events(host: &mut PluginHost, n: usize) -> Vec<CaptionEvent> {
        let mut got = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(5);
        while got.len() < n && Instant::now() < deadline {
            match host.try_recv() {
                Some(ev) => got.push(ev),
                None => thread::sleep(Duration::from_millis(20)),
            }
        }
        got
    }

    #[test]
    fn engine_lines_arrive_and_exit_is_detected() {
        let mut host = PluginHost::start(
            &cmd(),
            &["/C", "echo READY&echo PARTIAL text=Hello th&echo FINAL text=Hello there."],
        )
        .expect("start cmd");
        let got = wait_events(&mut host, 3);
        assert_eq!(got[0], CaptionEvent::Ready);
        assert_eq!(
            got[2],
            CaptionEvent::Final {
                text: "Hello there.".into()
            }
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        while host.exit_status().is_none() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(20));
        }
        assert!(host.exit_status().is_some(), "exit must be detected");
    }

    #[test]
    fn shutdown_collects_lines_printed_while_finishing() {
        // Engine waits for a stdin line (SHUTDOWN), then prints its last caption.
        let mut host = PluginHost::start(&cmd(), &["/C", "set /p x=&echo FINAL text=last words"])
            .expect("start cmd");
        thread::sleep(Duration::from_millis(200));
        assert!(host.exit_status().is_none(), "still waiting for SHUTDOWN");
        let tail = host.shutdown_collect();
        assert!(
            tail.contains(&CaptionEvent::Final {
                text: "last words".into()
            }),
            "{tail:?}"
        );
    }
}
