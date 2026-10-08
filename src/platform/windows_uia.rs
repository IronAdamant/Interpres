//! In-process UI Automation reader for Windows Live Captions (hand-written COM FFI).
//!
//! Zero crates: vtable slots below are checked against MinGW `uiautomationclient.h`
//! (slot numbers include the three IUnknown entries). Uses `IUIAutomation2` timeouts so
//! a busy or frozen Live Captions fails a call in ~2 s instead of the 20 s UIA default.
//!
//! Must be used on a thread that called `CoInitializeEx(.., COINIT_MULTITHREADED)`.

use std::os::raw::c_void;
use std::ptr;

type Hresult = i32;
type Bstr = *mut u16;

#[repr(C)]
struct Guid {
    data1: u32,
    data2: u16,
    data3: u16,
    data4: [u8; 8],
}

/// VARIANT (24 bytes on x64). Only VT_BSTR is used here.
#[repr(C)]
struct Variant {
    vt: u16,
    reserved1: u16,
    reserved2: u16,
    reserved3: u16,
    val: [usize; 2],
}

#[link(name = "ole32")]
extern "system" {
    pub fn CoInitializeEx(reserved: *mut c_void, coinit: u32) -> Hresult;
    pub fn CoUninitialize();
    fn CoCreateInstance(
        clsid: *const Guid,
        outer: *mut c_void,
        clsctx: u32,
        iid: *const Guid,
        out: *mut *mut c_void,
    ) -> Hresult;
}

#[link(name = "oleaut32")]
extern "system" {
    fn SysAllocString(s: *const u16) -> Bstr;
    fn SysFreeString(s: Bstr);
    fn SysStringLen(s: Bstr) -> u32;
}

#[link(name = "user32")]
extern "system" {
    fn FindWindowW(class: *const u16, title: *const u16) -> *mut c_void;
}

pub const COINIT_MULTITHREADED: u32 = 0x0;
const CLSCTX_INPROC_SERVER: u32 = 0x1;
const VT_BSTR: u16 = 8;
const TREE_SCOPE_DESCENDANTS: i32 = 0x4;
const UIA_AUTOMATION_ID_PROPERTY_ID: i32 = 30011;
const UIA_E_ELEMENTNOTAVAILABLE: Hresult = 0x8004_0201_u32 as i32;
const UIA_E_TIMEOUT: Hresult = 0x8013_1505_u32 as i32;

/// Per-call timeouts (IUIAutomation2). Defaults are 2 s connect / 20 s transaction.
const CONNECTION_TIMEOUT_MS: u32 = 1500;
const TRANSACTION_TIMEOUT_MS: u32 = 2500;
/// Re-resolve the caption element every N reads in case Live Captions rebuilt its UI.
const REFIND_EVERY: u32 = 40;

// CLSID_CUIAutomation8 {e22ad333-b25f-460c-83d0-0581107395c9}
const CLSID_CUI_AUTOMATION8: Guid = Guid {
    data1: 0xe22a_d333,
    data2: 0xb25f,
    data3: 0x460c,
    data4: [0x83, 0xd0, 0x05, 0x81, 0x10, 0x73, 0x95, 0xc9],
};
// IID_IUIAutomation2 {34723aff-0c9d-49d0-9896-7ab52df8cd8a}
const IID_IUI_AUTOMATION2: Guid = Guid {
    data1: 0x3472_3aff,
    data2: 0x0c9d,
    data3: 0x49d0,
    data4: [0x98, 0x96, 0x7a, 0xb5, 0x2d, 0xf8, 0xcd, 0x8a],
};
// CLSID_CUIAutomation {ff48dba4-60ef-4201-aa87-54103eef594e}
const CLSID_CUI_AUTOMATION: Guid = Guid {
    data1: 0xff48_dba4,
    data2: 0x60ef,
    data3: 0x4201,
    data4: [0xaa, 0x87, 0x54, 0x10, 0x3e, 0xef, 0x59, 0x4e],
};
// IID_IUIAutomation {30cbe57d-d9d0-452a-ab13-7ac5ac4825ee}
const IID_IUI_AUTOMATION: Guid = Guid {
    data1: 0x30cb_e57d,
    data2: 0xd9d0,
    data3: 0x452a,
    data4: [0xab, 0x13, 0x7a, 0xc5, 0xac, 0x48, 0x25, 0xee],
};

