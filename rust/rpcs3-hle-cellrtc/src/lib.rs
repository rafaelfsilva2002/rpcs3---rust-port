//! `rpcs3-hle-cellrtc` — PS3 RTC (Real-Time Clock) HLE layer.
//!
//! Ports `rpcs3/Emu/Cell/Modules/cellRtc.cpp`. The PS3 RTC is a
//! **microsecond counter** from epoch year 1 AD, exposed via the
//! [`RtcTick`] type, plus a human-friendly [`RtcDateTime`] struct.
//! Games use the API to read the wall clock, do date arithmetic,
//! and format timestamps for save data.
//!
//! This port implements the pure conversion math + field validation
//! directly; the actual "current time" source is injected via the
//! [`WallClock`] trait so tests are deterministic.
//!
//! ## Entry points covered
//!
//! | HLE function                  | Rust wrapper                     |
//! |-------------------------------|----------------------------------|
//! | `cellRtcGetCurrentTick`       | [`cell_rtc_get_current_tick`]    |
//! | `cellRtcGetCurrentClockLocalTime` | [`cell_rtc_get_current_clock_local_time`] |
//! | `cellRtcSetTick`              | [`cell_rtc_set_tick`]            |
//! | `cellRtcGetTick`              | [`cell_rtc_get_tick`]            |
//! | `cellRtcCheckValid`           | [`cell_rtc_check_valid`]         |
//! | `cellRtcTickAddTicks`         | [`cell_rtc_tick_add_ticks`]      |
//! | `cellRtcTickAddMicroseconds`  | [`cell_rtc_tick_add_microseconds`] |
//! | `cellRtcTickAddSeconds`       | [`cell_rtc_tick_add_seconds`]    |
//! | `cellRtcTickAddMinutes`       | [`cell_rtc_tick_add_minutes`]    |
//! | `cellRtcTickAddHours`         | [`cell_rtc_tick_add_hours`]      |
//! | `cellRtcTickAddDays`          | [`cell_rtc_tick_add_days`]       |
//! | `cellRtcCompareTick`          | [`cell_rtc_compare_tick`]        |

use rpcs3_emu_types::CellError;

// =====================================================================
// Error codes — byte-exact with cellRtc.h:9-22
// =====================================================================

pub mod errors {
    use rpcs3_emu_types::CellError;

    pub const NOT_INITIALIZED: CellError = CellError(0x8001_0601);
    pub const INVALID_POINTER: CellError = CellError(0x8001_0602);
    pub const INVALID_VALUE: CellError = CellError(0x8001_0603);
    pub const INVALID_ARG: CellError = CellError(0x8001_0604);
    pub const NOT_SUPPORTED: CellError = CellError(0x8001_0605);
    pub const NO_CLOCK: CellError = CellError(0x8001_0606);
    pub const BAD_PARSE: CellError = CellError(0x8001_0607);
    pub const INVALID_YEAR: CellError = CellError(0x8001_0621);
    pub const INVALID_MONTH: CellError = CellError(0x8001_0622);
    pub const INVALID_DAY: CellError = CellError(0x8001_0623);
    pub const INVALID_HOUR: CellError = CellError(0x8001_0624);
    pub const INVALID_MINUTE: CellError = CellError(0x8001_0625);
    pub const INVALID_SECOND: CellError = CellError(0x8001_0626);
    pub const INVALID_MICROSECOND: CellError = CellError(0x8001_0627);
}

// =====================================================================
// Data model
// =====================================================================

/// Microseconds since 1 AD (the PS3 epoch).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct RtcTick(pub u64);

/// Broken-down wall-clock date. Matches `CellRtcDateTime` field order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RtcDateTime {
    pub year: u16,
    pub month: u16,
    pub day: u16,
    pub hour: u16,
    pub minute: u16,
    pub second: u16,
    pub microsecond: u32,
}

// =====================================================================
// Constants & helpers
// =====================================================================

/// Microseconds per second.
pub const US_PER_SEC: u64 = 1_000_000;
pub const US_PER_MIN: u64 = 60 * US_PER_SEC;
pub const US_PER_HOUR: u64 = 60 * US_PER_MIN;
pub const US_PER_DAY: u64 = 24 * US_PER_HOUR;

fn is_leap_year(year: u16) -> bool {
    let y = year as u32;
    (y % 4 == 0 && y % 100 != 0) || (y % 400 == 0)
}

fn days_in_month(year: u16, month: u16) -> u16 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => if is_leap_year(year) { 29 } else { 28 },
        _ => 0,
    }
}

fn days_in_year(year: u16) -> u32 {
    if is_leap_year(year) { 366 } else { 365 }
}

