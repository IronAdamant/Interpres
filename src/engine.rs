//! Background Live Captions capture engine (pure std). Used by GUI and CLI.

use crate::buffer::{live_edge_phrase, BufferEmit, CaptionBuffer, ShortLineHold};
use crate::config::Config;
use crate::health::{Health, HealthMonitor};
use crate::lifecycle::{Lifecycle, LifecycleAction};
use crate::platform;
use crate::plugin_host::PluginHost;
use crate::protocol::{CaptionEvent, LcState};
use crate::transcript::{format_clock, TranscriptWriter};
use crate::ui_labels::{
    session_open_status, CaptureErrorHysteresis, LiveSurfaceTracker, LAG_TIP,
};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime};

/// Live Captions back within this window → keep writing the same transcript file
/// (a meeting should not split because LC restarted or blipped).
pub const RESUME_SAME_FILE_WITHIN: Duration = Duration::from_secs(15 * 60);
/// After Stop, keep reading this long so the sentence being spoken still lands.
const STOP_DRAIN_MAX: Duration = Duration::from_millis(2500);
/// …but finish early once the caption text has not changed for this long.
const STOP_DRAIN_SETTLED: Duration = Duration::from_millis(900);
/// Wait this long before restarting an external engine that failed or exited.
const ENGINE_RESTART_BACKOFF: Duration = Duration::from_secs(5);
/// Re-inject short lines a reader dropped. macOS reads every caption line in screen
/// order, so there is nothing to re-inject (re-injecting scrambled the line order).
const USE_SHORT_HOLD: bool = !cfg!(target_os = "macos");
/// Log polls slower than this (field failure: polls stretched to ~27 s unnoticed).
const SLOW_POLL_LOG: Duration = Duration::from_millis(2000);

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
    /// Capture health changed (banner state). See `crate::health`.
    Health(Health),
}

struct EngineInner {
    stop: AtomicBool,
    /// Written to the transcript as `# Session ended (<reason>)`.
    stop_reason: Mutex<String>,
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
            stop_reason: Mutex::new("user".into()),
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
        if let Ok(mut r) = self.inner.stop_reason.lock() {
            *r = "user".into();
        }
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

    /// Stop and wait for the capture thread (including the short end-of-session drain).
    pub fn stop(&self) {
        self.inner.stop.store(true, Ordering::SeqCst);
        let joined = self
            .handle
            .lock()
            .ok()
            .and_then(|mut g| g.take())
            .map(|h| h.join())
            .is_some();
        if !joined {
            // No thread: still tell the UI it is idle (thread sends these itself on exit).
            send_stopped(&self.tx);
        }
    }

    /// Ask the capture thread to finish without blocking the caller (UI thread).
    /// The thread drains the last caption, saves, then sends `Listening(false)`.
    pub fn request_stop(&self) {
        self.inner.stop.store(true, Ordering::SeqCst);
    }

    /// `request_stop`, recording why in the transcript (e.g. "no sound for 5 min").
    pub fn request_stop_because(&self, reason: &str) {
        if let Ok(mut r) = self.inner.stop_reason.lock() {
            *r = reason.to_string();
        }
        self.request_stop();
    }
}

impl EngineInner {
    fn stop_reason(&self) -> String {
        self.stop_reason
            .lock()
            .map(|r| r.clone())
            .unwrap_or_else(|_| "user".into())
    }
}

