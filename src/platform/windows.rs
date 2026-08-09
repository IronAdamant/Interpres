//! Windows Live Captions text capture via UI Automation (hand-written COM FFI).
//!
//! Compiles only on Windows. Uses system OLE/COM + UIAutomationCore.
//! Primary text path today: PowerShell + UIAutomationClient (OS assemblies),
//! with FindWindow process/window checks in-process.

use super::detect::LiveCaptionsPresence;
use super::signals::windows_signals;
use super::CaptureSnapshot;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::ptr;

/// Hide console windows when a GUI-subsystem app spawns console tools (powershell, etc.).
/// Without this, each poll flashes a terminal while listening.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

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

/// Built-in copy of `helpers/windows/Get-LiveCaptionsText.ps1` so a lone
/// `interpres.exe` (e.g. `target/release` or a copied binary) still works.
const EMBEDDED_UIA_HELPER: &str =
    include_str!("../../helpers/windows/Get-LiveCaptionsText.ps1");

/// Locate `Get-LiveCaptionsText.ps1` next to the binary, in parent trees
/// (cargo layout: `target/release` → repo `helpers/`), cwd, or common layout.
pub fn find_uia_helper() -> Option<PathBuf> {
    let name = "Get-LiveCaptionsText.ps1";
    let rel = Path::new("helpers").join("windows").join(name);
    let mut candidates: Vec<PathBuf> = Vec::new();

    if let Ok(exe) = std::env::current_exe() {
        let mut dir = exe.parent().map(|p| p.to_path_buf());
        // Walk up so `target/release/interpres.exe` finds repo helpers/.
        for _ in 0..8 {
            let Some(d) = dir else { break };
            candidates.push(d.join(&rel));
            candidates.push(d.join(name));
            candidates.push(d.join("helpers").join("windows").join(name));
            dir = d.parent().map(|p| p.to_path_buf());
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        let mut dir = Some(cwd);
        for _ in 0..6 {
            let Some(d) = dir else { break };
            candidates.push(d.join(&rel));
            candidates.push(d.join(name));
            candidates.push(d.join("helpers").join("windows").join(name));
            dir = d.parent().map(|p| p.to_path_buf());
        }
    }
    candidates.push(rel);
    candidates.push(PathBuf::from(name));

    for c in candidates {
        if c.is_file() {
            return Some(c);
        }
    }
    None
}

/// Prefer a real file on disk; otherwise write the embedded helper next to the
/// exe (or under %TEMP%\Interpres) so listening never depends on packaging alone.
pub fn resolve_uia_helper() -> Option<PathBuf> {
    if let Some(p) = find_uia_helper() {
        return Some(p);
    }
    materialize_embedded_helper()
}

fn materialize_embedded_helper() -> Option<PathBuf> {
    let name = "Get-LiveCaptionsText.ps1";
    let mut targets: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            targets.push(dir.join(name));
        }
    }
    targets.push(std::env::temp_dir().join("Interpres").join(name));

    for t in targets {
        if t.is_file() {
            if let Ok(existing) = std::fs::read_to_string(&t) {
                if existing.contains("CaptionsTextBlock") {
                    return Some(t);
                }
            }
        }
        if let Some(parent) = t.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if std::fs::write(&t, EMBEDDED_UIA_HELPER).is_ok() {
            return Some(t);
        }
    }
    None
}

fn live_captions_window_found() -> bool {
    let signals = windows_signals();
    let class = to_wide(
        signals
            .window_classes
            .first()
            .copied()
            .unwrap_or("LiveCaptionsDesktopWindow"),
    );
    let hwnd = unsafe { FindWindowW(class.as_ptr(), ptr::null()) };
    !hwnd.is_null()
}

/// Best-effort UIA read of CaptionsTextBlock Name.
/// Full COM vtable walk is fragile; we use FindWindow + PowerShell UIAutomation.
pub fn poll_text(presence: LiveCaptionsPresence) -> CaptureSnapshot {
    if !live_captions_window_found() {
        return CaptureSnapshot {
            process_running: true,
            detail: presence.detail,
            surface_text: None,
            error: Some(
                "LiveCaptions process found but LiveCaptionsDesktopWindow not found \
                 (is Live Captions open? Win+Ctrl+L)"
                    .into(),
            ),
        };
    }

    if let Some(text) = try_uia_via_powershell() {
        return CaptureSnapshot {
            process_running: true,
            detail: format!(
                "{}; window=LiveCaptionsDesktopWindow; via=powershell-uia",
                presence.detail
            ),
            surface_text: Some(text),
            error: None,
        };
    }

    let helper_hint = resolve_uia_helper()
        .map(|p| format!("helper at {}", p.display()))
        .unwrap_or_else(|| {
            "could not locate or create Get-LiveCaptionsText.ps1".into()
        });

    CaptureSnapshot {
        process_running: true,
        detail: format!("{}; window=LiveCaptionsDesktopWindow", presence.detail),
        surface_text: None,
        error: Some(format!(
            "UIA text scrape failed ({helper_hint}). \
             Keep Live Captions open and showing text (Win+Ctrl+L). \
             If this persists, run Diagnose or place Get-LiveCaptionsText.ps1 next to interpres.exe."
        )),
    }
}

