//! `rpcs3-hle-cellsysutil` — system-utility HLE module.
//!
//! Ports `rpcs3/Emu/Cell/Modules/cellSysutil.cpp`. Every game calls
//! into cellSysutil for callback registration (home button, drawing
//! begin/end, system menu) and system parameter lookups (language,
//! nickname, date format).
//!
//! ## Scope
//!
//! * `cellSysutilCheckCallback` — drain pending callbacks.
//! * `cellSysutilRegisterCallback(slot, fn, user_data)`.
//! * `cellSysutilUnregisterCallback(slot)`.
//! * `cellSysutilGetSystemParamInt(id, value*)`.
//! * `cellSysutilGetSystemParamString(id, buf, bufsize)`.

use rpcs3_emu_types::CellError;

// =====================================================================
// Error codes (cellSysutil facility 0x8002B__)
// =====================================================================

pub const CELL_SYSUTIL_ERROR_TYPE: CellError = CellError(0x8002_B101);
pub const CELL_SYSUTIL_ERROR_VALUE: CellError = CellError(0x8002_B102);
pub const CELL_SYSUTIL_ERROR_SIZE: CellError = CellError(0x8002_B103);
pub const CELL_SYSUTIL_ERROR_NUM: CellError = CellError(0x8002_B104);
pub const CELL_SYSUTIL_ERROR_BUSY: CellError = CellError(0x8002_B105);
pub const CELL_SYSUTIL_ERROR_STATUS: CellError = CellError(0x8002_B106);
pub const CELL_SYSUTIL_ERROR_MEMORY: CellError = CellError(0x8002_B107);

// =====================================================================
// Callback slot constants
// =====================================================================

/// Maximum simultaneous callback slots (cellSysutil.h).
pub const CB_SLOT_MAX: u32 = 8;

// =====================================================================
// Callback event ids
// =====================================================================

#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CallbackEvent {
    RequestExitGame = 0x0101,
    DrawingBegin = 0x0121,
    DrawingEnd = 0x0122,
    SystemMenuOpen = 0x0131,
    SystemMenuClose = 0x0132,
    BgmplaybackPlay = 0x0141,
    BgmplaybackStop = 0x0142,
    NpInvitationSelected = 0x0151,
    NpDataMessageSelected = 0x0152,
    SysBgmplaybackPlay = 0x0161,
    SysBgmplaybackStop = 0x0162,
}

// =====================================================================
// System parameter IDs
// =====================================================================

/// System parameter IDs (mirror `CELL_SYSUTIL_SYSTEMPARAM_ID_*`).
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SysParamId {
    Lang = 0x0111,
    EnterButtonAssign = 0x0112,
    Nickname = 0x0113,
    DateFormat = 0x0114,
    TimeFormat = 0x0115,
    Timezone = 0x0116,
    Summertime = 0x0117,
    GameParentalLevel = 0x0121,
    CurrentUserHasNpAccount = 0x0123,
    CameraPlfreq = 0x0124,
    PadAutoOff = 0x0125,
    CurrentUsername = 0x0126,
    Unknown = 0,
}

impl SysParamId {
    #[must_use]
    pub fn from_i32(v: i32) -> Self {
        match v {
            0x0111 => Self::Lang,
            0x0112 => Self::EnterButtonAssign,
            0x0113 => Self::Nickname,
            0x0114 => Self::DateFormat,
            0x0115 => Self::TimeFormat,
            0x0116 => Self::Timezone,
            0x0117 => Self::Summertime,
            0x0121 => Self::GameParentalLevel,
            0x0123 => Self::CurrentUserHasNpAccount,
            0x0124 => Self::CameraPlfreq,
            0x0125 => Self::PadAutoOff,
            0x0126 => Self::CurrentUsername,
            _ => Self::Unknown,
        }
    }

    /// True for string-typed parameters; false for int-typed.
    #[must_use]
    pub const fn is_string(self) -> bool {
        matches!(self, Self::Nickname | Self::CurrentUsername)
    }
}

// =====================================================================
// Callback storage
// =====================================================================

/// Registered callback — a guest function address plus opaque user data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Callback {
    pub fn_addr: u32,
    pub user_data: u32,
}

/// Ring of pending (event, param) pairs queued for the game to drain
/// via `cellSysutilCheckCallback`.
#[derive(Debug, Default)]
pub struct CallbackQueue {
    pending: std::collections::VecDeque<(u32, u64)>,
}

