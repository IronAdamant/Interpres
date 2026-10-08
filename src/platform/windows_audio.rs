//! Speaker output level via Core Audio peak meters (hand-written COM FFI, zero crates).
//!
//! Reads `IAudioMeterInformation::GetPeakValue` on every active playback device — the
//! same number the volume mixer's green bar shows. No audio is captured or stored; this
//! only answers "is something playing right now, and how loud?".
//!
//! Vtable slots checked against MinGW `mmdeviceapi.h`; `IAudioMeterInformation` follows
//! the Windows SDK `endpointvolume.h` (MinGW only forward-declares it).

use std::os::raw::c_void;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

type Hresult = i32;

#[repr(C)]
struct Guid {
    data1: u32,
    data2: u16,
    data3: u16,
    data4: [u8; 8],
}

#[link(name = "ole32")]
extern "system" {
    fn CoInitializeEx(reserved: *mut c_void, coinit: u32) -> Hresult;
    fn CoUninitialize();
    fn CoCreateInstance(
        clsid: *const Guid,
        outer: *mut c_void,
        clsctx: u32,
        iid: *const Guid,
        out: *mut *mut c_void,
    ) -> Hresult;
}

const COINIT_MULTITHREADED: u32 = 0x0;
const CLSCTX_ALL: u32 = 0x17;
const E_RENDER: i32 = 0;
const DEVICE_STATE_ACTIVE: u32 = 0x1;

/// How often the meter is read.
const SAMPLE_EVERY: Duration = Duration::from_millis(200);
/// Re-list playback devices this often (headset plugged in, Bluetooth connected).
const REFRESH_DEVICES_EVERY: Duration = Duration::from_secs(10);

// CLSID_MMDeviceEnumerator {bcde0395-e52f-467c-8e3d-c4579291692e}
const CLSID_MM_DEVICE_ENUMERATOR: Guid = Guid {
    data1: 0xbcde_0395,
    data2: 0xe52f,
    data3: 0x467c,
    data4: [0x8e, 0x3d, 0xc4, 0x57, 0x92, 0x91, 0x69, 0x2e],
};
// IID_IMMDeviceEnumerator {a95664d2-9614-4f35-a746-de8db63617e6}
const IID_IMM_DEVICE_ENUMERATOR: Guid = Guid {
    data1: 0xa956_64d2,
    data2: 0x9614,
    data3: 0x4f35,
    data4: [0xa7, 0x46, 0xde, 0x8d, 0xb6, 0x36, 0x17, 0xe6],
};
// IID_IAudioMeterInformation {c02216f6-8c67-4b5b-9d00-d008e73e0064}
const IID_IAUDIO_METER_INFORMATION: Guid = Guid {
    data1: 0xc022_16f6,
    data2: 0x8c67,
    data3: 0x4b5b,
    data4: [0x9d, 0x00, 0xd0, 0x08, 0xe7, 0x3e, 0x00, 0x64],
};

// Vtable slots (absolute, IUnknown = 0..2).
const SLOT_RELEASE: usize = 2;
const SLOT_ENUM_AUDIO_ENDPOINTS: usize = 3;
const SLOT_COLLECTION_GET_COUNT: usize = 3;
const SLOT_COLLECTION_ITEM: usize = 4;
const SLOT_DEVICE_ACTIVATE: usize = 3;
const SLOT_METER_GET_PEAK_VALUE: usize = 3;

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

/// Peak meters for every active playback device. COM objects: keep on one MTA thread.
struct OutputMeters {
    enumerator: *mut c_void,
    meters: Vec<*mut c_void>,
}

