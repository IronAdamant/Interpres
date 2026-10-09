//! Windows Live Captions text capture via in-process UI Automation (no PowerShell).
//!
//! A background reader thread owns the UIA client (`windows_uia`) and samples the
//! Live Captions window every `READER_INTERVAL_MS`. `poll_text` returns the latest
//! read. A watchdog replaces the thread if reads stop arriving; per-call UIA timeouts
//! normally keep that from ever being needed.

use super::detect::LiveCaptionsPresence;
use super::signals::windows_signals;
use super::windows_uia::{CaptionRead, CaptionsUia, CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};
use super::CaptureSnapshot;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// Hide console windows when a GUI-subsystem app spawns console tools (taskkill, etc.).
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// How often the reader thread samples Live Captions.
const READER_INTERVAL_MS: u64 = 200;
/// No completed read for this long → reader is wedged; replace the thread.
const READER_HUNG_AFTER: Duration = Duration::from_secs(5);
/// One-shot callers (probe / diagnose / first engine poll) wait this long for a first read.
const FIRST_READ_WAIT: Duration = Duration::from_secs(4);
/// Do not respawn a reader that keeps dying more often than this.
const RESPAWN_BACKOFF: Duration = Duration::from_secs(3);

#[derive(Default)]
struct ReaderShared {
    latest: Option<CaptionRead>,
    at: Option<Instant>,
}

struct Reader {
    shared: Arc<Mutex<ReaderShared>>,
    stop: Arc<AtomicBool>,
    handle: JoinHandle<()>,
    started: Instant,
}

