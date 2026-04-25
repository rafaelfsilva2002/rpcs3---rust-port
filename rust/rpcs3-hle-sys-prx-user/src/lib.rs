//! `rpcs3-hle-sys-prx-user` — PS3 PRX loader user-mode helpers.
//!
//! Ports `rpcs3/Emu/Cell/Modules/sys_prx_.cpp`.  This is the user-mode
//! shim that games link against — each function acquires
//! `g_ppu_prx_lwm` (a process-wide light-weight mutex) and delegates
//! to the underlying `_sys_prx_*` syscall.  The observable surface the
//! game sees is: argument validation + `sys_prx_start_stop_module_option_t`
//! builder + the `cmd=1 → entry → cmd=2 + res` state machine driven by
//! `entryx` in sys_prx_.cpp:19-34.
//!
//! ## Entry points covered
//!
//! | Name                                              | Validation                            |
//! |---------------------------------------------------|---------------------------------------|
//! | `sys_prx_load_module{,_by_fd}`                    | delegate                              |
//! | `sys_prx_load_module_on_memcontainer{,_by_fd}`    | delegate                              |
//! | `sys_prx_load_module_list{,_on_memcontainer}`     | delegate + `convert_path_list`        |
//! | `sys_prx_start_module` / `sys_prx_stop_module`    | `result` ptr null → `CELL_EINVAL`     |
//! | `sys_prx_unload_module`                           | delegate                              |
//! | `sys_prx_register_library` / `unregister_library` | delegate                              |
//! | `sys_prx_get_module_list`                         | `info` / `info.idlist` null → EINVAL  |
//! | `sys_prx_get_module_info`                         | `info` null → EINVAL                  |
//! | `sys_prx_get_module_id_by_name`                   | `flags != 0` or `pOpt != 0` → EINVAL |
//! | `sys_prx_get_module_id_by_address`                | delegate                              |
//! | `sys_prx_exitspawn_with_level`                    | stub → `CELL_OK`                      |
//! | `sys_prx_get_my_module_id`                        | delegate with LR                      |

extern crate alloc;

use alloc::vec::Vec;
use rpcs3_emu_types::CellError;

// =====================================================================
// Error codes
// =====================================================================

/// `CELL_EINVAL`.
pub const CELL_EINVAL: CellError = CellError(0x8001_0002);

// =====================================================================
// Constants — byte-exact with sys_prx_.cpp
// =====================================================================

/// `opt->cmd = 1` in sys_prx_.cpp:103 / 136 — start/stop "prepare" phase.
pub const OPTION_CMD_PREPARE: u32 = 1;

/// `opt->cmd = 2` in sys_prx_.cpp:115 / 148 — start/stop "finalize" phase.
pub const OPTION_CMD_FINALIZE: u32 = 2;

/// Sentinel the firmware writes to `opt.entry2` pre-call (`set(-1)`).  In
/// PPU's 32-bit pointer world, `-1` sign-extends to `0xFFFF_FFFF`.
pub const OPTION_ENTRY2_NONE: u32 = 0xFFFF_FFFF;

/// `sys_prx_get_module_list` internally calls `_sys_prx_get_module_list`
/// with `opt_index = 2` — see sys_prx_.cpp:201.
pub const GET_MODULE_LIST_OPT_INDEX: i32 = 2;

// =====================================================================
// Option blocks
// =====================================================================

/// Mirror of `sys_prx_start_stop_module_option_t`.  The firmware
/// populates it in-place and the caller's `entryx` routine consumes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StartStopOption {
    pub size: u64,
    pub cmd: u32,
    /// `opt.entry` — primary PRX entry point (may be `0xFFFF_FFFF` = none).
    pub entry: u32,
    /// `opt.entry2` — secondary entry point that super-cedes `entry`
    /// when set.  `OPTION_ENTRY2_NONE` means "don't use".
    pub entry2: u32,
    /// `opt.res` — populated by the caller after `entryx` runs; mirrors
    /// `s32` in the C++ struct.
    pub res: i32,
}

impl StartStopOption {
    /// Build the "prepare" option block the firmware hands to
    /// `_sys_prx_start_module` before any game code runs.  The caller
    /// supplies the size (`opt.size()` in C++); we preserve it verbatim.
    #[must_use]
    pub fn prepare(size: u64) -> Self {
        Self {
            size,
            cmd: OPTION_CMD_PREPARE,
            entry: 0,
            entry2: OPTION_ENTRY2_NONE,
            res: 0,
        }
    }