fn try_uia_via_powershell() -> Option<String> {
    let helper = resolve_uia_helper()?;
    run_uia_helper(&helper)
}

fn run_uia_helper(helper: &Path) -> Option<String> {
    // Prefer Windows PowerShell 5.1; fall back to pwsh if present.
    // CREATE_NO_WINDOW is required: interpres is a GUI PE, so each unflagged
    // powershell.exe poll would open a visible console (~every poll_ms).
    for shell in ["powershell.exe", "powershell", "pwsh.exe", "pwsh"] {
        let out = Command::new(shell)
            .args([
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-WindowStyle",
                "Hidden",
                "-File",
            ])
            .arg(helper)
            .creation_flags(CREATE_NO_WINDOW)
            .output();
        let Ok(out) = out else {
            continue;
        };
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !s.is_empty() {
                return Some(s);
            }
        }
    }
    None
}

/// Extra diagnostics for `interpres diagnose` on Windows.
pub fn diagnose_lines() -> Vec<String> {
    let mut lines = Vec::new();
    let presence = super::detect::live_captions_present();
    lines.push(format!("process_running={}", presence.running));
    lines.push(format!("detail={}", presence.detail));
    lines.push(format!("window_found={}", live_captions_window_found()));

    match resolve_uia_helper() {
        Some(h) => {
            lines.push(format!("helper={}", h.display()));
            match run_uia_helper(&h) {
                Some(text) => {
                    lines.push(format!("helper_ok=true chars={}", text.chars().count()));
                    let preview: String = text.chars().take(120).collect();
                    lines.push(format!("surface_preview={preview}"));
                }
                None => {
                    lines.push("helper_ok=false (empty output or non-zero exit)".into());
                    lines.push(
                        "Tip: turn Live Captions on (Win+Ctrl+L) and play audio so text appears."
                            .into(),
                    );
                }
            }
        }
        None => {
            lines.push("helper=not_found_and_could_not_materialize".into());
            lines.push(
                "Place Get-LiveCaptionsText.ps1 next to interpres.exe \
                 (portable pack includes it; release builds also embed a fallback)."
                    .into(),
            );
        }
    }

    if presence.running {
        let snap = poll_text(presence);
        lines.push(format!("poll_surface={}", snap.surface_text.is_some()));
        if let Some(e) = snap.error {
            lines.push(format!("poll_error={e}"));
        }
    }

    lines.push("Windows tip: Live Captions is Win+Ctrl+L (Settings → Accessibility → Captions).".into());
    lines
}

/// Supported primary scrape path is PowerShell + OS UIAutomation assemblies
/// (silent via CREATE_NO_WINDOW). In-process COM UIA remains a scaffold only
/// (PR6 spike deferred) so we never block releases on incomplete FFI.

// Scaffold for a future full in-process UIA COM walk (no PowerShell).
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_no_window_is_standard_flag() {
        // Win32 CREATE_NO_WINDOW — required so GUI PE does not flash consoles.
        assert_eq!(CREATE_NO_WINDOW, 0x0800_0000);
    }

    #[test]
    fn resolve_uia_helper_finds_or_materializes_script() {
        let p = resolve_uia_helper().expect("helper path");
        assert!(p.is_file(), "missing {}", p.display());
        let body = std::fs::read_to_string(&p).expect("read helper");
        assert!(
            body.contains("CaptionsTextBlock"),
            "helper does not look like UIA script"
        );
    }

    #[test]
    fn embedded_helper_source_is_nonempty() {
        assert!(EMBEDDED_UIA_HELPER.contains("LiveCaptionsDesktopWindow"));
        assert!(EMBEDDED_UIA_HELPER.contains("CaptionsTextBlock"));
    }
}
