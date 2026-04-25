//! Rust port of `rpcs3/Emu/Cell/Modules/cellSysutilMisc.cpp`.
//!
//! Single-entry-point module — `cellSysutilGetLicenseArea` returns the
//! PS3's configured license area code (SCEA/SCEJ/SCEE/SCEH/SCEK/SCH/
//! OTHER). The C++ (20 lines total) reads the value out of
//! `g_cfg.sys.license_area`; the Rust port exposes a trait so callers
//! can inject the backing value.
//!
//! Module name byte-exact at cpp:6 / cpp:17. REG_FUNC at cpp:19.

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

/// Byte-exact at cpp:6 / cpp:17.
pub const MODULE_NAME: &str = "cellSysutilMisc";

/// REG_FUNC at cpp:19.
pub const REGISTERED_ENTRY_POINTS: &[&str] = &["cellSysutilGetLicenseArea"];

// --- License area enum (byte-exact cellSysutil.h) -----------------------

pub const CELL_SYSUTIL_LICENSE_AREA_J: i32 = 0;
pub const CELL_SYSUTIL_LICENSE_AREA_A: i32 = 1;
pub const CELL_SYSUTIL_LICENSE_AREA_E: i32 = 2;
pub const CELL_SYSUTIL_LICENSE_AREA_H: i32 = 3;
pub const CELL_SYSUTIL_LICENSE_AREA_K: i32 = 4;
pub const CELL_SYSUTIL_LICENSE_AREA_C: i32 = 5;
pub const CELL_SYSUTIL_LICENSE_AREA_OTHER: i32 = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum CellSysutilLicenseArea {
    /// SCEJ (Japan).
    J = CELL_SYSUTIL_LICENSE_AREA_J,
    /// SCEA (Americas + Canada + Brazil).
    A = CELL_SYSUTIL_LICENSE_AREA_A,
    /// SCEE (Europe + UK + Oceania + Russia).
    E = CELL_SYSUTIL_LICENSE_AREA_E,
    /// SCEH (Hong Kong + Taiwan + Southeast Asia).
    H = CELL_SYSUTIL_LICENSE_AREA_H,
    /// SCEK (Korea).
    K = CELL_SYSUTIL_LICENSE_AREA_K,
    /// SCH (China).
    C = CELL_SYSUTIL_LICENSE_AREA_C,
    /// Any other (firmware sentinel).
    Other = CELL_SYSUTIL_LICENSE_AREA_OTHER,
}

impl CellSysutilLicenseArea {
    #[must_use]
    pub const fn as_i32(self) -> i32 {
        self as i32
    }

    #[must_use]
    pub const fn from_i32(v: i32) -> Option<Self> {
        match v {
            CELL_SYSUTIL_LICENSE_AREA_J => Some(Self::J),
            CELL_SYSUTIL_LICENSE_AREA_A => Some(Self::A),
            CELL_SYSUTIL_LICENSE_AREA_E => Some(Self::E),
            CELL_SYSUTIL_LICENSE_AREA_H => Some(Self::H),
            CELL_SYSUTIL_LICENSE_AREA_K => Some(Self::K),
            CELL_SYSUTIL_LICENSE_AREA_C => Some(Self::C),
            CELL_SYSUTIL_LICENSE_AREA_OTHER => Some(Self::Other),
            _ => None,
        }
    }

    /// 3-letter SCE region code matching the comment block at
    /// `cellSysutil.h`.
    #[must_use]
    pub const fn sce_tag(self) -> &'static str {
        match self {
            Self::J => "SCEJ",
            Self::A => "SCEA",
            Self::E => "SCEE",
            Self::H => "SCEH",
            Self::K => "SCEK",
            Self::C => "SCH",
            Self::Other => "OTHER",
        }
    }
}

impl Default for CellSysutilLicenseArea {
    fn default() -> Self {
        Self::A
    }
}

// --- Backend trait ------------------------------------------------------

/// Backing source for [`cell_sysutil_get_license_area`] — mirrors
/// `g_cfg.sys.license_area` in the C++.
pub trait LicenseAreaSource {
    fn license_area(&self) -> CellSysutilLicenseArea;
}

/// Reference backend: returns the supplied area unconditionally.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FixedLicenseArea(pub CellSysutilLicenseArea);

impl LicenseAreaSource for FixedLicenseArea {
    fn license_area(&self) -> CellSysutilLicenseArea {
        self.0
    }
}

// --- Entry point --------------------------------------------------------

/// `cellSysutilGetLicenseArea` (cpp:8-15). Reads the configured license
/// area via the supplied [`LicenseAreaSource`].
pub fn cell_sysutil_get_license_area<S: LicenseAreaSource>(source: &S) -> i32 {
    source.license_area().as_i32()
}

