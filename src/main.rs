//! Interpres — native UI (default on Mac) + CLI. Zero crates.io dependencies.

use interpres::buffer::{BufferEmit, CaptionBuffer};
use interpres::config::Config;
use interpres::gui;
use interpres::lifecycle::{Lifecycle, LifecycleAction};
use interpres::platform;
use interpres::plugin_host::PluginHost;
use interpres::probe::{self, print_probe};
use interpres::protocol::{CaptionEvent, LcState};
use interpres::transcript::{format_clock, TranscriptWriter};
use interpres::watcher;
use std::env;
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, SystemTime};

fn main() {
    let mut args: Vec<String> = env::args().skip(1).collect();

    // Allow `interpres cli …` to force command-line mode.
    if args.first().map(|a| a.as_str()) == Some("cli") {
        args.remove(0);
    } else {
        // Non-technical default: open the native window (Win32 / AppKit).
        // Double-click also lands here (no args, or INTERPRES_FRIENDLY / INTERPRES_GUI).
        let want_gui = args.is_empty()
            || args.first().map(|a| a.as_str()) == Some("gui")
            || env::var_os("INTERPRES_GUI").is_some()
            || env::var_os("INTERPRES_FRIENDLY").is_some();
        if want_gui {
            #[cfg(any(target_os = "macos", windows))]
            {
                if args.is_empty()
                    || args[0] == "gui"
                    || env::var_os("INTERPRES_FRIENDLY").is_some()
                    || env::var_os("INTERPRES_GUI").is_some()
                {
                    // If they passed a real CLI subcommand with FRIENDLY, still honor CLI…
                    let is_cli_cmd = args.first().map(|a| {
                        matches!(
                            a.as_str(),
                            "run"
                                | "probe"
                                | "diagnose"
                                | "watch"
                                | "demo"
                                | "help"
                                | "set-folder"
                                | "remember"
                                | "show-config"
                                | "fix-folder"
                                | "version"
                                | "-h"
                                | "--help"
                                | "-V"
                                | "--version"
                        )
                    });
                    if args.is_empty()
                        || args[0] == "gui"
                        || is_cli_cmd != Some(true)
                    {
                        std::process::exit(gui::run_native_gui());
                    }
                }
            }
            #[cfg(not(any(target_os = "macos", windows)))]
            {
                if args.is_empty() {
                    args.push("run".into());
                }
            }
        }
    }

    if args.is_empty() {
        print_help();
        args.push("run".into());
    }

    let cmd = args[0].as_str();
    let rest: Vec<&str> = args[1..].iter().map(|s| s.as_str()).collect();

    let code = match cmd {
        "gui" => gui::run_native_gui(),
        "help" | "-h" | "--help" => {
            print_help();
            0
        }
        "version" | "-V" | "--version" => {
            println!("interpres {}", env!("CARGO_PKG_VERSION"));
            0
        }
        "probe" => cmd_probe(),
        "diagnose" => cmd_diagnose(),
        "run" => cmd_run(&rest),
        "watch" => cmd_watch(&rest),
        "set-folder" => cmd_set_folder(&rest),
        "show-config" => cmd_show_config(),
        "remember" => cmd_remember(&rest),
        "demo" => cmd_demo(&rest),
        "fix-folder" => cmd_fix_folder(),
        other => {
            eprintln!("Unknown command: {other}");
            print_help();
            1
        }
    };
    std::process::exit(code);
}

