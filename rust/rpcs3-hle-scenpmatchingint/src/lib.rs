//! Rust port of `rpcs3/Emu/Cell/Modules/sceNpMatchingInt.cpp` — PS3 internal
//! NP matching PRX (room/lobby operations).
//!
//! Upstream is a **thin wrapper** over the `sceNp` module's real matching
//! functions (`matching_join_room`, `matching_create_room`, etc.), with a
//! few entries that are still `UNIMPLEMENTED_FUNC` stubs returning CELL_OK.
//! The PRX exists to resolve symbol name conflicts with `sceNp` — several
//! entries are registered via `REG_FNID` with a different visible name than
//! the C++ function symbol (the `OLD_*` variants).
//!
//! 10 entries total. The forwarding targets live in `sceNp` which is already
//! ported as its own crate; this crate is just a dispatch layer that
//! preserves the FNID → handler mapping.
//!
//! `no_std` + `alloc`. Single dep: `rpcs3-emu-types`.

#![no_std]

extern crate alloc;

use alloc::vec::Vec;
use rpcs3_emu_types::CellError;

pub const MODULE_NAME: &str = "sceNpMatchingInt";

/// Matching entry registration kind — preserves the upstream quirk where 3
/// entries use `REG_FNID` because the public FNID differs from the Rust
/// function symbol (C++ uses `OLD_*` prefixes to dodge name collisions with
/// the main `sceNp` module).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchingRegKind {
    /// Standard `REG_FUNC(module, symbol)` — FNID = symbol name.
    Func,
    /// `REG_FNID(module, "public_name", internal_symbol)` — FNID differs.
    Fnid,
}

/// Entry descriptor: `(fnid, impl_symbol, kind)`.
pub type MatchingEntry = (&'static str, &'static str, MatchingRegKind);

/// 10 entries in exact registration order (cpp:86-99), preserving the
/// `REG_FUNC` vs `REG_FNID` distinction.
pub const REGISTERED_ENTRIES: &[MatchingEntry] = &[
    ("sceNpMatchingCancelRequest", "sceNpMatchingCancelRequest", MatchingRegKind::Func),
    ("sceNpMatchingGetRoomMemberList", "sceNpMatchingGetRoomMemberList", MatchingRegKind::Func),
    ("sceNpMatchingJoinRoomWithoutGUI", "sceNpMatchingJoinRoomWithoutGUI", MatchingRegKind::Func),
    ("sceNpMatchingJoinRoomGUI", "OLD_sceNpMatchingJoinRoomGUI", MatchingRegKind::Fnid),
    ("sceNpMatchingSetRoomInfoNoLimit", "OLD_sceNpMatchingSetRoomInfoNoLimit", MatchingRegKind::Fnid),
    ("sceNpMatchingGetRoomListWithoutGUI", "sceNpMatchingGetRoomListWithoutGUI", MatchingRegKind::Func),
    ("sceNpMatchingGetRoomListGUI", "sceNpMatchingGetRoomListGUI", MatchingRegKind::Func),
    ("sceNpMatchingGetRoomInfoNoLimit", "OLD_sceNpMatchingGetRoomInfoNoLimit", MatchingRegKind::Fnid),
    ("sceNpMatchingCancelRequestGUI", "sceNpMatchingCancelRequestGUI", MatchingRegKind::Func),
    ("sceNpMatchingSendRoomMessage", "sceNpMatchingSendRoomMessage", MatchingRegKind::Func),
    ("sceNpMatchingCreateRoomWithoutGUI", "sceNpMatchingCreateRoomWithoutGUI", MatchingRegKind::Func),
];

