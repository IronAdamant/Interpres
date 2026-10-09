//! Native desktop UI entry.
//! - macOS: AppKit via system clang (`native/macos/`), driven from `mac` below
//! - Windows: Win32 via hand-written FFI (`gui_win.rs`)
//!
//! Both windows show the same thing: `app_view::AppModel` decides, the OS code draws.

/// Run the native GUI (blocks until the window is closed).
pub fn run_native_gui() -> i32 {
    #[cfg(target_os = "macos")]
    {
        return mac::run_macos_gui();
    }
    #[cfg(windows)]
    {
        return crate::gui_win::run_windows_gui();
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        eprintln!("Native window UI is available on Windows and macOS.");
        eprintln!("Use: interpres run");
        1
    }
}

#[cfg(target_os = "macos")]
mod mac {
    use crate::app_view::{first_changed_row, AppModel, LcAction, LC_NAME};
    use crate::config::Config;
    use crate::platform;
    use crate::theme::ThemeMode;
    use std::ffi::{CStr, CString};
    use std::os::raw::{c_char, c_int, c_long, c_void};
    use std::path::{Path, PathBuf};
    use std::ptr;
    use std::time::Instant;

    /// Command-line flag used by the login item: open minimized, don't steal focus.
    const MINIMIZED_ARG: &str = "--minimized";
    /// LaunchAgent label (also the plist file name).
    const LOGIN_LABEL: &str = "org.interpres.app";

    // Button ids — mirror INTERPRES_CMD_* in native/macos/interpres_gui.h.
    const CMD_TOGGLE: c_int = 1;
    const CMD_ACTION: c_int = 2;
    const CMD_SETTINGS: c_int = 3;
    const CMD_OPEN_FILE: c_int = 4;
    const CMD_COPY: c_int = 5;
    const CMD_OPEN_FOLDER: c_int = 6;
    const CMD_AUTO: c_int = 7;
    const CMD_PREFERENCES: c_int = 8;

    // Settings menu ids (Rust-only; the menu is built here).
    const IDM_SAVE: c_int = 101;
    const IDM_FOLDER: c_int = 102;
    const IDM_THEME_SYSTEM: c_int = 104;
    const IDM_THEME_LIGHT: c_int = 105;
    const IDM_THEME_DARK: c_int = 106;
    const IDM_CHECK: c_int = 107;
    const IDM_DEBUG: c_int = 108;
    const IDM_RESTART_LC: c_int = 109;
    const IDM_SOURCE_LC: c_int = 110;
    const IDM_SOURCE_ENGINE: c_int = 111;
    const IDM_EDIT_SETTINGS: c_int = 112;
    const IDM_LOGIN: c_int = 113;
    const IDM_ACCESSIBILITY: c_int = 114;
    const IDM_LC_SETTINGS: c_int = 115;

    #[repr(C)]
    struct InterpresGuiCallbacks {
        user: *mut c_void,
        on_command: Option<extern "C" fn(*mut c_void, c_int)>,
        on_tick: Option<extern "C" fn(*mut c_void)>,
        on_ready: Option<extern "C" fn(*mut c_void)>,
        on_quit: Option<extern "C" fn(*mut c_void)>,
        on_appearance: Option<extern "C" fn(*mut c_void)>,
    }

    #[repr(C)]
    struct InterpresMenuItem {
        id: c_int,
        title: *const c_char,
        checked: c_int,
        enabled: c_int,
    }

    extern "C" {
        fn interpres_gui_main(callbacks: InterpresGuiCallbacks, start_minimized: c_int) -> c_int;
        fn interpres_gui_set_title(text: *const c_char);
        fn interpres_gui_set_banner(head: *const c_char, guidance: *const c_char, tone: c_int);
        fn interpres_gui_set_toggle(label: *const c_char, enabled: c_int, recording: c_int);
        fn interpres_gui_set_action(label: *const c_char);
        fn interpres_gui_set_checks(text: *const c_char);
        fn interpres_gui_set_detail(text: *const c_char);
        fn interpres_gui_set_footer(text: *const c_char);
        fn interpres_gui_set_enabled(open_file: c_int, copy: c_int);
        fn interpres_gui_set_auto(on: c_int);
        fn interpres_gui_set_theme(mode: c_int);
        fn interpres_gui_transcript_replace_tail(start: c_long, tail: *const c_char);
        fn interpres_gui_show_menu(items: *const InterpresMenuItem, count: c_int) -> c_int;
        fn interpres_gui_attention(critical: c_int);
        fn interpres_gui_copy_text(text: *const c_char) -> c_int;
        fn interpres_gui_pick_folder(buf: *mut c_char, buflen: c_int) -> c_int;
    }

