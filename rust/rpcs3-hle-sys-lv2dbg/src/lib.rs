//! `rpcs3-hle-sys-lv2dbg` — PS3 LV2 debugger-interface HLE.
//!
//! Ports `rpcs3/Emu/Cell/Modules/sys_lv2dbg.cpp` (316 linhas).  The
//! firmware exposes a large debug API; every entry point in RPCS3 is
//! a `todo` stub that returns `CELL_OK`, but the module ships 47
//! byte-exact error codes plus 35 registered entry points.  The Rust
//! port captures:
//!
//! * All 47 `CELL_LV2DBG_ERROR_*` codes.
//! * A `REGISTERED_ENTRY_POINTS` table in REG_FUNC order.
//! * A tiny state store for the DABR (Data Address Breakpoint Register)
//!   set/get pair — the one observable state the firmware would hold.
//! * A MAT (Memory Access Trace) condition table keyed by address —
//!   lets tests exercise `sys_dbg_mat_{set,get}_condition` roundtrips.

extern crate alloc;

use alloc::vec::Vec;
use rpcs3_emu_types::CellError;

// =====================================================================
// Error codes — byte-exact with sys_lv2dbg.h:19-65
// =====================================================================

pub mod errors {
    use rpcs3_emu_types::CellError;

    pub const DEINVALIDPROCESSID:         CellError = CellError(0x8001_0401);
    pub const DEINVALIDTHREADID:          CellError = CellError(0x8001_0402);
    pub const DEILLEGALREGISTERTYPE:      CellError = CellError(0x8001_0403);
    pub const DEILLEGALREGISTERNUMBER:    CellError = CellError(0x8001_0404);
    pub const DEILLEGALTHREADSTATE:       CellError = CellError(0x8001_0405);
    pub const DEINVALIDEFFECTIVEADDRESS:  CellError = CellError(0x8001_0406);
    pub const DENOTFOUNDPROCESSID:        CellError = CellError(0x8001_0407);
    pub const DENOMEM:                    CellError = CellError(0x8001_0408);
    pub const DEINVALIDARGUMENTS:         CellError = CellError(0x8001_0409);
    pub const DENOTFOUNDFILE:             CellError = CellError(0x8001_040A);
    pub const DEINVALIDFILETYPE:          CellError = CellError(0x8001_040B);
    pub const DENOTFOUNDTHREADID:         CellError = CellError(0x8001_040C);
    pub const DEINVALIDTHREADSTATUS:      CellError = CellError(0x8001_040D);
    pub const DENOAVAILABLEPROCESSID:     CellError = CellError(0x8001_040E);
    pub const DENOTFOUNDEVENTHANDLER:     CellError = CellError(0x8001_040F);
    pub const DESPNOROOM:                 CellError = CellError(0x8001_0410);
    pub const DESPNOTFOUND:               CellError = CellError(0x8001_0411);
    pub const DESPINPROCESS:              CellError = CellError(0x8001_0412);
    pub const DEINVALIDPRIMARYSPUTHREADID: CellError = CellError(0x8001_0413);
    pub const DETHREADSTATEISNOTSTOPPED:  CellError = CellError(0x8001_0414);
    pub const DEINVALIDTHREADTYPE:        CellError = CellError(0x8001_0415);
    pub const DECONTINUEFAILED:           CellError = CellError(0x8001_0416);
    pub const DESTOPFAILED:               CellError = CellError(0x8001_0417);
    pub const DENOEXCEPTION:              CellError = CellError(0x8001_0418);
    pub const DENOMOREEVENTQUE:           CellError = CellError(0x8001_0419);
    pub const DEEVENTQUENOTCREATED:       CellError = CellError(0x8001_041A);
    pub const DEEVENTQUEOVERFLOWED:       CellError = CellError(0x8001_041B);
    pub const DENOTIMPLEMENTED:           CellError = CellError(0x8001_041C);
    pub const DEQUENOTREGISTERED:         CellError = CellError(0x8001_041D);
    pub const DENOMOREEVENTPROCESS:       CellError = CellError(0x8001_041E);
    pub const DEPROCESSNOTREGISTERED:     CellError = CellError(0x8001_041F);
    pub const DEEVENTDISCARDED:           CellError = CellError(0x8001_0420);
    pub const DENOMORESYNCID:             CellError = CellError(0x8001_0421);
    pub const DESYNCIDALREADYADDED:       CellError = CellError(0x8001_0422);
    pub const DESYNCIDNOTFOUND:           CellError = CellError(0x8001_0423);
    pub const DESYNCIDNOTACQUIRED:        CellError = CellError(0x8001_0424);
    pub const DEPROCESSALREADYREGISTERED: CellError = CellError(0x8001_0425);
    pub const DEINVALIDLSADDRESS:         CellError = CellError(0x8001_0426);
    pub const DEINVALIDOPERATION:         CellError = CellError(0x8001_0427);
    pub const DEINVALIDMODULEID:          CellError = CellError(0x8001_0428);
    pub const DEHANDLERALREADYREGISTERED: CellError = CellError(0x8001_0429);
    pub const DEINVALIDHANDLER:           CellError = CellError(0x8001_042A);
    pub const DEHANDLENOTREGISTERED:      CellError = CellError(0x8001_042B);
    pub const DEOPERATIONDENIED:          CellError = CellError(0x8001_042C);
    pub const DEHANDLERNOTINITIALIZED:    CellError = CellError(0x8001_042D);
    pub const DEHANDLERALREADYINITIALIZED: CellError = CellError(0x8001_042E);
    pub const DEILLEGALCOREDUMPPARAMETER: CellError = CellError(0x8001_042F);
}

