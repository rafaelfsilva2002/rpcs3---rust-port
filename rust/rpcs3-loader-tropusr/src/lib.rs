//! `rpcs3-loader-tropusr` — Rust port of `rpcs3/Loader/TROPUSR.cpp`.
//!
//! Parses TROPUSR.DAT — the per-user "trophy progress" file inside every
//! game's trophy VFS directory. Contains two tables: type 4 records
//! describe trophy metadata (grade, platinum link), type 6 records track
//! unlock state and timestamps.
//!
//! Frozen here:
//!
//! - `TROPUSR_MAGIC = 0x818F_54AD` (cpp:9).
//! - Header / table-header / entry-4 / entry-6 layouts (h:6..58) with
//!   compile-time size assertions.
//! - `TrophyGrade` enum (Unknown=0, Platinum=1, Gold=2, Silver=3,
//!   Bronze=4).
//! - `SCE_NP_TROPHY_INVALID_TROPHY_ID = 0xFFFF_FFFF` sentinel used by
//!   `trophy_pid` when no platinum link (h:36).
//! - Unlock / lock state transitions and counters.

use core::mem::size_of;

pub const TROPUSR_MAGIC: u32 = 0x818F_54AD;
pub const SCE_NP_TROPHY_INVALID_TROPHY_ID: u32 = 0xFFFF_FFFF;

/// TrophyGrade discriminants from `TROPUSR.h:68..75`.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrophyGrade {
    Unknown = 0,
    Platinum = 1,
    Gold = 2,
    Silver = 3,
    Bronze = 4,
}

/// Trophy unlock state (cpp only uses 0 / 1).
pub const TROPHY_STATE_LOCKED: u32 = 0;
pub const TROPHY_STATE_UNLOCKED: u32 = 1;

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TropusrHeader {
    pub magic: u32,
    pub unk1: u32,
    pub tables_count: u32,
    pub unk2: u32,
    pub reserved: [u8; 32],
}

impl Default for TropusrHeader {
    fn default() -> Self {
        Self { magic: 0, unk1: 0, tables_count: 0, unk2: 0, reserved: [0; 32] }
    }
}

const _: () = assert!(size_of::<TropusrHeader>() == 48);

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TropusrTableHeader {
    pub type_: u32,
    pub entries_size: u32,
    pub unk1: u32,
    pub entries_count: u32,
    pub offset: u64,
    pub reserved: u64,
}

const _: () = assert!(size_of::<TropusrTableHeader>() == 32);

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TropusrEntry4 {
    // Header
    pub entry_type: u32,     // Always 0x4
    pub entry_size: u32,     // Always 0x50
    pub entry_id: u32,
    pub entry_unk1: u32,
    // Contents
    pub trophy_id: u32,
    pub trophy_grade: u32,
    pub trophy_pid: u32,
    pub unk6: [u8; 68],
}

impl Default for TropusrEntry4 {
    fn default() -> Self {
        Self {
            entry_type: 4,
            entry_size: 0x50,
            entry_id: 0,
            entry_unk1: 0,
            trophy_id: 0,
            trophy_grade: 0,
            trophy_pid: SCE_NP_TROPHY_INVALID_TROPHY_ID,
            unk6: [0; 68],
        }
    }
}

const _: () = assert!(size_of::<TropusrEntry4>() == 0x60);

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TropusrEntry6 {
    pub entry_type: u32,     // Always 6
    pub entry_size: u32,     // Always 0x60
    pub entry_id: u32,
    pub entry_unk1: u32,
    pub trophy_id: u32,
    pub trophy_state: u32,   // 0 locked / 1 unlocked
    pub unk4: u32,
    pub unk5: u32,
    pub timestamp1: u64,
    pub timestamp2: u64,
    pub unk6: [u8; 64],
}

impl Default for TropusrEntry6 {
    fn default() -> Self {
        Self {
            entry_type: 6,
            entry_size: 0x60,
            entry_id: 0,
            entry_unk1: 0,
            trophy_id: 0,
            trophy_state: TROPHY_STATE_LOCKED,
            unk4: 0,
            unk5: 0,
            timestamp1: 0,
            timestamp2: 0,
            unk6: [0; 64],
        }
    }
}

const _: () = assert!(size_of::<TropusrEntry6>() == 0x70);

/// `TROPUSRLoader::load_result` (h:92..96).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LoadResult {
    pub discarded_existing: bool,
    pub success: bool,
}

/// In-memory TROPUSR.DAT. Vectors of entries for each of the two tables.
#[derive(Debug, Default, Clone)]
pub struct TropusrLoader {
    pub header: TropusrHeader,
    pub table_headers: Vec<TropusrTableHeader>,
    pub table4: Vec<TropusrEntry4>,
    pub table6: Vec<TropusrEntry6>,
}

impl TropusrLoader {
    /// `GetTrophiesCount` (cpp:101) — reports the number of type-6 entries.
    #[must_use]
    pub fn trophies_count(&self) -> u32 {
        self.table6.len() as u32
    }

    /// `GetUnlockedTrophiesCount` — entries whose `trophy_state == 1`.
    #[must_use]
    pub fn unlocked_trophies_count(&self) -> u32 {
        self.table6
            .iter()
            .filter(|e| e.trophy_state == TROPHY_STATE_UNLOCKED)
            .count() as u32
    }

    /// `GetTrophyGrade(id)` — looks up in table4. Returns
    /// `SCE_NP_TROPHY_INVALID_TROPHY_ID` for unknown ids.
    #[must_use]
    pub fn trophy_grade(&self, id: u32) -> u32 {
        self.table4
            .iter()
            .find(|e| e.trophy_id == id)
            .map_or(SCE_NP_TROPHY_INVALID_TROPHY_ID, |e| e.trophy_grade)
    }