    /// Main-thread-only state (AppKit calls every callback on the main thread).
    static mut APP: *mut AppModel = ptr::null_mut();
    /// True while a modal loop (menu, folder picker) or a pump runs — skip re-entry.
    static mut BUSY: bool = false;

    fn with_app<R>(f: impl FnOnce(&mut AppModel) -> R) -> Option<R> {
        unsafe {
            let app = APP;
            if app.is_null() {
                None
            } else {
                Some(f(&mut *app))
            }
        }
    }

    fn c_string(s: &str) -> CString {
        CString::new(s.replace('\0', "")).unwrap_or_default()
    }

    pub fn run_macos_gui() -> i32 {
        let mut cfg = Config::load();
        let fs = cfg.transcript_folder.to_string_lossy();
        if fs.contains("/var/folders/") || fs.contains("/tmp") {
            cfg.transcript_folder = crate::config::default_transcript_folder();
            let _ = cfg.save();
        }

        crate::debuglog::init_from_config(cfg.debug, &cfg.transcript_folder);
        crate::debuglog::log("gui open (macos)");

        let start_minimized = std::env::args().any(|a| a == MINIMIZED_ARG);
        // Portable app: if Interpres moved, point the login item at this copy.
        if login_item_exists() && !login_item_current() {
            set_open_at_login(true);
        }

        let app = Box::new(AppModel::new(&cfg));
        unsafe {
            APP = Box::into_raw(app);
        }
        let cbs = InterpresGuiCallbacks {
            user: ptr::null_mut(),
            on_command: Some(cb_command),
            on_tick: Some(cb_tick),
            on_ready: Some(cb_ready),
            on_quit: Some(cb_quit),
            on_appearance: None,
        };
        let code = unsafe { interpres_gui_main(cbs, start_minimized as c_int) };

        // Normally the process exits inside NSApp terminate; this covers a plain return.
        unsafe {
            if !APP.is_null() {
                let app = Box::from_raw(APP);
                APP = ptr::null_mut();
                app.engine.stop();
            }
        }
        platform::shutdown_capture();
        code
    }

    extern "C" fn cb_ready(_user: *mut c_void) {
        with_app(|app| {
            unsafe {
                interpres_gui_set_theme(app.theme_mode.as_int());
            }
            if !app.engine_mode && !platform::macos::is_accessibility_trusted() {
                app.view.detail = "⚠  Allow Interpres under System Settings → Privacy & Security → Accessibility so it can read Live Captions (Settings ▾ → Accessibility permission…).".into();
            }
            crate::debuglog::log(&format!(
                "ui ready folder={} remember={} debug={} theme={}",
                app.engine.folder().display(),
                app.remember,
                app.debug,
                app.theme_mode.as_str()
            ));
        });
    }

    extern "C" fn cb_tick(_user: *mut c_void) {
        pump_ui();
    }

    extern "C" fn cb_quit(_user: *mut c_void) {
        crate::debuglog::log("ui quit");
        with_app(|app| app.shutdown());
    }

    extern "C" fn cb_command(_user: *mut c_void, id: c_int) {
        on_command(id);
    }

    fn pump_ui() {
        unsafe {
            if BUSY {
                return;
            }
            BUSY = true;
        }
        with_app(|app| {
            let att = app.pump();
            if att.alert || att.nudge {
                unsafe { interpres_gui_attention(att.alert as c_int) };
            }
            if app.transcript_due() {
                render_transcript(app);
            }
            refresh_static_ui(app);
        });
        unsafe {
            BUSY = false;
        }
    }

    /// Run `f` with the pump paused (menus and dialogs run their own event loop).
    fn modal<R>(f: impl FnOnce() -> R) -> R {
        let was = unsafe { BUSY };
        unsafe { BUSY = true };
        let r = f();
        unsafe { BUSY = was };
        r
    }

