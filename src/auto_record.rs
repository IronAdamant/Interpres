//! "Auto-record when sound plays": start recording when a meeting or video starts
//! playing through the speakers, and stop + save after a long silence.
//!
//! Pure logic (no OS calls) so it can be tested; the Windows UI feeds it speaker peak
//! levels from `platform::windows_audio`.

/// Peak level (0.0–1.0) that counts as sound. ~-40 dBFS: speech in a call or video is far
/// above this; idle devices read exactly 0.
pub const SOUND_THRESHOLD: f32 = 0.01;
/// Sound must keep going this long before recording starts (skips notification dings).
pub const SUSTAIN_MS: u64 = 3_000;
/// Gaps shorter than this still count as "still playing" (pauses between sentences).
pub const GAP_MS: u64 = 1_500;
/// After any stop, wait for this much quiet before auto-starting again — so pressing
/// Stop while a video is still playing does not immediately start a new recording.
pub const REARM_QUIET_MS: u64 = 60_000;

/// Tracks whether sound is playing, from periodic peak readings.
#[derive(Clone, Debug)]
pub struct SoundDetector {
    run_start_ms: Option<u64>,
    last_sound_ms: u64,
}

impl SoundDetector {
    pub fn new(now_ms: u64) -> Self {
        Self {
            run_start_ms: None,
            last_sound_ms: now_ms,
        }
    }

    /// Feed one peak reading.
    pub fn sample(&mut self, now_ms: u64, peak: f32) {
        if peak < SOUND_THRESHOLD {
            return;
        }
        let continues = self.run_start_ms.is_some()
            && now_ms.saturating_sub(self.last_sound_ms) <= GAP_MS;
        if !continues {
            self.run_start_ms = Some(now_ms);
        }
        self.last_sound_ms = now_ms;
    }

    /// The level can't be read right now: treat it as "maybe playing" so a broken meter
    /// never ends a recording.
    pub fn assume_sound(&mut self, now_ms: u64) {
        self.last_sound_ms = now_ms;
    }

    /// Sound has been going for at least `SUSTAIN_MS` and has not stopped.
    pub fn playing(&self, now_ms: u64) -> bool {
        match self.run_start_ms {
            Some(start) => {
                now_ms.saturating_sub(self.last_sound_ms) <= GAP_MS
                    && self.last_sound_ms.saturating_sub(start) >= SUSTAIN_MS
            }
            None => false,
        }
    }

    /// How long nothing has played.
    pub fn quiet_ms(&self, now_ms: u64) -> u64 {
        now_ms.saturating_sub(self.last_sound_ms)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutoAction {
    None,
    Start,
    /// Silent for the configured time: stop and save.
    Stop,
}

/// When to start / stop recording automatically.
#[derive(Clone, Debug)]
pub struct AutoRecord {
    /// Stop after this long without sound (0 = never stop automatically).
    quiet_stop_ms: u64,
    armed: bool,
}

impl AutoRecord {
    /// Armed immediately: ticking the box mid-meeting starts recording right away.
    pub fn new(quiet_stop_ms: u64) -> Self {
        Self {
            quiet_stop_ms,
            armed: true,
        }
    }

    pub fn quiet_stop_ms(&self) -> u64 {
        self.quiet_stop_ms
    }

    pub fn set_quiet_stop_ms(&mut self, ms: u64) {
        self.quiet_stop_ms = ms;
    }

    /// Recording ended (by the user or automatically).
    pub fn on_stopped(&mut self) {
        self.armed = false;
    }

    pub fn tick(&mut self, listening: bool, playing: bool, quiet_ms: u64) -> AutoAction {
        if listening {
            if self.quiet_stop_ms > 0 && quiet_ms >= self.quiet_stop_ms {
                return AutoAction::Stop;
            }
            return AutoAction::None;
        }
        if !self.armed {
            if quiet_ms < REARM_QUIET_MS {
                return AutoAction::None;
            }
            self.armed = true;
        }
        if playing {
            AutoAction::Start
        } else {
            AutoAction::None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: u64 = 60_000;

    /// Feed `peak` every 200 ms from `from` to `to`.
    fn feed(d: &mut SoundDetector, from: u64, to: u64, peak: f32) {
        let mut t = from;
        while t <= to {
            d.sample(t, peak);
            t += 200;
        }
    }

    #[test]
    fn a_ding_is_not_a_meeting() {
        let mut d = SoundDetector::new(0);
        feed(&mut d, 1_000, 1_600, 0.4);
        assert!(!d.playing(1_600));
        assert!(!d.playing(5_000));
    }

    #[test]
    fn steady_speech_counts_as_playing_through_short_pauses() {
        let mut d = SoundDetector::new(0);
        feed(&mut d, 0, 2_000, 0.2);
        // One-second pause between sentences.
        feed(&mut d, 3_000, 4_000, 0.2);
        assert!(d.playing(4_000));
        assert_eq!(d.quiet_ms(4_000), 0);
        assert!(!d.playing(4_000 + GAP_MS + 1), "stopped once the gap is too long");
    }

    #[test]
    fn faint_noise_is_ignored() {
        let mut d = SoundDetector::new(0);
        feed(&mut d, 0, 10_000, SOUND_THRESHOLD / 2.0);
        assert!(!d.playing(10_000));
        assert_eq!(d.quiet_ms(10_000), 10_000);
    }

    #[test]
    fn starts_when_sound_plays_and_stops_after_quiet_limit() {
        let mut a = AutoRecord::new(5 * MIN);
        assert_eq!(a.tick(false, false, 0), AutoAction::None);
        assert_eq!(a.tick(false, true, 0), AutoAction::Start);
        assert_eq!(a.tick(true, true, 0), AutoAction::None);
        assert_eq!(a.tick(true, false, 5 * MIN - 1), AutoAction::None);
        assert_eq!(a.tick(true, false, 5 * MIN), AutoAction::Stop);
    }

    #[test]
    fn manual_stop_while_video_plays_does_not_restart() {
        let mut a = AutoRecord::new(5 * MIN);
        a.on_stopped();
        assert_eq!(a.tick(false, true, 0), AutoAction::None);
        // Quiet for a minute, then the next meeting starts.
        assert_eq!(a.tick(false, false, REARM_QUIET_MS), AutoAction::None);
        assert_eq!(a.tick(false, true, 0), AutoAction::Start);
    }

    #[test]
    fn rearmed_after_an_automatic_stop() {
        let mut a = AutoRecord::new(5 * MIN);
        assert_eq!(a.tick(true, false, 5 * MIN), AutoAction::Stop);
        a.on_stopped();
        // The silence that caused the stop already exceeds the re-arm wait.
        assert_eq!(a.tick(false, true, 5 * MIN), AutoAction::Start);
    }

    #[test]
    fn zero_means_never_stop_automatically() {
        let mut a = AutoRecord::new(0);
        assert_eq!(a.tick(true, false, 600 * MIN), AutoAction::None);
    }

    #[test]
    fn unreadable_meter_holds_off_the_stop() {
        let mut d = SoundDetector::new(0);
        d.assume_sound(5 * MIN);
        assert_eq!(d.quiet_ms(5 * MIN + 1_000), 1_000);
    }
}