/// Same entry, but returns the strongly-typed enum — convenient for
/// tests.
pub fn get_license_area_typed<S: LicenseAreaSource>(source: &S) -> CellSysutilLicenseArea {
    source.license_area()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_name_byte_exact() {
        assert_eq!(MODULE_NAME, "cellSysutilMisc");
    }

    #[test]
    fn registered_entry_points_exact_order() {
        assert_eq!(REGISTERED_ENTRY_POINTS, &["cellSysutilGetLicenseArea"]);
    }

    #[test]
    fn license_area_values_byte_exact() {
        assert_eq!(CELL_SYSUTIL_LICENSE_AREA_J, 0);
        assert_eq!(CELL_SYSUTIL_LICENSE_AREA_A, 1);
        assert_eq!(CELL_SYSUTIL_LICENSE_AREA_E, 2);
        assert_eq!(CELL_SYSUTIL_LICENSE_AREA_H, 3);
        assert_eq!(CELL_SYSUTIL_LICENSE_AREA_K, 4);
        assert_eq!(CELL_SYSUTIL_LICENSE_AREA_C, 5);
        assert_eq!(CELL_SYSUTIL_LICENSE_AREA_OTHER, 100);
    }

    #[test]
    fn enum_as_i32_matches_constants() {
        assert_eq!(CellSysutilLicenseArea::J.as_i32(), 0);
        assert_eq!(CellSysutilLicenseArea::A.as_i32(), 1);
        assert_eq!(CellSysutilLicenseArea::E.as_i32(), 2);
        assert_eq!(CellSysutilLicenseArea::H.as_i32(), 3);
        assert_eq!(CellSysutilLicenseArea::K.as_i32(), 4);
        assert_eq!(CellSysutilLicenseArea::C.as_i32(), 5);
        assert_eq!(CellSysutilLicenseArea::Other.as_i32(), 100);
    }

    #[test]
    fn enum_roundtrip_known_values() {
        for area in [
            CellSysutilLicenseArea::J,
            CellSysutilLicenseArea::A,
            CellSysutilLicenseArea::E,
            CellSysutilLicenseArea::H,
            CellSysutilLicenseArea::K,
            CellSysutilLicenseArea::C,
            CellSysutilLicenseArea::Other,
        ] {
            assert_eq!(
                CellSysutilLicenseArea::from_i32(area.as_i32()),
                Some(area)
            );
        }
    }

    #[test]
    fn enum_rejects_unknown_values() {
        assert_eq!(CellSysutilLicenseArea::from_i32(6), None);
        assert_eq!(CellSysutilLicenseArea::from_i32(99), None);
        assert_eq!(CellSysutilLicenseArea::from_i32(101), None);
        assert_eq!(CellSysutilLicenseArea::from_i32(-1), None);
    }

    #[test]
    fn sce_tags_match_header_comments() {
        assert_eq!(CellSysutilLicenseArea::J.sce_tag(), "SCEJ");
        assert_eq!(CellSysutilLicenseArea::A.sce_tag(), "SCEA");
        assert_eq!(CellSysutilLicenseArea::E.sce_tag(), "SCEE");
        assert_eq!(CellSysutilLicenseArea::H.sce_tag(), "SCEH");
        assert_eq!(CellSysutilLicenseArea::K.sce_tag(), "SCEK");
        // The header lists "SCH" (3 letters) for China specifically.
        assert_eq!(CellSysutilLicenseArea::C.sce_tag(), "SCH");
        assert_eq!(CellSysutilLicenseArea::Other.sce_tag(), "OTHER");
    }

    #[test]
    fn default_is_scea() {
        assert_eq!(
            CellSysutilLicenseArea::default(),
            CellSysutilLicenseArea::A
        );
    }

    #[test]
    fn fixed_backend_returns_configured_area() {
        let src = FixedLicenseArea(CellSysutilLicenseArea::E);
        assert_eq!(cell_sysutil_get_license_area(&src), 2);
        assert_eq!(get_license_area_typed(&src), CellSysutilLicenseArea::E);
    }

    #[test]
    fn fixed_backend_handles_other_sentinel() {
        let src = FixedLicenseArea(CellSysutilLicenseArea::Other);
        assert_eq!(
            cell_sysutil_get_license_area(&src),
            CELL_SYSUTIL_LICENSE_AREA_OTHER
        );
    }

    #[test]
    fn each_region_tag_unique() {
        use alloc::collections::BTreeSet;
        let tags: BTreeSet<&str> = [
            CellSysutilLicenseArea::J,
            CellSysutilLicenseArea::A,
            CellSysutilLicenseArea::E,
            CellSysutilLicenseArea::H,
            CellSysutilLicenseArea::K,
            CellSysutilLicenseArea::C,
            CellSysutilLicenseArea::Other,
        ]
        .iter()
        .map(|a| a.sce_tag())
        .collect();
        assert_eq!(tags.len(), 7);
    }

    #[test]
    fn custom_backend_impl() {
        struct DynamicBackend {
            current: CellSysutilLicenseArea,
        }
        impl LicenseAreaSource for DynamicBackend {
            fn license_area(&self) -> CellSysutilLicenseArea {
                self.current
            }
        }
        let mut b = DynamicBackend {
            current: CellSysutilLicenseArea::J,
        };
        assert_eq!(cell_sysutil_get_license_area(&b), 0);
        b.current = CellSysutilLicenseArea::K;
        assert_eq!(cell_sysutil_get_license_area(&b), 4);
    }

    #[test]
    fn full_license_area_lifecycle_smoke() {
        // Cycle through every region, simulating a PS3 booting in
        // different territories.
        for area in [
            CellSysutilLicenseArea::J,
            CellSysutilLicenseArea::A,
            CellSysutilLicenseArea::E,
            CellSysutilLicenseArea::H,
            CellSysutilLicenseArea::K,
            CellSysutilLicenseArea::C,
            CellSysutilLicenseArea::Other,
        ] {
            let src = FixedLicenseArea(area);
            let raw = cell_sysutil_get_license_area(&src);
            let typed = get_license_area_typed(&src);
            assert_eq!(raw, area.as_i32());
            assert_eq!(typed, area);
            assert_eq!(
                CellSysutilLicenseArea::from_i32(raw),
                Some(area),
                "roundtrip for {:?}",
                area
            );
        }
    }
}