fn print_help() {
    #[cfg(windows)]
    {
        println!(
            r#"Interpres — local companion that records what Live Captions shows

Not a speech engine. Requires Windows Live Captions (Win+Ctrl+L). Cannot work alone.
Cannot guarantee 100% accuracy — only that captions can be saved when LC works.

Easy: download a Release pack, open interpres.exe, Start listening.
Optional: Save to disk ON. Keep Get-LiveCaptionsText.ps1 next to the exe.

Commands (advanced):
  gui | run | probe | diagnose | set-folder | remember | demo | help | cli

Zero crates.io dependencies. Free & open source. 100% local.
"#
        );
    }
    #[cfg(target_os = "macos")]
    {
        println!(
            r#"Interpres — local companion that records what Live Captions shows

Not a speech engine. Requires Mac Live Captions. Cannot work alone.
Cannot guarantee 100% accuracy — only that captions can be saved when LC works.

Easy: download a Release, open Interpres.app, Start listening.
Optional: Save to disk ON. Accessibility may be required.

Commands (advanced):
  gui | run | probe | diagnose | set-folder | remember | demo | help | cli

Zero crates.io dependencies. Free & open source. 100% local.
"#
        );
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        println!(
            r#"Interpres — save Live Captions as transcripts

Live Captions capture is implemented for Windows and macOS.
Commands: run | probe | diagnose | set-folder | remember | demo | help
"#
        );
    }
}

fn cmd_probe() -> i32 {
    #[cfg(target_os = "macos")]
    {
        let _ = interpres::platform::macos::request_accessibility_prompt();
    }
    let report = probe::run_probe();
    let _ = print_probe(&mut io::stdout(), &report);
    if report.exit_code == probe::EXIT_PERMISSION {
        println!();
        println!(">>> ACTION NEEDED (Mac):");
        println!("  1. System Settings → Privacy & Security → Accessibility");
        println!("  2. Turn ON the app that runs Interpres:");
        println!("       • Terminal  (if you started it from Terminal)");
        println!("       • Interpres (if you double-clicked Interpres.app)");
        println!("  3. Quit Interpres completely, open it again, run: interpres diagnose");
        #[cfg(target_os = "macos")]
        {
            interpres::platform::macos::open_accessibility_settings();
            println!("  (Tried to open Accessibility settings for you.)");
        }
    }
    report.exit_code
}

fn cmd_diagnose() -> i32 {
    println!("Interpres diagnose");
    let presence = platform::live_captions_present();
    println!("live_captions_running={}", presence.running);
    println!("detail={}", presence.detail);
    #[cfg(target_os = "macos")]
    {
        for line in interpres::platform::macos::diagnose_lines() {
            println!("{line}");
        }
        if !interpres::platform::macos::is_accessibility_trusted() {
            interpres::platform::macos::open_accessibility_settings();
            return probe::EXIT_PERMISSION;
        }
        if !presence.running {
            return probe::EXIT_LC_NOT_RUNNING;
        }
        let snap = platform::poll_capture();
        println!("surface_present={}", snap.surface_text.is_some());
        if let Some(ref t) = snap.surface_text {
            let preview: String = t.chars().take(200).collect();
            println!("surface_preview={preview}");
            return 0;
        }
        if let Some(e) = snap.error {
            println!("error={e}");
            return probe::EXIT_SIGNALS_STALE;
        }
        return probe::EXIT_SIGNALS_STALE;
    }
    #[cfg(windows)]
    {
        for line in interpres::platform::windows::diagnose_lines() {
            println!("{line}");
        }
        if !presence.running {
            println!();
            println!(">>> Turn on Live Captions: Win+Ctrl+L");
            return probe::EXIT_LC_NOT_RUNNING;
        }
        let snap = platform::poll_capture();
        println!("surface_present={}", snap.surface_text.is_some());
        if let Some(ref t) = snap.surface_text {
            let preview: String = t.chars().take(200).collect();
            println!("surface_preview={preview}");
            return 0;
        }
        if let Some(e) = snap.error {
            println!("error={e}");
            return probe::EXIT_SIGNALS_STALE;
        }
        probe::EXIT_SIGNALS_STALE
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let report = probe::run_probe();
        let _ = print_probe(&mut io::stdout(), &report);
        report.exit_code
    }
}

fn cmd_fix_folder() -> i32 {
    let mut cfg = Config::load();
    cfg.transcript_folder = interpres::config::default_transcript_folder();
    if let Err(e) = cfg.save() {
        eprintln!("Could not save settings: {e}");
        return 1;
    }
    println!(
        "Transcript folder reset to: {}",
        cfg.transcript_folder.display()
    );
    0
}

