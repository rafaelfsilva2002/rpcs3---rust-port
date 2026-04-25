//! `rpcs3-hle-celluserinfo` — user profile / account picker HLE.
//!
//! Ports `rpcs3/Emu/Cell/Modules/cellUserInfo.cpp`. Exposes the PS3
//! account table: games query the list of registered users, get the
//! current user, or open a picker dialog to let the player choose one.
//!
//! ## Entry points covered
//!
//! | HLE function                     | Rust wrapper                          |
//! |----------------------------------|---------------------------------------|
//! | `cellUserInfoGetList`            | [`UserInfoRegistry::list`]            |
//! | `cellUserInfoGetStat`            | [`UserInfoRegistry::stat`]            |
//! | `cellUserInfoGetCurrentUser`     | [`UserInfoRegistry::current_user`]    |
//! | `cellUserInfoSelectUser_ListType` | [`UserInfoRegistry::select_user_list_type`] |
//! | `cellUserInfoSelectUser_SetList`  | [`UserInfoRegistry::select_user_set_list`]  |
//! | `cellUserInfoEnableOverlay`      | [`UserInfoRegistry::enable_overlay`]  |

use rpcs3_emu_types::CellError;

// =====================================================================
// Error codes — byte-exact with cellUserInfo.h:6-12
// =====================================================================

pub mod errors {
    use rpcs3_emu_types::CellError;

    pub const BUSY: CellError = CellError(0x8002_c301);
    pub const INTERNAL: CellError = CellError(0x8002_c302);
    pub const PARAM: CellError = CellError(0x8002_c303);
    pub const NOUSER: CellError = CellError(0x8002_c304);
}

// =====================================================================
// Constants (cellUserInfo.h:15-43)
// =====================================================================

pub const USER_MAX: usize = 16;
pub const TITLE_SIZE: usize = 256;
pub const USERNAME_SIZE: usize = 64;

pub const LISTTYPE_ALL: u32 = 0;
pub const LISTTYPE_NOCURRENT: u32 = 1;

pub const RET_OK: u32 = 0;
pub const RET_CANCEL: u32 = 1;

pub const USERID_CURRENT: u32 = 0;
pub const USERID_MAX: u32 = 99_999_999;

pub const FOCUS_LISTHEAD: u32 = 0xFFFF_FFFF;

// =====================================================================
// Domain types
// =====================================================================

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserStat {
    pub id: u32,
    pub name: String, // ≤ USERNAME_SIZE
}

