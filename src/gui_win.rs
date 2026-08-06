//! Windows native UI — hand-written Win32 (user32/gdi32/shell32). Zero crates.io.
//!
//! Mirrors the macOS AppKit control set; talks to the same CaptureEngine core.

use crate::buffer::same_or_refinement;
use crate::config::Config;
use crate::engine::{CaptureEngine, EngineEvent};
use crate::probe;
use crate::ui_labels::{folder_label, remember_toggle_status, session_footer};
use std::os::raw::{c_int, c_void};
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

type Hwnd = *mut c_void;
type Hinstance = *mut c_void;
type Hbrush = *mut c_void;
type Hfont = *mut c_void;
type Hmenu = *mut c_void;
type Hdc = *mut c_void;
type Lparam = isize;
type Wparam = usize;
type Lresult = isize;

const WM_DESTROY: u32 = 0x0002;
const WM_SIZE: u32 = 0x0005;
const WM_CLOSE: u32 = 0x0010;
const WM_COMMAND: u32 = 0x0111;
const WM_TIMER: u32 = 0x0113;
const WM_SETFONT: u32 = 0x0030;
const WM_CTLCOLORSTATIC: u32 = 0x0138;
const WM_CTLCOLOREDIT: u32 = 0x0133;
const WS_OVERLAPPEDWINDOW: u32 = 0x00CF_0000;
const WS_VISIBLE: u32 = 0x1000_0000;
const WS_CHILD: u32 = 0x4000_0000;
const WS_TABSTOP: u32 = 0x0001_0000;
const WS_VSCROLL: u32 = 0x0020_0000;
const WS_BORDER: u32 = 0x0080_0000;
const WS_CLIPSIBLINGS: u32 = 0x0400_0000;

const ES_LEFT: u32 = 0x0000;
const ES_MULTILINE: u32 = 0x0004;
const ES_READONLY: u32 = 0x0800;
const ES_AUTOVSCROLL: u32 = 0x0040;
const ES_WANTRETURN: u32 = 0x1000;

const BS_PUSHBUTTON: u32 = 0x0000_0000;
const SS_LEFT: u32 = 0x0000_0000;
const SS_NOPREFIX: u32 = 0x0000_0080;

const SW_HIDE: c_int = 0;
const SW_SHOW: c_int = 5;
const SW_SHOWNORMAL: c_int = 1;

const CW_USEDEFAULT: c_int = 0x8000_0000_u32 as c_int;

const IDI_APPLICATION: usize = 32512;
const IDC_ARROW: usize = 32512;
const COLOR_WINDOW: u32 = 5;

const EM_SETSEL: u32 = 0x00B1;
const EM_SCROLLCARET: u32 = 0x00B7;

const BN_CLICKED: u16 = 0;

const IDT_PUMP: usize = 1;
const IDC_START: i32 = 1001;
const IDC_STOP: i32 = 1002;
const IDC_REMEMBER: i32 = 1003;
const IDC_FOLDER: i32 = 1004;
const IDC_OPEN: i32 = 1005;
const IDC_CHECK: i32 = 1006;
const IDC_DEBUG: i32 = 1007;
const IDC_STATUS: i32 = 1101;
const IDC_LIVE: i32 = 1102;
const IDC_HISTORY: i32 = 1103;
const IDC_FOLDER_LBL: i32 = 1104;
const IDC_SESSION: i32 = 1105;
const IDC_TITLE: i32 = 1106;
const IDC_SUBTITLE: i32 = 1107;
const IDC_LIVE_LBL: i32 = 1108;
const IDC_HIST_LBL: i32 = 1109;
const IDC_STATUS_LBL: i32 = 1110;

const BIF_RETURNONLYFSDIRS: u32 = 0x0001;
const BIF_NEWDIALOGSTYLE: u32 = 0x0040;

#[repr(C)]
struct WndClassExW {
    cb_size: u32,
    style: u32,
    lpfn_wnd_proc: Option<unsafe extern "system" fn(Hwnd, u32, Wparam, Lparam) -> Lresult>,
    cb_cls_extra: c_int,
    cb_wnd_extra: c_int,
    h_instance: Hinstance,
    h_icon: Hwnd,
    h_cursor: Hwnd,
    hbr_background: Hbrush,
    lpsz_menu_name: *const u16,
    lpsz_class_name: *const u16,
    h_icon_sm: Hwnd,
}

#[repr(C)]
struct Point {
    x: c_int,
    y: c_int,
}

#[repr(C)]
struct Msg {
    hwnd: Hwnd,
    message: u32,
    w_param: Wparam,
    l_param: Lparam,
    time: u32,
    pt: Point,
}

#[repr(C)]
struct Rect {
    left: c_int,
    top: c_int,
    right: c_int,
    bottom: c_int,
}

#[repr(C)]
struct BrowseInfoW {
    hwnd_owner: Hwnd,
    pidl_root: *const c_void,
    psz_display_name: *mut u16,
    lpsz_title: *const u16,
    ul_flags: u32,
    lpfn: *const c_void,
    l_param: Lparam,
    i_image: c_int,
}

