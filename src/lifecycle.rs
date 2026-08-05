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

impl Lifecycle {
    pub fn new(off_delay_ms: u64) -> Self {
        Self {
            companion_active: false,
            absent_ms: 0,
            off_delay_ms,
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
        let mut lc = Lifecycle::new(2000);
        assert_eq!(lc.tick(true, 0), LifecycleAction::Open);
    }
}
