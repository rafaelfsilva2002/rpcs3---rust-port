//! `rpcs3-hle-cellspudll` — SPU DLL loader HLE.
//!
//! Ports `rpcs3/Emu/Cell/Modules/cellSpudll.cpp`.  cellSpudll is the
//! minimal firmware layer responsible for loading SPU shared objects
//! (`.so`) into an SPU thread — measuring image size, populating a
//! default handle-config, and serving unresolved-symbol fallbacks.  The
//! real C++ side is mostly stubs (`cellSpudllGetImageSize` is marked
//! `todo` and always returns `CELL_OK`) but the argument validation and
//! default-value table are fully defined and byte-exact.
//!
//! ## Entry points covered
//!
//! | HLE function                               | Rust wrapper                          |
//! |--------------------------------------------|---------------------------------------|
//! | `cellSpudllGetImageSize`                   | [`cell_spudll_get_image_size`]        |
//! | `cellSpudllHandleConfigSetDefaultValues`   | [`cell_spudll_handle_config_set_default_values`] |

use rpcs3_emu_types::CellError;

// =====================================================================
// Error codes — byte-exact with cellSpudll.h:5-14
// =====================================================================

pub mod errors {
    use rpcs3_emu_types::CellError;

    pub const INVAL:        CellError = CellError(0x8041_0602);
    pub const STAT:         CellError = CellError(0x8041_060F);
    pub const ALIGN:        CellError = CellError(0x8041_0610);
    pub const NULL_POINTER: CellError = CellError(0x8041_0611);
    pub const SRCH:         CellError = CellError(0x8041_0605);
    pub const UNDEF:        CellError = CellError(0x8041_0612);
    pub const FATAL:        CellError = CellError(0x8041_0613);
}

// =====================================================================
// CellSpudllHandleConfig — byte-exact layout with cellSpudll.h:16-26
// =====================================================================

/// Default for `numMaxReferred` populated by
/// `cellSpudllHandleConfigSetDefaultValues`.
pub const DEFAULT_NUM_MAX_REFERRED: u32 = 16;

/// Default for `numMaxDepend` populated by
/// `cellSpudllHandleConfigSetDefaultValues`.
pub const DEFAULT_NUM_MAX_DEPEND: u32 = 16;

/// Number of trailing reserved `u32` words in `CellSpudllHandleConfig`.
pub const RESERVED_WORDS: usize = 9;

/// `CellSpudllHandleConfig` — mirror of the 9 leading fields + 9 trailing
/// reserved words, all stored in host-endian for test purposes.  The real
/// layout is big-endian in guest memory; the Rust mirror is used by tests
/// and callers that wrap the guest pointer themselves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HandleConfig {
    pub mode: u32,
    pub dma_tag: u32,
    pub num_max_referred: u32,
    pub num_max_depend: u32,
    pub unresolved_symbol_value_for_func:   u32,
    pub unresolved_symbol_value_for_object: u32,
    pub unresolved_symbol_value_for_other:  u32,
    pub reserved: [u32; RESERVED_WORDS],
}

impl Default for HandleConfig {
    /// A "zero" config — **not** the same as the firmware default;
    /// [`cell_spudll_handle_config_set_default_values`] produces the
    /// observable default layout.  Keep this for fresh allocations.
    fn default() -> Self {
        Self {
            mode: 0,
            dma_tag: 0,
            num_max_referred: 0,
            num_max_depend: 0,
            unresolved_symbol_value_for_func: 0,
            unresolved_symbol_value_for_object: 0,
            unresolved_symbol_value_for_other: 0,
            reserved: [0; RESERVED_WORDS],
        }
    }
}

/// Port of `cellSpudllGetImageSize`.
///
/// The real firmware measures a SPU `.so` image, then writes the result
/// to `*psize`.  The C++ RPCS3 implementation is a stub that leaves
/// `*psize` untouched and returns `CELL_OK` once the null checks pass;
/// higher layers must treat the output as unchanged.
///
/// # Errors
/// * [`errors::NULL_POINTER`] if either `psize` or `so_elf` is null
///   (represented here as `psize_valid = false` / `so_elf_valid = false`).
pub fn cell_spudll_get_image_size(
    psize_valid: bool,
    so_elf_valid: bool,
) -> Result<(), CellError> {
    if !psize_valid || !so_elf_valid {
        return Err(errors::NULL_POINTER);
    }
    Ok(())
}