impl OutputMeters {
    fn new() -> Result<Self, String> {
        let mut enumerator: *mut c_void = ptr::null_mut();
        let hr = unsafe {
            CoCreateInstance(
                &CLSID_MM_DEVICE_ENUMERATOR,
                ptr::null_mut(),
                CLSCTX_ALL,
                &IID_IMM_DEVICE_ENUMERATOR,
                &mut enumerator,
            )
        };
        if hr < 0 || enumerator.is_null() {
            return Err(format!("audio device list unavailable (0x{:08x})", hr as u32));
        }
        let mut m = Self {
            enumerator,
            meters: Vec::new(),
        };
        m.refresh()?;
        Ok(m)
    }

    fn release_meters(&mut self) {
        for meter in self.meters.drain(..) {
            unsafe { release(meter) };
        }
    }

    /// Re-list active playback devices and open a peak meter on each.
    fn refresh(&mut self) -> Result<(), String> {
        self.release_meters();
        unsafe {
            let mut collection: *mut c_void = ptr::null_mut();
            let enum_endpoints: unsafe extern "system" fn(
                *mut c_void,
                i32,
                u32,
                *mut *mut c_void,
            ) -> Hresult = std::mem::transmute(vfn(self.enumerator, SLOT_ENUM_AUDIO_ENDPOINTS));
            let hr = enum_endpoints(self.enumerator, E_RENDER, DEVICE_STATE_ACTIVE, &mut collection);
            if hr < 0 || collection.is_null() {
                return Err(format!("could not list speakers (0x{:08x})", hr as u32));
            }
            let get_count: unsafe extern "system" fn(*mut c_void, *mut u32) -> Hresult =
                std::mem::transmute(vfn(collection, SLOT_COLLECTION_GET_COUNT));
            let item: unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> Hresult =
                std::mem::transmute(vfn(collection, SLOT_COLLECTION_ITEM));
            let mut count = 0u32;
            get_count(collection, &mut count);
            for i in 0..count {
                let mut device: *mut c_void = ptr::null_mut();
                if item(collection, i, &mut device) < 0 || device.is_null() {
                    continue;
                }
                let activate: unsafe extern "system" fn(
                    *mut c_void,
                    *const Guid,
                    u32,
                    *mut c_void,
                    *mut *mut c_void,
                ) -> Hresult = std::mem::transmute(vfn(device, SLOT_DEVICE_ACTIVATE));
                let mut meter: *mut c_void = ptr::null_mut();
                let hr = activate(
                    device,
                    &IID_IAUDIO_METER_INFORMATION,
                    CLSCTX_ALL,
                    ptr::null_mut(),
                    &mut meter,
                );
                if hr >= 0 && !meter.is_null() {
                    self.meters.push(meter);
                }
                release(device);
            }
            release(collection);
        }
        Ok(())
    }

    /// Loudest current peak (0.0–1.0) across playback devices. `None` if every read failed.
    fn peak(&self) -> Option<f32> {
        let mut best: Option<f32> = None;
        for &meter in &self.meters {
            let mut v = 0f32;
            let hr = unsafe {
                let get: unsafe extern "system" fn(*mut c_void, *mut f32) -> Hresult =
                    std::mem::transmute(vfn(meter, SLOT_METER_GET_PEAK_VALUE));
                get(meter, &mut v)
            };
            if hr >= 0 && v.is_finite() {
                best = Some(best.map_or(v, |b| b.max(v)));
            }
        }
        best
    }
}

impl Drop for OutputMeters {
    fn drop(&mut self) {
        self.release_meters();
        unsafe { release(self.enumerator) };
    }
}

/// Shared between the meter thread and the UI.
pub struct SoundLevel {
    /// Loudest peak since the UI last took it (f32 bits; non-negative floats order like u32).
    peak_bits: AtomicU32,
    /// `epoch`-relative ms of the last successful read (0 = never).
    ok_at_ms: AtomicU64,
    stop: AtomicBool,
    epoch: Instant,
}

impl SoundLevel {
    /// Loudest peak since the previous call (resets to 0).
    pub fn take_peak(&self) -> f32 {
        f32::from_bits(self.peak_bits.swap(0, Ordering::SeqCst))
    }

