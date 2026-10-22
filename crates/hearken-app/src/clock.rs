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
//!
//! # Trusted time
//!
//! A player can set their computer's clock to any date, which opens a
//! dated event early. A game that cares calls [`sync_in_background`] once
//! at start-up: it asks a public time server (SNTP over UDP, or a web
//! server's Date header where UDP is blocked) how far the computer's clock
//! is off, and [`trusted_unix_now`] and
//! [`trusted_local_now`] add that correction. Offline, or before the
//! answer arrives, they use the computer's clock and say so through
//! [`Source`]. What to do about a wrong clock is the game's call.

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::Once;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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

/// The public time servers [`sync_now`] asks, in order, until one answers.
pub const TIME_SERVERS: [&str; 3] = [
    "pool.ntp.org:123",
    "time.cloudflare.com:123",
    "time.google.com:123",
];

/// Web servers whose Date header [`sync_now`] reads when no time server
/// answers, such as on a network that blocks UDP port 123. Plain HTTP on
/// port 80: only the header is read, and it needs no TLS.
pub const WEB_SERVERS: [&str; 2] = ["www.google.com", "www.cloudflare.com"];

/// How long [`sync_now`] waits for each server.
pub const SERVER_TIMEOUT: Duration = Duration::from_secs(2);

/// Where a trusted time came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Source {
    /// The computer's clock, corrected by a time server's answer.
    Server,
    /// The computer's clock alone: no server has answered yet.
    Computer,
}

/// Seconds to add to the computer's clock to get the server's.
static SKEW: AtomicI64 = AtomicI64::new(0);
static SYNCED: AtomicBool = AtomicBool::new(false);
static STARTED: Once = Once::new();

/// Starts one background check of the computer's clock against the
/// [`TIME_SERVERS`]; later calls do nothing. Never blocks.
pub fn sync_in_background() {
    STARTED.call_once(|| {
        let spawned = std::thread::Builder::new()
            .name("hearken-clock-sync".into())
            .spawn(|| {
                if sync_now(SERVER_TIMEOUT).is_none() {
                    log::info!("clock: no time server answered; trusting the computer's clock");
                }
            });
        if let Err(e) = spawned {
            log::warn!("clock: could not start the time check: {e}");
        }
    });
}

/// Asks the [`TIME_SERVERS`] in turn, then the [`WEB_SERVERS`], blocking up to `timeout` for each,
/// and keeps the first answer for [`trusted_unix_now`]. Returns how many
/// seconds the computer's clock is behind (positive) or ahead (negative).
pub fn sync_now(timeout: Duration) -> Option<i64> {
    let skew = TIME_SERVERS
        .iter()
        .find_map(|server| query_server(server, timeout))
        .or_else(|| {
            WEB_SERVERS
                .iter()
                .find_map(|host| query_web_server(host, timeout))
        })?;
    SKEW.store(skew, Ordering::Relaxed);
    SYNCED.store(true, Ordering::Release);
    log::info!("clock: computer clock is {skew} s off the time server");
    Some(skew)
}

/// How far the computer's clock is behind the time server, in seconds,
/// once one has answered.
pub fn skew_seconds() -> Option<i64> {
    SYNCED
        .load(Ordering::Acquire)
        .then(|| SKEW.load(Ordering::Relaxed))
}

/// Seconds since 1970 UTC, corrected by the time server when it has
/// answered, and where the time came from.
pub fn trusted_unix_now() -> (i64, Source) {
    let secs = unix_now().unwrap_or(0);
    match skew_seconds() {
        Some(skew) => (secs + skew, Source::Server),
        None => (secs, Source::Computer),
    }
}

/// The player's local time on the trusted clock: the server's moment in
/// the computer's time zone (UTC when that can't be read).
pub fn trusted_local_now() -> (LocalTime, Source) {
    let (secs, source) = trusted_unix_now();
    let offset = utc_offset_minutes(secs).unwrap_or(0);
    (LocalTime::from_unix(secs, offset), source)
}

/// One SNTP exchange (RFC 4330): the server's clock minus ours, in
/// seconds, or `None` on any failure.
fn query_server(server: &str, timeout: Duration) -> Option<i64> {
    let socket = UdpSocket::bind(("0.0.0.0", 0)).ok()?;
    socket.set_read_timeout(Some(timeout)).ok()?;
    socket.set_write_timeout(Some(timeout)).ok()?;
    socket.connect(server).ok()?;
    let mut request = [0u8; 48];
    // Leap indicator 0, version 4, mode 3 (client).
    request[0] = 0b00_100_011;
    socket.send(&request).ok()?;
    let mut reply = [0u8; 48];
    let n = socket.recv(&mut reply).ok()?;
    let server_secs = parse_sntp_reply(&reply[..n])?;
    let ours = unix_now()?;
    Some(server_secs - ours)
}

/// One HTTP HEAD request: the web server's Date header minus our clock,
/// in seconds, or `None` on any failure.
fn query_web_server(host: &str, timeout: Duration) -> Option<i64> {
    let addr = (host, 80).to_socket_addrs().ok()?.next()?;
    let mut stream = TcpStream::connect_timeout(&addr, timeout).ok()?;
    stream.set_read_timeout(Some(timeout)).ok()?;
    stream.set_write_timeout(Some(timeout)).ok()?;
    let request = format!("HEAD / HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).ok()?;
    let mut head = Vec::new();
    let mut buf = [0u8; 1024];
    while head.len() < 16 * 1024 && !head.windows(4).any(|w| w == b"\r\n\r\n") {
        let n = stream.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        head.extend_from_slice(&buf[..n]);
    }
    let text = String::from_utf8_lossy(&head);
    let date = text.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.trim()
            .eq_ignore_ascii_case("date")
            .then(|| value.trim())
    })?;
    let server_secs = parse_http_date(date)?;
    Some(server_secs - unix_now()?)
}