    /// Upgrade the option block to the "finalize" phase.  Corresponds
    /// to sys_prx_.cpp:115-116 (start) / 148-149 (stop):
    /// ```cpp
    /// opt->cmd = 2;
    /// opt->res = *result;
    /// ```
    pub fn finalize(&mut self, result: i32) {
        self.cmd = OPTION_CMD_FINALIZE;
        self.res = result;
    }
}

/// Mirror of `sys_prx_get_module_list_t` (the caller-visible struct).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GetModuleList {
    pub max: u32,
    pub count: u32,
    /// `idlist` guest pointer — `0` models `vm::null`.
    pub idlist: u32,
}

impl GetModuleList {
    /// Build the internal option the firmware hands to the syscall —
    /// see sys_prx_.cpp:193-199.
    #[must_use]
    pub fn build_option(&self, size: u64) -> GetModuleListOption {
        GetModuleListOption {
            size,
            max: self.max,
            count: 0,
            idlist: self.idlist,
            unk: 0,
        }
    }

    /// Consume the syscall out-params back into the caller-visible
    /// struct (sys_prx_.cpp:203-204).
    pub fn apply_option(&mut self, opt: &GetModuleListOption) {
        self.max = opt.max;
        self.count = opt.count;
    }
}

/// Mirror of `sys_prx_get_module_list_option_t`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GetModuleListOption {
    pub size: u64,
    pub max: u32,
    pub count: u32,
    pub idlist: u32,
    pub unk: u32,
}

// =====================================================================
// entryx — the start/stop entry-selection logic
// =====================================================================

/// Decision produced by [`entryx`] — which of the two PRX entry points
/// is invoked (if any).  Mirrors the C++ cascade in sys_prx_.cpp:19-34.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryDecision {
    /// `opt->entry2` was set — call it with `(entry, args, argp)`.
    Entry2 { entry: u32, args: u32, argp: u32 },
    /// `opt->entry2` was `OPTION_ENTRY2_NONE` but `opt->entry` was set —
    /// call it with `(args, argp)`.
    Entry { args: u32, argp: u32 },
    /// Neither entry point was set; `*res` becomes `0`.
    None,
}

/// Port of the C++ `entryx` helper.  Decides which PRX entry to invoke
/// based on the option block's `entry` / `entry2` fields.
#[must_use]
pub fn entryx(opt: &StartStopOption, args: u32, argp: u32) -> EntryDecision {
    if opt.entry2 != OPTION_ENTRY2_NONE {
        return EntryDecision::Entry2 { entry: opt.entry, args, argp };
    }
    if opt.entry != 0 {
        return EntryDecision::Entry { args, argp };
    }
    EntryDecision::None
}

// =====================================================================
// Argument validators
// =====================================================================

/// Port of the `start_module` / `stop_module` pre-call check.
///
/// # Errors
/// [`CELL_EINVAL`] if `result_valid` is false (null `result` pointer).
pub fn validate_start_stop_module(result_valid: bool) -> Result<(), CellError> {
    if !result_valid {
        return Err(CELL_EINVAL);
    }
    Ok(())
}

/// Port of `sys_prx_get_module_list`'s pre-call check.
///
/// # Errors
/// [`CELL_EINVAL`] if `info_valid` is false, or if the struct's internal
/// `idlist` pointer is null.
pub fn validate_get_module_list(
    info_valid: bool,
    info_idlist_valid: bool,
) -> Result<(), CellError> {
    if !info_valid || !info_idlist_valid {
        return Err(CELL_EINVAL);
    }
    Ok(())
}

/// Port of `sys_prx_get_module_info`'s pre-call check.
///
/// # Errors
/// [`CELL_EINVAL`] if `info_valid` is false.
pub fn validate_get_module_info(info_valid: bool) -> Result<(), CellError> {
    if !info_valid {
        return Err(CELL_EINVAL);
    }
    Ok(())
}

/// Port of `sys_prx_get_module_id_by_name`'s pre-call check —
/// sys_prx_.cpp:231-234 rejects any non-zero `flags` or non-null `pOpt`.
///
/// # Errors
/// [`CELL_EINVAL`] if `flags != 0` or `p_opt != 0`.
pub fn validate_get_module_id_by_name(flags: u64, p_opt: u32) -> Result<(), CellError> {
    if flags != 0 || p_opt != 0 {
        return Err(CELL_EINVAL);
    }
    Ok(())
}

