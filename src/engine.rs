//! Background Live Captions capture engine (pure std). Used by GUI and CLI.

use crate::buffer::{BufferEmit, CaptionBuffer};
use crate::config::Config;
use crate::lifecycle::{Lifecycle, LifecycleAction};
use crate::platform;
use crate::transcript::{format_clock, TranscriptWriter};
use crate::ui_labels::{
    session_open_status, should_clear_live_after_empty, should_show_lag_tip,
    should_skip_stale_surface, LAG_TIP,
};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime};

/// Events the UI (or CLI) can show.
#[derive(Clone, Debug)]
pub enum EngineEvent {
    Status(String),
    Live(String),
    Final(String),
    Error(String),
    SessionFile(Option<PathBuf>),
    Listening(bool),
}

struct EngineInner {
    stop: AtomicBool,
    remember: AtomicBool,
    folder: Mutex<PathBuf>,
}

pub struct CaptureEngine {
    inner: Arc<EngineInner>,
    tx: Sender<EngineEvent>,
    handle: Mutex<Option<JoinHandle<()>>>,
}

impl CaptureEngine {
    pub fn new(cfg: &Config) -> (Self, Receiver<EngineEvent>) {
        let (tx, rx) = mpsc::channel();
        let inner = Arc::new(EngineInner {
            stop: AtomicBool::new(true),
            remember: AtomicBool::new(cfg.remember),
            folder: Mutex::new(cfg.transcript_folder.clone()),
        });
        (
            Self {
                inner,
                tx,
                handle: Mutex::new(None),
            },
            rx,
        )
    }

    pub fn set_remember(&self, on: bool) {
        self.inner.remember.store(on, Ordering::SeqCst);
        let mut cfg = Config::load();
        cfg.remember = on;
        let _ = cfg.save();
    }

    pub fn remember(&self) -> bool {
        self.inner.remember.load(Ordering::SeqCst)
    }

    pub fn set_folder(&self, path: PathBuf) {
        if let Ok(mut g) = self.inner.folder.lock() {
            *g = path.clone();
        }
        crate::debuglog::set_folder(&path);
        let mut cfg = Config::load();
        cfg.transcript_folder = path;
        let _ = cfg.save();
    }

    pub fn folder(&self) -> PathBuf {
        self.inner
            .folder
            .lock()
            .map(|g| g.clone())
            .unwrap_or_else(|_| Config::default().transcript_folder)
    }

    pub fn is_running(&self) -> bool {
        !self.inner.stop.load(Ordering::SeqCst)
            && self
                .handle
                .lock()
                .map(|h| h.is_some())
                .unwrap_or(false)
    }

    pub fn start(&self) {
        // Stop previous if any
        self.stop();
        self.inner.stop.store(false, Ordering::SeqCst);
        let inner = self.inner.clone();
        let tx = self.tx.clone();
        let _ = tx.send(EngineEvent::Listening(true));
        let _ = tx.send(EngineEvent::Status(
            "Listening… Turn on Live Captions and play audio.".into(),
        ));

        let folder = self.folder();
        crate::debuglog::set_folder(&folder);
        crate::debuglog::log("engine start");
        let handle = thread::spawn(move || {
            run_loop(inner, tx);
        });
        if let Ok(mut g) = self.handle.lock() {
            *g = Some(handle);
        }
    }

    pub fn stop(&self) {
        self.inner.stop.store(true, Ordering::SeqCst);
        if let Ok(mut g) = self.handle.lock() {
            if let Some(h) = g.take() {
                let _ = h.join();
            }
        }
        let _ = self.tx.send(EngineEvent::Listening(false));
        let _ = self.tx.send(EngineEvent::Status("Stopped.".into()));
        let _ = self.tx.send(EngineEvent::Live(String::new()));
        let _ = self.tx.send(EngineEvent::SessionFile(None));
    }
}

fn source_label() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "macOS Live Captions"
    }
    #[cfg(windows)]
    {
        "Windows Live Captions"
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        "Live Captions"
    }
}