    /// True if the meter answered within `within` — otherwise silence is not trustworthy.
    pub fn healthy(&self, within: Duration) -> bool {
        let ok = self.ok_at_ms.load(Ordering::SeqCst);
        ok != 0 && (self.epoch.elapsed().as_millis() as u64).saturating_sub(ok) <= within.as_millis() as u64
    }

    /// Time since the meter was started (the first reading takes a moment).
    pub fn age(&self) -> Duration {
        self.epoch.elapsed()
    }

    /// Ask the meter thread to exit (it releases its COM objects itself).
    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

/// Start reading speaker levels in the background until `SoundLevel::stop`.
pub fn spawn_sound_meter() -> Arc<SoundLevel> {
    let level = Arc::new(SoundLevel {
        peak_bits: AtomicU32::new(0),
        ok_at_ms: AtomicU64::new(0),
        stop: AtomicBool::new(false),
        epoch: Instant::now(),
    });
    let shared = level.clone();
    thread::spawn(move || meter_loop(&shared));
    level
}

fn meter_loop(level: &SoundLevel) {
    let hr = unsafe { CoInitializeEx(ptr::null_mut(), COINIT_MULTITHREADED) };
    let mut meters: Option<OutputMeters> = None;
    let mut refreshed = Instant::now();
    let mut last_error = String::new();
    while !level.stop.load(Ordering::SeqCst) {
        if meters.is_none() {
            match OutputMeters::new() {
                Ok(m) => {
                    crate::debuglog::log(&format!("sound meter: {} playback device(s)", m.meters.len()));
                    meters = Some(m);
                    refreshed = Instant::now();
                }
                Err(e) => {
                    if e != last_error {
                        crate::debuglog::log(&format!("sound meter: {e}"));
                        last_error = e;
                    }
                }
            }
        } else if refreshed.elapsed() >= REFRESH_DEVICES_EVERY {
            refreshed = Instant::now();
            if let Some(m) = meters.as_mut() {
                if let Err(e) = m.refresh() {
                    crate::debuglog::log(&format!("sound meter: {e}"));
                    meters = None;
                }
            }
        }
        if let Some(peak) = meters.as_ref().and_then(|m| m.peak()) {
            level.peak_bits.fetch_max(peak.max(0.0).to_bits(), Ordering::SeqCst);
            let now = (level.epoch.elapsed().as_millis() as u64).max(1);
            level.ok_at_ms.store(now, Ordering::SeqCst);
        } else if meters.as_ref().is_some_and(|m| !m.meters.is_empty()) {
            // Every meter failed (device removed mid-read): re-list next round.
            meters = None;
        }
        thread::sleep(SAMPLE_EVERY);
    }
    drop(meters);
    if hr >= 0 {
        unsafe { CoUninitialize() };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Live check against real speakers: `cargo test reads_speaker_level -- --ignored`.
    #[test]
    #[ignore]
    fn reads_speaker_level() {
        let level = spawn_sound_meter();
        thread::sleep(Duration::from_secs(1));
        assert!(level.healthy(Duration::from_secs(1)), "meter answered");
        let mut idle = 0f32;
        for _ in 0..10 {
            thread::sleep(Duration::from_millis(200));
            idle = idle.max(level.take_peak());
        }
        println!("loudest peak before playing: {idle}");
        let wav = r"C:\Windows\Media\Alarm01.wav";
        let mut player = std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", &format!("(New-Object Media.SoundPlayer '{wav}').PlaySync()")])
            .spawn()
            .unwrap();
        let mut loudest = 0f32;
        for _ in 0..20 {
            thread::sleep(Duration::from_millis(200));
            loudest = loudest.max(level.take_peak());
        }
        let _ = player.wait();
        level.stop();
        println!("loudest peak while playing: {loudest}");
        assert!(loudest >= crate::auto_record::SOUND_THRESHOLD, "heard the sound");
    }
}
