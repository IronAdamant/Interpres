//! Windows native UI — hand-written Win32 (user32/gdi32/shell32). Zero crates.io.
//!
//! Layout (top → bottom): title + Auto-record checkbox + Settings menu, a coloured status banner driven by
//! `EngineEvent::Health`, one Start/Stop button plus a contextual Live Captions
//! button, a setup checklist, a single transcript view (saved lines + the line being
//! spoken), and a footer with Open / Copy / Folder actions.

use crate::auto_record::{AutoAction, AutoRecord, SoundDetector};
use crate::buffer::same_or_refinement;
use crate::config::Config;
use crate::engine::{CaptureEngine, EngineEvent};
use crate::health::{Health, IdlePrompt};
use crate::history_ui::{apply_plan, plan_family, FamilyPlan};
use crate::platform;
use crate::platform::windows_audio::{spawn_sound_meter, SoundLevel};
use crate::theme::{palette_for_dark, ThemeMode};
use crate::transcript::format_clock;
use crate::ui_labels::LAG_TIP;
use std::os::raw::{c_int, c_void};
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime};

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
const WM_ERASEBKGND: u32 = 0x0014;
const WM_SETTINGCHANGE: u32 = 0x001A;
const WM_GETMINMAXINFO: u32 = 0x0024;
const WM_DRAWITEM: u32 = 0x002B;
const WM_SETFONT: u32 = 0x0030;
const WM_SETICON: u32 = 0x0080;
const WM_COMMAND: u32 = 0x0111;
const WM_TIMER: u32 = 0x0113;
const WM_CTLCOLOREDIT: u32 = 0x0133;
const WM_CTLCOLORBTN: u32 = 0x0135;
const WM_CTLCOLORSTATIC: u32 = 0x0138;

const WS_OVERLAPPEDWINDOW: u32 = 0x00CF_0000;
const WS_VISIBLE: u32 = 0x1000_0000;
const WS_CHILD: u32 = 0x4000_0000;
const WS_CLIPCHILDREN: u32 = 0x0200_0000;
const WS_CLIPSIBLINGS: u32 = 0x0400_0000;
const WS_TABSTOP: u32 = 0x0001_0000;
const WS_VSCROLL: u32 = 0x0020_0000;

const ES_MULTILINE: u32 = 0x0004;
const ES_AUTOVSCROLL: u32 = 0x0040;
const ES_READONLY: u32 = 0x0800;
const BS_OWNERDRAW: u32 = 0x000B;
const SS_LEFT: u32 = 0x0000;
const SS_NOPREFIX: u32 = 0x0080;
const SS_ENDELLIPSIS: u32 = 0x4000;
const SS_PATHELLIPSIS: u32 = 0x8000;
const WS_EX_CLIENTEDGE: u32 = 0x0200;

const SW_HIDE: c_int = 0;
const SW_SHOWNORMAL: c_int = 1;
const SW_SHOW: c_int = 5;
const SW_SHOWMINNOACTIVE: c_int = 7;
const CW_USEDEFAULT: c_int = 0x8000_0000_u32 as c_int;
const IDI_APPLICATION: usize = 32512;
const IDC_ARROW: usize = 32512;

const EM_SETSEL: u32 = 0x00B1;
const EM_LINESCROLL: u32 = 0x00B6;
const EM_SCROLLCARET: u32 = 0x00B7;
const EM_GETFIRSTVISIBLELINE: u32 = 0x00CE;
const EM_REPLACESEL: u32 = 0x00C2;
const EM_SETLIMITTEXT: u32 = 0x00C5;
const SB_VERT: c_int = 1;
const SIF_ALL: u32 = 0x17;

const ODS_SELECTED: u32 = 0x0001;
const ODS_DISABLED: u32 = 0x0004;
const ODS_FOCUS: u32 = 0x0010;
const DT_CENTER: u32 = 0x0001;
const DT_VCENTER: u32 = 0x0004;
const DT_SINGLELINE: u32 = 0x0020;
const DT_NOPREFIX: u32 = 0x0800;
const DT_LEFT: u32 = 0x0000;
const PS_SOLID: c_int = 0;
const TRANSPARENT: c_int = 1;

const MF_STRING: u32 = 0x0000;
const MF_GRAYED: u32 = 0x0001;
const MF_CHECKED: u32 = 0x0008;
const MF_SEPARATOR: u32 = 0x0800;
const TPM_RETURNCMD: u32 = 0x0100;

const CF_UNICODETEXT: u32 = 13;
const GMEM_MOVEABLE: u32 = 0x0002;
const FLASHW_ALL: u32 = 0x3;
const FLASHW_TIMERNOFG: u32 = 0xC;
const MB_ICONWARNING: u32 = 0x30;
const MB_OK: u32 = 0x0;

const BIF_RETURNONLYFSDIRS: u32 = 0x0001;
const BIF_NEWDIALOGSTYLE: u32 = 0x0040;
const COINIT_APARTMENTTHREADED: u32 = 0x2;
const S_OK: i32 = 0;

const IMAGE_ICON: u32 = 1;
const LR_LOADFROMFILE: u32 = 0x0010;
const LR_DEFAULTSIZE: u32 = 0x0040;
const ICON_SMALL: Wparam = 0;
const ICON_BIG: Wparam = 1;

const DWMWA_USE_IMMERSIVE_DARK_MODE: u32 = 20;
const HKEY_CURRENT_USER: *mut c_void = 0x8000_0001u32 as usize as *mut c_void;
const KEY_READ: u32 = 0x20019;
const REG_DWORD: u32 = 4;
const REG_SZ: u32 = 1;
const KEY_SET_VALUE: u32 = 0x0002;
const ERROR_FILE_NOT_FOUND: i32 = 2;
/// Per-user startup list (no admin needed).
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE: &str = "Interpres";
/// Command-line flag used by the startup entry: open minimized, don't steal focus.
const MINIMIZED_ARG: &str = "--minimized";

const FW_NORMAL: c_int = 400;
const FW_SEMIBOLD: c_int = 600;
const FW_BOLD: c_int = 700;
const DEFAULT_CHARSET: u32 = 1;
const CLEARTYPE_QUALITY: u32 = 5;

const IDT_PUMP: usize = 1;
const PUMP_MS: u32 = 50;
/// Re-render the transcript at most this often (engine emits every poll).
const TRANSCRIPT_MIN_INTERVAL: Duration = Duration::from_millis(200);
/// While stopped, re-check whether Live Captions is on this often.
const IDLE_LC_CHECK: Duration = Duration::from_secs(2);
/// Speaker level older than this is treated as unknown (never as silence).
const SOUND_STALE: Duration = Duration::from_secs(3);
/// Auto-record turned Live Captions on: wait this long for it before recording anyway.
const AUTO_LC_WAIT: Duration = Duration::from_secs(15);

// Controls
const IDC_TITLE: i32 = 1001;
const IDC_SETTINGS: i32 = 1002;
const IDC_BANNER_HEAD: i32 = 1003;
const IDC_BANNER_DETAIL: i32 = 1004;
const IDC_TOGGLE: i32 = 1005;
const IDC_ACTION: i32 = 1006;
const IDC_CHECKS: i32 = 1007;
const IDC_DETAIL: i32 = 1008;
const IDC_TRANSCRIPT_LBL: i32 = 1009;
const IDC_TRANSCRIPT: i32 = 1010;
const IDC_FILE: i32 = 1011;
const IDC_OPEN_FILE: i32 = 1012;
const IDC_COPY: i32 = 1013;
const IDC_OPEN_FOLDER: i32 = 1014;
const IDC_AUTO: i32 = 1015;

// Settings menu
const IDM_SAVE: i32 = 2001;
const IDM_FOLDER: i32 = 2002;
const IDM_OPEN_FOLDER: i32 = 2003;
const IDM_THEME_SYSTEM: i32 = 2004;
const IDM_THEME_LIGHT: i32 = 2005;
const IDM_THEME_DARK: i32 = 2006;
const IDM_CHECK: i32 = 2007;
const IDM_DEBUG: i32 = 2008;
const IDM_RESTART_LC: i32 = 2009;
const IDM_SOURCE_LC: i32 = 2010;
const IDM_SOURCE_ENGINE: i32 = 2011;
const IDM_EDIT_SETTINGS: i32 = 2012;
const IDM_START_WITH_WINDOWS: i32 = 2013;

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
#[derive(Clone, Copy, Default)]
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
#[derive(Clone, Copy, Default)]
struct Rect {
    left: c_int,
    top: c_int,
    right: c_int,
    bottom: c_int,
}

#[repr(C)]
struct MinMaxInfo {
    reserved: Point,
    max_size: Point,
    max_position: Point,
    min_track: Point,
    max_track: Point,
}

#[repr(C)]
struct DrawItemStruct {
    ctl_type: u32,
    ctl_id: u32,
    item_id: u32,
    item_action: u32,
    item_state: u32,
    hwnd_item: Hwnd,
    hdc: Hdc,
    rc_item: Rect,
    item_data: usize,
}

#[repr(C)]
struct ScrollInfo {
    cb_size: u32,
    f_mask: u32,
    n_min: c_int,
    n_max: c_int,
    n_page: u32,
    n_pos: c_int,
    n_track_pos: c_int,
}

