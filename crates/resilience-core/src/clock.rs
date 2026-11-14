//! Setting the system clock by hand (no NTP on an offline system).
//!
//! Pure rules shared by the TUI and the privileged helper (ADR 0009): the
//! TUI reads a local date and time typed by the user and converts it to UTC;
//! the helper accepts only one strict UTC form and checks the same bounds,
//! whatever the TUI did.

use std::fmt;

/// The earliest time the clock may be set to: 2025-01-01T00:00:00Z. A clock
/// before this is certainly wrong (an empty RTC battery typically resets to
/// 1970 or 2000), and setting it back that far would disturb logs,
/// lockouts and later signature checks.
pub const MIN_TIME: i64 = 1_735_689_600;
/// The latest accepted time: 2100-01-01T00:00:00Z.
pub const MAX_TIME: i64 = 4_102_444_800;

/// A calendar date and time of day, without a time zone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Civil {
    pub year: i64,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TimeRefusal {
    /// Not in the expected format; the string is the expected format.
    Format(&'static str),
    /// A field is out of range (e.g. month 13, 31 April).
    NoSuchDate,
    TooEarly,
    TooLate,
}

impl fmt::Display for TimeRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TimeRefusal::Format(want) => write!(f, "write the date and time as {want}"),
            TimeRefusal::NoSuchDate => f.write_str("that date or time does not exist"),
            TimeRefusal::TooEarly => f.write_str("the date must be in 2025 or later"),
            TimeRefusal::TooLate => f.write_str("the date must be before the year 2100"),
        }
    }
}

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

fn days_in_month(y: i64, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap(y) => 29,
        2 => 28,
        _ => 0,
    }
}

/// Days since 1970-01-01 (Howard Hinnant's days-from-civil algorithm).
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let (m, d) = (i64::from(m), i64::from(d));
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

impl Civil {
    fn checked(self) -> Result<Civil, TimeRefusal> {
        let ok = (1..=12).contains(&self.month)
            && self.day >= 1
            && self.day <= days_in_month(self.year, self.month)
            && self.hour < 24
            && self.minute < 60
            && self.second < 60;
        if ok {
            Ok(self)
        } else {
            Err(TimeRefusal::NoSuchDate)
        }
    }

    /// Seconds since the epoch if this civil time is read as UTC.
    pub fn as_utc(self) -> i64 {
        days_from_civil(self.year, self.month, self.day) * 86_400
            + i64::from(self.hour) * 3600
            + i64::from(self.minute) * 60
            + i64::from(self.second)
    }
}

/// Fixed-width ASCII digits at `s[at..at+n]`.
fn digits(s: &[u8], at: usize, n: usize) -> Option<u32> {
    let part = s.get(at..at + n)?;
    if !part.iter().all(u8::is_ascii_digit) {
        return None;
    }
    std::str::from_utf8(part).ok()?.parse().ok()
}

/// What the TUI accepts: `YYYY-MM-DD HH:MM` or `YYYY-MM-DD HH:MM:SS`, with
/// surrounding spaces ignored. The result is a local time.
pub const LOCAL_FORMAT: &str = "YYYY-MM-DD HH:MM, for example 2026-11-14 09:30";

pub fn parse_local(input: &str) -> Result<Civil, TimeRefusal> {
    let s = input.trim().as_bytes();
    let bad = TimeRefusal::Format(LOCAL_FORMAT);
    let seconds = match s.len() {
        16 => false,
        19 => true,
        _ => return Err(bad),
    };
    let sep = |i: usize, c: u8| s[i] == c;
    if !(sep(4, b'-') && sep(7, b'-') && sep(10, b' ') && sep(13, b':'))
        || (seconds && !sep(16, b':'))
    {
        return Err(bad);
    }
    let f = |at, n| digits(s, at, n).ok_or(TimeRefusal::Format(LOCAL_FORMAT));
    Civil {
        year: i64::from(f(0, 4)?),
        month: f(5, 2)?,
        day: f(8, 2)?,
        hour: f(11, 2)?,
        minute: f(14, 2)?,
        second: if seconds { f(17, 2)? } else { 0 },
    }
    .checked()
}

/// What the helper accepts: exactly `YYYY-MM-DDTHH:MM:SSZ` (UTC), within
/// [`MIN_TIME`, `MAX_TIME`). Returns seconds since the epoch.
pub const UTC_FORMAT: &str = "YYYY-MM-DDTHH:MM:SSZ (UTC)";

pub fn parse_utc(s: &str) -> Result<i64, TimeRefusal> {
    let b = s.as_bytes();
    let bad = TimeRefusal::Format(UTC_FORMAT);
    if b.len() != 20
        || b[4] != b'-'
        || b[7] != b'-'
        || b[10] != b'T'
        || b[13] != b':'
        || b[16] != b':'
        || b[19] != b'Z'
    {
        return Err(bad);
    }
    let f = |at, n| digits(b, at, n).ok_or(TimeRefusal::Format(UTC_FORMAT));
    let c = Civil {
        year: i64::from(f(0, 4)?),
        month: f(5, 2)?,
        day: f(8, 2)?,
        hour: f(11, 2)?,
        minute: f(14, 2)?,
        second: f(17, 2)?,
    }
    .checked()?;
    check_bounds(c.as_utc())
}

pub fn check_bounds(t: i64) -> Result<i64, TimeRefusal> {
    if t < MIN_TIME {
        Err(TimeRefusal::TooEarly)
    } else if t >= MAX_TIME {
        Err(TimeRefusal::TooLate)
    } else {
        Ok(t)
    }
}

