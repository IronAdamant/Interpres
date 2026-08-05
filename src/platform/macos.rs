//! macOS Live Captions text capture via Accessibility (hand-written FFI).

use super::detect::LiveCaptionsPresence;
use super::CaptureSnapshot;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_void};
use std::ptr;

// Minimal CoreFoundation / ApplicationServices bindings (system libs only).

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
}

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> u8;
    fn AXUIElementCreateApplication(pid: i32) -> *const c_void;
    fn AXUIElementCopyAttributeValue(
        element: *const c_void,
        attribute: *const c_void,
        value: *mut *const c_void,
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
        if CFGetTypeID(cf) != CFStringGetTypeID() {
            return None;
        }
        let len = CFStringGetLength(cf);
        let mut buf = vec![0i8; (len as usize) * 4 + 16];
        let ok = CFStringGetCString(
            cf,
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

fn collect_strings(element: *const c_void, depth: u32, out: &mut Vec<String>) {
    if element.is_null() || depth > 12 {
        return;
    }
    // value
    if let Some(v) = ax_copy(element, "AXValue") {
        if let Some(s) = cfstring_to_rust(v) {
            let t = s.trim().to_string();
            if t.len() > 1 {
                out.push(t);
            }
        }
        unsafe { CFRelease(v) };
    }
    // title
    if let Some(v) = ax_copy(element, "AXTitle") {
        if let Some(s) = cfstring_to_rust(v) {
            let t = s.trim().to_string();
            if t.len() > 1 && t.len() < 500 {
                out.push(t);
            }
        }
        unsafe { CFRelease(v) };
    }
    // children
    if let Some(children) = ax_copy(element, "AXChildren") {
        unsafe {
            let n = CFArrayGetCount(children);
            for i in 0..n.min(40) {
                let child = CFArrayGetValueAtIndex(children, i);
                collect_strings(child, depth + 1, out);
            }
            CFRelease(children);
        }
    }
}

fn pid_for_live_captions() -> Option<i32> {
    let output = std::process::Command::new("pgrep")
        .args(["-f", "Live Captions"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&output.stdout);
    s.lines()
        .filter_map(|l| l.trim().parse::<i32>().ok())
        .next()
}

pub fn poll_text(presence: LiveCaptionsPresence) -> CaptureSnapshot {
    let trusted = unsafe { AXIsProcessTrusted() } != 0;
    if !trusted {
        return CaptureSnapshot {
            process_running: true,
            detail: presence.detail,
            surface_text: None,
            error: Some(
                "macOS Accessibility permission not granted for Interpres. \
                 System Settings → Privacy & Security → Accessibility → enable Interpres."
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
    // Windows first
    if let Some(windows) = ax_copy(app, "AXWindows") {
        unsafe {
            let n = CFArrayGetCount(windows);
            for i in 0..n.min(8) {
                let w = CFArrayGetValueAtIndex(windows, i);
                collect_strings(w, 0, &mut strings);
            }
            CFRelease(windows);
        }
    } else {
        collect_strings(app, 0, &mut strings);
    }
    unsafe { CFRelease(app) };

    // Pick the longest string as caption surface (heuristic L1 fallback).
    let surface = strings.into_iter().max_by_key(|s| s.len());

    CaptureSnapshot {
        process_running: true,
        detail: format!("{}; pid={pid}; ax_trusted=true", presence.detail),
        surface_text: surface,
        error: None,
    }
}