/// Stub for `sys_prx_exitspawn_with_level` (sys_prx_.cpp:248-252).
///
/// # Errors
/// Never errors — `CELL_OK` forever.
#[must_use]
pub fn exitspawn_with_level() -> Result<(), CellError> { Ok(()) }

// =====================================================================
// convert_path_list — 32-bit → 64-bit stack translation
// =====================================================================

/// Port of the `convert_path_list` helper in sys_prx_.cpp:13-16.  In
/// PPU memory, caller's `path_list` is a `vm::cpptr<char>` = `u32`
/// array of guest addresses; the firmware widens each entry to a
/// 64-bit `vm::cptr<char, u64>` for the underlying syscall.  The Rust
/// port returns a freshly allocated `Vec<u64>` so tests can inspect the
/// widened form.
#[must_use]
pub fn convert_path_list(path_list_32: &[u32]) -> Vec<u64> {
    path_list_32.iter().map(|&a| u64::from(a)).collect()
}

// =====================================================================
// Entry-point name registry (17 functions registered with REG_FUNC)
// =====================================================================

/// Names of the 17 functions registered via `REG_FUNC(sysPrxForUser, …)`
/// in sys_prx_.cpp:264-280.  Order matches the C++ source so that
/// cross-checks against the PS3 binary stay stable.
pub const REGISTERED_ENTRY_POINTS: &[&str] = &[
    "sys_prx_load_module",
    "sys_prx_load_module_by_fd",
    "sys_prx_load_module_on_memcontainer",
    "sys_prx_load_module_on_memcontainer_by_fd",
    "sys_prx_load_module_list",
    "sys_prx_load_module_list_on_memcontainer",
    "sys_prx_start_module",
    "sys_prx_stop_module",
    "sys_prx_unload_module",
    "sys_prx_register_library",
    "sys_prx_unregister_library",
    "sys_prx_get_module_list",
    "sys_prx_get_module_info",
    "sys_prx_get_module_id_by_name",
    "sys_prx_get_module_id_by_address",
    "sys_prx_exitspawn_with_level",
    "sys_prx_get_my_module_id",
];