#[link(name = "user32")]
extern "system" {
    fn RegisterClassExW(wc: *const WndClassExW) -> u16;
    fn CreateWindowExW(
        ex: u32,
        class: *const u16,
        name: *const u16,
        style: u32,
        x: c_int,
        y: c_int,
        w: c_int,
        h: c_int,
        parent: Hwnd,
        menu: Hmenu,
        instance: Hinstance,
        param: *mut c_void,
    ) -> Hwnd;
    fn DefWindowProcW(hwnd: Hwnd, msg: u32, wp: Wparam, lp: Lparam) -> Lresult;
    fn ShowWindow(hwnd: Hwnd, cmd: c_int) -> c_int;
    fn UpdateWindow(hwnd: Hwnd) -> c_int;
    fn GetMessageW(msg: *mut Msg, hwnd: Hwnd, min: u32, max: u32) -> c_int;
    fn TranslateMessage(msg: *const Msg) -> c_int;
    fn DispatchMessageW(msg: *const Msg) -> Lresult;
    fn PostQuitMessage(code: c_int);
    fn DestroyWindow(hwnd: Hwnd) -> c_int;
    fn SetWindowTextW(hwnd: Hwnd, text: *const u16) -> c_int;
    fn GetWindowTextW(hwnd: Hwnd, buf: *mut u16, max: c_int) -> c_int;
    fn GetWindowTextLengthW(hwnd: Hwnd) -> c_int;
    fn EnableWindow(hwnd: Hwnd, enable: c_int) -> c_int;
    fn SetTimer(hwnd: Hwnd, id: usize, elapse: u32, timer_fn: *const c_void) -> usize;
    fn KillTimer(hwnd: Hwnd, id: usize) -> c_int;
    fn SendMessageW(hwnd: Hwnd, msg: u32, wp: Wparam, lp: Lparam) -> Lresult;
    fn GetClientRect(hwnd: Hwnd, rc: *mut Rect) -> c_int;
    fn MoveWindow(hwnd: Hwnd, x: c_int, y: c_int, w: c_int, h: c_int, repaint: c_int) -> c_int;
    fn LoadCursorW(instance: Hinstance, name: usize) -> Hwnd;
    fn LoadIconW(instance: Hinstance, name: usize) -> Hwnd;
    fn LoadImageW(
        instance: Hinstance,
        name: *const u16,
        ty: u32,
        cx: c_int,
        cy: c_int,
        fu_load: u32,
    ) -> Hwnd;
    fn GetModuleHandleW(name: *const u16) -> Hinstance;
    fn GetConsoleWindow() -> Hwnd;
    fn SetFocus(hwnd: Hwnd) -> Hwnd;
    fn GetSysColorBrush(index: c_int) -> Hbrush;
}

const IMAGE_ICON: u32 = 1;
const LR_LOADFROMFILE: u32 = 0x0010;
const LR_DEFAULTSIZE: u32 = 0x0040;
const WM_SETICON: u32 = 0x0080;
const ICON_SMALL: Wparam = 0;
const ICON_BIG: Wparam = 1;

#[link(name = "gdi32")]
extern "system" {
    fn CreateFontW(
        height: c_int,
        width: c_int,
        escapement: c_int,
        orientation: c_int,
        weight: c_int,
        italic: u32,
        underline: u32,
        strike: u32,
        charset: u32,
        out_precision: u32,
        clip_precision: u32,
        quality: u32,
        pitch_and_family: u32,
        face: *const u16,
    ) -> Hfont;
    fn DeleteObject(obj: *mut c_void) -> c_int;
    fn CreateSolidBrush(color: u32) -> Hbrush;
    fn SetTextColor(hdc: Hdc, color: u32) -> u32;
    fn SetBkColor(hdc: Hdc, color: u32) -> u32;
    fn SetBkMode(hdc: Hdc, mode: c_int) -> c_int;
}

#[link(name = "shell32")]
extern "system" {
    fn SHBrowseForFolderW(bi: *mut BrowseInfoW) -> *mut c_void;
    fn SHGetPathFromIDListW(pidl: *mut c_void, buf: *mut u16) -> c_int;
    fn ShellExecuteW(
        hwnd: Hwnd,
        op: *const u16,
        file: *const u16,
        params: *const u16,
        dir: *const u16,
        show: c_int,
    ) -> Hwnd;
    fn ILFree(pidl: *mut c_void);
}

#[link(name = "ole32")]
extern "system" {
    fn CoInitializeEx(pvreserved: *mut c_void, dwcoinit: u32) -> i32;
    fn CoUninitialize();
}

const COINIT_APARTMENTTHREADED: u32 = 0x2;
const S_OK: i32 = 0;
const S_FALSE: i32 = 1;

const FW_NORMAL: c_int = 400;
const FW_BOLD: c_int = 700;
const DEFAULT_CHARSET: u32 = 1;
const OUT_DEFAULT_PRECIS: u32 = 0;
const CLIP_DEFAULT_PRECIS: u32 = 0;
const CLEARTYPE_QUALITY: u32 = 5;
const DEFAULT_PITCH: u32 = 0;
const TRANSPARENT: c_int = 1;

