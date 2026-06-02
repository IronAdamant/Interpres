//! Extremely minimal logging for Phase 1.
//! For test/debug builds (via "debug-logs" feature or debug profile) we also persist
//! *everything* to a .txt file next to the executable for easy post-run investigation
//! without any install or extra tools.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

static VERBOSE: AtomicBool = AtomicBool::new(false);
static LOG_FILE: Mutex<Option<File>> = Mutex::new(None);

pub fn set_verbose(v: bool) {
    VERBOSE.store(v, Ordering::Relaxed);
}

pub fn is_verbose() -> bool {
    VERBOSE.load(Ordering::Relaxed)
}

/// Initialize (or re-init) the debug log file for test investigation.
/// The log is placed *next to the executable* as `interpres-test-debug.log.txt`.
/// This makes the test .exe completely portable: copy the .exe anywhere, run it,
/// and the log appears beside it — zero config, zero install.
///
/// Also forces verbose mode so all debug details (plugin stdout/stderr, restarts, etc.)
/// are captured in the file.
///
/// Safe to call multiple times; appends a new session header each time.
pub fn init_debug_log_file() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent().unwrap_or_else(|| std::path::Path::new("."));
    let log_path = dir.join("interpres-test-debug.log.txt");

    let mut file = match OpenOptions::new().create(true).append(true).open(&log_path) {
        Ok(f) => f,
        Err(e) => {
            // Fallback: try cwd
            let fallback = std::path::Path::new("interpres-test-debug.log.txt");
            match OpenOptions::new().create(true).append(true).open(fallback) {
                Ok(f) => {
                    eprintln!("[warn] Could not open log next to exe ({}), using cwd fallback", e);
                    f
                }
                Err(_) => return None,
            }
        }
    };

    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let pid = std::process::id();
    let args: Vec<String> = std::env::args().collect();
    let cwd = std::env::current_dir().unwrap_or_default();

    let header = format!(
        "\n\n========== NEW RUN @ {} (unix secs) ==========\nexe: {}\npid: {}\nargs: {:?}\ncwd: {}\nRUST_BACKTRACE={}\n==============================================\n",
        ts,
        exe.display(),
        pid,
        args,
        cwd.display(),
        std::env::var("RUST_BACKTRACE").unwrap_or_else(|_| "0".into())
    );

    let _ = file.write_all(header.as_bytes());
    let _ = file.flush();

    *LOG_FILE.lock().unwrap() = Some(file);

    // For test/debug: capture *everything*
    set_verbose(true);

    // Also tell user (if they have a console)
    eprintln!("[debug-logs] Detailed test log is being written to: {}", log_path.display());

    Some(log_path)
}

/// Install a panic hook that writes full panic + backtrace info into the debug log file.
/// Only really useful when debug-logs is active (or RUST_BACKTRACE=1).
pub fn install_debug_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let payload: &str = info
            .payload()
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| info.payload().downcast_ref::<String>().map(|s| s.as_str()))
            .unwrap_or("<non-string payload>");

        let loc = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_else(|| "<unknown location>".into());

        // Force a backtrace capture (will be detailed if RUST_BACKTRACE=1 or full)
        let bt = std::backtrace::Backtrace::force_capture();

        let full = format!(
            "!!! PANIC: {} @ {}\nBacktrace:\n{:?}\n",
            payload, loc, bt
        );

        // This will go to both stderr (if any) and the file (because log always writes when file open)
        log("error", &full);

        // Ensure the default-ish message also appears if possible
        eprintln!("FATAL PANIC: {} @ {}", payload, loc);
    }));
}

/// Core log function. In debug-logs mode the file is open and we force verbose,
/// so debug plugin chatter etc. ends up in the .txt as well as stderr.
pub fn log(level: &str, msg: &str) {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    if level == "debug" && !is_verbose() {
        return;
    }

    let line = format!("[{:>5}] [{:05}] {}", level.to_uppercase(), ts % 100000, msg);
    eprintln!("{}", line);

    if let Some(f) = LOG_FILE.lock().unwrap().as_mut() {
        let _ = writeln!(f, "{}", line);
        // Flush on every line for test builds so logs are visible even if crash
        let _ = f.flush();
    }
}

#[macro_export]
macro_rules! log_info {
    ($($arg:tt)*) => {
        $crate::util::logging::log("info", &format!($($arg)*))
    };
}

#[macro_export]
macro_rules! log_error {
    ($($arg:tt)*) => {
        $crate::util::logging::log("error", &format!($($arg)*))
    };
}
