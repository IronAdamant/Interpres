//! Live Captions process-presence lifecycle with debounce.

/// Pure state machine: open companion when LC present; close after off-delay when absent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LifecycleAction {
    /// No change this tick.
    None,
    /// Live Captions appeared (or already present on cold start) → ensure main/capture on.
    Open,
    /// Live Captions gone longer than off-delay → stop capture / quit or idle.
    Close,
}

#[derive(Clone, Debug)]
pub struct Lifecycle {
    /// Whether we currently treat LC as "on" for the companion.
    pub companion_active: bool,
    /// Milliseconds LC has been continuously absent while companion was active.
    absent_ms: u64,
    /// Configured off-delay.
    pub off_delay_ms: u64,
}

/// Floor for LC-off debounce so brief process/window detection flakes do not
/// end an open session (field packs showed early `lc_stopped` under load).
pub const MIN_OFF_DELAY_MS: u64 = 2500;

impl Lifecycle {
    pub fn new(off_delay_ms: u64) -> Self {
        Self {
            companion_active: false,
            absent_ms: 0,
            // Never weaker than MIN; config may raise further.
            off_delay_ms: off_delay_ms.max(MIN_OFF_DELAY_MS),
        }
    }

    /// Feed one observation of process presence and the poll interval elapsed.
    pub fn tick(&mut self, lc_present: bool, elapsed_ms: u64) -> LifecycleAction {
        if lc_present {
            self.absent_ms = 0;
            if !self.companion_active {
                self.companion_active = true;
                return LifecycleAction::Open;
            }
            return LifecycleAction::None;
        }

        // LC not present
        if !self.companion_active {
            self.absent_ms = 0;
            return LifecycleAction::None;
        }

        self.absent_ms = self.absent_ms.saturating_add(elapsed_ms);
        if self.absent_ms >= self.off_delay_ms {
            self.companion_active = false;
            self.absent_ms = 0;
            return LifecycleAction::Close;
        }
        LifecycleAction::None
    }

    /// For tests / status.
    pub fn absent_ms(&self) -> u64 {
        self.absent_ms
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_on_present_close_after_off_delay() {
        let mut lc = Lifecycle::new(2500);

        assert_eq!(lc.tick(false, 1000), LifecycleAction::None);
        assert_eq!(lc.tick(true, 1000), LifecycleAction::Open);
        assert_eq!(lc.tick(true, 1000), LifecycleAction::None);

        // Brief flicker: absent but under delay
        assert_eq!(lc.tick(false, 1000), LifecycleAction::None);
        assert_eq!(lc.companion_active, true);
        // Return before delay → stay open
        assert_eq!(lc.tick(true, 1000), LifecycleAction::None);

        // Full off sequence
        assert_eq!(lc.tick(false, 1000), LifecycleAction::None);
        assert_eq!(lc.tick(false, 1000), LifecycleAction::None);
        assert_eq!(lc.tick(false, 600), LifecycleAction::Close);
        assert_eq!(lc.companion_active, false);

        // Second open
        assert_eq!(lc.tick(true, 500), LifecycleAction::Open);
    }

    #[test]
    fn cold_start_already_running() {
        // Requested 2000 is raised to MIN_OFF_DELAY_MS floor.
        let mut lc = Lifecycle::new(2000);
        assert_eq!(lc.off_delay_ms, MIN_OFF_DELAY_MS);
        assert_eq!(lc.tick(true, 0), LifecycleAction::Open);
    }

    #[test]
    fn brief_presence_flicker_does_not_close_session() {
        let mut lc = Lifecycle::new(2500);
        assert_eq!(lc.tick(true, 400), LifecycleAction::Open);
        // Several short absences under delay (detection flake pattern).
        for _ in 0..5 {
            assert_eq!(lc.tick(false, 400), LifecycleAction::None);
            assert!(lc.companion_active);
            assert_eq!(lc.tick(true, 400), LifecycleAction::None);
            assert!(lc.companion_active);
        }
        // Sustained absence closes only after full off-delay.
        let mut closed = false;
        for _ in 0..10 {
            match lc.tick(false, 400) {
                LifecycleAction::Close => {
                    closed = true;
                    break;
                }
                LifecycleAction::None => assert!(lc.companion_active || closed),
                LifecycleAction::Open => panic!("unexpected open"),
            }
        }
        assert!(closed, "must Close after sustained absence");
        assert!(!lc.companion_active);
    }

    #[test]
    fn off_delay_floor_not_weaker_than_min() {
        let lc = Lifecycle::new(0);
        assert_eq!(lc.off_delay_ms, MIN_OFF_DELAY_MS);
        let lc2 = Lifecycle::new(10_000);
        assert_eq!(lc2.off_delay_ms, 10_000);
    }
}
