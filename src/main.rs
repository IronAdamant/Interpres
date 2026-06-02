//! Interpres — Zero-Dep Core + optional egui GUI (Phases 2–3)

// Crate-level conditional attributes (must be at the very top, before any other items).
// These select the Windows subsystem for the .exe depending on whether we are producing
// a debug/test build (with "debug-logs" feature or debug profile) or a normal release GUI.
// Using cfg_attr so it only emits the attribute for matching builds and does not hide
// any other items (like the const below).
#![cfg_attr(
    all(target_os = "windows", feature = "gui", any(feature = "debug-logs", debug_assertions)),
    windows_subsystem = "console"
)]
#![cfg_attr(
    all(target_os = "windows", feature = "gui", not(any(feature = "debug-logs", debug_assertions))),
    windows_subsystem = "windows"
)]

mod audio;
mod engine;
mod plugins;
mod server;
mod state;
mod util;

#[cfg(feature = "gui")]
mod gui;

const DEFAULT_SERVER_PORT: u16 = 43123;

fn load_config() -> util::config::AppConfig {
    let cfg = util::config::AppConfig::load();
    if let Ok(path) = util::config::config_path() {
        if !path.exists() {
            let _ = cfg.save();
        }
    }
    cfg
}

#[cfg(not(feature = "gui"))]
fn main() {
    // Load debug logging features if this binary was compiled with them (or in debug profile).
    // This creates/appends interpres-test-debug.log.txt next to the .exe with *everything*.
    let _ = crate::util::logging::init_debug_log_file();
    #[cfg(any(feature = "debug-logs", debug_assertions))]
    crate::util::logging::install_debug_panic_hook();

    println!("INTERPRES — Zero-Dependency Core");
    println!("Thin client + plugin pipeline (headless)");
    crate::util::logging::log("info", "headless core starting");

    let config = load_config();
    match engine::start_core(DEFAULT_SERVER_PORT, config) {
        Ok(handle) => {
            if handle.broadcast_tx.is_some() {
                println!("Capture active. Thin client on :{}", DEFAULT_SERVER_PORT);
                crate::util::logging::log("info", &format!("capture active, thin client on :{}", DEFAULT_SERVER_PORT));
            } else {
                println!("Capture active (thin-client server unavailable)");
                crate::util::logging::log("warn", "capture active but no thin-client server (bind failed?)");
            }
            engine::run_until_shutdown(&handle);
        }
        Err(e) => {
            eprintln!("Failed to start core: {}", e);
            crate::util::logging::log("error", &format!("failed to start core: {}", e));
            std::process::exit(1);
        }
    }
}

#[cfg(feature = "gui")]
fn main() {
    // Load debug logging features if this binary was compiled with them (or in debug profile).
    // This creates/appends interpres-test-debug.log.txt next to the .exe with *everything*.
    let _ = crate::util::logging::init_debug_log_file();
    #[cfg(any(feature = "debug-logs", debug_assertions))]
    crate::util::logging::install_debug_panic_hook();

    let args: Vec<String> = std::env::args().collect();
    let subtitles_only = args.iter().any(|a| {
        a == "--subtitles-only" || a == "--pure" || a == "--floating-only" || a == "--overlay"
    });
    let overlay_alias_used = args.iter().any(|a| a == "--overlay")
        && !args.iter().any(|a| a == "--subtitles-only");

    println!("INTERPRES — GUI + floating always-on-top subtitles");
    crate::util::logging::log("info", "GUI starting (eframe + floating overlay support)");

    if subtitles_only {
        println!("Launch mode: pure floating subtitles only (giant text primary; almost no UI surface)");
        crate::util::logging::log("info", "launch mode: --subtitles-only (pure floating subs primary)");
        if overlay_alias_used {
            println!("  (note: --overlay accepted as alias for --subtitles-only / --pure / --floating-only; prefer the explicit --subtitles-only)");
        }
    }

    let config = load_config();
    let core = match engine::start_core(DEFAULT_SERVER_PORT, config.clone()) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("Failed to start core: {}", e);
            std::process::exit(1);
        }
    };

    if subtitles_only {
        // Auto-enable so live transcription "just works" immediately for the pure subs launch (no control UI to toggle START SESSION).
        core.session_active.store(true, std::sync::atomic::Ordering::Relaxed);
    }

    let server_port = core
        .server_handle
        .as_ref()
        .map(|h| h.port())
        .unwrap_or(DEFAULT_SERVER_PORT);

    let gui_cfg = gui::GuiConfig {
        subtitles_only,
        server_port,
        font_scale: config.font_scale,
        max_contrast: config.max_contrast,
        show_onboarding: !config.onboarding_complete,
        minimal_hud: subtitles_only || config.minimal_hud,
        passthrough: config.passthrough && subtitles_only,
    };

    let shutdown = core.shutdown.clone();
    if let Err(e) = gui::run(&core, gui_cfg, &config) {
        eprintln!("GUI exited with error: {}", e);
        crate::util::logging::log("error", &format!("GUI exited with error: {}", e));
        util::shutdown::request(&shutdown);
        std::process::exit(1);
    }
    util::shutdown::request(&shutdown);
    engine::run_until_shutdown(&core);
}