impl UserStat {
    #[must_use]
    pub fn new(id: u32, name: impl Into<String>) -> Self {
        Self { id, name: name.into() }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct UserList {
    pub ids: Vec<u32>, // ≤ USER_MAX
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListSet {
    pub title: String,
    pub focus: u32, // user id or FOCUS_LISTHEAD
    pub fixed_list: Option<Vec<u32>>, // if Some, restrict to these ids
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeSet {
    pub title: String,
    pub focus: u32,
    pub list_type: u32,
}

// =====================================================================
// Registry — singleton model for the PS3 user table
// =====================================================================

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SelectionState {
    Idle,
    DialogOpen,
}

#[derive(Clone, Debug)]
pub struct UserInfoRegistry {
    users: Vec<UserStat>,
    current_id: u32,
    selection: SelectionState,
    overlay_enabled: bool,
}

impl UserInfoRegistry {
    /// Constructs a registry with a single default user (id=1 "User1").
    #[must_use]
    pub fn new_single() -> Self {
        Self {
            users: vec![UserStat::new(1, "User1")],
            current_id: 1,
            selection: SelectionState::Idle,
            overlay_enabled: false,
        }
    }

    /// Empty registry — useful to test NOUSER paths.
    #[must_use]
    pub fn empty() -> Self {
        Self { users: Vec::new(), current_id: USERID_CURRENT, selection: SelectionState::Idle, overlay_enabled: false }
    }

    pub fn add_user(&mut self, stat: UserStat) -> Result<(), CellError> {
        if self.users.len() >= USER_MAX {
            return Err(errors::INTERNAL);
        }
        if stat.id == 0 || stat.id > USERID_MAX {
            return Err(errors::PARAM);
        }
        if stat.name.len() >= USERNAME_SIZE {
            return Err(errors::PARAM);
        }
        if self.users.iter().any(|u| u.id == stat.id) {
            return Err(errors::PARAM);
        }
        self.users.push(stat);
        Ok(())
    }

    pub fn set_current(&mut self, id: u32) -> Result<(), CellError> {
        if !self.users.iter().any(|u| u.id == id) {
            return Err(errors::NOUSER);
        }
        self.current_id = id;
        Ok(())
    }

    #[must_use]
    pub fn overlay_enabled(&self) -> bool {
        self.overlay_enabled
    }

    // ----------------- Queries -----------------

    /// `cellUserInfoGetList(listNum, listBuf, currentUser)`.
    /// Returns (count, list, current_id). Gracefully handles empty.
    pub fn list(&self) -> Result<(u32, UserList, u32), CellError> {
        let ids: Vec<u32> = self.users.iter().map(|u| u.id).collect();
        let count = u32::try_from(ids.len()).unwrap_or(u32::MAX);
        Ok((count, UserList { ids }, self.current_id))
    }

    /// `cellUserInfoGetStat(id, userStat)`. `USERID_CURRENT` resolves to
    /// the active user. NOUSER if not found.
    pub fn stat(&self, id: u32) -> Result<UserStat, CellError> {
        let effective = if id == USERID_CURRENT { self.current_id } else { id };
        if id > USERID_MAX && id != USERID_CURRENT {
            return Err(errors::PARAM);
        }
        self.users.iter().find(|u| u.id == effective).cloned().ok_or(errors::NOUSER)
    }

    pub fn current_user(&self) -> Result<u32, CellError> {
        if self.users.is_empty() {
            return Err(errors::NOUSER);
        }
        Ok(self.current_id)
    }

    // ----------------- Selection dialog -----------------

    /// `cellUserInfoSelectUser_ListType(setParam, finishCallback, userData,
    ///  container)`. Opens a picker filtered by `list_type`.
    pub fn select_user_list_type(&mut self, set: &TypeSet) -> Result<(), CellError> {
        if self.selection == SelectionState::DialogOpen {
            return Err(errors::BUSY);
        }
        if set.title.len() >= TITLE_SIZE {
            return Err(errors::PARAM);
        }
        if !matches!(set.list_type, LISTTYPE_ALL | LISTTYPE_NOCURRENT) {
            return Err(errors::PARAM);
        }
        if set.focus != FOCUS_LISTHEAD && !self.users.iter().any(|u| u.id == set.focus) {
            return Err(errors::PARAM);
        }
        if self.users.is_empty() {
            return Err(errors::NOUSER);
        }
        self.selection = SelectionState::DialogOpen;
        Ok(())
    }

    /// `cellUserInfoSelectUser_SetList(setParam, finishCallback, userData,
    ///  container)`. Opens a picker with an explicit id whitelist.
    pub fn select_user_set_list(&mut self, set: &ListSet) -> Result<(), CellError> {
        if self.selection == SelectionState::DialogOpen {
            return Err(errors::BUSY);
        }
        if set.title.len() >= TITLE_SIZE {
            return Err(errors::PARAM);
        }
        if let Some(list) = &set.fixed_list {
            if list.is_empty() || list.len() > USER_MAX {
                return Err(errors::PARAM);
            }
            for &id in list {
                if id == 0 || id > USERID_MAX {
                    return Err(errors::PARAM);
                }
            }
        }
        if set.focus != FOCUS_LISTHEAD && !self.users.iter().any(|u| u.id == set.focus) {
            return Err(errors::PARAM);
        }
        if self.users.is_empty() {
            return Err(errors::NOUSER);
        }
        self.selection = SelectionState::DialogOpen;
        Ok(())
    }

    /// Test hook: complete the dialog with a chosen id. Real lib posts
    /// the callback asynchronously.
    pub fn complete_selection(&mut self, id: u32) -> Result<(u32, UserStat), CellError> {
        if self.selection != SelectionState::DialogOpen {
            return Err(errors::INTERNAL);
        }
        let stat = self.users.iter().find(|u| u.id == id).cloned().ok_or(errors::NOUSER)?;
        self.selection = SelectionState::Idle;
        Ok((RET_OK, stat))
    }

    pub fn cancel_selection(&mut self) -> Result<u32, CellError> {
        if self.selection != SelectionState::DialogOpen {
            return Err(errors::INTERNAL);
        }
        self.selection = SelectionState::Idle;
        Ok(RET_CANCEL)
    }

    #[must_use]
    pub fn selection_state(&self) -> &SelectionState {
        &self.selection
    }

    pub fn enable_overlay(&mut self) -> Result<(), CellError> {
        if self.overlay_enabled {
            return Err(errors::BUSY);
        }
        self.overlay_enabled = true;
        Ok(())
    }
}

impl Default for UserInfoRegistry {
    fn default() -> Self {
        Self::new_single()
    }
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn multi_registry() -> UserInfoRegistry {
        let mut r = UserInfoRegistry::new_single();
        r.add_user(UserStat::new(2, "Alice")).unwrap();
        r.add_user(UserStat::new(3, "Bob")).unwrap();
        r.add_user(UserStat::new(4, "Carol")).unwrap();
        r
    }

    #[test]
    fn error_codes_byte_exact() {
        assert_eq!(errors::BUSY.0, 0x8002_c301);
        assert_eq!(errors::INTERNAL.0, 0x8002_c302);
        assert_eq!(errors::PARAM.0, 0x8002_c303);
        assert_eq!(errors::NOUSER.0, 0x8002_c304);
    }

    #[test]
    fn constants_stable() {
        assert_eq!(USER_MAX, 16);
        assert_eq!(TITLE_SIZE, 256);
        assert_eq!(USERNAME_SIZE, 64);
        assert_eq!(LISTTYPE_ALL, 0);
        assert_eq!(LISTTYPE_NOCURRENT, 1);
        assert_eq!(RET_OK, 0);
        assert_eq!(RET_CANCEL, 1);
        assert_eq!(USERID_CURRENT, 0);
        assert_eq!(USERID_MAX, 99_999_999);
        assert_eq!(FOCUS_LISTHEAD, 0xFFFF_FFFF);
    }

    #[test]
    fn fresh_registry_has_one_user() {
        let r = UserInfoRegistry::new_single();
        let (count, list, current) = r.list().unwrap();
        assert_eq!(count, 1);
        assert_eq!(list.ids, vec![1]);
        assert_eq!(current, 1);
    }

    #[test]
    fn empty_registry_list_returns_zero() {
        let r = UserInfoRegistry::empty();
        let (count, list, _current) = r.list().unwrap();
        assert_eq!(count, 0);
        assert_eq!(list.ids.len(), 0);
    }

    #[test]
    fn add_user_happy_path() {
        let mut r = UserInfoRegistry::new_single();
        r.add_user(UserStat::new(2, "Alice")).unwrap();
        let (count, _, _) = r.list().unwrap();
        assert_eq!(count, 2);
    }

    #[test]
    fn add_user_zero_id_rejected() {
        let mut r = UserInfoRegistry::new_single();
        assert_eq!(r.add_user(UserStat::new(0, "Zero")), Err(errors::PARAM));
    }

    #[test]
    fn add_user_over_max_id_rejected() {
        let mut r = UserInfoRegistry::new_single();
        assert_eq!(r.add_user(UserStat::new(USERID_MAX + 1, "X")), Err(errors::PARAM));
    }

    #[test]
    fn add_user_name_too_long_rejected() {
        let mut r = UserInfoRegistry::new_single();
        let long_name = "a".repeat(USERNAME_SIZE);
        assert_eq!(r.add_user(UserStat::new(5, long_name)), Err(errors::PARAM));
    }

    #[test]
    fn add_user_duplicate_id_rejected() {
        let mut r = UserInfoRegistry::new_single();
        assert_eq!(r.add_user(UserStat::new(1, "DupUser")), Err(errors::PARAM));
    }

    #[test]
    fn add_user_over_capacity_is_internal() {
        let mut r = UserInfoRegistry::new_single();
        for i in 2..=USER_MAX as u32 {
            r.add_user(UserStat::new(i, format!("U{i}"))).unwrap();
        }
        assert_eq!(r.add_user(UserStat::new(999, "Overflow")), Err(errors::INTERNAL));
    }

    #[test]
    fn stat_by_id_returns_user() {
        let r = multi_registry();
        assert_eq!(r.stat(3).unwrap().name, "Bob");
    }

    #[test]
    fn stat_by_current_resolves() {
        let r = multi_registry();
        assert_eq!(r.stat(USERID_CURRENT).unwrap().id, 1);
    }

    #[test]
    fn stat_unknown_id_is_nouser() {
        let r = multi_registry();
        assert_eq!(r.stat(99).err(), Some(errors::NOUSER));
    }

    #[test]
    fn stat_id_over_max_is_param() {
        let r = multi_registry();
        assert_eq!(r.stat(USERID_MAX + 1).err(), Some(errors::PARAM));
    }

    #[test]
    fn current_user_returns_active() {
        let mut r = multi_registry();
        r.set_current(3).unwrap();
        assert_eq!(r.current_user(), Ok(3));
    }

    #[test]
    fn current_user_empty_is_nouser() {
        let r = UserInfoRegistry::empty();
        assert_eq!(r.current_user(), Err(errors::NOUSER));
    }

    #[test]
    fn set_current_unknown_is_nouser() {
        let mut r = multi_registry();
        assert_eq!(r.set_current(99), Err(errors::NOUSER));
    }

    #[test]
    fn select_user_list_type_happy_path() {
        let mut r = multi_registry();
        let set = TypeSet { title: "Pick user".into(), focus: FOCUS_LISTHEAD, list_type: LISTTYPE_ALL };
        r.select_user_list_type(&set).unwrap();
        assert_eq!(r.selection_state(), &SelectionState::DialogOpen);
    }

    #[test]
    fn select_user_list_type_twice_is_busy() {
        let mut r = multi_registry();
        let set = TypeSet { title: "Pick".into(), focus: FOCUS_LISTHEAD, list_type: LISTTYPE_ALL };
        r.select_user_list_type(&set).unwrap();
        assert_eq!(r.select_user_list_type(&set), Err(errors::BUSY));
    }

    #[test]
    fn select_user_list_type_title_too_long_is_param() {
        let mut r = multi_registry();
        let set = TypeSet { title: "a".repeat(TITLE_SIZE), focus: FOCUS_LISTHEAD, list_type: LISTTYPE_ALL };
        assert_eq!(r.select_user_list_type(&set), Err(errors::PARAM));
    }

    #[test]
    fn select_user_list_type_bad_list_type_is_param() {
        let mut r = multi_registry();
        let set = TypeSet { title: "x".into(), focus: FOCUS_LISTHEAD, list_type: 99 };
        assert_eq!(r.select_user_list_type(&set), Err(errors::PARAM));
    }

    #[test]
    fn select_user_list_type_unknown_focus_is_param() {
        let mut r = multi_registry();
        let set = TypeSet { title: "x".into(), focus: 999, list_type: LISTTYPE_ALL };
        assert_eq!(r.select_user_list_type(&set), Err(errors::PARAM));
    }

    #[test]
    fn select_user_list_type_empty_registry_is_nouser() {
        let mut r = UserInfoRegistry::empty();
        let set = TypeSet { title: "x".into(), focus: FOCUS_LISTHEAD, list_type: LISTTYPE_ALL };
        assert_eq!(r.select_user_list_type(&set), Err(errors::NOUSER));
    }

    #[test]
    fn select_user_set_list_happy_path() {
        let mut r = multi_registry();
        let set = ListSet { title: "Pick".into(), focus: 2, fixed_list: Some(vec![2, 3]) };
        r.select_user_set_list(&set).unwrap();
    }

    #[test]
    fn select_user_set_list_empty_fixed_list_rejected() {
        let mut r = multi_registry();
        let set = ListSet { title: "x".into(), focus: FOCUS_LISTHEAD, fixed_list: Some(Vec::new()) };
        assert_eq!(r.select_user_set_list(&set), Err(errors::PARAM));
    }

    #[test]
    fn select_user_set_list_oversize_fixed_list_rejected() {
        let mut r = multi_registry();
        let set = ListSet {
            title: "x".into(),
            focus: FOCUS_LISTHEAD,
            fixed_list: Some((1u32..=(USER_MAX as u32 + 1)).collect()),
        };
        assert_eq!(r.select_user_set_list(&set), Err(errors::PARAM));
    }

    #[test]
    fn select_user_set_list_bad_id_in_list_rejected() {
        let mut r = multi_registry();
        let set = ListSet { title: "x".into(), focus: FOCUS_LISTHEAD, fixed_list: Some(vec![0, 1]) };
        assert_eq!(r.select_user_set_list(&set), Err(errors::PARAM));
    }

    #[test]
    fn complete_selection_happy_path() {
        let mut r = multi_registry();
        let set = TypeSet { title: "x".into(), focus: FOCUS_LISTHEAD, list_type: LISTTYPE_ALL };
        r.select_user_list_type(&set).unwrap();
        let (result, stat) = r.complete_selection(2).unwrap();
        assert_eq!(result, RET_OK);
        assert_eq!(stat.id, 2);
        assert_eq!(r.selection_state(), &SelectionState::Idle);
    }

    #[test]
    fn complete_selection_unknown_user_is_nouser() {
        let mut r = multi_registry();
        let set = TypeSet { title: "x".into(), focus: FOCUS_LISTHEAD, list_type: LISTTYPE_ALL };
        r.select_user_list_type(&set).unwrap();
        assert_eq!(r.complete_selection(99), Err(errors::NOUSER));
    }

    #[test]
    fn complete_selection_without_dialog_is_internal() {
        let mut r = multi_registry();
        assert_eq!(r.complete_selection(1), Err(errors::INTERNAL));
    }

    #[test]
    fn cancel_selection_returns_cancel_code() {
        let mut r = multi_registry();
        let set = TypeSet { title: "x".into(), focus: FOCUS_LISTHEAD, list_type: LISTTYPE_ALL };
        r.select_user_list_type(&set).unwrap();
        assert_eq!(r.cancel_selection(), Ok(RET_CANCEL));
        assert_eq!(r.selection_state(), &SelectionState::Idle);
    }

    #[test]
    fn cancel_selection_without_dialog_is_internal() {
        let mut r = multi_registry();
        assert_eq!(r.cancel_selection(), Err(errors::INTERNAL));
    }

    #[test]
    fn enable_overlay_toggles_flag() {
        let mut r = UserInfoRegistry::new_single();
        assert!(!r.overlay_enabled());
        r.enable_overlay().unwrap();
        assert!(r.overlay_enabled());
        assert_eq!(r.enable_overlay(), Err(errors::BUSY));
    }

    #[test]
    fn list_includes_all_ids_in_order() {
        let r = multi_registry();
        let (_, list, _) = r.list().unwrap();
        assert_eq!(list.ids, vec![1, 2, 3, 4]);
    }

    #[test]
    fn full_selection_flow_smoke() {
        let mut r = multi_registry();
        let set = ListSet { title: "Choose profile".into(), focus: 2, fixed_list: Some(vec![2, 3]) };
        r.select_user_set_list(&set).unwrap();
        let (result, stat) = r.complete_selection(3).unwrap();
        assert_eq!(result, RET_OK);
        assert_eq!(stat.name, "Bob");
        // After selection, dialog closed — can open another.
        let set2 = TypeSet { title: "Again".into(), focus: FOCUS_LISTHEAD, list_type: LISTTYPE_NOCURRENT };
        r.select_user_list_type(&set2).unwrap();
        assert_eq!(r.cancel_selection(), Ok(RET_CANCEL));
    }
}