/// Validate that every field of `dt` is within its canonical range.
pub fn validate(dt: &RtcDateTime) -> Result<(), CellError> {
    if dt.year == 0 || dt.year > 9999 {
        return Err(errors::INVALID_YEAR);
    }
    if dt.month < 1 || dt.month > 12 {
        return Err(errors::INVALID_MONTH);
    }
    let dim = days_in_month(dt.year, dt.month);
    if dt.day < 1 || dt.day > dim {
        return Err(errors::INVALID_DAY);
    }
    if dt.hour > 23 {
        return Err(errors::INVALID_HOUR);
    }
    if dt.minute > 59 {
        return Err(errors::INVALID_MINUTE);
    }
    if dt.second > 59 {
        return Err(errors::INVALID_SECOND);
    }
    if dt.microsecond > 999_999 {
        return Err(errors::INVALID_MICROSECOND);
    }
    Ok(())
}

// =====================================================================
// Tick ↔ DateTime conversion
// =====================================================================

fn date_time_to_tick(dt: &RtcDateTime) -> u64 {
    // Count days from year 1 to (dt.year - 1) inclusive.
    let mut days: u64 = 0;
    for y in 1..dt.year {
        days += days_in_year(y) as u64;
    }
    for m in 1..dt.month {
        days += days_in_month(dt.year, m) as u64;
    }
    days += (dt.day - 1) as u64;

    let us = days * US_PER_DAY
        + (dt.hour as u64) * US_PER_HOUR
        + (dt.minute as u64) * US_PER_MIN
        + (dt.second as u64) * US_PER_SEC
        + (dt.microsecond as u64);
    us
}

fn tick_to_date_time(tick: u64) -> RtcDateTime {
    let total_days = tick / US_PER_DAY;
    let rem_us = tick % US_PER_DAY;

    // Year walk.
    let mut year: u16 = 1;
    let mut days_left = total_days;
    loop {
        let diy = days_in_year(year) as u64;
        if days_left < diy {
            break;
        }
        days_left -= diy;
        year += 1;
    }

    let mut month: u16 = 1;
    loop {
        let dim = days_in_month(year, month) as u64;
        if days_left < dim {
            break;
        }
        days_left -= dim;
        month += 1;
    }

    let day = (days_left as u16) + 1;

    let hour = (rem_us / US_PER_HOUR) as u16;
    let mm = rem_us % US_PER_HOUR;
    let minute = (mm / US_PER_MIN) as u16;
    let ss = mm % US_PER_MIN;
    let second = (ss / US_PER_SEC) as u16;
    let microsecond = (ss % US_PER_SEC) as u32;

    RtcDateTime { year, month, day, hour, minute, second, microsecond }
}

// =====================================================================
// WallClock trait — injects "current time" for deterministic tests
// =====================================================================

pub trait WallClock {
    fn now_tick(&self) -> RtcTick;
}

/// Fixed-time clock useful in tests.
#[derive(Debug, Clone, Copy)]
pub struct FixedClock(pub RtcTick);

impl WallClock for FixedClock {
    fn now_tick(&self) -> RtcTick { self.0 }
}

// =====================================================================
// Syscalls
// =====================================================================

#[must_use]
pub fn cell_rtc_get_current_tick<C: WallClock + ?Sized>(clock: &C) -> RtcTick {
    clock.now_tick()
}

/// `cellRtcGetCurrentClockLocalTime(clock_out)` — current time as a
/// fully-populated `RtcDateTime`. Port ignores timezone (host is
/// assumed UTC for the unit tests; real impl would offset).
#[must_use]
pub fn cell_rtc_get_current_clock_local_time<C: WallClock + ?Sized>(clock: &C) -> RtcDateTime {
    tick_to_date_time(clock.now_tick().0)
}

/// `cellRtcSetTick(datetime_out, tick)`.
#[must_use]
pub fn cell_rtc_set_tick(tick: RtcTick) -> RtcDateTime {
    tick_to_date_time(tick.0)
}

/// `cellRtcGetTick(datetime, tick_out)` — validates fields first.
#[must_use]
pub fn cell_rtc_get_tick(dt: &RtcDateTime) -> Result<RtcTick, CellError> {
    validate(dt)?;
    Ok(RtcTick(date_time_to_tick(dt)))
}

/// `cellRtcCheckValid(datetime)` — field validation.
#[must_use]
pub fn cell_rtc_check_valid(dt: &RtcDateTime) -> Result<(), CellError> {
    validate(dt)
}

