//! Native desktop UI entry.
//! - macOS: AppKit via system clang (`native/macos/`)
//! - Windows: Win32 via hand-written FFI (`gui_win.rs`)
//! Shared CaptureEngine core; OS-specific chrome only.

#[cfg(target_os = "macos")]
use crate::buffer::same_or_refinement;
#[cfg(target_os = "macos")]
use crate::config::Config;
#[cfg(target_os = "macos")]
use crate::engine::{CaptureEngine, EngineEvent};
#[cfg(target_os = "macos")]
use crate::probe;
#[cfg(target_os = "macos")]
use crate::ui_labels::{folder_label, remember_toggle_status, session_footer};
#[cfg(target_os = "macos")]
use std::ffi::{CStr, CString};
#[cfg(target_os = "macos")]
use std::os::raw::{c_char, c_int, c_void};
#[cfg(target_os = "macos")]
use std::path::{Path, PathBuf};
#[cfg(target_os = "macos")]
use std::sync::mpsc::{Receiver, TryRecvError};
#[cfg(target_os = "macos")]
use std::sync::{Arc, Mutex};
#[cfg(target_os = "macos")]
use std::thread;
#[cfg(target_os = "macos")]
use std::time::Duration;

#[cfg(target_os = "macos")]
#[repr(C)]
struct InterpresGuiCallbacks {
    user: *mut c_void,
    on_start: Option<extern "C" fn(*mut c_void)>,
    on_stop: Option<extern "C" fn(*mut c_void)>,
    on_remember: Option<extern "C" fn(*mut c_void, c_int)>,
    on_choose_folder: Option<extern "C" fn(*mut c_void)>,
    on_open_folder: Option<extern "C" fn(*mut c_void)>,
    on_check: Option<extern "C" fn(*mut c_void)>,
    on_debug: Option<extern "C" fn(*mut c_void, c_int)>,
    on_ready: Option<extern "C" fn(*mut c_void)>,
}

#[cfg(target_os = "macos")]
extern "C" {
    fn interpres_gui_main(callbacks: InterpresGuiCallbacks) -> c_int;
    fn interpres_gui_set_status(text: *const c_char);
    fn interpres_gui_set_live_text(text: *const c_char);
    fn interpres_gui_append_history(line: *const c_char);
    fn interpres_gui_set_folder(path: *const c_char);
    fn interpres_gui_set_remember(on: c_int);
    fn interpres_gui_set_debug(on: c_int);
    fn interpres_gui_set_listening(on: c_int);
    fn interpres_gui_set_session_file(path: *const c_char);
    fn interpres_gui_pick_folder(buf: *mut c_char, buflen: c_int) -> c_int;
}

#[cfg(target_os = "macos")]
struct GuiState {
    engine: CaptureEngine,
}

/// Run the native GUI (blocks until the window is closed).
pub fn run_native_gui() -> i32 {
    #[cfg(target_os = "macos")]
    {
        return run_macos_gui();
    }
    #[cfg(windows)]
    {
        return crate::gui_win::run_windows_gui();
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        eprintln!("Native window UI is available on Windows and macOS.");
        eprintln!("Use: interpres run");
        1
    }
}

#[cfg(target_os = "macos")]
fn run_macos_gui() -> i32 {
    let mut cfg = Config::load();
    let fs = cfg.transcript_folder.to_string_lossy();
    if fs.contains("/var/folders/") || fs.contains("/tmp") {
        cfg.transcript_folder = crate::config::default_transcript_folder();
        let _ = cfg.save();
    }

    crate::debuglog::init_from_config(cfg.debug, &cfg.transcript_folder);
    crate::debuglog::log("gui open");

    let (engine, rx) = CaptureEngine::new(&cfg);

    let last_hist = Arc::new(Mutex::new(String::new()));
    let last_hist_pump = last_hist.clone();
    thread::spawn(move || pump_events(rx, last_hist_pump));

    let state = Arc::new(Mutex::new(GuiState { engine }));
    let raw = Arc::into_raw(state) as *mut c_void;
    let cbs = InterpresGuiCallbacks {
        user: raw,
        on_start: Some(cb_start),
        on_stop: Some(cb_stop),
        on_remember: Some(cb_remember),
        on_choose_folder: Some(cb_choose_folder),
        on_open_folder: Some(cb_open_folder),
        on_check: Some(cb_check),
        on_debug: Some(cb_debug),
        on_ready: Some(cb_ready),
    };

    let code = unsafe { interpres_gui_main(cbs) };

    let state = unsafe { Arc::from_raw(raw as *const Mutex<GuiState>) };
    if let Ok(st) = state.lock() {
        st.engine.stop();
    }
    drop(state);
    thread::sleep(Duration::from_millis(100));
    code
}