// Vtable slots (absolute, IUnknown = 0..2).
const SLOT_RELEASE: usize = 2;
const SLOT_UIA_ELEMENT_FROM_HANDLE: usize = 6;
const SLOT_UIA_CREATE_PROPERTY_CONDITION: usize = 23;
const SLOT_UIA2_PUT_CONNECTION_TIMEOUT: usize = 61;
const SLOT_UIA2_PUT_TRANSACTION_TIMEOUT: usize = 63;
const SLOT_ELEMENT_FIND_FIRST: usize = 5;
const SLOT_ELEMENT_GET_CURRENT_NAME: usize = 23;

/// Fetch vtable entry `slot` of COM object `obj`.
unsafe fn vfn(obj: *mut c_void, slot: usize) -> *const c_void {
    let vtbl = *(obj as *const *const *const c_void);
    *vtbl.add(slot)
}

unsafe fn release(obj: *mut c_void) {
    if !obj.is_null() {
        let f: unsafe extern "system" fn(*mut c_void) -> u32 =
            std::mem::transmute(vfn(obj, SLOT_RELEASE));
        f(obj);
    }
}

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn hr_text(hr: Hresult) -> String {
    match hr {
        UIA_E_TIMEOUT => "UIA call timed out (Live Captions busy or frozen)".into(),
        UIA_E_ELEMENTNOTAVAILABLE => "caption element went away".into(),
        _ => format!("UIA error 0x{:08x}", hr as u32),
    }
}

/// One read of the Live Captions window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CaptionRead {
    /// Caption text currently shown.
    Text(String),
    /// Window open, no caption text yet (waiting for speech). Not an error.
    Waiting,
    /// Live Captions window not found.
    NoWindow,
    /// UIA call failed (element cache dropped; next read re-resolves).
    Error(String),
}

/// Owns the UIA client, AutomationId conditions and the cached caption element.
pub struct CaptionsUia {
    automation: *mut c_void,
    cond_text_block: *mut c_void,
    cond_scroll_viewer: *mut c_void,
    window_class: Vec<u16>,
    hwnd: *mut c_void,
    element: *mut c_void,
    reads: u32,
    /// False when only the Win7-era interface is available (no per-call timeouts).
    pub has_timeouts: bool,
}

// Raw COM pointers stay on the creating MTA thread; only moved at construction.
unsafe impl Send for CaptionsUia {}

impl CaptionsUia {
    pub fn new(window_class: &str, text_ids: &[&str]) -> Result<Self, String> {
        let mut automation: *mut c_void = ptr::null_mut();
        let mut has_timeouts = true;
        unsafe {
            let hr = CoCreateInstance(
                &CLSID_CUI_AUTOMATION8,
                ptr::null_mut(),
                CLSCTX_INPROC_SERVER,
                &IID_IUI_AUTOMATION2,
                &mut automation,
            );
            if hr < 0 || automation.is_null() {
                has_timeouts = false;
                let hr = CoCreateInstance(
                    &CLSID_CUI_AUTOMATION,
                    ptr::null_mut(),
                    CLSCTX_INPROC_SERVER,
                    &IID_IUI_AUTOMATION,
                    &mut automation,
                );
                if hr < 0 || automation.is_null() {
                    return Err(format!("could not create UI Automation ({})", hr_text(hr)));
                }
            }
        }

        let mut me = Self {
            automation,
            cond_text_block: ptr::null_mut(),
            cond_scroll_viewer: ptr::null_mut(),
            window_class: to_wide(window_class),
            hwnd: ptr::null_mut(),
            element: ptr::null_mut(),
            reads: 0,
            has_timeouts,
        };

        if has_timeouts {
            unsafe {
                let put: unsafe extern "system" fn(*mut c_void, u32) -> Hresult =
                    std::mem::transmute(vfn(automation, SLOT_UIA2_PUT_CONNECTION_TIMEOUT));
                put(automation, CONNECTION_TIMEOUT_MS);
                let put: unsafe extern "system" fn(*mut c_void, u32) -> Hresult =
                    std::mem::transmute(vfn(automation, SLOT_UIA2_PUT_TRANSACTION_TIMEOUT));
                put(automation, TRANSACTION_TIMEOUT_MS);
            }
        }

        let primary = text_ids.first().copied().unwrap_or("CaptionsTextBlock");
        me.cond_text_block = me.automation_id_condition(primary)?;
        if let Some(fallback) = text_ids.get(1) {
            me.cond_scroll_viewer = me.automation_id_condition(fallback)?;
        }
        Ok(me)
    }