/// Port of `cellSpudllHandleConfigSetDefaultValues`.
///
/// Populates `config` with the firmware defaults: `mode=0`, `dmaTag=0`,
/// `numMaxReferred=16`, `numMaxDepend=16`, all three unresolved-symbol
/// fallbacks to `vm::null` (represented as `0`), and zero-fills the
/// trailing 9-word reserved region.
///
/// # Errors
/// * [`errors::NULL_POINTER`] if `config` is null.
pub fn cell_spudll_handle_config_set_default_values(
    config: Option<&mut HandleConfig>,
) -> Result<(), CellError> {
    let Some(cfg) = config else {
        return Err(errors::NULL_POINTER);
    };
    cfg.mode = 0;
    cfg.dma_tag = 0;
    cfg.num_max_referred = DEFAULT_NUM_MAX_REFERRED;
    cfg.num_max_depend   = DEFAULT_NUM_MAX_DEPEND;
    cfg.unresolved_symbol_value_for_func   = 0;
    cfg.unresolved_symbol_value_for_object = 0;
    cfg.unresolved_symbol_value_for_other  = 0;
    cfg.reserved = [0; RESERVED_WORDS];
    Ok(())
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_codes_byte_exact() {
        assert_eq!(errors::INVAL.0,        0x8041_0602);
        assert_eq!(errors::STAT.0,         0x8041_060F);
        assert_eq!(errors::ALIGN.0,        0x8041_0610);
        assert_eq!(errors::NULL_POINTER.0, 0x8041_0611);
        assert_eq!(errors::SRCH.0,         0x8041_0605);
        assert_eq!(errors::UNDEF.0,        0x8041_0612);
        assert_eq!(errors::FATAL.0,        0x8041_0613);
    }

    #[test]
    fn default_constants_byte_exact() {
        assert_eq!(DEFAULT_NUM_MAX_REFERRED, 16);
        assert_eq!(DEFAULT_NUM_MAX_DEPEND,   16);
        assert_eq!(RESERVED_WORDS, 9);
    }

    // ---- cellSpudllGetImageSize ------------------------------------

    #[test]
    fn get_image_size_null_psize_is_null_pointer() {
        assert_eq!(
            cell_spudll_get_image_size(false, true).unwrap_err(),
            errors::NULL_POINTER,
        );
    }

    #[test]
    fn get_image_size_null_so_elf_is_null_pointer() {
        assert_eq!(
            cell_spudll_get_image_size(true, false).unwrap_err(),
            errors::NULL_POINTER,
        );
    }

    #[test]
    fn get_image_size_both_null_is_null_pointer() {
        assert_eq!(
            cell_spudll_get_image_size(false, false).unwrap_err(),
            errors::NULL_POINTER,
        );
    }

    #[test]
    fn get_image_size_happy_path_returns_ok() {
        // C++ stub returns CELL_OK once both pointers are non-null.
        assert!(cell_spudll_get_image_size(true, true).is_ok());
    }

    // ---- cellSpudllHandleConfigSetDefaultValues --------------------

    #[test]
    fn set_default_values_null_config_is_null_pointer() {
        assert_eq!(
            cell_spudll_handle_config_set_default_values(None).unwrap_err(),
            errors::NULL_POINTER,
        );
    }

    #[test]
    fn set_default_values_writes_mode_zero() {
        let mut cfg = HandleConfig { mode: 0xDEAD, ..HandleConfig::default() };
        cell_spudll_handle_config_set_default_values(Some(&mut cfg)).unwrap();
        assert_eq!(cfg.mode, 0);
    }

    #[test]
    fn set_default_values_writes_dma_tag_zero() {
        let mut cfg = HandleConfig { dma_tag: 0xBEEF, ..HandleConfig::default() };
        cell_spudll_handle_config_set_default_values(Some(&mut cfg)).unwrap();
        assert_eq!(cfg.dma_tag, 0);
    }

    #[test]
    fn set_default_values_populates_num_max_referred() {
        let mut cfg = HandleConfig::default();
        cell_spudll_handle_config_set_default_values(Some(&mut cfg)).unwrap();
        assert_eq!(cfg.num_max_referred, 16);
    }

    #[test]
    fn set_default_values_populates_num_max_depend() {
        let mut cfg = HandleConfig::default();
        cell_spudll_handle_config_set_default_values(Some(&mut cfg)).unwrap();
        assert_eq!(cfg.num_max_depend, 16);
    }

    #[test]
    fn set_default_values_resets_unresolved_symbol_fallbacks() {
        let mut cfg = HandleConfig {
            unresolved_symbol_value_for_func:   0x1000,
            unresolved_symbol_value_for_object: 0x2000,
            unresolved_symbol_value_for_other:  0x3000,
            ..HandleConfig::default()
        };
        cell_spudll_handle_config_set_default_values(Some(&mut cfg)).unwrap();
        assert_eq!(cfg.unresolved_symbol_value_for_func,   0);
        assert_eq!(cfg.unresolved_symbol_value_for_object, 0);
        assert_eq!(cfg.unresolved_symbol_value_for_other,  0);
    }

    #[test]
    fn set_default_values_zero_fills_reserved_region() {
        let mut cfg = HandleConfig {
            reserved: [0xFFFF_FFFF; RESERVED_WORDS],
            ..HandleConfig::default()
        };
        cell_spudll_handle_config_set_default_values(Some(&mut cfg)).unwrap();
        assert_eq!(cfg.reserved, [0u32; RESERVED_WORDS]);
    }

    #[test]
    fn set_default_values_is_idempotent() {
        let mut cfg = HandleConfig::default();
        cell_spudll_handle_config_set_default_values(Some(&mut cfg)).unwrap();
        let snapshot = cfg;
        cell_spudll_handle_config_set_default_values(Some(&mut cfg)).unwrap();
        assert_eq!(snapshot, cfg);
    }

    #[test]
    fn set_default_values_produces_canonical_config() {
        let mut cfg = HandleConfig {
            mode: 1, dma_tag: 2, num_max_referred: 3, num_max_depend: 4,
            unresolved_symbol_value_for_func: 5,
            unresolved_symbol_value_for_object: 6,
            unresolved_symbol_value_for_other: 7,
            reserved: [8; RESERVED_WORDS],
        };
        cell_spudll_handle_config_set_default_values(Some(&mut cfg)).unwrap();
        assert_eq!(cfg, HandleConfig {
            mode: 0, dma_tag: 0, num_max_referred: 16, num_max_depend: 16,
            unresolved_symbol_value_for_func: 0,
            unresolved_symbol_value_for_object: 0,
            unresolved_symbol_value_for_other: 0,
            reserved: [0; RESERVED_WORDS],
        });
    }

    // ---- HandleConfig layout ---------------------------------------

    #[test]
    fn handle_config_has_all_public_fields() {
        // Compile-time smoke: every field must be usable.
        let cfg = HandleConfig {
            mode: 0, dma_tag: 0, num_max_referred: 0, num_max_depend: 0,
            unresolved_symbol_value_for_func: 0,
            unresolved_symbol_value_for_object: 0,
            unresolved_symbol_value_for_other: 0,
            reserved: [0; RESERVED_WORDS],
        };
        let _ = cfg.mode;
        let _ = cfg.dma_tag;
        let _ = cfg.num_max_referred;
        let _ = cfg.num_max_depend;
        let _ = cfg.unresolved_symbol_value_for_func;
        let _ = cfg.unresolved_symbol_value_for_object;
        let _ = cfg.unresolved_symbol_value_for_other;
        let _ = cfg.reserved;
    }

    #[test]
    fn handle_config_default_is_zeroed() {
        let cfg = HandleConfig::default();
        assert_eq!(cfg.mode, 0);
        assert_eq!(cfg.num_max_referred, 0);
        assert_eq!(cfg.reserved, [0u32; RESERVED_WORDS]);
    }

    #[test]
    fn handle_config_size_matches_cpp_layout() {
        // 7 leading u32 fields + 9 reserved u32 words = 16 u32 = 64 bytes.
        assert_eq!(core::mem::size_of::<HandleConfig>(), 16 * 4);
    }

    // ---- full smoke ------------------------------------------------

    #[test]
    fn full_spudll_lifecycle_smoke() {
        // Game allocates a fresh config with junk data, then calls the
        // firmware defaults before using it.
        let mut cfg = HandleConfig {
            mode: 0xAAAA_BBBB,
            dma_tag: 0xCCCC_DDDD,
            num_max_referred: 0xDEAD,
            num_max_depend: 0xBEEF,
            unresolved_symbol_value_for_func:   0x1111_1111,
            unresolved_symbol_value_for_object: 0x2222_2222,
            unresolved_symbol_value_for_other:  0x3333_3333,
            reserved: [0xFFFF_FFFF; RESERVED_WORDS],
        };
        cell_spudll_handle_config_set_default_values(Some(&mut cfg)).unwrap();

        // Check canonical defaults.
        assert_eq!(cfg.mode, 0);
        assert_eq!(cfg.num_max_referred, 16);
        assert_eq!(cfg.reserved, [0u32; RESERVED_WORDS]);

        // Use the config to probe image size.
        assert!(cell_spudll_get_image_size(true, true).is_ok());

        // Null fallback behaviour.
        assert_eq!(
            cell_spudll_get_image_size(false, true).unwrap_err(),
            errors::NULL_POINTER,
        );
        assert_eq!(
            cell_spudll_handle_config_set_default_values(None).unwrap_err(),
            errors::NULL_POINTER,
        );
    }
}