/// Exposed for convenience — 11 entries registered (10 if you collapse
/// duplicate aliases, but `cellSpursJq` counts everything).
pub const REGISTERED_ENTRY_POINTS: &[&str] = &[
    "sceNpMatchingCancelRequest",
    "sceNpMatchingGetRoomMemberList",
    "sceNpMatchingJoinRoomWithoutGUI",
    "sceNpMatchingJoinRoomGUI",
    "sceNpMatchingSetRoomInfoNoLimit",
    "sceNpMatchingGetRoomListWithoutGUI",
    "sceNpMatchingGetRoomListGUI",
    "sceNpMatchingGetRoomInfoNoLimit",
    "sceNpMatchingCancelRequestGUI",
    "sceNpMatchingSendRoomMessage",
    "sceNpMatchingCreateRoomWithoutGUI",
];

// ---------------------------------------------------------------------------
// Matching backend trait — all functional entries delegate to `sceNp` real
// matching helpers. The crate exposes these as a pluggable trait; the real
// implementation lives in the `sceNp` crate, while tests use `NullBackend`.
// ---------------------------------------------------------------------------

/// Mirror of upstream `matching_get_room_list`'s `is_gui` parameter. Upstream
/// quirk: `sceNpMatchingGetRoomListGUI` is registered with `is_gui=false`
/// (cpp:54) despite the "GUI" name — preserved.
pub trait MatchingBackend {
    fn get_room_member_list(&mut self, ctx_id: u32) -> Result<(), CellError>;
    fn join_room(&mut self, ctx_id: u32) -> Result<(), CellError>;
    fn set_room_info(&mut self, ctx_id: u32, no_limit: bool) -> Result<(), CellError>;
    fn get_room_list(&mut self, ctx_id: u32, is_gui: bool) -> Result<(), CellError>;
    fn get_room_info(&mut self, ctx_id: u32, no_limit: bool) -> Result<(), CellError>;
    fn create_room(&mut self, ctx_id: u32) -> Result<(), CellError>;
}

#[derive(Debug, Default)]
pub struct NullBackend {
    pub get_room_member_list_calls: u64,
    pub join_room_calls: u64,
    pub set_room_info_calls: u64,
    pub get_room_list_calls: u64,
    pub get_room_list_is_gui_values: Vec<bool>,
    pub set_room_info_no_limit_values: Vec<bool>,
    pub get_room_info_calls: u64,
    pub create_room_calls: u64,
}