    fn refresh_static_ui(app: &mut AppModel) {
        let b = app.banner();
        let key = format!("{}\n{}\n{}", b.head, b.guidance, b.tone.as_int());
        if key != app.view.rendered_banner {
            let (h, g) = (c_string(&b.head), c_string(&b.guidance));
            unsafe { interpres_gui_set_banner(h.as_ptr(), g.as_ptr(), b.tone.as_int()) };
            app.view.rendered_banner = key;
        }
        let title = c_string(&app.window_title());
        let toggle = c_string(app.toggle_label());
        let action = c_string(app.action_label().unwrap_or(""));
        let checks = c_string(&app.checklist());
        let detail = c_string(&app.view.detail);
        let footer = c_string(&app.footer());
        unsafe {
            interpres_gui_set_title(title.as_ptr());
            interpres_gui_set_toggle(
                toggle.as_ptr(),
                !app.view.stopping as c_int,
                app.view.listening as c_int,
            );
            interpres_gui_set_action(action.as_ptr());
            interpres_gui_set_checks(checks.as_ptr());
            interpres_gui_set_detail(detail.as_ptr());
            interpres_gui_set_footer(footer.as_ptr());
            interpres_gui_set_enabled(
                app.view.session_path.is_some() as c_int,
                !app.view.lines.is_empty() as c_int,
            );
            interpres_gui_set_auto(app.auto_on as c_int);
        }
    }

    /// Replace only the changed tail of the transcript (cheap for hours of lines).
    fn render_transcript(app: &mut AppModel) {
        let rows = app.transcript_rows();
        let v = &mut app.view;
        let k = first_changed_row(&v.rendered_rows, &rows);
        if k == rows.len() && k == v.rendered_rows.len() {
            v.transcript_dirty = false;
            return;
        }
        // NSString offsets are UTF-16 units, like the Windows edit control.
        let start: usize = v.rendered_u16[..k].iter().sum();
        let tail = c_string(&rows[k..].concat());
        unsafe { interpres_gui_transcript_replace_tail(start as c_long, tail.as_ptr()) };
        v.rendered_u16 = rows.iter().map(|r| r.encode_utf16().count()).collect();
        v.rendered_rows = rows;
        v.transcript_dirty = false;
        v.last_render = Some(Instant::now());
    }

    fn open_path(path: &Path) {
        let _ = std::process::Command::new("open").arg(path).status();
    }

