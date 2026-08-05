//! Native desktop UI entry (macOS AppKit via system clang). Zero crates.io deps.

use crate::buffer::same_or_refinement;
use crate::config::Config;
use crate::engine::{CaptureEngine, EngineEvent};
use crate::probe;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_void};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread;
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

struct GuiState {
    engine: CaptureEngine,
}

/// Run the native GUI (blocks until the window is closed).
pub fn run_native_gui() -> i32 {
    #[cfg(target_os = "macos")]
    {
        return run_macos_gui();
    }
    #[cfg(not(target_os = "macos"))]
    {
        eprintln!("Native window UI is available on macOS right now.");
        eprintln!(
            "On Windows, use: interpres run   (window UI when we build on your Win11 laptop)"
        );
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
    set_folder_label(&engine.folder());
    set_remember_ui(engine.remember());
    set_debug_ui(cfg.debug);
    set_status(
        "Turn on Live Captions (System Settings → Accessibility), then press Start listening.",
    );

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
            // Only append history when this is a new sentence family.
            let mut skip = false;
            if let Ok(mut last) = last_hist.lock() {
                if !last.is_empty() && same_or_refinement(&last, &s) {
                    // Keep the better text in memory but do not re-append.
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
        EngineEvent::Error(s) => set_status(&format!("⚠ {s}")),
        EngineEvent::SessionFile(Some(p)) => set_session(Some(&p.display().to_string())),
        EngineEvent::SessionFile(None) => set_session(None),
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
fn set_folder_label(path: &std::path::Path) {
    let c = c_string(&path.display().to_string());
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
    let c = c_string(path.unwrap_or(""));
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
extern "C" fn cb_start(user: *mut c_void) {
    let state = state_from(user);
    {
        let st = state.lock().unwrap();
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
    {
        let st = state.lock().unwrap();
        st.engine.set_remember(on);
    }
    set_remember_ui(on);
    if on {
        set_status(
            "Save to disk is ON. New sessions will create a dated file in your folder.",
        );
    } else {
        set_status("Save to disk is OFF. You can still watch captions live.");
    }
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
