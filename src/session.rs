//! Session identifiers and wall-clock naming.

use std::time::{SystemTime, UNIX_EPOCH};

/// Format local-ish wall clock for filenames: `YYYY-MM-DD_HH-MM-SS`.
/// Uses local offset via `chrono`-free approach: libc localtime when available,
/// else UTC from SystemTime.
pub fn format_session_stamp(now: SystemTime) -> String {
    let secs = now
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let (y, mo, d, h, mi, s) = civil_from_unix_local(secs);
    format!("{y:04}-{mo:02}-{d:02}_{h:02}-{mi:02}-{s:02}")
}

/// Build a unique session filename stem, adding `_2`, `_3`, … if the path exists.
pub fn unique_session_stem(folder: &std::path::Path, stamp: &str) -> String {
    let base = stamp.to_string();
    let candidate = folder.join(format!("{base}.txt"));
    if !candidate.exists() {
        return base;
    }
    for n in 2..1000 {
        let stem = format!("{base}_{n}");
        if !folder.join(format!("{stem}.txt")).exists() {
            return stem;
        }
    }
    format!("{base}_{}", std::process::id())
}

/// Convert Unix seconds to civil date/time in **local** timezone when possible.
fn civil_from_unix_local(secs: i64) -> (i32, u32, u32, u32, u32, u32) {
    #[cfg(unix)]
    {
        // SAFETY: localtime_r is used with our own tm buffer.
        unsafe {
            let t = secs as LibcTimeT;
            let mut tm = std::mem::MaybeUninit::<LibcTm>::uninit();
            if localtime_r(&t as *const _, tm.as_mut_ptr()).is_null() {
                return civil_from_unix_utc(secs);
            }
            let tm = tm.assume_init();
            return (
                tm.tm_year + 1900,
                (tm.tm_mon + 1) as u32,
                tm.tm_mday as u32,
                tm.tm_hour as u32,
                tm.tm_min as u32,
                tm.tm_sec as u32,
            );
        }
    }
    #[cfg(not(unix))]
    {
        civil_from_unix_utc(secs)
    }
}

fn civil_from_unix_utc(mut secs: i64) -> (i32, u32, u32, u32, u32, u32) {
    // Algorithm from civil_from_days (Howard Hinnant) + time of day.
    if secs < 0 {
        secs = 0;
    }
    let s_day = 86_400i64;
    let days = secs / s_day;
    let rem = secs % s_day;
    let h = (rem / 3600) as u32;
    let mi = ((rem % 3600) / 60) as u32;
    let s = (rem % 60) as u32;

    // days since 1970-01-01 → civil
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m as u32, d as u32, h, mi, s)
}

#[cfg(unix)]
type LibcTimeT = i64;

#[cfg(unix)]
#[repr(C)]
struct LibcTm {
    tm_sec: i32,
    tm_min: i32,
    tm_hour: i32,
    tm_mday: i32,
    tm_mon: i32,
    tm_year: i32,
    tm_wday: i32,
    tm_yday: i32,
    tm_isdst: i32,
    tm_gmtoff: i64,
    tm_zone: *const i8,
}

#[cfg(unix)]
extern "C" {
    fn localtime_r(timep: *const LibcTimeT, result: *mut LibcTm) -> *mut LibcTm;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::Duration;

    #[test]
    fn stamp_shape_is_sortable_datetime() {
        let t = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let s = format_session_stamp(t);
        // YYYY-MM-DD_HH-MM-SS
        assert_eq!(s.len(), 19, "stamp={s}");
        assert_eq!(&s[4..5], "-");
        assert_eq!(&s[7..8], "-");
        assert_eq!(&s[10..11], "_");
        assert_eq!(&s[13..14], "-");
        assert_eq!(&s[16..17], "-");
        // all digits except separators
        for (i, c) in s.chars().enumerate() {
            if matches!(i, 4 | 7 | 10 | 13 | 16) {
                continue;
            }
            assert!(c.is_ascii_digit(), "bad char at {i} in {s}");
        }
    }

    #[test]
    fn unique_stem_avoids_collision() {
        let dir = std::env::temp_dir().join(format!(
            "interpres-sess-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let stamp = "2026-08-05_14-22-01";
        fs::write(dir.join(format!("{stamp}.txt")), "x").unwrap();
        let stem = unique_session_stem(&dir, stamp);
        assert_eq!(stem, "2026-08-05_14-22-01_2");
        let _ = fs::remove_dir_all(dir);
    }
}
