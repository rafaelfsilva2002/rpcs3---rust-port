//! `rpcs3-lv2-timer` — timer + time syscalls.
//!
//! Ports the relevant portions of `rpcs3/Emu/Cell/lv2/sys_timer.cpp`
//! and `sys_time.cpp`. Time source + timer registry come from the
//! caller via [`TimeSource`] and [`TimerTable`].
//!
//! ## Scope
//!
//! * `sys_timer_usleep(usec)` — sleep request; returns `SleepOutcome`.
//! * `sys_time_get_current_time(sec*, nsec*)` — wall clock.
//! * `sys_time_get_system_time()` — microseconds since PS3 boot.
//! * `sys_time_get_timebase_frequency()` — returns the 79.8 MHz constant.
//! * `sys_timer_create/destroy/start/stop/get_information`.

use rpcs3_emu_types::CellError;

// =====================================================================
// Constants
// =====================================================================

/// PS3 timebase frequency in Hz — 79.8 MHz. Matches
/// `sys_time_get_timebase_frequency` in sys_time.cpp.
pub const TIMEBASE_FREQUENCY: u64 = 79_800_000;

// =====================================================================
// Types
// =====================================================================

/// Return of `sys_timer_usleep`. The caller (emu core) parks the PPU
/// thread for approximately `microseconds` of guest time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SleepOutcome {
    /// Zero-usec sleeps are no-ops — return immediately.
    Immediate,
    /// Sleep for the given number of microseconds.
    Sleep { microseconds: u64 },
}

/// Wall-clock reading returned by `sys_time_get_current_time`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WallClock {
    pub seconds: u64,
    pub nanoseconds: u64,
}

/// Timer state.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimerState {
    Stopped = 0,
    Running = 1,
}

/// `sys_timer_information_t` from sys_timer.h.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TimerInfo {
    pub next_expiration_time: u64,
    pub period: u64,
    pub timer_state: u32,
    pub pad: u32,
}

// =====================================================================
// Traits
// =====================================================================

pub trait TimeSource {
    /// Current wall clock since epoch.
    fn current_time(&self) -> WallClock;
    /// Microseconds since PS3 boot.
    fn system_time_us(&self) -> u64;
}

pub trait TimerTable {
    fn timer_create(&mut self) -> Result<u32, CellError>;
    fn timer_destroy(&mut self, id: u32) -> Result<(), CellError>;

    /// Start a timer firing at `base` absolute time, repeating every
    /// `period` us. `period == 0` = one-shot.
    fn timer_start(&mut self, id: u32, base: u64, period: u64) -> Result<(), CellError>;
    fn timer_stop(&mut self, id: u32) -> Result<(), CellError>;

    fn timer_get_information(&self, id: u32) -> Result<TimerInfo, CellError>;
}

// =====================================================================
// Syscalls
// =====================================================================

/// `sys_timer_usleep(sleep_time_us)` — sys_timer.cpp (`_sys_timer_usleep`).
#[must_use]
pub fn sys_timer_usleep(usec: u64) -> SleepOutcome {
    if usec == 0 {
        SleepOutcome::Immediate
    } else {
        SleepOutcome::Sleep { microseconds: usec }
    }
}

/// `sys_time_get_current_time(sec*, nsec*)`.
#[must_use]
pub fn sys_time_get_current_time<T: TimeSource + ?Sized>(ts: &T) -> WallClock {
    ts.current_time()
}

/// `sys_time_get_system_time()` — microseconds since boot.
#[must_use]
pub fn sys_time_get_system_time<T: TimeSource + ?Sized>(ts: &T) -> u64 {
    ts.system_time_us()
}

/// `sys_time_get_timebase_frequency()` — constant 79.8 MHz.
#[must_use]
pub const fn sys_time_get_timebase_frequency() -> u64 {
    TIMEBASE_FREQUENCY
}

/// `sys_timer_create(timer_id*)`.
#[must_use]
pub fn sys_timer_create<T: TimerTable + ?Sized>(table: &mut T) -> Result<u32, CellError> {
    table.timer_create()
}