    /// `GetTrophyUnlockState(id)` — looks up in table6.
    #[must_use]
    pub fn trophy_unlock_state(&self, id: u32) -> u32 {
        self.table6
            .iter()
            .find(|e| e.trophy_id == id)
            .map_or(TROPHY_STATE_LOCKED, |e| e.trophy_state)
    }

    /// `GetTrophyTimestamp(id)` — returns `timestamp1` on hit, 0 otherwise.
    #[must_use]
    pub fn trophy_timestamp(&self, id: u32) -> u64 {
        self.table6
            .iter()
            .find(|e| e.trophy_id == id)
            .map_or(0, |e| e.timestamp1)
    }

    /// `UnlockTrophy(id, t1, t2)` — flips state + writes timestamps.
    /// Returns false if the trophy isn't tracked.
    pub fn unlock_trophy(&mut self, id: u32, ts1: u64, ts2: u64) -> bool {
        if let Some(e) = self.table6.iter_mut().find(|e| e.trophy_id == id) {
            e.trophy_state = TROPHY_STATE_UNLOCKED;
            e.timestamp1 = ts1;
            e.timestamp2 = ts2;
            true
        } else {
            false
        }
    }

    /// `LockTrophy(id)` — reverts unlock, clears timestamps.
    pub fn lock_trophy(&mut self, id: u32) -> bool {
        if let Some(e) = self.table6.iter_mut().find(|e| e.trophy_id == id) {
            e.trophy_state = TROPHY_STATE_LOCKED;
            e.timestamp1 = 0;
            e.timestamp2 = 0;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn struct_sizes_frozen() {
        assert_eq!(size_of::<TropusrHeader>(), 48);
        assert_eq!(size_of::<TropusrTableHeader>(), 32);
        assert_eq!(size_of::<TropusrEntry4>(), 0x60);
        assert_eq!(size_of::<TropusrEntry6>(), 0x70);
    }

    #[test]
    fn magic_value() {
        assert_eq!(TROPUSR_MAGIC, 0x818F_54AD);
    }

    #[test]
    fn trophy_grade_discriminants() {
        assert_eq!(TrophyGrade::Unknown as u32, 0);
        assert_eq!(TrophyGrade::Platinum as u32, 1);
        assert_eq!(TrophyGrade::Gold as u32, 2);
        assert_eq!(TrophyGrade::Silver as u32, 3);
        assert_eq!(TrophyGrade::Bronze as u32, 4);
    }

    #[test]
    fn invalid_trophy_sentinel() {
        assert_eq!(SCE_NP_TROPHY_INVALID_TROPHY_ID, 0xFFFF_FFFF);
    }

    #[test]
    fn entry4_defaults_include_entry_type_and_size() {
        let e = TropusrEntry4::default();
        assert_eq!(e.entry_type, 4);
        assert_eq!(e.entry_size, 0x50);
        assert_eq!(e.trophy_pid, SCE_NP_TROPHY_INVALID_TROPHY_ID);
    }

    #[test]
    fn entry6_defaults() {
        let e = TropusrEntry6::default();
        assert_eq!(e.entry_type, 6);
        assert_eq!(e.entry_size, 0x60);
        assert_eq!(e.trophy_state, TROPHY_STATE_LOCKED);
    }

    #[test]
    fn trophies_count_empty() {
        let l = TropusrLoader::default();
        assert_eq!(l.trophies_count(), 0);
        assert_eq!(l.unlocked_trophies_count(), 0);
    }

    #[test]
    fn unlock_and_lock_transitions() {
        let mut l = TropusrLoader::default();
        let mut e = TropusrEntry6::default();
        e.trophy_id = 42;
        l.table6.push(e);
        assert_eq!(l.trophies_count(), 1);
        assert_eq!(l.unlocked_trophies_count(), 0);
        assert_eq!(l.trophy_unlock_state(42), TROPHY_STATE_LOCKED);

        assert!(l.unlock_trophy(42, 0x1111_2222, 0x3333_4444));
        assert_eq!(l.unlocked_trophies_count(), 1);
        assert_eq!(l.trophy_unlock_state(42), TROPHY_STATE_UNLOCKED);
        assert_eq!(l.trophy_timestamp(42), 0x1111_2222);

        assert!(l.lock_trophy(42));
        assert_eq!(l.unlocked_trophies_count(), 0);
        assert_eq!(l.trophy_timestamp(42), 0);
    }

    #[test]
    fn unknown_trophy_paths() {
        let l = TropusrLoader::default();
        assert_eq!(l.trophy_grade(99), SCE_NP_TROPHY_INVALID_TROPHY_ID);
        assert_eq!(l.trophy_unlock_state(99), TROPHY_STATE_LOCKED);
        assert_eq!(l.trophy_timestamp(99), 0);

        let mut l = l;
        assert!(!l.unlock_trophy(99, 1, 2));
        assert!(!l.lock_trophy(99));
    }

    #[test]
    fn trophy_grade_lookup_in_table4() {
        let mut l = TropusrLoader::default();
        let mut e = TropusrEntry4::default();
        e.trophy_id = 7;
        e.trophy_grade = TrophyGrade::Gold as u32;
        l.table4.push(e);
        assert_eq!(l.trophy_grade(7), TrophyGrade::Gold as u32);
        assert_eq!(l.trophy_grade(99), SCE_NP_TROPHY_INVALID_TROPHY_ID);
    }

    #[test]
    fn load_result_defaults_to_failure() {
        let r = LoadResult::default();
        assert!(!r.success);
        assert!(!r.discarded_existing);
    }
}
