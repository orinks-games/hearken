//! The player's local date and time, from the operating system's clock and
//! time zone.
//!
//! Hearken itself works in seconds since start-up. A game that cares what
//! time it is for the player, such as one that greets them with "Good
//! evening", asks [`local_now`]. Everything else here is plain arithmetic,
//! so tests pin a moment with [`LocalTime::from_unix`] or
//! [`LocalTime::parse`] instead of reading the clock.
//!
//! Every call fails open: when the time zone can't be read, [`local_now`]
//! returns `None` and [`now_or_utc`] falls back to UTC.

use std::time::{SystemTime, UNIX_EPOCH};

/// A moment on the player's wall clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LocalTime {
    pub year: i32,
    /// 1 to 12.
    pub month: u32,
    /// 1 to 31.
    pub day: u32,
    /// 0 to 23.
    pub hour: u32,
    /// 0 to 59.
    pub minute: u32,
    /// Minutes east of UTC: 60 for Central European Time, -300 for New
    /// York in winter.
    pub utc_offset_minutes: i32,
}

impl LocalTime {
    /// The wall-clock time at `unix_seconds` in a zone `offset_minutes`
    /// east of UTC.
    pub fn from_unix(unix_seconds: i64, offset_minutes: i32) -> LocalTime {
        let local = unix_seconds + i64::from(offset_minutes) * 60;
        let days = local.div_euclid(86_400);
        let secs = local.rem_euclid(86_400);
        let (year, month, day) = civil_from_days(days);
        LocalTime {
            year,
            month,
            day,
            hour: (secs / 3600) as u32,
            minute: (secs % 3600 / 60) as u32,
            utc_offset_minutes: offset_minutes,
        }
    }

    /// Reads "2026-10-31T21:30" (or with a space for the T) as a wall-clock
    /// time with no known offset, for tests and previews. `None` for any
    /// other shape or an impossible date or time.
    pub fn parse(text: &str) -> Option<LocalTime> {
        let text = text.trim();
        let (date, time) = text.split_once(['T', ' '])?;
        let mut d = date.splitn(3, '-');
        let year: i32 = d.next()?.parse().ok()?;
        let month: u32 = d.next()?.parse().ok()?;
        let day: u32 = d.next()?.parse().ok()?;
        let (h, m) = time.split_once(':')?;
        let hour: u32 = h.parse().ok()?;
        let minute: u32 = m.get(..2).unwrap_or(m).parse().ok()?;
        let valid = (1..=12).contains(&month)
            && day >= 1
            && day <= days_in_month(year, month)
            && hour < 24
            && minute < 60;
        valid.then_some(LocalTime {
            year,
            month,
            day,
            hour,
            minute,
            utc_offset_minutes: 0,
        })
    }

    /// Minutes since local midnight, 0 to 1439.
    pub fn minute_of_day(self) -> u32 {
        self.hour * 60 + self.minute
    }

    /// Monday 0 to Sunday 6.
    pub fn weekday(self) -> u32 {
        // 1970-01-01 was a Thursday.
        (days_from_civil(self.year, self.month, self.day) + 3).rem_euclid(7) as u32
    }
}

/// The player's local time now, or `None` when the time zone can't be
/// read.
pub fn local_now() -> Option<LocalTime> {
    let secs = unix_now()?;
    let offset = utc_offset_minutes(secs)?;
    Some(LocalTime::from_unix(secs, offset))
}

/// The local time now, or UTC (offset 0) when the time zone can't be read.
pub fn now_or_utc() -> LocalTime {
    local_now().unwrap_or_else(|| LocalTime::from_unix(unix_now().unwrap_or(0), 0))
}

fn unix_now() -> Option<i64> {
    let d = SystemTime::now().duration_since(UNIX_EPOCH).ok()?;
    i64::try_from(d.as_secs()).ok()
}

/// How far the local zone is east of UTC at `unix_seconds`, in minutes,
/// daylight saving included.
#[cfg(unix)]
fn utc_offset_minutes(unix_seconds: i64) -> Option<i32> {
    let t = libc::time_t::try_from(unix_seconds).ok()?;
    // SAFETY: `tm` is plain old data, so all zeros is a valid value, and
    // localtime_r writes only into the struct it is handed.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    let out = unsafe { libc::localtime_r(&t, &mut tm) };
    if out.is_null() {
        return None;
    }
    i32::try_from(tm.tm_gmtoff / 60).ok()
}

