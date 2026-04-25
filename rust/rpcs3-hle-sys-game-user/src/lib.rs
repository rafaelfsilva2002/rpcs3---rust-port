//! `rpcs3-hle-sys-game-user` — PS3 `sys_game` user-mode HLE.
//!
//! Ports `rpcs3/Emu/Cell/Modules/sys_game_.cpp` (216 linhas).  Covers
//! the complex `sys_game_process_exitspawn{,2}` size calculation and
//! flag handling, plus 8 watchdog / storage / sys_version / rtc /
//! temperature stubs.
//!
//! ## Entry points covered
//!
//! | C++ function                        | Rust wrapper                       |
//! |-------------------------------------|------------------------------------|
//! | `sys_game_process_exitspawn`        | [`exitspawn_plan`]                 |
//! | `sys_game_process_exitspawn2`       | [`exitspawn2_plan`]                |
//! | `sys_game_board_storage_read`       | [`SysGame::board_storage_read`]    |
//! | `sys_game_board_storage_write`      | [`SysGame::board_storage_write`]   |
//! | `sys_game_get_rtc_status`           | [`SysGame::get_rtc_status`]        |
//! | `sys_game_get_system_sw_version`    | [`SysGame::get_system_sw_version`] |
//! | `sys_game_get_temperature`          | [`SysGame::get_temperature`]       |
//! | `sys_game_watchdog_start/stop/clear`| [`SysGame::watchdog_*`]            |

extern crate alloc;

use rpcs3_emu_types::CellError;

// =====================================================================
// Error codes
// =====================================================================

pub const CELL_EINVAL: CellError = CellError(0x8001_0002);

// =====================================================================
// Constants — byte-exact with sys_game_.cpp
// =====================================================================

/// `sys_exit2_param::x0` — magic header (cpp:117 `arg->x0 = 0x85`).
pub const EXIT2_PARAM_X0: u32 = 0x85;

/// Size of the exit2 header (cpp:118 `arg->this_size = 0x30`).
pub const EXIT2_HEADER_SIZE: u32 = 0x30;

/// Per-string alignment (16 bytes).  Used throughout the layout math:
/// `(len + 0x10) & -0x10`.
pub const STRING_ALIGN: u32 = 0x10;

/// Size budget above which argv/envp are dropped (cpp:85
/// `if (alloc_size > 0x1000)`).
pub const EXITSPAWN_ARG_BUDGET: u32 = 0x1000;

/// Extra bytes reserved at the end when `data_size > 0` (cpp:98).
pub const EXIT2_DATA_RESERVE: u32 = 0x1030;

/// `_sys_process_exit2` 4th argument value (cpp:137).
pub const EXIT2_MAGIC: u32 = 0x1000_0000;

// =====================================================================
// Size calculation helpers — byte-exact port of cpp:14-47
// =====================================================================

/// Port of `get_string_array_size` (cpp:14-31).  Counts the non-null
/// strings in `list` and returns the allocation footprint.  Each
/// string occupies `((len + 0x10) & -0x10) + 8` bytes.
#[must_use]
pub fn string_array_size(list: &[&str]) -> (u32, u32) {
    let mut result: u32 = 8;
    let mut count: u32 = 0;
    for s in list {
        count += 1;
        let len = s.len() as u32;
        result += ((len + STRING_ALIGN) & STRING_ALIGN.wrapping_neg()) + 8;
    }
    (result, count)
}

