//! macOS Live Captions text capture via Accessibility (hand-written FFI).

use super::detect::LiveCaptionsPresence;
use super::CaptureSnapshot;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_void};
use std::ptr;

// Minimal CoreFoundation / ApplicationServices bindings (system libs only).

#[repr(C)]
struct CfDictionaryKeyCallBacks {
    version: isize,
    retain: *const c_void,
    release: *const c_void,
    copy_description: *const c_void,
    equal: *const c_void,
    hash: *const c_void,
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRelease(cf: *const c_void);
    fn CFStringCreateWithCString(
        alloc: *const c_void,
        c_str: *const c_char,
        encoding: u32,
    ) -> *const c_void;
    fn CFStringGetCString(
        the_string: *const c_void,
        buffer: *mut c_char,
        buffer_size: isize,
        encoding: u32,
    ) -> u8;
    fn CFStringGetLength(the_string: *const c_void) -> isize;
    fn CFArrayGetCount(the_array: *const c_void) -> isize;
    fn CFArrayGetValueAtIndex(the_array: *const c_void, idx: isize) -> *const c_void;
    fn CFGetTypeID(cf: *const c_void) -> usize;
    fn CFStringGetTypeID() -> usize;
    fn CFAttributedStringGetTypeID() -> usize;
    fn CFAttributedStringGetString(astr: *const c_void) -> *const c_void;
    fn CFDictionaryCreate(
        allocator: *const c_void,
        keys: *const *const c_void,
        values: *const *const c_void,
        num_values: isize,
        key_call_backs: *const CfDictionaryKeyCallBacks,
        value_call_backs: *const CfDictionaryKeyCallBacks,
    ) -> *const c_void;
    static kCFTypeDictionaryKeyCallBacks: CfDictionaryKeyCallBacks;
    static kCFTypeDictionaryValueCallBacks: CfDictionaryKeyCallBacks;
    static kCFBooleanTrue: *const c_void;
}

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> u8;
    fn AXIsProcessTrustedWithOptions(options: *const c_void) -> u8;
    fn AXUIElementCreateApplication(pid: i32) -> *const c_void;
    fn AXUIElementCopyAttributeValue(
        element: *const c_void,
        attribute: *const c_void,
        value: *mut *const c_void,
    ) -> c_int;
    fn AXUIElementSetMessagingTimeout(element: *const c_void, timeout_secs: f32) -> c_int;
    fn AXUIElementCreateSystemWide() -> *const c_void;
}

const K_CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;
const K_AX_ERROR_SUCCESS: c_int = 0;

fn cfstr(s: &str) -> *const c_void {
    let c = CString::new(s).unwrap_or_default();
    unsafe { CFStringCreateWithCString(ptr::null(), c.as_ptr(), K_CF_STRING_ENCODING_UTF8) }
}

fn cfstring_to_rust(cf: *const c_void) -> Option<String> {
    if cf.is_null() {
        return None;
    }
    unsafe {
        let tid = CFGetTypeID(cf);
        let s_ref = if tid == CFStringGetTypeID() {
            cf
        } else if tid == CFAttributedStringGetTypeID() {
            let inner = CFAttributedStringGetString(cf);
            if inner.is_null() {
                return None;
            }
            inner
        } else {
            return None;
        };
        let len = CFStringGetLength(s_ref);
        if len <= 0 {
            return None;
        }
        let mut buf = vec![0i8; (len as usize) * 4 + 16];
        let ok = CFStringGetCString(
            s_ref,
            buf.as_mut_ptr(),
            buf.len() as isize,
            K_CF_STRING_ENCODING_UTF8,
        );
        if ok == 0 {
            return None;
        }
        CStr::from_ptr(buf.as_ptr())
            .to_str()
            .ok()
            .map(|s| s.to_string())
    }
}