    fn on_command(id: c_int) {
        match id {
            CMD_TOGGLE => {
                with_app(|app| app.toggle_recording());
            }
            CMD_ACTION => {
                with_app(|app| {
                    let action = app.lc_action();
                    app.run_lc_action(action);
                });
            }
            IDM_RESTART_LC => {
                with_app(|app| app.run_lc_action(LcAction::Restart));
            }
            IDM_LC_SETTINGS => {
                let _ = platform::macos::open_live_captions_settings();
                with_app(|app| {
                    app.view.detail = "Opened Live Captions settings.".into();
                });
            }
            IDM_ACCESSIBILITY => {
                let _ = platform::macos::request_accessibility_prompt();
                platform::macos::open_accessibility_settings();
                with_app(|app| {
                    app.view.detail = "Turn on Interpres in the Accessibility list. If it is already on, switch it off and on again, then reopen Interpres.".into();
                });
            }
            CMD_AUTO => {
                with_app(|app| app.toggle_auto_record());
            }
            CMD_SETTINGS | CMD_PREFERENCES => show_settings_menu(),
            CMD_OPEN_FILE => {
                if let Some(Some(p)) = with_app(|app| app.view.session_path.clone()) {
                    open_path(&p);
                }
            }
            CMD_COPY => {
                with_app(|app| {
                    let text = app.plain_transcript();
                    let msg = if text.is_empty() {
                        "Nothing to copy yet."
                    } else {
                        let c = c_string(&text);
                        if unsafe { interpres_gui_copy_text(c.as_ptr()) } != 0 {
                            "Transcript copied — paste it into your notes."
                        } else {
                            "⚠  Could not copy to the clipboard."
                        }
                    };
                    app.view.detail = msg.into();
                });
            }
            CMD_OPEN_FOLDER => {
                if let Some(folder) = with_app(|app| app.engine.folder()) {
                    let _ = std::fs::create_dir_all(&folder);
                    open_path(&folder);
                }
            }
            IDM_SAVE => {
                with_app(|app| app.toggle_save());
            }
            IDM_FOLDER => {
                if let Some(path) = modal(pick_folder) {
                    with_app(|app| app.set_folder(&path));
                }
            }
            IDM_THEME_SYSTEM | IDM_THEME_LIGHT | IDM_THEME_DARK => {
                let mode = match id {
                    IDM_THEME_LIGHT => ThemeMode::Light,
                    IDM_THEME_DARK => ThemeMode::Dark,
                    _ => ThemeMode::System,
                };
                with_app(|app| {
                    app.set_theme(mode);
                    // Neutral banner colours follow the theme.
                    app.view.rendered_banner.clear();
                });
                unsafe { interpres_gui_set_theme(mode.as_int()) };
            }
            IDM_DEBUG => {
                with_app(|app| app.toggle_debug());
            }
            IDM_CHECK => {
                let trusted = platform::macos::request_accessibility_prompt();
                with_app(|app| {
                    app.run_setup_check();
                    if !trusted && !app.engine_mode {
                        app.view.detail = "⚠  Accessibility is off for Interpres — turn it on in the list that just opened, then Check again.".into();
                    }
                });
                if !trusted {
                    platform::macos::open_accessibility_settings();
                }
            }
            IDM_SOURCE_LC | IDM_SOURCE_ENGINE => {
                with_app(|app| app.set_source(id == IDM_SOURCE_ENGINE));
            }
            IDM_LOGIN => {
                let on = !login_item_exists();
                let ok = set_open_at_login(on);
                crate::debuglog::log(&format!("ui open at login on={on} ok={ok}"));
                with_app(|app| {
                    app.view.detail = match (ok, on) {
                        (true, true) => "Interpres will open (minimized) when you log in to your Mac, so auto-record is always ready.".into(),
                        (true, false) => "Interpres will no longer open at login.".into(),
                        (false, _) => "⚠  Could not change the login setting.".into(),
                    };
                });
            }
            IDM_EDIT_SETTINGS => {
                let path = crate::config::config_path();
                if !path.exists() {
                    let _ = Config::load().save();
                }
                let _ = std::process::Command::new("open").arg("-t").arg(&path).status();
                with_app(|app| {
                    app.view.detail =
                        "Opened the settings file. Changes apply next time you press Start recording.".into()
                });
            }
            _ => {}
        }
        pump_ui();
    }

    fn show_settings_menu() {
        let Some((remember, debug, theme, engine_mode, engine_configured, engine_name)) =
            with_app(|app| {
                (
                    app.remember,
                    app.debug,
                    app.theme_mode,
                    app.engine_mode,
                    app.engine_configured,
                    app.engine_name.clone(),
                )
            })
        else {
            return;
        };
        let engine_label = if engine_configured {
            format!("Captions from: external engine ({engine_name})")
        } else {
            "Captions from: external engine (set it up in the settings file)".into()
        };
        let lc = !engine_mode;
        // (id, title, checked, enabled); id 0 = separator.
        let rows: Vec<(c_int, String, bool, bool)> = vec![
            (IDM_SAVE, "Save transcripts to disk".into(), remember, true),
            (IDM_FOLDER, "Change transcripts folder…".into(), false, true),
            (CMD_OPEN_FOLDER, "Open transcripts folder".into(), false, true),
            (0, String::new(), false, false),
            (IDM_SOURCE_LC, format!("Captions from: {LC_NAME}"), !engine_mode, true),
            (IDM_SOURCE_ENGINE, engine_label, engine_mode, engine_configured),
            (
                IDM_LOGIN,
                "Open Interpres at login (keeps auto-record ready)".into(),
                login_item_exists(),
                true,
            ),
            (IDM_EDIT_SETTINGS, "Edit settings file…".into(), false, true),
            (0, String::new(), false, false),
            (IDM_CHECK, "Check Live Captions setup".into(), false, lc),
            (IDM_LC_SETTINGS, "Open Live Captions settings".into(), false, lc),
            (IDM_RESTART_LC, "Restart Live Captions".into(), false, lc),
            (IDM_ACCESSIBILITY, "Accessibility permission…".into(), false, lc),
            (0, String::new(), false, false),
            (IDM_THEME_SYSTEM, "Theme: match macOS".into(), theme == ThemeMode::System, true),
            (IDM_THEME_LIGHT, "Theme: light".into(), theme == ThemeMode::Light, true),
            (IDM_THEME_DARK, "Theme: dark".into(), theme == ThemeMode::Dark, true),
            (0, String::new(), false, false),
            (IDM_DEBUG, "Write debug log (for troubleshooting)".into(), debug, true),
        ];
        let titles: Vec<CString> = rows.iter().map(|r| c_string(&r.1)).collect();
        let items: Vec<InterpresMenuItem> = rows
            .iter()
            .zip(&titles)
            .map(|((id, _, checked, enabled), t)| InterpresMenuItem {
                id: *id,
                title: t.as_ptr(),
                checked: *checked as c_int,
                enabled: *enabled as c_int,
            })
            .collect();
        let cmd = modal(|| unsafe { interpres_gui_show_menu(items.as_ptr(), items.len() as c_int) });
        if cmd != 0 {
            on_command(cmd);
        }
    }

