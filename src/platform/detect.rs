//! Process presence via pure `std` + OS process listing commands / FFI-free tools.

#[cfg(windows)]
use super::signals::windows_signals;
#[cfg(target_os = "macos")]
use super::signals::macos_signals;

#[derive(Clone, Debug)]
pub struct LiveCaptionsPresence {
    pub running: bool,
    pub detail: String,
}

/// Detect whether OS Live Captions process is running.
pub fn live_captions_present() -> LiveCaptionsPresence {
    #[cfg(windows)]
    {
        return detect_windows();
    }
    #[cfg(target_os = "macos")]
    {
        return detect_macos();
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        LiveCaptionsPresence {
            running: false,
            detail: "unsupported OS for Live Captions detection".into(),
        }
    }
}

#[cfg(windows)]
mod toolhelp {
    //! In-process process listing (no `tasklist` spawn per poll).
    use std::os::raw::c_void;

    const TH32CS_SNAPPROCESS: u32 = 0x0000_0002;
    const INVALID_HANDLE_VALUE: isize = -1;

    #[repr(C)]
    struct ProcessEntry32W {
        dw_size: u32,
        cnt_usage: u32,
        th32_process_id: u32,
        th32_default_heap_id: usize,
        th32_module_id: u32,
        cnt_threads: u32,
        th32_parent_process_id: u32,
        pc_pri_class_base: i32,
        dw_flags: u32,
        sz_exe_file: [u16; 260],
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn CreateToolhelp32Snapshot(flags: u32, pid: u32) -> *mut c_void;
        fn Process32FirstW(snap: *mut c_void, entry: *mut ProcessEntry32W) -> i32;
        fn Process32NextW(snap: *mut c_void, entry: *mut ProcessEntry32W) -> i32;
        fn CloseHandle(h: *mut c_void) -> i32;
    }

    /// `Some(true/false)` when the snapshot worked; `None` to fall back to tasklist.
    pub fn any_process_named(names: &[&str]) -> Option<bool> {
        let wanted: Vec<String> = names.iter().map(|n| n.to_ascii_lowercase()).collect();
        unsafe {
            let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if snap.is_null() || snap as isize == INVALID_HANDLE_VALUE {
                return None;
            }
            let mut entry: ProcessEntry32W = std::mem::zeroed();
            entry.dw_size = std::mem::size_of::<ProcessEntry32W>() as u32;
            let mut found = false;
            let mut ok = Process32FirstW(snap, &mut entry);
            while ok != 0 {
                let len = entry.sz_exe_file.iter().position(|&c| c == 0).unwrap_or(260);
                let exe = String::from_utf16_lossy(&entry.sz_exe_file[..len]).to_ascii_lowercase();
                if wanted.iter().any(|w| exe == *w) {
                    found = true;
                    break;
                }
                ok = Process32NextW(snap, &mut entry);
            }
            CloseHandle(snap);
            Some(found)
        }
    }
}

#[cfg(windows)]
fn window_class_exists(class: &str) -> bool {
    #[link(name = "user32")]
    extern "system" {
        fn FindWindowW(class: *const u16, title: *const u16) -> *mut std::os::raw::c_void;
    }
    let wide: Vec<u16> = class.encode_utf16().chain(std::iter::once(0)).collect();
    !unsafe { FindWindowW(wide.as_ptr(), std::ptr::null()) }.is_null()
}

#[cfg(windows)]
fn detect_windows() -> LiveCaptionsPresence {
    use std::os::windows::process::CommandExt;

    // Hide console: GUI-subsystem interpres would otherwise flash a window every poll.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let signals = windows_signals();
    // Fast path, every poll: the captions window exists only while Live Captions runs.
    // Listing every process costs ~10 ms; this costs microseconds.
    if signals.window_classes.iter().any(|c| window_class_exists(c)) {
        return LiveCaptionsPresence {
            running: true,
            detail: "window found: Live Captions".into(),
        };
    }
    match toolhelp::any_process_named(&["LiveCaptions.exe"]) {
        Some(true) => {
            return LiveCaptionsPresence {
                running: true,
                detail: "process matched: LiveCaptions.exe".into(),
            }
        }
        Some(false) => {
            return LiveCaptionsPresence {
                running: false,
                detail: "LiveCaptions.exe not running".into(),
            }
        }
        None => {}
    }

    // Fallback: tasklist is always available on Windows interactive sessions.
    let output = std::process::Command::new("tasklist")
        .args(["/FO", "CSV", "/NH"])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    match output {
        Ok(out) => {
            let text = String::from_utf8_lossy(&out.stdout).to_ascii_lowercase();
            for name in signals.process_names {
                let n = name.to_ascii_lowercase();
                if text.contains(&n) {
                    return LiveCaptionsPresence {
                        running: true,
                        detail: format!("process matched: {name}"),
                    };
                }
            }
            LiveCaptionsPresence {
                running: false,
                detail: "LiveCaptions.exe not in tasklist".into(),
            }
        }
        Err(e) => LiveCaptionsPresence {
            running: false,
            detail: format!("tasklist failed: {e}"),
        },
    }
}

#[cfg(target_os = "macos")]
fn detect_macos() -> LiveCaptionsPresence {
    let signals = macos_signals();
    // pgrep by bundle id path / process name
    // Try pgrep -fl for full command line
    let output = std::process::Command::new("pgrep")
        .args(["-fl", "Live"])
        .output();
    match output {
        Ok(out) => {
            let text = String::from_utf8_lossy(&out.stdout);
            for sub in signals.process_substrings {
                if text.lines().any(|l| l.contains(sub)) {
                    return LiveCaptionsPresence {
                        running: true,
                        detail: format!("process matched: {sub}"),
                    };
                }
            }
            // Also try exact pgrep for Live Captions
            if let Ok(out2) = std::process::Command::new("pgrep")
                .args(["-f", "Live Captions"])
                .output()
            {
                if out2.status.success() && !out2.stdout.is_empty() {
                    return LiveCaptionsPresence {
                        running: true,
                        detail: "pgrep -f 'Live Captions' matched".into(),
                    };
                }
            }
            LiveCaptionsPresence {
                running: false,
                detail: "Live Captions agent not running".into(),
            }
        }
        Err(e) => {
            // Fallback: ps
            if let Ok(out) = std::process::Command::new("ps")
                .args(["-ax", "-o", "command="])
                .output()
            {
                let text = String::from_utf8_lossy(&out.stdout);
                for sub in signals.process_substrings {
                    if text.contains(sub) {
                        return LiveCaptionsPresence {
                            running: true,
                            detail: format!("ps matched: {sub}"),
                        };
                    }
                }
                return LiveCaptionsPresence {
                    running: false,
                    detail: format!("ps scan: Live Captions not found (pgrep err: {e})"),
                };
            }
            LiveCaptionsPresence {
                running: false,
                detail: format!("process scan failed: {e}"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detection_returns_structured_result() {
        let p = live_captions_present();
        // Must not panic; detail non-empty
        assert!(!p.detail.is_empty());
    }
}