/// Convert a local civil time to seconds since the epoch with the C
/// library's time-zone rules (`mktime`, honouring TZ and /etc/localtime).
/// `None` if the C library cannot represent it. In a daylight-saving gap or
/// overlap, mktime picks one of the possible instants.
pub fn local_to_unix(c: Civil) -> Option<i64> {
    // SAFETY: mktime reads and normalises the zero-initialised struct we own;
    // tzset is implied by mktime.
    let t = unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        tm.tm_year = i32::try_from(c.year - 1900).ok()?;
        tm.tm_mon = c.month as i32 - 1;
        tm.tm_mday = c.day as i32;
        tm.tm_hour = c.hour as i32;
        tm.tm_min = c.minute as i32;
        tm.tm_sec = c.second as i32;
        tm.tm_isdst = -1;
        libc::mktime(&mut tm)
    };
    (t != -1).then_some(t as i64)
}

/// Name of the system time zone from the target of the /etc/localtime
/// link (`.../zoneinfo/Europe/Paris` → `Europe/Paris`).
pub fn zone_from_link(target: &str) -> Option<String> {
    let (_, name) = target.rsplit_once("zoneinfo/")?;
    (!name.is_empty()).then(|| name.to_string())
}

pub fn system_zone() -> String {
    std::fs::read_link("/etc/localtime")
        .ok()
        .and_then(|p| zone_from_link(&p.to_string_lossy()))
        .unwrap_or_else(|| "UTC".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::rfc3339_utc;

    #[test]
    fn bounds_constants_are_what_they_say() {
        assert_eq!(rfc3339_utc(MIN_TIME), "2025-01-01T00:00:00Z");
        assert_eq!(rfc3339_utc(MAX_TIME), "2100-01-01T00:00:00Z");
    }

    #[test]
    fn civil_round_trips_with_rfc3339() {
        for t in [0, 951_782_400, MIN_TIME, 1_794_652_200, MAX_TIME - 1] {
            let s = rfc3339_utc(t);
            if t >= MIN_TIME {
                assert_eq!(parse_utc(&s), Ok(t), "{s}");
            }
            let c = parse_local(&s[..19].replace('T', " ")).expect("valid");
            assert_eq!(c.as_utc(), t, "{s}");
        }
    }

    #[test]
    fn local_input_is_strict_but_forgiving_about_spaces() {
        let c = parse_local("  2026-11-14 09:30 ").expect("valid");
        assert_eq!(
            (c.year, c.month, c.day, c.hour, c.minute, c.second),
            (2026, 11, 14, 9, 30, 0)
        );
        assert_eq!(parse_local("2026-11-14 09:30:59").map(|c| c.second), Ok(59));
        for bad in [
            "",
            "2026-11-14",
            "2026-11-14 9:30",
            "2026/11/14 09:30",
            "2026-11-14T09:30",
            "2026-11-14 09:30:5",
            "+026-11-14 09:30",
            "2026-11-14 09:3x",
            "２026-11-14 09:30",
        ] {
            assert!(
                matches!(parse_local(bad), Err(TimeRefusal::Format(_))),
                "{bad:?}"
            );
        }
        for impossible in [
            "2026-02-29 10:00",
            "2026-04-31 10:00",
            "2026-13-01 10:00",
            "2026-00-10 10:00",
            "2026-11-00 10:00",
            "2026-11-14 24:00",
            "2026-11-14 23:60",
            "2026-11-14 23:59:60",
        ] {
            assert_eq!(
                parse_local(impossible),
                Err(TimeRefusal::NoSuchDate),
                "{impossible}"
            );
        }
        assert!(parse_local("2028-02-29 10:00").is_ok());
        assert!(parse_local("2000-02-29 10:00").is_ok());
        assert!(parse_local("2100-02-29 10:00").is_err());
    }

    #[test]
    fn helper_accepts_only_strict_utc_within_bounds() {
        assert_eq!(parse_utc("2026-11-14T10:30:00Z"), Ok(1_794_652_200));
        for bad in [
            "2026-11-14T10:30:00",
            "2026-11-14 10:30:00Z",
            "2026-11-14T10:30:00+00:00",
            "2026-11-14T10:30Z",
            "@1794652200",
            "1794652200",
            " 2026-11-14T10:30:00Z",
            "2026-11-14T10:30:00Z\n",
            "now",
            "-2026-11-14T10:30:00Z",
        ] {
            assert!(
                matches!(parse_utc(bad), Err(TimeRefusal::Format(_))),
                "{bad:?}"
            );
        }
        assert_eq!(
            parse_utc("2026-02-30T10:30:00Z"),
            Err(TimeRefusal::NoSuchDate)
        );
        assert_eq!(
            parse_utc("2024-12-31T23:59:59Z"),
            Err(TimeRefusal::TooEarly)
        );
        assert_eq!(
            parse_utc("1970-01-01T00:00:00Z"),
            Err(TimeRefusal::TooEarly)
        );
        assert_eq!(parse_utc("2100-01-01T00:00:00Z"), Err(TimeRefusal::TooLate));
        assert_eq!(parse_utc("2099-12-31T23:59:59Z"), Ok(MAX_TIME - 1));
    }

    #[test]
    fn zone_names() {
        assert_eq!(
            zone_from_link("/etc/zoneinfo/Europe/Paris").as_deref(),
            Some("Europe/Paris")
        );
        assert_eq!(
            zone_from_link("/nix/store/abc-tzdata/share/zoneinfo/UTC").as_deref(),
            Some("UTC")
        );
        assert_eq!(zone_from_link("/etc/zoneinfo/"), None);
        assert_eq!(zone_from_link("/somewhere/else"), None);
    }
}
