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
    fn AXUIElementCopyAttributeNames(
        element: *const c_void,
        names: *mut *const c_void,
    ) -> c_int;
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

fn push_text(s: String, out: &mut Vec<String>) {
    let t = s.trim().to_string();
    if crate::buffer::is_junk_line(&t) {
        return;
    }
    out.push(t);
}

fn collect_strings(element: *const c_void, depth: u32, out: &mut Vec<String>) {
    if element.is_null() || depth > 16 {
        return;
    }
    for attr in [
        "AXValue",
        "AXTitle",
        "AXDescription",
        "AXLabel",
        "AXHelp",
        "AXSelectedText",
    ] {
        if let Some(v) = ax_copy(element, attr) {
            if let Some(s) = cfstring_to_rust(v) {
                push_text(s, out);
            }
            unsafe { CFRelease(v) };
        }
    }

    // Some UIs expose text only via attribute names walk
    if depth <= 2 {
        let mut names: *const c_void = ptr::null();
        let err = unsafe { AXUIElementCopyAttributeNames(element, &mut names) };
        if err == K_AX_ERROR_SUCCESS && !names.is_null() {
            unsafe {
                let n = CFArrayGetCount(names);
                for i in 0..n.min(30) {
                    let name_cf = CFArrayGetValueAtIndex(names, i);
                    if let Some(name) = cfstring_to_rust(name_cf) {
                        if name.contains("Value")
                            || name.contains("Title")
                            || name.contains("Description")
                            || name.contains("Caption")
                            || name.contains("Text")
                        {
                            if let Some(v) = ax_copy(element, &name) {
                                if let Some(s) = cfstring_to_rust(v) {
                                    push_text(s, out);
                                }
                                CFRelease(v);
                            }
                        }
                    }
                }
                CFRelease(names);
            }
        }
    }

    if let Some(children) = ax_copy(element, "AXChildren") {
        unsafe {
            let n = CFArrayGetCount(children);
            for i in 0..n.min(80) {
                let child = CFArrayGetValueAtIndex(children, i);
                collect_strings(child, depth + 1, out);
            }
            CFRelease(children);
        }
    }
}

fn pid_for_live_captions() -> Option<i32> {
    // Prefer exact path match via pgrep -f
    for pattern in [
        "Live Captions.app/Contents/MacOS/Live Captions",
        "Live Captions",
        "LiveTranscriptionAgent",
    ] {
        let output = std::process::Command::new("pgrep")
            .args(["-f", pattern])
            .output()
            .ok()?;
        if !output.status.success() {
            continue;
        }
        let s = String::from_utf8_lossy(&output.stdout);
        if let Some(pid) = s
            .lines()
            .filter_map(|l| l.trim().parse::<i32>().ok())
            .next()
        {
            return Some(pid);
        }
    }
    None
}

/// Ask macOS to show the Accessibility permission dialog (once per call when untrusted).
pub fn request_accessibility_prompt() -> bool {
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
    // Offer the system prompt if we are not trusted yet.
    let trusted = if unsafe { AXIsProcessTrusted() } != 0 {
        true
    } else {
        request_accessibility_prompt()
    };

    if !trusted {
        return CaptureSnapshot {
            process_running: true,
            detail: presence.detail,
            surface_text: None,
            error: Some(
                "macOS Accessibility is OFF for this app. \
                 System Settings → Privacy & Security → Accessibility → enable \
                 the app that launched Interpres (Terminal, Interpres, or iTerm). \
                 Quit and reopen Interpres after enabling. \
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

    let app = unsafe { AXUIElementCreateApplication(pid) };
    if app.is_null() {
        return CaptureSnapshot {
            process_running: true,
            detail: presence.detail,
            surface_text: None,
            error: Some("AXUIElementCreateApplication failed".into()),
        };
    }

    let mut strings = Vec::new();
    if let Some(windows) = ax_copy(app, "AXWindows") {
        unsafe {
            let n = CFArrayGetCount(windows);
            if n == 0 {
                // Window list empty — still walk the app element (agent UI).
                collect_strings(app, 0, &mut strings);
            }
            for i in 0..n.min(12) {
                let w = CFArrayGetValueAtIndex(windows, i);
                collect_strings(w, 0, &mut strings);
            }
            CFRelease(windows);
        }
    } else {
        collect_strings(app, 0, &mut strings);
    }
    // Also try focused UI under the app
    if let Some(focused) = ax_copy(app, "AXFocusedUIElement") {
        collect_strings(focused, 0, &mut strings);
        unsafe { CFRelease(focused) };
    }
    unsafe { CFRelease(app) };

    // Rank non-junk candidates; pure picker never returns junk-only surfaces.
    let candidate_count = strings.len();
    let surface = crate::buffer::pick_caption_surface(strings.iter().map(|s| s.as_str()));

    crate::debuglog::log(&format!(
        "macos poll pid={pid} surface_chars={} candidates={} detail={}",
        surface.as_ref().map(|s| s.chars().count()).unwrap_or(0),
        candidate_count,
        presence.detail
    ));

    if surface.is_none() {
        // AX trusted + process up, but only chrome / empty tree — not a permission error.
        // Engine treats surface_text=None as empty ticks (clear live); probe stays exit 0.
        return CaptureSnapshot {
            process_running: true,
            detail: format!(
                "{}; pid={pid}; ax_trusted=true; no_caption_surface",
                presence.detail
            ),
            surface_text: None,
            error: None,
        };
    }

    CaptureSnapshot {
        process_running: true,
        detail: format!(
            "{}; pid={pid}; ax_trusted=true; surface_ok",
            presence.detail
        ),
        surface_text: surface,
        error: None,
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
    let app = unsafe { AXUIElementCreateApplication(pid) };
    if app.is_null() {
        lines.push("AXUIElementCreateApplication failed".into());
        return lines;
    }
    let mut strings = Vec::new();
    if let Some(windows) = ax_copy(app, "AXWindows") {
        unsafe {
            let n = CFArrayGetCount(windows);
            lines.push(format!("ax_windows={n}"));
            for i in 0..n.min(12) {
                let w = CFArrayGetValueAtIndex(windows, i);
                collect_strings(w, 0, &mut strings);
            }
            CFRelease(windows);
        }
    } else {
        lines.push("ax_windows=none (walking app root)".into());
        collect_strings(app, 0, &mut strings);
    }
    unsafe { CFRelease(app) };
    lines.push(format!("ax_text_nodes={}", strings.len()));
    // Show top 5 longest samples (truncated)
    let mut ranked = strings;
    ranked.sort_by_key(|s| std::cmp::Reverse(s.chars().count()));
    for (i, s) in ranked.iter().take(5).enumerate() {
        let preview: String = s.chars().take(120).collect();
        lines.push(format!("sample[{i}] chars={} text={preview}", s.chars().count()));
    }
    if ranked.is_empty() {
        lines.push(
            "No AX text found. Confirm Live Captions window is visible and audio is playing."
                .into(),
        );
    }
    lines
}