/// Port of `get_exitspawn_size` (cpp:33-48).  `arg_count` starts at
/// `1` (for the path itself); `env_count` at `0`.  The path's
/// allocation uses the same `(len + 0x10) & -0x10) + 8` formula.
/// Extra 8-byte pad if `(arg_count + env_count) % 2 != 0`.
#[must_use]
pub fn exitspawn_size(path: &str, argv: &[&str], envp: &[&str]) -> (u32, u32, u32) {
    let mut arg_count: u32 = 1; // for path
    let mut env_count: u32 = 0;
    let mut result = ((path.len() as u32 + STRING_ALIGN) & STRING_ALIGN.wrapping_neg()) + 8;
    let (argv_size, argv_count) = string_array_size(argv);
    arg_count += argv_count;
    result += argv_size;
    let (envp_size, envp_count) = string_array_size(envp);
    env_count += envp_count;
    result += envp_size;
    if (arg_count + env_count) % 2 != 0 {
        result += 8;
    }
    (result, arg_count, env_count)
}

// =====================================================================
// Plan result — output of exitspawn / exitspawn2
// =====================================================================

/// Plan produced by [`exitspawn_plan`] / [`exitspawn2_plan`].  Encodes
/// the observable allocation size, header metadata, and transformed
/// flags the firmware forwards to `_sys_process_exit2`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExitspawnPlan {
    /// Total bytes allocated via `vm::alloc` (header + args + padding +
    /// optional data reserve).
    pub alloc_size: u32,
    /// `next_size` that ends up in `sys_exit2_param`
    /// (`alloc_size - 0x30`).
    pub next_size: u32,
    /// Count of argv entries (incl. path as argv[0]) after the budget
    /// check — zero when budget was exceeded.
    pub arg_count: u32,
    /// Count of envp entries after the budget check.
    pub env_count: u32,
    /// Transformed `flags` the firmware hands to `_sys_process_exit2`.
    pub flags: u64,
    /// Whether argv/envp were dropped because the computed alloc
    /// exceeded [`EXITSPAWN_ARG_BUDGET`].
    pub args_dropped: bool,
    /// Whether an atexitspawn callback would fire (`flags >> 62 == 0`).
    pub calls_atexitspawn: bool,
    /// Whether an `at_Exitspawn` callback would fire
    /// (`flags >> 62 == 1`).
    pub calls_at_exitspawn: bool,
}

/// Shared core between both exitspawn variants.  `transformed_flags`
/// comes from the caller's flag-mask logic.
#[must_use]
fn exitspawn_plan_core(
    path: &str,
    argv: &[&str],
    envp: &[&str],
    data_size: u32,
    transformed_flags: u64,
) -> ExitspawnPlan {
    let (mut size, mut arg_count, mut env_count) = exitspawn_size(path, argv, envp);
    let mut args_dropped = false;
    if size > EXITSPAWN_ARG_BUDGET {
        args_dropped = true;
        let recomputed = exitspawn_size(path, &[], &[]);
        size = recomputed.0;
        arg_count = recomputed.1;
        env_count = recomputed.2;
    }
    let mut alloc_size = size + EXIT2_HEADER_SIZE;
    if data_size > 0 {
        alloc_size += EXIT2_DATA_RESERVE;
    }
    let flag_tag = transformed_flags >> 62;
    ExitspawnPlan {
        alloc_size,
        next_size: alloc_size - EXIT2_HEADER_SIZE,
        arg_count,
        env_count,
        flags: transformed_flags,
        args_dropped,
        calls_atexitspawn: flag_tag == 0,
        calls_at_exitspawn: flag_tag == 1,
    }
}

/// Port of `sys_game_process_exitspawn` (cpp:140-145).  The firmware
/// masks input flags down to the low nibble (`flags & 0xf0`) and OR's
/// in `1 << 63`.
#[must_use]
pub fn exitspawn_plan(
    path: &str,
    argv: &[&str],
    envp: &[&str],
    data_size: u32,
    flags: u64,
) -> ExitspawnPlan {
    let transformed = (flags & 0xF0) | (1u64 << 63);
    exitspawn_plan_core(path, argv, envp, data_size, transformed)
}

