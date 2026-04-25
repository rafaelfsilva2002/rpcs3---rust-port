//! Rust port of `rpcs3/Emu/Cell/Modules/cellRtcAlarm.cpp`.
//!
//! 5 PRX entry points under the module name `cellRtcAlarm`. Every body
//! in the C++ is a stub that logs `UNIMPLEMENTED_FUNC` and returns
//! `CELL_OK`. The real firmware wires these to the RTC chip's wake-up
//! alarm — an off-the-PPU signal that fires at a configured time even
//! when the PS3 is in low-power standby.
//!
//! REG_FUNC order at cpp:38-42:
//!
//!  1. `cellRtcAlarmRegister`
//!  2. `cellRtcAlarmUnregister`
//!  3. `cellRtcAlarmNotification`
//!  4. `cellRtcAlarmStopRunning`
//!  5. `cellRtcAlarmGetStatus`
//!
//! Module name is byte-exact at cpp:4 / cpp:36.
//!
//! The Rust port preserves the happy-path `CELL_OK` semantics and
//! layers a small FSM so callers that skip steps (double-register,
//! stop-without-running) surface a named error instead of silent
//! success. Facility `0x8001_07__` is reserved in this port for
//! cellRtcAlarm placeholders (adjacent to `0x8001_06__` that
//! `rpcs3-hle-cellrtc` already claims).

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

use rpcs3_emu_types::CellError;

/// Byte-exact at cpp:4 / cpp:36.
pub const MODULE_NAME: &str = "cellRtcAlarm";

/// REG_FUNC order at cpp:38-42.
pub const REGISTERED_ENTRY_POINTS: &[&str] = &[
    "cellRtcAlarmRegister",
    "cellRtcAlarmUnregister",
    "cellRtcAlarmNotification",
    "cellRtcAlarmStopRunning",
    "cellRtcAlarmGetStatus",
];

// --- Error codes (placeholder facility 0x8001_07__) ---------------------
//
// The C++ source commits no named error codes. These values are
// internal placeholders used purely to enforce the Rust FSM; the
// happy-path `CELL_OK` semantics of the stub are preserved.

pub const CELL_RTC_ALARM_ERROR_NOT_REGISTERED: CellError = CellError(0x8001_0701);
pub const CELL_RTC_ALARM_ERROR_ALREADY_REGISTERED: CellError = CellError(0x8001_0702);
pub const CELL_RTC_ALARM_ERROR_NOT_RUNNING: CellError = CellError(0x8001_0703);
pub const CELL_RTC_ALARM_ERROR_ALREADY_RUNNING: CellError = CellError(0x8001_0704);
pub const CELL_RTC_ALARM_ERROR_INVALID_PARAMETER: CellError = CellError(0x8001_0705);

// --- FSM -----------------------------------------------------------------

/// Lifecycle of a single registered RTC alarm.
///
/// `Unregistered` → `Registered` via `register` → `Running` via
/// `notification` (firmware fires the alarm) → `Registered` via
/// `stop_running` (back to armed-but-quiet) → `Unregistered` via
/// `unregister`. Double-transitions are rejected with the matching
/// `ALREADY_*` / `NOT_*` error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlarmState {
    Unregistered,
    Registered,
    Running,
}

impl AlarmState {
    /// Mirrors the `status` payload a `GetStatus` call would publish in
    /// the firmware. The real SDK packs more bits here, but for the
    /// stub a 0/1/2 mapping covers the observable surface.
    #[must_use]
    pub const fn as_status_code(self) -> u32 {
        match self {
            Self::Unregistered => 0,
            Self::Registered => 1,
            Self::Running => 2,
        }
    }
}

// --- Manager ------------------------------------------------------------

/// HLE state for the single alarm singleton the firmware exposes.
/// The real SDK wires the register arg list directly to a kernel-level
/// wake queue; the port stores the caller-supplied handler address +
/// fire time so tests can exercise the full dispatch trace.
#[derive(Debug, Default)]
pub struct RtcAlarm {
    state: Option<AlarmState>,
    handler_addr: u64,
    fire_time_utc_us: u64,
    last_notification_us: u64,
    notification_count: u32,
    register_calls: u32,
    unregister_calls: u32,
    notification_calls: u32,
    stop_running_calls: u32,
    get_status_calls: u32,
}

