//! Windows Live Captions text capture via UI Automation (hand-written COM FFI).
//!
//! Compiles only on Windows. Uses system OLE/COM + UIAutomationCore.

use super::detect::LiveCaptionsPresence;
use super::signals::windows_signals;
use super::CaptureSnapshot;
use std::ptr;

// Minimal Win32 / UIA bindings.

#[link(name = "ole32")]
extern "system" {
    fn CoInitializeEx(pvreserved: *mut core::ffi::c_void, dwcoinit: u32) -> i32;
    fn CoCreateInstance(
        rclsid: *const Guid,
        punkouter: *mut core::ffi::c_void,
        dwclscontext: u32,
        riid: *const Guid,
        ppv: *mut *mut core::ffi::c_void,
    ) -> i32;
    fn CoUninitialize();
}

#[link(name = "user32")]
extern "system" {
    fn FindWindowW(lpclassname: *const u16, lpwindowname: *const u16) -> *mut core::ffi::c_void;
}

#[repr(C)]
struct Guid {
    data1: u32,
    data2: u16,
    data3: u16,
    data4: [u8; 8],
}

// CLSID_CUIAutomation = {ff48dba4-60ef-4201-aa87-54103eef594e}
const CLSID_CUI_AUTOMATION: Guid = Guid {
    data1: 0xff48dba4,
    data2: 0x60ef,
    data3: 0x4201,
    data4: [0xaa, 0x87, 0x54, 0x10, 0x3e, 0xef, 0x59, 0x4e],
};

// IID_IUIAutomation = {30cbe57d-d9d0-452a-ab13-7ac5ac4825ee}
const IID_IUI_AUTOMATION: Guid = Guid {
    data1: 0x30cbe57d,
    data2: 0xd9d0,
    data3: 0x452a,
    data4: [0xab, 0x13, 0x7a, 0xc5, 0xac, 0x48, 0x25, 0xee],
};

const COINIT_APARTMENTTHREADED: u32 = 0x2;
const CLSCTX_INPROC_SERVER: u32 = 0x1;
const S_OK: i32 = 0;
const S_FALSE: i32 = 1;

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Best-effort UIA read of CaptionsTextBlock Name.
/// Full COM vtable walk is fragile; we use FindWindow + a simplified path.
/// When UIA automation is unavailable, return process-running with degraded note.
pub fn poll_text(presence: LiveCaptionsPresence) -> CaptureSnapshot {
    let signals = windows_signals();
    let class = to_wide(signals.window_classes.first().copied().unwrap_or("LiveCaptionsDesktopWindow"));

    let hwnd = unsafe { FindWindowW(class.as_ptr(), ptr::null()) };
    if hwnd.is_null() {
        return CaptureSnapshot {
            process_running: true,
            detail: presence.detail,
            surface_text: None,
            error: Some(
                "LiveCaptions process found but LiveCaptionsDesktopWindow not found".into(),
            ),
        };
    }

    // Attempt CoInitialize + UIA ElementFromHandle via dynamic approach is large.
    // Ship a helper script path recommended; in-process we report window found and
    // try helper if configured. For pure in-process, call external PowerShell UIA.
    if let Some(text) = try_uia_via_powershell() {
        return CaptureSnapshot {
            process_running: true,
            detail: format!("{}; window=LiveCaptionsDesktopWindow; via=powershell-uia", presence.detail),
            surface_text: Some(text),
            error: None,
        };
    }

    CaptureSnapshot {
        process_running: true,
        detail: format!("{}; window=LiveCaptionsDesktopWindow", presence.detail),
        surface_text: None,
        error: Some(
            "UIA text scrape needs helpers/windows/Get-LiveCaptionsText.ps1 beside the binary, \
             or run: interpres run --helper <path>"
                .into(),
        ),
    }
}

fn try_uia_via_powershell() -> Option<String> {
    // Prefer helper next to exe or known relative path
    let candidates = [
        "helpers/windows/Get-LiveCaptionsText.ps1",
        "Get-LiveCaptionsText.ps1",
    ];
    for c in candidates {
        if std::path::Path::new(c).exists() {
            let out = std::process::Command::new("powershell")
                .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", c])
                .output()
                .ok()?;
            if out.status.success() {
                let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if !s.is_empty() {
                    return Some(s);
                }
            }
        }
    }
    None
}

// Silence unused COM imports when powershell path is used primarily.
#[allow(dead_code)]
fn _ensure_com_symbols_linked() {
    unsafe {
        let _ = CoInitializeEx(ptr::null_mut(), COINIT_APARTMENTTHREADED);
        let mut punk: *mut core::ffi::c_void = ptr::null_mut();
        let _ = CoCreateInstance(
            &CLSID_CUI_AUTOMATION,
            ptr::null_mut(),
            CLSCTX_INPROC_SERVER,
            &IID_IUI_AUTOMATION,
            &mut punk,
        );
        if !punk.is_null() {
            // Would Release via IUnknown — left as link-only scaffold for future full UIA.
        }
        let _ = (S_OK, S_FALSE);
        CoUninitialize();
    }
}