#[must_use]
pub fn cell_rtc_tick_add_ticks(t: RtcTick, delta: i64) -> RtcTick {
    RtcTick((t.0 as i64).wrapping_add(delta) as u64)
}
#[must_use]
pub fn cell_rtc_tick_add_microseconds(t: RtcTick, delta: i64) -> RtcTick {
    cell_rtc_tick_add_ticks(t, delta)
}
#[must_use]
pub fn cell_rtc_tick_add_seconds(t: RtcTick, delta: i64) -> RtcTick {
    cell_rtc_tick_add_ticks(t, delta.saturating_mul(US_PER_SEC as i64))
}
#[must_use]
pub fn cell_rtc_tick_add_minutes(t: RtcTick, delta: i64) -> RtcTick {
    cell_rtc_tick_add_ticks(t, delta.saturating_mul(US_PER_MIN as i64))
}
#[must_use]
pub fn cell_rtc_tick_add_hours(t: RtcTick, delta: i64) -> RtcTick {
    cell_rtc_tick_add_ticks(t, delta.saturating_mul(US_PER_HOUR as i64))
}
#[must_use]
pub fn cell_rtc_tick_add_days(t: RtcTick, delta: i64) -> RtcTick {
    cell_rtc_tick_add_ticks(t, delta.saturating_mul(US_PER_DAY as i64))
}

