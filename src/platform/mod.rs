//! OS Live Captions process detection and optional text capture.

mod detect;
mod signals;

#[cfg(target_os = "macos")]
pub mod macos;

#[cfg(windows)]
mod windows;

pub use detect::{live_captions_present, LiveCaptionsPresence};
pub use signals::{macos_signals, windows_signals, SignalTable};

/// Snapshot used by probe and run loops.
#[derive(Clone, Debug)]
pub struct CaptureSnapshot {
    pub process_running: bool,
    pub detail: String,
    /// Full caption surface text if scrape succeeded.
    pub surface_text: Option<String>,
    pub error: Option<String>,
}

/// Poll once: process presence + best-effort text.
pub fn poll_capture() -> CaptureSnapshot {
    let presence = live_captions_present();
    if !presence.running {
        return CaptureSnapshot {
            process_running: false,
            detail: presence.detail,
            surface_text: None,
            error: None,
        };
    }

    #[cfg(target_os = "macos")]
    {
        return macos::poll_text(presence);
    }

    #[cfg(windows)]
    {
        return windows::poll_text(presence);
    }

    #[cfg(not(any(target_os = "macos", windows)))]
    {
        CaptureSnapshot {
            process_running: true,
            detail: presence.detail,
            surface_text: None,
            error: Some(
                "Live Captions capture is only implemented for Windows and macOS".into(),
            ),
        }
    }
}