// GDI color: 0x00BBGGRR
const COL_BG: u32 = 0x001A_1412; // dark ~ (0.07,0.08,0.10)
const COL_PANEL: u32 = 0x0029_2120;
const COL_TEXT: u32 = 0x00F2_F2F2;

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn from_wide(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

fn set_text(hwnd: Hwnd, s: &str) {
    if hwnd.is_null() {
        return;
    }
    let w = to_wide(s);
    unsafe {
        SetWindowTextW(hwnd, w.as_ptr());
    }
}

fn get_text(hwnd: Hwnd) -> String {
    if hwnd.is_null() {
        return String::new();
    }
    unsafe {
        let len = GetWindowTextLengthW(hwnd);
        if len <= 0 {
            return String::new();
        }
        let mut buf = vec![0u16; (len as usize) + 1];
        GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as c_int);
        from_wide(&buf)
    }
}

fn append_line(hwnd: Hwnd, line: &str) {
    if hwnd.is_null() {
        return;
    }
    let mut cur = get_text(hwnd);
    if !cur.is_empty() && !cur.ends_with('\n') {
        cur.push('\r');
        cur.push('\n');
    }
    cur.push_str(line);
    cur.push('\r');
    cur.push('\n');
    set_text(hwnd, &cur);
    unsafe {
        let len = GetWindowTextLengthW(hwnd) as Wparam;
        SendMessageW(hwnd, EM_SETSEL, len, len as Lparam);
        SendMessageW(hwnd, EM_SCROLLCARET, 0, 0);
    }
}

/// Replace the last non-empty line in the session history box (draft → polish).
fn replace_last_history_line(hwnd: Hwnd, line: &str) {
    if hwnd.is_null() {
        return;
    }
    let cur = get_text(hwnd);
    if cur.trim().is_empty() {
        append_line(hwnd, line);
        return;
    }
    let mut lines: Vec<&str> = cur.lines().collect();
    while lines.last().is_some_and(|l| l.trim().is_empty()) {
        lines.pop();
    }
    if lines.is_empty() {
        append_line(hwnd, line);
        return;
    }
    lines.pop();
    let mut out = String::new();
    for l in lines {
        out.push_str(l);
        out.push('\r');
        out.push('\n');
    }
    out.push_str(line);
    out.push('\r');
    out.push('\n');
    set_text(hwnd, &out);
    unsafe {
        let len = GetWindowTextLengthW(hwnd) as Wparam;
        SendMessageW(hwnd, EM_SETSEL, len, len as Lparam);
        SendMessageW(hwnd, EM_SCROLLCARET, 0, 0);
    }
}

struct GuiState {
    engine: CaptureEngine,
    remember_on: bool,
    debug_on: bool,
}

struct UiHandles {
    main: Hwnd,
    status: Hwnd,
    live: Hwnd,
    history: Hwnd,
    folder: Hwnd,
    session: Hwnd,
    start: Hwnd,
    stop: Hwnd,
    remember: Hwnd,
    debug: Hwnd,
    // layout anchors
    title: Hwnd,
    subtitle: Hwnd,
    status_lbl: Hwnd,
    live_lbl: Hwnd,
    hist_lbl: Hwnd,
    open: Hwnd,
    choose: Hwnd,
    check: Hwnd,
}

struct AppCtx {
    state: Arc<Mutex<GuiState>>,
    last_hist: Arc<Mutex<String>>,
    rx: Mutex<Receiver<EngineEvent>>,
    ui: Mutex<UiHandles>,
    font_ui: Hfont,
    font_title: Hfont,
    font_live: Hfont,
    brush_bg: Hbrush,
    brush_panel: Hbrush,
}

static mut APP: *mut AppCtx = ptr::null_mut();
/// True while a modal dialog (folder picker) is open — skip timer pump re-entry.
static mut MODAL_OPEN: bool = false;

fn with_app<F: FnOnce(&AppCtx)>(f: F) {
    unsafe {
        if !APP.is_null() {
            f(&*APP);
        }
    }
}

fn main_hwnd() -> Hwnd {
    let mut h: Hwnd = ptr::null_mut();
    with_app(|app| {
        if let Ok(ui) = app.ui.lock() {
            h = ui.main;
        }
    });
    h
}