#[repr(C)]
struct FlashWInfo {
    cb_size: u32,
    hwnd: Hwnd,
    dw_flags: u32,
    u_count: u32,
    dw_timeout: u32,
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
    fn IsWindowEnabled(hwnd: Hwnd) -> c_int;
    fn SetTimer(hwnd: Hwnd, id: usize, elapse: u32, timer_fn: *const c_void) -> usize;
    fn KillTimer(hwnd: Hwnd, id: usize) -> c_int;
    fn SendMessageW(hwnd: Hwnd, msg: u32, wp: Wparam, lp: Lparam) -> Lresult;
    fn GetClientRect(hwnd: Hwnd, rc: *mut Rect) -> c_int;
    fn GetWindowRect(hwnd: Hwnd, rc: *mut Rect) -> c_int;
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
    fn GetDlgCtrlID(hwnd: Hwnd) -> c_int;
    fn InvalidateRect(hwnd: Hwnd, rc: *const Rect, erase: c_int) -> c_int;
    fn FillRect(hdc: Hdc, rc: *const Rect, brush: Hbrush) -> c_int;
    fn DrawTextW(hdc: Hdc, text: *const u16, len: c_int, rc: *mut Rect, format: u32) -> c_int;
    fn DrawFocusRect(hdc: Hdc, rc: *const Rect) -> c_int;
    fn CreatePopupMenu() -> Hmenu;
    fn AppendMenuW(menu: Hmenu, flags: u32, id: usize, text: *const u16) -> c_int;
    fn TrackPopupMenu(
        menu: Hmenu,
        flags: u32,
        x: c_int,
        y: c_int,
        reserved: c_int,
        hwnd: Hwnd,
        rc: *const Rect,
    ) -> c_int;
    fn DestroyMenu(menu: Hmenu) -> c_int;
    fn OpenClipboard(hwnd: Hwnd) -> c_int;
    fn EmptyClipboard() -> c_int;
    fn SetClipboardData(format: u32, mem: *mut c_void) -> *mut c_void;
    fn CloseClipboard() -> c_int;
    fn FlashWindowEx(info: *const FlashWInfo) -> c_int;
    fn MessageBeep(kind: u32) -> c_int;
    fn GetScrollInfo(hwnd: Hwnd, bar: c_int, si: *mut ScrollInfo) -> c_int;
}

#[link(name = "kernel32")]
extern "system" {
    fn GlobalAlloc(flags: u32, bytes: usize) -> *mut c_void;
    fn GlobalLock(mem: *mut c_void) -> *mut c_void;
    fn GlobalUnlock(mem: *mut c_void) -> c_int;
    fn GlobalFree(mem: *mut c_void) -> *mut c_void;
}

#[link(name = "uxtheme")]
extern "system" {
    fn SetWindowTheme(hwnd: Hwnd, sub_app: *const u16, sub_id: *const u16) -> i32;
}

#[link(name = "dwmapi")]
extern "system" {
    fn DwmSetWindowAttribute(hwnd: Hwnd, attr: u32, value: *const c_void, size: u32) -> i32;
}

#[link(name = "advapi32")]
extern "system" {
    fn RegOpenKeyExW(
        key: *mut c_void,
        sub: *const u16,
        options: u32,
        sam: u32,
        result: *mut *mut c_void,
    ) -> i32;
    fn RegQueryValueExW(
        key: *mut c_void,
        name: *const u16,
        reserved: *mut u32,
        ty: *mut u32,
        data: *mut u8,
        data_len: *mut u32,
    ) -> i32;
    fn RegCloseKey(key: *mut c_void) -> i32;
    fn RegSetValueExW(
        key: *mut c_void,
        name: *const u16,
        reserved: u32,
        ty: u32,
        data: *const u8,
        data_len: u32,
    ) -> i32;
    fn RegDeleteValueW(key: *mut c_void, name: *const u16) -> i32;
}

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
    fn CreatePen(style: c_int, width: c_int, color: u32) -> *mut c_void;
    fn SelectObject(hdc: Hdc, obj: *mut c_void) -> *mut c_void;
    fn RoundRect(hdc: Hdc, l: c_int, t: c_int, r: c_int, b: c_int, w: c_int, h: c_int) -> c_int;
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

/// GDI COLORREF from 0–255 RGB.
const fn rgb(r: u32, g: u32, b: u32) -> u32 {
    r | (g << 8) | (b << 16)
}

// Status colours: white text on all of these in both themes.
const COL_RECORDING: u32 = rgb(28, 128, 72);
const COL_WAITING: u32 = rgb(37, 99, 160);
const COL_PROBLEM: u32 = rgb(176, 36, 36);
const COL_START: u32 = rgb(28, 128, 72);
const COL_STOP: u32 = rgb(176, 36, 36);
const COL_ACTION: u32 = rgb(166, 92, 0);
const COL_WHITE: u32 = rgb(255, 255, 255);

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
    unsafe {
        let len = GetWindowTextLengthW(hwnd).max(0) as usize;
        let mut buf = vec![0u16; len + 1];
        GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as c_int);
        from_wide(&buf)
    }
}

/// Windows "AppsUseLightTheme" = 0 → dark apps.
fn system_apps_use_dark() -> bool {
    unsafe {
        let sub = to_wide(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
        let mut key: *mut c_void = ptr::null_mut();
        if RegOpenKeyExW(HKEY_CURRENT_USER, sub.as_ptr(), 0, KEY_READ, &mut key) != 0 {
            return true;
        }
        let name = to_wide("AppsUseLightTheme");
        let mut ty: u32 = 0;
        let mut data: u32 = 1;
        let mut len: u32 = 4;
        let ok = RegQueryValueExW(
            key,
            name.as_ptr(),
            ptr::null_mut(),
            &mut ty,
            &mut data as *mut u32 as *mut u8,
            &mut len,
        );
        RegCloseKey(key);
        if ok != 0 || ty != REG_DWORD {
            return true;
        }
        data == 0
    }
}

/// What the startup entry runs: this exe, opened minimized.
fn startup_command() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    Some(format!("\"{}\" gui {MINIMIZED_ARG}", exe.display()))
}

/// The startup entry's command, if Interpres is set to start with Windows.
fn read_startup_entry() -> Option<String> {
    unsafe {
        let sub = to_wide(RUN_KEY);
        let mut key: *mut c_void = ptr::null_mut();
        if RegOpenKeyExW(HKEY_CURRENT_USER, sub.as_ptr(), 0, KEY_READ, &mut key) != 0 {
            return None;
        }
        let name = to_wide(RUN_VALUE);
        let mut ty: u32 = 0;
        let mut buf = vec![0u16; 1024];
        let mut len = (buf.len() * 2) as u32;
        let ok = RegQueryValueExW(
            key,
            name.as_ptr(),
            ptr::null_mut(),
            &mut ty,
            buf.as_mut_ptr() as *mut u8,
            &mut len,
        );
        RegCloseKey(key);
        (ok == 0 && ty == REG_SZ).then(|| from_wide(&buf))
    }
}

/// Add or remove Interpres from the per-user startup list.
fn set_start_with_windows(on: bool) -> bool {
    unsafe {
        let sub = to_wide(RUN_KEY);
        let mut key: *mut c_void = ptr::null_mut();
        if RegOpenKeyExW(HKEY_CURRENT_USER, sub.as_ptr(), 0, KEY_SET_VALUE, &mut key) != 0 {
            return false;
        }
        let name = to_wide(RUN_VALUE);
        let res = if on {
            match startup_command() {
                Some(cmd) => {
                    let data = to_wide(&cmd);
                    RegSetValueExW(
                        key,
                        name.as_ptr(),
                        0,
                        REG_SZ,
                        data.as_ptr() as *const u8,
                        (data.len() * 2) as u32,
                    )
                }
                None => -1,
            }
        } else {
            match RegDeleteValueW(key, name.as_ptr()) {
                ERROR_FILE_NOT_FOUND => 0,
                r => r,
            }
        };
        RegCloseKey(key);
        res == 0
    }
}

fn make_font(face: &[u16], height: c_int, weight: c_int) -> Hfont {
    unsafe {
        CreateFontW(
            height,
            0,
            0,
            0,
            weight,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            0,
            0,
            CLEARTYPE_QUALITY,
            0,
            face.as_ptr(),
        )
    }
}

struct Controls {
    main: Hwnd,
    title: Hwnd,
    settings: Hwnd,
    banner_head: Hwnd,
    banner_detail: Hwnd,
    toggle: Hwnd,
    action: Hwnd,
    checks: Hwnd,
    detail: Hwnd,
    transcript_lbl: Hwnd,
    transcript: Hwnd,
    file: Hwnd,
    open_file: Hwnd,
    copy: Hwnd,
    open_folder: Hwnd,
    /// "Auto-record when sound plays" checkbox (owner-drawn).
    auto: Hwnd,
}

/// What the contextual second button does right now.
#[derive(Clone, Copy, PartialEq, Eq)]
enum LcAction {
    None,
    TurnOn,
    Restart,
    /// Answer to "are you done?" — snooze the prompt.
    KeepRecording,
}

/// Everything the window shows, derived from engine events.
struct View {
    listening: bool,
    health: Option<Health>,
    /// While stopped: is Live Captions running (polled every `IDLE_LC_CHECK`).
    idle_lc_on: Option<bool>,
    last_idle_check: Option<Instant>,
    started_at: Option<Instant>,
    lines: Vec<String>,
    times: Vec<String>,
    live: String,
    session_path: Option<PathBuf>,
    session_active: bool,
    detail: String,
    transcript_dirty: bool,
    last_render: Option<Instant>,
    rendered_banner: String,
    /// Rows currently in the transcript control (each ends with CRLF) and their UTF-16
    /// lengths, so updates only replace the changed tail instead of the whole text.
    rendered_rows: Vec<String>,
    rendered_u16: Vec<usize>,
    /// Stop pressed; engine is saving the last words.
    stopping: bool,
    idle: IdlePrompt,
}

struct AppCtx {
    engine: CaptureEngine,
    rx: Receiver<EngineEvent>,
    c: Controls,
    view: View,
    remember: bool,
    debug: bool,
    theme_mode: ThemeMode,
    idle_prompt_ms: u64,
    /// Captions come from an external speech engine instead of Live Captions.
    engine_mode: bool,
    /// Display name of the external engine (from settings).
    engine_name: String,
    /// `helper_path` is set, so the external engine can be chosen.
    engine_configured: bool,
    /// Monotonic base for `IdlePrompt` milliseconds.
    epoch: Instant,
    /// Auto-record checkbox state (saved as `auto_record`).
    auto_on: bool,
    auto: AutoRecord,
    /// Speaker level reader; running only while auto-record is on.
    sound: Option<Arc<SoundLevel>>,
    detector: SoundDetector,
    /// Last speaker-meter state reported to the user (None = not yet known).
    sound_readable: Option<bool>,
    /// Auto-record turned Live Captions on and is waiting for it before starting.
    auto_start_pending: Option<Instant>,
    /// Shown once the engine confirms an automatic start/stop (those events clear the
    /// detail line).
    auto_note: Option<String>,
    // fonts
    font_ui: Hfont,
    font_title: Hfont,
    font_banner: Hfont,
    font_button: Hfont,
    font_transcript: Hfont,
    font_small: Hfont,
    // colours / brushes
    col_bg: u32,
    col_panel: u32,
    col_text: u32,
    col_muted: u32,
    col_button: u32,
    col_border: u32,
    brush_bg: Hbrush,
    brush_panel: Hbrush,
    brush_banner: Hbrush,
    col_banner: u32,
    col_banner_text: u32,
    banner_rect: Rect,
}