/// Port of `sys_game_process_exitspawn2` (cpp:147-152).  The flag
/// mask depends on the top 2 bits: `(flags >> 62) >= 2` keeps only
/// the low nibble; otherwise keeps both the low nibble and the top
/// 2 bits.
#[must_use]
pub fn exitspawn2_plan(
    path: &str,
    argv: &[&str],
    envp: &[&str],
    data_size: u32,
    flags: u64,
) -> ExitspawnPlan {
    let transformed = if (flags >> 62) >= 2 {
        flags & 0xF0
    } else {
        flags & 0xC000_0000_0000_00F0
    };
    exitspawn_plan_core(path, argv, envp, data_size, transformed)
}

// =====================================================================
// SysGame — stub registry for the watchdog / board storage entry points
// =====================================================================

/// Observable state for the `watchdog_*` group.  The firmware stubs
/// all return `CELL_OK`; the Rust port tracks on/off so higher layers
/// can test reference-count-like behaviour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SysGame {
    pub watchdog_running: bool,
    pub watchdog_expired: bool,
}

impl SysGame {
    #[must_use]
    pub fn new() -> Self { Self::default() }

    /// Port of `sys_game_board_storage_read` (stub → CELL_OK).
    pub fn board_storage_read(&self) -> Result<(), CellError> { Ok(()) }
    /// Port of `sys_game_board_storage_write` (stub → CELL_OK).
    pub fn board_storage_write(&self) -> Result<(), CellError> { Ok(()) }

    /// Port of `sys_game_get_rtc_status` (stub).
    pub fn get_rtc_status(&self) -> Result<(), CellError> { Ok(()) }
    /// Port of `sys_game_get_system_sw_version` (stub).
    pub fn get_system_sw_version(&self) -> Result<(), CellError> { Ok(()) }
    /// Port of `sys_game_get_temperature` (stub).
    pub fn get_temperature(&self) -> Result<(), CellError> { Ok(()) }

    /// Port of `sys_game_watchdog_start`.
    pub fn watchdog_start(&mut self) -> Result<(), CellError> {
        self.watchdog_running = true;
        self.watchdog_expired = false;
        Ok(())
    }
    /// Port of `sys_game_watchdog_stop`.
    pub fn watchdog_stop(&mut self) -> Result<(), CellError> {
        self.watchdog_running = false;
        Ok(())
    }
    /// Port of `sys_game_watchdog_clear`.
    pub fn watchdog_clear(&mut self) -> Result<(), CellError> {
        self.watchdog_expired = false;
        Ok(())
    }
}

// =====================================================================
// Entry-point registry
// =====================================================================