/// How far the local zone is east of UTC now, in minutes: the local
/// wall clock minus the UTC one, read back to back.
#[cfg(windows)]
fn utc_offset_minutes(_unix_seconds: i64) -> Option<i32> {
    use windows_sys::Win32::Foundation::SYSTEMTIME;
    use windows_sys::Win32::System::SystemInformation::{GetLocalTime, GetSystemTime};
    // SAFETY: SYSTEMTIME is plain old data, and each call fills the
    // struct it is handed.
    let (local, utc) = unsafe {
        let mut local: SYSTEMTIME = std::mem::zeroed();
        let mut utc: SYSTEMTIME = std::mem::zeroed();
        GetSystemTime(&mut utc);
        GetLocalTime(&mut local);
        (local, utc)
    };
    let minutes = |t: &SYSTEMTIME| {
        days_from_civil(i32::from(t.wYear), u32::from(t.wMonth), u32::from(t.wDay)) * 1440
            + i64::from(t.wHour) * 60
            + i64::from(t.wMinute)
    };
    let offset = minutes(&local) - minutes(&utc);
    // Real zones run from UTC-12 to UTC+14.
    (-14 * 60..=14 * 60)
        .contains(&offset)
        .then_some(offset as i32)
}

#[cfg(not(any(unix, windows)))]
fn utc_offset_minutes(_unix_seconds: i64) -> Option<i32> {
    None
}

/// (year, month, day) of the day `days` after 1970-01-01 (Howard
/// Hinnant's civil_from_days).
fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    ((if m <= 2 { y + 1 } else { y }) as i32, m, d)
}

/// Days from 1970-01-01 to the date (Howard Hinnant's days_from_civil).
fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
    let y = i64::from(year) - i64::from(month <= 2);
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let m = i64::from(month);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        2 if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_unix_applies_the_offset() {
        // 2026-10-31 21:30 UTC.
        let secs = days_from_civil(2026, 10, 31) * 86_400 + 21 * 3600 + 30 * 60;
        let utc = LocalTime::from_unix(secs, 0);
        assert_eq!((utc.year, utc.month, utc.day), (2026, 10, 31));
        assert_eq!((utc.hour, utc.minute), (21, 30));
        let tokyo = LocalTime::from_unix(secs, 9 * 60);
        assert_eq!((tokyo.month, tokyo.day, tokyo.hour), (11, 1, 6));
        let la = LocalTime::from_unix(secs, -7 * 60);
        assert_eq!((la.day, la.hour, la.minute), (31, 14, 30));
        let kolkata = LocalTime::from_unix(secs, 330);
        assert_eq!(
            (kolkata.month, kolkata.day, kolkata.hour, kolkata.minute),
            (11, 1, 3, 0)
        );
    }

    #[test]
    fn days_round_trip() {
        for days in [-1_000_000, -1, 0, 1, 20_000, 20_757, 60_000] {
            let (y, m, d) = civil_from_days(days);
            assert_eq!(days_from_civil(y, m, d), days);
        }
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(civil_from_days(0), (1970, 1, 1));
    }

    #[test]
    fn parse_reads_previews() {
        let t = LocalTime::parse("2026-10-31T21:30").unwrap();
        assert_eq!(
            (t.year, t.month, t.day, t.hour, t.minute),
            (2026, 10, 31, 21, 30)
        );
        assert_eq!(
            LocalTime::parse("2026-10-31 07:05")
                .unwrap()
                .minute_of_day(),
            425
        );
        assert!(LocalTime::parse("2026-02-29T10:00").is_none());
        assert!(LocalTime::parse("2028-02-29T10:00").is_some());
        assert!(LocalTime::parse("2026-10-31T24:00").is_none());
        assert!(LocalTime::parse("2026-10-31").is_none());
        assert!(LocalTime::parse("tonight").is_none());
    }

    #[test]
    fn weekday_counts_from_monday() {
        // 22 October 2026 is a Thursday.
        assert_eq!(LocalTime::parse("2026-10-22T09:00").unwrap().weekday(), 3);
        assert_eq!(LocalTime::parse("2026-10-25T09:00").unwrap().weekday(), 6);
    }

    #[test]
    fn the_real_clock_reads_something_sane() {
        let now = now_or_utc();
        assert!(now.year >= 2024);
        assert!(now.hour < 24 && now.minute < 60);
        if let Some(local) = local_now() {
            assert!((-14 * 60..=14 * 60).contains(&local.utc_offset_minutes));
        }
    }
}