static mut APP: *mut AppCtx = ptr::null_mut();
/// True while a modal loop (menu, folder picker) runs — pause the event pump.
static mut MODAL_OPEN: bool = false;

/// UI-thread-only access. Never hold across calls that pump messages (menus, dialogs).
fn with_app<R>(f: impl FnOnce(&mut AppCtx) -> R) -> Option<R> {
    unsafe {
        if APP.is_null() {
            None
        } else {
            Some(f(&mut *APP))
        }
    }
}

fn set_modal(on: bool) {
    unsafe {
        MODAL_OPEN = on;
    }
}

fn format_elapsed(d: Duration) -> String {
    let s = d.as_secs();
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
    } else {
        format!("{:02}:{:02}", s / 60, s % 60)
    }
}

impl AppCtx {
    fn lc_action(&self) -> LcAction {
        if self.engine_mode {
            // No Live Captions to turn on; the engine restarts itself.
            return if self.view.listening && self.view.idle.asking() && !self.view.stopping {
                LcAction::KeepRecording
            } else {
                LcAction::None
            };
        }
        if self.view.listening {
            match self.view.health {
                Some(Health::LiveCaptionsOff) => LcAction::TurnOn,
                Some(Health::NotReading) => LcAction::Restart,
                _ if self.view.idle.asking() && !self.view.stopping => LcAction::KeepRecording,
                _ => LcAction::None,
            }
        } else if self.view.idle_lc_on == Some(false) {
            LcAction::TurnOn
        } else {
            LcAction::None
        }
    }

    fn now_ms(&self) -> u64 {
        self.epoch.elapsed().as_millis() as u64
    }

    /// While recording with auto-record on: time left before silence stops it.
    fn auto_stop_in_ms(&self) -> Option<u64> {
        let limit = self.auto.quiet_stop_ms();
        if !self.auto_on || !self.view.listening || limit == 0 {
            return None;
        }
        Some(limit.saturating_sub(self.quiet_ms()))
    }

    /// How long nothing has happened: no speaker sound and, while recording, no new
    /// captions (Live Captions can caption your microphone while the speakers are silent;
    /// "Keep recording" also resets this).
    fn quiet_ms(&self) -> u64 {
        let now = self.now_ms();
        let sound = self.detector.quiet_ms(now);
        if self.view.listening {
            sound.min(self.view.idle.quiet_ms(now))
        } else {
            sound
        }
    }

    /// (headline, guidance, background colour, text colour)
    fn banner(&self) -> (String, String, u32, u32) {
        let v = &self.view;
        if v.listening && v.stopping {
            return (
                "Saving the last words…".into(),
                "Finishing the sentence Live Captions is showing, then closing the file.".into(),
                COL_WAITING,
                COL_WHITE,
            );
        }
        let problem = v.health.is_some_and(|h| h.is_problem());
        if v.listening && v.idle.asking() && !problem {
            let mins = (v.idle.quiet_ms(self.now_ms()) / 60_000).max(1);
            let head = if v.lines.is_empty() {
                format!("Nothing heard for {mins} min — still waiting for your meeting?")
            } else {
                format!("No new captions for {mins} min — are you done?")
            };
            let guidance = match self.auto_stop_in_ms() {
                Some(left) => format!(
                    "If it stays silent, Interpres stops and saves by itself in {} min. Or choose now.",
                    left.div_ceil(60_000).max(1)
                ),
                None => {
                    "Recording keeps going until you choose: Stop & save, or Keep recording.".into()
                }
            };
            return (head, guidance, COL_ACTION, COL_WHITE);
        }
        if !v.listening && self.auto_on {
            let guidance = if self.auto_start_pending.is_some() {
                "Sound is playing — turning on Live Captions, then recording starts.".to_string()
            } else if self.sound_readable == Some(false) {
                "Can't read the speaker level right now, so auto-record is paused.".to_string()
            } else if self.engine_mode {
                format!(
                    "Recording starts by itself when a meeting or video plays. Captions from {}.",
                    self.engine_name
                )
            } else {
                "Recording starts by itself when a meeting or video plays (Live Captions turns on too)."
                    .to_string()
            };
            return (
                "Not recording — auto-record is on".into(),
                guidance,
                self.col_panel,
                self.col_text,
            );
        }
        if !v.listening && self.engine_mode {
            return (
                "Not recording".into(),
                format!(
                    "Press Start recording. Captions come from your engine: {}.",
                    self.engine_name
                ),
                self.col_panel,
                self.col_text,
            );
        }
        if !v.listening {
            return match v.idle_lc_on {
                Some(false) => (
                    "Not recording — Live Captions is off".into(),
                    "Turn on Live Captions first, then press Start recording.".into(),
                    COL_PROBLEM,
                    COL_WHITE,
                ),
                _ => (
                    "Not recording".into(),
                    "Press Start recording before your meeting. Live Captions must stay on."
                        .into(),
                    self.col_panel,
                    self.col_text,
                ),
            };
        }
        let elapsed = v
            .started_at
            .map(|t| format_elapsed(t.elapsed()))
            .unwrap_or_default();
        let lines = match v.lines.len() {
            1 => "1 line".to_string(),
            n => format!("{n} lines"),
        };
        match v.health {
            None => (
                format!("Starting…  ·  {elapsed}"),
                "Connecting to Live Captions.".into(),
                COL_WAITING,
                COL_WHITE,
            ),
            Some(h) => {
                let head = match h {
                    Health::Recording => format!("●  Recording  ·  {elapsed}  ·  {lines}"),
                    Health::WaitingForSpeech => format!("{}  ·  {elapsed}", h.headline()),
                    _ => format!("⚠  {}", h.headline()),
                };
                let bg = match h {
                    Health::Recording => COL_RECORDING,
                    Health::WaitingForSpeech => COL_WAITING,
                    Health::LiveCaptionsOff | Health::NotReading | Health::EngineStopped => {
                        COL_PROBLEM
                    }
                };
                let mut guidance = h.guidance().to_string();
                if self.engine_mode {
                    // Shared guidance talks about Live Captions; say where captions come from.
                    match h {
                        Health::Recording => {
                            guidance = format!("Saving what {} sends.", self.engine_name)
                        }
                        Health::WaitingForSpeech => {
                            guidance = format!(
                                "{} is ready. Lines appear here as soon as someone speaks.",
                                self.engine_name
                            )
                        }
                        _ => {}
                    }
                }
                if h == Health::Recording && !self.remember {
                    guidance = "Showing captions, but NOT saving to disk (turn on in Settings)."
                        .into();
                }
                (head, guidance, bg, COL_WHITE)
            }
        }
    }

    fn checklist(&self) -> String {
        let v = &self.view;
        if self.engine_mode {
            let engine = match (v.listening, v.health) {
                (true, Some(Health::EngineStopped)) => format!("✗ Engine stopped: {}", self.engine_name),
                (true, Some(_)) => format!("✓ Engine running: {}", self.engine_name),
                _ => format!("○ Engine: {}", self.engine_name),
            };
            let reading = match (v.listening, v.health) {
                (true, Some(Health::Recording)) => "✓ Receiving captions",
                (true, Some(Health::WaitingForSpeech)) => "○ Waiting for speech",
                (true, None) => "○ Engine starting…",
                _ => "○ Receiving captions",
            };
            let saving = if self.remember { "✓ Save to disk on" } else { "✗ Not saving to disk" };
            return format!("{engine}        {reading}        {saving}");
        }
        let lc_on = if v.listening {
            v.health.map(|h| h != Health::LiveCaptionsOff)
        } else {
            v.idle_lc_on
        };
        let lc = match lc_on {
            Some(true) => "✓ Live Captions on",
            Some(false) => "✗ Live Captions off",
            None => "○ Live Captions",
        };
        let reading = if !v.listening {
            "○ Reading captions (starts with recording)"
        } else {
            match v.health {
                Some(Health::Recording) => "✓ Reading captions",
                Some(Health::WaitingForSpeech) => "○ Waiting for speech",
                Some(Health::NotReading) => "✗ Can't read captions",
                _ => "○ Reading captions",
            }
        };
        let saving = if !self.remember {
            "✗ Not saving to disk"
        } else if v.listening && v.session_active {
            "✓ Saving to file"
        } else {
            "✓ Save to disk on"
        };
        format!("{lc}        {reading}        {saving}")
    }

    fn footer(&self) -> String {
        match (&self.view.session_path, self.view.session_active) {
            (Some(p), true) => format!("Saving to  {}", p.display()),
            (Some(p), false) => format!("Saved to  {}", p.display()),
            (None, _) if self.remember => {
                format!("Transcripts folder:  {}", self.engine.folder().display())
            }
            (None, _) => "Save to disk is off — transcripts are not being kept.".into(),
        }
    }

    /// Transcript rows as shown (each ends with CRLF): saved lines, then the live line.
    fn transcript_rows(&self) -> Vec<String> {
        let v = &self.view;
        let mut out: Vec<String> = v
            .times
            .iter()
            .zip(&v.lines)
            .map(|(t, line)| format!("{t}   {line}\r\n"))
            .collect();
        let live = v.live.trim();
        // Live Captions can re-show an older line; only show text not already saved.
        let live_is_new = !live.is_empty()
            && !v
                .lines
                .iter()
                .rev()
                .take(6)
                .any(|l| l == live || same_or_refinement(l, live));
        if v.listening && live_is_new {
            out.push(format!("   …     {}\r\n", live.replace('\n', " ")));
        }
        if out.is_empty() {
            out.push(
                if v.listening {
                    "Captions will appear here as they come in."
                } else {
                    "Press Start recording. Captions will appear here, and are saved to a file as you go."
                }
                .to_string(),
            );
        }
        out
    }

    fn plain_transcript(&self) -> String {
        self.view
            .times
            .iter()
            .zip(&self.view.lines)
            .map(|(t, l)| format!("[{t}] {l}\r\n"))
            .collect()
    }
}

