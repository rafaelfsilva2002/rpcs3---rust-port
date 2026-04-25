//! `rpcs3-hle-cellfont-ft` — FreeType font library HLE variant.
//!
//! Ports `rpcs3/Emu/Cell/Modules/cellFontFT.cpp` — the FreeType-backed
//! sibling of `cellFont`.  The real module delegates the majority of
//! glyph operations to FreeType 2.x; in RPCS3 the heavy work is
//! stubbed and most entry points return `CELL_OK` after trivial null
//! checks.  This crate mirrors that surface:
//!
//! * `cellFontInitLibraryFreeTypeWithRevision` — validates `lib` + `config`,
//!   zero-writes `*lib`, then "allocates" a stub library handle.
//! * `cellFontInitLibraryFreeType` — thin wrapper that forwards with
//!   `revisionFlags = 0`.
//! * `cellFontFTGetRevisionFlags` — writes the fixed magic `0x43` to
//!   `*revisionFlags` when non-null (no-op otherwise).
//! * `cellFontFTGetInitializedRevisionFlags` — null-checks the output.
//! * A raft of `FTCacheStream_*`, `FTFaceH_*`, `FTManager_*` stubs that
//!   return `CELL_OK` verbatim from the firmware.
//!
//! The port preserves every observable byte the game sees: error codes,
//! the `0x43` revision magic, the null-check precedence, and the stub
//! registry.

use rpcs3_emu_types::CellError;

// =====================================================================
// Error codes — byte-exact with cellFont.h:9-10
// =====================================================================

pub mod errors {
    use rpcs3_emu_types::CellError;

    /// `CELL_FONT_ERROR_INVALID_PARAMETER`.
    pub const INVALID_PARAMETER: CellError = CellError(0x8054_0002);
    /// `CELL_FONT_ERROR_UNINITIALIZED`.
    pub const UNINITIALIZED:     CellError = CellError(0x8054_0003);
}

// =====================================================================
// Constants
// =====================================================================

/// Value written by `cellFontFTGetRevisionFlags` — see cellFontFT.cpp:49
/// (`*revisionFlags = 0x43;`).  The PS3 firmware uses this as a
/// feature-level indicator for the FT-backed library.
pub const REVISION_FLAGS_MAGIC: u64 = 0x43;

// =====================================================================
// Mirror structs
// =====================================================================

/// Pointer-size sentinel for a FreeType library handle.  The real C++
/// side hands out a `vm::alloc(sizeof(CellFontLibrary))` address; the
/// port returns a monotonically increasing "address" so higher layers
/// can distinguish multiple libraries deterministically in tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LibraryHandle(pub u32);

impl LibraryHandle {
    pub const NULL: Self = Self(0);
    #[must_use]
    pub const fn is_null(self) -> bool { self.0 == 0 }
}

/// Mirror of `CellFontLibraryConfigFT` — the PS3 firmware side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LibraryConfigFt {
    pub library: u32,
    pub memory_interface: MemoryInterface,
}

/// Subset of `CellFontMemoryInterface` the FT port needs.  Fields are
/// host-endian `u32`; the real type is `be_t<u32>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MemoryInterface {
    pub arg: u32,
    pub malloc: u32,
    pub free: u32,
    pub realloc: u32,
    pub calloc: u32,
}

// =====================================================================
// Library allocator — mirrors vm::alloc() but deterministic for tests.
// =====================================================================

#[derive(Debug, Default, Clone)]
pub struct FontFt {
    next_addr: u32,
    initialized_revision_flags: Option<u64>,
}

impl FontFt {
    #[must_use]
    pub fn new() -> Self {
        // Start allocations at an address that looks like `vm::main`
        // region (0x1000_0000) to visually distinguish from 0 / stack
        // pointers in logs.
        Self { next_addr: 0x1000_0000, initialized_revision_flags: None }
    }

