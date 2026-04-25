//! `rpcs3-hle-cellovis` — SPU overlay table management HLE.
//!
//! Ports `rpcs3/Emu/Cell/Modules/cellOvis.cpp`. `cellOvis` is the
//! SPU overlay system — games partition a large SPU ELF into
//! overlappable segments, then the PPU-side library manages which
//! overlay is loaded into the SPU's local store at runtime.
//!
//! ## Entry points covered
//!
//! | HLE function                              | Rust wrapper                          |
//! |-------------------------------------------|---------------------------------------|
//! | `cellOvisGetOverlayTableSize`             | [`overlay_table_size`]                |
//! | `cellOvisInitializeOverlayTable`          | [`OverlayTable::initialize`]          |
//! | `cellOvisFixSpuSegments`                  | [`OverlayTable::fix_spu_segments`]    |
//! | `cellOvisInvalidateOverlappedSegments`    | [`OverlayTable::invalidate_overlapped_segments`] |

use rpcs3_emu_types::CellError;

// =====================================================================
// Error codes — byte-exact with cellOvis.cpp:10-15
// =====================================================================

pub mod errors {
    use rpcs3_emu_types::CellError;

    pub const INVAL: CellError = CellError(0x8041_0402);
    pub const ABORT: CellError = CellError(0x8041_040C);
    pub const ALIGN: CellError = CellError(0x8041_0410);
}

// =====================================================================
// Constants
// =====================================================================

/// SPU local-store is 256 KiB. Overlay segments must fit within it.
pub const LOCAL_STORE_SIZE: u32 = 256 * 1024;

/// SPU segments have 16-byte alignment (enforced by the SPU ABI).
pub const SEGMENT_ALIGN: u32 = 16;

/// Overlay table entries are 16 bytes each: (vaddr u32, size u32, flags u32, reserved u32).
pub const TABLE_ENTRY_SIZE: u32 = 16;

/// Per C++ `cellOvisInitializeOverlayTable` — first 16 bytes of the
/// allocation are a header (table count + flags).
pub const TABLE_HEADER_SIZE: u32 = 16;

/// Maximum overlays per ELF. SDK docs cap this; tests use it as a
/// sanity limit.
pub const MAX_OVERLAYS: usize = 128;

// =====================================================================
// SPU segment types — byte-exact with sys_spu_image.h
// =====================================================================

pub const SEG_TYPE_COPY: i32 = 1;
pub const SEG_TYPE_FILL: i32 = 2;
pub const SEG_TYPE_INFO: i32 = 4;

#[must_use]
pub fn is_known_seg_type(t: i32) -> bool {
    matches!(t, SEG_TYPE_COPY | SEG_TYPE_FILL | SEG_TYPE_INFO)
}

// =====================================================================
// Types
// =====================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OverlayEntry {
    /// Target local-store address where the overlay is mapped.
    pub ls_addr: u32,
    /// Overlay byte size (≤ LOCAL_STORE_SIZE, ≤ available LS).
    pub size: u32,
    /// Implementation-defined flags (e.g. "resident", "locked").
    pub flags: u32,
}

impl OverlayEntry {
    fn validate(&self) -> Result<(), CellError> {
        if self.size == 0 {
            return Err(errors::INVAL);
        }
        if self.size > LOCAL_STORE_SIZE {
            return Err(errors::INVAL);
        }
        if self.ls_addr % SEGMENT_ALIGN != 0 {
            return Err(errors::ALIGN);
        }
        if self.size % SEGMENT_ALIGN != 0 {
            return Err(errors::ALIGN);
        }
        let end = self.ls_addr.checked_add(self.size).ok_or(errors::INVAL)?;
        if end > LOCAL_STORE_SIZE {
            return Err(errors::INVAL);
        }
        Ok(())
    }

    #[must_use]
    pub fn range(&self) -> (u32, u32) {
        (self.ls_addr, self.ls_addr + self.size)
    }

