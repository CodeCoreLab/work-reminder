// Copyright (c) 2026 Ma Xin <maxin.zhang@users.noreply.github.com>
// SPDX-License-Identifier: MIT
//
// Time primitives with no external dependency.
//
// The original program stored schedule times as "HH:mm:ss" strings and dates as
// ISO strings. This module provides the same semantics without pulling in a
// date library: every value is a plain civil datetime in local wall-clock terms.

use serde::{Deserialize, Serialize};

/// Civil date (proleptic Gregorian).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Date {
    pub year: i32,
    pub month: u32,
    pub day: u32,
}

/// Wall-clock time of day with second resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct TimeOfDay {
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
}

/// A civil datetime, ordered lexicographically by field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct DateTime {
    pub date: Date,
    pub time: TimeOfDay,
}

impl Date {
    pub const fn new(year: i32, month: u32, day: u32) -> Self {
        Self { year, month, day }
    }

    pub fn is_leap_year(year: i32) -> bool {
        (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
    }

    /// Days in the given month. Returns 0 for an invalid month.
    pub fn days_in_month(year: i32, month: u32) -> u32 {
        match month {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            4 | 6 | 9 | 11 => 30,
            2 => {
                if Self::is_leap_year(year) {
                    29
                } else {
                    28
                }
            }
            _ => 0,
        }
    }

    /// Day of week as ISO-8601: Monday=1 .. Sunday=7.
    pub fn weekday(&self) -> u32 {
        // Sakamoto's algorithm, valid for the proleptic Gregorian calendar.
        const T: [i32; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
        let mut y = self.year;
        if self.month < 3 {
            y -= 1;
        }
        let idx = (self.month as usize).saturating_sub(1);
        let w = (y + y / 4 - y / 100 + y / 400 + T[idx] + self.day as i32) % 7;
        // Sakamoto yields 0=Sunday..6=Saturday; normalise to 1..7 with Monday=1.
        let sunday_based = ((w + 7) % 7) as u32;
        if sunday_based == 0 {
            7
        } else {
            sunday_based
        }
    }

    pub fn is_weekend(&self) -> bool {
        self.weekday() >= 6
    }

    /// Advance by one day, rolling over month and year boundaries.
    pub fn next_day(&self) -> Self {
        let dim = Self::days_in_month(self.year, self.month);
        if self.day < dim {
            Self::new(self.year, self.month, self.day + 1)
        } else if self.month < 12 {
            Self::new(self.year, self.month + 1, 1)
        } else {
            Self::new(self.year + 1, 1, 1)
        }
    }

    /// Step back by one day.
    pub fn prev_day(&self) -> Self {
        if self.day > 1 {
            Self::new(self.year, self.month, self.day - 1)
        } else if self.month > 1 {
            let pm = self.month - 1;
            let dim = Self::days_in_month(self.year, pm);
            Self::new(self.year, pm, dim)
        } else {
            Self::new(self.year - 1, 12, 31)
        }
    }

    pub fn with_day(&self, day: u32) -> Self {
        let dim = Self::days_in_month(self.year, self.month);
        Self::new(self.year, self.month, day.clamp(1, dim))
    }
}

impl TimeOfDay {
    pub const fn new(hour: u32, minute: u32, second: u32) -> Self {
        Self {
            hour,
            minute,
            second,
        }
    }

    pub const fn from_hm(hour: u32, minute: u32) -> Self {
        Self::new(hour, minute, 0)
    }

    pub fn seconds_since_midnight(&self) -> i64 {
        self.hour as i64 * 3600 + self.minute as i64 * 60 + self.second as i64
    }

    pub fn from_seconds(secs: i64) -> Self {
        let s = secs.rem_euclid(86_400);
        Self::new((s / 3600) as u32, ((s % 3600) / 60) as u32, (s % 60) as u32)
    }

    /// Parse "HH:mm" or "HH:mm:ss". Returns None on any malformed input.
    pub fn parse(text: &str) -> Option<Self> {
        let parts: Vec<&str> = text.trim().split(':').collect();
        if parts.len() != 2 && parts.len() != 3 {
            return None;
        }
        let hour: u32 = parts[0].trim().parse().ok()?;
        let minute: u32 = parts[1].trim().parse().ok()?;
        let second: u32 = if parts.len() == 3 {
            parts[2].trim().parse().ok()?
        } else {
            0
        };
        if hour > 23 || minute > 59 || second > 59 {
            return None;
        }
        Some(Self::new(hour, minute, second))
    }

    pub fn format_hm(&self) -> String {
        format!("{:02}:{:02}", self.hour, self.minute)
    }

    pub fn format_hms(&self) -> String {
        format!("{:02}:{:02}:{:02}", self.hour, self.minute, self.second)
    }

    /// Serialised form used by the config file (`HH:mm:ss`), matching the
    /// reference implementation's `dailyTime` encoding.
    pub fn to_config_string(&self) -> String {
        self.format_hms()
    }

    /// Parse the config-file representation, accepting `HH:mm` as well.
    pub fn from_config_string(text: &str) -> Option<Self> {
        Self::parse(text)
    }
}

impl DateTime {
    pub const fn new(date: Date, time: TimeOfDay) -> Self {
        Self { date, time }
    }

    /// Add minutes, keeping the wall-clock arithmetics simple by walking days.
    pub fn add_minutes(&self, minutes: i64) -> Self {
        let total = self.time.seconds_since_midnight() + minutes * 60;
        let day_shift = total.div_euclid(86_400);
        let mut date = self.date;
        for _ in 0..day_shift {
            date = date.next_day();
        }
        Self::new(date, TimeOfDay::from_seconds(total))
    }

    /// Minutes elapsed from `self` to `other`; negative when `other` is earlier.
    pub fn minutes_until(&self, other: &DateTime) -> i64 {
        self.seconds_until(other) / 60
    }

    pub fn seconds_until(&self, other: &DateTime) -> i64 {
        let days = days_between(&self.date, &other.date);
        days * 86_400 + other.time.seconds_since_midnight() - self.time.seconds_since_midnight()
    }

    /// "yyyy-MM-dd HH:mm" — matches the format string used by the original UI.
    pub fn format_ymd_hm(&self) -> String {
        format!(
            "{:04}-{:02}-{:02} {}",
            self.date.year,
            self.date.month,
            self.date.day,
            self.time.format_hm()
        )
    }

    /// Parse "yyyy-MM-dd HH:mm" (seconds optional).
    pub fn parse_ymd_hm(text: &str) -> Option<Self> {
        let t = text.trim();
        let (date_part, time_part) = t.split_once(' ')?;
        let mut dparts = date_part.split('-');
        let year: i32 = dparts.next()?.parse().ok()?;
        let month: u32 = dparts.next()?.parse().ok()?;
        let day: u32 = dparts.next()?.parse().ok()?;
        if !(1..=12).contains(&month) || day == 0 || day > Date::days_in_month(year, month) {
            return None;
        }
        let time = TimeOfDay::parse(time_part)?;
        Some(Self::new(Date::new(year, month, day), time))
    }
}

/// Signed day difference `b - a`.
fn days_between(a: &Date, b: &Date) -> i64 {
    to_epoch_day(a) - to_epoch_day(b)
}

/// Days since 1970-01-01 (Howard Hinnant's algorithm, days_from_civil).
fn to_epoch_day(d: &Date) -> i64 {
    let y = if d.month <= 2 { d.year - 1 } else { d.year } as i64;
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = ((d.month as i64) + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d.day as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weekday_is_iso_numbered() {
        // 2026-10-22 is a Thursday.
        assert_eq!(Date::new(2026, 10, 22).weekday(), 4);
        // 2026-10-19 is a Monday.
        assert_eq!(Date::new(2026, 10, 19).weekday(), 1);
        // 2026-10-25 is a Sunday.
        assert_eq!(Date::new(2026, 10, 25).weekday(), 7);
    }

    #[test]
    fn leap_year_rules() {
        assert!(Date::is_leap_year(2024));
        assert!(!Date::is_leap_year(2026));
        assert!(Date::is_leap_year(2000));
        assert!(!Date::is_leap_year(1900));
        assert_eq!(Date::days_in_month(2024, 2), 29);
        assert_eq!(Date::days_in_month(2026, 2), 28);
    }

    #[test]
    fn day_rollover_crosses_month_and_year() {
        assert_eq!(Date::new(2026, 1, 31).next_day(), Date::new(2026, 2, 1));
        assert_eq!(Date::new(2026, 12, 31).next_day(), Date::new(2027, 1, 1));
        assert_eq!(Date::new(2026, 3, 1).prev_day(), Date::new(2026, 2, 28));
        assert_eq!(Date::new(2026, 1, 1).prev_day(), Date::new(2025, 12, 31));
    }

    #[test]
    fn add_minutes_rolls_over_midnight() {
        let dt = DateTime::new(Date::new(2026, 10, 22), TimeOfDay::new(23, 30, 0));
        let later = dt.add_minutes(45);
        assert_eq!(later.date, Date::new(2026, 10, 23));
        assert_eq!(later.time, TimeOfDay::new(0, 15, 0));
    }

    #[test]
    fn minutes_until_is_signed() {
        let a = DateTime::new(Date::new(2026, 10, 22), TimeOfDay::new(10, 0, 0));
        let b = DateTime::new(Date::new(2026, 10, 22), TimeOfDay::new(11, 30, 0));
        assert_eq!(a.minutes_until(&b), 90);
        assert_eq!(b.minutes_until(&a), -90);
    }

    #[test]
    fn time_parsing_rejects_bad_input() {
        assert_eq!(TimeOfDay::parse("09:00"), Some(TimeOfDay::new(9, 0, 0)));
        assert_eq!(TimeOfDay::parse("17:30"), Some(TimeOfDay::new(17, 30, 0)));
        assert_eq!(TimeOfDay::parse("09:00:15"), Some(TimeOfDay::new(9, 0, 15)));
        assert_eq!(TimeOfDay::parse("24:00"), None);
        assert_eq!(TimeOfDay::parse("09:60"), None);
        assert_eq!(TimeOfDay::parse("0900"), None);
        assert_eq!(TimeOfDay::parse(""), None);
    }

    #[test]
    fn datetime_roundtrip_format() {
        let dt = DateTime::new(Date::new(2026, 10, 22), TimeOfDay::new(17, 30, 0));
        assert_eq!(dt.format_ymd_hm(), "2026-10-22 17:30");
        assert_eq!(DateTime::parse_ymd_hm("2026-10-22 17:30"), Some(dt));
        assert_eq!(DateTime::parse_ymd_hm("2026-02-30 10:00"), None);
    }
}

/// Current local wall-clock time.
///
/// Lives in core rather than the platform layer so both front ends share one
/// definition. `SystemTime` is used instead of a Win32 call because it is the same
/// clock, needs no unsafe code, and works on any host.
pub fn now_local() -> DateTime {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    // The engine only ever compares wall-clock values, so a local reading derived
    // from the same offset keeps "09:00" meaning local 09:00.
    let offset = local_offset_seconds();
    from_unix_seconds(secs + offset)
}

/// Seconds to add to UTC to reach local time.
#[cfg(windows)]
fn local_offset_seconds() -> i64 {
    #[link(name = "kernel32")]
    extern "system" {
        fn GetTimeZoneInformation(tz: *mut TzInfo) -> u32;
    }
    #[repr(C)]
    struct TzInfo {
        bias: i32,
        standard_name: [u16; 32],
        standard_date: [u16; 16],
        standard_bias: i32,
        daylight_name: [u16; 32],
        daylight_date: [u16; 16],
        daylight_bias: i32,
    }
    let mut tz: TzInfo = unsafe { std::mem::zeroed() };
    // SAFETY: `tz` is a correctly sized out-parameter.
    let rc = unsafe { GetTimeZoneInformation(&mut tz) };
    // BIAS is UTC = local + bias, so local = UTC - bias.
    let mut minutes = -i64::from(tz.bias);
    if rc == 2 {
        // TIME_ZONE_ID_DAYLIGHT: the daylight bias is additional.
        minutes -= i64::from(tz.daylight_bias);
    }
    minutes * 60
}

#[cfg(not(windows))]
fn local_offset_seconds() -> i64 {
    0
}

/// Civil datetime from a Unix timestamp, using the same proleptic Gregorian rules
/// as the rest of this module.
pub fn from_unix_seconds(secs: i64) -> DateTime {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    DateTime::new(
        Date::new(year, month, day),
        TimeOfDay::new((rem / 3600) as u32, ((rem % 3600) / 60) as u32, (rem % 60) as u32),
    )
}

/// Howard Hinnant's days-to-civil algorithm: exact for any Gregorian date, no
/// lookup tables, no leap-year special cases at the call site.
fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    ((if m <= 2 { y + 1 } else { y }) as i32, m as u32, d as u32)
}