/// `cellRtcCompareTick(a, b)` — returns negative/0/positive.
#[must_use]
pub fn cell_rtc_compare_tick(a: RtcTick, b: RtcTick) -> i32 {
    use core::cmp::Ordering::*;
    match a.cmp(&b) {
        Less => -1,
        Equal => 0,
        Greater => 1,
    }
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // --- constants ------------------------------------------------

    #[test]
    fn error_codes_match_cpp() {
        assert_eq!(errors::NOT_INITIALIZED.0, 0x8001_0601);
        assert_eq!(errors::INVALID_MONTH.0, 0x8001_0622);
        assert_eq!(errors::INVALID_DAY.0, 0x8001_0623);
        assert_eq!(errors::INVALID_MICROSECOND.0, 0x8001_0627);
    }

    #[test]
    fn tick_constants_derive_correctly() {
        assert_eq!(US_PER_SEC, 1_000_000);
        assert_eq!(US_PER_MIN, 60_000_000);
        assert_eq!(US_PER_HOUR, 3_600_000_000);
        assert_eq!(US_PER_DAY, 86_400_000_000);
    }

    // --- leap year ------------------------------------------------

    #[test]
    fn leap_year_rules() {
        assert!(!is_leap_year(1900), "centennial non-400");
        assert!(is_leap_year(2000), "centennial 400");
        assert!(is_leap_year(2024));
        assert!(!is_leap_year(2023));
    }

    #[test]
    fn days_in_february_matches_leap_year() {
        assert_eq!(days_in_month(2024, 2), 29);
        assert_eq!(days_in_month(2023, 2), 28);
    }

    // --- validation ----------------------------------------------

    #[test]
    fn validate_happy_path() {
        let dt = RtcDateTime { year: 2024, month: 4, day: 22, hour: 12, minute: 30, second: 45, microsecond: 123_456 };
        validate(&dt).unwrap();
    }

    #[test]
    fn validate_rejects_year_zero() {
        let dt = RtcDateTime { year: 0, month: 1, day: 1, ..RtcDateTime::default() };
        assert_eq!(validate(&dt).unwrap_err(), errors::INVALID_YEAR);
    }

    #[test]
    fn validate_rejects_month_13() {
        let dt = RtcDateTime { year: 2024, month: 13, day: 1, ..RtcDateTime::default() };
        assert_eq!(validate(&dt).unwrap_err(), errors::INVALID_MONTH);
    }

    #[test]
    fn validate_rejects_feb_30_in_non_leap() {
        let dt = RtcDateTime { year: 2023, month: 2, day: 29, ..RtcDateTime::default() };
        assert_eq!(validate(&dt).unwrap_err(), errors::INVALID_DAY);
    }

    #[test]
    fn validate_accepts_feb_29_in_leap() {
        let dt = RtcDateTime { year: 2024, month: 2, day: 29, ..RtcDateTime::default() };
        validate(&dt).unwrap();
    }

    #[test]
    fn validate_rejects_hour_24() {
        let dt = RtcDateTime { year: 2024, month: 1, day: 1, hour: 24, ..RtcDateTime::default() };
        assert_eq!(validate(&dt).unwrap_err(), errors::INVALID_HOUR);
    }

    #[test]
    fn validate_rejects_microsecond_1_million() {
        let dt = RtcDateTime { year: 2024, month: 1, day: 1, microsecond: 1_000_000, ..RtcDateTime::default() };
        assert_eq!(validate(&dt).unwrap_err(), errors::INVALID_MICROSECOND);
    }

    // --- tick ↔ datetime round-trip -------------------------------

    #[test]
    fn round_trip_epoch_start() {
        let dt = RtcDateTime { year: 1, month: 1, day: 1, ..RtcDateTime::default() };
        let tick = cell_rtc_get_tick(&dt).unwrap();
        assert_eq!(tick.0, 0, "year 1 jan 1 00:00:00 is tick 0");
        assert_eq!(cell_rtc_set_tick(tick), dt);
    }

    #[test]
    fn round_trip_arbitrary_datetime() {
        let dt = RtcDateTime { year: 2024, month: 4, day: 22, hour: 9, minute: 30, second: 15, microsecond: 500 };
        let tick = cell_rtc_get_tick(&dt).unwrap();
        assert_eq!(cell_rtc_set_tick(tick), dt);
    }

    #[test]
    fn round_trip_leap_day() {
        let dt = RtcDateTime { year: 2024, month: 2, day: 29, hour: 23, minute: 59, second: 59, microsecond: 999_999 };
        let tick = cell_rtc_get_tick(&dt).unwrap();
        assert_eq!(cell_rtc_set_tick(tick), dt);
    }

    #[test]
    fn get_tick_rejects_invalid_datetime() {
        let dt = RtcDateTime { year: 2024, month: 13, day: 1, ..RtcDateTime::default() };
        assert_eq!(cell_rtc_get_tick(&dt).unwrap_err(), errors::INVALID_MONTH);
    }

    // --- arithmetic -----------------------------------------------

    #[test]
    fn add_seconds_moves_forward() {
        let dt = RtcDateTime { year: 2024, month: 1, day: 1, hour: 12, minute: 0, second: 0, microsecond: 0 };
        let t0 = cell_rtc_get_tick(&dt).unwrap();
        let t1 = cell_rtc_tick_add_seconds(t0, 30);
        let dt1 = cell_rtc_set_tick(t1);
        assert_eq!(dt1.second, 30);
    }

    #[test]
    fn add_hours_overflows_day_when_past_midnight() {
        let dt = RtcDateTime { year: 2024, month: 1, day: 1, hour: 23, minute: 0, second: 0, microsecond: 0 };
        let t0 = cell_rtc_get_tick(&dt).unwrap();
        let t1 = cell_rtc_tick_add_hours(t0, 2);
        let dt1 = cell_rtc_set_tick(t1);
        assert_eq!(dt1.day, 2);
        assert_eq!(dt1.hour, 1);
    }

    #[test]
    fn add_days_crosses_month_boundary() {
        let dt = RtcDateTime { year: 2024, month: 1, day: 30, ..RtcDateTime::default() };
        let t0 = cell_rtc_get_tick(&dt).unwrap();
        let t1 = cell_rtc_tick_add_days(t0, 5);
        let dt1 = cell_rtc_set_tick(t1);
        assert_eq!(dt1.month, 2);
        assert_eq!(dt1.day, 4);
    }

    #[test]
    fn add_negative_seconds_moves_backward() {
        let dt = RtcDateTime { year: 2024, month: 1, day: 1, hour: 12, minute: 0, second: 30, ..RtcDateTime::default() };
        let t0 = cell_rtc_get_tick(&dt).unwrap();
        let t1 = cell_rtc_tick_add_seconds(t0, -45);
        let dt1 = cell_rtc_set_tick(t1);
        assert_eq!(dt1.hour, 11);
        assert_eq!(dt1.minute, 59);
        assert_eq!(dt1.second, 45);
    }

    #[test]
    fn compare_tick_returns_minus_zero_plus() {
        let a = RtcTick(100);
        let b = RtcTick(200);
        assert_eq!(cell_rtc_compare_tick(a, b), -1);
        assert_eq!(cell_rtc_compare_tick(b, a), 1);
        assert_eq!(cell_rtc_compare_tick(a, a), 0);
    }

    // --- wall clock ----------------------------------------------

    #[test]
    fn fixed_clock_returns_stored_tick() {
        let dt = RtcDateTime { year: 2024, month: 4, day: 22, hour: 14, ..RtcDateTime::default() };
        let t = cell_rtc_get_tick(&dt).unwrap();
        let clock = FixedClock(t);
        assert_eq!(cell_rtc_get_current_tick(&clock), t);
        assert_eq!(
            cell_rtc_get_current_clock_local_time(&clock).day,
            22,
        );
    }
}
