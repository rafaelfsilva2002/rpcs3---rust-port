//! Rust port of `rpcs3/Emu/Cell/lv2/sys_gpio.cpp` — PS3 LV2 GPIO get/set
//! syscalls (2 entries, 38 lines C++).
//!
//! Models retail-console behaviour byte-exato: GPIO devices for LED + DIP
//! switch always read 0 (retail consoles don't have these). Set on LED is
//! a no-op (CELL_OK), set on DIP returns CELL_EINVAL (DIP is read-only on
//! retail), unknown device returns CELL_ESRCH.
//!
//! `no_std` + `alloc`. Single dep: `rpcs3-emu-types`.

#![no_std]

extern crate alloc;

use rpcs3_emu_types::CellError;

pub const MODULE_NAME: &str = "sys_gpio";

pub const REGISTERED_ENTRY_POINTS: &[&str] = &["sys_gpio_get", "sys_gpio_set"];

/// Device IDs byte-exatos sys_gpio.h.
pub const SYS_GPIO_LED_DEVICE_ID: u64 = 0x1;
pub const SYS_GPIO_DIP_SWITCH_DEVICE_ID: u64 = 0x2;

/// Errors byte-exatos.
pub const CELL_EINVAL: CellError = CellError(0x8001_0002);
pub const CELL_ESRCH: CellError = CellError(0x8001_0005);
pub const CELL_EFAULT: CellError = CellError(0x8001_000D);

#[derive(Debug, Default)]
pub struct SysGpio {
    pub get_calls: u64,
    pub set_calls: u64,
}

impl SysGpio {
    pub fn new() -> Self {
        Self::default()
    }

    /// `sys_gpio_get(device_id, value)` — retail consoles always return 0.
    /// `value=None` simulates `try_write` failure → CELL_EFAULT.
    pub fn get(
        &mut self,
        device_id: u64,
        value_out: Option<&mut u64>,
    ) -> Result<(), CellError> {
        self.get_calls = self.get_calls.saturating_add(1);
        if device_id != SYS_GPIO_LED_DEVICE_ID && device_id != SYS_GPIO_DIP_SWITCH_DEVICE_ID {
            return Err(CELL_ESRCH);
        }
        match value_out {
            Some(slot) => {
                *slot = 0; // retail HW always 0
                Ok(())
            }
            None => Err(CELL_EFAULT), // try_write failure
        }
    }

    /// `sys_gpio_set(device_id, mask, value)` — LED no-op, DIP rejected,
    /// unknown ESRCH (matches cpp:31-37 switch).
    pub fn set(&mut self, device_id: u64, _mask: u64, _value: u64) -> Result<(), CellError> {
        self.set_calls = self.set_calls.saturating_add(1);
        match device_id {
            SYS_GPIO_LED_DEVICE_ID => Ok(()),
            SYS_GPIO_DIP_SWITCH_DEVICE_ID => Err(CELL_EINVAL),
            _ => Err(CELL_ESRCH),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_and_entries() {
        assert_eq!(MODULE_NAME, "sys_gpio");
        assert_eq!(REGISTERED_ENTRY_POINTS.len(), 2);
    }

    #[test]
    fn device_ids_byte_exact() {
        assert_eq!(SYS_GPIO_LED_DEVICE_ID, 0x1);
        assert_eq!(SYS_GPIO_DIP_SWITCH_DEVICE_ID, 0x2);
    }

    #[test]
    fn get_led_writes_zero() {
        let mut m = SysGpio::new();
        let mut v = 0xABCDu64;
        m.get(SYS_GPIO_LED_DEVICE_ID, Some(&mut v)).unwrap();
        assert_eq!(v, 0);
    }

    #[test]
    fn get_dip_switch_writes_zero() {
        let mut m = SysGpio::new();
        let mut v = 0xABCDu64;
        m.get(SYS_GPIO_DIP_SWITCH_DEVICE_ID, Some(&mut v)).unwrap();
        assert_eq!(v, 0);
    }

    #[test]
    fn get_unknown_device_esrch() {
        let mut m = SysGpio::new();
        let mut v = 0u64;
        assert_eq!(m.get(0xDEAD, Some(&mut v)), Err(CELL_ESRCH));
        assert_eq!(m.get(0, Some(&mut v)), Err(CELL_ESRCH));
        assert_eq!(m.get(3, Some(&mut v)), Err(CELL_ESRCH));
    }

    #[test]
    fn get_with_null_value_efault() {
        let mut m = SysGpio::new();
        assert_eq!(m.get(SYS_GPIO_LED_DEVICE_ID, None), Err(CELL_EFAULT));
    }

    #[test]
    fn set_led_is_noop_ok() {
        let mut m = SysGpio::new();
        m.set(SYS_GPIO_LED_DEVICE_ID, 0xFF, 0xAA).unwrap();
        assert_eq!(m.set_calls, 1);
    }

    #[test]
    fn set_dip_returns_einval() {
        let mut m = SysGpio::new();
        assert_eq!(m.set(SYS_GPIO_DIP_SWITCH_DEVICE_ID, 0, 0), Err(CELL_EINVAL));
    }

    #[test]
    fn set_unknown_device_esrch() {
        let mut m = SysGpio::new();
        assert_eq!(m.set(99, 0, 0), Err(CELL_ESRCH));
    }
}