fn cmd_show_config() -> i32 {
    let cfg = Config::load();
    println!("remember={}", cfg.remember);
    println!("transcript_folder={}", cfg.transcript_folder.display());
    println!("write_jsonl={}", cfg.write_jsonl);
    println!("off_delay_ms={}", cfg.off_delay_ms);
    println!("poll_ms={}", cfg.poll_ms);
    println!("source={}", cfg.source);
    println!("debug={}", cfg.debug);
    println!(
        "helper_path={}",
        cfg.helper_path
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_default()
    );
    println!("config_file={}", interpres::config::config_path().display());
    0
}

fn cmd_set_folder(args: &[&str]) -> i32 {
    if args.is_empty() {
        eprintln!("Usage: interpres set-folder \"/path/to/folder\"");
        return 1;
    }
    let path = PathBuf::from(args.join(" "));
    let mut cfg = Config::load();
    cfg.transcript_folder = path.clone();
    if let Err(e) = cfg.save() {
        eprintln!("Could not save settings: {e}");
        return 1;
    }
    println!("Transcript folder set to: {}", path.display());
    println!("(Saved in {})", interpres::config::config_path().display());
    0
}

fn cmd_remember(args: &[&str]) -> i32 {
    let v = args.first().copied().unwrap_or("");
    let mut cfg = Config::load();
    match v {
        "on" | "true" | "yes" | "1" => cfg.remember = true,
        "off" | "false" | "no" | "0" => cfg.remember = false,
        _ => {
            eprintln!("Usage: interpres remember on|off");
            return 1;
        }
    }
    if let Err(e) = cfg.save() {
        eprintln!("Could not save settings: {e}");
        return 1;
    }
    println!(
        "Remember is now {}",
        if cfg.remember { "ON" } else { "OFF" }
    );
    if cfg.remember {
        println!(
            "Sessions will be saved under: {}",
            cfg.transcript_folder.display()
        );
    }
    0
}

fn cmd_demo(args: &[&str]) -> i32 {
    let mut cfg = Config::load();
    // demo always writes so users can see the file shape
    let force_remember = !args.iter().any(|a| *a == "--no-save");
    if force_remember {
        cfg.remember = true;
    }
    let source = "demo";
    let mut writer = match TranscriptWriter::begin_session(
        &cfg.transcript_folder,
        cfg.remember,
        cfg.write_jsonl,
        source,
        SystemTime::now(),
    ) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("Could not start session file: {e}");
            return 1;
        }
    };

    println!("Demo mode — sample captions (no Live Captions required).");
    if let Some(ref w) = writer {
        println!("Writing: {}", w.txt_path().display());
    } else {
        println!("Remember is off — not writing files. Try: interpres remember on");
    }

    let lines = [
        "Hello from Interpres demo mode.",
        "This is how a saved Live Captions session looks.",
        "Each session gets its own file with the date and time in the name.",
    ];
    for line in lines {
        println!("FINAL {line}");
        if let Some(ref mut w) = writer {
            let clock = format_clock(SystemTime::now());
            let _ = w.write_final(&clock, line);
        }
        thread::sleep(Duration::from_millis(50));
    }
    if let Some(ref mut w) = writer {
        let _ = w.end_session("demo_done");
        println!("Saved: {}", w.txt_path().display());
    }
    0
}

fn cmd_watch(_args: &[&str]) -> i32 {
    let cfg = Config::load();
    println!(
        "Watching for Live Captions (poll {} ms, off-delay {} ms). Ctrl+C to stop.",
        cfg.poll_ms, cfg.off_delay_ms
    );
    let stop = Arc::new(AtomicBool::new(false));
    let stop_c = stop.clone();
    ctrlc_flag(stop_c);

    watcher::run_watcher_loop(
        cfg.off_delay_ms,
        cfg.poll_ms,
        stop,
        |action| match action {
            LifecycleAction::Open => {
                println!("STATUS lc=running reason=watcher_open");
                println!("→ Live Captions is on — companion would capture now.");
            }
            LifecycleAction::Close => {
                println!("STATUS lc=stopped reason=watcher_close");
                println!("→ Live Captions is off — companion would stop.");
            }
            LifecycleAction::None => {}
        },
    );
    0
}

