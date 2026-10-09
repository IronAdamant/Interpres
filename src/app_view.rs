//! What the app window shows and does, shared by the Windows (`gui_win.rs`) and macOS
//! (`gui.rs` + `native/macos/`) windows. Pure state and decisions — no drawing.
//!
//! Each OS window owns an `AppModel`, calls `pump()` on a ~50 ms timer, and draws
//! `banner()`, `checklist()`, `footer()` and `transcript_rows()` with its own controls.

use crate::auto_record::{AutoAction, AutoRecord, SoundDetector};
use crate::buffer::same_or_refinement;
use crate::config::Config;
use crate::engine::{CaptureEngine, EngineEvent};
use crate::health::{Health, IdlePrompt};
use crate::history_ui::{apply_plan, plan_family, FamilyPlan};
use crate::platform;
use crate::platform::sound::{spawn_sound_meter, SoundLevel};
use crate::theme::ThemeMode;
use crate::transcript::format_clock;
use crate::ui_labels::LAG_TIP;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime};

/// Re-render the transcript at most this often (engine emits every poll).
pub const TRANSCRIPT_MIN_INTERVAL: Duration = Duration::from_millis(200);
/// While stopped, re-check whether Live Captions is on this often.
const IDLE_LC_CHECK: Duration = Duration::from_secs(2);
/// Speaker level older than this is treated as unknown (never as silence).
const SOUND_STALE: Duration = Duration::from_secs(3);
/// Auto-record turned Live Captions on: wait this long for it before recording anyway.
const AUTO_LC_WAIT: Duration = Duration::from_secs(15);
/// Windows can switch Live Captions on for the user; macOS only lets the user do it.
const CAN_LAUNCH_LC: bool = cfg!(windows);

/// Line break used inside the transcript view.
#[cfg(windows)]
const EOL: &str = "\r\n";
#[cfg(not(windows))]
const EOL: &str = "\n";

/// Name of the OS caption feature, for menus and messages.
#[cfg(windows)]
pub const LC_NAME: &str = "Windows Live Captions";
#[cfg(not(windows))]
pub const LC_NAME: &str = "Mac Live Captions";

/// What the contextual second button does right now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LcAction {
    None,
    TurnOn,
    Restart,
    /// Answer to "are you done?" — snooze the prompt.
    KeepRecording,
}

/// Banner colour role; each window maps it to its own palette.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Recording,
    Waiting,
    Problem,
    Action,
    /// Panel colour with normal text (nothing urgent).
    Neutral,
}

impl Tone {
    pub fn as_int(self) -> i32 {
        match self {
            Tone::Neutral => 0,
            Tone::Recording => 1,
            Tone::Waiting => 2,
            Tone::Problem => 3,
            Tone::Action => 4,
        }
    }
}

/// Status banner: headline, one line of guidance, colour role.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Banner {
    pub head: String,
    pub guidance: String,
    pub tone: Tone,
}

impl Banner {
    fn new(head: impl Into<String>, guidance: impl Into<String>, tone: Tone) -> Self {
        Self {
            head: head.into(),
            guidance: guidance.into(),
            tone,
        }
    }
}

/// Attention the window should ask for after a pump.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Attention {
    /// Something broke while recording (flash + warning sound).
    pub alert: bool,
    /// The "are you done?" prompt just appeared (gentle flash + sound).
    pub nudge: bool,
}

/// Everything the window shows, derived from engine events.
#[derive(Default)]
pub struct View {
    pub listening: bool,
    pub health: Option<Health>,
    /// While stopped: is Live Captions running (polled every `IDLE_LC_CHECK`).
    pub idle_lc_on: Option<bool>,
    pub last_idle_check: Option<Instant>,
    pub started_at: Option<Instant>,
    pub lines: Vec<String>,
    pub times: Vec<String>,
    pub live: String,
    pub session_path: Option<PathBuf>,
    pub session_active: bool,
    pub detail: String,
    pub transcript_dirty: bool,
    pub last_render: Option<Instant>,
    pub rendered_banner: String,
    /// Rows currently in the transcript control and their UTF-16 lengths, so updates
    /// only replace the changed tail instead of the whole text.
    pub rendered_rows: Vec<String>,
    pub rendered_u16: Vec<usize>,
    /// Stop pressed; engine is saving the last words.
    pub stopping: bool,
    pub idle: IdlePrompt,
}