// =====================================================================
// Stub-registry
// =====================================================================

/// Exactly 35 functions registered under `sys_lv2dbg` in REG_FUNC
/// block cpp:278-314, in source order.
pub const REGISTERED_ENTRY_POINTS: &[&str] = &[
    "sys_dbg_read_ppu_thread_context",
    "sys_dbg_read_spu_thread_context",
    "sys_dbg_read_spu_thread_context2",
    "sys_dbg_set_stacksize_ppu_exception_handler",
    "sys_dbg_initialize_ppu_exception_handler",
    "sys_dbg_finalize_ppu_exception_handler",
    "sys_dbg_register_ppu_exception_handler",
    "sys_dbg_unregister_ppu_exception_handler",
    "sys_dbg_signal_to_ppu_exception_handler",
    "sys_dbg_get_mutex_information",
    "sys_dbg_get_cond_information",
    "sys_dbg_get_rwlock_information",
    "sys_dbg_get_event_queue_information",
    "sys_dbg_get_semaphore_information",
    "sys_dbg_get_lwmutex_information",
    "sys_dbg_get_lwcond_information",
    "sys_dbg_get_event_flag_information",
    "sys_dbg_get_ppu_thread_ids",
    "sys_dbg_get_spu_thread_group_ids",
    "sys_dbg_get_spu_thread_ids",
    "sys_dbg_get_ppu_thread_name",
    "sys_dbg_get_spu_thread_name",
    "sys_dbg_get_spu_thread_group_name",
    "sys_dbg_get_ppu_thread_status",
    "sys_dbg_get_spu_thread_group_status",
    "sys_dbg_enable_floating_point_enabled_exception",
    "sys_dbg_disable_floating_point_enabled_exception",
    "sys_dbg_vm_get_page_information",
    "sys_dbg_set_address_to_dabr",
    "sys_dbg_get_address_from_dabr",
    "sys_dbg_signal_to_coredump_handler",
    "sys_dbg_mat_set_condition",
    "sys_dbg_mat_get_condition",
    "sys_dbg_get_coredump_params",
    "sys_dbg_set_mask_to_ppu_exception_handler",
];

#[must_use]
pub fn is_registered(name: &str) -> bool {
    REGISTERED_ENTRY_POINTS.contains(&name)
}

// =====================================================================
// Observable state — DABR + MAT conditions + exception-handler refcount
// =====================================================================

/// Tracks the few pieces of observable state the firmware module
/// carries across calls: DABR value + ctrl flag, MAT condition table,
/// PPU exception-handler stack size + priority.
#[derive(Debug, Clone, Default)]
pub struct Lv2Dbg {
    pub dabr_addr: u64,
    pub dabr_ctrl_flag: u64,
    pub mat_conditions: Vec<(u32, u64)>,
    pub ppu_exc_stack_size: u32,
    pub ppu_exc_priority: i32,
    pub ppu_exc_initialized: bool,
    pub ppu_exc_handler: Option<u32>,
    pub ppu_exc_mask: u64,
    pub ppu_exc_mask_flags: u64,
}