fn refresh_static_ui(app: &mut AppCtx) {
    let (head, guidance, bg, fg) = app.banner();
    if bg != app.col_banner || app.brush_banner.is_null() {
        unsafe {
            if !app.brush_banner.is_null() {
                DeleteObject(app.brush_banner);
            }
            app.brush_banner = CreateSolidBrush(bg);
        }
        app.col_banner = bg;
        unsafe {
            InvalidateRect(app.c.main, &app.banner_rect, 1);
        }
    }
    app.col_banner_text = fg;
    let banner_key = format!("{head}\n{guidance}");
    if banner_key != app.view.rendered_banner {
        app.view.rendered_banner = banner_key;
        set_text(app.c.banner_head, &head);
        set_text(app.c.banner_detail, &guidance);
        unsafe {
            InvalidateRect(app.c.banner_head, ptr::null(), 1);
            InvalidateRect(app.c.banner_detail, ptr::null(), 1);
        }
    }

    let title = if !app.view.listening {
        "Interpres".to_string()
    } else {
        match app.view.health {
            Some(Health::Recording) => "● Recording — Interpres".into(),
            Some(h) if h.is_problem() => format!("⚠ {} — Interpres", h.headline()),
            _ => "Interpres — listening".into(),
        }
    };
    if get_text(app.c.main) != title {
        set_text(app.c.main, &title);
    }

    let problem = app.view.health.is_some_and(|h| h.is_problem());
    let toggle_label = if app.view.stopping {
        "Saving…"
    } else if app.view.listening && app.view.idle.asking() && !problem {
        "■   Stop & save"
    } else if app.view.listening {
        "■   Stop recording"
    } else {
        "▶   Start recording"
    };
    if get_text(app.c.toggle) != toggle_label {
        set_text(app.c.toggle, toggle_label);
        unsafe {
            InvalidateRect(app.c.toggle, ptr::null(), 1);
        }
    }

    match app.lc_action() {
        LcAction::None => unsafe {
            ShowWindow(app.c.action, SW_HIDE);
        },
        a => {
            let label = match a {
                LcAction::TurnOn => "Turn on Live Captions",
                LcAction::KeepRecording => "Keep recording",
                _ => "Restart Live Captions",
            };
            if get_text(app.c.action) != label {
                set_text(app.c.action, label);
            }
            unsafe {
                ShowWindow(app.c.action, SW_SHOW);
                InvalidateRect(app.c.action, ptr::null(), 1);
            }
        }
    }

    let checks = app.checklist();
    if get_text(app.c.checks) != checks {
        set_text(app.c.checks, &checks);
    }
    if get_text(app.c.detail) != app.view.detail {
        set_text(app.c.detail, &app.view.detail);
    }
    let footer = app.footer();
    if get_text(app.c.file) != footer {
        set_text(app.c.file, &footer);
    }
    set_enabled(app.c.toggle, !app.view.stopping);
    set_enabled(app.c.open_file, app.view.session_path.is_some());
    set_enabled(app.c.copy, !app.view.lines.is_empty());
}

/// Only touch enabled state on change (each EnableWindow repaints the button).
fn set_enabled(hwnd: Hwnd, on: bool) {
    unsafe {
        if (IsWindowEnabled(hwnd) != 0) != on {
            EnableWindow(hwnd, on as c_int);
        }
    }
}

/// Index of the first row that differs (rows after it must be re-rendered).
fn first_changed_row(old: &[String], new: &[String]) -> usize {
    old.iter()
        .zip(new)
        .position(|(a, b)| a != b)
        .unwrap_or_else(|| old.len().min(new.len()))
}

/// Update transcript text by replacing only the changed tail (cheap for 2 h of lines),
/// keeping the reader's scroll position unless they are at the bottom.
fn render_transcript(app: &mut AppCtx) {
    let hwnd = app.c.transcript;
    let rows = app.transcript_rows();
    let k = first_changed_row(&app.view.rendered_rows, &rows);
    if k == rows.len() && k == app.view.rendered_rows.len() {
        app.view.transcript_dirty = false;
        return;
    }
    let start: usize = app.view.rendered_u16[..k].iter().sum();
    let end: usize = app.view.rendered_u16.iter().sum();
    let tail: String = rows[k..].concat();
    let tail_wide = to_wide(&tail);
    unsafe {
        let mut si = ScrollInfo {
            cb_size: std::mem::size_of::<ScrollInfo>() as u32,
            f_mask: SIF_ALL,
            n_min: 0,
            n_max: 0,
            n_page: 0,
            n_pos: 0,
            n_track_pos: 0,
        };
        let has_scroll = GetScrollInfo(hwnd, SB_VERT, &mut si) != 0;
        let at_bottom = !has_scroll
            || si.n_page == 0
            || si.n_pos + si.n_page as c_int >= si.n_max - 1;
        let first_visible = SendMessageW(hwnd, EM_GETFIRSTVISIBLELINE, 0, 0);
        SendMessageW(hwnd, EM_SETSEL, start, end as Lparam);
        SendMessageW(hwnd, EM_REPLACESEL, 0, tail_wide.as_ptr() as Lparam);
        if at_bottom {
            let len = GetWindowTextLengthW(hwnd) as Wparam;
            SendMessageW(hwnd, EM_SETSEL, len, len as Lparam);
            SendMessageW(hwnd, EM_SCROLLCARET, 0, 0);
        } else {
            // Replacing moves the caret (and view) to the edit point; scroll back.
            let now_first = SendMessageW(hwnd, EM_GETFIRSTVISIBLELINE, 0, 0);
            SendMessageW(hwnd, EM_LINESCROLL, 0, first_visible - now_first);
        }
    }
    app.view.rendered_u16 = rows.iter().map(|r| r.encode_utf16().count()).collect();
    app.view.rendered_rows = rows;
    app.view.transcript_dirty = false;
    app.view.last_render = Some(Instant::now());
}

fn alert_user(main: Hwnd) {
    let info = FlashWInfo {
        cb_size: std::mem::size_of::<FlashWInfo>() as u32,
        hwnd: main,
        dw_flags: FLASHW_ALL | FLASHW_TIMERNOFG,
        u_count: 0,
        dw_timeout: 0,
    };
    unsafe {
        FlashWindowEx(&info);
        MessageBeep(MB_ICONWARNING);
    }
}

/// Gentle attention request (taskbar flash + default sound) for the idle prompt.
fn nudge_user(main: Hwnd) {
    let info = FlashWInfo {
        cb_size: std::mem::size_of::<FlashWInfo>() as u32,
        hwnd: main,
        dw_flags: FLASHW_ALL | FLASHW_TIMERNOFG,
        u_count: 0,
        dw_timeout: 0,
    };
    unsafe {
        FlashWindowEx(&info);
        MessageBeep(MB_OK);
    }
}

fn apply_event(app: &mut AppCtx, ev: EngineEvent) {
    let now_ms = app.now_ms();
    let idle_prompt_ms = app.idle_prompt_ms;
    let engine_mode = app.engine_mode;
    let v = &mut app.view;
    match ev {
        EngineEvent::Status(s) => {
            // Banner + checklist already say these; keep the detail line for news.
            let redundant = s.starts_with("Live Captions detected")
                || s.starts_with("Folder:")
                || s.starts_with("Listening to Live Captions")
                || s.starts_with("Saving what Live Captions shows")
                || s == "Stopped."
                || s.starts_with("Live Captions stopped")
                // The health banner covers stuck / missing captions.
                || s.starts_with(LAG_TIP);
            if !redundant {
                v.detail = s;
            }
        }
        EngineEvent::Error(s) => v.detail = format!("⚠  {s}"),
        EngineEvent::Live(s) => {
            if s != v.live {
                if !s.trim().is_empty() {
                    v.idle.on_activity(now_ms);
                }
                v.live = s;
                v.transcript_dirty = true;
            }
        }
        EngineEvent::Final(s) => {
            // External engines send finished lines: list them exactly as the file does.
            let plan = if engine_mode {
                FamilyPlan::Append
            } else {
                plan_family(&v.lines, s.trim())
            };
            v.idle.on_activity(now_ms);
            apply_caption(v, plan, &s);
        }
        EngineEvent::Revised(s) => {
            let plan = plan_family(&v.lines, s.trim());
            v.idle.on_activity(now_ms);
            apply_caption(v, plan, &s);
        }
        EngineEvent::SessionFile(Some(p)) => {
            v.session_path = Some(p);
            v.session_active = true;
        }
        EngineEvent::SessionFile(None) => v.session_active = false,
        EngineEvent::Listening(on) => {
            v.listening = on;
            v.stopping = false;
            v.health = None;
            v.live.clear();
            v.idle = IdlePrompt::new(if on { idle_prompt_ms } else { 0 }, now_ms);
            if on {
                v.started_at = Some(Instant::now());
                v.lines.clear();
                v.times.clear();
                v.detail.clear();
                v.session_path = None;
                app.auto_start_pending = None;
            } else {
                v.started_at = None;
                v.last_idle_check = None;
                v.detail.clear();
                app.auto.on_stopped();
            }
            if let Some(note) = app.auto_note.take() {
                v.detail = note;
            }
            v.transcript_dirty = true;
        }
        EngineEvent::Health(h) => {
            let was = v.health;
            v.health = Some(h);
            if h == Health::Recording {
                v.detail.clear();
            }
            if v.listening && h.is_problem() && was != Some(h) {
                alert_user(app.c.main);
            }
        }
    }
}

/// Apply a caption the same way the transcript file does; each line keeps the time it
/// was first heard (a merge keeps the earliest).
fn apply_caption(v: &mut View, plan: FamilyPlan, text: &str) {
    let text = text.trim();
    if text.is_empty() || matches!(plan, FamilyPlan::NoOp) {
        return;
    }
    apply_plan(&mut v.lines, &mut v.times, plan, text, || format_clock(SystemTime::now()));
    v.transcript_dirty = true;
}

/// True while `pump_ui` runs: Win32 calls inside it can dispatch messages synchronously.
static mut IN_PUMP: bool = false;

fn pump_ui() {
    unsafe {
        if MODAL_OPEN || IN_PUMP {
            return;
        }
        IN_PUMP = true;
    }
    pump_ui_inner();
    unsafe {
        IN_PUMP = false;
    }
}