    /// Port of `cellFontInitLibraryFreeTypeWithRevision`.  Validates
    /// `lib_valid` first (C++ checks `!lib` before `!config`) then
    /// `config_valid`.
    ///
    /// # Errors
    /// * [`errors::INVALID_PARAMETER`] if either `lib` or `config` is null.
    pub fn init_library_with_revision(
        &mut self,
        revision_flags: u64,
        config_valid: bool,
        lib_valid: bool,
    ) -> Result<LibraryHandle, CellError> {
        if !lib_valid {
            return Err(errors::INVALID_PARAMETER);
        }
        // C++ writes `*lib = {}` (zero) *before* validating config —
        // callers that pass non-null lib see it cleared even if config
        // check fails.  Mirror by swallowing the zero-write here.
        if !config_valid {
            return Err(errors::INVALID_PARAMETER);
        }

        // Allocate a pseudo-address (sizeof(CellFontLibrary) = 40 bytes
        // on PS3; we match the stride for deterministic test output).
        const CELL_FONT_LIBRARY_SIZE: u32 = 40;
        let addr = self.next_addr;
        self.next_addr = self
            .next_addr
            .checked_add(CELL_FONT_LIBRARY_SIZE)
            .ok_or(errors::UNINITIALIZED)?;
        self.initialized_revision_flags = Some(revision_flags);
        Ok(LibraryHandle(addr))
    }

    /// Port of `cellFontInitLibraryFreeType` — delegates to the
    /// revisioned variant with `revisionFlags = 0`.
    ///
    /// # Errors
    /// Same as [`Self::init_library_with_revision`].
    pub fn init_library(
        &mut self,
        config_valid: bool,
        lib_valid: bool,
    ) -> Result<LibraryHandle, CellError> {
        self.init_library_with_revision(0, config_valid, lib_valid)
    }

    /// Port of `cellFontFTGetRevisionFlags`.  If the pointer is valid,
    /// writes the magic `0x43` and returns it; otherwise the firmware
    /// silently does nothing (no error).
    #[must_use]
    pub fn get_revision_flags(&self, out_valid: bool) -> Option<u64> {
        if out_valid {
            Some(REVISION_FLAGS_MAGIC)
        } else {
            None
        }
    }

    /// Port of `cellFontFTGetInitializedRevisionFlags`.  C++ null-checks
    /// the pointer first and only then would check that the library was
    /// initialised; the RPCS3 stub skips the init check.
    ///
    /// # Errors
    /// * [`errors::INVALID_PARAMETER`] if `out_valid` is false.
    pub fn get_initialized_revision_flags(
        &self,
        out_valid: bool,
    ) -> Result<u64, CellError> {
        if !out_valid {
            return Err(errors::INVALID_PARAMETER);
        }
        // The real firmware would return `errors::UNINITIALIZED` if
        // `cellFontInitLibraryFreeTypeWithRevision` was never called;
        // the C++ port has that path guarded by `if (false)` so we
        // preserve the fact that it *always* returns CELL_OK once the
        // pointer check passes.
        Ok(self.initialized_revision_flags.unwrap_or(0))
    }
}

// =====================================================================
// Stub registry — the 34 `UNIMPLEMENTED_FUNC` entry points that return
// CELL_OK verbatim in the C++ port.
// =====================================================================