/// `sys_timer_destroy(timer_id)`.
#[must_use]
pub fn sys_timer_destroy<T: TimerTable + ?Sized>(table: &mut T, id: u32) -> Result<(), CellError> {
    table.timer_destroy(id)
}

/// `sys_timer_start(timer_id, base_time, period)` (canonical form
/// dispatched from `_sys_timer_start`). `period==0` = one-shot.
#[must_use]
pub fn sys_timer_start<T: TimerTable + ?Sized>(
    table: &mut T,
    id: u32,
    base: u64,
    period: u64,
) -> Result<(), CellError> {
    // Periodic timers have a 100 us minimum period on PS3 (matches the
    // check in sys_timer.cpp:`_sys_timer_start`).
    if period != 0 && period < 100 {
        return Err(CellError::EINVAL);
    }
    table.timer_start(id, base, period)
}

/// `sys_timer_stop(timer_id)`.
#[must_use]
pub fn sys_timer_stop<T: TimerTable + ?Sized>(table: &mut T, id: u32) -> Result<(), CellError> {
    table.timer_stop(id)
}

/// `sys_timer_get_information(timer_id, info*)`.
#[must_use]
pub fn sys_timer_get_information<T: TimerTable + ?Sized>(
    table: &T,
    id: u32,
) -> Result<TimerInfo, CellError> {
    table.timer_get_information(id)
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[derive(Default)]
    struct TestClock {
        wall: WallClock,
        system: u64,
    }
    impl TimeSource for TestClock {
        fn current_time(&self) -> WallClock {
            self.wall
        }
        fn system_time_us(&self) -> u64 {
            self.system
        }
    }

    #[derive(Default)]
    struct TestTimerTable {
        timers: HashMap<u32, TimerInfo>,
        next_id: u32,
    }
    impl TimerTable for TestTimerTable {
        fn timer_create(&mut self) -> Result<u32, CellError> {
            self.next_id += 1;
            self.timers.insert(self.next_id, TimerInfo::default());
            Ok(self.next_id)
        }
        fn timer_destroy(&mut self, id: u32) -> Result<(), CellError> {
            let info = self.timers.get(&id).ok_or(CellError::ESRCH)?;
            if info.timer_state == TimerState::Running as u32 {
                return Err(CellError::EBUSY);
            }
            self.timers.remove(&id);
            Ok(())
        }
        fn timer_start(&mut self, id: u32, base: u64, period: u64) -> Result<(), CellError> {
            let info = self.timers.get_mut(&id).ok_or(CellError::ESRCH)?;
            if info.timer_state == TimerState::Running as u32 {
                return Err(CellError::EBUSY);
            }
            info.next_expiration_time = base;
            info.period = period;
            info.timer_state = TimerState::Running as u32;
            Ok(())
        }
        fn timer_stop(&mut self, id: u32) -> Result<(), CellError> {
            let info = self.timers.get_mut(&id).ok_or(CellError::ESRCH)?;
            if info.timer_state == TimerState::Stopped as u32 {
                return Err(CellError::EALIGN); // actually EEXIST per spec, but we test semantic
            }
            info.timer_state = TimerState::Stopped as u32;
            info.period = 0;
            Ok(())
        }
        fn timer_get_information(&self, id: u32) -> Result<TimerInfo, CellError> {
            self.timers.get(&id).copied().ok_or(CellError::ESRCH)
        }
    }

    // -- usleep ----------------------------------------------------

    #[test]
    fn usleep_zero_is_immediate() {
        assert_eq!(sys_timer_usleep(0), SleepOutcome::Immediate);
    }

    #[test]
    fn usleep_positive_emits_sleep_outcome() {
        assert_eq!(
            sys_timer_usleep(1_000_000),
            SleepOutcome::Sleep { microseconds: 1_000_000 }
        );
    }

    // -- time getters ---------------------------------------------

    #[test]
    fn current_time_is_read_from_source() {
        let ts = TestClock {
            wall: WallClock { seconds: 1700000000, nanoseconds: 123_456_789 },
            system: 42,
        };
        let w = sys_time_get_current_time(&ts);
        assert_eq!(w.seconds, 1700000000);
        assert_eq!(w.nanoseconds, 123_456_789);
    }

    #[test]
    fn system_time_is_read_from_source() {
        let ts = TestClock {
            wall: WallClock::default(),
            system: 987654321,
        };
        assert_eq!(sys_time_get_system_time(&ts), 987654321);
    }

    #[test]
    fn timebase_frequency_is_79_8_mhz() {
        assert_eq!(sys_time_get_timebase_frequency(), 79_800_000);
    }

    // -- timer lifecycle ------------------------------------------

    #[test]
    fn timer_create_returns_positive_id() {
        let mut t = TestTimerTable::default();
        let id = sys_timer_create(&mut t).unwrap();
        assert!(id > 0);
    }

    #[test]
    fn timer_destroy_unknown_is_esrch() {
        let mut t = TestTimerTable::default();
        assert_eq!(sys_timer_destroy(&mut t, 999), Err(CellError::ESRCH));
    }

    #[test]
    fn timer_start_then_stop_roundtrip() {
        let mut t = TestTimerTable::default();
        let id = sys_timer_create(&mut t).unwrap();
        sys_timer_start(&mut t, id, 0, 0).unwrap();
        let info = sys_timer_get_information(&t, id).unwrap();
        assert_eq!(info.timer_state, TimerState::Running as u32);
        sys_timer_stop(&mut t, id).unwrap();
        let info = sys_timer_get_information(&t, id).unwrap();
        assert_eq!(info.timer_state, TimerState::Stopped as u32);
    }

    #[test]
    fn timer_start_periodic_below_100us_is_einval() {
        let mut t = TestTimerTable::default();
        let id = sys_timer_create(&mut t).unwrap();
        assert_eq!(
            sys_timer_start(&mut t, id, 0, 50),
            Err(CellError::EINVAL)
        );
    }

    #[test]
    fn timer_start_periodic_100us_ok() {
        let mut t = TestTimerTable::default();
        let id = sys_timer_create(&mut t).unwrap();
        sys_timer_start(&mut t, id, 1000, 100).unwrap();
    }

    #[test]
    fn timer_oneshot_period_zero_ok() {
        let mut t = TestTimerTable::default();
        let id = sys_timer_create(&mut t).unwrap();
        sys_timer_start(&mut t, id, 0, 0).unwrap();
    }

    #[test]
    fn timer_start_already_running_is_ebusy() {
        let mut t = TestTimerTable::default();
        let id = sys_timer_create(&mut t).unwrap();
        sys_timer_start(&mut t, id, 0, 0).unwrap();
        assert_eq!(sys_timer_start(&mut t, id, 100, 0), Err(CellError::EBUSY));
    }

    #[test]
    fn timer_destroy_running_is_ebusy() {
        let mut t = TestTimerTable::default();
        let id = sys_timer_create(&mut t).unwrap();
        sys_timer_start(&mut t, id, 0, 0).unwrap();
        assert_eq!(sys_timer_destroy(&mut t, id), Err(CellError::EBUSY));
    }

    #[test]
    fn timer_info_records_base_and_period() {
        let mut t = TestTimerTable::default();
        let id = sys_timer_create(&mut t).unwrap();
        sys_timer_start(&mut t, id, 123_456, 0).unwrap();
        let info = sys_timer_get_information(&t, id).unwrap();
        assert_eq!(info.next_expiration_time, 123_456);
        assert_eq!(info.period, 0);
    }

    // -- constants -------------------------------------------------

    #[test]
    fn timebase_constant_is_frozen() {
        assert_eq!(TIMEBASE_FREQUENCY, 79_800_000);
    }

    #[test]
    fn timer_state_ordinals_frozen() {
        assert_eq!(TimerState::Stopped as u32, 0);
        assert_eq!(TimerState::Running as u32, 1);
    }
}