fn pump_ui_inner() {
    with_app(|app| {
        loop {
            match app.rx.try_recv() {
                Ok(ev) => apply_event(app, ev),
                Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => break,
            }
        }
        tick_auto_record(app);
        let now_ms = app.now_ms();
        let problem = app.view.health.is_some_and(|h| h.is_problem());
        if app.view.listening && !app.view.stopping && !problem && app.view.idle.tick(now_ms) {
            crate::debuglog::log("ui idle prompt: asking if the meeting is done");
            nudge_user(app.c.main);
        }
        if !app.view.listening && !app.engine_mode {
            let due = app
                .view
                .last_idle_check
                .map_or(true, |t| t.elapsed() >= IDLE_LC_CHECK);
            if due {
                app.view.last_idle_check = Some(Instant::now());
                app.view.idle_lc_on = Some(platform::live_captions_present().running);
            }
        }
        if app.view.transcript_dirty
            && app
                .view
                .last_render
                .map_or(true, |t| t.elapsed() >= TRANSCRIPT_MIN_INTERVAL)
        {
            render_transcript(app);
        }
        refresh_static_ui(app);
    });
}

/// Auto-record: feed the speaker level, then start or stop recording as needed.
fn tick_auto_record(app: &mut AppCtx) {
    let Some(sound) = app.sound.clone() else {
        return;
    };
    let now = app.now_ms();
    let readable = sound.healthy(SOUND_STALE);
    if readable {
        app.detector.sample(now, sound.take_peak());
    } else {
        // Unknown is not silence: never stop a recording on a broken meter.
        app.detector.assume_sound(now);
    }
    if (readable || sound.age() >= SOUND_STALE) && app.sound_readable != Some(readable) {
        app.sound_readable = Some(readable);
        crate::debuglog::log(&format!("auto-record: speaker level readable={readable}"));
        if !readable {
            app.view.detail =
                "⚠  Can't read the speaker level — auto-record won't start or stop by itself.".into();
        }
    }
    if app.view.stopping {
        return;
    }
    if let Some(since) = app.auto_start_pending {
        if app.view.listening {
            app.auto_start_pending = None;
        } else if app.view.idle_lc_on == Some(true) || since.elapsed() >= AUTO_LC_WAIT {
            app.auto_start_pending = None;
            auto_start(app);
        }
        return;
    }
    let playing = readable && app.detector.playing(now);
    let quiet = app.quiet_ms();
    match app.auto.tick(app.view.listening, playing, quiet) {
        AutoAction::None => {}
        AutoAction::Start => {
            refresh_source(app);
            let lc_off = !app.engine_mode
                && app.view.idle_lc_on != Some(true)
                && !platform::live_captions_present().running;
            if lc_off {
                // Start once Live Captions is up, so the banner doesn't flash "off".
                crate::debuglog::log("auto-record: sound playing — turning on Live Captions");
                app.auto_start_pending = Some(Instant::now());
                app.view.idle_lc_on = Some(false);
                app.view.last_idle_check = Some(Instant::now());
                thread::spawn(|| {
                    if let Err(e) = platform::windows::launch_live_captions() {
                        crate::debuglog::log(&format!("auto-record: Live Captions launch failed: {e}"));
                    }
                });
            } else {
                auto_start(app);
            }
        }
        AutoAction::Stop => {
            let mins = app.auto.quiet_stop_ms() / 60_000;
            crate::debuglog::log(&format!("auto-record: no sound for {mins} min — stop and save"));
            app.view.stopping = true;
            app.engine.request_stop_because(&format!("no sound for {mins} min"));
            app.auto_note = Some(format!(
                "Stopped and saved after {mins} min with no sound. Recording starts again when sound plays."
            ));
        }
    }
}

fn auto_start(app: &mut AppCtx) {
    crate::debuglog::log("auto-record: sound playing — start recording");
    refresh_source(app);
    app.auto_note = Some("Sound is playing — recording started automatically.".into());
    app.engine.start();
}

fn child(
    class: &str,
    text: &str,
    style: u32,
    ex: u32,
    parent: Hwnd,
    id: i32,
    instance: Hinstance,
) -> Hwnd {
    let c = to_wide(class);
    let t = to_wide(text);
    unsafe {
        CreateWindowExW(
            ex,
            c.as_ptr(),
            t.as_ptr(),
            WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS | style,
            0,
            0,
            10,
            10,
            parent,
            id as usize as Hmenu,
            instance,
            ptr::null_mut(),
        )
    }
}

fn apply_font(hwnd: Hwnd, font: Hfont) {
    if !hwnd.is_null() && !font.is_null() {
        unsafe {
            SendMessageW(hwnd, WM_SETFONT, font as Wparam, 1);
        }
    }
}

fn layout(app: &mut AppCtx) {
    let c = &app.c;
    let mut rc = Rect::default();
    unsafe {
        GetClientRect(c.main, &mut rc);
    }
    let w = rc.right - rc.left;
    let h = rc.bottom - rc.top;
    let m = 20;
    let bw = (w - m * 2).max(200);
    let mv = |hwnd: Hwnd, x: i32, y: i32, ww: i32, hh: i32| unsafe {
        MoveWindow(hwnd, x, y, ww.max(1), hh.max(1), 1);
    };

    mv(c.title, m, 14, 300, 36);
    mv(c.settings, w - m - 140, 16, 140, 34);
    mv(c.auto, w - m - 140 - 16 - 260, 16, 260, 34);

    let banner = Rect {
        left: m,
        top: 62,
        right: m + bw,
        bottom: 62 + 78,
    };
    mv(c.banner_head, m + 18, 72, bw - 36, 30);
    mv(c.banner_detail, m + 18, 104, bw - 36, 26);

    let by = 156;
    mv(c.toggle, m, by, 240, 48);
    mv(c.action, m + 256, by, 240, 48);
    mv(c.checks, m, by + 62, bw, 24);
    mv(c.detail, m, by + 88, bw, 22);

    let ty = by + 120;
    mv(c.transcript_lbl, m, ty, 200, 22);
    let footer_h = 58;
    let th = (h - (ty + 26) - footer_h).max(80);
    mv(c.transcript, m, ty + 26, bw, th);

    let fy = h - footer_h + 12;
    let btn_w = [180, 120, 140];
    let gap = 10;
    let buttons_w: i32 = btn_w.iter().sum::<i32>() + gap * 2;
    let mut x = m + bw - buttons_w;
    mv(c.file, m, fy + 8, (x - m - 12).max(80), 22);
    for (hwnd, bwid) in [(c.open_file, btn_w[0]), (c.copy, btn_w[1]), (c.open_folder, btn_w[2])] {
        mv(hwnd, x, fy, bwid, 36);
        x += bwid + gap;
    }

    app.banner_rect = banner;
    unsafe {
        InvalidateRect(app.c.main, ptr::null(), 1);
    }
}

/// Owner-drawn rounded button: primary (Start/Stop), action (amber) or neutral.
fn draw_button(app: &AppCtx, dis: &DrawItemStruct) {
    let id = dis.ctl_id as i32;
    if id == IDC_AUTO {
        draw_checkbox(app, dis, app.auto_on);
        return;
    }
    let pressed = dis.item_state & ODS_SELECTED != 0;
    let disabled = dis.item_state & ODS_DISABLED != 0;
    let (mut fill, text_col, border, font) = match id {
        IDC_TOGGLE => {
            let c = if app.view.listening { COL_STOP } else { COL_START };
            (c, COL_WHITE, c, app.font_button)
        }
        IDC_ACTION => (COL_ACTION, COL_WHITE, COL_ACTION, app.font_button),
        _ => (app.col_button, app.col_text, app.col_border, app.font_ui),
    };
    if pressed {
        // Darken ~15%.
        let ch = |shift: u32| (((fill >> shift) & 0xFF) * 85 / 100) << shift;
        fill = ch(0) | ch(8) | ch(16);
    }
    let text_col = if disabled { app.col_muted } else { text_col };
    let mut rc = dis.rc_item;
    unsafe {
        let hdc = dis.hdc;
        FillRect(hdc, &rc, app.brush_bg);
        let brush = CreateSolidBrush(fill);
        let pen = CreatePen(PS_SOLID, 1, border);
        let old_brush = SelectObject(hdc, brush);
        let old_pen = SelectObject(hdc, pen);
        RoundRect(hdc, rc.left, rc.top, rc.right, rc.bottom, 10, 10);
        SelectObject(hdc, old_brush);
        SelectObject(hdc, old_pen);
        DeleteObject(brush);
        DeleteObject(pen);

        let label = to_wide(&get_text(dis.hwnd_item));
        let old_font = SelectObject(hdc, font);
        SetBkMode(hdc, TRANSPARENT);
        SetTextColor(hdc, text_col);
        DrawTextW(
            hdc,
            label.as_ptr(),
            -1,
            &mut rc,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
        );
        SelectObject(hdc, old_font);
        if dis.item_state & ODS_FOCUS != 0 {
            let focus = Rect {
                left: rc.left + 4,
                top: rc.top + 4,
                right: rc.right - 4,
                bottom: rc.bottom - 4,
            };
            DrawFocusRect(hdc, &focus);
        }
    }
}

/// Owner-drawn checkbox: rounded box (green with a tick when on) and a label, in theme colours.
fn draw_checkbox(app: &AppCtx, dis: &DrawItemStruct, checked: bool) {
    let rc = dis.rc_item;
    let size = 20;
    let top = rc.top + (rc.bottom - rc.top - size) / 2;
    let bx = Rect {
        left: rc.left + 2,
        top,
        right: rc.left + 2 + size,
        bottom: top + size,
    };
    unsafe {
        let hdc = dis.hdc;
        FillRect(hdc, &rc, app.brush_bg);
        let (fill, border) = if checked {
            (COL_RECORDING, COL_RECORDING)
        } else {
            (app.col_panel, app.col_muted)
        };
        let brush = CreateSolidBrush(fill);
        let pen = CreatePen(PS_SOLID, 1, border);
        let old_brush = SelectObject(hdc, brush);
        let old_pen = SelectObject(hdc, pen);
        RoundRect(hdc, bx.left, bx.top, bx.right, bx.bottom, 6, 6);
        SelectObject(hdc, old_brush);
        SelectObject(hdc, old_pen);
        DeleteObject(brush);
        DeleteObject(pen);

        SetBkMode(hdc, TRANSPARENT);
        let old_font = SelectObject(hdc, app.font_button);
        if checked {
            let tick = to_wide("✓");
            let mut r = bx;
            SetTextColor(hdc, COL_WHITE);
            DrawTextW(hdc, tick.as_ptr(), -1, &mut r, DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);
        }
        SelectObject(hdc, app.font_ui);
        let label = to_wide(&get_text(dis.hwnd_item));
        let mut lr = Rect {
            left: bx.right + 8,
            ..rc
        };
        SetTextColor(hdc, app.col_text);
        DrawTextW(hdc, label.as_ptr(), -1, &mut lr, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);
        SelectObject(hdc, old_font);
        if dis.item_state & ODS_FOCUS != 0 {
            DrawFocusRect(hdc, &rc);
        }
    }
}

