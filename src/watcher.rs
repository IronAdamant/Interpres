//! Light watcher loop: poll LC process and report open/close actions.

use crate::lifecycle::{Lifecycle, LifecycleAction};
use crate::platform;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

/// Run watcher until `stop` is set. Calls `on_action` for Open/Close.
pub fn run_watcher_loop(
    off_delay_ms: u64,
    poll_ms: u64,
    stop: Arc<AtomicBool>,
    mut on_action: impl FnMut(LifecycleAction),
) {
    let mut life = Lifecycle::new(off_delay_ms);
    while !stop.load(Ordering::SeqCst) {
        let present = platform::live_captions_present().running;
        let action = life.tick(present, poll_ms);
        if action != LifecycleAction::None {
            on_action(action);
        }
        thread::sleep(Duration::from_millis(poll_ms.max(200)));
    }
}

/// Single-step helper for integration: one poll + lifecycle tick.
pub fn watcher_step(life: &mut Lifecycle, poll_ms: u64) -> LifecycleAction {
    let present = platform::live_captions_present().running;
    life.tick(present, poll_ms)
}
