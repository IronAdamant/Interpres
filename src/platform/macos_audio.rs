//! "Is a meeting or video playing?" for auto-record on macOS (hand-written FFI, zero crates).
//!
//! macOS has no public speaker peak meter without capturing audio (which needs the
//! "System Audio Recording" permission). Instead, two signals that need no new permission
//! and capture nothing:
//!
//! 1. **Something is playing**: an app has audio output running (Core Audio process
//!    objects, `kAudioProcessPropertyIsRunningOutput`; whole devices on macOS < 14.2).
//!    Always-on system speech services (Siri / dictation keep the speakers open) and
//!    Interpres itself are ignored.
//! 2. **Live Captions is captioning it**: its text changed within the last few seconds.
//!    An open-but-silent output stream (a paused browser tab) never starts a recording.
//!
//! The reading is 1.0 while both hold, else 0.0. Same API as `windows_audio`, so
//! `app_view` drives auto-record identically on both OSes.

use std::os::raw::c_void;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

type OsStatus = i32;
type AudioObjectId = u32;

#[repr(C)]
struct PropertyAddress {
    selector: u32,
    scope: u32,
    element: u32,
}

#[link(name = "CoreAudio", kind = "framework")]
extern "C" {
    fn AudioObjectGetPropertyDataSize(
        object: AudioObjectId,
        address: *const PropertyAddress,
        qualifier_size: u32,
        qualifier: *const c_void,
        data_size: *mut u32,
    ) -> OsStatus;
    fn AudioObjectGetPropertyData(
        object: AudioObjectId,
        address: *const PropertyAddress,
        qualifier_size: u32,
        qualifier: *const c_void,
        data_size: *mut u32,
        data: *mut c_void,
    ) -> OsStatus;
}

const fn fourcc(s: &[u8; 4]) -> u32 {
    ((s[0] as u32) << 24) | ((s[1] as u32) << 16) | ((s[2] as u32) << 8) | s[3] as u32
}

const SYSTEM_OBJECT: AudioObjectId = 1;
const ELEMENT_MAIN: u32 = 0;
const SCOPE_GLOBAL: u32 = fourcc(b"glob");
const SCOPE_OUTPUT: u32 = fourcc(b"outp");
const PROP_DEVICES: u32 = fourcc(b"dev#");
const PROP_STREAMS: u32 = fourcc(b"stm#");
const PROP_RUNNING_SOMEWHERE: u32 = fourcc(b"gone");
const PROP_PROCESSES: u32 = fourcc(b"prs#");
const PROP_PROCESS_PID: u32 = fourcc(b"ppid");
const PROP_PROCESS_RUNNING_OUTPUT: u32 = fourcc(b"piro");

/// Live Captions text changed this recently → it is captioning speech now.
const CAPTIONS_RECENT: Duration = Duration::from_secs(4);
/// How often Live Captions' text is read while something plays.
const CAPTIONS_EVERY: Duration = Duration::from_millis(500);
/// System services that keep the speakers open with nothing to hear.
const IGNORED_PLAYERS: &[&str] = &[
    "CoreSpeech",
    "corespeechd",
    "SpeechRecognitionCore",
    "assistantd",
    "Live Captions",
    "LiveTranscription",
    "AccessibilitySharedSupport",
];

/// How often the devices are asked.
const SAMPLE_EVERY: Duration = Duration::from_millis(250);
/// Re-list output devices this often (headphones plugged in, AirPods connected).
const REFRESH_DEVICES_EVERY: Duration = Duration::from_secs(10);

fn property_size(object: AudioObjectId, selector: u32, scope: u32) -> Result<u32, OsStatus> {
    let addr = PropertyAddress {
        selector,
        scope,
        element: ELEMENT_MAIN,
    };
    let mut size = 0u32;
    let st = unsafe { AudioObjectGetPropertyDataSize(object, &addr, 0, ptr::null(), &mut size) };
    if st == 0 {
        Ok(size)
    } else {
        Err(st)
    }
}