fn apply_theme_colors(app: &mut AppCtx) {
    let is_dark = app.theme_mode.resolve_dark(system_apps_use_dark());
    let p = palette_for_dark(is_dark);
    app.col_bg = p.bg.to_gdi();
    app.col_panel = p.panel.to_gdi();
    app.col_text = p.text.to_gdi();
    app.col_muted = p.muted.to_gdi();
    app.col_button = p.button.to_gdi();
    app.col_border = p.border.to_gdi();
    unsafe {
        for b in [app.brush_bg, app.brush_panel] {
            if !b.is_null() {
                DeleteObject(b);
            }
        }
        app.brush_bg = CreateSolidBrush(app.col_bg);
        app.brush_panel = CreateSolidBrush(app.col_panel);
        // Force banner brush rebuild (idle banner uses the panel colour).
        app.col_banner = u32::MAX;
        app.view.rendered_banner.clear();

        let dark_flag: i32 = is_dark as i32;
        DwmSetWindowAttribute(
            app.c.main,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            &dark_flag as *const i32 as *const c_void,
            std::mem::size_of::<i32>() as u32,
        );
        // Dark scrollbar on the transcript when dark.
        let theme = to_wide(if is_dark { "DarkMode_Explorer" } else { "Explorer" });
        SetWindowTheme(app.c.transcript, theme.as_ptr(), ptr::null());
        InvalidateRect(app.c.main, ptr::null(), 1);
    }
    refresh_static_ui(app);
}