impl Lv2Dbg {
    #[must_use]
    pub fn new() -> Self { Self::default() }

    /// Port of `sys_dbg_set_address_to_dabr`.
    pub fn set_address_to_dabr(&mut self, addr: u64, ctrl_flag: u64) -> Result<(), CellError> {
        self.dabr_addr = addr;
        self.dabr_ctrl_flag = ctrl_flag;
        Ok(())
    }

    /// Port of `sys_dbg_get_address_from_dabr`.
    #[must_use]
    pub fn get_address_from_dabr(&self) -> (u64, u64) {
        (self.dabr_addr, self.dabr_ctrl_flag)
    }

    /// Port of `sys_dbg_mat_set_condition`.
    pub fn mat_set_condition(&mut self, addr: u32, cond: u64) -> Result<(), CellError> {
        if let Some(slot) = self.mat_conditions.iter_mut().find(|(a, _)| *a == addr) {
            slot.1 = cond;
        } else {
            self.mat_conditions.push((addr, cond));
        }
        Ok(())
    }

    /// Port of `sys_dbg_mat_get_condition`.  Returns `None` if no
    /// condition is set for the address.
    #[must_use]
    pub fn mat_get_condition(&self, addr: u32) -> Option<u64> {
        self.mat_conditions.iter().find(|(a, _)| *a == addr).map(|(_, c)| *c)
    }

    /// Port of `sys_dbg_set_stacksize_ppu_exception_handler`.
    pub fn set_stacksize(&mut self, stacksize: u32) -> Result<(), CellError> {
        self.ppu_exc_stack_size = stacksize;
        Ok(())
    }

    /// Port of `sys_dbg_initialize_ppu_exception_handler`.
    ///
    /// # Errors
    /// * [`errors::DEHANDLERALREADYINITIALIZED`] if called twice
    ///   (firmware-style double-init guard).
    pub fn initialize_ppu_exception_handler(&mut self, prio: i32) -> Result<(), CellError> {
        if self.ppu_exc_initialized {
            return Err(errors::DEHANDLERALREADYINITIALIZED);
        }
        self.ppu_exc_priority = prio;
        self.ppu_exc_initialized = true;
        Ok(())
    }

    /// Port of `sys_dbg_finalize_ppu_exception_handler`.
    ///
    /// # Errors
    /// * [`errors::DEHANDLERNOTINITIALIZED`] if called before init.
    pub fn finalize_ppu_exception_handler(&mut self) -> Result<(), CellError> {
        if !self.ppu_exc_initialized {
            return Err(errors::DEHANDLERNOTINITIALIZED);
        }
        self.ppu_exc_initialized = false;
        self.ppu_exc_handler = None;
        Ok(())
    }

    /// Port of `sys_dbg_register_ppu_exception_handler`.
    ///
    /// # Errors
    /// * [`errors::DEHANDLERALREADYREGISTERED`] if a handler is already
    ///   registered.
    pub fn register_ppu_exception_handler(
        &mut self,
        callback: u32,
        ctrl_flags: u64,
    ) -> Result<(), CellError> {
        if self.ppu_exc_handler.is_some() {
            return Err(errors::DEHANDLERALREADYREGISTERED);
        }
        self.ppu_exc_handler = Some(callback);
        self.ppu_exc_mask_flags = ctrl_flags;
        Ok(())
    }

    /// Port of `sys_dbg_unregister_ppu_exception_handler`.
    ///
    /// # Errors
    /// * [`errors::DEHANDLENOTREGISTERED`] if no handler is registered.
    pub fn unregister_ppu_exception_handler(&mut self) -> Result<(), CellError> {
        if self.ppu_exc_handler.is_none() {
            return Err(errors::DEHANDLENOTREGISTERED);
        }
        self.ppu_exc_handler = None;
        Ok(())
    }

    /// Port of `sys_dbg_set_mask_to_ppu_exception_handler`.
    pub fn set_mask(&mut self, mask: u64, flags: u64) -> Result<(), CellError> {
        self.ppu_exc_mask = mask;
        self.ppu_exc_mask_flags = flags;
        Ok(())
    }
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ---- error codes (sample byte-exact + full count) ---------------