fn ax_copy(element: *const c_void, attr: &str) -> Option<*const c_void> {
    let attr_cf = cfstr(attr);
    if attr_cf.is_null() {
        return None;
    }
    let mut value: *const c_void = ptr::null();
    let err = unsafe { AXUIElementCopyAttributeValue(element, attr_cf, &mut value) };
    unsafe { CFRelease(attr_cf) };
    if err != K_AX_ERROR_SUCCESS || value.is_null() {
        return None;
    }
    Some(value)
}

/// Live Captions marks each caption line with this subrole (macOS 15+: an
/// `AXStaticText` inside the "Captions" list of the `AXLiveCaptionsWindow`).
const CAPTION_SUBROLE: &str = "AXCaptionsText";

fn ax_string(element: *const c_void, attr: &str) -> Option<String> {
    let v = ax_copy(element, attr)?;
    let s = cfstring_to_rust(v);
    unsafe { CFRelease(v) };
    s
}

/// Static texts under `element`, in screen order. `captions_only` keeps just the
/// caption lines; otherwise any static text (older layouts), minus window chrome.
fn collect_texts(element: *const c_void, depth: u32, captions_only: bool, out: &mut Vec<String>) {
    if element.is_null() || depth > 14 {
        return;
    }
    let role = ax_string(element, "AXRole").unwrap_or_default();
    // Buttons ("Pause Live Captions"), menus and their items are never captions.
    if matches!(role.as_str(), "AXButton" | "AXMenuBar" | "AXMenu" | "AXMenuItem" | "AXMenuButton") {
        return;
    }
    if role == "AXStaticText" {
        let is_caption = ax_string(element, "AXSubrole").as_deref() == Some(CAPTION_SUBROLE);
        if is_caption || !captions_only {
            if let Some(v) = ax_string(element, "AXValue") {
                for line in v.lines().map(str::trim).filter(|l| !l.is_empty()) {
                    if is_caption || !crate::buffer::is_junk_line(line) {
                        out.push(line.to_string());
                    }
                }
            }
        }
        return;
    }
    if let Some(children) = ax_copy(element, "AXChildren") {
        unsafe {
            let n = CFArrayGetCount(children);
            for i in 0..n.min(200) {
                collect_texts(CFArrayGetValueAtIndex(children, i), depth + 1, captions_only, out);
            }
            CFRelease(children);
        }
    }
}

/// Caption lines in the order Live Captions shows them, read only from its windows
/// (never the menu bar: its Recent Items list held media file names that were saved as
/// captions). `None` when no caption window is up.
fn read_caption_lines(app: *const c_void) -> Option<Vec<String>> {
    let windows = ax_copy(app, "AXWindows")?;
    let mut captions = Vec::new();
    let mut fallback = Vec::new();
    unsafe {
        let n = CFArrayGetCount(windows);
        for i in 0..n.min(12) {
            let w = CFArrayGetValueAtIndex(windows, i);
            collect_texts(w, 0, true, &mut captions);
        }
        if captions.is_empty() {
            for i in 0..n.min(12) {
                let w = CFArrayGetValueAtIndex(windows, i);
                collect_texts(w, 0, false, &mut fallback);
            }
        }
        CFRelease(windows);
    }
    let lines = if captions.is_empty() { fallback } else { captions };
    (!lines.is_empty()).then_some(lines)
}

extern "C" {
    // libproc + libc (libSystem, always linked).
    fn proc_listallpids(buffer: *mut c_void, buffersize: c_int) -> c_int;
    fn proc_pidpath(pid: c_int, buffer: *mut c_void, buffersize: u32) -> c_int;
    fn kill(pid: c_int, sig: c_int) -> c_int;
}

const PROC_PIDPATHINFO_MAXSIZE: usize = 4096;
const SIGTERM: c_int = 15;
/// Longest a single Accessibility call may block (default is ~6 s). A busy Live
/// Captions must not stall the capture loop (Windows v0.3.0 uses 2.5 s for UIA).
const AX_CALL_TIMEOUT_SECS: f32 = 2.5;