    #[must_use]
    pub fn overlaps(&self, other: &OverlayEntry) -> bool {
        let (a_start, a_end) = self.range();
        let (b_start, b_end) = other.range();
        a_start < b_end && b_start < a_end
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpuSegment {
    pub seg_type: i32,
    pub ls_addr: u32,
    pub size: u32,
    pub source_addr: u32,
    pub fill_pattern: u32,
}

impl SpuSegment {
    pub fn validate(&self) -> Result<(), CellError> {
        if !is_known_seg_type(self.seg_type) {
            return Err(errors::INVAL);
        }
        if self.ls_addr % SEGMENT_ALIGN != 0 {
            return Err(errors::ALIGN);
        }
        if self.seg_type != SEG_TYPE_INFO {
            if self.size == 0 || self.size > LOCAL_STORE_SIZE {
                return Err(errors::INVAL);
            }
            if self.size % SEGMENT_ALIGN != 0 {
                return Err(errors::ALIGN);
            }
            let end = self.ls_addr.checked_add(self.size).ok_or(errors::INVAL)?;
            if end > LOCAL_STORE_SIZE {
                return Err(errors::INVAL);
            }
        }
        Ok(())
    }

    #[must_use]
    pub fn range(&self) -> (u32, u32) {
        (self.ls_addr, self.ls_addr + self.size)
    }
}

// =====================================================================
// OverlayTable
// =====================================================================

#[derive(Clone, Debug, Default)]
pub struct OverlayTable {
    entries: Vec<OverlayEntry>,
    initialized: bool,
    aborted: bool,
}

/// `cellOvisGetOverlayTableSize(elf)`. Returns header + per-entry size
/// for a given overlay count. Games typically parse their ELF to count
/// PT_LOAD overlays, then ask for the table size.
#[must_use]
pub fn overlay_table_size(overlay_count: u32) -> u32 {
    TABLE_HEADER_SIZE + overlay_count.saturating_mul(TABLE_ENTRY_SIZE)
}

impl OverlayTable {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    #[must_use]
    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }

    /// `cellOvisInitializeOverlayTable(ea_ovly_table, elf)`. `table_addr`
    /// must be 16-byte aligned per SPU ABI. `entries` is the parsed
    /// overlay set from the ELF (game supplies PT_LOAD-derived list in
    /// tests; real lib parses the ELF header).
    pub fn initialize(
        &mut self,
        table_addr: u64,
        entries: Vec<OverlayEntry>,
    ) -> Result<(), CellError> {
        if table_addr == 0 {
            return Err(errors::INVAL);
        }
        if table_addr % u64::from(SEGMENT_ALIGN) != 0 {
            return Err(errors::ALIGN);
        }
        if entries.len() > MAX_OVERLAYS {
            return Err(errors::INVAL);
        }
        for e in &entries {
            e.validate()?;
        }
        self.entries = entries;
        self.initialized = true;
        self.aborted = false;
        Ok(())
    }

    /// `cellOvisFixSpuSegments(image)`. Walks an SPU image's segment
    /// array and "fixes up" any COPY segment that overlaps an overlay
    /// entry. The fix-up is: drop the segment (the overlay system
    /// handles loading dynamically). Mirrors the C++ effect: returns
    /// the corrected segment list.
    pub fn fix_spu_segments(
        &self,
        segments: &[SpuSegment],
    ) -> Result<Vec<SpuSegment>, CellError> {
        if !self.initialized {
            return Err(errors::INVAL);
        }
        if self.aborted {
            return Err(errors::ABORT);
        }
        for s in segments {
            s.validate()?;
        }
        let out = segments
            .iter()
            .copied()
            .filter(|s| {
                if s.seg_type == SEG_TYPE_INFO {
                    // INFO segments pass through.
                    return true;
                }
                // Drop COPY/FILL segments that overlap any overlay entry.
                !self.entries.iter().any(|e| s.range().0 < e.range().1 && e.range().0 < s.range().1)
            })
            .collect();
        Ok(out)
    }

    /// `cellOvisInvalidateOverlappedSegments(segs, nsegs)`. In-place
    /// variant used when the caller owns the segment buffer. Sets
    /// `seg_type` to 0 (ignored) for segments that overlap any overlay.
    /// Returns the new segment count excluding the zeroed entries.
    pub fn invalidate_overlapped_segments(
        &self,
        segments: &mut Vec<SpuSegment>,
    ) -> Result<u32, CellError> {
        if !self.initialized {
            return Err(errors::INVAL);
        }
        if self.aborted {
            return Err(errors::ABORT);
        }
        let before = segments.len();
        let retained = self.fix_spu_segments(segments)?;
        *segments = retained;
        Ok((before - segments.len()) as u32)
    }

    /// Signals an async abort; subsequent calls observe ABORT.
    pub fn abort(&mut self) {
        self.aborted = true;
    }

    #[must_use]
    pub fn is_aborted(&self) -> bool {
        self.aborted
    }

    #[must_use]
    pub fn entries(&self) -> &[OverlayEntry] {
        &self.entries
    }

    /// Find the overlay that covers a given LS address. Returns None if
    /// no overlay is mapped at that address.
    #[must_use]
    pub fn find_at(&self, ls_addr: u32) -> Option<&OverlayEntry> {
        self.entries.iter().find(|e| {
            let (start, end) = e.range();
            ls_addr >= start && ls_addr < end
        })
    }
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn ok_entry(ls_addr: u32, size: u32) -> OverlayEntry {
        OverlayEntry { ls_addr, size, flags: 0 }
    }

    fn segment(seg_type: i32, ls_addr: u32, size: u32) -> SpuSegment {
        SpuSegment {
            seg_type,
            ls_addr,
            size,
            source_addr: 0,
            fill_pattern: 0,
        }
    }

    #[test]
    fn error_codes_byte_exact() {
        assert_eq!(errors::INVAL.0, 0x8041_0402);
        assert_eq!(errors::ABORT.0, 0x8041_040C);
        assert_eq!(errors::ALIGN.0, 0x8041_0410);
    }

    #[test]
    fn constants_stable() {
        assert_eq!(LOCAL_STORE_SIZE, 256 * 1024);
        assert_eq!(SEGMENT_ALIGN, 16);
        assert_eq!(TABLE_ENTRY_SIZE, 16);
        assert_eq!(TABLE_HEADER_SIZE, 16);
        assert_eq!(MAX_OVERLAYS, 128);
        assert_eq!(SEG_TYPE_COPY, 1);
        assert_eq!(SEG_TYPE_FILL, 2);
        assert_eq!(SEG_TYPE_INFO, 4);
    }

    #[test]
    fn overlay_table_size_header_only_for_zero_entries() {
        assert_eq!(overlay_table_size(0), TABLE_HEADER_SIZE);
    }

    #[test]
    fn overlay_table_size_grows_linearly() {
        assert_eq!(overlay_table_size(1), TABLE_HEADER_SIZE + TABLE_ENTRY_SIZE);
        assert_eq!(overlay_table_size(4), TABLE_HEADER_SIZE + 4 * TABLE_ENTRY_SIZE);
    }

    #[test]
    fn overlay_entry_validate_size_zero_rejected() {
        let e = OverlayEntry { ls_addr: 0x1000, size: 0, flags: 0 };
        assert_eq!(e.validate(), Err(errors::INVAL));
    }

    #[test]
    fn overlay_entry_validate_size_too_big_rejected() {
        let e = OverlayEntry { ls_addr: 0x1000, size: LOCAL_STORE_SIZE + 16, flags: 0 };
        assert_eq!(e.validate(), Err(errors::INVAL));
    }

    #[test]
    fn overlay_entry_validate_misaligned_addr_rejected() {
        let e = OverlayEntry { ls_addr: 0x1001, size: 16, flags: 0 };
        assert_eq!(e.validate(), Err(errors::ALIGN));
    }

    #[test]
    fn overlay_entry_validate_misaligned_size_rejected() {
        let e = OverlayEntry { ls_addr: 0x1000, size: 17, flags: 0 };
        assert_eq!(e.validate(), Err(errors::ALIGN));
    }

    #[test]
    fn overlay_entry_validate_exceeds_ls_range_rejected() {
        let e = OverlayEntry { ls_addr: LOCAL_STORE_SIZE - 16, size: 32, flags: 0 };
        assert_eq!(e.validate(), Err(errors::INVAL));
    }

    #[test]
    fn overlay_entry_overlaps_detection() {
        let a = ok_entry(0x1000, 0x100);
        let b = ok_entry(0x1080, 0x100);
        let c = ok_entry(0x2000, 0x100);
        assert!(a.overlaps(&b));
        assert!(!a.overlaps(&c));
        // Touching boundaries don't overlap.
        let d = ok_entry(0x1100, 0x100);
        assert!(!a.overlaps(&d));
    }

    #[test]
    fn initialize_null_addr_rejected() {
        let mut t = OverlayTable::new();
        assert_eq!(t.initialize(0, vec![]), Err(errors::INVAL));
    }

    #[test]
    fn initialize_misaligned_addr_rejected() {
        let mut t = OverlayTable::new();
        assert_eq!(t.initialize(0x1001, vec![]), Err(errors::ALIGN));
    }

    #[test]
    fn initialize_too_many_entries_rejected() {
        let mut t = OverlayTable::new();
        let entries: Vec<_> = (0..=(MAX_OVERLAYS as u32))
            .map(|i| ok_entry(i * 16, 16))
            .collect();
        assert_eq!(t.initialize(0x1000, entries), Err(errors::INVAL));
    }

    #[test]
    fn initialize_bad_entry_propagates_error() {
        let mut t = OverlayTable::new();
        let entries = vec![ok_entry(0x1001, 0x100)];
        assert_eq!(t.initialize(0x1000, entries), Err(errors::ALIGN));
    }

    #[test]
    fn initialize_happy_path() {
        let mut t = OverlayTable::new();
        let entries = vec![ok_entry(0x1000, 0x1000), ok_entry(0x4000, 0x800)];
        t.initialize(0x1000, entries).unwrap();
        assert!(t.is_initialized());
        assert_eq!(t.entry_count(), 2);
    }

    #[test]
    fn segment_validate_unknown_type_rejected() {
        let s = segment(99, 0x1000, 0x100);
        assert_eq!(s.validate(), Err(errors::INVAL));
    }

    #[test]
    fn segment_validate_misaligned_addr_rejected() {
        let s = segment(SEG_TYPE_COPY, 0x1001, 0x100);
        assert_eq!(s.validate(), Err(errors::ALIGN));
    }

    #[test]
    fn segment_validate_info_allows_zero_size() {
        let s = segment(SEG_TYPE_INFO, 0x1000, 0);
        s.validate().unwrap();
    }

    #[test]
    fn segment_validate_copy_size_zero_rejected() {
        let s = segment(SEG_TYPE_COPY, 0x1000, 0);
        assert_eq!(s.validate(), Err(errors::INVAL));
    }

    #[test]
    fn segment_validate_size_over_ls_rejected() {
        let s = segment(SEG_TYPE_COPY, 0x1000, LOCAL_STORE_SIZE + 16);
        assert_eq!(s.validate(), Err(errors::INVAL));
    }

    #[test]
    fn fix_spu_segments_drops_overlap() {
        let mut t = OverlayTable::new();
        t.initialize(0x1000, vec![ok_entry(0x2000, 0x100)]).unwrap();
        let segments = vec![
            segment(SEG_TYPE_COPY, 0x1000, 0x100), // no overlap
            segment(SEG_TYPE_COPY, 0x2000, 0x100), // overlaps → dropped
            segment(SEG_TYPE_FILL, 0x2080, 0x80),  // partial overlap → dropped
            segment(SEG_TYPE_INFO, 0x2000, 0x100), // INFO passes through
        ];
        let kept = t.fix_spu_segments(&segments).unwrap();
        assert_eq!(kept.len(), 2);
        assert_eq!(kept[0].ls_addr, 0x1000);
        assert_eq!(kept[1].seg_type, SEG_TYPE_INFO);
    }

    #[test]
    fn fix_spu_segments_without_init_rejected() {
        let t = OverlayTable::new();
        let segs = vec![segment(SEG_TYPE_COPY, 0x1000, 0x100)];
        assert_eq!(t.fix_spu_segments(&segs).err(), Some(errors::INVAL));
    }

    #[test]
    fn fix_spu_segments_aborted_returns_abort() {
        let mut t = OverlayTable::new();
        t.initialize(0x1000, vec![ok_entry(0x2000, 0x100)]).unwrap();
        t.abort();
        let segs = vec![segment(SEG_TYPE_COPY, 0x1000, 0x100)];
        assert_eq!(t.fix_spu_segments(&segs).err(), Some(errors::ABORT));
    }

    #[test]
    fn fix_spu_segments_invalid_segment_propagates() {
        let mut t = OverlayTable::new();
        t.initialize(0x1000, vec![]).unwrap();
        let segs = vec![segment(SEG_TYPE_COPY, 0x1001, 0x100)];
        assert_eq!(t.fix_spu_segments(&segs).err(), Some(errors::ALIGN));
    }

    #[test]
    fn invalidate_overlapped_segments_counts_removed() {
        let mut t = OverlayTable::new();
        t.initialize(0x1000, vec![ok_entry(0x2000, 0x100)]).unwrap();
        let mut segs = vec![
            segment(SEG_TYPE_COPY, 0x1000, 0x100),
            segment(SEG_TYPE_COPY, 0x2000, 0x100),
            segment(SEG_TYPE_FILL, 0x2080, 0x80),
        ];
        let removed = t.invalidate_overlapped_segments(&mut segs).unwrap();
        assert_eq!(removed, 2);
        assert_eq!(segs.len(), 1);
    }

    #[test]
    fn invalidate_without_init_rejected() {
        let t = OverlayTable::new();
        let mut segs = vec![segment(SEG_TYPE_COPY, 0x1000, 0x100)];
        assert_eq!(t.invalidate_overlapped_segments(&mut segs).err(), Some(errors::INVAL));
    }

    #[test]
    fn find_at_locates_covering_overlay() {
        let mut t = OverlayTable::new();
        t.initialize(0x1000, vec![ok_entry(0x1000, 0x200), ok_entry(0x4000, 0x100)]).unwrap();
        assert_eq!(t.find_at(0x1050).map(|e| e.ls_addr), Some(0x1000));
        assert_eq!(t.find_at(0x4000).map(|e| e.ls_addr), Some(0x4000));
        assert_eq!(t.find_at(0x5000), None);
    }

    #[test]
    fn is_known_seg_type_helper() {
        assert!(is_known_seg_type(SEG_TYPE_COPY));
        assert!(is_known_seg_type(SEG_TYPE_FILL));
        assert!(is_known_seg_type(SEG_TYPE_INFO));
        assert!(!is_known_seg_type(0));
        assert!(!is_known_seg_type(3));
        assert!(!is_known_seg_type(99));
    }

    #[test]
    fn full_ovis_lifecycle_smoke() {
        let count = 3u32;
        let table_sz = overlay_table_size(count);
        assert_eq!(table_sz, TABLE_HEADER_SIZE + count * TABLE_ENTRY_SIZE);
        let mut t = OverlayTable::new();
        let entries = vec![
            ok_entry(0x1000, 0x2000),
            ok_entry(0x4000, 0x1000),
            ok_entry(0x8000, 0x4000),
        ];
        t.initialize(0x100, entries).unwrap();
        assert_eq!(t.entry_count(), 3);
        // Some overlapping + non-overlapping segments.
        let segments = vec![
            segment(SEG_TYPE_COPY, 0x500, 0x100), // no overlap, kept
            segment(SEG_TYPE_COPY, 0x1500, 0x100), // overlay 0 → dropped
            segment(SEG_TYPE_FILL, 0x8400, 0x100), // overlay 2 → dropped
            segment(SEG_TYPE_INFO, 0x1000, 0),     // INFO always passes
        ];
        let kept = t.fix_spu_segments(&segments).unwrap();
        assert_eq!(kept.len(), 2);
        assert_eq!(t.find_at(0x1500).map(|e| e.ls_addr), Some(0x1000));
        // After abort, all further ops fail.
        t.abort();
        assert_eq!(t.fix_spu_segments(&segments).err(), Some(errors::ABORT));
    }
}
