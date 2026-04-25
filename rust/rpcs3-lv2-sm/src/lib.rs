//! Rust port of `rpcs3/Emu/Cell/lv2/sys_sm.cpp` — PS3 LV2 System Manager
//! syscalls (6 entries, 105 lines C++).
//!
//! Lifecycle entries: `get_params/get_ext_event2/shutdown/set_shop_mode/
//! control_led/ring_buzzer`. The `shutdown` syscall has rich op-code
//! dispatching for shutdown vs reboot vs LPAR ops, and a root-permission
//! gate that returns `CELL_ENOSYS` for non-root callers.
//!
//! `no_std` + `alloc`. Single dep: `rpcs3-emu-types`.

#![no_std]

extern crate alloc;

use rpcs3_emu_types::CellError;

pub const MODULE_NAME: &str = "sys_sm";

pub const REGISTERED_ENTRY_POINTS: &[&str] = &[
    "sys_sm_get_params",
    "sys_sm_get_ext_event2",
    "sys_sm_shutdown",
    "sys_sm_set_shop_mode",
    "sys_sm_control_led",
    "sys_sm_ring_buzzer",
];

pub const CELL_EINVAL: CellError = CellError(0x8001_0002);
pub const CELL_ENOSYS: CellError = CellError(0x8001_0001);
pub const CELL_EFAULT: CellError = CellError(0x8001_000D);
pub const CELL_EAGAIN: CellError = CellError(0x8001_000B);
pub const CELL_ENOTSUP: CellError = CellError(0x8001_0033);

/// `get_params` constant outputs (cpp:17-20 byte-exato).
pub const GET_PARAMS_C: u32 = 0x200;
pub const GET_PARAMS_D: u64 = 7;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShutdownAction {
    /// op 0x100 / 0x1100 — application shutdown (calls _sys_process_exit).
    AppShutdown,
    /// op 0x200 / 0x1200 — application reboot (calls lv2_exitspawn).
    AppReboot,
}

/// `sys_sm_shutdown(op)` — return type-rich result modeling the upstream
/// dispatch table cpp:57-83.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShutdownOutcome {
    /// `op ∈ {0x100, 0x1100, 0x200, 0x1200}` succeeded — caller should
    /// invoke the side-effect (process_exit or lv2_exitspawn).
    Action(ShutdownAction),
    /// `op ∈ {0x8201, 0x8202, 0x8204}` LPAR ops — return CELL_ENOTSUP.
    Unsupported,
    /// All other ops → CELL_EINVAL.
    Invalid,
    /// Caller without root permission — CELL_ENOSYS (rejected before op switch).
    NoPermission,
}

#[derive(Debug, Default)]
pub struct SysSm {
    pub shop_mode: i32,
    pub led_states: [u8; 16], // small fixed table, indexed by led id
    pub buzzer_packets: u64,
    pub get_params_calls: u64,
    pub get_ext_event2_calls: u64,
    pub shutdown_calls: u64,
    pub set_shop_mode_calls: u64,
    pub control_led_calls: u64,
    pub ring_buzzer_calls: u64,
}

impl SysSm {
    pub fn new() -> Self {
        Self::default()
    }

    /// `sys_sm_get_params(a, b, c, d)` cpp:13-23 — writes 0,0,0x200,7;
    /// any null pointer = EFAULT (in arg-order checks).
    pub fn get_params(
        &mut self,
        a: Option<&mut u8>,
        b: Option<&mut u8>,
        c: Option<&mut u32>,
        d: Option<&mut u64>,
    ) -> Result<(), CellError> {
        self.get_params_calls = self.get_params_calls.saturating_add(1);
        let a = a.ok_or(CELL_EFAULT)?;
        *a = 0;
        let b = b.ok_or(CELL_EFAULT)?;
        *b = 0;
        let c = c.ok_or(CELL_EFAULT)?;
        *c = GET_PARAMS_C;
        let d = d.ok_or(CELL_EFAULT)?;
        *d = GET_PARAMS_D;
        Ok(())
    }

