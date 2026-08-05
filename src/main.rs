//! Interpres CLI — OS Live Captions companion (strict zero-dependency).

use interpres::buffer::{BufferEmit, CaptionBuffer};
use interpres::config::Config;
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
    // Double-click / .app launchers set this so we print a short welcome.
    if env::var_os("INTERPRES_FRIENDLY").is_some() && args.is_empty() {
        println!("Interpres is starting…");
        println!("Leave Live Captions on while this runs. Ctrl+C (or close the window) to stop.\n");
        args.push("run".into());
    }
    if args.is_empty() {
        print_help();
        // Friendly default for non-technical users: run companion
        args.push("run".into());
    }

    let cmd = args[0].as_str();
    let rest: Vec<&str> = args[1..].iter().map(|s| s.as_str()).collect();

    let code = match cmd {
        "help" | "-h" | "--help" => {
            print_help();
            0
        }
        "version" | "-V" | "--version" => {
            println!("interpres {}", env!("CARGO_PKG_VERSION"));
            0
        }
        "probe" => cmd_probe(),
        "run" => cmd_run(&rest),
        "watch" => cmd_watch(&rest),
        "set-folder" => cmd_set_folder(&rest),
        "show-config" => cmd_show_config(),
        "remember" => cmd_remember(&rest),
        "demo" => cmd_demo(&rest),
        other => {
            eprintln!("Unknown command: {other}");
            print_help();
            1
        }
    };
    std::process::exit(code);
}

fn print_help() {
    println!(
        r#"Interpres — save Windows & macOS Live Captions as transcripts

Easy path:
  1. Turn on Live Captions (Windows: Win+Ctrl+L · Mac: System Settings → Accessibility → Live Captions)
  2. Run:  interpres run
  3. Optional: interpres set-folder "/path/to/My Transcripts"
  4. Optional: interpres remember on

Commands:
  run              Capture captions while Live Captions is on (default)
  probe            Check if Live Captions is visible to Interpres
  watch            Stay in the background; print open/close when Live Captions starts/stops
  set-folder PATH  Choose where dated session files are saved (sticky)
  remember on|off  Save transcripts to disk (default: off)
  show-config      Print current settings
  demo             Write a sample session without Live Captions (for testing)
  help             This message

Files are named like 2026-08-05_14-22-01.txt — one file per session.

Free & open source. 100% local. Not a paid product.
"#
    );
}

fn cmd_probe() -> i32 {
    let report = probe::run_probe();
    let _ = print_probe(&mut io::stdout(), &report);
    report.exit_code
}

fn cmd_show_config() -> i32 {
    let cfg = Config::load();
    println!("remember={}", cfg.remember);
    println!("transcript_folder={}", cfg.transcript_folder.display());
    println!("write_jsonl={}", cfg.write_jsonl);
    println!("off_delay_ms={}", cfg.off_delay_ms);
    println!("poll_ms={}", cfg.poll_ms);
    println!("source={}", cfg.source);
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
    let mut life = Lifecycle::new(cfg.off_delay_ms);
    let mut buffer = CaptionBuffer::new();
    buffer.stable_needed = 3;

    let mut writer: Option<TranscriptWriter> = None;
    let mut session_open = false;

    let stop = Arc::new(AtomicBool::new(false));
    ctrlc_flag(stop.clone());

    println!("Waiting for Live Captions… (Ctrl+C to stop)");
    println!("Run  interpres probe  if nothing happens.");

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
                if snap.error.is_some() {
                    println!(
                        "{}",
                        CaptionEvent::Error {
                            message: snap.error.clone().unwrap_or_default(),
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
                // flush buffer
                match buffer.finish() {
                    BufferEmit::Final(t) => {
                        println!("FINAL {t}");
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
                    _ => {}
                }
                if let Some(ref mut w) = writer {
                    let _ = w.end_session("lc_stopped");
                    println!("Saved: {}", w.txt_path().display());
                }
                writer = None;
                session_open = false;
                // Stay running to wait for next LC open (appliance mode)
            }
            LifecycleAction::None => {}
        }

        if life.companion_active {
            if let Some(ref err) = snap.error {
                // Emit once-ish: print degraded each poll is noisy — throttle simply
                static mut LAST: bool = false;
                let show = unsafe {
                    let was = LAST;
                    LAST = true;
                    !was
                };
                if show {
                    println!(
                        "{}",
                        CaptionEvent::Status {
                            lc: LcState::Degraded,
                            reason: err.clone(),
                        }
                    );
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

/// Best-effort Ctrl+C without crates: install no handler on unsupported, use flag via SIGINT on unix.
fn ctrlc_flag(stop: Arc<AtomicBool>) {
    #[cfg(unix)]
    {
        // SAFETY: simple flag set on SIGINT
        unsafe {
            STOP_PTR = Some(stop);
            libc_signal(SIGINT, handle_sigint);
        }
    }
    #[cfg(not(unix))]
    {
        let _ = stop;
        // On Windows without crates, user closes the console or kills the process.
    }
}

#[cfg(unix)]
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