unsafe extern "system" fn wnd_proc(hwnd: Hwnd, msg: u32, wp: Wparam, lp: Lparam) -> Lresult {
    match msg {
        WM_COMMAND => {
            // Only button clicks (BN_CLICKED = 0) and menu items (code 0, lp = 0). Edit
            // notifications (EN_CHANGE when the transcript updates, EN_SETFOCUS, …) must
            // not run commands: EN_CHANGE → pump → transcript update → EN_CHANGE recursed
            // until the stack overflowed.
            let id = (wp & 0xFFFF) as i32;
            let code = ((wp >> 16) & 0xFFFF) as u16;
            if code == 0 {
                on_command(id);
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
            with_app(layout);
            0
        }
        WM_GETMINMAXINFO => {
            let info = &mut *(lp as *mut MinMaxInfo);
            info.min_track = Point { x: 780, y: 560 };
            0
        }
        WM_DRAWITEM => {
            let dis = &*(lp as *const DrawItemStruct);
            with_app(|app| draw_button(app, dis));
            1
        }
        WM_CTLCOLORSTATIC | WM_CTLCOLOREDIT | WM_CTLCOLORBTN => {
            let hdc = wp as Hdc;
            let id = GetDlgCtrlID(lp as Hwnd);
            let brush = with_app(|app| {
                let (text, bk, brush) = match id {
                    IDC_BANNER_HEAD | IDC_BANNER_DETAIL => {
                        (app.col_banner_text, app.col_banner, app.brush_banner)
                    }
                    IDC_TRANSCRIPT => (app.col_text, app.col_panel, app.brush_panel),
                    IDC_DETAIL | IDC_FILE | IDC_CHECKS => (app.col_muted, app.col_bg, app.brush_bg),
                    _ => (app.col_text, app.col_bg, app.brush_bg),
                };
                SetTextColor(hdc, text);
                SetBkColor(hdc, bk);
                brush
            });
            brush.unwrap_or(ptr::null_mut()) as Lresult
        }
        WM_ERASEBKGND => {
            let hdc = wp as Hdc;
            let painted = with_app(|app| {
                let mut rc = Rect::default();
                GetClientRect(hwnd, &mut rc);
                FillRect(hdc, &rc, app.brush_bg);
                let b = app.banner_rect;
                let pen = CreatePen(PS_SOLID, 1, app.col_banner);
                let old_brush = SelectObject(hdc, app.brush_banner);
                let old_pen = SelectObject(hdc, pen);
                RoundRect(hdc, b.left, b.top, b.right, b.bottom, 14, 14);
                SelectObject(hdc, old_brush);
                SelectObject(hdc, old_pen);
                DeleteObject(pen);
            });
            if painted.is_some() {
                1
            } else {
                DefWindowProcW(hwnd, msg, wp, lp)
            }
        }
        WM_SETTINGCHANGE => {
            with_app(|app| {
                if app.theme_mode == ThemeMode::System {
                    apply_theme_colors(app);
                }
            });
            0
        }
        WM_CLOSE => {
            KillTimer(hwnd, IDT_PUMP);
            // Disappear at once; saving the last sentence can take a couple of seconds.
            ShowWindow(hwnd, SW_HIDE);
            with_app(|app| {
                if let Some(s) = app.sound.take() {
                    s.stop();
                }
                app.engine.request_stop_because("window closed");
                app.engine.stop();
            });
            platform::shutdown_capture();
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

fn on_command(id: i32) {
    match id {
        IDC_TOGGLE => {
            let listening = with_app(|app| app.view.listening).unwrap_or(false);
            with_app(|app| {
                if listening {
                    if !app.view.stopping {
                        app.view.stopping = true;
                        app.engine.request_stop();
                        crate::debuglog::log("ui Stop recording clicked");
                    }
                } else {
                    refresh_source(app);
                    app.engine.start();
                    crate::debuglog::log("ui Start recording clicked");
                }
            });
            pump_ui();
        }
        IDC_ACTION => {
            let action = with_app(|app| app.lc_action()).unwrap_or(LcAction::None);
            run_lc_action(action);
        }
        IDM_RESTART_LC => run_lc_action(LcAction::Restart),
        IDC_AUTO => {
            with_app(|app| {
                app.auto_on = !app.auto_on;
                let mut cfg = Config::load();
                cfg.auto_record = app.auto_on;
                let _ = cfg.save();
                if let Some(s) = app.sound.take() {
                    s.stop();
                }
                app.auto_start_pending = None;
                app.sound_readable = None;
                if app.auto_on {
                    let quiet_min = cfg.auto_stop_quiet_minutes;
                    app.auto = AutoRecord::new(quiet_min.saturating_mul(60_000));
                    app.detector = SoundDetector::new(app.now_ms());
                    app.sound = Some(spawn_sound_meter());
                    app.view.detail = if quiet_min > 0 {
                        format!(
                            "Auto-record ON — starts when sound plays, stops and saves after {quiet_min} min of silence."
                        )
                    } else {
                        "Auto-record ON — starts when sound plays.".into()
                    };
                } else {
                    app.view.detail = "Auto-record OFF — press Start recording yourself.".into();
                }
                crate::debuglog::log(&format!("ui auto-record {}", if app.auto_on { "on" } else { "off" }));
                unsafe {
                    InvalidateRect(app.c.auto, ptr::null(), 1);
                }
            });
        }
        IDC_SETTINGS => show_settings_menu(),
        IDC_OPEN_FILE => {
            if let Some((owner, Some(p))) = with_app(|app| (app.c.main, app.view.session_path.clone())) {
                shell_open(owner, "open", &p);
            }
        }
        IDC_COPY => {
            if let Some((owner, text)) = with_app(|app| (app.c.main, app.plain_transcript())) {
                let msg = if text.is_empty() {
                    "Nothing to copy yet."
                } else if copy_to_clipboard(owner, &text) {
                    "Transcript copied — paste it into your notes."
                } else {
                    "⚠  Could not copy to the clipboard."
                };
                with_app(|app| app.view.detail = msg.into());
            }
        }
        IDC_OPEN_FOLDER | IDM_OPEN_FOLDER => {
            if let Some((owner, folder)) = with_app(|app| (app.c.main, app.engine.folder())) {
                let _ = std::fs::create_dir_all(&folder);
                shell_open(owner, "explore", &folder);
            }
        }
        IDM_SAVE => {
            with_app(|app| {
                app.remember = !app.remember;
                app.engine.set_remember(app.remember);
                app.view.detail = if app.remember {
                    "Save to disk is ON — the next recording is saved.".into()
                } else {
                    "Save to disk is OFF — captions are shown but not kept.".into()
                };
            });
        }
        IDM_FOLDER => {
            let owner = with_app(|app| app.c.main).unwrap_or(ptr::null_mut());
            set_modal(true);
            let picked = pick_folder(owner);
            set_modal(false);
            if let Some(path) = picked {
                with_app(|app| {
                    app.engine.set_folder(PathBuf::from(&path));
                    crate::debuglog::set_folder(Path::new(&path));
                    app.view.detail = format!("Transcripts will be saved in {path}");
                });
            }
        }
        IDM_THEME_SYSTEM | IDM_THEME_LIGHT | IDM_THEME_DARK => {
            let mode = match id {
                IDM_THEME_LIGHT => ThemeMode::Light,
                IDM_THEME_DARK => ThemeMode::Dark,
                _ => ThemeMode::System,
            };
            with_app(|app| {
                app.theme_mode = mode;
                apply_theme_colors(app);
            });
            let mut cfg = Config::load();
            cfg.theme = mode;
            let _ = cfg.save();
        }
        IDM_DEBUG => {
            let (on, folder) = with_app(|app| {
                app.debug = !app.debug;
                (app.debug, app.engine.folder())
            })
            .unwrap_or((false, PathBuf::new()));
            crate::debuglog::set_folder(&folder);
            crate::debuglog::set_enabled(on);
            let mut cfg = Config::load();
            cfg.debug = on;
            let _ = cfg.save();
            with_app(|app| {
                app.view.detail = if on {
                    format!(
                        "Debug log ON — {}",
                        crate::debuglog::path_for_display().display()
                    )
                } else {
                    "Debug log OFF.".into()
                };
            });
            if on {
                crate::debuglog::log("debug enabled from UI");
            }
        }
        IDM_CHECK => run_setup_check(),
        IDM_SOURCE_LC | IDM_SOURCE_ENGINE => {
            let mut cfg = Config::load();
            cfg.source = if id == IDM_SOURCE_ENGINE { "engine" } else { "os" }.into();
            let _ = cfg.save();
            with_app(|app| {
                refresh_source(app);
                let what = if app.engine_mode {
                    format!("external engine ({})", app.engine_name)
                } else {
                    "Windows Live Captions".into()
                };
                app.view.detail = if app.view.listening {
                    format!("Caption source: {what}. Takes effect next time you press Start recording.")
                } else {
                    format!("Caption source: {what}.")
                };
                app.view.idle_lc_on = None;
                app.view.last_idle_check = None;
            });
        }
        IDM_START_WITH_WINDOWS => {
            let on = read_startup_entry().is_none();
            let ok = set_start_with_windows(on);
            crate::debuglog::log(&format!("ui start with Windows on={on} ok={ok}"));
            with_app(|app| {
                app.view.detail = match (ok, on) {
                    (true, true) => "Interpres will start (minimized) when you sign in to Windows, so auto-record is always ready.".into(),
                    (true, false) => "Interpres will no longer start with Windows.".into(),
                    (false, _) => "⚠  Could not change the Windows startup setting.".into(),
                };
            });
        }
        IDM_EDIT_SETTINGS => {
            let path = crate::config::config_path();
            if !path.exists() {
                let _ = Config::load().save();
            }
            let owner = with_app(|app| app.c.main).unwrap_or(ptr::null_mut());
            let op = to_wide("open");
            let notepad = to_wide("notepad.exe");
            let file = to_wide(&format!("\"{}\"", path.display()));
            unsafe {
                ShellExecuteW(owner, op.as_ptr(), notepad.as_ptr(), file.as_ptr(), ptr::null(), SW_SHOWNORMAL);
            }
            with_app(|app| {
                app.view.detail =
                    "Opened the settings file. Changes apply next time you press Start recording.".into()
            });
        }
        _ => {}
    }
    pump_ui();
}

/// Turn on / restart Live Captions off the UI thread (restart waits on taskkill).
fn run_lc_action(action: LcAction) {
    if action == LcAction::KeepRecording {
        with_app(|app| {
            let now = app.now_ms();
            app.view.idle.snooze(now);
            app.view.detail =
                "OK — still recording. Interpres will check again if it stays quiet.".into();
        });
        crate::debuglog::log("ui idle prompt: keep recording");
        return;
    }
    let msg = match action {
        LcAction::None | LcAction::KeepRecording => return,
        LcAction::TurnOn => "Turning on Live Captions…",
        LcAction::Restart => "Restarting Live Captions… (recording continues in the same file)",
    };
    with_app(|app| app.view.detail = msg.into());
    crate::debuglog::log(&format!("ui {msg}"));
    thread::spawn(move || {
        let res = match action {
            LcAction::TurnOn => platform::windows::launch_live_captions(),
            _ => platform::windows::restart_live_captions(),
        };
        if let Err(e) = res {
            crate::debuglog::log(&format!("Live Captions action failed: {e}"));
        }
    });
}

/// Re-read the caption source from settings (the file may have been edited by hand).
fn refresh_source(app: &mut AppCtx) {
    let cfg = Config::load();
    app.engine_mode = cfg.uses_external_engine();
    app.engine_name = cfg.engine_name();
    app.engine_configured = cfg.helper_path.is_some();
    app.idle_prompt_ms = cfg.idle_prompt_minutes.saturating_mul(60_000);
    app.auto
        .set_quiet_stop_ms(cfg.auto_stop_quiet_minutes.saturating_mul(60_000));
}

fn run_setup_check() {
    with_app(|app| app.view.detail = "Checking Live Captions…".into());
    let listening = with_app(|app| app.view.listening).unwrap_or(false);
    let presence = platform::live_captions_present();
    let msg = if !presence.running {
        "Live Captions is off. Use “Turn on Live Captions” (or Win+Ctrl+L).".to_string()
    } else {
        let snap = platform::poll_capture();
        if !listening {
            platform::shutdown_capture();
        }
        match (snap.surface_text, snap.error) {
            (Some(_), _) => "✓ All good — Interpres can read Live Captions.".into(),
            (None, None) => {
                "✓ Live Captions is on and readable — waiting for someone to speak.".into()
            }
            (None, Some(e)) => format!("⚠  Live Captions is on but can't be read: {e}"),
        }
    };
    with_app(|app| app.view.detail = msg);
}

fn show_settings_menu() {
    let Some((owner, button, remember, debug, theme, engine_mode, engine_configured, engine_name)) =
        with_app(|app| {
            (
                app.c.main,
                app.c.settings,
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
    unsafe {
        let menu = CreatePopupMenu();
        if menu.is_null() {
            return;
        }
        let add = |flags: u32, id: i32, text: &str| {
            let w = to_wide(text);
            AppendMenuW(menu, flags, id as usize, w.as_ptr());
        };
        let check = |on: bool| if on { MF_CHECKED } else { 0 };
        add(MF_STRING | check(remember), IDM_SAVE, "Save transcripts to disk");
        add(MF_STRING, IDM_FOLDER, "Change transcripts folder…");
        add(MF_STRING, IDM_OPEN_FOLDER, "Open transcripts folder");
        AppendMenuW(menu, MF_SEPARATOR, 0, ptr::null());
        add(MF_STRING | check(!engine_mode), IDM_SOURCE_LC, "Captions from: Windows Live Captions");
        let engine_label = if engine_configured {
            format!("Captions from: external engine ({engine_name})")
        } else {
            "Captions from: external engine (set it up in the settings file)".into()
        };
        add(
            MF_STRING | check(engine_mode) | if engine_configured { 0 } else { MF_GRAYED },
            IDM_SOURCE_ENGINE,
            &engine_label,
        );
        add(
            MF_STRING | check(read_startup_entry().is_some()),
            IDM_START_WITH_WINDOWS,
            "Start Interpres when Windows starts (keeps auto-record ready)",
        );
        add(MF_STRING, IDM_EDIT_SETTINGS, "Edit settings file…");
        AppendMenuW(menu, MF_SEPARATOR, 0, ptr::null());
        let lc_flags = if engine_mode { MF_GRAYED } else { 0 };
        add(MF_STRING | lc_flags, IDM_CHECK, "Check Live Captions setup");
        add(MF_STRING | lc_flags, IDM_RESTART_LC, "Restart Live Captions");
        AppendMenuW(menu, MF_SEPARATOR, 0, ptr::null());
        add(MF_STRING | check(theme == ThemeMode::System), IDM_THEME_SYSTEM, "Theme: match Windows");
        add(MF_STRING | check(theme == ThemeMode::Light), IDM_THEME_LIGHT, "Theme: light");
        add(MF_STRING | check(theme == ThemeMode::Dark), IDM_THEME_DARK, "Theme: dark");
        AppendMenuW(menu, MF_SEPARATOR, 0, ptr::null());
        add(MF_STRING | check(debug), IDM_DEBUG, "Write debug log (for troubleshooting)");

        let mut rc = Rect::default();
        GetWindowRect(button, &mut rc);
        set_modal(true);
        let cmd = TrackPopupMenu(menu, TPM_RETURNCMD, rc.left, rc.bottom + 2, 0, owner, ptr::null());
        set_modal(false);
        DestroyMenu(menu);
        if cmd != 0 {
            on_command(cmd);
        }
    }
}

fn shell_open(owner: Hwnd, verb: &str, path: &Path) {
    let op = to_wide(verb);
    let file = to_wide(&path.to_string_lossy());
    unsafe {
        ShellExecuteW(
            owner,
            op.as_ptr(),
            file.as_ptr(),
            ptr::null(),
            ptr::null(),
            SW_SHOWNORMAL,
        );
    }
}

fn copy_to_clipboard(owner: Hwnd, text: &str) -> bool {
    let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    let bytes = wide.len() * 2;
    unsafe {
        if OpenClipboard(owner) == 0 {
            return false;
        }
        EmptyClipboard();
        let mem = GlobalAlloc(GMEM_MOVEABLE, bytes);
        let mut ok = false;
        if !mem.is_null() {
            let dst = GlobalLock(mem) as *mut u16;
            if !dst.is_null() {
                ptr::copy_nonoverlapping(wide.as_ptr(), dst, wide.len());
                GlobalUnlock(mem);
                ok = !SetClipboardData(CF_UNICODETEXT, mem).is_null();
            }
            if !ok {
                GlobalFree(mem);
            }
        }
        CloseClipboard();
        ok
    }
}

fn pick_folder(owner: Hwnd) -> Option<String> {
    // BIF_NEWDIALOGSTYLE needs COM apartment init on this thread.
    let hr = unsafe { CoInitializeEx(ptr::null_mut(), COINIT_APARTMENTTHREADED) };
    let title = to_wide("Choose where transcripts are saved");
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
        let s = from_wide(&path);
        (ok != 0 && !s.is_empty()).then_some(s)
    };
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

/// Load icon from PE resource (id 1) or `Interpres.ico` beside the exe.
fn load_app_icon(instance: Hinstance) -> Hwnd {
    unsafe {
        let from_res = LoadImageW(instance, 1usize as *const u16, IMAGE_ICON, 0, 0, LR_DEFAULTSIZE);
        if !from_res.is_null() {
            return from_res;
        }
    }
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("Interpres.ico"));
            candidates.push(dir.join("assets").join("Interpres.ico"));
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("assets").join("Interpres.ico"));
    }
    for c in candidates.iter().filter(|c| c.is_file()) {
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

    let start_minimized = std::env::args().any(|a| a == MINIMIZED_ARG);
    // Portable app: if the folder moved, point the startup entry at this exe.
    if read_startup_entry().is_some_and(|cmd| Some(cmd) != startup_command()) {
        set_start_with_windows(true);
    }

    let (engine, rx) = CaptureEngine::new(&cfg);
    let remember0 = engine.remember();

    let instance = unsafe { GetModuleHandleW(ptr::null()) };
    let class_name = to_wide("InterpresMainWnd");
    let face = to_wide("Segoe UI");
    let app_icon = load_app_icon(instance);

    let wc = WndClassExW {
        cb_size: std::mem::size_of::<WndClassExW>() as u32,
        style: 0,
        lpfn_wnd_proc: Some(wnd_proc),
        cb_cls_extra: 0,
        cb_wnd_extra: 0,
        h_instance: instance,
        h_icon: if app_icon.is_null() {
            unsafe { LoadIconW(ptr::null_mut(), IDI_APPLICATION) }
        } else {
            app_icon
        },
        h_cursor: unsafe { LoadCursorW(ptr::null_mut(), IDC_ARROW) },
        hbr_background: ptr::null_mut(),
        lpsz_menu_name: ptr::null(),
        lpsz_class_name: class_name.as_ptr(),
        h_icon_sm: app_icon,
    };
    unsafe {
        RegisterClassExW(&wc);
    }

    let title = to_wide("Interpres");
    let main = unsafe {
        CreateWindowExW(
            0,
            class_name.as_ptr(),
            title.as_ptr(),
            WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            1000,
            780,
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

    let label = SS_LEFT | SS_NOPREFIX;
    let btn = BS_OWNERDRAW | WS_TABSTOP;
    let c = Controls {
        main,
        title: child("STATIC", "Interpres", label, 0, main, IDC_TITLE, instance),
        settings: child("BUTTON", "Settings  ▾", btn, 0, main, IDC_SETTINGS, instance),
        banner_head: child("STATIC", "", label | SS_ENDELLIPSIS, 0, main, IDC_BANNER_HEAD, instance),
        banner_detail: child("STATIC", "", label | SS_ENDELLIPSIS, 0, main, IDC_BANNER_DETAIL, instance),
        toggle: child("BUTTON", "▶   Start recording", btn, 0, main, IDC_TOGGLE, instance),
        action: child("BUTTON", "Turn on Live Captions", btn, 0, main, IDC_ACTION, instance),
        checks: child("STATIC", "", label | SS_ENDELLIPSIS, 0, main, IDC_CHECKS, instance),
        detail: child("STATIC", "", label | SS_ENDELLIPSIS, 0, main, IDC_DETAIL, instance),
        transcript_lbl: child("STATIC", "Transcript", label, 0, main, IDC_TRANSCRIPT_LBL, instance),
        transcript: child(
            "EDIT",
            "",
            ES_MULTILINE | ES_READONLY | ES_AUTOVSCROLL | WS_VSCROLL | WS_TABSTOP,
            WS_EX_CLIENTEDGE,
            main,
            IDC_TRANSCRIPT,
            instance,
        ),
        file: child("STATIC", "", label | SS_PATHELLIPSIS, 0, main, IDC_FILE, instance),
        open_file: child("BUTTON", "Open transcript", btn, 0, main, IDC_OPEN_FILE, instance),
        copy: child("BUTTON", "Copy all", btn, 0, main, IDC_COPY, instance),
        open_folder: child("BUTTON", "Open folder", btn, 0, main, IDC_OPEN_FOLDER, instance),
        auto: child("BUTTON", "Auto-record when sound plays", btn, 0, main, IDC_AUTO, instance),
    };
    unsafe {
        ShowWindow(c.action, SW_HIDE);
        // Default edit limit is ~32K characters (~280 lines) and applies to EM_REPLACESEL;
        // 0 = maximum, so multi-hour transcripts keep growing in the window.
        SendMessageW(c.transcript, EM_SETLIMITTEXT, 0, 0);
    }

    let font_ui = make_font(&face, -16, FW_NORMAL);
    let font_title = make_font(&face, -28, FW_BOLD);
    let font_banner = make_font(&face, -21, FW_SEMIBOLD);
    let font_button = make_font(&face, -17, FW_SEMIBOLD);
    let font_transcript = make_font(&face, -18, FW_NORMAL);
    let font_small = make_font(&face, -15, FW_NORMAL);
    apply_font(c.title, font_title);
    apply_font(c.banner_head, font_banner);
    apply_font(c.transcript, font_transcript);
    for h in [c.banner_detail, c.checks, c.transcript_lbl, c.settings] {
        apply_font(h, font_ui);
    }
    for h in [c.detail, c.file] {
        apply_font(h, font_small);
    }

    let app = Box::new(AppCtx {
        engine,
        rx,
        c,
        view: View {
            listening: false,
            health: None,
            idle_lc_on: None,
            last_idle_check: None,
            started_at: None,
            lines: Vec::new(),
            times: Vec::new(),
            live: String::new(),
            session_path: None,
            session_active: false,
            detail: String::new(),
            transcript_dirty: true,
            last_render: None,
            rendered_banner: String::new(),
            rendered_rows: Vec::new(),
            rendered_u16: Vec::new(),
            stopping: false,
            idle: IdlePrompt::new(0, 0),
        },
        remember: remember0,
        debug: cfg.debug,
        theme_mode: cfg.theme,
        idle_prompt_ms: cfg.idle_prompt_minutes.saturating_mul(60_000),
        engine_mode: cfg.uses_external_engine(),
        engine_name: cfg.engine_name(),
        engine_configured: cfg.helper_path.is_some(),
        epoch: Instant::now(),
        auto_on: cfg.auto_record,
        auto: AutoRecord::new(cfg.auto_stop_quiet_minutes.saturating_mul(60_000)),
        sound: cfg.auto_record.then(spawn_sound_meter),
        detector: SoundDetector::new(0),
        sound_readable: None,
        auto_start_pending: None,
        auto_note: None,
        font_ui,
        font_title,
        font_banner,
        font_button,
        font_transcript,
        font_small,
        col_bg: 0,
        col_panel: 0,
        col_text: 0,
        col_muted: 0,
        col_button: 0,
        col_border: 0,
        brush_bg: ptr::null_mut(),
        brush_panel: ptr::null_mut(),
        brush_banner: ptr::null_mut(),
        col_banner: u32::MAX,
        col_banner_text: 0,
        banner_rect: Rect::default(),
    });
    unsafe {
        APP = Box::into_raw(app);
    }
    with_app(|app| {
        layout(app);
        apply_theme_colors(app);
    });
    pump_ui();
    unsafe {
        SetTimer(main, IDT_PUMP, PUMP_MS, ptr::null());
        ShowWindow(main, if start_minimized { SW_SHOWMINNOACTIVE } else { SW_SHOW });
        UpdateWindow(main);
        SetFocus(main);
    }

    crate::debuglog::log(&format!(
        "ui ready folder={} remember={} debug={} theme={}",
        cfg.transcript_folder.display(),
        remember0,
        cfg.debug,
        cfg.theme.as_str()
    ));

    let mut msg = Msg {
        hwnd: ptr::null_mut(),
        message: 0,
        w_param: 0,
        l_param: 0,
        time: 0,
        pt: Point::default(),
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

    // Teardown (engine already stopped on WM_CLOSE; stop again is harmless).
    unsafe {
        if !APP.is_null() {
            let app = Box::from_raw(APP);
            APP = ptr::null_mut();
            app.engine.stop();
            for obj in [
                app.font_ui,
                app.font_title,
                app.font_banner,
                app.font_button,
                app.font_transcript,
                app.font_small,
                app.brush_bg,
                app.brush_panel,
                app.brush_banner,
            ] {
                if !obj.is_null() {
                    DeleteObject(obj);
                }
            }
        }
    }
    platform::shutdown_capture();
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elapsed_formats_minutes_then_hours() {
        assert_eq!(format_elapsed(Duration::from_secs(5)), "00:05");
        assert_eq!(format_elapsed(Duration::from_secs(14 * 60 + 32)), "14:32");
        assert_eq!(format_elapsed(Duration::from_secs(3600 + 62)), "1:01:02");
    }

    #[test]
    fn only_the_changed_tail_is_rerendered() {
        let r = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        // New line appended: keep everything before it.
        assert_eq!(first_changed_row(&r(&["a", "b"]), &r(&["a", "b", "c"])), 2);
        // Live line changed: only the last row.
        assert_eq!(first_changed_row(&r(&["a", "live1"]), &r(&["a", "live2"])), 1);
        // A recent line polished: re-render from that line.
        assert_eq!(first_changed_row(&r(&["a", "b", "c"]), &r(&["a", "B", "c"])), 1);
        // Live line removed after it became a saved line.
        assert_eq!(first_changed_row(&r(&["a", "live"]), &r(&["a"])), 1);
        assert_eq!(first_changed_row(&r(&["a"]), &r(&["a"])), 1);
    }

    /// Writes the real per-user startup entry, then restores it:
    /// `cargo test start_with_windows_roundtrip -- --ignored`.
    #[test]
    #[ignore]
    fn start_with_windows_roundtrip() {
        let before = read_startup_entry();
        assert!(set_start_with_windows(true));
        let cmd = read_startup_entry().expect("entry written");
        assert_eq!(Some(cmd.clone()), startup_command());
        assert!(cmd.starts_with('"') && cmd.ends_with(" gui --minimized"), "{cmd}");
        assert!(set_start_with_windows(false));
        assert_eq!(read_startup_entry(), None);
        assert!(set_start_with_windows(false), "removing twice is fine");
        if before.is_some() {
            set_start_with_windows(true);
        }
    }

    #[test]
    fn rgb_is_gdi_order() {
        assert_eq!(rgb(0x12, 0x34, 0x56), 0x0056_3412);
    }

    #[test]
    fn line_times_stay_in_step_with_history() {
        let mut v = View {
            listening: true,
            health: None,
            idle_lc_on: None,
            last_idle_check: None,
            started_at: None,
            lines: Vec::new(),
            times: Vec::new(),
            live: String::new(),
            session_path: None,
            session_active: false,
            detail: String::new(),
            transcript_dirty: false,
            last_render: None,
            rendered_banner: String::new(),
            rendered_rows: Vec::new(),
            rendered_u16: Vec::new(),
            stopping: false,
            idle: IdlePrompt::new(0, 0),
        };
        let add = |v: &mut View, t: &str| {
            let plan = plan_family(&v.lines, t);
            apply_caption(v, plan, t);
        };
        add(&mut v, "We can meet on Thursday.");
        let first_time = v.times[0].clone();
        add(&mut v, "I'll send the invite tonight.");
        assert_eq!(v.lines.len(), 2);
        assert_eq!(v.times.len(), 2);
        // Polishing the first line keeps its time.
        add(&mut v, "We can meet on Thursday, if that suits you.");
        assert_eq!(v.times.len(), v.lines.len());
        assert_eq!(v.times[0], first_time);
        assert!(v.transcript_dirty);
    }
}