fn cmd_run(args: &[&str]) -> i32 {
    let mut cfg = Config::load();
    let mut demo = cfg.source == "demo";
    let mut helper: Option<PathBuf> = cfg.helper_path.clone();

    let mut i = 0;
    while i < args.len() {
        match args[i] {
            "--demo" => demo = true,
            "--remember" => cfg.remember = true,
            "--folder" => {
                i += 1;
                if i < args.len() {
                    cfg.transcript_folder = PathBuf::from(args[i]);
                }
            }
            "--helper" => {
                i += 1;
                if i < args.len() {
                    helper = Some(PathBuf::from(args[i]));
                }
            }
            "--jsonl" => cfg.write_jsonl = true,
            other => {
                eprintln!("Unknown flag: {other}");
                return 1;
            }
        }
        i += 1;
    }

    println!("Interpres — Live Captions companion");
    println!(
        "Remember: {} · Folder: {}",
        if cfg.remember { "ON" } else { "OFF" },
        cfg.transcript_folder.display()
    );
    if !cfg.remember {
        println!("Tip: turn on saving with  interpres remember on");
    }

    if demo {
        return cmd_demo(&[]);
    }

    // Prefer external helper if set; else in-process poll loop.
    if let Some(ref h) = helper {
        return run_with_helper(&cfg, h);
    }

    run_in_process(&cfg)
}

fn run_with_helper(cfg: &Config, helper: &std::path::Path) -> i32 {
    let mut host = match PluginHost::start(helper, &[]) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("Could not start helper {}: {e}", helper.display());
            eprintln!("Falling back to in-process capture.");
            return run_in_process(cfg);
        }
    };

    let mut writer = match TranscriptWriter::begin_session(
        &cfg.transcript_folder,
        cfg.remember,
        cfg.write_jsonl,
        source_label(),
        SystemTime::now(),
    ) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("Session file error: {e}");
            return 1;
        }
    };
    if let Some(ref w) = writer {
        println!("Session file: {}", w.txt_path().display());
    }

    let stop = Arc::new(AtomicBool::new(false));
    ctrlc_flag(stop.clone());
    println!("Capturing from helper. Ctrl+C to stop.");

    while !stop.load(Ordering::SeqCst) {
        while let Some(ev) = host.try_recv() {
            handle_event(&ev, &mut writer);
            if let CaptionEvent::Status {
                lc: LcState::Stopped,
                ..
            } = ev
            {
                stop.store(true, Ordering::SeqCst);
            }
        }
        thread::sleep(Duration::from_millis(50));
    }
    host.shutdown();
    if let Some(ref mut w) = writer {
        let _ = w.end_session("stopped");
        println!("Saved: {}", w.txt_path().display());
    }
    0
}