/// Executable path of `pid`, if it is still running.
pub(crate) fn process_path(pid: i32) -> Option<String> {
    let mut buf = vec![0u8; PROC_PIDPATHINFO_MAXSIZE];
    let n = unsafe {
        proc_pidpath(pid, buf.as_mut_ptr() as *mut c_void, buf.len() as u32)
    };
    if n <= 0 {
        return None;
    }
    buf.truncate(n as usize);
    String::from_utf8(buf).ok()
}

fn is_live_captions_path(path: &str) -> bool {
    path.ends_with("/Live Captions.app/Contents/MacOS/Live Captions")
        || path.ends_with("/LiveTranscriptionAgent")
}

/// Live Captions' PID, found in-process (no `pgrep` per poll). The last PID is cached
/// and re-checked by path, so a steady poll costs one syscall.
pub fn live_captions_pid() -> Option<i32> {
    use std::sync::atomic::{AtomicI32, Ordering};
    static LAST: AtomicI32 = AtomicI32::new(0);

    let last = LAST.load(Ordering::Relaxed);
    if last > 0 && process_path(last).is_some_and(|p| is_live_captions_path(&p)) {
        return Some(last);
    }
    let mut pids = vec![0i32; 4096];
    let n = unsafe {
        proc_listallpids(
            pids.as_mut_ptr() as *mut c_void,
            (pids.len() * std::mem::size_of::<i32>()) as c_int,
        )
    };
    if n <= 0 {
        return None;
    }
    pids.truncate((n as usize).min(pids.len()));
    let found = pids
        .into_iter()
        .filter(|&p| p > 0)
        .find(|&p| process_path(p).is_some_and(|path| is_live_captions_path(&path)));
    LAST.store(found.unwrap_or(0), Ordering::Relaxed);
    found
}

/// Cap every Accessibility call (set on the system-wide element, it applies to all
/// elements this process reads).
fn set_ax_timeout_once() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| unsafe {
        let sys = AXUIElementCreateSystemWide();
        if !sys.is_null() {
            // Kept for the life of the process so the setting stays in force.
            AXUIElementSetMessagingTimeout(sys, AX_CALL_TIMEOUT_SECS);
        }
    });
}

fn pid_for_live_captions() -> Option<i32> {
    live_captions_pid()
}

/// Open System Settings → Accessibility → Live Captions. macOS does not let other apps
/// switch Live Captions on, so the user flips the switch there.
pub fn open_live_captions_settings() -> std::io::Result<()> {
    let status = std::process::Command::new("open")
        .arg("x-apple.systempreferences:com.apple.Accessibility-Settings.extension?LiveCaptions")
        .status()?;
    if status.success() {
        return Ok(());
    }
    // Older layouts: the Accessibility pane itself.
    std::process::Command::new("open")
        .arg("x-apple.systempreferences:com.apple.preference.universalaccess")
        .status()
        .map(|_| ())
}

/// Quit Live Captions so macOS starts it again (it stays on in Settings). If it has
/// not come back after a few seconds, open its settings so the user can switch it on.
pub fn restart_live_captions() -> std::io::Result<()> {
    let Some(pid) = live_captions_pid() else {
        return open_live_captions_settings();
    };
    unsafe {
        kill(pid, SIGTERM);
    }
    for _ in 0..12 {
        std::thread::sleep(std::time::Duration::from_millis(500));
        if live_captions_pid().is_some_and(|p| p != pid) {
            return Ok(());
        }
    }
    crate::debuglog::log("Live Captions did not restart by itself — opening its settings");
    open_live_captions_settings()
}