fn apply_event(ev: EngineEvent, last_hist: &Mutex<String>, ui: &UiHandles) {
    match ev {
        EngineEvent::Status(s) => set_text(ui.status, &s),
        EngineEvent::Live(s) => set_text(ui.live, &s),
        EngineEvent::Final(s) => {
            set_text(ui.live, &s);
            let mut skip = false;
            if let Ok(mut last) = last_hist.lock() {
                if !last.is_empty() && same_or_refinement(&last, &s) {
                    if s.len() >= last.len() {
                        *last = s.clone();
                        replace_last_history_line(ui.history, &s);
                    }
                    skip = true;
                } else {
                    *last = s.clone();
                }
            }
            if !skip {
                append_line(ui.history, &s);
            }
        }
        EngineEvent::Revised(s) => {
            set_text(ui.live, &s);
            if let Ok(mut last) = last_hist.lock() {
                *last = s.clone();
            }
            replace_last_history_line(ui.history, &s);
        }
        EngineEvent::Error(s) => set_text(ui.status, &format!("! {s}")),
        EngineEvent::SessionFile(Some(p)) => {
            let footer = session_footer(Some(&p));
            let path_only = footer
                .strip_prefix("Saving to file: ")
                .unwrap_or(footer.as_str());
            set_text(ui.session, path_only);
            if let Some(parent) = p.parent() {
                set_folder_label_ui(ui, parent);
            }
            set_remember_btn(ui, true);
        }
        EngineEvent::SessionFile(None) => set_text(ui.session, ""),
        EngineEvent::Listening(on) => {
            unsafe {
                EnableWindow(ui.start, if on { 0 } else { 1 });
                EnableWindow(ui.stop, if on { 1 } else { 0 });
            }
        }
    }
}

fn set_folder_label_ui(ui: &UiHandles, path: &Path) {
    let display = folder_label(path);
    let path_only = display
        .strip_prefix("Folder: ")
        .unwrap_or(display.as_str());
    set_text(ui.folder, &format!("Folder: {path_only}"));
}

/// Button label only — never takes `state` (callers already holding it would deadlock).
fn set_remember_btn(ui: &UiHandles, on: bool) {
    set_text(
        ui.remember,
        if on {
            "Save to disk: ON"
        } else {
            "Save to disk: OFF"
        },
    );
}

/// Button label only — never takes `state`.
fn set_debug_btn(ui: &UiHandles, on: bool) {
    set_text(ui.debug, if on { "Debug: ON" } else { "Debug: OFF" });
}

fn child(
    class: &str,
    text: &str,
    style: u32,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    parent: Hwnd,
    id: i32,
    instance: Hinstance,
) -> Hwnd {
    let c = to_wide(class);
    let t = to_wide(text);
    unsafe {
        CreateWindowExW(
            0,
            c.as_ptr(),
            t.as_ptr(),
            WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS | style,
            x,
            y,
            w,
            h,
            parent,
            id as usize as Hmenu,
            instance,
            ptr::null_mut(),
        )
    }
}

fn apply_font(hwnd: Hwnd, font: Hfont) {
    if hwnd.is_null() || font.is_null() {
        return;
    }
    unsafe {
        SendMessageW(hwnd, WM_SETFONT, font as Wparam, 1);
    }
}