/// Seconds since 1970 from an HTTP date, "Thu, 22 Oct 2026 17:49:16 GMT"
/// (RFC 9110's IMF-fixdate, the only form servers may send).
pub fn parse_http_date(text: &str) -> Option<i64> {
    let mut parts = text.split_whitespace();
    let _weekday = parts.next()?;
    let day: u32 = parts.next()?.parse().ok()?;
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let month_name = parts.next()?;
    let month = MONTHS.iter().position(|m| *m == month_name)? as u32 + 1;
    let year: i32 = parts.next()?.parse().ok()?;
    let mut hms = parts.next()?.splitn(3, ':');
    let hour: i64 = hms.next()?.parse().ok()?;
    let minute: i64 = hms.next()?.parse().ok()?;
    let second: i64 = hms.next()?.parse().ok()?;
    if parts.next()? != "GMT"
        || day == 0
        || day > days_in_month(year, month)
        || hour > 23
        || minute > 59
        || second > 60
    {
        return None;
    }
    Some(days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60 + second)
}

/// Seconds between 1900 (NTP's epoch) and 1970.
const NTP_TO_UNIX: i64 = 2_208_988_800;

/// The server's transmit time, as seconds since 1970, from an SNTP reply;
/// `None` for anything but a usable server answer.
pub fn parse_sntp_reply(reply: &[u8]) -> Option<i64> {
    if reply.len() < 48 {
        return None;
    }
    let leap = reply[0] >> 6;
    let mode = reply[0] & 0b111;
    let stratum = reply[1];
    // Mode 4 is a server's answer; stratum 0 is a "kiss of death"
    // (go away), and leap 3 means the server's own clock is not set.
    if mode != 4 || stratum == 0 || stratum > 15 || leap == 3 {
        return None;
    }
    let secs = u32::from_be_bytes([reply[40], reply[41], reply[42], reply[43]]);
    if secs == 0 {
        return None;
    }
    // NTP seconds wrap in February 2036: a value with the top bit clear
    // is in the next era (RFC 4330, section 3).
    let ntp = if secs & 0x8000_0000 == 0 {
        i64::from(secs) + (1 << 32)
    } else {
        i64::from(secs)
    };
    Some(ntp - NTP_TO_UNIX)
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

    fn reply(ntp_secs: u32) -> [u8; 48] {
        let mut r = [0u8; 48];
        r[0] = 0b00_100_100; // leap 0, version 4, mode 4
        r[1] = 2;
        r[40..44].copy_from_slice(&ntp_secs.to_be_bytes());
        r
    }

    #[test]
    fn sntp_reply_gives_unix_seconds() {
        // 2026-10-22 00:00 UTC.
        let unix = days_from_civil(2026, 10, 22) * 86_400;
        let ntp = (unix + NTP_TO_UNIX) as u32;
        assert_eq!(parse_sntp_reply(&reply(ntp)), Some(unix));
    }

    #[test]
    fn sntp_reply_rolls_over_in_2036() {
        // 2040-01-01 is past the 2036 wrap, so its NTP seconds are small.
        let unix = days_from_civil(2040, 1, 1) * 86_400;
        let ntp = (unix + NTP_TO_UNIX - (1 << 32)) as u32;
        assert_eq!(parse_sntp_reply(&reply(ntp)), Some(unix));
    }

    #[test]
    fn sntp_rejects_what_is_not_an_answer() {
        let good = reply(3_970_000_000);
        assert!(parse_sntp_reply(&good).is_some());
        assert!(parse_sntp_reply(&good[..40]).is_none());
        let mut kiss = good;
        kiss[1] = 0;
        assert!(parse_sntp_reply(&kiss).is_none());
        let mut client = good;
        client[0] = 0b00_100_011;
        assert!(parse_sntp_reply(&client).is_none());
        let mut unset = good;
        unset[0] |= 0b11 << 6;
        assert!(parse_sntp_reply(&unset).is_none());
        assert!(parse_sntp_reply(&reply(0)).is_none());
    }

    #[test]
    fn before_any_answer_the_computer_clock_is_used() {
        if skew_seconds().is_none() {
            let (secs, source) = trusted_unix_now();
            assert_eq!(source, Source::Computer);
            assert!((secs - unix_now().unwrap()).abs() <= 1);
        }
    }

    #[test]
    fn http_dates() {
        let t = parse_http_date("Thu, 22 Oct 2026 17:49:16 GMT").unwrap();
        let at = LocalTime::from_unix(t, 0);
        assert_eq!((at.year, at.month, at.day), (2026, 10, 22));
        assert_eq!((at.hour, at.minute), (17, 49));
        assert_eq!(t % 60, 16);
        assert!(parse_http_date("Thu, 22 Oct 2026 17:49:16 PST").is_none());
        assert!(parse_http_date("Sun, 30 Feb 2026 00:00:00 GMT").is_none());
        assert!(parse_http_date("Thursday, 22-Oct-26 17:49:16 GMT").is_none());
        assert!(parse_http_date("").is_none());
    }

    /// Asks the real servers; run with `--ignored` where UDP 123 is open.
    #[test]
    #[ignore]
    fn a_time_server_answers() {
        let skew = sync_now(SERVER_TIMEOUT).expect("no time server answered");
        assert!(skew.abs() < 86_400, "computer clock is {skew} s off");
        assert_eq!(trusted_unix_now().1, Source::Server);
    }
}