fn run_in_process(cfg: &Config) -> i32 {
    let mut cfg = cfg.clone();
    // Heal accidental temp folders from earlier tests.
    let folder_s = cfg.transcript_folder.to_string_lossy();
    if folder_s.contains("/var/folders/") || folder_s.contains("/tmp") || folder_s.contains("\\Temp")
    {
        cfg.transcript_folder = interpres::config::default_transcript_folder();
        let _ = cfg.save();
        println!(
            "Note: save folder was a temp path; reset to {}",
            cfg.transcript_folder.display()
        );
    }

    #[cfg(target_os = "macos")]
    {
        if !interpres::platform::macos::is_accessibility_trusted() {
            println!("Requesting macOS Accessibility permission…");
            let _ = interpres::platform::macos::request_accessibility_prompt();
            if !interpres::platform::macos::is_accessibility_trusted() {
                println!();
                println!(">>> Captions cannot be read until Accessibility is ON.");
                println!("    System Settings → Privacy & Security → Accessibility");
                println!("    Enable Terminal (or Interpres.app), then quit & reopen this app.");
                interpres::platform::macos::open_accessibility_settings();
            }
        }
    }

    let mut life = Lifecycle::new(cfg.off_delay_ms);
    let mut buffer = CaptionBuffer::new();
    buffer.stable_needed = 3;

    let mut writer: Option<TranscriptWriter> = None;
    let mut session_open = false;
    let mut err_hyst = interpres::ui_labels::CaptureErrorHysteresis::new();

    let stop = Arc::new(AtomicBool::new(false));
    ctrlc_flag(stop.clone());

    println!("Waiting for Live Captions… (Ctrl+C to stop)");
    if !cfg.remember {
        println!("Remember is OFF — you will see text here but nothing is saved to disk.");
        println!("  Turn on with:  interpres remember on");
    } else {
        println!("Saving to: {}", cfg.transcript_folder.display());
    }
    println!("If nothing appears, run:  interpres diagnose");

    while !stop.load(Ordering::SeqCst) {
        let snap = platform::poll_capture();
        let action = life.tick(snap.process_running, cfg.poll_ms);

        match action {
            LifecycleAction::Open => {
                println!(
                    "{}",
                    CaptionEvent::Status {
                        lc: LcState::Running,
                        reason: snap.detail.clone(),
                    }
                );
                if let Some(ref err) = snap.error {
                    println!(
                        "{}",
                        CaptionEvent::Error {
                            message: err.clone(),
                        }
                    );
                }
                if !session_open {
                    match TranscriptWriter::begin_session(
                        &cfg.transcript_folder,
                        cfg.remember,
                        cfg.write_jsonl,
                        source_label(),
                        SystemTime::now(),
                    ) {
                        Ok(w) => {
                            if let Some(ref wr) = w {
                                println!("Session file: {}", wr.txt_path().display());
                            } else if cfg.remember {
                                println!("(remember on but no file — check folder permissions)");
                            }
                            writer = w;
                            session_open = true;
                            buffer.reset();
                        }
                        Err(e) => eprintln!("Could not create session file: {e}"),
                    }
                }
            }
            LifecycleAction::Close => {
                println!(
                    "{}",
                    CaptionEvent::Status {
                        lc: LcState::Stopped,
                        reason: "live_captions_stopped".into(),
                    }
                );
                match buffer.finish() {
                    BufferEmit::Final(t) | BufferEmit::Partial(t) => {
                        println!("FINAL {t}");
                        if let Some(ref mut w) = writer {
                            let _ = w.write_final(&format_clock(SystemTime::now()), &t);
                        }
                    }
                    BufferEmit::Revised(t) => {
                        println!("REVISED {t}");
                        if let Some(ref mut w) = writer {
                            let _ = w.write_final(&format_clock(SystemTime::now()), &t);
                        }
                    }
                    BufferEmit::Finals(v) => {
                        for t in v {
                            println!("FINAL {t}");
                            if let Some(ref mut w) = writer {
                                let _ = w.write_final(&format_clock(SystemTime::now()), &t);
                            }
                        }
                    }
                    BufferEmit::None => {}
                }
                if let Some(ref mut w) = writer {
                    let _ = w.end_session("lc_stopped");
                    println!("Saved: {}", w.txt_path().display());
                }
                writer = None;
                session_open = false;
            }
            LifecycleAction::None => {}
        }

        if life.companion_active {
            let surface_ok = snap.surface_text.as_ref().is_some_and(|s| !s.trim().is_empty());
            let err_tick = err_hyst.on_poll(surface_ok, snap.error.as_deref());
            if err_tick.clear_error {
                println!(
                    "{}",
                    CaptionEvent::Status {
                        lc: LcState::Running,
                        reason: "capture_ok".into(),
                    }
                );
            } else if err_tick.show_hard_error {
                if let Some(ref err) = err_tick.message {
                    println!(
                        "{}",
                        CaptionEvent::Status {
                            lc: LcState::Degraded,
                            reason: err.clone(),
                        }
                    );
                    println!("  Still no caption text. Run: interpres diagnose");
                }
            }
            if let Some(ref surface) = snap.surface_text {
                match buffer.observe(surface) {
                    BufferEmit::Partial(t) => {
                        println!("PARTIAL {t}");
                    }
                    BufferEmit::Final(t) => {
                        println!("FINAL {t}");
                        if let Some(ref mut w) = writer {
                            let _ = w.write_final(&format_clock(SystemTime::now()), &t);
                        }
                    }
                    BufferEmit::Revised(t) => {
                        println!("REVISED {t}");
                        if let Some(ref mut w) = writer {
                            let _ = w.write_final(&format_clock(SystemTime::now()), &t);
                        }
                    }
                    BufferEmit::Finals(v) => {
                        for t in v {
                            println!("FINAL {t}");
                            if let Some(ref mut w) = writer {
                                let _ = w.write_final(&format_clock(SystemTime::now()), &t);
                            }
                        }
                    }
                    BufferEmit::None => {}
                }
            }
        }

        thread::sleep(Duration::from_millis(cfg.poll_ms.max(300)));
    }

    if let Some(ref mut w) = writer {
        let _ = w.end_session("user");
        println!("Saved: {}", w.txt_path().display());
    }
    0
}