/// Every device that has at least one output stream.
fn output_devices() -> Result<Vec<AudioObjectId>, String> {
    let size = property_size(SYSTEM_OBJECT, PROP_DEVICES, SCOPE_GLOBAL)
        .map_err(|st| format!("device list size failed (OSStatus {st})"))?;
    let count = size as usize / std::mem::size_of::<AudioObjectId>();
    let mut ids = vec![0 as AudioObjectId; count];
    let addr = PropertyAddress {
        selector: PROP_DEVICES,
        scope: SCOPE_GLOBAL,
        element: ELEMENT_MAIN,
    };
    let mut got = size;
    let st = unsafe {
        AudioObjectGetPropertyData(
            SYSTEM_OBJECT,
            &addr,
            0,
            ptr::null(),
            &mut got,
            ids.as_mut_ptr() as *mut c_void,
        )
    };
    if st != 0 {
        return Err(format!("device list failed (OSStatus {st})"));
    }
    ids.truncate(got as usize / std::mem::size_of::<AudioObjectId>());
    Ok(ids
        .into_iter()
        .filter(|&id| property_size(id, PROP_STREAMS, SCOPE_OUTPUT).is_ok_and(|s| s > 0))
        .collect())
}

/// Some(true) if any app is playing through `device`; None if it can't be asked.
fn device_running(device: AudioObjectId) -> Option<bool> {
    let addr = PropertyAddress {
        selector: PROP_RUNNING_SOMEWHERE,
        scope: SCOPE_GLOBAL,
        element: ELEMENT_MAIN,
    };
    let mut value = 0u32;
    let mut size = std::mem::size_of::<u32>() as u32;
    let st = unsafe {
        AudioObjectGetPropertyData(
            device,
            &addr,
            0,
            ptr::null(),
            &mut size,
            &mut value as *mut u32 as *mut c_void,
        )
    };
    (st == 0).then_some(value != 0)
}

fn read_u32(object: AudioObjectId, selector: u32) -> Option<u32> {
    let addr = PropertyAddress {
        selector,
        scope: SCOPE_GLOBAL,
        element: ELEMENT_MAIN,
    };
    let mut value = 0u32;
    let mut size = std::mem::size_of::<u32>() as u32;
    let st = unsafe {
        AudioObjectGetPropertyData(
            object,
            &addr,
            0,
            ptr::null(),
            &mut size,
            &mut value as *mut u32 as *mut c_void,
        )
    };
    (st == 0).then_some(value)
}

/// Is `pid` an always-on system service (or Interpres itself)?
fn ignored_player(pid: i32) -> bool {
    if pid == std::process::id() as i32 {
        return true;
    }
    super::macos::process_path(pid)
        .is_some_and(|path| IGNORED_PLAYERS.iter().any(|s| path.contains(s)))
}

/// Some(true) if any app (not a system speech service) has audio output running.
/// None when the OS has no per-process audio objects (macOS < 14.2).
fn app_output_running() -> Option<bool> {
    let size = property_size(SYSTEM_OBJECT, PROP_PROCESSES, SCOPE_GLOBAL).ok()?;
    let mut ids = vec![0 as AudioObjectId; size as usize / std::mem::size_of::<AudioObjectId>()];
    let addr = PropertyAddress {
        selector: PROP_PROCESSES,
        scope: SCOPE_GLOBAL,
        element: ELEMENT_MAIN,
    };
    let mut got = size;
    let st = unsafe {
        AudioObjectGetPropertyData(
            SYSTEM_OBJECT,
            &addr,
            0,
            ptr::null(),
            &mut got,
            ids.as_mut_ptr() as *mut c_void,
        )
    };
    if st != 0 {
        return None;
    }
    ids.truncate(got as usize / std::mem::size_of::<AudioObjectId>());
    Some(ids.into_iter().any(|p| {
        read_u32(p, PROP_PROCESS_RUNNING_OUTPUT).is_some_and(|r| r != 0)
            && read_u32(p, PROP_PROCESS_PID).is_some_and(|pid| !ignored_player(pid as i32))
    }))
}

/// Remembers when Live Captions' text last changed.
struct CaptionWatch {
    last_text: Option<String>,
    changed_at: Option<Instant>,
    read_at: Option<Instant>,
}

impl CaptionWatch {
    fn new() -> Self {
        Self {
            last_text: None,
            changed_at: None,
            read_at: None,
        }
    }

    /// Read Live Captions (at most every `CAPTIONS_EVERY`) and say whether it is
    /// captioning right now. Never prompts for Accessibility from this thread.
    fn active(&mut self) -> bool {
        let due = self.read_at.map_or(true, |t| t.elapsed() >= CAPTIONS_EVERY);
        if due && super::macos::is_accessibility_trusted() {
            self.read_at = Some(Instant::now());
            let text = super::poll_capture()
                .surface_text
                .filter(|t| !t.trim().is_empty());
            if text.is_some() && text != self.last_text {
                // The first reading is only a baseline (text left over from earlier).
                if self.last_text.is_some() {
                    self.changed_at = Some(Instant::now());
                }
                self.last_text = text;
            }
        }
        self.changed_at.is_some_and(|t| t.elapsed() <= CAPTIONS_RECENT)
    }