impl Reader {
    fn spawn() -> Self {
        let shared = Arc::new(Mutex::new(ReaderShared::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let (sink, stop_t) = (shared.clone(), stop.clone());
        let handle = thread::spawn(move || reader_thread(sink, stop_t));
        crate::debuglog::log("caption reader started (in-process UIA)");
        Self {
            shared,
            stop,
            handle,
            started: Instant::now(),
        }
    }

    fn snapshot(&self) -> (Option<CaptionRead>, Option<Instant>) {
        self.shared
            .lock()
            .map(|g| (g.latest.clone(), g.at))
            .unwrap_or((None, None))
    }

    /// Signal stop. A thread wedged inside a UIA call is detached and exits when it returns.
    fn abandon(self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

fn reader_thread(shared: Arc<Mutex<ReaderShared>>, stop: Arc<AtomicBool>) {
    let publish = |r: CaptionRead| {
        if let Ok(mut g) = shared.lock() {
            g.latest = Some(r);
            g.at = Some(Instant::now());
        }
    };

    let hr = unsafe { CoInitializeEx(ptr::null_mut(), COINIT_MULTITHREADED) };
    let signals = windows_signals();
    let class = signals
        .window_classes
        .first()
        .copied()
        .unwrap_or("LiveCaptionsDesktopWindow");
    // UIA client setup can fail transiently (E_FAIL) while another thread is
    // initializing a client — e.g. a replaced reader still winding down. Retry briefly.
    let mut attempt = 0;
    let init = loop {
        attempt += 1;
        match CaptionsUia::new(class, signals.text_automation_ids) {
            Err(e) if attempt < 5 => {
                crate::debuglog::log(&format!("caption reader init attempt {attempt} failed: {e}"));
                thread::sleep(Duration::from_millis(300));
            }
            other => break other,
        }
    };
    let mut uia = match init {
        Ok(u) => u,
        Err(e) => {
            crate::debuglog::log(&format!("caption reader init failed: {e}"));
            publish(CaptionRead::Error(e));
            if hr >= 0 {
                unsafe { CoUninitialize() };
            }
            return;
        }
    };
    if !uia.has_timeouts {
        crate::debuglog::log("caption reader: IUIAutomation2 unavailable — no per-call timeouts");
    }

    let mut last_kind = "";
    while !stop.load(Ordering::SeqCst) {
        let started = Instant::now();
        let read = uia.read();
        let took = started.elapsed();
        let kind = match &read {
            CaptionRead::Text(_) => "text",
            CaptionRead::Waiting => "waiting",
            CaptionRead::NoWindow => "no_window",
            CaptionRead::Error(_) => "error",
        };
        if kind != last_kind || took > Duration::from_millis(1000) {
            let extra = match &read {
                CaptionRead::Error(e) => format!(" ({e})"),
                _ => String::new(),
            };
            crate::debuglog::log(&format!(
                "caption read: {kind}{extra} in {}ms",
                took.as_millis()
            ));
            last_kind = kind;
        }
        if stop.load(Ordering::SeqCst) {
            break;
        }
        publish(read);
        thread::sleep(Duration::from_millis(READER_INTERVAL_MS));
    }

    drop(uia);
    if hr >= 0 {
        unsafe { CoUninitialize() };
    }
}

struct ReaderSlot {
    reader: Option<Reader>,
    last_spawn: Option<Instant>,
}

static READER: Mutex<ReaderSlot> = Mutex::new(ReaderSlot {
    reader: None,
    last_spawn: None,
});

/// Stop the caption reader (engine stop / app exit). Safe to call anytime.
pub fn shutdown_reader() {
    let taken = READER.lock().ok().and_then(|mut g| g.reader.take());
    if let Some(r) = taken {
        r.abandon();
        crate::debuglog::log("caption reader stopped");
    }
}

/// Latest Live Captions text from the reader thread, restarting the reader when stuck.
pub fn poll_text(presence: LiveCaptionsPresence) -> CaptureSnapshot {
    let snap = |surface_text: Option<String>, error: Option<String>| CaptureSnapshot {
        process_running: true,
        detail: format!("{}; via=uia", presence.detail),
        surface_text,
        error,
    };

    let Ok(mut slot) = READER.lock() else {
        return snap(None, Some("caption reader lock poisoned".into()));
    };

    if slot.reader.as_ref().is_some_and(|r| r.handle.is_finished()) {
        // Thread ended on its own (UIA init failure). Keep its last error visible.
        let r = slot.reader.take().expect("checked");
        let (frame, _) = r.snapshot();
        let backoff_over = slot
            .last_spawn
            .map_or(true, |t| t.elapsed() > RESPAWN_BACKOFF);
        if !backoff_over {
            let msg = match frame {
                Some(CaptionRead::Error(e)) => e,
                _ => "caption reader stopped unexpectedly".into(),
            };
            slot.reader = None;
            return snap(None, Some(msg));
        }
    }

    let mut fresh = false;
    if slot.reader.is_none() {
        slot.reader = Some(Reader::spawn());
        slot.last_spawn = Some(Instant::now());
        fresh = true;
    }

    let reader = slot.reader.as_ref().expect("reader present");
    if fresh {
        let deadline = Instant::now() + FIRST_READ_WAIT;
        while reader.snapshot().1.is_none()
            && !reader.handle.is_finished()
            && Instant::now() < deadline
        {
            thread::sleep(Duration::from_millis(20));
        }
    }

    let (frame, at) = reader.snapshot();
    let stuck = match at {
        Some(at) => at.elapsed() > READER_HUNG_AFTER,
        None => reader.started.elapsed() > READER_HUNG_AFTER,
    };
    if stuck {
        let since = at.map_or(0, |a| a.elapsed().as_secs());
        crate::debuglog::log(&format!(
            "caption reader stuck (no read for {since}s) — replacing thread"
        ));
        if let Some(r) = slot.reader.take() {
            r.abandon();
        }
        return snap(
            None,
            Some("Live Captions is not responding (restarting the reader)".into()),
        );
    }

    match frame {
        Some(CaptionRead::Text(t)) => snap(Some(t), None),
        // Waiting for speech is not a failure — keep the error path quiet.
        Some(CaptionRead::Waiting) | None => snap(None, None),
        Some(CaptionRead::NoWindow) => snap(
            None,
            Some("Live Captions is running but its window was not found".into()),
        ),
        Some(CaptionRead::Error(e)) => snap(None, Some(format!("Live Captions read failed: {e}"))),
    }
}

/// `C:\Windows\System32\LiveCaptions.exe` (what Win+Ctrl+L launches).
fn live_captions_exe() -> PathBuf {
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
    PathBuf::from(root).join("System32").join("LiveCaptions.exe")
}

/// Turn on Windows Live Captions (same as Win+Ctrl+L when it is off).
pub fn launch_live_captions() -> std::io::Result<()> {
    Command::new(live_captions_exe()).spawn().map(|_| ())
}

/// Close and reopen Live Captions (recovers a frozen captions window).
pub fn restart_live_captions() -> std::io::Result<()> {
    shutdown_reader();
    let _ = Command::new("taskkill")
        .args(["/IM", "LiveCaptions.exe", "/F"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .status();
    thread::sleep(Duration::from_millis(800));
    launch_live_captions()
}

/// Extra diagnostics for `interpres diagnose` on Windows.
pub fn diagnose_lines() -> Vec<String> {
    let mut lines = Vec::new();
    let presence = super::detect::live_captions_present();
    lines.push(format!("process_running={}", presence.running));
    lines.push(format!("detail={}", presence.detail));

    if presence.running {
        let started = Instant::now();
        let snap = poll_text(presence);
        lines.push(format!("first_read_ms={}", started.elapsed().as_millis()));
        lines.push(format!("poll_surface={}", snap.surface_text.is_some()));
        if let Some(ref text) = snap.surface_text {
            lines.push(format!("surface_chars={}", text.chars().count()));
            let preview: String = text.chars().take(120).collect();
            lines.push(format!("surface_preview={preview}"));
        } else if snap.error.is_none() {
            lines.push("waiting_for_speech=true (Live Captions open, no text yet)".into());
        }
        if let Some(e) = snap.error {
            lines.push(format!("poll_error={e}"));
        }
        shutdown_reader();
    }

    lines.push("Windows tip: Live Captions is Win+Ctrl+L (Settings → Accessibility → Captions).".into());
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_no_window_is_standard_flag() {
        // Win32 CREATE_NO_WINDOW — required so GUI PE does not flash consoles.
        assert_eq!(CREATE_NO_WINDOW, 0x0800_0000);
    }

    use crate::platform::UIA_TEST_LOCK;

    /// Like the reader thread: client setup can fail transiently while another client
    /// (e.g. the caption reader) is still winding down, so retry briefly.
    fn uia_client(class: &str, ids: &[&'static str]) -> CaptionsUia {
        let mut last = String::new();
        for _ in 0..5 {
            match CaptionsUia::new(class, ids) {
                Ok(u) => return u,
                Err(e) => last = e,
            }
            thread::sleep(Duration::from_millis(300));
        }
        panic!("UIA client: {last:?}");
    }

    #[test]
    fn uia_client_initializes_with_timeouts() {
        // Exercises CoCreateInstance + CreatePropertyCondition vtable slots on this OS.
        let _g = UIA_TEST_LOCK.lock();
        unsafe { CoInitializeEx(ptr::null_mut(), COINIT_MULTITHREADED) };
        let uia = uia_client("LiveCaptionsDesktopWindow", &["CaptionsTextBlock", "CaptionsScrollViewer"]);
        assert!(uia.has_timeouts, "IUIAutomation2 expected on Windows 10/11");
    }

    #[test]
    fn uia_read_without_window_reports_no_window() {
        let _g = UIA_TEST_LOCK.lock();
        unsafe { CoInitializeEx(ptr::null_mut(), COINIT_MULTITHREADED) };
        let mut uia = uia_client("InterpresNoSuchWindowClass", &["CaptionsTextBlock"]);
        assert_eq!(uia.read(), CaptionRead::NoWindow);
    }

    /// Manual: `cargo test --lib dump_live_surface -- --ignored --nocapture` with LC open.
    #[test]
    #[ignore]
    fn dump_live_surface() {
        let presence = super::super::detect::live_captions_present();
        let snap = poll_text(presence);
        println!("error={:?}", snap.error);
        for (i, line) in snap.surface_text.unwrap_or_default().split('\n').enumerate() {
            println!("{i:02}: {line:?}");
        }
        shutdown_reader();
    }

    #[test]
    fn live_captions_exe_is_under_system32() {
        let p = live_captions_exe();
        assert!(p.ends_with("System32\\LiveCaptions.exe") || p.ends_with("System32/LiveCaptions.exe"));
    }
}