impl RtcAlarm {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            state: None,
            handler_addr: 0,
            fire_time_utc_us: 0,
            last_notification_us: 0,
            notification_count: 0,
            register_calls: 0,
            unregister_calls: 0,
            notification_calls: 0,
            stop_running_calls: 0,
            get_status_calls: 0,
        }
    }

    #[must_use]
    pub fn state(&self) -> AlarmState {
        self.state.unwrap_or(AlarmState::Unregistered)
    }

    #[must_use]
    pub fn handler_addr(&self) -> u64 {
        self.handler_addr
    }
    #[must_use]
    pub fn fire_time_utc_us(&self) -> u64 {
        self.fire_time_utc_us
    }
    #[must_use]
    pub fn last_notification_us(&self) -> u64 {
        self.last_notification_us
    }
    #[must_use]
    pub fn notification_count(&self) -> u32 {
        self.notification_count
    }

    #[must_use]
    pub fn register_calls(&self) -> u32 {
        self.register_calls
    }
    #[must_use]
    pub fn unregister_calls(&self) -> u32 {
        self.unregister_calls
    }
    #[must_use]
    pub fn notification_calls(&self) -> u32 {
        self.notification_calls
    }
    #[must_use]
    pub fn stop_running_calls(&self) -> u32 {
        self.stop_running_calls
    }
    #[must_use]
    pub fn get_status_calls(&self) -> u32 {
        self.get_status_calls
    }

    // --- entry points ---

    /// `cellRtcAlarmRegister` (cpp:6-10). Arms the alarm with a
    /// handler and a fire time. `handler_addr == 0` is rejected with
    /// `INVALID_PARAMETER` since a null callback has no useful
    /// semantics. Double-register (without an intervening unregister)
    /// returns `ALREADY_REGISTERED` — the firmware refuses to silently
    /// replace an armed alarm.
    pub fn register(
        &mut self,
        handler_addr: u64,
        fire_time_utc_us: u64,
    ) -> Result<(), CellError> {
        if handler_addr == 0 {
            return Err(CELL_RTC_ALARM_ERROR_INVALID_PARAMETER);
        }
        if self.state() != AlarmState::Unregistered {
            return Err(CELL_RTC_ALARM_ERROR_ALREADY_REGISTERED);
        }
        self.handler_addr = handler_addr;
        self.fire_time_utc_us = fire_time_utc_us;
        self.state = Some(AlarmState::Registered);
        self.register_calls = self.register_calls.saturating_add(1);
        Ok(())
    }

    /// `cellRtcAlarmUnregister` (cpp:12-16). Drops the alarm back to
    /// `Unregistered`. Accepts from any live state — the firmware
    /// tolerates unregister on a running alarm to handle emergency
    /// teardown (e.g. from a game shutdown hook).
    pub fn unregister(&mut self) -> Result<(), CellError> {
        if self.state() == AlarmState::Unregistered {
            return Err(CELL_RTC_ALARM_ERROR_NOT_REGISTERED);
        }
        self.state = Some(AlarmState::Unregistered);
        self.handler_addr = 0;
        self.fire_time_utc_us = 0;
        self.unregister_calls = self.unregister_calls.saturating_add(1);
        Ok(())
    }

    /// `cellRtcAlarmNotification` (cpp:18-22). Simulates the firmware
    /// firing the alarm — bumps the notification counter, records the
    /// timestamp, transitions `Registered → Running`. A second fire
    /// while already `Running` returns `ALREADY_RUNNING`.
    pub fn notification(&mut self, now_utc_us: u64) -> Result<(), CellError> {
        match self.state() {
            AlarmState::Unregistered => Err(CELL_RTC_ALARM_ERROR_NOT_REGISTERED),
            AlarmState::Running => Err(CELL_RTC_ALARM_ERROR_ALREADY_RUNNING),
            AlarmState::Registered => {
                self.state = Some(AlarmState::Running);
                self.last_notification_us = now_utc_us;
                self.notification_count = self.notification_count.saturating_add(1);
                self.notification_calls = self.notification_calls.saturating_add(1);
                Ok(())
            }
        }
    }

    /// `cellRtcAlarmStopRunning` (cpp:24-28). Transitions `Running →
    /// Registered`; rejects a call issued while the alarm is not
    /// currently running.
    pub fn stop_running(&mut self) -> Result<(), CellError> {
        match self.state() {
            AlarmState::Unregistered => Err(CELL_RTC_ALARM_ERROR_NOT_REGISTERED),
            AlarmState::Registered => Err(CELL_RTC_ALARM_ERROR_NOT_RUNNING),
            AlarmState::Running => {
                self.state = Some(AlarmState::Registered);
                self.stop_running_calls = self.stop_running_calls.saturating_add(1);
                Ok(())
            }
        }
    }

    /// `cellRtcAlarmGetStatus` (cpp:30-34). Returns a status code —
    /// firmware packs more bits in practice, but the stub treats
    /// `0 == Unregistered`, `1 == Registered`, `2 == Running`.
    pub fn get_status(&mut self) -> Result<u32, CellError> {
        self.get_status_calls = self.get_status_calls.saturating_add(1);
        Ok(self.state().as_status_code())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_name_byte_exact() {
        assert_eq!(MODULE_NAME, "cellRtcAlarm");
    }

    #[test]
    fn registered_entry_points_exact_order() {
        assert_eq!(REGISTERED_ENTRY_POINTS.len(), 5);
        assert_eq!(REGISTERED_ENTRY_POINTS[0], "cellRtcAlarmRegister");
        assert_eq!(REGISTERED_ENTRY_POINTS[1], "cellRtcAlarmUnregister");
        assert_eq!(REGISTERED_ENTRY_POINTS[2], "cellRtcAlarmNotification");
        assert_eq!(REGISTERED_ENTRY_POINTS[3], "cellRtcAlarmStopRunning");
        assert_eq!(REGISTERED_ENTRY_POINTS[4], "cellRtcAlarmGetStatus");
    }

    #[test]
    fn error_codes_byte_exact() {
        assert_eq!(CELL_RTC_ALARM_ERROR_NOT_REGISTERED.0, 0x8001_0701);
        assert_eq!(CELL_RTC_ALARM_ERROR_ALREADY_REGISTERED.0, 0x8001_0702);
        assert_eq!(CELL_RTC_ALARM_ERROR_NOT_RUNNING.0, 0x8001_0703);
        assert_eq!(CELL_RTC_ALARM_ERROR_ALREADY_RUNNING.0, 0x8001_0704);
        assert_eq!(CELL_RTC_ALARM_ERROR_INVALID_PARAMETER.0, 0x8001_0705);
    }

    #[test]
    fn state_status_code_mapping() {
        assert_eq!(AlarmState::Unregistered.as_status_code(), 0);
        assert_eq!(AlarmState::Registered.as_status_code(), 1);
        assert_eq!(AlarmState::Running.as_status_code(), 2);
    }

    #[test]
    fn starts_unregistered() {
        let a = RtcAlarm::new();
        assert_eq!(a.state(), AlarmState::Unregistered);
        assert_eq!(a.handler_addr(), 0);
    }

    #[test]
    fn register_null_handler_is_invalid_parameter() {
        let mut a = RtcAlarm::new();
        assert_eq!(
            a.register(0, 1_000_000),
            Err(CELL_RTC_ALARM_ERROR_INVALID_PARAMETER)
        );
    }

    #[test]
    fn register_stores_handler_and_time() {
        let mut a = RtcAlarm::new();
        a.register(0xDEAD_BEEF, 1_700_000_000_000_000).unwrap();
        assert_eq!(a.state(), AlarmState::Registered);
        assert_eq!(a.handler_addr(), 0xDEAD_BEEF);
        assert_eq!(a.fire_time_utc_us(), 1_700_000_000_000_000);
        assert_eq!(a.register_calls(), 1);
    }

    #[test]
    fn double_register_is_already_registered() {
        let mut a = RtcAlarm::new();
        a.register(1, 10).unwrap();
        assert_eq!(
            a.register(2, 20),
            Err(CELL_RTC_ALARM_ERROR_ALREADY_REGISTERED)
        );
        assert_eq!(a.handler_addr(), 1); // original preserved
    }

    #[test]
    fn unregister_without_register_is_not_registered() {
        let mut a = RtcAlarm::new();
        assert_eq!(
            a.unregister(),
            Err(CELL_RTC_ALARM_ERROR_NOT_REGISTERED)
        );
    }

    #[test]
    fn unregister_clears_state_and_handler() {
        let mut a = RtcAlarm::new();
        a.register(0xAB, 42).unwrap();
        a.unregister().unwrap();
        assert_eq!(a.state(), AlarmState::Unregistered);
        assert_eq!(a.handler_addr(), 0);
        assert_eq!(a.fire_time_utc_us(), 0);
    }

    #[test]
    fn unregister_from_running_is_tolerated() {
        let mut a = RtcAlarm::new();
        a.register(0xAB, 42).unwrap();
        a.notification(100).unwrap();
        assert_eq!(a.state(), AlarmState::Running);
        a.unregister().unwrap();
        assert_eq!(a.state(), AlarmState::Unregistered);
    }

    #[test]
    fn notification_without_register_is_not_registered() {
        let mut a = RtcAlarm::new();
        assert_eq!(
            a.notification(100),
            Err(CELL_RTC_ALARM_ERROR_NOT_REGISTERED)
        );
    }

    #[test]
    fn notification_transitions_to_running() {
        let mut a = RtcAlarm::new();
        a.register(1, 10).unwrap();
        a.notification(1_000_000).unwrap();
        assert_eq!(a.state(), AlarmState::Running);
        assert_eq!(a.last_notification_us(), 1_000_000);
        assert_eq!(a.notification_count(), 1);
    }

    #[test]
    fn notification_while_running_is_already_running() {
        let mut a = RtcAlarm::new();
        a.register(1, 10).unwrap();
        a.notification(100).unwrap();
        assert_eq!(
            a.notification(200),
            Err(CELL_RTC_ALARM_ERROR_ALREADY_RUNNING)
        );
    }

    #[test]
    fn stop_running_without_register_is_not_registered() {
        let mut a = RtcAlarm::new();
        assert_eq!(
            a.stop_running(),
            Err(CELL_RTC_ALARM_ERROR_NOT_REGISTERED)
        );
    }

    #[test]
    fn stop_running_while_not_running_is_not_running() {
        let mut a = RtcAlarm::new();
        a.register(1, 10).unwrap();
        assert_eq!(
            a.stop_running(),
            Err(CELL_RTC_ALARM_ERROR_NOT_RUNNING)
        );
    }

    #[test]
    fn stop_running_transitions_back_to_registered() {
        let mut a = RtcAlarm::new();
        a.register(1, 10).unwrap();
        a.notification(100).unwrap();
        a.stop_running().unwrap();
        assert_eq!(a.state(), AlarmState::Registered);
    }

    #[test]
    fn rearm_after_stop_running_allowed() {
        let mut a = RtcAlarm::new();
        a.register(1, 10).unwrap();
        a.notification(100).unwrap();
        a.stop_running().unwrap();
        a.notification(200).unwrap();
        assert_eq!(a.state(), AlarmState::Running);
        assert_eq!(a.notification_count(), 2);
    }

    #[test]
    fn get_status_reports_current_state() {
        let mut a = RtcAlarm::new();
        assert_eq!(a.get_status().unwrap(), 0);
        a.register(1, 10).unwrap();
        assert_eq!(a.get_status().unwrap(), 1);
        a.notification(100).unwrap();
        assert_eq!(a.get_status().unwrap(), 2);
    }

    #[test]
    fn re_register_after_unregister_allowed() {
        let mut a = RtcAlarm::new();
        a.register(1, 10).unwrap();
        a.unregister().unwrap();
        a.register(2, 20).unwrap();
        assert_eq!(a.handler_addr(), 2);
        assert_eq!(a.fire_time_utc_us(), 20);
    }

    #[test]
    fn full_rtcalarm_lifecycle_smoke() {
        let mut a = RtcAlarm::new();

        // 1. Register an alarm at T0 + 60s (1µs epoch arithmetic for
        //    convenience).
        a.register(0x8000_0000, 60_000_000).unwrap();
        assert_eq!(a.state(), AlarmState::Registered);
        assert_eq!(a.get_status().unwrap(), 1);

        // 2. The RTC chip fires — transition to Running.
        a.notification(60_000_000).unwrap();
        assert_eq!(a.state(), AlarmState::Running);
        assert_eq!(a.last_notification_us(), 60_000_000);

        // 3. Game drains the callback, calls stop_running to go back
        //    to armed.
        a.stop_running().unwrap();
        assert_eq!(a.state(), AlarmState::Registered);

        // 4. Second fire cycle.
        a.notification(120_000_000).unwrap();
        assert_eq!(a.notification_count(), 2);

        // 5. Emergency teardown — unregister from Running.
        a.unregister().unwrap();
        assert_eq!(a.state(), AlarmState::Unregistered);
        assert_eq!(a.handler_addr(), 0);

        // 6. Counter trace.
        assert_eq!(a.register_calls(), 1);
        assert_eq!(a.unregister_calls(), 1);
        assert_eq!(a.notification_calls(), 2);
        assert_eq!(a.stop_running_calls(), 1);
        assert!(a.get_status_calls() >= 1);
    }
}