    #[test]
    fn error_codes_first_byte_exact() {
        assert_eq!(errors::DEINVALIDPROCESSID.0,  0x8001_0401);
        assert_eq!(errors::DEINVALIDTHREADID.0,   0x8001_0402);
        assert_eq!(errors::DEILLEGALREGISTERTYPE.0, 0x8001_0403);
    }

    #[test]
    fn error_codes_mid_byte_exact() {
        assert_eq!(errors::DEINVALIDTHREADTYPE.0,   0x8001_0415);
        assert_eq!(errors::DENOEXCEPTION.0,         0x8001_0418);
        assert_eq!(errors::DENOMOREEVENTQUE.0,      0x8001_0419);
    }

    #[test]
    fn error_codes_last_byte_exact() {
        assert_eq!(errors::DEHANDLERALREADYREGISTERED.0, 0x8001_0429);
        assert_eq!(errors::DEHANDLENOTREGISTERED.0,      0x8001_042B);
        assert_eq!(errors::DEHANDLERALREADYINITIALIZED.0, 0x8001_042E);
        assert_eq!(errors::DEILLEGALCOREDUMPPARAMETER.0, 0x8001_042F);
    }

    #[test]
    fn error_codes_all_47_are_distinct() {
        let codes = [
            errors::DEINVALIDPROCESSID,
            errors::DEINVALIDTHREADID,
            errors::DEILLEGALREGISTERTYPE,
            errors::DEILLEGALREGISTERNUMBER,
            errors::DEILLEGALTHREADSTATE,
            errors::DEINVALIDEFFECTIVEADDRESS,
            errors::DENOTFOUNDPROCESSID,
            errors::DENOMEM,
            errors::DEINVALIDARGUMENTS,
            errors::DENOTFOUNDFILE,
            errors::DEINVALIDFILETYPE,
            errors::DENOTFOUNDTHREADID,
            errors::DEINVALIDTHREADSTATUS,
            errors::DENOAVAILABLEPROCESSID,
            errors::DENOTFOUNDEVENTHANDLER,
            errors::DESPNOROOM,
            errors::DESPNOTFOUND,
            errors::DESPINPROCESS,
            errors::DEINVALIDPRIMARYSPUTHREADID,
            errors::DETHREADSTATEISNOTSTOPPED,
            errors::DEINVALIDTHREADTYPE,
            errors::DECONTINUEFAILED,
            errors::DESTOPFAILED,
            errors::DENOEXCEPTION,
            errors::DENOMOREEVENTQUE,
            errors::DEEVENTQUENOTCREATED,
            errors::DEEVENTQUEOVERFLOWED,
            errors::DENOTIMPLEMENTED,
            errors::DEQUENOTREGISTERED,
            errors::DENOMOREEVENTPROCESS,
            errors::DEPROCESSNOTREGISTERED,
            errors::DEEVENTDISCARDED,
            errors::DENOMORESYNCID,
            errors::DESYNCIDALREADYADDED,
            errors::DESYNCIDNOTFOUND,
            errors::DESYNCIDNOTACQUIRED,
            errors::DEPROCESSALREADYREGISTERED,
            errors::DEINVALIDLSADDRESS,
            errors::DEINVALIDOPERATION,
            errors::DEINVALIDMODULEID,
            errors::DEHANDLERALREADYREGISTERED,
            errors::DEINVALIDHANDLER,
            errors::DEHANDLENOTREGISTERED,
            errors::DEOPERATIONDENIED,
            errors::DEHANDLERNOTINITIALIZED,
            errors::DEHANDLERALREADYINITIALIZED,
            errors::DEILLEGALCOREDUMPPARAMETER,
        ];
        assert_eq!(codes.len(), 47);
        let mut sorted: Vec<u32> = codes.iter().map(|c| c.0).collect();
        sorted.sort_unstable();
        for pair in sorted.windows(2) {
            assert_ne!(pair[0], pair[1]);
        }
    }

    #[test]
    fn error_codes_span_contiguous_range() {
        // 0x80010401..=0x8001042F = 47 entries.
        assert_eq!(
            errors::DEILLEGALCOREDUMPPARAMETER.0 - errors::DEINVALIDPROCESSID.0 + 1,
            47,
        );
    }

    // ---- registry ---------------------------------------------------