fn send_stopped(tx: &Sender<EngineEvent>) {
    let _ = tx.send(EngineEvent::Listening(false));
    let _ = tx.send(EngineEvent::Status("Stopped.".into()));
    let _ = tx.send(EngineEvent::Live(String::new()));
    let _ = tx.send(EngineEvent::SessionFile(None));
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
    let cfg0 = Config::load();
    if cfg0.uses_external_engine() {
        run_external_engine(&inner, &tx, &cfg0);
        return;
    }
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
    let mut health = HealthMonitor::new();
    // Hard errors repeat every poll while showing; log each distinct one once.
    let mut last_logged_error: Option<String> = None;
    // When LC went away mid-session (writer kept open for RESUME_SAME_FILE_WITHIN).
    let mut lc_gone_at: Option<Instant> = None;
    // Floor 100ms so config can go lower; default is 150 (short-line fidelity).
    let poll = cfg.poll_ms.max(100);
    let loop_start = Instant::now();
    let mut last_tick = Instant::now();
    // Caption text already on screen at Start belongs to an earlier meeting: the first
    // surface of the recording is taken as "seen" (see CaptionBuffer::prime).
    let mut polls: u64 = 0;
    let mut primed = false;

    while !inner.stop.load(Ordering::SeqCst) {
        polls += 1;
        let poll_started = Instant::now();
        let snap = platform::poll_capture();
        let poll_took = poll_started.elapsed();
        if poll_took > SLOW_POLL_LOG {
            crate::debuglog::log(&format!("slow poll: {}ms", poll_took.as_millis()));
        }
        // Real elapsed time, not the nominal poll interval: slow polls must not
        // stretch the LC-off debounce into minutes.
        let elapsed_ms = last_tick.elapsed().as_millis() as u64;
        last_tick = Instant::now();
        let action = life.tick(snap.process_running, elapsed_ms);

        let has_text = snap
            .surface_text
            .as_ref()
            .is_some_and(|s| !s.trim().is_empty());
        let now_ms = loop_start.elapsed().as_millis() as u64;
        if let Some(h) = health.on_poll(now_ms, snap.process_running, has_text, snap.error.as_deref()) {
            crate::debuglog::log(&format!(
                "health {} (error={:?})",
                h.as_str(),
                snap.error.as_deref().unwrap_or("")
            ));
            let _ = tx.send(EngineEvent::Health(h));
        }

        match action {
            LifecycleAction::Open => {
                let _ = tx.send(EngineEvent::Status(format!(
                    "Live Captions detected — {}",
                    snap.detail
                )));
                // Do not pin scrape errors on Open — hysteresis handles sticky UI errors.
                if !session_open {
                    // LC came back: continue the same file if it was a short gap.
                    if let Some(gone) = lc_gone_at.take() {
                        if writer.is_some() && gone.elapsed() <= RESUME_SAME_FILE_WITHIN {
                            if let Some(ref mut w) = writer {
                                let _ = w.write_note(
                                    &format_clock(SystemTime::now()),
                                    "Live Captions back on",
                                );
                            }
                            crate::debuglog::log("Live Captions back — continuing same session file");
                            session_open = true;
                            buffer.reset();
                            surface_tr.reset();
                            short_hold.clear();
                            last_live_edge.clear();
                            if let Some(ref wr) = writer {
                                let _ = tx.send(EngineEvent::SessionFile(Some(
                                    wr.txt_path().to_path_buf(),
                                )));
                            }
                        } else if let Some(ref mut w) = writer {
                            let _ = w.end_session("lc_stopped");
                            writer = None;
                        }
                    }
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
                // Keep the file open: if LC returns soon, the meeting stays in one file.
                if let Some(ref mut w) = writer {
                    let _ = w.write_note(
                        &format_clock(SystemTime::now()),
                        "Live Captions turned off — nothing captured until it is back on",
                    );
                }
                crate::debuglog::log("Live Captions gone — session paused");
                lc_gone_at = Some(Instant::now());
                session_open = false;
                err_hyst = CaptureErrorHysteresis::new();
                short_hold.clear();
                last_live_edge.clear();
                let _ = tx.send(EngineEvent::Live(String::new()));
            }
            LifecycleAction::None => {}
        }

        if life.companion_active {
            let surface_ok = snap.surface_text.as_ref().is_some_and(|s| !s.trim().is_empty());
            let err_tick = err_hyst.on_poll(surface_ok, snap.error.as_deref());
            if err_tick.clear_error {
                last_logged_error = None;
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
                    if last_logged_error.as_deref() != Some(msg.as_str()) {
                        crate::debuglog::log(&format!("capture error: {msg}"));
                        last_logged_error = Some(msg.clone());
                    }
                    let _ = tx.send(EngineEvent::Error(msg.clone()));
                }
            }

            // Short-line hold from picked surface (merge pick already joins multi-line siblings).
            let hold_inputs: Vec<String> = if USE_SHORT_HOLD {
                snap.surface_text.iter().cloned().collect()
            } else {
                Vec::new()
            };
            let _held = short_hold.on_poll(&hold_inputs, |t| buffer.is_covered(t));

            if let Some(ref raw_surface) = snap.surface_text {
                let surface = if USE_SHORT_HOLD {
                    short_hold.inject_into_surface(raw_surface)
                } else {
                    raw_surface.clone()
                };
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

                if tick.process_surface && !primed {
                    primed = true;
                    // On screen when Start was pressed: skip it all. Window appeared
                    // later (Mac re-shows old lines with the new one): keep the newest.
                    let keep_last = polls > 1;
                    buffer.prime(&surface, keep_last);
                    crate::debuglog::log(&format!(
                        "start: {} on-screen line(s) taken as already seen (keep_last={keep_last})",
                        surface.lines().filter(|l| !l.trim().is_empty()).count()
                    ));
                } else if tick.process_surface {
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

    if session_open {
        drain_after_stop(&mut buffer, &mut writer, &tx);
    }
    flush_buffer(&mut buffer, &mut writer, &tx);
    if let Some(ref mut w) = writer {
        let _ = w.end_session(&inner.stop_reason());
    }
    crate::debuglog::set_session_stem(None);
    platform::shutdown_capture();
    send_stopped(&tx);
}

/// Report health changes once (UI banner + debug log).
struct HealthReporter<'a> {
    tx: &'a Sender<EngineEvent>,
    current: Option<Health>,
}

impl HealthReporter<'_> {
    fn set(&mut self, h: Health) {
        if self.current != Some(h) {
            self.current = Some(h);
            crate::debuglog::log(&format!("health {}", h.as_str()));
            let _ = self.tx.send(EngineEvent::Health(h));
        }
    }
}

/// Bring-your-own speech engine (docs/ENGINES.md): run `helper_path helper_args`, read
/// protocol lines from its stdout, save FINAL lines. The engine captures audio itself.
fn run_external_engine(inner: &EngineInner, tx: &Sender<EngineEvent>, cfg: &Config) {
    let name = cfg.engine_name();
    let Some(program) = cfg.helper_path.clone() else {
        return;
    };
    let args = crate::config::split_args(&cfg.helper_args);
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let folder = inner
        .folder
        .lock()
        .map(|g| g.clone())
        .unwrap_or_else(|_| cfg.transcript_folder.clone());
    let remember = inner.remember.load(Ordering::SeqCst);

    let mut writer = match TranscriptWriter::begin_session(
        &folder,
        remember,
        cfg.write_jsonl,
        &format!("External engine ({name})"),
        SystemTime::now(),
    ) {
        Ok(w) => w,
        Err(e) => {
            let _ = tx.send(EngineEvent::Error(format!("Could not create session file: {e}")));
            None
        }
    };
    if let Some(ref wr) = writer {
        crate::debuglog::set_session_stem(Some(wr.stem().to_string()));
        crate::debuglog::log(&format!("session file {}", wr.txt_path().display()));
        let _ = tx.send(EngineEvent::SessionFile(Some(wr.txt_path().to_path_buf())));
    }
    let _ = tx.send(EngineEvent::Status(format!("Captions from external engine: {name}")));

    let mut health = HealthReporter { tx, current: None };
    let mut host: Option<PluginHost> = None;
    let mut next_start = Instant::now();
    let mut last_live = String::new();

    while !inner.stop.load(Ordering::SeqCst) {
        if host.is_none() && Instant::now() >= next_start {
            match PluginHost::start(&program, &arg_refs) {
                Ok(h) => {
                    crate::debuglog::log(&format!(
                        "engine started: {} {}",
                        program.display(),
                        cfg.helper_args
                    ));
                    host = Some(h);
                }
                Err(e) => {
                    crate::debuglog::log(&format!("engine failed to start: {e}"));
                    let _ = tx.send(EngineEvent::Error(format!(
                        "Could not start {name} ({}): {e}",
                        program.display()
                    )));
                    health.set(Health::EngineStopped);
                    next_start = Instant::now() + ENGINE_RESTART_BACKOFF;
                }
            }
        }

        if let Some(h) = host.as_mut() {
            while let Some(ev) = h.try_recv() {
                handle_engine_event(ev, &mut writer, tx, &mut health, &mut last_live);
            }
            if let Some(status) = h.exit_status() {
                for ev in h.shutdown_collect() {
                    handle_engine_event(ev, &mut writer, tx, &mut health, &mut last_live);
                }
                crate::debuglog::log(&format!("engine exited ({status}) — restarting"));
                if let Some(ref mut w) = writer {
                    let _ = w.write_note(
                        &format_clock(SystemTime::now()),
                        &format!("Caption engine stopped ({status}) — restarting"),
                    );
                }
                let _ = tx.send(EngineEvent::Error(format!(
                    "{name} stopped ({status}). Restarting in {} s…",
                    ENGINE_RESTART_BACKOFF.as_secs()
                )));
                health.set(Health::EngineStopped);
                host = None;
                next_start = Instant::now() + ENGINE_RESTART_BACKOFF;
            }
        }
        thread::sleep(Duration::from_millis(50));
    }

    if let Some(mut h) = host.take() {
        let _ = tx.send(EngineEvent::Status("Saving the last words…".into()));
        for ev in h.shutdown_collect() {
            handle_engine_event(ev, &mut writer, tx, &mut health, &mut last_live);
        }
        crate::debuglog::log("engine stopped by user");
    }
    if let Some(ref mut w) = writer {
        let _ = w.end_session(&inner.stop_reason());
    }
    crate::debuglog::set_session_stem(None);
    send_stopped(tx);
}

fn handle_engine_event(
    ev: CaptionEvent,
    writer: &mut Option<TranscriptWriter>,
    tx: &Sender<EngineEvent>,
    health: &mut HealthReporter,
    last_live: &mut String,
) {
    match ev {
        CaptionEvent::Ready => {
            crate::debuglog::log("engine READY");
            health.set(Health::WaitingForSpeech);
        }
        CaptionEvent::Partial { text } => {
            let text = text.trim().to_string();
            if !text.is_empty() && text != *last_live {
                health.set(Health::Recording);
                *last_live = text.clone();
                let _ = tx.send(EngineEvent::Live(text));
            }
        }
        CaptionEvent::Final { text } => {
            let text = text.trim();
            if text.is_empty() {
                return;
            }
            health.set(Health::Recording);
            crate::debuglog::log(&format!("FINAL {text}"));
            last_live.clear();
            let _ = tx.send(EngineEvent::Live(String::new()));
            let _ = tx.send(EngineEvent::Final(text.to_string()));
            if let Some(w) = writer.as_mut() {
                if let Err(e) = w.append_final(&format_clock(SystemTime::now()), text) {
                    crate::debuglog::log(&format!("write_final error: {e}"));
                }
            }
        }
        CaptionEvent::Status { lc, reason } => {
            crate::debuglog::log(&format!("engine STATUS {} {reason}", lc.as_str()));
            match lc {
                LcState::Stopped | LcState::Degraded => health.set(Health::EngineStopped),
                LcState::Running if health.current.is_none() => {
                    health.set(Health::WaitingForSpeech)
                }
                _ => {}
            }
        }
        CaptionEvent::Error { message } => {
            crate::debuglog::log(&format!("engine ERROR {message}"));
            let _ = tx.send(EngineEvent::Error(message));
        }
        CaptionEvent::Log { level, message } => {
            crate::debuglog::log(&format!("engine {level}: {message}"));
        }
        CaptionEvent::Shutdown | CaptionEvent::Unknown(_) => {}
    }
}

/// Keep reading briefly after Stop so a sentence still being captioned is saved whole.
fn drain_after_stop(
    buffer: &mut CaptionBuffer,
    writer: &mut Option<TranscriptWriter>,
    tx: &Sender<EngineEvent>,
) {
    let _ = tx.send(EngineEvent::Status("Saving the last words…".into()));
    let started = Instant::now();
    let mut last_change = Instant::now();
    let mut last_surface = String::new();
    let mut live = String::new();
    let mut tracker = LiveSurfaceTracker::new();
    while started.elapsed() < STOP_DRAIN_MAX {
        let snap = platform::poll_capture();
        let Some(surface) = snap.surface_text.filter(|s| !s.trim().is_empty()) else {
            break;
        };
        if surface != last_surface {
            last_change = Instant::now();
            last_surface = surface.clone();
            let emit = buffer.observe(&surface);
            apply_buffer_emit(emit, writer, tx, &mut live, &mut tracker);
        } else if last_change.elapsed() >= STOP_DRAIN_SETTLED {
            break;
        }
        thread::sleep(Duration::from_millis(150));
    }
    crate::debuglog::log(&format!(
        "stop drain {}ms",
        started.elapsed().as_millis()
    ));
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
    let collapsed = crate::buffer::collapse_repeats(t);
    let t = collapsed.as_str();
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
    let collapsed = crate::buffer::collapse_repeats(t);
    let t = collapsed.as_str();
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