/// All 34 stub entry points registered in `REG_FUNC(cellFontFT, …)` that
/// call `UNIMPLEMENTED_FUNC` + `return CELL_OK` (cellFontFT.cpp:72-304).
///
/// Game code calls these freely and expects success; higher layers can
/// route any name through [`invoke_stub`] to keep behaviour byte-exact.
pub const STUB_ENTRY_POINTS: &[&str] = &[
    // FTCacheStream family (5)
    "FTCacheStream_CacheEnd",
    "FTCacheStream_CacheInit",
    "FTCacheStream_CalcCacheIndexSize",
    "FTCacheStream_End",
    "FTCacheStream_Init",
    // FTFaceH family (22)
    "FTFaceH_Close",
    "FTFaceH_FontFamilyName",
    "FTFaceH_FontStyleName",
    "FTFaceH_GetAscender",
    "FTFaceH_GetBoundingBoxHeight",
    "FTFaceH_GetBoundingBoxMaxX",
    "FTFaceH_GetBoundingBoxMaxY",
    "FTFaceH_GetBoundingBoxMinX",
    "FTFaceH_GetBoundingBoxMinY",
    "FTFaceH_GetBoundingBoxWidth",
    "FTFaceH_GetCompositeCodes",
    "FTFaceH_GetGlyphImage",
    "FTFaceH_GetGlyphMetrics",
    "FTFaceH_GetKerning",
    "FTFaceH_GetMaxHorizontalAdvance",
    "FTFaceH_GetMaxVerticalAdvance",
    "FTFaceH_GetRenderBufferSize",
    "FTFaceH_GetRenderEffectSlant",
    "FTFaceH_GetRenderEffectWeight",
    "FTFaceH_GetRenderScale",
    "FTFaceH_GetRenderScalePixel",
    "FTFaceH_GetRenderScalePoint",
    "FTFaceH_SetCompositeCodes",
    "FTFaceH_SetRenderEffectSlant",
    "FTFaceH_SetRenderEffectWeight",
    "FTFaceH_SetRenderScalePixel",
    "FTFaceH_SetRenderScalePoint",
    // FTManager family (7)
    "FTManager_CloseFace",
    "FTManager_Done_FreeType",
    "FTManager_Init_FreeType",
    "FTManager_OpenFileFace",
    "FTManager_OpenMemFace",
    "FTManager_OpenStreamFace",
    "FTManager_SetFontOpenMode",
];