impl CallbackQueue {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
    pub fn push(&mut self, event: u32, param: u64) {
        self.pending.push_back((event, param));
    }
    pub fn pop(&mut self) -> Option<(u32, u64)> {
        self.pending.pop_front()
    }
    #[must_use]
    pub fn len(&self) -> usize {
        self.pending.len()
    }
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }
}

/// 8-slot callback table. Slot index 0..=7 is valid; anything else
/// yields `CELL_SYSUTIL_ERROR_VALUE`.
#[derive(Debug, Default)]
pub struct CallbackTable {
    slots: [Option<Callback>; CB_SLOT_MAX as usize],
}

impl CallbackTable {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
    pub fn register(&mut self, slot: u32, cb: Callback) -> Result<(), CellError> {
        if slot >= CB_SLOT_MAX {
            return Err(CELL_SYSUTIL_ERROR_VALUE);
        }
        self.slots[slot as usize] = Some(cb);
        Ok(())
    }
    pub fn unregister(&mut self, slot: u32) -> Result<(), CellError> {
        if slot >= CB_SLOT_MAX {
            return Err(CELL_SYSUTIL_ERROR_VALUE);
        }
        self.slots[slot as usize] = None;
        Ok(())
    }
    #[must_use]
    pub fn get(&self, slot: u32) -> Option<Callback> {
        if slot < CB_SLOT_MAX {
            self.slots[slot as usize]
        } else {
            None
        }
    }
    pub fn registered_slots(&self) -> impl Iterator<Item = (u32, Callback)> + '_ {
        self.slots.iter().enumerate().filter_map(|(i, c)| {
            c.map(|cb| (i as u32, cb))
        })
    }
}

// =====================================================================
// SysutilState trait — emu core provides system param data
// =====================================================================

pub trait SysutilState {
    fn get_param_int(&self, id: SysParamId) -> Option<i32>;
    fn get_param_string(&self, id: SysParamId) -> Option<&str>;
    /// Firmware version string (e.g. "04.5500"). Returned by
    /// `cellSysutilGetSystemMediaVer`.
    fn media_ver(&self) -> &str;
}

// =====================================================================
// HLE functions
// =====================================================================

/// `cellSysutilCheckCallback()`. Drains `queue` and invokes each
/// matching callback through the caller's dispatch. Returns the
/// number of events drained.
///
/// The real C++ implementation enqueues PPU callbacks to be run at
/// the next safe point; here we just return the list of
/// `(Callback, event_id, param)` tuples for the emu core to invoke.
#[must_use]
pub fn cell_sysutil_check_callback(
    table: &CallbackTable,
    queue: &mut CallbackQueue,
) -> Vec<PendingDispatch> {
    let mut out = Vec::new();
    while let Some((event, param)) = queue.pop() {
        // Any registered slot receives the event — the real cellSysutil
        // broadcasts to every registered callback slot. Order is slot 0..7.
        for (_, cb) in table.registered_slots() {
            out.push(PendingDispatch { cb, event, param });
        }
    }
    out
}

/// One "please run the callback" directive produced by [`cell_sysutil_check_callback`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PendingDispatch {
    pub cb: Callback,
    pub event: u32,
    pub param: u64,
}

/// `cellSysutilRegisterCallback(slot, fn, user_data)`.
#[must_use]
pub fn cell_sysutil_register_callback(
    table: &mut CallbackTable,
    slot: u32,
    fn_addr: u32,
    user_data: u32,
) -> Result<(), CellError> {
    if fn_addr == 0 {
        return Err(CELL_SYSUTIL_ERROR_VALUE);
    }
    table.register(slot, Callback { fn_addr, user_data })
}

/// `cellSysutilUnregisterCallback(slot)`.
#[must_use]
pub fn cell_sysutil_unregister_callback(
    table: &mut CallbackTable,
    slot: u32,
) -> Result<(), CellError> {
    table.unregister(slot)
}

/// `cellSysutilGetSystemParamInt(id, value*)`.
#[must_use]
pub fn cell_sysutil_get_system_param_int<S: SysutilState + ?Sized>(
    state: &S,
    id: i32,
) -> Result<i32, CellError> {
    let pid = SysParamId::from_i32(id);
    if matches!(pid, SysParamId::Unknown) {
        return Err(CELL_SYSUTIL_ERROR_VALUE);
    }
    if pid.is_string() {
        return Err(CELL_SYSUTIL_ERROR_TYPE);
    }
    state.get_param_int(pid).ok_or(CELL_SYSUTIL_ERROR_VALUE)
}