/// Ask macOS to show the Accessibility permission dialog at most **once per process**.
/// Calling AXTrustedCheckOptionPrompt on every poll re-pops the dialog forever when untrusted.
pub fn request_accessibility_prompt() -> bool {
    use std::sync::atomic::{AtomicBool, Ordering};
    static PROMPTED: AtomicBool = AtomicBool::new(false);

    if unsafe { AXIsProcessTrusted() } != 0 {
        return true;
    }
    // Only show the system sheet once; later checks are silent until the user enables + restarts.
    if PROMPTED.swap(true, Ordering::SeqCst) {
        return unsafe { AXIsProcessTrusted() != 0 };
    }
    unsafe {
        let key = cfstr("AXTrustedCheckOptionPrompt");
        if key.is_null() {
            return AXIsProcessTrusted() != 0;
        }
        let keys = [key];
        let values = [kCFBooleanTrue as *const c_void];
        let dict = CFDictionaryCreate(
            ptr::null(),
            keys.as_ptr() as *const *const c_void,
            values.as_ptr(),
            1,
            &kCFTypeDictionaryKeyCallBacks,
            &kCFTypeDictionaryValueCallBacks,
        );
        let trusted = if dict.is_null() {
            AXIsProcessTrusted() != 0
        } else {
            let t = AXIsProcessTrustedWithOptions(dict) != 0;
            CFRelease(dict);
            t
        };
        CFRelease(key);
        trusted
    }
}

pub fn is_accessibility_trusted() -> bool {
    unsafe { AXIsProcessTrusted() != 0 }
}

/// Open System Settings to the Accessibility privacy pane (best-effort).
pub fn open_accessibility_settings() {
    let urls = [
        "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility",
        "x-apple.systempreferences:com.apple.settings.PrivacySecurity.extension?Privacy_Accessibility",
    ];
    for u in urls {
        let _ = std::process::Command::new("open").arg(u).status();
    }
}

pub fn poll_text(presence: LiveCaptionsPresence) -> CaptureSnapshot {
    // Prompt at most once per process when untrusted (see request_accessibility_prompt).
    let trusted = request_accessibility_prompt();

    if !trusted {
        return CaptureSnapshot {
            process_running: true,
            detail: presence.detail,
            surface_text: None,
            error: Some(
                "macOS Accessibility is OFF for this copy of Interpres. \
                 System Settings → Privacy & Security → Accessibility → enable \
                 “Interpres” (the .app) or the “interpres” binary you launched. \
                 After a rebuild, toggle it OFF then ON again, fully Quit Interpres, and reopen. \
                 Live Captions is running but macOS will not let us read its text."
                    .into(),
            ),
        };
    }

    let Some(pid) = pid_for_live_captions() else {
        return CaptureSnapshot {
            process_running: true,
            detail: presence.detail,
            surface_text: None,
            error: Some("could not resolve Live Captions PID".into()),
        };
    };

    set_ax_timeout_once();
    let app = unsafe { AXUIElementCreateApplication(pid) };
    if app.is_null() {
        return CaptureSnapshot {
            process_running: true,
            detail: presence.detail,
            surface_text: None,
            error: Some("AXUIElementCreateApplication failed".into()),
        };
    }

    let lines = read_caption_lines(app);
    unsafe { CFRelease(app) };
    let surface = lines.map(|l| l.join("\n"));

    // Log only when what Live Captions shows changes (was every poll: 41 MB in 3 h).
    {
        use std::sync::Mutex;
        static LAST: Mutex<Option<String>> = Mutex::new(None);
        if let Ok(mut last) = LAST.lock() {
            if *last != surface {
                let count = surface.as_ref().map_or(0, |s| s.lines().count());
                crate::debuglog::log(&format!(
                    "macos captions pid={pid} lines={count} chars={} last={:?}",
                    surface.as_ref().map_or(0, |s| s.chars().count()),
                    surface
                        .as_ref()
                        .and_then(|s| s.lines().last())
                        .map(|l| l.chars().take(100).collect::<String>())
                        .unwrap_or_default()
                ));
                *last = surface.clone();
            }
        }
    }

    match surface {
        // AX trusted + process up, but no caption window (nothing heard yet, or hidden).
        // Engine treats surface_text=None as empty ticks (clear live); probe stays exit 0.
        None => CaptureSnapshot {
            process_running: true,
            detail: format!(
                "{}; pid={pid}; ax_trusted=true; no_caption_surface",
                presence.detail
            ),
            surface_text: None,
            error: None,
        },
        Some(text) => CaptureSnapshot {
            process_running: true,
            detail: format!("{}; pid={pid}; ax_trusted=true; surface_ok", presence.detail),
            surface_text: Some(text),
            error: None,
        },
    }
}

