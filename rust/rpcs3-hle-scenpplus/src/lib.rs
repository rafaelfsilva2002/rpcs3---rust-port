//! Rust port of `rpcs3/Emu/Cell/Modules/sceNpPlus.cpp` — PS3 NP Plus
//! subscription check (smallest module yet, 17 lines C++).
//!
//! Single entry point `sceNpManagerIsSP` queries whether the signed-in PSN
//! account has an active PlayStation Plus subscription. Upstream returns a
//! hard-coded `not_an_error(1)` (always SP=true). The C++ TODO comment at
//! cpp:10 notes that PSHome appears to truncate the return to 1 byte —
//! likely treated as a `bool` in callers.
//!
//! Despite being trivial in upstream, this entry shows up in PSN-aware
//! titles (PSHome, store DLC checks, beta gates), so preserving the exact
//! return value matters for behavioral fidelity.
//!
//! `no_std` + `alloc`. Single dep: `rpcs3-emu-types`.

#![no_std]

extern crate alloc;

use rpcs3_emu_types::CellError;

pub const MODULE_NAME: &str = "sceNpPlus";

/// Single FNID registered (cpp:16 `REG_FUNC(sceNpPlus, sceNpManagerIsSP)`).
pub const REGISTERED_ENTRY_POINTS: &[&str] = &["sceNpManagerIsSP"];

/// Hard-coded return value from `sceNpManagerIsSP` matching cpp:11
/// `not_an_error(1)` — caller may treat as `bool` (truncated to 1 byte by
/// PSHome per the C++ TODO comment).
pub const SP_STATUS_TRUE: u32 = 1;

#[derive(Debug, Default)]
pub struct SceNpPlus {
    pub is_sp_calls: u64,
}

impl SceNpPlus {
    pub fn new() -> Self {
        Self::default()
    }

    /// `sceNpManagerIsSP()` — returns `Ok(1)` (always-SP).
    ///
    /// `not_an_error(1)` in C++ means "successful return value of 1 (not an
    /// error code)". The Rust shape returns `Result<u32, CellError>` so the
    /// caller can disambiguate by checking `Ok(value)`.
    pub fn is_sp(&mut self) -> Result<u32, CellError> {
        self.is_sp_calls = self.is_sp_calls.saturating_add(1);
        Ok(SP_STATUS_TRUE)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_and_entry() {
        assert_eq!(MODULE_NAME, "sceNpPlus");
        assert_eq!(REGISTERED_ENTRY_POINTS, &["sceNpManagerIsSP"]);
    }

    #[test]
    fn sp_status_constant_is_one() {
        assert_eq!(SP_STATUS_TRUE, 1);
    }

    #[test]
    fn is_sp_returns_one_always() {
        let mut m = SceNpPlus::new();
        assert_eq!(m.is_sp(), Ok(1));
        assert_eq!(m.is_sp(), Ok(1));
        assert_eq!(m.is_sp(), Ok(1));
    }

    #[test]
    fn counter_tracks_invocations() {
        let mut m = SceNpPlus::new();
        for _ in 0..100 {
            m.is_sp().unwrap();
        }
        assert_eq!(m.is_sp_calls, 100);
    }

    #[test]
    fn return_value_fits_in_one_byte() {
        // Per C++ TODO at cpp:10: PSHome truncates to 1 byte. Confirm value fits.
        let mut m = SceNpPlus::new();
        let v = m.is_sp().unwrap();
        assert!(v <= u8::MAX as u32);
        assert_eq!(v as u8, 1);
    }
}