/// `cellSysutilGetSystemParamString(id, buf, bufsize)`.
/// Returns the requested string; caller copies into guest memory
/// with truncation.
#[must_use]
pub fn cell_sysutil_get_system_param_string<'a, S: SysutilState + ?Sized>(
    state: &'a S,
    id: i32,
    bufsize: u32,
) -> Result<&'a str, CellError> {
    if bufsize == 0 {
        return Err(CELL_SYSUTIL_ERROR_SIZE);
    }
    let pid = SysParamId::from_i32(id);
    if matches!(pid, SysParamId::Unknown) {
        return Err(CELL_SYSUTIL_ERROR_VALUE);
    }
    if !pid.is_string() {
        return Err(CELL_SYSUTIL_ERROR_TYPE);
    }
    state.get_param_string(pid).ok_or(CELL_SYSUTIL_ERROR_VALUE)
}

/// `cellSysutilGetSystemMediaVer(out_buf, bufsize)`.
#[must_use]
pub fn cell_sysutil_get_system_media_ver<'a, S: SysutilState + ?Sized>(
    state: &'a S,
) -> &'a str {
    state.media_ver()
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[derive(Default)]
    struct TestSys {
        ints: HashMap<i32, i32>,
        strings: HashMap<i32, String>,
        media: String,
    }

    impl SysutilState for TestSys {
        fn get_param_int(&self, id: SysParamId) -> Option<i32> {
            self.ints.get(&(id as i32)).copied()
        }
        fn get_param_string(&self, id: SysParamId) -> Option<&str> {
            self.strings.get(&(id as i32)).map(String::as_str)
        }
        fn media_ver(&self) -> &str {
            &self.media
        }
    }

    fn demo_sys() -> TestSys {
        let mut s = TestSys::default();
        s.ints.insert(SysParamId::Lang as i32, 1); // ENG
        s.ints.insert(SysParamId::DateFormat as i32, 0);
        s.strings.insert(SysParamId::Nickname as i32, "TestUser".into());
        s.strings.insert(SysParamId::CurrentUsername as i32, "testuser".into());
        s.media = "04.5500".into();
        s
    }

    // -- CallbackTable -------------------------------------------

    #[test]
    fn register_stores_callback_in_slot() {
        let mut t = CallbackTable::new();
        cell_sysutil_register_callback(&mut t, 0, 0xABCD, 42).unwrap();
        let cb = t.get(0).unwrap();
        assert_eq!(cb.fn_addr, 0xABCD);
        assert_eq!(cb.user_data, 42);
    }

    #[test]
    fn register_in_bad_slot_is_value_error() {
        let mut t = CallbackTable::new();
        assert_eq!(
            cell_sysutil_register_callback(&mut t, 99, 0xABCD, 0),
            Err(CELL_SYSUTIL_ERROR_VALUE)
        );
    }

    #[test]
    fn register_null_fn_is_value_error() {
        let mut t = CallbackTable::new();
        assert_eq!(
            cell_sysutil_register_callback(&mut t, 0, 0, 0),
            Err(CELL_SYSUTIL_ERROR_VALUE)
        );
    }

    #[test]
    fn unregister_clears_slot() {
        let mut t = CallbackTable::new();
        cell_sysutil_register_callback(&mut t, 3, 0xFEED, 1).unwrap();
        cell_sysutil_unregister_callback(&mut t, 3).unwrap();
        assert!(t.get(3).is_none());
    }

    #[test]
    fn unregister_bad_slot_is_value_error() {
        let mut t = CallbackTable::new();
        assert_eq!(
            cell_sysutil_unregister_callback(&mut t, 99),
            Err(CELL_SYSUTIL_ERROR_VALUE)
        );
    }

    #[test]
    fn check_callback_drains_queue_broadcast_to_all_slots() {
        let mut t = CallbackTable::new();
        cell_sysutil_register_callback(&mut t, 0, 0x1111, 0).unwrap();
        cell_sysutil_register_callback(&mut t, 2, 0x2222, 0).unwrap();
        let mut q = CallbackQueue::new();
        q.push(CallbackEvent::RequestExitGame as u32, 0);
        let pending = cell_sysutil_check_callback(&t, &mut q);
        assert_eq!(pending.len(), 2); // 2 slots × 1 event
        assert_eq!(pending[0].event, 0x0101);
        assert!(q.is_empty());
    }

    #[test]
    fn check_callback_empty_queue_returns_empty() {
        let t = CallbackTable::new();
        let mut q = CallbackQueue::new();
        let pending = cell_sysutil_check_callback(&t, &mut q);
        assert!(pending.is_empty());
    }

    #[test]
    fn check_callback_drains_multiple_events() {
        let mut t = CallbackTable::new();
        cell_sysutil_register_callback(&mut t, 0, 0x1111, 0).unwrap();
        let mut q = CallbackQueue::new();
        q.push(CallbackEvent::DrawingBegin as u32, 100);
        q.push(CallbackEvent::DrawingEnd as u32, 200);
        let pending = cell_sysutil_check_callback(&t, &mut q);
        assert_eq!(pending.len(), 2);
        assert_eq!(pending[0].event, 0x0121);
        assert_eq!(pending[1].event, 0x0122);
        assert_eq!(pending[1].param, 200);
    }

    // -- System params ------------------------------------------

    #[test]
    fn get_param_int_returns_stored() {
        let s = demo_sys();
        assert_eq!(
            cell_sysutil_get_system_param_int(&s, SysParamId::Lang as i32),
            Ok(1)
        );
    }

    #[test]
    fn get_param_int_rejects_string_id() {
        let s = demo_sys();
        assert_eq!(
            cell_sysutil_get_system_param_int(&s, SysParamId::Nickname as i32),
            Err(CELL_SYSUTIL_ERROR_TYPE)
        );
    }

    #[test]
    fn get_param_int_unknown_id_is_value_error() {
        let s = demo_sys();
        assert_eq!(
            cell_sysutil_get_system_param_int(&s, 0x9999),
            Err(CELL_SYSUTIL_ERROR_VALUE)
        );
    }

    #[test]
    fn get_param_int_missing_value_is_value_error() {
        let s = TestSys::default();
        assert_eq!(
            cell_sysutil_get_system_param_int(&s, SysParamId::Lang as i32),
            Err(CELL_SYSUTIL_ERROR_VALUE)
        );
    }

    #[test]
    fn get_param_string_returns_nickname() {
        let s = demo_sys();
        assert_eq!(
            cell_sysutil_get_system_param_string(&s, SysParamId::Nickname as i32, 128),
            Ok("TestUser")
        );
    }

    #[test]
    fn get_param_string_rejects_int_id() {
        let s = demo_sys();
        assert_eq!(
            cell_sysutil_get_system_param_string(&s, SysParamId::Lang as i32, 128),
            Err(CELL_SYSUTIL_ERROR_TYPE)
        );
    }

    #[test]
    fn get_param_string_zero_bufsize_is_size_error() {
        let s = demo_sys();
        assert_eq!(
            cell_sysutil_get_system_param_string(&s, SysParamId::Nickname as i32, 0),
            Err(CELL_SYSUTIL_ERROR_SIZE)
        );
    }

    #[test]
    fn get_system_media_ver_returns_firmware() {
        let s = demo_sys();
        assert_eq!(cell_sysutil_get_system_media_ver(&s), "04.5500");
    }

    // -- Constants + enum classification ------------------------

    #[test]
    fn cb_slot_max_is_8() {
        assert_eq!(CB_SLOT_MAX, 8);
    }

    #[test]
    fn callback_event_ordinals_frozen() {
        assert_eq!(CallbackEvent::RequestExitGame as u32, 0x0101);
        assert_eq!(CallbackEvent::DrawingBegin as u32, 0x0121);
        assert_eq!(CallbackEvent::SystemMenuOpen as u32, 0x0131);
        assert_eq!(CallbackEvent::BgmplaybackPlay as u32, 0x0141);
        assert_eq!(CallbackEvent::NpInvitationSelected as u32, 0x0151);
    }

    #[test]
    fn sys_param_id_ordinals_frozen() {
        assert_eq!(SysParamId::Lang as i32, 0x0111);
        assert_eq!(SysParamId::Nickname as i32, 0x0113);
        assert_eq!(SysParamId::DateFormat as i32, 0x0114);
        assert_eq!(SysParamId::GameParentalLevel as i32, 0x0121);
        assert_eq!(SysParamId::CurrentUsername as i32, 0x0126);
    }

    #[test]
    fn sys_param_id_classification() {
        assert!(!SysParamId::Lang.is_string());
        assert!(!SysParamId::DateFormat.is_string());
        assert!(SysParamId::Nickname.is_string());
        assert!(SysParamId::CurrentUsername.is_string());
    }

    #[test]
    fn error_codes_in_sysutil_facility() {
        for e in [
            CELL_SYSUTIL_ERROR_TYPE,
            CELL_SYSUTIL_ERROR_VALUE,
            CELL_SYSUTIL_ERROR_SIZE,
            CELL_SYSUTIL_ERROR_NUM,
        ] {
            assert_eq!(e.0 & 0xFFFF_FF00, 0x8002_B100);
        }
    }
}