    /// `sys_sm_get_ext_event2(a1, a2, a3, a4)` — cpp:25-44.
    /// `a4 ∉ {0, 1}` → EINVAL; null outputs → EFAULT (in order); on success
    /// writes 0/0/0 and returns CELL_EAGAIN ("no event").
    pub fn get_ext_event2(
        &mut self,
        a1: Option<&mut u64>,
        a2: Option<&mut u64>,
        a3: Option<&mut u64>,
        a4: u64,
    ) -> Result<(), CellError> {
        self.get_ext_event2_calls = self.get_ext_event2_calls.saturating_add(1);
        if a4 != 0 && a4 != 1 {
            return Err(CELL_EINVAL);
        }
        let a1 = a1.ok_or(CELL_EFAULT)?;
        *a1 = 0;
        let a2 = a2.ok_or(CELL_EFAULT)?;
        *a2 = 0;
        let a3 = a3.ok_or(CELL_EFAULT)?;
        *a3 = 0;
        // not_an_error(CELL_EAGAIN) — successful return signaling "no event"
        Err(CELL_EAGAIN)
    }

    /// `sys_sm_shutdown(op, param, size, has_root_perm)` — cpp:46-84.
    /// Returns ShutdownOutcome enum encoding the C++ dispatch table.
    pub fn shutdown(
        &mut self,
        op: u16,
        has_root_perm: bool,
    ) -> ShutdownOutcome {
        self.shutdown_calls = self.shutdown_calls.saturating_add(1);
        if !has_root_perm {
            return ShutdownOutcome::NoPermission;
        }
        match op {
            0x100 | 0x1100 => ShutdownOutcome::Action(ShutdownAction::AppShutdown),
            0x200 | 0x1200 => ShutdownOutcome::Action(ShutdownAction::AppReboot),
            0x8201 | 0x8202 | 0x8204 => ShutdownOutcome::Unsupported,
            _ => ShutdownOutcome::Invalid,
        }
    }

    /// `sys_sm_set_shop_mode(mode)` — upstream stub returning CELL_OK.
    pub fn set_shop_mode(&mut self, mode: i32) -> Result<(), CellError> {
        self.set_shop_mode_calls = self.set_shop_mode_calls.saturating_add(1);
        self.shop_mode = mode;
        Ok(())
    }

    /// `sys_sm_control_led(led, action)` — upstream stub returning CELL_OK.
    pub fn control_led(&mut self, led: u8, action: u8) -> Result<(), CellError> {
        self.control_led_calls = self.control_led_calls.saturating_add(1);
        if (led as usize) < self.led_states.len() {
            self.led_states[led as usize] = action;
        }
        Ok(())
    }