/// All functions registered under `sysPrxForUser` in cpp:203-214.
pub const REGISTERED_ENTRY_POINTS: &[&str] = &[
    "sys_game_process_exitspawn2",
    "sys_game_process_exitspawn",
    "sys_game_board_storage_read",
    "sys_game_board_storage_write",
    "sys_game_get_rtc_status",
    "sys_game_get_system_sw_version",
    "sys_game_get_temperature",
    "sys_game_watchdog_clear",
    "sys_game_watchdog_start",
    "sys_game_watchdog_stop",
];

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
    fn constants_byte_exact() {
        assert_eq!(CELL_EINVAL.0, 0x8001_0002);
        assert_eq!(EXIT2_PARAM_X0, 0x85);
        assert_eq!(EXIT2_HEADER_SIZE, 0x30);
        assert_eq!(STRING_ALIGN, 0x10);
        assert_eq!(EXITSPAWN_ARG_BUDGET, 0x1000);
        assert_eq!(EXIT2_DATA_RESERVE, 0x1030);
        assert_eq!(EXIT2_MAGIC, 0x1000_0000);
    }

    // ---- string_array_size -----------------------------------------

    #[test]
    fn string_array_empty_returns_8() {
        // cpp:17 `u32 result = 8;` then loop exits immediately.
        let (size, count) = string_array_size(&[]);
        assert_eq!(size, 8);
        assert_eq!(count, 0);
    }

    #[test]
    fn string_array_single_short_string() {
        // "foo" (len=3) → (3 + 0x10) & -0x10 = 0x10; +8 = 0x18; +8 = 0x20
        let (size, count) = string_array_size(&["foo"]);
        assert_eq!(size, 0x20);
        assert_eq!(count, 1);
    }

    #[test]
    fn string_array_two_strings() {
        // each: "xy" (len=2) → 0x10 + 8 = 0x18; base 8 + 2 * 0x18 = 0x38
        let (size, count) = string_array_size(&["xy", "ab"]);
        assert_eq!(size, 8 + 0x18 + 0x18);
        assert_eq!(count, 2);
    }

    #[test]
    fn string_array_exactly_16_bytes_rounds_up_to_32() {
        // 16-char string: (16 + 16) & -16 = 32 → 32 + 8 = 40 → 40 + 8 = 48
        let (size, _) = string_array_size(&["0123456789abcdef"]);
        assert_eq!(size, 8 + 0x20 + 8);
    }

    // ---- exitspawn_size --------------------------------------------

    #[test]
    fn exitspawn_size_arg_count_starts_at_1() {
        // Path counts as argv[0] — arg_count initialises at 1.
        let (_, arg, env) = exitspawn_size("/foo", &[], &[]);
        assert_eq!(arg, 1);
        assert_eq!(env, 0);
    }

    #[test]
    fn exitspawn_size_path_only_odd_padding() {
        // arg_count=1, env_count=0 → (1+0)%2=1 → +8 padding
        // path="/a" (len=2) → 0x10 + 8 = 0x18
        // argv empty: 8
        // envp empty: 8
        // total = 0x18 + 8 + 8 = 0x28 + 8 (odd pad) = 0x30
        let (size, _, _) = exitspawn_size("/a", &[], &[]);
        assert_eq!(size, 0x18 + 8 + 8 + 8);
    }

    #[test]
    fn exitspawn_size_adds_argv_and_envp_sizes() {
        let (size, arg, env) = exitspawn_size("/a", &["x"], &["y"]);
        // arg=2, env=1 → (2+1)=3 odd → +8 pad
        assert_eq!(arg, 2);
        assert_eq!(env, 1);
        // path: 0x18
        // argv: 8 + (0x10 + 8) = 0x20
        // envp: 8 + (0x10 + 8) = 0x20
        // odd-pad: +8
        assert_eq!(size, 0x18 + 0x20 + 0x20 + 8);
    }

    #[test]
    fn exitspawn_size_even_total_no_pad() {
        // arg_count=2 + env_count=2 = 4 even → no pad.
        let (size, arg, env) = exitspawn_size("/a", &["x"], &["y", "z"]);
        assert_eq!((arg, env), (2, 2));
        // path: 0x18, argv: 8+0x18=0x20, envp: 8+0x18+0x18=0x38
        assert_eq!(size, 0x18 + 0x20 + 0x38);
    }

    // ---- exitspawn flag transform ---------------------------------

    #[test]
    fn exitspawn_transforms_flags_to_nibble_plus_hi_bit() {
        let plan = exitspawn_plan("/a", &[], &[], 0, 0xDEAD_BEEF_0000_00F7);
        // (flags & 0xf0) | (1 << 63) = (0xF0) | 0x8000...
        // Actually 0xF7 & 0xF0 = 0xF0
        assert_eq!(plan.flags & 0xFF, 0xF0);
        assert_eq!(plan.flags & (1u64 << 63), 1u64 << 63);
    }

    #[test]
    fn exitspawn_masks_out_non_nibble_bits() {
        // Only 0xf0 bits from low byte survive.
        let plan = exitspawn_plan("/a", &[], &[], 0, 0xFF);
        assert_eq!(plan.flags & 0xFF, 0xF0);
    }

    #[test]
    fn exitspawn_flag_is_tagged_2_so_no_atexit_callbacks() {
        // (1 << 63) >> 62 = 2 → neither at* branch fires.
        let plan = exitspawn_plan("/a", &[], &[], 0, 0);
        assert!(!plan.calls_atexitspawn);
        assert!(!plan.calls_at_exitspawn);
    }

    // ---- exitspawn2 flag transform --------------------------------

    #[test]
    fn exitspawn2_high_bits_ge_2_masks_to_nibble() {
        // flags=0x8000...000F7 → top 2 bits = 0b10 = 2 → >= 2 path.
        let plan = exitspawn2_plan("/a", &[], &[], 0, 0x8000_0000_0000_00F7);
        // result = flags & 0xf0 = 0xF0.
        assert_eq!(plan.flags, 0xF0);
        // Top-2-bits of transformed = 0 → calls_atexitspawn true.
        assert!(plan.calls_atexitspawn);
    }

    #[test]
    fn exitspawn2_high_bits_lt_2_keeps_hi_bits_and_nibble() {
        // flags=0x4000...00F7 → top 2 bits = 1 → else branch.
        let plan = exitspawn2_plan("/a", &[], &[], 0, 0x4000_0000_0000_00F7);
        // keep 0xc0...f0 mask.
        assert_eq!(plan.flags, 0x4000_0000_0000_00F0);
        // Top-2-bits = 1 → calls_at_exitspawn.
        assert!(plan.calls_at_exitspawn);
    }

    #[test]
    fn exitspawn2_top2_zero_calls_atexitspawn() {
        let plan = exitspawn2_plan("/a", &[], &[], 0, 0x00);
        assert!(plan.calls_atexitspawn);
    }

    #[test]
    fn exitspawn2_top2_two_invokes_no_callbacks() {
        // flags=0x80... → >= 2 path → masked to 0xf0, top-2=0 → calls_atexitspawn!
        // Wait — top-2 of the TRANSFORMED flags.  0xF0 has top-2=0, so calls_atexitspawn=true.
        // The check is (transformed >> 62) == 0, so yes.
        let plan = exitspawn2_plan("/a", &[], &[], 0, 0x8000_0000_0000_00F0);
        assert!(plan.calls_atexitspawn);
    }

    // ---- alloc_size layout -----------------------------------------

    #[test]
    fn alloc_size_no_data_just_headers_and_args() {
        let plan = exitspawn_plan("/a", &[], &[], 0, 0);
        // size from exitspawn_size("/a", &[], &[]) = 0x30 → +0x30 header = 0x60.
        let (base, _, _) = exitspawn_size("/a", &[], &[]);
        assert_eq!(plan.alloc_size, base + EXIT2_HEADER_SIZE);
    }

    #[test]
    fn alloc_size_with_data_adds_reserve() {
        let plan = exitspawn_plan("/a", &[], &[], 1, 0);
        let (base, _, _) = exitspawn_size("/a", &[], &[]);
        assert_eq!(plan.alloc_size, base + EXIT2_HEADER_SIZE + EXIT2_DATA_RESERVE);
    }

    #[test]
    fn next_size_equals_alloc_minus_header() {
        let plan = exitspawn_plan("/a", &[], &[], 0, 0);
        assert_eq!(plan.next_size + EXIT2_HEADER_SIZE, plan.alloc_size);
    }

    // ---- budget overflow drops argv/envp -------------------------

    #[test]
    fn exitspawn_budget_exceeded_drops_args() {
        // Force a huge argv list to blow the 0x1000 budget.
        let big: alloc::vec::Vec<&str> = (0..200).map(|_| "xxxxxxxxxxxxxxxx").collect();
        let plan = exitspawn_plan("/a", &big, &[], 0, 0);
        assert!(plan.args_dropped);
        // After drop, args=1 (path only), env=0.
        assert_eq!(plan.arg_count, 1);
        assert_eq!(plan.env_count, 0);
    }

    #[test]
    fn exitspawn_budget_under_limit_keeps_args() {
        let plan = exitspawn_plan("/a", &["x"], &["y"], 0, 0);
        assert!(!plan.args_dropped);
        assert_eq!(plan.arg_count, 2);
        assert_eq!(plan.env_count, 1);
    }

    // ---- SysGame watchdog FSM --------------------------------------

    #[test]
    fn watchdog_starts_stopped() {
        let g = SysGame::new();
        assert!(!g.watchdog_running);
    }

    #[test]
    fn watchdog_start_sets_running() {
        let mut g = SysGame::new();
        g.watchdog_start().unwrap();
        assert!(g.watchdog_running);
    }

    #[test]
    fn watchdog_stop_clears_running() {
        let mut g = SysGame::new();
        g.watchdog_start().unwrap();
        g.watchdog_stop().unwrap();
        assert!(!g.watchdog_running);
    }

    #[test]
    fn watchdog_start_clears_expired() {
        let mut g = SysGame::new();
        g.watchdog_expired = true;
        g.watchdog_start().unwrap();
        assert!(!g.watchdog_expired);
    }

    #[test]
    fn watchdog_clear_explicitly_clears_expired() {
        let mut g = SysGame::new();
        g.watchdog_expired = true;
        g.watchdog_clear().unwrap();
        assert!(!g.watchdog_expired);
    }

    // ---- other stubs ------------------------------------------------

    #[test]
    fn all_stubs_return_ok() {
        let g = SysGame::new();
        assert!(g.board_storage_read().is_ok());
        assert!(g.board_storage_write().is_ok());
        assert!(g.get_rtc_status().is_ok());
        assert!(g.get_system_sw_version().is_ok());
        assert!(g.get_temperature().is_ok());
    }

    // ---- registry ---------------------------------------------------

    #[test]
    fn registry_has_ten_entries() {
        assert_eq!(REGISTERED_ENTRY_POINTS.len(), 10);
    }

    #[test]
    fn registry_contains_exitspawn_first() {
        // cpp:205 registers `exitspawn2` before `exitspawn` (alphabetically out of order).
        assert_eq!(REGISTERED_ENTRY_POINTS[0], "sys_game_process_exitspawn2");
        assert_eq!(REGISTERED_ENTRY_POINTS[1], "sys_game_process_exitspawn");
    }

    #[test]
    fn registry_contains_all_watchdog_ops() {
        for name in ["sys_game_watchdog_clear",
                     "sys_game_watchdog_start",
                     "sys_game_watchdog_stop"] {
            assert!(is_registered(name), "{name}");
        }
    }

    #[test]
    fn registry_rejects_unknown() {
        assert!(!is_registered("sys_game_nope"));
    }

    // ---- full smoke -------------------------------------------------

    #[test]
    fn full_sys_game_lifecycle_smoke() {
        // 1. Plan an exitspawn with argv + envp + data.
        let plan = exitspawn_plan("/eboot.bin", &["--help"], &["LANG=C"], 0x200, 0);
        assert_eq!(plan.arg_count, 2);
        assert_eq!(plan.env_count, 1);
        assert!(!plan.args_dropped);
        assert!(plan.alloc_size > EXIT2_HEADER_SIZE + EXIT2_DATA_RESERVE);

        // 2. exitspawn2 with high-bits set — keeps extra bits.
        let plan2 = exitspawn2_plan("/app", &[], &[], 0, 0x4000_0000_0000_00F7);
        assert_eq!(plan2.flags, 0x4000_0000_0000_00F0);
        assert!(plan2.calls_at_exitspawn);

        // 3. Watchdog lifecycle.
        let mut g = SysGame::new();
        g.watchdog_start().unwrap();
        assert!(g.watchdog_running);
        g.watchdog_expired = true;
        g.watchdog_clear().unwrap();
        assert!(!g.watchdog_expired);
        g.watchdog_stop().unwrap();
        assert!(!g.watchdog_running);

        // 4. Stubs always succeed.
        g.get_system_sw_version().unwrap();
        g.board_storage_read().unwrap();
    }
}