#[cfg(target_os = "macos")]
fn pump_events(rx: Receiver<EngineEvent>, last_hist: Arc<Mutex<String>>) {
    loop {
        match rx.try_recv() {
            Ok(ev) => apply_event(ev, &last_hist),
            Err(TryRecvError::Empty) => thread::sleep(Duration::from_millis(30)),
            Err(TryRecvError::Disconnected) => break,
        }
    }
}

#[cfg(target_os = "macos")]
fn apply_event(ev: EngineEvent, last_hist: &Arc<Mutex<String>>) {
    match ev {
        EngineEvent::Status(s) => set_status(&s),
        EngineEvent::Live(s) => set_live(&s),
        EngineEvent::Final(s) => {
            set_live(&s);
            let mut skip = false;
            if let Ok(mut last) = last_hist.lock() {
                if !last.is_empty() && same_or_refinement(&last, &s) {
                    if s.len() >= last.len() {
                        *last = s.clone();
                    }
                    skip = true;
                } else {
                    *last = s.clone();
                }
            }
            if !skip {
                append_history(&s);
            }
        }
        EngineEvent::Revised(s) => {
            // OS polished an existing family — update live; history keeps one line (Mac append skips same family).
            set_live(&s);
            if let Ok(mut last) = last_hist.lock() {
                *last = s.clone();
            }
        }
        EngineEvent::Error(s) => set_status(&format!("⚠ {s}")),
        EngineEvent::SessionFile(Some(p)) => {
            set_session(Some(&p.display().to_string()));
            if let Some(parent) = p.parent() {
                set_folder_label(parent);
            }
            set_remember_ui(true);
        }
        EngineEvent::SessionFile(None) => {
            set_session(None);
        }
        EngineEvent::Listening(on) => set_listening(on),
    }
}

#[cfg(target_os = "macos")]
fn c_string(s: &str) -> CString {
    CString::new(s.replace('\0', "")).unwrap_or_else(|_| CString::new("").unwrap())
}

#[cfg(target_os = "macos")]
fn set_status(s: &str) {
    let c = c_string(s);
    unsafe { interpres_gui_set_status(c.as_ptr()) };
}
#[cfg(target_os = "macos")]
fn set_live(s: &str) {
    let c = c_string(s);
    unsafe { interpres_gui_set_live_text(c.as_ptr()) };
}
#[cfg(target_os = "macos")]
fn append_history(s: &str) {
    let c = c_string(s);
    unsafe { interpres_gui_append_history(c.as_ptr()) };
}
#[cfg(target_os = "macos")]
fn set_folder_label(path: &Path) {
    let display = folder_label(path);
    let path_only = display
        .strip_prefix("Folder: ")
        .unwrap_or(display.as_str());
    let c = c_string(path_only);
    unsafe { interpres_gui_set_folder(c.as_ptr()) };
}
#[cfg(target_os = "macos")]
fn set_remember_ui(on: bool) {
    unsafe { interpres_gui_set_remember(if on { 1 } else { 0 }) };
}
#[cfg(target_os = "macos")]
fn set_debug_ui(on: bool) {
    unsafe { interpres_gui_set_debug(if on { 1 } else { 0 }) };
}
#[cfg(target_os = "macos")]
fn set_listening(on: bool) {
    unsafe { interpres_gui_set_listening(if on { 1 } else { 0 }) };
}
#[cfg(target_os = "macos")]
fn set_session(path: Option<&str>) {
    let footer = match path {
        Some(p) if !p.is_empty() => session_footer(Some(Path::new(p))),
        _ => session_footer(None),
    };
    let path_only = footer
        .strip_prefix("Saving to file: ")
        .unwrap_or("");
    let c = c_string(path_only);
    unsafe { interpres_gui_set_session_file(c.as_ptr()) };
}

#[cfg(target_os = "macos")]
fn state_from(user: *mut c_void) -> Arc<Mutex<GuiState>> {
    unsafe {
        Arc::increment_strong_count(user as *const Mutex<GuiState>);
        Arc::from_raw(user as *const Mutex<GuiState>)
    }
}