pub struct AppModel {
    pub engine: CaptureEngine,
    pub rx: Receiver<EngineEvent>,
    pub view: View,
    pub remember: bool,
    pub debug: bool,
    pub theme_mode: ThemeMode,
    pub idle_prompt_ms: u64,
    /// Captions come from an external speech engine instead of Live Captions.
    pub engine_mode: bool,
    /// Display name of the external engine (from settings).
    pub engine_name: String,
    /// `helper_path` is set, so the external engine can be chosen.
    pub engine_configured: bool,
    /// Monotonic base for `IdlePrompt` milliseconds.
    pub epoch: Instant,
    /// Auto-record checkbox state (saved as `auto_record`).
    pub auto_on: bool,
    pub auto: AutoRecord,
    /// Speaker level reader; running only while auto-record is on.
    pub sound: Option<Arc<SoundLevel>>,
    pub detector: SoundDetector,
    /// Last speaker-meter state reported to the user (None = not yet known).
    pub sound_readable: Option<bool>,
    /// Auto-record turned Live Captions on and is waiting for it before starting.
    pub auto_start_pending: Option<Instant>,
    /// Shown once the engine confirms an automatic start/stop (those events clear the
    /// detail line).
    pub auto_note: Option<String>,
}

pub fn format_elapsed(d: Duration) -> String {
    let s = d.as_secs();
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
    } else {
        format!("{:02}:{:02}", s / 60, s % 60)
    }
}

/// Index of the first row that differs (rows after it must be re-rendered).
pub fn first_changed_row(old: &[String], new: &[String]) -> usize {
    old.iter().zip(new).take_while(|(a, b)| a == b).count()
}

/// Apply a caption the same way the transcript file does; each line keeps the time it
/// was first heard (a merge keeps the earliest).
pub fn apply_caption(v: &mut View, plan: FamilyPlan, text: &str) {
    let text = text.trim();
    if text.is_empty() || matches!(plan, FamilyPlan::NoOp) {
        return;
    }
    apply_plan(&mut v.lines, &mut v.times, plan, text, || format_clock(SystemTime::now()));
    v.transcript_dirty = true;
}

impl AppModel {
    pub fn new(cfg: &Config) -> Self {
        let (engine, rx) = CaptureEngine::new(cfg);
        let remember = engine.remember();
        Self {
            engine,
            rx,
            view: View {
                transcript_dirty: true,
                ..View::default()
            },
            remember,
            debug: cfg.debug,
            theme_mode: cfg.theme,
            idle_prompt_ms: cfg.idle_prompt_minutes.saturating_mul(60_000),
            engine_mode: cfg.uses_external_engine(),
            engine_name: cfg.engine_name(),
            engine_configured: cfg.helper_path.is_some(),
            epoch: Instant::now(),
            auto_on: cfg.auto_record,
            auto: AutoRecord::new(cfg.auto_stop_quiet_minutes.saturating_mul(60_000)),
            sound: cfg.auto_record.then(spawn_sound_meter),
            detector: SoundDetector::new(0),
            sound_readable: None,
            auto_start_pending: None,
            auto_note: None,
        }
    }

    pub fn lc_action(&self) -> LcAction {
        if self.engine_mode {
            // No Live Captions to turn on; the engine restarts itself.
            return if self.view.listening && self.view.idle.asking() && !self.view.stopping {
                LcAction::KeepRecording
            } else {
                LcAction::None
            };
        }
        if self.view.listening {
            match self.view.health {
                Some(Health::LiveCaptionsOff) => LcAction::TurnOn,
                Some(Health::NotReading) => LcAction::Restart,
                _ if self.view.idle.asking() && !self.view.stopping => LcAction::KeepRecording,
                _ => LcAction::None,
            }
        } else if self.view.idle_lc_on == Some(false) {
            LcAction::TurnOn
        } else {
            LcAction::None
        }
    }

    pub fn now_ms(&self) -> u64 {
        self.epoch.elapsed().as_millis() as u64
    }