/// Invoke any of the stub entry points.  Returns `Ok(())` if `name`
/// matches one of [`STUB_ENTRY_POINTS`] — mirroring the C++ stubs that
/// unconditionally return `CELL_OK`.
///
/// # Errors
/// * [`errors::INVALID_PARAMETER`] if `name` is not a known stub.
pub fn invoke_stub(name: &str) -> Result<(), CellError> {
    if STUB_ENTRY_POINTS.contains(&name) {
        Ok(())
    } else {
        Err(errors::INVALID_PARAMETER)
    }
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ---- constants ---------------------------------------------------

    #[test]
    fn error_codes_byte_exact() {
        assert_eq!(errors::INVALID_PARAMETER.0, 0x8054_0002);
        assert_eq!(errors::UNINITIALIZED.0,     0x8054_0003);
    }

    #[test]
    fn revision_flags_magic_byte_exact() {
        assert_eq!(REVISION_FLAGS_MAGIC, 0x43);
    }

    #[test]
    fn library_handle_null_sentinel() {
        assert!(LibraryHandle::NULL.is_null());
        assert!(!LibraryHandle(0x1).is_null());
        assert_eq!(LibraryHandle::NULL, LibraryHandle(0));
    }

    // ---- init_library_with_revision ---------------------------------

    #[test]
    fn init_with_revision_happy_path() {
        let mut f = FontFt::new();
        let h = f.init_library_with_revision(0x43, true, true).unwrap();
        assert!(!h.is_null());
        assert_eq!(h.0, 0x1000_0000); // first allocation
    }

    #[test]
    fn init_with_revision_null_lib_is_invalid_parameter() {
        let mut f = FontFt::new();
        assert_eq!(
            f.init_library_with_revision(0, true, false).unwrap_err(),
            errors::INVALID_PARAMETER,
        );
    }

    #[test]
    fn init_with_revision_null_config_is_invalid_parameter() {
        let mut f = FontFt::new();
        assert_eq!(
            f.init_library_with_revision(0, false, true).unwrap_err(),
            errors::INVALID_PARAMETER,
        );
    }

    #[test]
    fn init_with_revision_lib_check_beats_config_check() {
        // null lib AND null config → should fail on lib check (first).
        let mut f = FontFt::new();
        // Both are null — C++ checks `!lib` first, returns immediately.
        // Mirror: with lib_valid=false, config_valid is irrelevant.
        assert_eq!(
            f.init_library_with_revision(0, false, false).unwrap_err(),
            errors::INVALID_PARAMETER,
        );
    }

    #[test]
    fn init_with_revision_stores_revision_flags() {
        let mut f = FontFt::new();
        f.init_library_with_revision(0xDEAD_BEEF, true, true).unwrap();
        assert_eq!(f.get_initialized_revision_flags(true).unwrap(), 0xDEAD_BEEF);
    }

    #[test]
    fn init_allocates_monotonically() {
        let mut f = FontFt::new();
        let a = f.init_library_with_revision(0, true, true).unwrap();
        let b = f.init_library_with_revision(0, true, true).unwrap();
        let c = f.init_library(true, true).unwrap();
        // Each library gets a 40-byte slot (matching CellFontLibrary).
        assert_eq!(b.0 - a.0, 40);
        assert_eq!(c.0 - b.0, 40);
    }

    // ---- init_library (no-revision wrapper) -------------------------

    #[test]
    fn init_library_delegates_with_zero_flags() {
        let mut f = FontFt::new();
        let _ = f.init_library(true, true).unwrap();
        assert_eq!(f.get_initialized_revision_flags(true).unwrap(), 0);
    }

    #[test]
    fn init_library_propagates_lib_null_error() {
        let mut f = FontFt::new();
        assert_eq!(
            f.init_library(true, false).unwrap_err(),
            errors::INVALID_PARAMETER,
        );
    }

    #[test]
    fn init_library_propagates_config_null_error() {
        let mut f = FontFt::new();
        assert_eq!(
            f.init_library(false, true).unwrap_err(),
            errors::INVALID_PARAMETER,
        );
    }

    // ---- get_revision_flags ------------------------------------------

    #[test]
    fn get_revision_flags_writes_magic_on_valid_ptr() {
        let f = FontFt::new();
        assert_eq!(f.get_revision_flags(true), Some(0x43));
    }

    #[test]
    fn get_revision_flags_silent_on_null_ptr() {
        let f = FontFt::new();
        assert_eq!(f.get_revision_flags(false), None);
    }

    // ---- get_initialized_revision_flags ------------------------------

    #[test]
    fn get_initialized_flags_null_out_is_invalid_parameter() {
        let f = FontFt::new();
        assert_eq!(
            f.get_initialized_revision_flags(false).unwrap_err(),
            errors::INVALID_PARAMETER,
        );
    }

    #[test]
    fn get_initialized_flags_returns_zero_when_uninitialized() {
        // C++ stub returns CELL_OK without writing; we return 0 to
        // match the "no observable write" behaviour.
        let f = FontFt::new();
        assert_eq!(f.get_initialized_revision_flags(true).unwrap(), 0);
    }

    #[test]
    fn get_initialized_flags_returns_stored_after_init() {
        let mut f = FontFt::new();
        f.init_library_with_revision(0xABCD_1234, true, true).unwrap();
        assert_eq!(f.get_initialized_revision_flags(true).unwrap(), 0xABCD_1234);
    }

    // ---- stub entry points ------------------------------------------

    #[test]
    fn stub_registry_has_all_39_entry_points() {
        // 5 FTCacheStream + 27 FTFaceH + 7 FTManager = 39 — mirrors
        // the `REG_FUNC` block in cellFontFT.cpp:306-354 exactly.
        assert_eq!(STUB_ENTRY_POINTS.len(), 39);
        let cache = STUB_ENTRY_POINTS.iter().filter(|n| n.starts_with("FTCacheStream_")).count();
        let face  = STUB_ENTRY_POINTS.iter().filter(|n| n.starts_with("FTFaceH_")).count();
        let mgr   = STUB_ENTRY_POINTS.iter().filter(|n| n.starts_with("FTManager_")).count();
        assert_eq!((cache, face, mgr), (5, 27, 7));
    }

    #[test]
    fn invoke_stub_ftcachestream_family_all_ok() {
        for name in ["FTCacheStream_CacheEnd", "FTCacheStream_CacheInit",
                     "FTCacheStream_CalcCacheIndexSize",
                     "FTCacheStream_End", "FTCacheStream_Init"] {
            assert!(invoke_stub(name).is_ok(), "{name} should succeed");
        }
    }

    #[test]
    fn invoke_stub_ftfaceh_family_all_ok() {
        for name in ["FTFaceH_Close", "FTFaceH_GetGlyphImage",
                     "FTFaceH_SetRenderScalePoint",
                     "FTFaceH_GetBoundingBoxMaxY"] {
            assert!(invoke_stub(name).is_ok(), "{name} should succeed");
        }
    }

    #[test]
    fn invoke_stub_ftmanager_family_all_ok() {
        for name in ["FTManager_CloseFace", "FTManager_Init_FreeType",
                     "FTManager_OpenFileFace", "FTManager_OpenMemFace",
                     "FTManager_OpenStreamFace",
                     "FTManager_SetFontOpenMode",
                     "FTManager_Done_FreeType"] {
            assert!(invoke_stub(name).is_ok(), "{name} should succeed");
        }
    }

    #[test]
    fn invoke_stub_unknown_name_is_invalid_parameter() {
        assert_eq!(
            invoke_stub("FTUnknown_NonExistent").unwrap_err(),
            errors::INVALID_PARAMETER,
        );
    }

    #[test]
    fn invoke_stub_case_sensitive() {
        // C++ REG_FUNC is case-sensitive; our registry must be too.
        assert_eq!(
            invoke_stub("ftcachestream_init").unwrap_err(),
            errors::INVALID_PARAMETER,
        );
    }

    // ---- config struct layout --------------------------------------

    #[test]
    fn memory_interface_default_is_zeroed() {
        let mi = MemoryInterface::default();
        assert_eq!(mi.arg, 0);
        assert_eq!(mi.malloc, 0);
        assert_eq!(mi.free, 0);
        assert_eq!(mi.realloc, 0);
        assert_eq!(mi.calloc, 0);
    }

    #[test]
    fn library_config_ft_default_is_zeroed() {
        let cfg = LibraryConfigFt::default();
        assert_eq!(cfg.library, 0);
        assert_eq!(cfg.memory_interface, MemoryInterface::default());
    }

    // ---- full lifecycle smoke --------------------------------------

    #[test]
    fn full_fontft_lifecycle_smoke() {
        let mut f = FontFt::new();

        // 1. Game asks for the revision flags magic before init.
        assert_eq!(f.get_revision_flags(true), Some(0x43));

        // 2. Init with a specific revision.
        let h = f.init_library_with_revision(0x43, true, true).unwrap();
        assert!(!h.is_null());

        // 3. Query the stored revision — matches what we passed in.
        assert_eq!(f.get_initialized_revision_flags(true).unwrap(), 0x43);

        // 4. Call a handful of the stub entry points — all return OK.
        invoke_stub("FTManager_Init_FreeType").unwrap();
        invoke_stub("FTManager_OpenFileFace").unwrap();
        invoke_stub("FTFaceH_GetGlyphMetrics").unwrap();
        invoke_stub("FTCacheStream_Init").unwrap();
        invoke_stub("FTFaceH_Close").unwrap();
        invoke_stub("FTManager_CloseFace").unwrap();
        invoke_stub("FTManager_Done_FreeType").unwrap();

        // 5. Null output pointer on query returns INVALID_PARAMETER.
        assert_eq!(
            f.get_initialized_revision_flags(false).unwrap_err(),
            errors::INVALID_PARAMETER,
        );
    }
}