#[cfg(target_os = "macos")]
extern "C" fn cb_ready(user: *mut c_void) {
    let state = state_from(user);
    let (folder, remember) = {
        let st = state.lock().unwrap();
        (st.engine.folder(), st.engine.remember())
    };
    let cfg = Config::load();
    set_folder_label(&folder);
    set_remember_ui(remember);
    set_debug_ui(cfg.debug);
    set_session(None);
    set_status(
        "Turn on Live Captions (System Settings → Accessibility), then press Start listening.",
    );
    crate::debuglog::set_folder(&folder);
    crate::debuglog::log(&format!(
        "ui ready folder={} remember={} debug={}",
        folder.display(),
        remember,
        cfg.debug
    ));
}

#[cfg(target_os = "macos")]
extern "C" fn cb_start(user: *mut c_void) {
    let state = state_from(user);
    {
        let st = state.lock().unwrap();
        set_folder_label(&st.engine.folder());
        set_remember_ui(st.engine.remember());
        st.engine.start();
    }
}

#[cfg(target_os = "macos")]
extern "C" fn cb_stop(user: *mut c_void) {
    let state = state_from(user);
    {
        let st = state.lock().unwrap();
        st.engine.stop();
    }
}

#[cfg(target_os = "macos")]
extern "C" fn cb_remember(user: *mut c_void, value: c_int) {
    let state = state_from(user);
    let on = value != 0;
    let folder = {
        let st = state.lock().unwrap();
        st.engine.set_remember(on);
        st.engine.folder()
    };
    set_remember_ui(on);
    set_folder_label(&folder);
    set_status(&remember_toggle_status(on, &folder));
}

#[cfg(target_os = "macos")]
extern "C" fn cb_debug(user: *mut c_void, value: c_int) {
    let state = state_from(user);
    let on = value != 0;
    let folder = {
        let st = state.lock().unwrap();
        st.engine.folder()
    };
    crate::debuglog::set_folder(&folder);
    crate::debuglog::set_enabled(on);
    let mut cfg = Config::load();
    cfg.debug = on;
    let _ = cfg.save();
    set_debug_ui(on);
    if on {
        let p = crate::debuglog::path_for_display();
        set_status(&format!(
            "Debug ON — logging to {} (same folder as transcripts)",
            p.display()
        ));
        crate::debuglog::log("debug enabled from UI");
    } else {
        set_status("Debug OFF.");
    }
}

#[cfg(target_os = "macos")]
extern "C" fn cb_choose_folder(user: *mut c_void) {
    let mut buf = vec![0i8; 4096];
    let ok = unsafe { interpres_gui_pick_folder(buf.as_mut_ptr(), buf.len() as c_int) };
    if ok == 0 {
        return;
    }
    let path = unsafe { CStr::from_ptr(buf.as_ptr()) }
        .to_string_lossy()
        .to_string();
    if path.is_empty() {
        return;
    }
    let state = state_from(user);
    {
        let st = state.lock().unwrap();
        st.engine.set_folder(PathBuf::from(&path));
    }
    crate::debuglog::set_folder(std::path::Path::new(&path));
    set_folder_label(std::path::Path::new(&path));
    set_status(&format!("Transcript folder set to {path}"));
}

#[cfg(target_os = "macos")]
extern "C" fn cb_open_folder(user: *mut c_void) {
    let state = state_from(user);
    let folder = {
        let st = state.lock().unwrap();
        st.engine.folder()
    };
    let _ = std::fs::create_dir_all(&folder);
    let _ = std::process::Command::new("open").arg(&folder).status();
}

#[cfg(target_os = "macos")]
extern "C" fn cb_check(_user: *mut c_void) {
    let _ = crate::platform::macos::request_accessibility_prompt();
    let report = probe::run_probe();
    if report.exit_code == probe::EXIT_PERMISSION {
        set_status(
            "Accessibility is OFF. Enable Terminal or Interpres in System Settings → Privacy & Security → Accessibility, then Check again.",
        );
        crate::platform::macos::open_accessibility_settings();
    } else if report.exit_code == probe::EXIT_LC_NOT_RUNNING {
        set_status(
            "Live Captions is not running. System Settings → Accessibility → Live Captions → ON, then Start listening.",
        );
    } else if report.exit_code == 0 {
        set_status("Check OK — Live Captions looks readable. Press Start listening.");
    } else {
        set_status(&format!(
            "Check finished (code {}). See details if captions stay empty.",
            report.exit_code
        ));
    }
}