    fn pick_folder() -> Option<String> {
        let mut buf = vec![0 as c_char; 4096];
        let ok = unsafe { interpres_gui_pick_folder(buf.as_mut_ptr(), buf.len() as c_int) };
        if ok == 0 {
            return None;
        }
        let path = unsafe { CStr::from_ptr(buf.as_ptr()) }
            .to_string_lossy()
            .into_owned();
        (!path.is_empty()).then_some(path)
    }

    // ---------- Open at login (per-user LaunchAgent; no admin, no helper app) ----------

    fn login_plist_path() -> Option<PathBuf> {
        std::env::var_os("HOME").map(|h| {
            PathBuf::from(h)
                .join("Library/LaunchAgents")
                .join(format!("{LOGIN_LABEL}.plist"))
        })
    }

    fn xml_escape(s: &str) -> String {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
    }

    /// What the login item runs: this binary (inside Interpres.app when packaged), minimized.
    pub(super) fn login_plist_text(exe: &Path) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{LOGIN_LABEL}</string>
  <key>ProgramArguments</key>
  <array>
    <string>{}</string>
    <string>gui</string>
    <string>{MINIMIZED_ARG}</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>ProcessType</key>
  <string>Interactive</string>
  <key>LimitLoadToSessionType</key>
  <string>Aqua</string>
</dict>
</plist>
"#,
            xml_escape(&exe.display().to_string())
        )
    }

    fn login_item_exists() -> bool {
        login_plist_path().is_some_and(|p| p.is_file())
    }

    /// The login item points at this copy of Interpres.
    fn login_item_current() -> bool {
        let (Some(p), Ok(exe)) = (login_plist_path(), std::env::current_exe()) else {
            return false;
        };
        std::fs::read_to_string(p).is_ok_and(|t| t == login_plist_text(&exe))
    }

    /// Add or remove the login item. Takes effect at the next login.
    fn set_open_at_login(on: bool) -> bool {
        let Some(path) = login_plist_path() else {
            return false;
        };
        if !on {
            return match std::fs::remove_file(&path) {
                Ok(()) => true,
                Err(e) => e.kind() == std::io::ErrorKind::NotFound,
            };
        }
        let Ok(exe) = std::env::current_exe() else {
            return false;
        };
        if let Some(dir) = path.parent() {
            if std::fs::create_dir_all(dir).is_err() {
                return false;
            }
        }
        std::fs::write(&path, login_plist_text(&exe)).is_ok()
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use std::path::Path;

    #[test]
    fn login_item_runs_this_binary_minimized() {
        let t = super::mac::login_plist_text(Path::new("/Applications/Interpres & Co.app/Contents/MacOS/Interpres"));
        assert!(t.contains("<string>/Applications/Interpres &amp; Co.app/Contents/MacOS/Interpres</string>"));
        assert!(t.contains("<string>gui</string>\n    <string>--minimized</string>"));
        assert!(t.contains("<key>RunAtLoad</key>\n  <true/>"));
    }
}
