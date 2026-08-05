//! Local debug log (pure std).
//!
//! Enable with the UI **Debug** button, `INTERPRES_DEBUG=1`, or `debug=true` in settings.
//! When a transcript folder is set, logs go there automatically (same place as .txt sessions).

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

struct DebugState {
    enabled: bool,
    /// Directory for log files (transcript folder preferred).
    folder: Option<PathBuf>,
    /// Optional per-session file name stem (e.g. session stamp).
    session_stem: Option<String>,
}

static STATE: Mutex<DebugState> = Mutex::new(DebugState {
    enabled: false,
    folder: None,
    session_stem: None,
});

fn env_wants_debug() -> bool {
    std::env::var_os("INTERPRES_DEBUG").is_some_and(|v| {
        let s = v.to_string_lossy();
        s != "0" && s != "false" && s != "off"
    })
}

/// Call once at startup from config.
pub fn init_from_config(debug: bool, folder: &Path) {
    let mut g = STATE.lock().unwrap_or_else(|e| e.into_inner());
    g.enabled = debug || env_wants_debug();
    g.folder = Some(folder.to_path_buf());
}

pub fn set_enabled(on: bool) {
    let mut g = STATE.lock().unwrap_or_else(|e| e.into_inner());
    g.enabled = on || env_wants_debug();
    if g.enabled {
        drop(g);
        log("debug logging ON");
    }
}

pub fn is_enabled() -> bool {
    STATE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .enabled
        || env_wants_debug()
}

pub fn set_folder(folder: &Path) {
    let mut g = STATE.lock().unwrap_or_else(|e| e.into_inner());
    g.folder = Some(folder.to_path_buf());
}

pub fn set_session_stem(stem: Option<String>) {
    let mut g = STATE.lock().unwrap_or_else(|e| e.into_inner());
    g.session_stem = stem;
}

fn fallback_folder() -> PathBuf {
    if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
        return PathBuf::from(home)
            .join("Documents")
            .join("Interpres Transcripts");
    }
    PathBuf::from("Interpres Transcripts")
}

/// Primary rolling log in the transcript folder.
pub fn log_path() -> PathBuf {
    let g = STATE.lock().unwrap_or_else(|e| e.into_inner());
    let folder = g.folder.clone().unwrap_or_else(fallback_folder);
    if let Some(ref stem) = g.session_stem {
        return folder.join(format!("{stem}.debug.log"));
    }
    folder.join("interpres-debug.log")
}

/// Append one debug line when enabled.
pub fn log(msg: &str) {
    if !is_enabled() {
        return;
    }
    let path = log_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "[{ts}] {msg}");
        let _ = f.flush();
    }
}

pub fn path_for_display() -> PathBuf {
    log_path()
}
