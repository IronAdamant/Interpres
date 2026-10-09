//! OS Live Captions process detection and optional text capture.

mod detect;
mod signals;

#[cfg(target_os = "macos")]
pub mod macos;

#[cfg(windows)]
pub mod windows;

#[cfg(windows)]
mod windows_uia;

#[cfg(windows)]
pub mod windows_audio;

#[cfg(target_os = "macos")]
pub mod macos_audio;

/// Speaker activity for auto-record (same API on both OSes).
#[cfg(windows)]
pub use windows_audio as sound;
#[cfg(target_os = "macos")]
pub use macos_audio as sound;

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

/// Tests that create UI Automation clients (directly, or via the caption reader when Live
/// Captions is open) take turns: concurrent client setup can fail with E_FAIL.
#[cfg(all(test, windows))]
pub(crate) static UIA_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Release long-lived capture resources (Windows caption reader). No-op elsewhere.
pub fn shutdown_capture() {
    #[cfg(windows)]
    {
        windows::shutdown_reader();
    }
}

/// Switch Live Captions on. Windows starts it; macOS only lets the user switch it on, so
/// this opens its page in System Settings.
#[cfg(any(windows, target_os = "macos"))]
pub fn launch_live_captions() -> std::io::Result<()> {
    #[cfg(windows)]
    {
        windows::launch_live_captions()
    }
    #[cfg(target_os = "macos")]
    {
        macos::open_live_captions_settings()
    }
}

/// Close Live Captions and start it again (the transcript file continues).
#[cfg(any(windows, target_os = "macos"))]
pub fn restart_live_captions() -> std::io::Result<()> {
    #[cfg(windows)]
    {
        windows::restart_live_captions()
    }
    #[cfg(target_os = "macos")]
    {
        macos::restart_live_captions()
    }
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