impl MatchingBackend for NullBackend {
    fn get_room_member_list(&mut self, _ctx_id: u32) -> Result<(), CellError> {
        self.get_room_member_list_calls = self.get_room_member_list_calls.saturating_add(1);
        Ok(())
    }
    fn join_room(&mut self, _ctx_id: u32) -> Result<(), CellError> {
        self.join_room_calls = self.join_room_calls.saturating_add(1);
        Ok(())
    }
    fn set_room_info(&mut self, _ctx_id: u32, no_limit: bool) -> Result<(), CellError> {
        self.set_room_info_calls = self.set_room_info_calls.saturating_add(1);
        self.set_room_info_no_limit_values.push(no_limit);
        Ok(())
    }
    fn get_room_list(&mut self, _ctx_id: u32, is_gui: bool) -> Result<(), CellError> {
        self.get_room_list_calls = self.get_room_list_calls.saturating_add(1);
        self.get_room_list_is_gui_values.push(is_gui);
        Ok(())
    }
    fn get_room_info(&mut self, _ctx_id: u32, _no_limit: bool) -> Result<(), CellError> {
        self.get_room_info_calls = self.get_room_info_calls.saturating_add(1);
        Ok(())
    }
    fn create_room(&mut self, _ctx_id: u32) -> Result<(), CellError> {
        self.create_room_calls = self.create_room_calls.saturating_add(1);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Manager.
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct SceNpMatchingInt {
    pub cancel_request_calls: u64,
    pub get_room_member_list_calls: u64,
    pub join_room_without_gui_calls: u64,
    pub old_join_room_gui_calls: u64,
    pub old_set_room_info_no_limit_calls: u64,
    pub get_room_list_without_gui_calls: u64,
    pub get_room_list_gui_calls: u64,
    pub old_get_room_info_no_limit_calls: u64,
    pub cancel_request_gui_calls: u64,
    pub send_room_message_calls: u64,
    pub create_room_without_gui_calls: u64,
}

impl SceNpMatchingInt {
    pub fn new() -> Self {
        Self::default()
    }

    // -- Pure stubs ------------------------------------------------------

    pub fn cancel_request(&mut self) -> Result<(), CellError> {
        self.cancel_request_calls = self.cancel_request_calls.saturating_add(1);
        Ok(())
    }

    pub fn cancel_request_gui(&mut self) -> Result<(), CellError> {
        self.cancel_request_gui_calls = self.cancel_request_gui_calls.saturating_add(1);
        Ok(())
    }

    pub fn send_room_message(&mut self) -> Result<(), CellError> {
        self.send_room_message_calls = self.send_room_message_calls.saturating_add(1);
        Ok(())
    }

    // -- Delegating wrappers --------------------------------------------

    pub fn get_room_member_list<B: MatchingBackend>(
        &mut self,
        backend: &mut B,
        ctx_id: u32,
    ) -> Result<(), CellError> {
        self.get_room_member_list_calls = self.get_room_member_list_calls.saturating_add(1);
        backend.get_room_member_list(ctx_id)
    }

    pub fn join_room_without_gui<B: MatchingBackend>(
        &mut self,
        backend: &mut B,
        ctx_id: u32,
    ) -> Result<(), CellError> {
        self.join_room_without_gui_calls = self.join_room_without_gui_calls.saturating_add(1);
        backend.join_room(ctx_id)
    }

    pub fn old_join_room_gui<B: MatchingBackend>(
        &mut self,
        backend: &mut B,
        ctx_id: u32,
    ) -> Result<(), CellError> {
        self.old_join_room_gui_calls = self.old_join_room_gui_calls.saturating_add(1);
        backend.join_room(ctx_id)
    }

    /// `OLD_sceNpMatchingSetRoomInfoNoLimit` always passes `no_limit=false`
    /// to the backend (cpp:38 `matching_set_room_info(..., false)`).
    pub fn old_set_room_info_no_limit<B: MatchingBackend>(
        &mut self,
        backend: &mut B,
        ctx_id: u32,
    ) -> Result<(), CellError> {
        self.old_set_room_info_no_limit_calls =
            self.old_set_room_info_no_limit_calls.saturating_add(1);
        backend.set_room_info(ctx_id, false)
    }

    /// `sceNpMatchingGetRoomListWithoutGUI` → backend with `is_gui=false`.
    pub fn get_room_list_without_gui<B: MatchingBackend>(
        &mut self,
        backend: &mut B,
        ctx_id: u32,
    ) -> Result<(), CellError> {
        self.get_room_list_without_gui_calls =
            self.get_room_list_without_gui_calls.saturating_add(1);
        backend.get_room_list(ctx_id, false)
    }

    /// `sceNpMatchingGetRoomListGUI` → backend with `is_gui=false` (!!)
    /// Upstream quirk cpp:54 preserved — despite the "GUI" suffix, the
    /// backend is called with `is_gui=false` (same as WithoutGUI variant).
    pub fn get_room_list_gui<B: MatchingBackend>(
        &mut self,
        backend: &mut B,
        ctx_id: u32,
    ) -> Result<(), CellError> {
        self.get_room_list_gui_calls = self.get_room_list_gui_calls.saturating_add(1);
        backend.get_room_list(ctx_id, false)
    }

    pub fn old_get_room_info_no_limit<B: MatchingBackend>(
        &mut self,
        backend: &mut B,
        ctx_id: u32,
    ) -> Result<(), CellError> {
        self.old_get_room_info_no_limit_calls =
            self.old_get_room_info_no_limit_calls.saturating_add(1);
        backend.get_room_info(ctx_id, false)
    }

    pub fn create_room_without_gui<B: MatchingBackend>(
        &mut self,
        backend: &mut B,
        ctx_id: u32,
    ) -> Result<(), CellError> {
        self.create_room_without_gui_calls =
            self.create_room_without_gui_calls.saturating_add(1);
        backend.create_room(ctx_id)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn module_name() {
        assert_eq!(MODULE_NAME, "sceNpMatchingInt");
    }

    #[test]
    fn registered_entries_preserve_reg_kind() {
        assert_eq!(REGISTERED_ENTRIES.len(), 11);
        // Func entries (8)
        assert_eq!(REGISTERED_ENTRIES[0], ("sceNpMatchingCancelRequest", "sceNpMatchingCancelRequest", MatchingRegKind::Func));
        assert_eq!(REGISTERED_ENTRIES[1].2, MatchingRegKind::Func);
        assert_eq!(REGISTERED_ENTRIES[2].2, MatchingRegKind::Func);
        assert_eq!(REGISTERED_ENTRIES[5].2, MatchingRegKind::Func);
        assert_eq!(REGISTERED_ENTRIES[6].2, MatchingRegKind::Func);
        assert_eq!(REGISTERED_ENTRIES[8].2, MatchingRegKind::Func);
        assert_eq!(REGISTERED_ENTRIES[9].2, MatchingRegKind::Func);
        assert_eq!(REGISTERED_ENTRIES[10].2, MatchingRegKind::Func);
        // Fnid entries (3) — registered via REG_FNID
        assert_eq!(REGISTERED_ENTRIES[3], ("sceNpMatchingJoinRoomGUI", "OLD_sceNpMatchingJoinRoomGUI", MatchingRegKind::Fnid));
        assert_eq!(REGISTERED_ENTRIES[4], ("sceNpMatchingSetRoomInfoNoLimit", "OLD_sceNpMatchingSetRoomInfoNoLimit", MatchingRegKind::Fnid));
        assert_eq!(REGISTERED_ENTRIES[7], ("sceNpMatchingGetRoomInfoNoLimit", "OLD_sceNpMatchingGetRoomInfoNoLimit", MatchingRegKind::Fnid));
    }

    #[test]
    fn entry_points_order_matches_cpp() {
        assert_eq!(REGISTERED_ENTRY_POINTS.len(), 11);
        assert_eq!(REGISTERED_ENTRY_POINTS[0], "sceNpMatchingCancelRequest");
        assert_eq!(REGISTERED_ENTRY_POINTS[10], "sceNpMatchingCreateRoomWithoutGUI");
    }

    #[test]
    fn reg_fnid_count_is_three() {
        let fnid_count = REGISTERED_ENTRIES
            .iter()
            .filter(|(_, _, k)| *k == MatchingRegKind::Fnid)
            .count();
        assert_eq!(fnid_count, 3);
    }

    #[test]
    fn pure_stubs_count() {
        let mut m = SceNpMatchingInt::new();
        m.cancel_request().unwrap();
        m.cancel_request_gui().unwrap();
        m.send_room_message().unwrap();
        assert_eq!(m.cancel_request_calls, 1);
        assert_eq!(m.cancel_request_gui_calls, 1);
        assert_eq!(m.send_room_message_calls, 1);
    }

    #[test]
    fn get_room_member_list_delegates() {
        let mut m = SceNpMatchingInt::new();
        let mut b = NullBackend::default();
        m.get_room_member_list(&mut b, 42).unwrap();
        assert_eq!(m.get_room_member_list_calls, 1);
        assert_eq!(b.get_room_member_list_calls, 1);
    }

    #[test]
    fn join_room_both_variants_delegate_to_same_backend() {
        let mut m = SceNpMatchingInt::new();
        let mut b = NullBackend::default();
        m.join_room_without_gui(&mut b, 1).unwrap();
        m.old_join_room_gui(&mut b, 2).unwrap();
        // Both flavors hit the same `join_room` — 2 total.
        assert_eq!(b.join_room_calls, 2);
    }

    #[test]
    fn old_set_room_info_no_limit_passes_false() {
        let mut m = SceNpMatchingInt::new();
        let mut b = NullBackend::default();
        m.old_set_room_info_no_limit(&mut b, 1).unwrap();
        // cpp:38 passes `false` for the no_limit parameter despite the name.
        assert_eq!(b.set_room_info_no_limit_values, vec![false]);
    }

    #[test]
    fn get_room_list_gui_and_without_gui_both_pass_false() {
        // Upstream cpp:46/54 both pass `is_gui=false` to matching_get_room_list.
        let mut m = SceNpMatchingInt::new();
        let mut b = NullBackend::default();
        m.get_room_list_without_gui(&mut b, 1).unwrap();
        m.get_room_list_gui(&mut b, 2).unwrap();
        assert_eq!(b.get_room_list_is_gui_values, vec![false, false]);
    }

    #[test]
    fn old_get_room_info_no_limit_delegates() {
        let mut m = SceNpMatchingInt::new();
        let mut b = NullBackend::default();
        m.old_get_room_info_no_limit(&mut b, 5).unwrap();
        assert_eq!(b.get_room_info_calls, 1);
    }

    #[test]
    fn create_room_without_gui_delegates() {
        let mut m = SceNpMatchingInt::new();
        let mut b = NullBackend::default();
        m.create_room_without_gui(&mut b, 9).unwrap();
        assert_eq!(b.create_room_calls, 1);
    }

    #[test]
    fn backend_error_propagates() {
        struct ErrBackend;
        impl MatchingBackend for ErrBackend {
            fn get_room_member_list(&mut self, _: u32) -> Result<(), CellError> { Err(CellError(0xDEAD)) }
            fn join_room(&mut self, _: u32) -> Result<(), CellError> { Err(CellError(0xBEEF)) }
            fn set_room_info(&mut self, _: u32, _: bool) -> Result<(), CellError> { Ok(()) }
            fn get_room_list(&mut self, _: u32, _: bool) -> Result<(), CellError> { Ok(()) }
            fn get_room_info(&mut self, _: u32, _: bool) -> Result<(), CellError> { Ok(()) }
            fn create_room(&mut self, _: u32) -> Result<(), CellError> { Ok(()) }
        }
        let mut m = SceNpMatchingInt::new();
        let mut b = ErrBackend;
        assert_eq!(
            m.get_room_member_list(&mut b, 1),
            Err(CellError(0xDEAD))
        );
        assert_eq!(m.join_room_without_gui(&mut b, 1), Err(CellError(0xBEEF)));
    }

    #[test]
    fn full_lifecycle_smoke() {
        let mut m = SceNpMatchingInt::new();
        let mut b = NullBackend::default();
        // Create room, join, get list, get info, send message, leave.
        m.create_room_without_gui(&mut b, 1).unwrap();
        m.get_room_list_without_gui(&mut b, 1).unwrap();
        m.get_room_list_gui(&mut b, 1).unwrap();
        m.get_room_member_list(&mut b, 1).unwrap();
        m.old_get_room_info_no_limit(&mut b, 1).unwrap();
        m.join_room_without_gui(&mut b, 1).unwrap();
        m.old_join_room_gui(&mut b, 1).unwrap();
        m.old_set_room_info_no_limit(&mut b, 1).unwrap();
        m.send_room_message().unwrap();
        m.cancel_request_gui().unwrap();
        m.cancel_request().unwrap();

        assert_eq!(b.create_room_calls, 1);
        assert_eq!(b.get_room_list_calls, 2);
        assert_eq!(b.get_room_member_list_calls, 1);
        assert_eq!(b.get_room_info_calls, 1);
        assert_eq!(b.join_room_calls, 2);
        assert_eq!(b.set_room_info_calls, 1);
    }
}