    /// `sys_sm_ring_buzzer(packet, a1, a2)` — upstream stub returning CELL_OK.
    pub fn ring_buzzer(&mut self, packet: u64, _a1: u64, _a2: u64) -> Result<(), CellError> {
        self.ring_buzzer_calls = self.ring_buzzer_calls.saturating_add(1);
        self.buzzer_packets = self.buzzer_packets.saturating_add(packet);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_and_entries() {
        assert_eq!(MODULE_NAME, "sys_sm");
        assert_eq!(REGISTERED_ENTRY_POINTS.len(), 6);
    }

    #[test]
    fn get_params_writes_constants() {
        let mut m = SysSm::new();
        let mut a = 0xFFu8;
        let mut b = 0xFFu8;
        let mut c = 0u32;
        let mut d = 0u64;
        m.get_params(Some(&mut a), Some(&mut b), Some(&mut c), Some(&mut d))
            .unwrap();
        assert_eq!(a, 0);
        assert_eq!(b, 0);
        assert_eq!(c, 0x200);
        assert_eq!(d, 7);
    }

    #[test]
    fn get_params_null_a_is_efault_first() {
        let mut m = SysSm::new();
        let mut b = 0u8;
        let mut c = 0u32;
        let mut d = 0u64;
        // Only `a` is None — should fail with EFAULT before checking others.
        assert_eq!(
            m.get_params(None, Some(&mut b), Some(&mut c), Some(&mut d)),
            Err(CELL_EFAULT)
        );
    }

    #[test]
    fn get_ext_event2_bad_a4_is_einval() {
        let mut m = SysSm::new();
        let mut a1 = 0u64;
        let mut a2 = 0u64;
        let mut a3 = 0u64;
        assert_eq!(
            m.get_ext_event2(Some(&mut a1), Some(&mut a2), Some(&mut a3), 2),
            Err(CELL_EINVAL)
        );
        assert_eq!(
            m.get_ext_event2(Some(&mut a1), Some(&mut a2), Some(&mut a3), 99),
            Err(CELL_EINVAL)
        );
    }

    #[test]
    fn get_ext_event2_returns_eagain_on_success() {
        let mut m = SysSm::new();
        let mut a1 = 0u64;
        let mut a2 = 0u64;
        let mut a3 = 0u64;
        // Both a4=0 and a4=1 should succeed (then return EAGAIN as "no event").
        for a4 in [0u64, 1u64] {
            let r = m.get_ext_event2(Some(&mut a1), Some(&mut a2), Some(&mut a3), a4);
            assert_eq!(r, Err(CELL_EAGAIN));
        }
    }

    #[test]
    fn shutdown_no_root_returns_no_permission() {
        let mut m = SysSm::new();
        assert_eq!(m.shutdown(0x100, false), ShutdownOutcome::NoPermission);
    }

    #[test]
    fn shutdown_app_shutdown_ops() {
        let mut m = SysSm::new();
        assert_eq!(
            m.shutdown(0x100, true),
            ShutdownOutcome::Action(ShutdownAction::AppShutdown)
        );
        assert_eq!(
            m.shutdown(0x1100, true),
            ShutdownOutcome::Action(ShutdownAction::AppShutdown)
        );
    }

    #[test]
    fn shutdown_app_reboot_ops() {
        let mut m = SysSm::new();
        assert_eq!(
            m.shutdown(0x200, true),
            ShutdownOutcome::Action(ShutdownAction::AppReboot)
        );
        assert_eq!(
            m.shutdown(0x1200, true),
            ShutdownOutcome::Action(ShutdownAction::AppReboot)
        );
    }

    #[test]
    fn shutdown_lpar_ops_unsupported() {
        let mut m = SysSm::new();
        assert_eq!(m.shutdown(0x8201, true), ShutdownOutcome::Unsupported);
        assert_eq!(m.shutdown(0x8202, true), ShutdownOutcome::Unsupported);
        assert_eq!(m.shutdown(0x8204, true), ShutdownOutcome::Unsupported);
    }

    #[test]
    fn shutdown_unknown_op_invalid() {
        let mut m = SysSm::new();
        assert_eq!(m.shutdown(0xDEAD, true), ShutdownOutcome::Invalid);
        assert_eq!(m.shutdown(0, true), ShutdownOutcome::Invalid);
    }

    #[test]
    fn set_shop_mode_persists() {
        let mut m = SysSm::new();
        m.set_shop_mode(0).unwrap();
        assert_eq!(m.shop_mode, 0);
        m.set_shop_mode(42).unwrap();
        assert_eq!(m.shop_mode, 42);
    }

    #[test]
    fn control_led_writes_state() {
        let mut m = SysSm::new();
        m.control_led(2, 0xFF).unwrap();
        assert_eq!(m.led_states[2], 0xFF);
        m.control_led(2, 0x00).unwrap();
        assert_eq!(m.led_states[2], 0x00);
        // Out-of-range led id is no-op (not error).
        m.control_led(200, 0xAA).unwrap();
    }

    #[test]
    fn ring_buzzer_accumulates_packets() {
        let mut m = SysSm::new();
        m.ring_buzzer(10, 0, 0).unwrap();
        m.ring_buzzer(20, 0, 0).unwrap();
        assert_eq!(m.buzzer_packets, 30);
    }

    #[test]
    fn full_sm_lifecycle_smoke() {
        let mut m = SysSm::new();
        // Boot-time: query params
        let mut a = 0u8;
        let mut b = 0u8;
        let mut c = 0u32;
        let mut d = 0u64;
        m.get_params(Some(&mut a), Some(&mut b), Some(&mut c), Some(&mut d))
            .unwrap();
        assert_eq!((c, d), (0x200, 7));

        // Polling: no event
        let mut a1 = 0u64;
        let mut a2 = 0u64;
        let mut a3 = 0u64;
        assert_eq!(
            m.get_ext_event2(Some(&mut a1), Some(&mut a2), Some(&mut a3), 0),
            Err(CELL_EAGAIN)
        );

        // Shop mode + LED + buzzer
        m.set_shop_mode(1).unwrap();
        m.control_led(0, 1).unwrap();
        m.ring_buzzer(100, 0, 0).unwrap();

        // Shutdown
        let outcome = m.shutdown(0x100, true);
        assert_eq!(outcome, ShutdownOutcome::Action(ShutdownAction::AppShutdown));
    }
}