fn handle_event(ev: &CaptionEvent, writer: &mut Option<TranscriptWriter>) {
    match ev {
        CaptionEvent::Partial { text } => println!("PARTIAL {text}"),
        CaptionEvent::Final { text } => {
            println!("FINAL {text}");
            if let Some(w) = writer.as_mut() {
                let _ = w.write_final(&format_clock(SystemTime::now()), text);
            }
        }
        CaptionEvent::Status { lc, reason } => {
            println!("STATUS lc={} reason={reason}", lc.as_str());
        }
        CaptionEvent::Error { message } => println!("ERROR {message}"),
        CaptionEvent::Ready => println!("READY"),
        CaptionEvent::Log { level, message } => println!("LOG {level}: {message}"),
        _ => {}
    }
}

fn source_label() -> &'static str {
    #[cfg(windows)]
    {
        "Windows Live Captions"
    }
    #[cfg(target_os = "macos")]
    {
        "macOS Live Captions"
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        "Live Captions"
    }
}

/// Best-effort Ctrl+C without crates: SIGINT on Unix, SetConsoleCtrlHandler on Windows.
fn ctrlc_flag(stop: Arc<AtomicBool>) {
    #[cfg(unix)]
    {
        // SAFETY: simple flag set on SIGINT
        unsafe {
            STOP_PTR = Some(stop);
            libc_signal(SIGINT, handle_sigint);
        }
    }
    #[cfg(windows)]
    {
        // SAFETY: handler only stores a flag; process continues until the run loop exits.
        unsafe {
            STOP_PTR = Some(stop);
            let _ = SetConsoleCtrlHandler(Some(handle_ctrl), 1);
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = stop;
    }
}

#[cfg(any(unix, windows))]
static mut STOP_PTR: Option<Arc<AtomicBool>> = None;

#[cfg(unix)]
const SIGINT: i32 = 2;

#[cfg(unix)]
extern "C" fn handle_sigint(_: i32) {
    unsafe {
        if let Some(ref s) = STOP_PTR {
            s.store(true, Ordering::SeqCst);
        }
    }
}

#[cfg(unix)]
extern "C" {
    fn signal(sig: i32, handler: extern "C" fn(i32)) -> extern "C" fn(i32);
}

#[cfg(unix)]
unsafe fn libc_signal(sig: i32, handler: extern "C" fn(i32)) {
    let _ = signal(sig, handler);
}

#[cfg(windows)]
#[link(name = "kernel32")]
extern "system" {
    fn SetConsoleCtrlHandler(
        handler: Option<unsafe extern "system" fn(u32) -> i32>,
        add: i32,
    ) -> i32;
}

/// CTRL_C_EVENT=0, CTRL_BREAK=1, CTRL_CLOSE=2 — return TRUE if handled.
#[cfg(windows)]
unsafe extern "system" fn handle_ctrl(ctrl_type: u32) -> i32 {
    if matches!(ctrl_type, 0 | 1 | 2) {
        if let Some(ref s) = STOP_PTR {
            s.store(true, Ordering::SeqCst);
        }
        return 1;
    }
    0
}