/// Look up whether `name` is one of the registered PRX user-mode
/// entry points.
#[must_use]
pub fn is_registered(name: &str) -> bool {
    REGISTERED_ENTRY_POINTS.contains(&name)
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ---- constants ---------------------------------------------------

    #[test]
    fn cell_einval_byte_exact() {
        assert_eq!(CELL_EINVAL.0, 0x8001_0002);
    }

    #[test]
    fn option_cmd_values_byte_exact() {
        assert_eq!(OPTION_CMD_PREPARE, 1);
        assert_eq!(OPTION_CMD_FINALIZE, 2);
    }

    #[test]
    fn entry2_none_sentinel() {
        // vm::ptr::set(-1) sign-extends to 0xFFFF_FFFF on PPU.
        assert_eq!(OPTION_ENTRY2_NONE, 0xFFFF_FFFF);
    }

    #[test]
    fn get_module_list_opt_index_byte_exact() {
        // sys_prx_.cpp:201 passes 2 as the opt index.
        assert_eq!(GET_MODULE_LIST_OPT_INDEX, 2);
    }

    // ---- StartStopOption builder ------------------------------------

    #[test]
    fn prepare_builds_cmd1_and_entry2_sentinel() {
        let opt = StartStopOption::prepare(32);
        assert_eq!(opt.size, 32);
        assert_eq!(opt.cmd, OPTION_CMD_PREPARE);
        assert_eq!(opt.entry, 0);
        assert_eq!(opt.entry2, OPTION_ENTRY2_NONE);
        assert_eq!(opt.res, 0);
    }

    #[test]
    fn finalize_updates_cmd_and_res() {
        let mut opt = StartStopOption::prepare(32);
        opt.finalize(42);
        assert_eq!(opt.cmd, OPTION_CMD_FINALIZE);
        assert_eq!(opt.res, 42);
        // size and entry fields should stay.
        assert_eq!(opt.size, 32);
    }

    #[test]
    fn finalize_preserves_entry_fields() {
        let mut opt = StartStopOption::prepare(16);
        opt.entry = 0x1000_0000;
        opt.entry2 = 0x2000_0000;
        opt.finalize(-7);
        assert_eq!(opt.entry, 0x1000_0000);
        assert_eq!(opt.entry2, 0x2000_0000);
        assert_eq!(opt.res, -7);
    }

    // ---- entryx decision logic --------------------------------------

    #[test]
    fn entryx_entry2_wins() {
        let mut opt = StartStopOption::prepare(0);
        opt.entry = 0x1234;
        opt.entry2 = 0x5678;
        let decision = entryx(&opt, 42, 0xAAAA);
        assert!(matches!(
            decision,
            EntryDecision::Entry2 { entry: 0x1234, args: 42, argp: 0xAAAA }
        ));
    }

    #[test]
    fn entryx_falls_back_to_entry_when_entry2_none() {
        let mut opt = StartStopOption::prepare(0);
        opt.entry = 0x1234;
        opt.entry2 = OPTION_ENTRY2_NONE;
        let decision = entryx(&opt, 7, 0xBBBB);
        assert!(matches!(
            decision,
            EntryDecision::Entry { args: 7, argp: 0xBBBB }
        ));
    }

    #[test]
    fn entryx_returns_none_when_both_unset() {
        let opt = StartStopOption::prepare(0);
        // entry=0 and entry2=OPTION_ENTRY2_NONE → None.
        assert!(matches!(entryx(&opt, 0, 0), EntryDecision::None));
    }

    #[test]
    fn entryx_entry2_sentinel_is_ignored() {
        let mut opt = StartStopOption::prepare(0);
        opt.entry = 0xBEEF;
        opt.entry2 = 0xFFFF_FFFF; // sentinel
        // entry2 is the sentinel — should fall through to entry.
        assert!(matches!(
            entryx(&opt, 1, 2),
            EntryDecision::Entry { args: 1, argp: 2 }
        ));
    }

    // ---- validate_start_stop_module ---------------------------------

    #[test]
    fn start_stop_null_result_is_einval() {
        assert_eq!(
            validate_start_stop_module(false).unwrap_err(),
            CELL_EINVAL,
        );
    }

    #[test]
    fn start_stop_valid_result_ok() {
        assert!(validate_start_stop_module(true).is_ok());
    }

    // ---- validate_get_module_list -----------------------------------

    #[test]
    fn get_module_list_null_info_is_einval() {
        assert_eq!(
            validate_get_module_list(false, true).unwrap_err(),
            CELL_EINVAL,
        );
    }

    #[test]
    fn get_module_list_null_idlist_is_einval() {
        assert_eq!(
            validate_get_module_list(true, false).unwrap_err(),
            CELL_EINVAL,
        );
    }

    #[test]
    fn get_module_list_both_null_is_einval() {
        assert_eq!(
            validate_get_module_list(false, false).unwrap_err(),
            CELL_EINVAL,
        );
    }

    #[test]
    fn get_module_list_both_valid_ok() {
        assert!(validate_get_module_list(true, true).is_ok());
    }

    // ---- validate_get_module_info -----------------------------------

    #[test]
    fn get_module_info_null_info_is_einval() {
        assert_eq!(validate_get_module_info(false).unwrap_err(), CELL_EINVAL);
    }

    #[test]
    fn get_module_info_valid_ok() {
        assert!(validate_get_module_info(true).is_ok());
    }

    // ---- validate_get_module_id_by_name -----------------------------

    #[test]
    fn id_by_name_flags_nonzero_is_einval() {
        assert_eq!(
            validate_get_module_id_by_name(1, 0).unwrap_err(),
            CELL_EINVAL,
        );
    }

    #[test]
    fn id_by_name_p_opt_nonzero_is_einval() {
        assert_eq!(
            validate_get_module_id_by_name(0, 0x1000).unwrap_err(),
            CELL_EINVAL,
        );
    }

    #[test]
    fn id_by_name_both_nonzero_is_einval() {
        assert_eq!(
            validate_get_module_id_by_name(1, 0x1000).unwrap_err(),
            CELL_EINVAL,
        );
    }

    #[test]
    fn id_by_name_both_zero_ok() {
        assert!(validate_get_module_id_by_name(0, 0).is_ok());
    }

    // ---- exitspawn_with_level ---------------------------------------

    #[test]
    fn exitspawn_is_stub_ok() {
        assert!(exitspawn_with_level().is_ok());
    }

    // ---- convert_path_list ------------------------------------------

    #[test]
    fn convert_path_list_widens_addresses() {
        let input = [0x1000_0000_u32, 0x2000_0000, 0x3000_0000];
        let out = convert_path_list(&input);
        assert_eq!(out, [0x1000_0000_u64, 0x2000_0000, 0x3000_0000]);
    }

    #[test]
    fn convert_path_list_empty_stays_empty() {
        let out = convert_path_list(&[]);
        assert!(out.is_empty());
    }

    // ---- GetModuleList plumbing -------------------------------------

    #[test]
    fn get_module_list_builds_option() {
        let info = GetModuleList { max: 128, count: 99, idlist: 0x4000_0000 };
        let opt = info.build_option(24);
        assert_eq!(opt.size, 24);
        assert_eq!(opt.max, 128);
        assert_eq!(opt.count, 0); // always reset
        assert_eq!(opt.idlist, 0x4000_0000);
        assert_eq!(opt.unk, 0);
    }

    #[test]
    fn get_module_list_apply_option() {
        let mut info = GetModuleList { max: 128, count: 0, idlist: 0x4000_0000 };
        let opt = GetModuleListOption {
            size: 24, max: 64, count: 17, idlist: 0x4000_0000, unk: 0,
        };
        info.apply_option(&opt);
        assert_eq!(info.max, 64);
        assert_eq!(info.count, 17);
    }

    // ---- registry ---------------------------------------------------

    #[test]
    fn registry_has_17_entries() {
        assert_eq!(REGISTERED_ENTRY_POINTS.len(), 17);
    }

    #[test]
    fn registry_contains_all_expected() {
        assert!(is_registered("sys_prx_load_module"));
        assert!(is_registered("sys_prx_start_module"));
        assert!(is_registered("sys_prx_stop_module"));
        assert!(is_registered("sys_prx_exitspawn_with_level"));
        assert!(is_registered("sys_prx_get_my_module_id"));
    }

    #[test]
    fn registry_rejects_unknown() {
        assert!(!is_registered("sys_prx_nonexistent"));
        assert!(!is_registered(""));
        // Case-sensitive.
        assert!(!is_registered("SYS_PRX_LOAD_MODULE"));
    }

    #[test]
    fn registry_has_no_duplicates() {
        let mut seen: Vec<&&str> = REGISTERED_ENTRY_POINTS.iter().collect();
        seen.sort();
        for pair in seen.windows(2) {
            assert_ne!(pair[0], pair[1]);
        }
    }

    // ---- full smoke -------------------------------------------------

    #[test]
    fn full_sys_prx_lifecycle_smoke() {
        // 1. Game calls sys_prx_start_module — with null result it fails.
        assert_eq!(
            validate_start_stop_module(false).unwrap_err(),
            CELL_EINVAL,
        );

        // 2. With a valid result ptr, the firmware builds an option
        // block, invokes the syscall, then executes entryx.
        validate_start_stop_module(true).unwrap();
        let mut opt = StartStopOption::prepare(64);
        opt.entry = 0x1234_0000;
        // entry2 sentinel → fall through to entry.
        let decision = entryx(&opt, 3, 0x5000_0000);
        assert!(matches!(
            decision,
            EntryDecision::Entry { args: 3, argp: 0x5000_0000 }
        ));

        // 3. After the game's entry returns, firmware finalises the
        // option block with the returned result.
        opt.finalize(42);
        assert_eq!(opt.cmd, OPTION_CMD_FINALIZE);
        assert_eq!(opt.res, 42);

        // 4. Later, game queries module list: null idlist → EINVAL.
        assert_eq!(
            validate_get_module_list(true, false).unwrap_err(),
            CELL_EINVAL,
        );
        // Valid → builds opt; `count` starts at 0 and gets filled by
        // the syscall; back to the user struct via apply_option.
        let mut info = GetModuleList { max: 32, count: 0, idlist: 0x4000_0000 };
        let out_opt = GetModuleListOption {
            size: 24, max: 32, count: 17, idlist: 0x4000_0000, unk: 0,
        };
        info.apply_option(&out_opt);
        assert_eq!(info.count, 17);

        // 5. id_by_name rejects non-zero flags / pOpt.
        assert!(validate_get_module_id_by_name(0, 0).is_ok());
        assert_eq!(
            validate_get_module_id_by_name(1, 0).unwrap_err(),
            CELL_EINVAL,
        );

        // 6. exitspawn is forever OK.
        assert!(exitspawn_with_level().is_ok());

        // 7. convert_path_list widens pointers.
        assert_eq!(convert_path_list(&[0x100, 0x200]), [0x100_u64, 0x200]);
    }
}
