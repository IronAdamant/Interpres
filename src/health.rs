//! Capture health: one user-facing answer to "is Interpres actually recording?"
//!
//! Pure state machine (no threads / clocks) shared by Windows and macOS engines.
//! Distinguishes "Live Captions has nothing to show yet" (normal) from "we cannot
//! read Live Captions" (alarm), which the old status text conflated.

/// What the user should know right now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Health {
    /// Live Captions is not running.
    LiveCaptionsOff,
    /// Live Captions is open but showing no caption text (nobody speaking yet).
    WaitingForSpeech,
    /// Caption text is being read.
    Recording,
    /// Live Captions is running but reads keep failing — captions are being missed.
    NotReading,
    /// External speech engine is not running (failed to start or exited; auto-restarts).
    EngineStopped,
}

/// Failures must persist this long before raising `NotReading` (rides out blips).
pub const NOT_READING_AFTER_MS: u64 = 6_000;

impl Health {
    /// True when the user is losing captions and should act.
    pub fn is_problem(self) -> bool {
        matches!(
            self,
            Health::NotReading | Health::LiveCaptionsOff | Health::EngineStopped
        )
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Health::LiveCaptionsOff => "live_captions_off",
            Health::WaitingForSpeech => "waiting_for_speech",
            Health::Recording => "recording",
            Health::NotReading => "not_reading",
            Health::EngineStopped => "engine_stopped",
        }
    }

    /// Short banner text.
    pub fn headline(self) -> &'static str {
        match self {
            Health::LiveCaptionsOff => "Live Captions is off",
            Health::WaitingForSpeech => "Ready — waiting for speech",
            Health::Recording => "Recording",
            Health::NotReading => "Not capturing — can't read Live Captions",
            Health::EngineStopped => "Not capturing — caption engine stopped",
        }
    }

    /// One-line guidance under the banner.
    pub fn guidance(self) -> &'static str {
        match self {
            Health::LiveCaptionsOff => {
                #[cfg(target_os = "macos")]
                {
                    "Turn on Live Captions (System Settings → Accessibility). Nothing is being saved."
                }
                #[cfg(not(target_os = "macos"))]
                {
                    "Turn on Live Captions (Win+Ctrl+L or the button). Nothing is being saved."
                }
            }
            Health::WaitingForSpeech => {
                "Live Captions is on. Lines appear here as soon as someone speaks."
            }
            Health::Recording => "Saving what Live Captions shows.",
            Health::NotReading => {
                "Captions are being missed. Restart Live Captions — Interpres keeps the same file."
            }
            Health::EngineStopped => {
                "Restarting it automatically. If this repeats, check the engine settings and debug log."
            }
        }
    }
}

/// Tracks poll outcomes and reports `Health` changes.
#[derive(Clone, Debug, Default)]
pub struct HealthMonitor {
    current: Option<Health>,
    failing_since_ms: Option<u64>,
}

impl HealthMonitor {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn current(&self) -> Option<Health> {
        self.current
    }

    /// Feed one poll. `now_ms` is any monotonic millisecond clock.
    /// Returns `Some(new)` only when the health changed.
    pub fn on_poll(
        &mut self,
        now_ms: u64,
        lc_running: bool,
        has_text: bool,
        error: Option<&str>,
    ) -> Option<Health> {
        let next = if !lc_running {
            self.failing_since_ms = None;
            Health::LiveCaptionsOff
        } else if has_text {
            self.failing_since_ms = None;
            Health::Recording
        } else if error.is_some() {
            let since = *self.failing_since_ms.get_or_insert(now_ms);
            if now_ms.saturating_sub(since) >= NOT_READING_AFTER_MS {
                Health::NotReading
            } else {
                // Short blip: keep the last state; LC just (re)appeared → waiting.
                match self.current {
                    None | Some(Health::LiveCaptionsOff) => Health::WaitingForSpeech,
                    Some(h) => h,
                }
            }
        } else {
            self.failing_since_ms = None;
            Health::WaitingForSpeech
        };

        if self.current == Some(next) {
            None
        } else {
            self.current = Some(next);
            Some(next)
        }
    }
}

/// "Are you done?" prompt while recording. Pure timing; never stops anything itself.
///
/// Fires once `quiet_after_ms` passes with no new caption activity. Any new caption
/// dismisses it; "Keep recording" snoozes for another full quiet period.
#[derive(Clone, Debug, Default)]
pub struct IdlePrompt {
    quiet_after_ms: u64,
    last_activity_ms: u64,
    asking: bool,
}

