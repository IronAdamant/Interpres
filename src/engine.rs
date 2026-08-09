//! Background Live Captions capture engine (pure std). Used by GUI and CLI.

use crate::buffer::{live_edge_phrase, BufferEmit, CaptionBuffer, ShortLineHold};
use crate::config::Config;
use crate::lifecycle::{Lifecycle, LifecycleAction};
use crate::platform;
use crate::transcript::{format_clock, TranscriptWriter};
use crate::ui_labels::{
    session_open_status, CaptureErrorHysteresis, LiveSurfaceTracker, LAG_TIP,
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
    /// Polished rewrite of the last committed caption family (replace in UI/history).
    Revised(String),
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
        // Order: Listening(true) first so UI clears Session/Live before new finals arrive.
        let _ = tx.send(EngineEvent::Listening(true));
        let _ = tx.send(EngineEvent::Live(String::new()));
        let _ = tx.send(EngineEvent::SessionFile(None));
        let _ = tx.send(EngineEvent::Status(
            crate::ui_labels::listening_status().into(),
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
    let mut err_hyst = CaptureErrorHysteresis::new();
    let mut surface_tr = LiveSurfaceTracker::new();
    let mut short_hold = ShortLineHold::new();
    let mut last_live_edge = String::new();
    // Floor 100ms so config can go lower; default is 150 (short-line fidelity).
    let poll = cfg.poll_ms.max(100);

    while !inner.stop.load(Ordering::SeqCst) {
        let snap = platform::poll_capture();
        let action = life.tick(snap.process_running, poll);

        match action {
            LifecycleAction::Open => {
                let _ = tx.send(EngineEvent::Status(format!(
                    "Live Captions detected — {}",
                    snap.detail
                )));
                // Do not pin scrape errors on Open — hysteresis handles sticky UI errors.
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
                            surface_tr.reset();
                            short_hold.clear();
                            last_live_edge.clear();
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
                err_hyst = CaptureErrorHysteresis::new();
                short_hold.clear();
                last_live_edge.clear();
                crate::debuglog::set_session_stem(None);
                let _ = tx.send(EngineEvent::SessionFile(None));
                let _ = tx.send(EngineEvent::Live(String::new()));
            }
            LifecycleAction::None => {}
        }

        if life.companion_active {
            let surface_ok = snap.surface_text.as_ref().is_some_and(|s| !s.trim().is_empty());
            let err_tick = err_hyst.on_poll(surface_ok, snap.error.as_deref());
            if err_tick.clear_error {
                // Restore non-error status after a good surface (do not leave UIA error pinned).
                if let Some(ref wr) = writer {
                    let _ = tx.send(EngineEvent::Status(session_open_status(
                        true,
                        Some(wr.txt_path()),
                    )));
                } else {
                    let _ = tx.send(EngineEvent::Status(
                        crate::ui_labels::listening_status().into(),
                    ));
                }
            } else if err_tick.show_hard_error {
                if let Some(ref msg) = err_tick.message {
                    let _ = tx.send(EngineEvent::Error(msg.clone()));
                }
            }

            // Short-line hold from picked surface (merge pick already joins multi-line siblings).
            let hold_inputs: Vec<String> = snap
                .surface_text
                .iter()
                .cloned()
                .collect();
            let _held = short_hold.on_poll(&hold_inputs, |t| buffer.is_covered(t));

            if let Some(ref raw_surface) = snap.surface_text {
                let surface = short_hold.inject_into_surface(raw_surface);
                let tick =
                    surface_tr.on_surface(&surface, buffer.is_covered(&surface));

                if tick.show_lag_tip {
                    let _ = tx.send(EngineEvent::Status(LAG_TIP.into()));
                }

                // Always refresh Live from surface edge (even when skip_stale).
                let edge = live_edge_phrase(&surface);
                if !edge.is_empty() && edge != last_live_edge {
                    last_live_edge = edge.clone();
                    let _ = tx.send(EngineEvent::Live(edge));
                }

                // Throttle identical stale surface debug spam (AFK / sticky LC).
                let log_surface = !tick.skip_stale || surface_tr.stale_ticks % 50 == 1;
                if log_surface {
                    crate::debuglog::log(&format!(
                        "surface_chars={} stale={} empty={} skip={} preview={:?}",
                        surface.chars().count(),
                        surface_tr.stale_ticks,
                        surface_tr.empty_ticks,
                        tick.skip_stale,
                        surface.chars().take(80).collect::<String>()
                    ));
                }

                if tick.process_surface {
                    let emit = buffer.observe(&surface);
                    apply_buffer_emit(emit, &mut writer, &tx, &mut last_live_edge, &mut surface_tr);
                }
            } else {
                // Decay holds even without surface; try empty leave-window via held inject.
                if !_held.is_empty() {
                    let synthetic = short_hold.inject_into_surface("");
                    if !synthetic.trim().is_empty() {
                        let emit = buffer.observe(&synthetic);
                        apply_buffer_emit(
                            emit,
                            &mut writer,
                            &tx,
                            &mut last_live_edge,
                            &mut surface_tr,
                        );
                    }
                }
                // No surface (junk filtered out) — dedicated empty_ticks, not shared stale.
                let tick = surface_tr.on_empty();
                if tick.clear_live {
                    crate::debuglog::log("no caption surface (junk filtered or empty AX) — clear live");
                    last_live_edge.clear();
                    let _ = tx.send(EngineEvent::Live(String::new()));
                }
                if tick.show_lag_tip {
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
    let mut last_live = String::new();
    let mut surface_tr = LiveSurfaceTracker::new();
    let emit = buffer.finish();
    // finish Partial becomes Final at end of session.
    let emit = match emit {
        BufferEmit::Partial(t) => BufferEmit::Final(t),
        other => other,
    };
    apply_buffer_emit(emit, writer, tx, &mut last_live, &mut surface_tr);
}

/// Map buffer emissions to UI events + disk (Revised → write_revised).
fn apply_buffer_emit(
    emit: BufferEmit,
    writer: &mut Option<TranscriptWriter>,
    tx: &Sender<EngineEvent>,
    last_live_edge: &mut String,
    surface_tr: &mut LiveSurfaceTracker,
) {
    match emit {
        BufferEmit::None => {}
        BufferEmit::Partial(t) => {
            let edge = live_edge_phrase(&t);
            let edge = if edge.is_empty() { t } else { edge };
            crate::debuglog::log(&format!("PARTIAL {edge}"));
            if edge != *last_live_edge {
                *last_live_edge = edge.clone();
                let _ = tx.send(EngineEvent::Live(edge));
            }
        }
        BufferEmit::Final(t) => {
            emit_final_line(&t, writer, tx, last_live_edge);
            surface_tr.note_final();
        }
        BufferEmit::Revised(t) => {
            emit_revised_line(&t, writer, tx, last_live_edge);
            surface_tr.note_final();
        }
        BufferEmit::Finals(v) => {
            for t in v {
                emit_final_line(&t, writer, tx, last_live_edge);
            }
            surface_tr.note_final();
        }
        BufferEmit::Batch { revised, finals } => {
            for t in revised {
                emit_revised_line(&t, writer, tx, last_live_edge);
            }
            for t in finals {
                emit_final_line(&t, writer, tx, last_live_edge);
            }
            surface_tr.note_final();
        }
    }
}

fn emit_final_line(
    t: &str,
    writer: &mut Option<TranscriptWriter>,
    tx: &Sender<EngineEvent>,
    last_live_edge: &mut String,
) {
    crate::debuglog::log(&format!("FINAL {t}"));
    let edge = live_edge_phrase(t);
    let edge = if edge.is_empty() {
        t.to_string()
    } else {
        edge
    };
    *last_live_edge = edge.clone();
    let _ = tx.send(EngineEvent::Live(edge));
    let _ = tx.send(EngineEvent::Final(t.to_string()));
    if let Some(w) = writer.as_mut() {
        if let Err(e) = w.write_final(&format_clock(SystemTime::now()), t) {
            crate::debuglog::log(&format!("write_final error: {e}"));
        }
    }
}

fn emit_revised_line(
    t: &str,
    writer: &mut Option<TranscriptWriter>,
    tx: &Sender<EngineEvent>,
    last_live_edge: &mut String,
) {
    crate::debuglog::log(&format!("REVISED {t}"));
    let edge = live_edge_phrase(t);
    let edge = if edge.is_empty() {
        t.to_string()
    } else {
        edge
    };
    *last_live_edge = edge.clone();
    let _ = tx.send(EngineEvent::Live(edge));
    let _ = tx.send(EngineEvent::Revised(t.to_string()));
    if let Some(w) = writer.as_mut() {
        if let Err(e) = w.write_revised(&format_clock(SystemTime::now()), t) {
            crate::debuglog::log(&format!("write_revised error: {e}"));
        }
    }
}