fn layout(ui: &UiHandles) {
    if ui.main.is_null() {
        return;
    }
    let mut rc = Rect {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    unsafe {
        GetClientRect(ui.main, &mut rc);
    }
    let w = (rc.right - rc.left).max(720);
    let h = (rc.bottom - rc.top).max(560);
    let m = 24;
    let bw = w - m * 2;

    let move_c = |hwnd: Hwnd, x: i32, y: i32, ww: i32, hh: i32| unsafe {
        if !hwnd.is_null() {
            MoveWindow(hwnd, x, y, ww, hh, 1);
        }
    };

    move_c(ui.title, m, 16, 400, 36);
    move_c(ui.subtitle, m, 52, bw.min(620), 24);

    // Button row
    let by = 90;
    move_c(ui.start, m, by, 200, 48);
    move_c(ui.stop, m + 212, by, 120, 48);
    move_c(ui.remember, m + 344, by, 200, 48);
    move_c(ui.choose, m + 556, by, 160, 48);

    let by2 = 148;
    move_c(ui.check, m, by2, 160, 40);
    move_c(ui.debug, m + 172, by2, 140, 40);
    move_c(ui.open, m + 556, by2, 160, 40);

    move_c(ui.status_lbl, m, 200, 100, 20);
    move_c(ui.status, m, 222, bw, 40);

    move_c(ui.live_lbl, m, 270, 200, 20);
    let live_h = 100;
    move_c(ui.live, m, 292, bw, live_h);

    let hist_top = 292 + live_h + 12;
    move_c(ui.hist_lbl, m, hist_top, 280, 20);
    let hist_y = hist_top + 22;
    let hist_h = (h - hist_y - 70).max(120);
    move_c(ui.history, m, hist_y, bw, hist_h);

    move_c(ui.folder, m, h - 52, bw, 22);
    move_c(ui.session, m, h - 28, bw, 20);
}

unsafe extern "system" fn wnd_proc(hwnd: Hwnd, msg: u32, wp: Wparam, lp: Lparam) -> Lresult {
    match msg {
        WM_COMMAND => {
            let id = (wp & 0xFFFF) as i32;
            let code = ((wp >> 16) & 0xFFFF) as u16;
            if code == BN_CLICKED || code == 0 {
                on_button(id);
            }
            0
        }
        WM_TIMER => {
            if wp == IDT_PUMP {
                pump_ui();
            }
            0
        }
        WM_SIZE => {
            with_app(|app| {
                if let Ok(ui) = app.ui.lock() {
                    layout(&ui);
                }
            });
            0
        }
        WM_CTLCOLORSTATIC | WM_CTLCOLOREDIT => {
            let hdc = wp as Hdc;
            SetTextColor(hdc, COL_TEXT);
            SetBkColor(
                hdc,
                if msg == WM_CTLCOLOREDIT {
                    COL_PANEL
                } else {
                    COL_BG
                },
            );
            SetBkMode(hdc, TRANSPARENT);
            let brush = if APP.is_null() {
                GetSysColorBrush(COLOR_WINDOW as c_int)
            } else if msg == WM_CTLCOLOREDIT {
                (*APP).brush_panel
            } else {
                (*APP).brush_bg
            };
            brush as Lresult
        }
        WM_CLOSE => {
            with_app(|app| {
                if let Ok(st) = app.state.lock() {
                    st.engine.stop();
                }
                KillTimer(hwnd, IDT_PUMP);
            });
            DestroyWindow(hwnd);
            0
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

fn on_button(id: i32) {
    match id {
        IDC_START => {
            // Never nest state locks (set_remember_btn used to re-lock → freeze/"crash").
            // Do not hold ui/state across engine.start() beyond the call itself.
            let (folder, remember) = {
                let mut folder = PathBuf::new();
                let mut remember = false;
                with_app(|app| {
                    if let Ok(st) = app.state.lock() {
                        folder = st.engine.folder();
                        remember = st.engine.remember();
                    }
                });
                (folder, remember)
            };
            with_app(|app| {
                if let Ok(ui) = app.ui.lock() {
                    set_folder_label_ui(&ui, &folder);
                    set_remember_btn(&ui, remember);
                    set_text(
                        ui.status,
                        "Starting… Listening for Live Captions (Win+Ctrl+L).",
                    );
                }
            });
            with_app(|app| {
                if let Ok(mut st) = app.state.lock() {
                    st.remember_on = remember;
                    st.engine.start();
                }
            });
            crate::debuglog::log("ui Start listening clicked");
        }
        IDC_STOP => {
            with_app(|app| {
                if let Ok(st) = app.state.lock() {
                    st.engine.stop();
                }
            });
            crate::debuglog::log("ui Stop clicked");
        }
        IDC_REMEMBER => {
            let (next, folder) = {
                let mut next = true;
                let mut folder = PathBuf::new();
                with_app(|app| {
                    if let Ok(mut st) = app.state.lock() {
                        next = !st.remember_on;
                        st.remember_on = next;
                        st.engine.set_remember(next);
                        folder = st.engine.folder();
                    }
                });
                (next, folder)
            };
            with_app(|app| {
                if let Ok(ui) = app.ui.lock() {
                    set_remember_btn(&ui, next);
                    set_folder_label_ui(&ui, &folder);
                    set_text(ui.status, &remember_toggle_status(next, &folder));
                }
            });
        }
        IDC_DEBUG => {
            let (next, folder) = {
                let mut next = true;
                let mut folder = PathBuf::new();
                with_app(|app| {
                    if let Ok(mut st) = app.state.lock() {
                        next = !st.debug_on;
                        st.debug_on = next;
                        folder = st.engine.folder();
                    }
                });
                (next, folder)
            };
            crate::debuglog::set_folder(&folder);
            crate::debuglog::set_enabled(next);
            let mut cfg = Config::load();
            cfg.debug = next;
            let _ = cfg.save();
            with_app(|app| {
                if let Ok(ui) = app.ui.lock() {
                    set_debug_btn(&ui, next);
                    if next {
                        let p = crate::debuglog::path_for_display();
                        set_text(
                            ui.status,
                            &format!(
                                "Debug ON — logging to {} (same folder as transcripts)",
                                p.display()
                            ),
                        );
                        crate::debuglog::log("debug enabled from UI");
                    } else {
                        set_text(ui.status, "Debug OFF.");
                    }
                }
            });
        }
        IDC_FOLDER => {
            // Never hold Mutex across SHBrowseForFolder: nested message loop
            // would re-enter timer/paint and deadlock on std::Mutex.
            let owner = main_hwnd();
            unsafe {
                MODAL_OPEN = true;
                if !owner.is_null() {
                    KillTimer(owner, IDT_PUMP);
                }
            }
            let picked = pick_folder(owner);
            unsafe {
                if !owner.is_null() {
                    SetTimer(owner, IDT_PUMP, 30, ptr::null());
                }
                MODAL_OPEN = false;
            }
            if let Some(path) = picked {
                with_app(|app| {
                    let Ok(ui) = app.ui.lock() else { return };
                    if let Ok(st) = app.state.lock() {
                        st.engine.set_folder(PathBuf::from(&path));
                    }
                    crate::debuglog::set_folder(Path::new(&path));
                    set_folder_label_ui(&ui, Path::new(&path));
                    set_text(ui.status, &format!("Transcript folder set to {path}"));
                });
            }
        }
        IDC_OPEN => {
            let (owner, folder) = {
                let mut owner: Hwnd = ptr::null_mut();
                let mut folder = PathBuf::new();
                with_app(|app| {
                    if let Ok(ui) = app.ui.lock() {
                        owner = ui.main;
                    }
                    if let Ok(st) = app.state.lock() {
                        folder = st.engine.folder();
                    }
                });
                (owner, folder)
            };
            let _ = std::fs::create_dir_all(&folder);
            let path = to_wide(&folder.to_string_lossy());
            let op = to_wide("explore");
            unsafe {
                ShellExecuteW(
                    owner,
                    op.as_ptr(),
                    path.as_ptr(),
                    ptr::null(),
                    ptr::null(),
                    SW_SHOWNORMAL,
                );
            }
        }
        IDC_CHECK => {
            let status_hwnd = {
                let mut h: Hwnd = ptr::null_mut();
                with_app(|app| {
                    if let Ok(ui) = app.ui.lock() {
                        h = ui.status;
                        set_text(ui.status, "Checking Live Captions setup…");
                    }
                });
                h
            };
            let owner = main_hwnd();
            unsafe {
                MODAL_OPEN = true;
                if !owner.is_null() {
                    KillTimer(owner, IDT_PUMP);
                }
            }
            let report = probe::run_probe();
            unsafe {
                if !owner.is_null() {
                    SetTimer(owner, IDT_PUMP, 30, ptr::null());
                }
                MODAL_OPEN = false;
            }
            let msg = if report.exit_code == probe::EXIT_LC_NOT_RUNNING {
                "Live Captions is not running. Press Win+Ctrl+L, then Start listening.".to_string()
            } else if report.exit_code == 0 {
                "Check OK — Live Captions looks readable. Press Start listening.".to_string()
            } else if report.exit_code == probe::EXIT_SIGNALS_STALE {
                "Live Captions process/window found but text not readable yet. Play audio, keep captions open, ensure helper .ps1 is next to interpres.exe.".to_string()
            } else {
                format!(
                    "Check finished (code {}). Run: interpres diagnose",
                    report.exit_code
                )
            };
            set_text(status_hwnd, &msg);
        }
        _ => {}
    }
}

fn pump_ui() {
    unsafe {
        if MODAL_OPEN {
            return;
        }
    }
    with_app(|app| {
        // try_lock: skip tick if a handler holds the lock (never block UI thread).
        let Ok(ui) = app.ui.try_lock() else {
            return;
        };
        let Ok(rx) = app.rx.try_lock() else {
            return;
        };
        loop {
            match rx.try_recv() {
                Ok(ev) => apply_event(ev, &app.last_hist, &ui),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break,
            }
        }
    });
}

fn pick_folder(owner: Hwnd) -> Option<String> {
    // BIF_NEWDIALOGSTYLE needs COM apartment init on this thread.
    let hr = unsafe { CoInitializeEx(ptr::null_mut(), COINIT_APARTMENTTHREADED) };
    if hr != S_OK && hr != S_FALSE {
        crate::debuglog::log(&format!("CoInitializeEx failed hr={hr:#x}"));
    }

    let title = to_wide("Choose transcript folder");
    let mut display = vec![0u16; 520];
    let mut bi = BrowseInfoW {
        hwnd_owner: owner,
        pidl_root: ptr::null(),
        psz_display_name: display.as_mut_ptr(),
        lpsz_title: title.as_ptr(),
        ul_flags: BIF_RETURNONLYFSDIRS | BIF_NEWDIALOGSTYLE,
        lpfn: ptr::null(),
        l_param: 0,
        i_image: 0,
    };
    let pidl = unsafe { SHBrowseForFolderW(&mut bi) };
    let result = if pidl.is_null() {
        None
    } else {
        let mut path = vec![0u16; 520];
        let ok = unsafe { SHGetPathFromIDListW(pidl, path.as_mut_ptr()) };
        unsafe { ILFree(pidl) };
        if ok == 0 {
            None
        } else {
            let s = from_wide(&path);
            if s.is_empty() {
                None
            } else {
                Some(s)
            }
        }
    };

    // Uninit only if we initialized (S_OK); S_FALSE means already initialized.
    if hr == S_OK {
        unsafe { CoUninitialize() };
    }
    result
}

fn hide_console() {
    unsafe {
        let c = GetConsoleWindow();
        if !c.is_null() {
            ShowWindow(c, SW_HIDE);
        }
    }
}

/// Load icon from PE resource (id 1) or `Interpres.ico` / `logo-256.png` beside the exe.
fn load_app_icon(instance: Hinstance) -> Hwnd {
    unsafe {
        // MAKEINTRESOURCE(1) — resource embedded by build.rs / windres
        let from_res = LoadImageW(
            instance,
            1usize as *const u16,
            IMAGE_ICON,
            0,
            0,
            LR_DEFAULTSIZE,
        );
        if !from_res.is_null() {
            return from_res;
        }
    }
    // File fallback for unpackaged / windres-less builds
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("Interpres.ico"));
            candidates.push(dir.join("assets").join("Interpres.ico"));
            candidates.push(dir.join("logo-256.png"));
            candidates.push(dir.join("logo.png"));
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("assets").join("Interpres.ico"));
        candidates.push(cwd.join("Interpres.ico"));
    }
    for c in candidates {
        if !c.is_file() {
            continue;
        }
        let w = to_wide(&c.to_string_lossy());
        let h = unsafe {
            LoadImageW(
                ptr::null_mut(),
                w.as_ptr(),
                IMAGE_ICON,
                0,
                0,
                LR_LOADFROMFILE | LR_DEFAULTSIZE,
            )
        };
        if !h.is_null() {
            return h;
        }
    }
    ptr::null_mut()
}

/// Run the native Windows GUI (blocks until the window is closed).
pub fn run_windows_gui() -> i32 {
    hide_console();

    let mut cfg = Config::load();
    let fs = cfg.transcript_folder.to_string_lossy();
    if fs.contains("/var/folders/") || fs.contains("/tmp") || fs.contains("\\Temp") {
        cfg.transcript_folder = crate::config::default_transcript_folder();
        let _ = cfg.save();
    }

    crate::debuglog::init_from_config(cfg.debug, &cfg.transcript_folder);
    crate::debuglog::log("gui open (windows)");

    let (engine, rx) = CaptureEngine::new(&cfg);
    let remember0 = engine.remember();
    let folder0 = engine.folder();
    let debug0 = cfg.debug;

    let state = Arc::new(Mutex::new(GuiState {
        engine,
        remember_on: remember0,
        debug_on: debug0,
    }));
    let last_hist = Arc::new(Mutex::new(String::new()));

    let instance = unsafe { GetModuleHandleW(ptr::null()) };
    let class_name = to_wide("InterpresMainWnd");
    let face = to_wide("Segoe UI");
    // Prefer PE resource id 1 (embedded by build.rs + windres); else file next to exe.
    let app_icon = load_app_icon(instance);

    let font_ui = unsafe {
        CreateFontW(
            -16,
            0,
            0,
            0,
            FW_NORMAL,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            CLEARTYPE_QUALITY,
            DEFAULT_PITCH,
            face.as_ptr(),
        )
    };
    let font_title = unsafe {
        CreateFontW(
            -28,
            0,
            0,
            0,
            FW_BOLD,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            CLEARTYPE_QUALITY,
            DEFAULT_PITCH,
            face.as_ptr(),
        )
    };
    let font_live = unsafe {
        CreateFontW(
            -22,
            0,
            0,
            0,
            FW_NORMAL,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            CLEARTYPE_QUALITY,
            DEFAULT_PITCH,
            face.as_ptr(),
        )
    };
    let brush_bg = unsafe { CreateSolidBrush(COL_BG) };
    let brush_panel = unsafe { CreateSolidBrush(COL_PANEL) };

    let wc = WndClassExW {
        cb_size: std::mem::size_of::<WndClassExW>() as u32,
        style: 0,
        lpfn_wnd_proc: Some(wnd_proc),
        cb_cls_extra: 0,
        cb_wnd_extra: 0,
        h_instance: instance,
        h_icon: if !app_icon.is_null() {
            app_icon
        } else {
            unsafe { LoadIconW(ptr::null_mut(), IDI_APPLICATION) }
        },
        h_cursor: unsafe { LoadCursorW(ptr::null_mut(), IDC_ARROW) },
        hbr_background: brush_bg,
        lpsz_menu_name: ptr::null(),
        lpsz_class_name: class_name.as_ptr(),
        h_icon_sm: if !app_icon.is_null() {
            app_icon
        } else {
            ptr::null_mut()
        },
    };

    unsafe {
        if RegisterClassExW(&wc) == 0 {
            // already registered is ok on re-entry in same process rarely
        }
    }

    let title = to_wide("Interpres — Live Captions companion");
    let main = unsafe {
        CreateWindowExW(
            0,
            class_name.as_ptr(),
            title.as_ptr(),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            940,
            720,
            ptr::null_mut(),
            ptr::null_mut(),
            instance,
            ptr::null_mut(),
        )
    };
    if main.is_null() {
        eprintln!("Failed to create Interpres window (CreateWindowExW).");
        return 1;
    }
    if !app_icon.is_null() {
        unsafe {
            SendMessageW(main, WM_SETICON, ICON_BIG, app_icon as Lparam);
            SendMessageW(main, WM_SETICON, ICON_SMALL, app_icon as Lparam);
        }
    }

    let edit_style = ES_LEFT
        | ES_MULTILINE
        | ES_READONLY
        | ES_AUTOVSCROLL
        | ES_WANTRETURN
        | WS_VSCROLL
        | WS_BORDER
        | WS_TABSTOP;
    let btn = BS_PUSHBUTTON | WS_TABSTOP;

    let ui = UiHandles {
        main,
        title: child("STATIC", "Interpres", SS_LEFT | SS_NOPREFIX, 0, 0, 10, 10, main, IDC_TITLE, instance),
        subtitle: child(
            "STATIC",
            "Saves what Live Captions say — on this PC only",
            SS_LEFT | SS_NOPREFIX,
            0,
            0,
            10,
            10,
            main,
            IDC_SUBTITLE,
            instance,
        ),
        start: child("BUTTON", "Start listening", btn, 0, 0, 10, 10, main, IDC_START, instance),
        stop: child("BUTTON", "Stop", btn, 0, 0, 10, 10, main, IDC_STOP, instance),
        remember: child(
            "BUTTON",
            if remember0 {
                "Save to disk: ON"
            } else {
                "Save to disk: OFF"
            },
            btn,
            0,
            0,
            10,
            10,
            main,
            IDC_REMEMBER,
            instance,
        ),
        choose: child("BUTTON", "Choose folder...", btn, 0, 0, 10, 10, main, IDC_FOLDER, instance),
        open: child("BUTTON", "Open folder", btn, 0, 0, 10, 10, main, IDC_OPEN, instance),
        check: child("BUTTON", "Check setup", btn, 0, 0, 10, 10, main, IDC_CHECK, instance),
        debug: child(
            "BUTTON",
            if debug0 { "Debug: ON" } else { "Debug: OFF" },
            btn,
            0,
            0,
            10,
            10,
            main,
            IDC_DEBUG,
            instance,
        ),
        status_lbl: child("STATIC", "Status", SS_LEFT | SS_NOPREFIX, 0, 0, 10, 10, main, IDC_STATUS_LBL, instance),
        status: child(
            "STATIC",
            "Turn on Live Captions (Win+Ctrl+L), then press Start listening.",
            SS_LEFT | SS_NOPREFIX,
            0,
            0,
            10,
            10,
            main,
            IDC_STATUS,
            instance,
        ),
        live_lbl: child("STATIC", "Live (now)", SS_LEFT | SS_NOPREFIX, 0, 0, 10, 10, main, IDC_LIVE_LBL, instance),
        live: child("EDIT", "", edit_style, 0, 0, 10, 10, main, IDC_LIVE, instance),
        hist_lbl: child(
            "STATIC",
            "Session (saved lines)",
            SS_LEFT | SS_NOPREFIX,
            0,
            0,
            10,
            10,
            main,
            IDC_HIST_LBL,
            instance,
        ),
        history: child("EDIT", "", edit_style, 0, 0, 10, 10, main, IDC_HISTORY, instance),
        folder: child("STATIC", "Folder: …", SS_LEFT | SS_NOPREFIX, 0, 0, 10, 10, main, IDC_FOLDER_LBL, instance),
        session: child("STATIC", "", SS_LEFT | SS_NOPREFIX, 0, 0, 10, 10, main, IDC_SESSION, instance),
    };

    unsafe {
        EnableWindow(ui.stop, 0);
    }

    // Fonts
    for h in [
        ui.subtitle,
        ui.start,
        ui.stop,
        ui.remember,
        ui.choose,
        ui.open,
        ui.check,
        ui.debug,
        ui.status_lbl,
        ui.status,
        ui.live_lbl,
        ui.hist_lbl,
        ui.folder,
        ui.session,
        ui.history,
    ] {
        apply_font(h, font_ui);
    }
    apply_font(ui.title, font_title);
    apply_font(ui.live, font_live);

    set_folder_label_ui(&ui, &folder0);
    set_text(ui.session, "");

    layout(&ui);

    let app = Box::new(AppCtx {
        state: state.clone(),
        last_hist: last_hist.clone(),
        rx: Mutex::new(rx),
        ui: Mutex::new(ui),
        font_ui,
        font_title,
        font_live,
        brush_bg,
        brush_panel,
    });
    unsafe {
        APP = Box::into_raw(app);
        SetTimer(main, IDT_PUMP, 30, ptr::null());
        ShowWindow(main, SW_SHOW);
        UpdateWindow(main);
        SetFocus(main);
    }

    crate::debuglog::log(&format!(
        "ui ready folder={} remember={} debug={}",
        folder0.display(),
        remember0,
        debug0
    ));

    // Message loop
    let mut msg = Msg {
        hwnd: ptr::null_mut(),
        message: 0,
        w_param: 0,
        l_param: 0,
        time: 0,
        pt: Point { x: 0, y: 0 },
    };
    loop {
        let r = unsafe { GetMessageW(&mut msg, ptr::null_mut(), 0, 0) };
        if r == 0 || r == -1 {
            break;
        }
        unsafe {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }

    // Teardown
    if let Ok(st) = state.lock() {
        st.engine.stop();
    }
    thread::sleep(Duration::from_millis(80));

    unsafe {
        if !APP.is_null() {
            let app = Box::from_raw(APP);
            APP = ptr::null_mut();
            if !app.font_ui.is_null() {
                DeleteObject(app.font_ui);
            }
            if !app.font_title.is_null() {
                DeleteObject(app.font_title);
            }
            if !app.font_live.is_null() {
                DeleteObject(app.font_live);
            }
            if !app.brush_bg.is_null() {
                DeleteObject(app.brush_bg);
            }
            if !app.brush_panel.is_null() {
                DeleteObject(app.brush_panel);
            }
        }
    }

    0
}