    /// While recording with auto-record on: time left before silence stops it.
    fn auto_stop_in_ms(&self) -> Option<u64> {
        let limit = self.auto.quiet_stop_ms();
        if !self.auto_on || !self.view.listening || limit == 0 {
            return None;
        }
        Some(limit.saturating_sub(self.quiet_ms()))
    }

    /// How long nothing has happened: no speaker sound and, while recording, no new
    /// captions (Live Captions can caption your microphone while the speakers are silent;
    /// "Keep recording" also resets this).
    fn quiet_ms(&self) -> u64 {
        let now = self.now_ms();
        let sound = self.detector.quiet_ms(now);
        if self.view.listening {
            sound.min(self.view.idle.quiet_ms(now))
        } else {
            sound
        }
    }

    pub fn banner(&self) -> Banner {
        let v = &self.view;
        if v.listening && v.stopping {
            return Banner::new(
                "Saving the last words…",
                "Finishing the sentence Live Captions is showing, then closing the file.",
                Tone::Waiting,
            );
        }
        let problem = v.health.is_some_and(|h| h.is_problem());
        if v.listening && v.idle.asking() && !problem {
            let mins = (v.idle.quiet_ms(self.now_ms()) / 60_000).max(1);
            let head = if v.lines.is_empty() {
                format!("Nothing heard for {mins} min — still waiting for your meeting?")
            } else {
                format!("No new captions for {mins} min — are you done?")
            };
            let guidance = match self.auto_stop_in_ms() {
                Some(left) => format!(
                    "If it stays silent, Interpres stops and saves by itself in {} min. Or choose now.",
                    left.div_ceil(60_000).max(1)
                ),
                None => {
                    "Recording keeps going until you choose: Stop & save, or Keep recording.".into()
                }
            };
            return Banner::new(head, guidance, Tone::Action);
        }
        if !v.listening && self.auto_on {
            let guidance = if self.auto_start_pending.is_some() {
                "Sound is playing — turning on Live Captions, then recording starts.".to_string()
            } else if self.sound_readable == Some(false) {
                "Can't read the speaker level right now, so auto-record is paused.".to_string()
            } else if self.engine_mode {
                format!(
                    "Recording starts by itself when a meeting or video plays. Captions from {}.",
                    self.engine_name
                )
            } else if CAN_LAUNCH_LC {
                "Recording starts by itself when a meeting or video plays (Live Captions turns on too)."
                    .to_string()
            } else {
                "Recording starts by itself when a meeting or video plays. Keep Live Captions on."
                    .to_string()
            };
            return Banner::new("Not recording — auto-record is on", guidance, Tone::Neutral);
        }
        if !v.listening && self.engine_mode {
            return Banner::new(
                "Not recording",
                format!(
                    "Press Start recording. Captions come from your engine: {}.",
                    self.engine_name
                ),
                Tone::Neutral,
            );
        }
        if !v.listening {
            return match v.idle_lc_on {
                Some(false) => Banner::new(
                    "Not recording — Live Captions is off",
                    "Turn on Live Captions first, then press Start recording.",
                    Tone::Problem,
                ),
                _ => Banner::new(
                    "Not recording",
                    "Press Start recording before your meeting. Live Captions must stay on.",
                    Tone::Neutral,
                ),
            };
        }
        let elapsed = v
            .started_at
            .map(|t| format_elapsed(t.elapsed()))
            .unwrap_or_default();
        let lines = match v.lines.len() {
            1 => "1 line".to_string(),
            n => format!("{n} lines"),
        };
        match v.health {
            None => Banner::new(
                format!("Starting…  ·  {elapsed}"),
                "Connecting to Live Captions.",
                Tone::Waiting,
            ),
            Some(h) => {
                let head = match h {
                    Health::Recording => format!("●  Recording  ·  {elapsed}  ·  {lines}"),
                    Health::WaitingForSpeech => format!("{}  ·  {elapsed}", h.headline()),
                    _ => format!("⚠  {}", h.headline()),
                };
                let tone = match h {
                    Health::Recording => Tone::Recording,
                    Health::WaitingForSpeech => Tone::Waiting,
                    Health::LiveCaptionsOff | Health::NotReading | Health::EngineStopped => {
                        Tone::Problem
                    }
                };
                let mut guidance = h.guidance().to_string();
                if self.engine_mode {
                    // Shared guidance talks about Live Captions; say where captions come from.
                    match h {
                        Health::Recording => {
                            guidance = format!("Saving what {} sends.", self.engine_name)
                        }
                        Health::WaitingForSpeech => {
                            guidance = format!(
                                "{} is ready. Lines appear here as soon as someone speaks.",
                                self.engine_name
                            )
                        }
                        _ => {}
                    }
                }
                if h == Health::Recording && !self.remember {
                    guidance = "Showing captions, but NOT saving to disk (turn on in Settings)."
                        .into();
                }
                Banner::new(head, guidance, tone)
            }
        }
    }