    /// Nothing playing: forget the baseline so stale text is not "new" later.
    fn idle(&mut self) {
        self.read_at = None;
    }
}

/// Shared between the meter thread and the UI.
pub struct SoundLevel {
    /// Loudest reading since the UI last took it (f32 bits; non-negative floats order like u32).
    peak_bits: AtomicU32,
    /// `epoch`-relative ms of the last successful read (0 = never).
    ok_at_ms: AtomicU64,
    stop: AtomicBool,
    epoch: Instant,
}

impl SoundLevel {
    /// Loudest reading since the previous call (resets to 0).
    pub fn take_peak(&self) -> f32 {
        f32::from_bits(self.peak_bits.swap(0, Ordering::SeqCst))
    }

    /// True if the devices answered within `within` — otherwise silence is not trustworthy.
    pub fn healthy(&self, within: Duration) -> bool {
        let ok = self.ok_at_ms.load(Ordering::SeqCst);
        ok != 0
            && (self.epoch.elapsed().as_millis() as u64).saturating_sub(ok)
                <= within.as_millis() as u64
    }

    /// Time since the meter was started (the first reading takes a moment).
    pub fn age(&self) -> Duration {
        self.epoch.elapsed()
    }

    /// Ask the meter thread to exit.
    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

/// Start watching speaker activity in the background until `SoundLevel::stop`.
pub fn spawn_sound_meter() -> Arc<SoundLevel> {
    let level = Arc::new(SoundLevel {
        peak_bits: AtomicU32::new(0),
        ok_at_ms: AtomicU64::new(0),
        stop: AtomicBool::new(false),
        epoch: Instant::now(),
    });
    let shared = level.clone();
    thread::spawn(move || meter_loop(&shared));
    level
}

fn meter_loop(level: &SoundLevel) {
    let mut devices: Option<Vec<AudioObjectId>> = None;
    let mut refreshed = Instant::now();
    let mut last_error = String::new();
    let mut captions = CaptionWatch::new();
    let mut per_app = true;
    while !level.stop.load(Ordering::SeqCst) {
        let playing = if per_app {
            match app_output_running() {
                Some(p) => Some(p),
                None => {
                    crate::debuglog::log("sound meter: no per-app audio info; watching output devices");
                    per_app = false;
                    continue;
                }
            }
        } else {
            if devices.is_none() || refreshed.elapsed() >= REFRESH_DEVICES_EVERY {
                refreshed = Instant::now();
                match output_devices() {
                    Ok(d) => devices = Some(d),
                    Err(e) => {
                        if e != last_error {
                            crate::debuglog::log(&format!("sound meter: {e}"));
                            last_error = e;
                        }
                        devices = None;
                    }
                }
            }
            devices.as_ref().and_then(|list| {
                let states: Vec<bool> = list.iter().filter_map(|&d| device_running(d)).collect();
                if states.is_empty() && !list.is_empty() {
                    None
                } else {
                    Some(states.contains(&true))
                }
            })
        };
        match playing {
            Some(on) => {
                let heard = if on {
                    captions.active()
                } else {
                    captions.idle();
                    false
                };
                let peak: f32 = if heard { 1.0 } else { 0.0 };
                level.peak_bits.fetch_max(peak.to_bits(), Ordering::SeqCst);
                let now = (level.epoch.elapsed().as_millis() as u64).max(1);
                level.ok_at_ms.store(now, Ordering::SeqCst);
            }
            // Every device failed (unplugged mid-read): re-list next round.
            None => devices = None,
        }
        thread::sleep(SAMPLE_EVERY);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fourcc_matches_core_audio_constants() {
        assert_eq!(SCOPE_GLOBAL, 0x676c_6f62);
        assert_eq!(PROP_RUNNING_SOMEWHERE, 0x676f_6e65);
    }

    #[test]
    fn meter_answers() {
        let level = spawn_sound_meter();
        thread::sleep(Duration::from_millis(800));
        assert!(level.healthy(Duration::from_secs(1)), "Core Audio answered");
        level.stop();
    }

    #[test]
    fn interpres_itself_is_ignored() {
        assert!(ignored_player(std::process::id() as i32));
    }

    /// Prints which apps have audio output running:
    /// `cargo test lists_playing_apps -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn lists_playing_apps() {
        println!("app output running (system services ignored): {:?}", app_output_running());
    }
}