    #[test]
    fn registry_has_35_entries() {
        assert_eq!(REGISTERED_ENTRY_POINTS.len(), 35);
    }

    #[test]
    fn registry_order_matches_cpp() {
        // First and last entries from cpp:280-314.
        assert_eq!(REGISTERED_ENTRY_POINTS[0], "sys_dbg_read_ppu_thread_context");
        assert_eq!(REGISTERED_ENTRY_POINTS[34], "sys_dbg_set_mask_to_ppu_exception_handler");
    }

    #[test]
    fn registry_contains_information_helpers() {
        for name in ["sys_dbg_get_mutex_information",
                     "sys_dbg_get_cond_information",
                     "sys_dbg_get_semaphore_information",
                     "sys_dbg_get_lwmutex_information",
                     "sys_dbg_get_event_flag_information"] {
            assert!(is_registered(name), "{name}");
        }
    }

    #[test]
    fn registry_rejects_unknown() {
        assert!(!is_registered("sys_dbg_nonexistent"));
    }

    #[test]
    fn registry_no_duplicates() {
        let mut sorted: Vec<&&str> = REGISTERED_ENTRY_POINTS.iter().collect();
        sorted.sort();
        for pair in sorted.windows(2) {
            assert_ne!(pair[0], pair[1]);
        }
    }

    // ---- DABR -------------------------------------------------------

    #[test]
    fn dabr_starts_zero() {
        let d = Lv2Dbg::new();
        assert_eq!(d.get_address_from_dabr(), (0, 0));
    }

    #[test]
    fn dabr_roundtrip() {
        let mut d = Lv2Dbg::new();
        d.set_address_to_dabr(0xDEAD_BEEF_CAFE, 0x3).unwrap();
        assert_eq!(d.get_address_from_dabr(), (0xDEAD_BEEF_CAFE, 0x3));
    }

    #[test]
    fn dabr_overwrite() {
        let mut d = Lv2Dbg::new();
        d.set_address_to_dabr(0x1000, 0x1).unwrap();
        d.set_address_to_dabr(0x2000, 0x2).unwrap();
        assert_eq!(d.get_address_from_dabr(), (0x2000, 0x2));
    }

    // ---- MAT conditions --------------------------------------------

    #[test]
    fn mat_get_unknown_returns_none() {
        let d = Lv2Dbg::new();
        assert_eq!(d.mat_get_condition(0x1000), None);
    }

    #[test]
    fn mat_set_then_get_roundtrips() {
        let mut d = Lv2Dbg::new();
        d.mat_set_condition(0x4000, 0x42).unwrap();
        assert_eq!(d.mat_get_condition(0x4000), Some(0x42));
    }

    #[test]
    fn mat_set_overwrites_existing_address() {
        let mut d = Lv2Dbg::new();
        d.mat_set_condition(0x4000, 0x42).unwrap();
        d.mat_set_condition(0x4000, 0x84).unwrap();
        assert_eq!(d.mat_get_condition(0x4000), Some(0x84));
        // Only one entry for this address.
        assert_eq!(d.mat_conditions.len(), 1);
    }

    #[test]
    fn mat_multiple_addresses_coexist() {
        let mut d = Lv2Dbg::new();
        d.mat_set_condition(0x1000, 0x1).unwrap();
        d.mat_set_condition(0x2000, 0x2).unwrap();
        d.mat_set_condition(0x3000, 0x3).unwrap();
        assert_eq!(d.mat_get_condition(0x1000), Some(0x1));
        assert_eq!(d.mat_get_condition(0x2000), Some(0x2));
        assert_eq!(d.mat_get_condition(0x3000), Some(0x3));
    }

    // ---- PPU exception handler -------------------------------------

    #[test]
    fn initialize_ppu_handler_sets_flag() {
        let mut d = Lv2Dbg::new();
        d.initialize_ppu_exception_handler(512).unwrap();
        assert!(d.ppu_exc_initialized);
        assert_eq!(d.ppu_exc_priority, 512);
    }

    #[test]
    fn initialize_twice_is_already_initialized() {
        let mut d = Lv2Dbg::new();
        d.initialize_ppu_exception_handler(512).unwrap();
        assert_eq!(
            d.initialize_ppu_exception_handler(1000).unwrap_err(),
            errors::DEHANDLERALREADYINITIALIZED,
        );
    }