fn run_loop(inner: Arc<EngineInner>, tx: Sender<EngineEvent>) {
    #[cfg(target_os = "macos")]
    {
        if !crate::platform::macos::is_accessibility_trusted() {
            let _ = crate::platform::macos::request_accessibility_prompt();
            if !crate::platform::macos::is_accessibility_trusted() {
                let _ = tx.send(EngineEvent::Error(
                    "Accessibility is OFF. Open System Settings → Privacy & Security → Accessibility \
                     and enable the app that opened Interpres (or Terminal). Then press Start again."
                        .into(),
                ));
                crate::platform::macos::open_accessibility_settings();
            }
        }
    }

    let cfg = Config::load();
    let mut life = Lifecycle::new(cfg.off_delay_ms);
    let mut buffer = CaptionBuffer::new();
    buffer.stable_needed = 2;
    let mut writer: Option<TranscriptWriter> = None;
    let mut session_open = false;
    let mut err_ticks: u64 = 0;
    let mut last_surface = String::new();
    let mut stale_ticks: u64 = 0;
    let poll = cfg.poll_ms.max(250);

    while !inner.stop.load(Ordering::SeqCst) {
        let snap = platform::poll_capture();
        let action = life.tick(snap.process_running, poll);

        match action {
            LifecycleAction::Open => {
                let _ = tx.send(EngineEvent::Status(format!(
                    "Live Captions detected — {}",
                    snap.detail
                )));
                if let Some(ref e) = snap.error {
                    let _ = tx.send(EngineEvent::Error(e.clone()));
                }
                if !session_open {
                    let folder = inner
                        .folder
                        .lock()
                        .map(|g| g.clone())
                        .unwrap_or_else(|_| cfg.transcript_folder.clone());
                    let remember = inner.remember.load(Ordering::SeqCst);
                    // Keep UI folder label in sync whenever a session starts.
                    let _ = tx.send(EngineEvent::Status(format!(
                        "Folder: {} · Save: {}",
                        folder.display(),
                        if remember { "ON" } else { "OFF" }
                    )));
                    match TranscriptWriter::begin_session(
                        &folder,
                        remember,
                        cfg.write_jsonl,
                        source_label(),
                        SystemTime::now(),
                    ) {
                        Ok(w) => {
                            if let Some(ref wr) = w {
                                crate::debuglog::set_session_stem(Some(wr.stem().to_string()));
                                crate::debuglog::log(&format!(
                                    "session file {}",
                                    wr.txt_path().display()
                                ));
                                let _ = tx.send(EngineEvent::SessionFile(Some(
                                    wr.txt_path().to_path_buf(),
                                )));
                                let _ = tx.send(EngineEvent::Status(session_open_status(
                                    true,
                                    Some(wr.txt_path()),
                                )));
                            } else if !remember {
                                crate::debuglog::set_session_stem(None);
                                let _ = tx.send(EngineEvent::SessionFile(None));
                                let _ = tx.send(EngineEvent::Status(session_open_status(
                                    false, None,
                                )));
                            }
                            writer = w;
                            session_open = true;
                            buffer.reset();
                            last_surface.clear();
                            stale_ticks = 0;
                        }
                        Err(e) => {
                            let _ = tx.send(EngineEvent::Error(format!(
                                "Could not create session file: {e}"
                            )));
                        }
                    }
                }
            }
            LifecycleAction::Close => {
                let _ = tx.send(EngineEvent::Status(
                    "Live Captions stopped — waiting…".into(),
                ));
                flush_buffer(&mut buffer, &mut writer, &tx);
                if let Some(ref mut w) = writer {
                    let _ = w.end_session("lc_stopped");
                }
                writer = None;
                session_open = false;
                crate::debuglog::set_session_stem(None);
                let _ = tx.send(EngineEvent::SessionFile(None));
                let _ = tx.send(EngineEvent::Live(String::new()));
            }
            LifecycleAction::None => {}
        }

        if life.companion_active {
            if let Some(ref err) = snap.error {
                err_ticks += 1;
                if err_ticks == 1 || err_ticks % 15 == 0 {
                    let _ = tx.send(EngineEvent::Error(err.clone()));
                }
            } else {
                err_ticks = 0;
            }
            if let Some(ref surface) = snap.surface_text {
                // Detect AX surface stuck on the same string (common after FINAL or on chrome).
                if surface == &last_surface {
                    stale_ticks = stale_ticks.saturating_add(1);
                } else {
                    stale_ticks = 0;
                    last_surface = surface.clone();
                }
                crate::debuglog::log(&format!(
                    "surface_chars={} stale={} preview={:?}",
                    surface.chars().count(),
                    stale_ticks,
                    surface.chars().take(80).collect::<String>()
                ));

                if should_show_lag_tip(stale_ticks) {
                    let _ = tx.send(EngineEvent::Status(LAG_TIP.into()));
                }

                // If AX is stuck on a line we already finalized, skip re-processing.
                // Still re-check every 5th stale tick in case the string is growing in place
                // with the same prefix (rare) or we missed a commit.
                let skip_stale =
                    should_skip_stale_surface(stale_ticks, buffer.is_covered(surface));

                if !skip_stale {
                    match buffer.observe(surface) {
                        BufferEmit::Partial(t) => {
                            crate::debuglog::log(&format!("PARTIAL {t}"));
                            let _ = tx.send(EngineEvent::Live(t));
                        }
                        BufferEmit::Final(t) => {
                            crate::debuglog::log(&format!("FINAL {t}"));
                            let _ = tx.send(EngineEvent::Live(t.clone()));
                            let _ = tx.send(EngineEvent::Final(t.clone()));
                            if let Some(ref mut w) = writer {
                                let _ = w.write_final(&format_clock(SystemTime::now()), &t);
                            }
                            stale_ticks = 0;
                        }
                        BufferEmit::Finals(v) => {
                            for t in v {
                                crate::debuglog::log(&format!("FINAL {t}"));
                                let _ = tx.send(EngineEvent::Live(t.clone()));
                                let _ = tx.send(EngineEvent::Final(t.clone()));
                                if let Some(ref mut w) = writer {
                                    let _ = w.write_final(&format_clock(SystemTime::now()), &t);
                                }
                            }
                            stale_ticks = 0;
                        }
                        BufferEmit::None => {}
                    }
                }
            } else {
                // No surface (junk filtered out) — clear live after a short while.
                stale_ticks = stale_ticks.saturating_add(1);
                if should_clear_live_after_empty(stale_ticks) {
                    crate::debuglog::log("no caption surface (junk filtered or empty AX)");
                    let _ = tx.send(EngineEvent::Live(String::new()));
                }
                if should_show_lag_tip(stale_ticks) {
                    let _ = tx.send(EngineEvent::Status(format!(
                        "{LAG_TIP} (no caption surface — junk filtered or empty AX)"
                    )));
                }
            }
        }

        thread::sleep(Duration::from_millis(poll));
    }

    flush_buffer(&mut buffer, &mut writer, &tx);
    if let Some(ref mut w) = writer {
        let _ = w.end_session("user");
    }
}

fn flush_buffer(
    buffer: &mut CaptionBuffer,
    writer: &mut Option<TranscriptWriter>,
    tx: &Sender<EngineEvent>,
) {
    match buffer.finish() {
        BufferEmit::Final(t) => {
            let _ = tx.send(EngineEvent::Final(t.clone()));
            if let Some(w) = writer.as_mut() {
                let _ = w.write_final(&format_clock(SystemTime::now()), &t);
            }
        }
        BufferEmit::Finals(v) => {
            for t in v {
                let _ = tx.send(EngineEvent::Final(t.clone()));
                if let Some(w) = writer.as_mut() {
                    let _ = w.write_final(&format_clock(SystemTime::now()), &t);
                }
            }
        }
        _ => {}
    }
}