    /// Window title: shows recording state in the taskbar / Dock.
    pub fn window_title(&self) -> String {
        if !self.view.listening {
            return "Interpres".into();
        }
        match self.view.health {
            Some(Health::Recording) => "● Recording — Interpres".into(),
            Some(h) if h.is_problem() => format!("⚠ {} — Interpres", h.headline()),
            _ => "Interpres — listening".into(),
        }
    }

    /// Label of the Start/Stop button.
    pub fn toggle_label(&self) -> &'static str {
        let problem = self.view.health.is_some_and(|h| h.is_problem());
        if self.view.stopping {
            "Saving…"
        } else if self.view.listening && self.view.idle.asking() && !problem {
            "■   Stop & save"
        } else if self.view.listening {
            "■   Stop recording"
        } else {
            "▶   Start recording"
        }
    }

    /// Label of the contextual Live Captions button (None = hidden).
    pub fn action_label(&self) -> Option<&'static str> {
        match self.lc_action() {
            LcAction::None => None,
            LcAction::TurnOn => Some("Turn on Live Captions"),
            LcAction::KeepRecording => Some("Keep recording"),
            LcAction::Restart => Some("Restart Live Captions"),
        }
    }

    pub fn checklist(&self) -> String {
        let v = &self.view;
        if self.engine_mode {
            let engine = match (v.listening, v.health) {
                (true, Some(Health::EngineStopped)) => format!("✗ Engine stopped: {}", self.engine_name),
                (true, Some(_)) => format!("✓ Engine running: {}", self.engine_name),
                _ => format!("○ Engine: {}", self.engine_name),
            };
            let reading = match (v.listening, v.health) {
                (true, Some(Health::Recording)) => "✓ Receiving captions",
                (true, Some(Health::WaitingForSpeech)) => "○ Waiting for speech",
                (true, None) => "○ Engine starting…",
                _ => "○ Receiving captions",
            };
            let saving = if self.remember { "✓ Save to disk on" } else { "✗ Not saving to disk" };
            return format!("{engine}        {reading}        {saving}");
        }
        let lc_on = if v.listening {
            v.health.map(|h| h != Health::LiveCaptionsOff)
        } else {
            v.idle_lc_on
        };
        let lc = match lc_on {
            Some(true) => "✓ Live Captions on",
            Some(false) => "✗ Live Captions off",
            None => "○ Live Captions",
        };
        let reading = if !v.listening {
            "○ Reading captions (starts with recording)"
        } else {
            match v.health {
                Some(Health::Recording) => "✓ Reading captions",
                Some(Health::WaitingForSpeech) => "○ Waiting for speech",
                Some(Health::NotReading) => "✗ Can't read captions",
                _ => "○ Reading captions",
            }
        };
        let saving = if !self.remember {
            "✗ Not saving to disk"
        } else if v.listening && v.session_active {
            "✓ Saving to file"
        } else {
            "✓ Save to disk on"
        };
        format!("{lc}        {reading}        {saving}")
    }

    pub fn footer(&self) -> String {
        match (&self.view.session_path, self.view.session_active) {
            (Some(p), true) => format!("Saving to  {}", p.display()),
            (Some(p), false) => format!("Saved to  {}", p.display()),
            (None, _) if self.remember => {
                format!("Transcripts folder:  {}", self.engine.folder().display())
            }
            (None, _) => "Save to disk is off — transcripts are not being kept.".into(),
        }
    }

    /// Transcript rows as shown (each ends with a line break): saved lines, then the
    /// live line.
    pub fn transcript_rows(&self) -> Vec<String> {
        let v = &self.view;
        let mut out: Vec<String> = v
            .times
            .iter()
            .zip(&v.lines)
            .map(|(t, line)| format!("{t}   {line}{EOL}"))
            .collect();
        let live = v.live.trim();
        // Live Captions can re-show an older line; only show text not already saved.
        let live_is_new = !live.is_empty()
            && !v
                .lines
                .iter()
                .rev()
                .take(6)
                .any(|l| l == live || same_or_refinement(l, live));
        if v.listening && live_is_new {
            out.push(format!("   …     {}{EOL}", live.replace('\n', " ")));
        }
        if out.is_empty() {
            out.push(
                if v.listening {
                    "Captions will appear here as they come in."
                } else {
                    "Press Start recording. Captions will appear here, and are saved to a file as you go."
                }
                .to_string(),
            );
        }
        out
    }

    pub fn plain_transcript(&self) -> String {
        self.view
            .times
            .iter()
            .zip(&self.view.lines)
            .map(|(t, l)| format!("[{t}] {l}{EOL}"))
            .collect()
    }

    /// Returns true when the window should alert the user (a problem started).
    pub fn apply_event(&mut self, ev: EngineEvent) -> bool {
        let now_ms = self.now_ms();
        let idle_prompt_ms = self.idle_prompt_ms;
        let engine_mode = self.engine_mode;
        let v = &mut self.view;
        match ev {
            EngineEvent::Status(s) => {
                // Banner + checklist already say these; keep the detail line for news.
                let redundant = s.starts_with("Live Captions detected")
                    || s.starts_with("Folder:")
                    || s.starts_with("Listening to Live Captions")
                    || s.starts_with("Saving what Live Captions shows")
                    || s == "Stopped."
                    || s.starts_with("Live Captions stopped")
                    // The health banner covers stuck / missing captions.
                    || s.starts_with(LAG_TIP);
                if !redundant {
                    v.detail = s;
                }
            }
            EngineEvent::Error(s) => v.detail = format!("⚠  {s}"),
            EngineEvent::Live(s) => {
                if s != v.live {
                    if !s.trim().is_empty() {
                        v.idle.on_activity(now_ms);
                    }
                    v.live = s;
                    v.transcript_dirty = true;
                }
            }
            EngineEvent::Final(s) => {
                // External engines send finished lines: list them exactly as the file does.
                let plan = if engine_mode {
                    FamilyPlan::Append
                } else {
                    plan_family(&v.lines, s.trim())
                };
                v.idle.on_activity(now_ms);
                apply_caption(v, plan, &s);
            }
            EngineEvent::Revised(s) => {
                let plan = plan_family(&v.lines, s.trim());
                v.idle.on_activity(now_ms);
                apply_caption(v, plan, &s);
            }
            EngineEvent::SessionFile(Some(p)) => {
                v.session_path = Some(p);
                v.session_active = true;
            }
            EngineEvent::SessionFile(None) => v.session_active = false,
            EngineEvent::Listening(on) => {
                v.listening = on;
                v.stopping = false;
                v.health = None;
                v.live.clear();
                v.idle = IdlePrompt::new(if on { idle_prompt_ms } else { 0 }, now_ms);
                if on {
                    v.started_at = Some(Instant::now());
                    v.lines.clear();
                    v.times.clear();
                    v.detail.clear();
                    v.session_path = None;
                    self.auto_start_pending = None;
                } else {
                    v.started_at = None;
                    v.last_idle_check = None;
                    v.detail.clear();
                    self.auto.on_stopped();
                }
                if let Some(note) = self.auto_note.take() {
                    self.view.detail = note;
                }
                self.view.transcript_dirty = true;
            }
            EngineEvent::Health(h) => {
                let was = v.health;
                v.health = Some(h);
                if h == Health::Recording {
                    v.detail.clear();
                }
                return v.listening && h.is_problem() && was != Some(h);
            }
        }
        false
    }

    /// Drain engine events, run auto-record and the idle prompt, refresh Live Captions
    /// presence while stopped. Call on the UI timer.
    pub fn pump(&mut self) -> Attention {
        let mut att = Attention::default();
        loop {
            match self.rx.try_recv() {
                Ok(ev) => att.alert |= self.apply_event(ev),
                Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => break,
            }
        }
        self.tick_auto_record();
        let now_ms = self.now_ms();
        let problem = self.view.health.is_some_and(|h| h.is_problem());
        if self.view.listening && !self.view.stopping && !problem && self.view.idle.tick(now_ms) {
            crate::debuglog::log("ui idle prompt: asking if the meeting is done");
            att.nudge = true;
        }
        if !self.view.listening && !self.engine_mode {
            let due = self
                .view
                .last_idle_check
                .map_or(true, |t| t.elapsed() >= IDLE_LC_CHECK);
            if due {
                self.view.last_idle_check = Some(Instant::now());
                self.view.idle_lc_on = Some(platform::live_captions_present().running);
            }
        }
        att
    }

    /// True when the transcript should be re-rendered now.
    pub fn transcript_due(&self) -> bool {
        self.view.transcript_dirty
            && self
                .view
                .last_render
                .map_or(true, |t| t.elapsed() >= TRANSCRIPT_MIN_INTERVAL)
    }

    /// Auto-record: feed the speaker level, then start or stop recording as needed.
    fn tick_auto_record(&mut self) {
        let Some(sound) = self.sound.clone() else {
            return;
        };
        let now = self.now_ms();
        let readable = sound.healthy(SOUND_STALE);
        if readable {
            self.detector.sample(now, sound.take_peak());
        } else {
            // Unknown is not silence: never stop a recording on a broken meter.
            self.detector.assume_sound(now);
        }
        if (readable || sound.age() >= SOUND_STALE) && self.sound_readable != Some(readable) {
            self.sound_readable = Some(readable);
            crate::debuglog::log(&format!("auto-record: speaker level readable={readable}"));
            if !readable {
                self.view.detail =
                    "⚠  Can't read the speaker level — auto-record won't start or stop by itself."
                        .into();
            }
        }
        if self.view.stopping {
            return;
        }
        if let Some(since) = self.auto_start_pending {
            if self.view.listening {
                self.auto_start_pending = None;
            } else if self.view.idle_lc_on == Some(true) || since.elapsed() >= AUTO_LC_WAIT {
                self.auto_start_pending = None;
                self.auto_start();
            }
            return;
        }
        let playing = readable && self.detector.playing(now);
        let quiet = self.quiet_ms();
        match self.auto.tick(self.view.listening, playing, quiet) {
            AutoAction::None => {}
            AutoAction::Start => {
                self.refresh_source();
                let lc_off = CAN_LAUNCH_LC
                    && !self.engine_mode
                    && self.view.idle_lc_on != Some(true)
                    && !platform::live_captions_present().running;
                if lc_off {
                    // Start once Live Captions is up, so the banner doesn't flash "off".
                    crate::debuglog::log("auto-record: sound playing — turning on Live Captions");
                    self.auto_start_pending = Some(Instant::now());
                    self.view.idle_lc_on = Some(false);
                    self.view.last_idle_check = Some(Instant::now());
                    thread::spawn(|| {
                        if let Err(e) = platform::launch_live_captions() {
                            crate::debuglog::log(&format!(
                                "auto-record: Live Captions launch failed: {e}"
                            ));
                        }
                    });
                } else {
                    // macOS: Live Captions can only be turned on by the user; if it is
                    // off, the recording's banner says so and alerts them.
                    self.auto_start();
                }
            }
            AutoAction::Stop => {
                let mins = self.auto.quiet_stop_ms() / 60_000;
                crate::debuglog::log(&format!("auto-record: no sound for {mins} min — stop and save"));
                self.view.stopping = true;
                self.engine.request_stop_because(&format!("no sound for {mins} min"));
                self.auto_note = Some(format!(
                    "Stopped and saved after {mins} min with no sound. Recording starts again when sound plays."
                ));
            }
        }
    }

    fn auto_start(&mut self) {
        crate::debuglog::log("auto-record: sound playing — start recording");
        self.refresh_source();
        self.auto_note = Some("Sound is playing — recording started automatically.".into());
        self.engine.start();
    }

    /// Re-read the caption source from settings (the file may have been edited by hand).
    pub fn refresh_source(&mut self) {
        let cfg = Config::load();
        self.engine_mode = cfg.uses_external_engine();
        self.engine_name = cfg.engine_name();
        self.engine_configured = cfg.helper_path.is_some();
        self.idle_prompt_ms = cfg.idle_prompt_minutes.saturating_mul(60_000);
        self.auto
            .set_quiet_stop_ms(cfg.auto_stop_quiet_minutes.saturating_mul(60_000));
    }

    /// Start / Stop button.
    pub fn toggle_recording(&mut self) {
        if self.view.listening {
            if !self.view.stopping {
                self.view.stopping = true;
                self.engine.request_stop();
                crate::debuglog::log("ui Stop recording clicked");
            }
        } else {
            self.refresh_source();
            self.engine.start();
            crate::debuglog::log("ui Start recording clicked");
        }
    }

    /// "Auto-record when sound plays" checkbox.
    pub fn toggle_auto_record(&mut self) {
        self.auto_on = !self.auto_on;
        let mut cfg = Config::load();
        cfg.auto_record = self.auto_on;
        let _ = cfg.save();
        if let Some(s) = self.sound.take() {
            s.stop();
        }
        self.auto_start_pending = None;
        self.sound_readable = None;
        if self.auto_on {
            let quiet_min = cfg.auto_stop_quiet_minutes;
            self.auto = AutoRecord::new(quiet_min.saturating_mul(60_000));
            self.detector = SoundDetector::new(self.now_ms());
            self.sound = Some(spawn_sound_meter());
            self.view.detail = if quiet_min > 0 {
                format!(
                    "Auto-record ON — starts when sound plays, stops and saves after {quiet_min} min of silence."
                )
            } else {
                "Auto-record ON — starts when sound plays.".into()
            };
        } else {
            self.view.detail = "Auto-record OFF — press Start recording yourself.".into();
        }
        crate::debuglog::log(&format!(
            "ui auto-record {}",
            if self.auto_on { "on" } else { "off" }
        ));
    }

    pub fn toggle_save(&mut self) {
        self.remember = !self.remember;
        self.engine.set_remember(self.remember);
        self.view.detail = if self.remember {
            "Save to disk is ON — the next recording is saved.".into()
        } else {
            "Save to disk is OFF — captions are shown but not kept.".into()
        };
    }

    pub fn set_folder(&mut self, path: &str) {
        self.engine.set_folder(PathBuf::from(path));
        crate::debuglog::set_folder(std::path::Path::new(path));
        self.view.detail = format!("Transcripts will be saved in {path}");
    }

    pub fn toggle_debug(&mut self) {
        self.debug = !self.debug;
        let on = self.debug;
        crate::debuglog::set_folder(&self.engine.folder());
        crate::debuglog::set_enabled(on);
        let mut cfg = Config::load();
        cfg.debug = on;
        let _ = cfg.save();
        self.view.detail = if on {
            format!(
                "Debug log ON — {}",
                crate::debuglog::path_for_display().display()
            )
        } else {
            "Debug log OFF.".into()
        };
        if on {
            crate::debuglog::log("debug enabled from UI");
        }
    }

    pub fn set_theme(&mut self, mode: ThemeMode) {
        self.theme_mode = mode;
        let mut cfg = Config::load();
        cfg.theme = mode;
        let _ = cfg.save();
    }

    /// Captions from Live Captions (`false`) or the external engine (`true`).
    pub fn set_source(&mut self, engine: bool) {
        let mut cfg = Config::load();
        cfg.source = if engine { "engine" } else { "os" }.into();
        let _ = cfg.save();
        self.refresh_source();
        let what = if self.engine_mode {
            format!("external engine ({})", self.engine_name)
        } else {
            LC_NAME.into()
        };
        self.view.detail = if self.view.listening {
            format!("Caption source: {what}. Takes effect next time you press Start recording.")
        } else {
            format!("Caption source: {what}.")
        };
        self.view.idle_lc_on = None;
        self.view.last_idle_check = None;
    }

    /// Turn on / restart Live Captions (off the UI thread), or snooze the idle prompt.
    pub fn run_lc_action(&mut self, action: LcAction) {
        if action == LcAction::KeepRecording {
            let now = self.now_ms();
            self.view.idle.snooze(now);
            self.view.detail =
                "OK — still recording. Interpres will check again if it stays quiet.".into();
            crate::debuglog::log("ui idle prompt: keep recording");
            return;
        }
        let msg = match action {
            LcAction::None | LcAction::KeepRecording => return,
            LcAction::TurnOn if CAN_LAUNCH_LC => "Turning on Live Captions…",
            LcAction::TurnOn => {
                "Opened Live Captions settings — switch Live Captions on there."
            }
            LcAction::Restart => {
                "Restarting Live Captions… (recording continues in the same file)"
            }
        };
        self.view.detail = msg.into();
        crate::debuglog::log(&format!("ui {msg}"));
        thread::spawn(move || {
            let res = match action {
                LcAction::TurnOn => platform::launch_live_captions(),
                _ => platform::restart_live_captions(),
            };
            if let Err(e) = res {
                crate::debuglog::log(&format!("Live Captions action failed: {e}"));
            }
        });
    }

    /// Settings → Check Live Captions setup. Returns the message to show.
    pub fn run_setup_check(&mut self) {
        let presence = platform::live_captions_present();
        let msg = if !presence.running {
            #[cfg(windows)]
            {
                "Live Captions is off. Use “Turn on Live Captions” (or Win+Ctrl+L).".to_string()
            }
            #[cfg(not(windows))]
            {
                "Live Captions is off. Use “Turn on Live Captions” (System Settings → Accessibility → Live Captions)."
                    .to_string()
            }
        } else {
            let snap = platform::poll_capture();
            if !self.view.listening {
                platform::shutdown_capture();
            }
            match (snap.surface_text, snap.error) {
                (Some(_), _) => "✓ All good — Interpres can read Live Captions.".into(),
                (None, None) => {
                    "✓ Live Captions is on and readable — waiting for someone to speak.".into()
                }
                (None, Some(e)) => format!("⚠  Live Captions is on but can't be read: {e}"),
            }
        };
        self.view.detail = msg;
    }

    /// Window is closing: stop the meter and save what was heard.
    pub fn shutdown(&mut self) {
        if let Some(s) = self.sound.take() {
            s.stop();
        }
        self.engine.request_stop_because("window closed");
        self.engine.stop();
        platform::shutdown_capture();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elapsed_formats_minutes_then_hours() {
        assert_eq!(format_elapsed(Duration::from_secs(5)), "00:05");
        assert_eq!(format_elapsed(Duration::from_secs(14 * 60 + 32)), "14:32");
        assert_eq!(format_elapsed(Duration::from_secs(3600 + 62)), "1:01:02");
    }

    #[test]
    fn only_the_changed_tail_is_rerendered() {
        let r = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        // New line appended: keep everything before it.
        assert_eq!(first_changed_row(&r(&["a", "b"]), &r(&["a", "b", "c"])), 2);
        // Live line changed: only the last row.
        assert_eq!(first_changed_row(&r(&["a", "live1"]), &r(&["a", "live2"])), 1);
        // A recent line polished: re-render from that line.
        assert_eq!(first_changed_row(&r(&["a", "b", "c"]), &r(&["a", "B", "c"])), 1);
        // Live line removed after it became a saved line.
        assert_eq!(first_changed_row(&r(&["a", "live"]), &r(&["a"])), 1);
        assert_eq!(first_changed_row(&r(&["a"]), &r(&["a"])), 1);
    }

    #[test]
    fn line_times_stay_in_step_with_history() {
        let mut v = View {
            listening: true,
            ..View::default()
        };
        let add = |v: &mut View, t: &str| {
            let plan = plan_family(&v.lines, t);
            apply_caption(v, plan, t);
        };
        add(&mut v, "We can meet on Thursday.");
        let first_time = v.times[0].clone();
        add(&mut v, "I'll send the invite tonight.");
        assert_eq!(v.lines.len(), 2);
        assert_eq!(v.times.len(), 2);
        // Polishing the first line keeps its time.
        add(&mut v, "We can meet on Thursday, if that suits you.");
        assert_eq!(v.times.len(), v.lines.len());
        assert_eq!(v.times[0], first_time);
        assert!(v.transcript_dirty);
    }
}