impl IdlePrompt {
    /// `quiet_after_ms == 0` disables the prompt.
    pub fn new(quiet_after_ms: u64, now_ms: u64) -> Self {
        Self {
            quiet_after_ms,
            last_activity_ms: now_ms,
            asking: false,
        }
    }

    pub fn asking(&self) -> bool {
        self.asking
    }

    /// New caption text arrived (also dismisses an open prompt).
    pub fn on_activity(&mut self, now_ms: u64) {
        self.last_activity_ms = now_ms;
        self.asking = false;
    }

    /// User chose "Keep recording".
    pub fn snooze(&mut self, now_ms: u64) {
        self.on_activity(now_ms);
    }

    /// How long nothing new has been captioned.
    pub fn quiet_ms(&self, now_ms: u64) -> u64 {
        now_ms.saturating_sub(self.last_activity_ms)
    }

    /// Returns true exactly when the prompt should start showing.
    pub fn tick(&mut self, now_ms: u64) -> bool {
        if self.quiet_after_ms == 0 || self.asking {
            return false;
        }
        if self.quiet_ms(now_ms) >= self.quiet_after_ms {
            self.asking = true;
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waiting_for_speech_is_not_an_error() {
        // Field log: LC open before anyone spoke showed "unable to capture".
        let mut m = HealthMonitor::new();
        assert_eq!(m.on_poll(0, true, false, None), Some(Health::WaitingForSpeech));
        for t in 1..100 {
            assert_eq!(m.on_poll(t * 200, true, false, None), None);
        }
        assert!(!m.current().unwrap().is_problem());
    }

    #[test]
    fn sustained_read_failures_raise_not_reading() {
        let mut m = HealthMonitor::new();
        assert_eq!(m.on_poll(0, true, true, None), Some(Health::Recording));
        // Blip under threshold keeps Recording.
        assert_eq!(m.on_poll(1_000, true, false, Some("timeout")), None);
        assert_eq!(m.on_poll(1_200, true, true, None), None);
        // Sustained failure crosses threshold.
        let mut raised = None;
        for t in 0..=40 {
            if let Some(h) = m.on_poll(2_000 + t * 200, true, false, Some("timeout")) {
                raised = Some((h, t * 200));
                break;
            }
        }
        let (h, after) = raised.expect("must raise");
        assert_eq!(h, Health::NotReading);
        assert!(after >= NOT_READING_AFTER_MS, "raised after {after}ms");
        assert!(h.is_problem());
        // Recovery.
        assert_eq!(m.on_poll(20_000, true, true, None), Some(Health::Recording));
    }

    #[test]
    fn live_captions_off_wins_and_clears_failure_streak() {
        let mut m = HealthMonitor::new();
        m.on_poll(0, true, false, Some("x"));
        assert_eq!(m.on_poll(100, false, false, None), Some(Health::LiveCaptionsOff));
        // Coming back with an error does not instantly alarm (streak reset).
        assert_eq!(
            m.on_poll(200, true, false, Some("x")),
            Some(Health::WaitingForSpeech)
        );
        assert_eq!(m.on_poll(200 + NOT_READING_AFTER_MS - 1, true, false, Some("x")), None);
    }

    #[test]
    fn idle_prompt_asks_once_and_never_stops_by_itself() {
        let min = 60_000;
        let mut p = IdlePrompt::new(3 * min, 0);
        assert!(!p.tick(2 * min));
        p.on_activity(2 * min); // caption at 2 min
        assert!(!p.tick(4 * min));
        assert!(p.tick(5 * min), "3 quiet minutes after last caption");
        assert!(p.asking());
        assert!(!p.tick(6 * min), "fires once, then just keeps asking");
        // New caption dismisses.
        p.on_activity(7 * min);
        assert!(!p.asking());
        // Keep recording snoozes a full period.
        assert!(p.tick(10 * min));
        p.snooze(10 * min);
        assert!(!p.tick(12 * min));
        assert!(p.tick(13 * min));
    }

    #[test]
    fn idle_prompt_zero_disables() {
        let mut p = IdlePrompt::new(0, 0);
        assert!(!p.tick(u64::MAX / 2));
    }

    #[test]
    fn headlines_and_guidance_are_nonempty() {
        for h in [
            Health::LiveCaptionsOff,
            Health::WaitingForSpeech,
            Health::Recording,
            Health::NotReading,
            Health::EngineStopped,
        ] {
            assert!(!h.headline().is_empty());
            assert!(!h.guidance().is_empty());
            assert!(!h.as_str().is_empty());
        }
    }
}