    #[test]
    fn finalize_without_init_is_not_initialized() {
        let mut d = Lv2Dbg::new();
        assert_eq!(
            d.finalize_ppu_exception_handler().unwrap_err(),
            errors::DEHANDLERNOTINITIALIZED,
        );
    }

    #[test]
    fn finalize_clears_handler() {
        let mut d = Lv2Dbg::new();
        d.initialize_ppu_exception_handler(512).unwrap();
        d.register_ppu_exception_handler(0x1000, 0xF).unwrap();
        d.finalize_ppu_exception_handler().unwrap();
        assert!(!d.ppu_exc_initialized);
        assert_eq!(d.ppu_exc_handler, None);
    }

    #[test]
    fn register_handler_sets_callback() {
        let mut d = Lv2Dbg::new();
        d.register_ppu_exception_handler(0xABCD, 0x3).unwrap();
        assert_eq!(d.ppu_exc_handler, Some(0xABCD));
    }

    #[test]
    fn register_twice_is_already_registered() {
        let mut d = Lv2Dbg::new();
        d.register_ppu_exception_handler(0xABCD, 0x3).unwrap();
        assert_eq!(
            d.register_ppu_exception_handler(0x1234, 0x5).unwrap_err(),
            errors::DEHANDLERALREADYREGISTERED,
        );
    }

    #[test]
    fn unregister_without_register_is_not_registered() {
        let mut d = Lv2Dbg::new();
        assert_eq!(
            d.unregister_ppu_exception_handler().unwrap_err(),
            errors::DEHANDLENOTREGISTERED,
        );
    }

    #[test]
    fn register_unregister_roundtrip() {
        let mut d = Lv2Dbg::new();
        d.register_ppu_exception_handler(0x1234, 0x0).unwrap();
        d.unregister_ppu_exception_handler().unwrap();
        // Re-register after unregister is OK.
        d.register_ppu_exception_handler(0x5678, 0x0).unwrap();
        assert_eq!(d.ppu_exc_handler, Some(0x5678));
    }

    #[test]
    fn set_stacksize_stores_value() {
        let mut d = Lv2Dbg::new();
        d.set_stacksize(0x1000).unwrap();
        assert_eq!(d.ppu_exc_stack_size, 0x1000);
    }

    #[test]
    fn set_mask_stores_both_fields() {
        let mut d = Lv2Dbg::new();
        d.set_mask(0xFF, 0x3).unwrap();
        assert_eq!(d.ppu_exc_mask, 0xFF);
        assert_eq!(d.ppu_exc_mask_flags, 0x3);
    }

    // ---- full smoke -------------------------------------------------

    #[test]
    fn full_lv2dbg_lifecycle_smoke() {
        let mut d = Lv2Dbg::new();

        // 1. Initialize + register an exception handler.
        d.set_stacksize(0x2000).unwrap();
        d.initialize_ppu_exception_handler(512).unwrap();
        d.register_ppu_exception_handler(0x4000_0000, 0xF).unwrap();

        // 2. Arm the DABR.
        d.set_address_to_dabr(0x1234_5678, 0x3).unwrap();
        assert_eq!(d.get_address_from_dabr(), (0x1234_5678, 0x3));

        // 3. Set a couple MAT conditions.
        d.mat_set_condition(0x4000, 0x01).unwrap();
        d.mat_set_condition(0x8000, 0x02).unwrap();
        assert_eq!(d.mat_get_condition(0x4000), Some(0x01));

        // 4. Set mask.
        d.set_mask(0xFF, 0x1).unwrap();

        // 5. Double-register → error.
        assert_eq!(
            d.register_ppu_exception_handler(0x5000_0000, 0x0).unwrap_err(),
            errors::DEHANDLERALREADYREGISTERED,
        );

        // 6. Unregister + register new handler.
        d.unregister_ppu_exception_handler().unwrap();
        d.register_ppu_exception_handler(0x5000_0000, 0x0).unwrap();

        // 7. Finalize clears everything.
        d.finalize_ppu_exception_handler().unwrap();
        assert!(!d.ppu_exc_initialized);
        assert_eq!(d.ppu_exc_handler, None);

        // 8. Finalize again → error.
        assert_eq!(
            d.finalize_ppu_exception_handler().unwrap_err(),
            errors::DEHANDLERNOTINITIALIZED,
        );
    }
}