/// Extra diagnostics for `interpres diagnose`.
pub fn diagnose_lines() -> Vec<String> {
    let mut lines = Vec::new();
    let trusted = is_accessibility_trusted();
    lines.push(format!("ax_trusted={trusted}"));
    if !trusted {
        let _ = request_accessibility_prompt();
        lines.push("ax_prompt_requested=true".into());
        lines.push(
            "Enable Accessibility for the host app (Terminal / Interpres), then re-run diagnose."
                .into(),
        );
        return lines;
    }
    let pid = pid_for_live_captions();
    lines.push(format!("live_captions_pid={pid:?}"));
    let Some(pid) = pid else {
        lines.push("Live Captions process not found — turn Live Captions on.".into());
        return lines;
    };
    set_ax_timeout_once();
    let app = unsafe { AXUIElementCreateApplication(pid) };
    if app.is_null() {
        lines.push("AXUIElementCreateApplication failed".into());
        return lines;
    }
    let windows = ax_copy(app, "AXWindows")
        .map(|w| {
            let n = unsafe { CFArrayGetCount(w) };
            unsafe { CFRelease(w) };
            n
        })
        .unwrap_or(0);
    lines.push(format!("ax_windows={windows}"));
    match read_caption_lines(app) {
        Some(captions) => {
            lines.push(format!("caption_lines={}", captions.len()));
            for (i, c) in captions.iter().enumerate().rev().take(5).rev() {
                let preview: String = c.chars().take(120).collect();
                lines.push(format!("caption[{i}] {preview}"));
            }
        }
        None => lines.push(
            "No caption lines. Live Captions shows its window once it hears speech — play something and run diagnose again."
                .into(),
        ),
    }
    unsafe { CFRelease(app) };
    lines
}

#[cfg(test)]
mod tree_tests {
    use super::*;

    fn dump(el: *const c_void, depth: usize) {
        if depth > 14 {
            return;
        }
        let get = |a: &str| {
            ax_copy(el, a).and_then(|v| {
                let s = cfstring_to_rust(v);
                unsafe { CFRelease(v) };
                s
            })
        };
        let role = get("AXRole").unwrap_or_default();
        let sub = get("AXSubrole").unwrap_or_default();
        let id = get("AXIdentifier").unwrap_or_default();
        let desc = get("AXDescription").unwrap_or_default();
        let title = get("AXTitle").unwrap_or_default();
        let val: String = get("AXValue").unwrap_or_default().chars().take(70).collect();
        println!(
            "{}{role} sub={sub} id={id} desc={desc:?} title={title:?} value={val:?}",
            "  ".repeat(depth)
        );
        if let Some(children) = ax_copy(el, "AXChildren") {
            unsafe {
                for i in 0..CFArrayGetCount(children) {
                    dump(CFArrayGetValueAtIndex(children, i), depth + 1);
                }
                CFRelease(children);
            }
        }
    }

    /// `cargo test --lib dump_ax_tree -- --ignored --nocapture` with Live Captions on.
    #[test]
    #[ignore]
    fn dump_ax_tree() {
        let pid = live_captions_pid().expect("Live Captions running");
        let app = unsafe { AXUIElementCreateApplication(pid) };
        dump(app, 0);
        if let Some(f) = ax_copy(app, "AXFocusedUIElement") {
            println!("--- focused:");
            dump(f, 0);
        }
    }
}