    fn automation_id_condition(&self, id: &str) -> Result<*mut c_void, String> {
        let wide = to_wide(id);
        unsafe {
            let bstr = SysAllocString(wide.as_ptr());
            if bstr.is_null() {
                return Err("out of memory (BSTR)".into());
            }
            let mut v = Variant {
                vt: VT_BSTR,
                reserved1: 0,
                reserved2: 0,
                reserved3: 0,
                val: [0, 0],
            };
            v.val[0] = bstr as usize;
            let mut cond: *mut c_void = ptr::null_mut();
            let f: unsafe extern "system" fn(*mut c_void, i32, Variant, *mut *mut c_void) -> Hresult =
                std::mem::transmute(vfn(self.automation, SLOT_UIA_CREATE_PROPERTY_CONDITION));
            let hr = f(self.automation, UIA_AUTOMATION_ID_PROPERTY_ID, v, &mut cond);
            // The condition keeps its own copy of the value.
            SysFreeString(bstr);
            if hr < 0 || cond.is_null() {
                return Err(format!("CreatePropertyCondition failed ({})", hr_text(hr)));
            }
            Ok(cond)
        }
    }

    fn drop_element(&mut self) {
        unsafe { release(self.element) };
        self.element = ptr::null_mut();
    }

    /// Find the caption text element under `hwnd`. Ok(null) when not present yet.
    fn find_element(&self, hwnd: *mut c_void) -> Result<*mut c_void, Hresult> {
        unsafe {
            let mut root: *mut c_void = ptr::null_mut();
            let from_handle: unsafe extern "system" fn(
                *mut c_void,
                *mut c_void,
                *mut *mut c_void,
            ) -> Hresult = std::mem::transmute(vfn(self.automation, SLOT_UIA_ELEMENT_FROM_HANDLE));
            let hr = from_handle(self.automation, hwnd, &mut root);
            if hr < 0 || root.is_null() {
                return Err(hr);
            }
            let find_first: unsafe extern "system" fn(
                *mut c_void,
                i32,
                *mut c_void,
                *mut *mut c_void,
            ) -> Hresult = std::mem::transmute(vfn(root, SLOT_ELEMENT_FIND_FIRST));
            let mut found: *mut c_void = ptr::null_mut();
            for cond in [self.cond_text_block, self.cond_scroll_viewer] {
                if cond.is_null() {
                    continue;
                }
                let hr = find_first(root, TREE_SCOPE_DESCENDANTS, cond, &mut found);
                if hr < 0 {
                    release(root);
                    return Err(hr);
                }
                if !found.is_null() {
                    break;
                }
            }
            release(root);
            Ok(found)
        }
    }

    fn current_name(&self) -> Result<String, Hresult> {
        unsafe {
            let mut bstr: Bstr = ptr::null_mut();
            let get_name: unsafe extern "system" fn(*mut c_void, *mut Bstr) -> Hresult =
                std::mem::transmute(vfn(self.element, SLOT_ELEMENT_GET_CURRENT_NAME));
            let hr = get_name(self.element, &mut bstr);
            if hr < 0 {
                return Err(hr);
            }
            if bstr.is_null() {
                return Ok(String::new());
            }
            let len = SysStringLen(bstr) as usize;
            let text = String::from_utf16_lossy(std::slice::from_raw_parts(bstr, len));
            SysFreeString(bstr);
            Ok(text)
        }
    }

    /// Read the text Live Captions is showing right now.
    pub fn read(&mut self) -> CaptionRead {
        let hwnd = unsafe { FindWindowW(self.window_class.as_ptr(), ptr::null()) };
        if hwnd.is_null() {
            self.drop_element();
            self.hwnd = ptr::null_mut();
            return CaptionRead::NoWindow;
        }

        self.reads = self.reads.wrapping_add(1);
        if hwnd != self.hwnd || self.reads % REFIND_EVERY == 0 {
            self.drop_element();
            self.hwnd = hwnd;
        }

        if self.element.is_null() {
            match self.find_element(hwnd) {
                Ok(el) if el.is_null() => return CaptionRead::Waiting,
                Ok(el) => self.element = el,
                Err(hr) => return CaptionRead::Error(hr_text(hr)),
            }
        }

        match self.current_name() {
            Ok(text) if text.trim().is_empty() => CaptionRead::Waiting,
            Ok(text) => CaptionRead::Text(text.replace('\r', "")),
            Err(hr) => {
                self.drop_element();
                CaptionRead::Error(hr_text(hr))
            }
        }
    }
}

impl Drop for CaptionsUia {
    fn drop(&mut self) {
        unsafe {
            release(self.element);
            release(self.cond_text_block);
            release(self.cond_scroll_viewer);
            release(self.automation);
        }
    }
}

