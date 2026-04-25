//! `rpcs3-hle-cellbgdl` — background download HLE.
//!
//! Ports `rpcs3/Emu/Cell/Modules/cellBgdl.cpp`. PS3's background
//! download utility drives PSN store patches / DLC prefetch while the
//! XMB is idle or during gameplay. The surface is tiny:
//!
//! 1. `SetMode(mode)` — AUTO or ALWAYS_ALLOW.
//! 2. `GetInfo(task_id)` — poll progress + state.
//! 3. `GetInfo2(task_id)` — extended variant.
//!
//! The real lib also exposes internal register/start/stop/pause hooks
//! that the shell uses; games only ever query status.
//!
//! ## Entry points covered
//!
//! | HLE function                 | Rust wrapper                         |
//! |------------------------------|--------------------------------------|
//! | `cellBGDLSetMode`            | [`BgdlManager::set_mode`]            |
//! | `cellBGDLGetInfo`            | [`BgdlManager::info`]                |
//! | `cellBGDLGetInfo2`           | [`BgdlManager::info2`]               |

use rpcs3_emu_types::CellError;

// =====================================================================
// Error codes — byte-exact with cellBgdl.h:6-13
// =====================================================================

pub mod errors {
    use rpcs3_emu_types::CellError;

    pub const BUSY: CellError = CellError(0x8002_ce01);
    pub const INTERNAL: CellError = CellError(0x8002_ce02);
    pub const PARAM: CellError = CellError(0x8002_ce03);
    pub const ACCESS_ERROR: CellError = CellError(0x8002_ce04);
    pub const INITIALIZE: CellError = CellError(0x8002_ce05);
}

// =====================================================================
// State / mode enums (cellBgdl.h:15-28)
// =====================================================================

pub const STATE_ERROR: i32 = 0;
pub const STATE_PAUSE: i32 = 1;
pub const STATE_READY: i32 = 2;
pub const STATE_RUN: i32 = 3;
pub const STATE_COMPLETE: i32 = 4;

#[must_use]
pub fn is_known_state(state: i32) -> bool {
    (STATE_ERROR..=STATE_COMPLETE).contains(&state)
}

pub const MODE_AUTO: i32 = 0;
pub const MODE_ALWAYS_ALLOW: i32 = 1;

#[must_use]
pub fn is_known_mode(mode: i32) -> bool {
    matches!(mode, MODE_AUTO | MODE_ALWAYS_ALLOW)
}

// =====================================================================
// Types
// =====================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct BgdlInfo {
    pub received_size: u64,
    pub content_size: u64,
    pub state: i32,
}

#[derive(Clone, Debug)]
pub struct BgdlTask {
    pub task_id: u32,
    pub received_size: u64,
    pub content_size: u64,
    pub state: i32,
    pub title: String,
}

// =====================================================================
// Manager — backs the XMB-level download queue
// =====================================================================

pub const MAX_TASKS: usize = 64;

#[derive(Clone, Debug)]
pub struct BgdlManager {
    mode: i32,
    tasks: Vec<BgdlTask>,
}

impl BgdlManager {
    #[must_use]
    pub fn new() -> Self {
        Self { mode: MODE_AUTO, tasks: Vec::new() }
    }

    #[must_use]
    pub fn mode(&self) -> i32 {
        self.mode
    }

    #[must_use]
    pub fn task_count(&self) -> usize {
        self.tasks.len()
    }

    /// `cellBGDLSetMode(mode)`. Only AUTO (shell-arbitrated) and
    /// ALWAYS_ALLOW (game always allowed to run BGDL) are valid.
    pub fn set_mode(&mut self, mode: i32) -> Result<(), CellError> {
        if !is_known_mode(mode) {
            return Err(errors::PARAM);
        }
        self.mode = mode;
        Ok(())
    }

    // ----------------- Shell-side hooks -----------------

    /// Admin-side: register a new BGDL task — games cannot do this via
    /// the HLE API; the shell adds tasks when the user starts a store
    /// download.
    pub fn register_task(&mut self, task: BgdlTask) -> Result<(), CellError> {
        if self.tasks.len() >= MAX_TASKS {
            return Err(errors::INTERNAL);
        }
        if task.task_id == 0 {
            return Err(errors::PARAM);
        }
        if task.content_size == 0 {
            return Err(errors::PARAM);
        }
        if task.received_size > task.content_size {
            return Err(errors::PARAM);
        }
        if !is_known_state(task.state) {
            return Err(errors::PARAM);
        }
        if self.tasks.iter().any(|t| t.task_id == task.task_id) {
            return Err(errors::PARAM);
        }
        self.tasks.push(task);
        Ok(())
    }

    pub fn unregister_task(&mut self, task_id: u32) -> Result<(), CellError> {
        let idx = self.tasks.iter().position(|t| t.task_id == task_id).ok_or(errors::PARAM)?;
        self.tasks.remove(idx);
        Ok(())
    }

    pub fn update_progress(
        &mut self,
        task_id: u32,
        received_size: u64,
        state: i32,
    ) -> Result<(), CellError> {
        if !is_known_state(state) {
            return Err(errors::PARAM);
        }
        let t = self.tasks.iter_mut().find(|t| t.task_id == task_id).ok_or(errors::PARAM)?;
        if received_size > t.content_size {
            return Err(errors::PARAM);
        }
        t.received_size = received_size;
        t.state = state;
        Ok(())
    }

    // ----------------- Queries -----------------

    /// `cellBGDLGetInfo(task_id, info)`. Returns the current receive
    /// counters + state snapshot.
    pub fn info(&self, task_id: u32) -> Result<BgdlInfo, CellError> {
        let t = self.tasks.iter().find(|t| t.task_id == task_id).ok_or(errors::PARAM)?;
        Ok(BgdlInfo {
            received_size: t.received_size,
            content_size: t.content_size,
            state: t.state,
        })
    }

    /// `cellBGDLGetInfo2(task_id)` — same payload in the Rust model;
    /// the C++ lib adds a few reserved bytes the shell doesn't expose.
    pub fn info2(&self, task_id: u32) -> Result<BgdlInfo, CellError> {
        self.info(task_id)
    }

    /// Not part of the game-facing API but handy for tests / shell:
    /// return the full list.
    #[must_use]
    pub fn tasks(&self) -> &[BgdlTask] {
        &self.tasks
    }
}

impl Default for BgdlManager {
    fn default() -> Self {
        Self::new()
    }
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_task(id: u32, state: i32) -> BgdlTask {
        BgdlTask {
            task_id: id,
            received_size: 0,
            content_size: 1024,
            state,
            title: format!("Task {id}"),
        }
    }

    #[test]
    fn error_codes_byte_exact() {
        assert_eq!(errors::BUSY.0, 0x8002_ce01);
        assert_eq!(errors::INTERNAL.0, 0x8002_ce02);
        assert_eq!(errors::PARAM.0, 0x8002_ce03);
        assert_eq!(errors::ACCESS_ERROR.0, 0x8002_ce04);
        assert_eq!(errors::INITIALIZE.0, 0x8002_ce05);
    }

    #[test]
    fn state_constants_stable() {
        assert_eq!(STATE_ERROR, 0);
        assert_eq!(STATE_PAUSE, 1);
        assert_eq!(STATE_READY, 2);
        assert_eq!(STATE_RUN, 3);
        assert_eq!(STATE_COMPLETE, 4);
    }

    #[test]
    fn mode_constants_stable() {
        assert_eq!(MODE_AUTO, 0);
        assert_eq!(MODE_ALWAYS_ALLOW, 1);
    }

    #[test]
    fn is_known_state_helper() {
        for s in 0..=STATE_COMPLETE {
            assert!(is_known_state(s));
        }
        assert!(!is_known_state(-1));
        assert!(!is_known_state(99));
    }

    #[test]
    fn is_known_mode_helper() {
        assert!(is_known_mode(MODE_AUTO));
        assert!(is_known_mode(MODE_ALWAYS_ALLOW));
        assert!(!is_known_mode(-1));
        assert!(!is_known_mode(2));
    }

    #[test]
    fn fresh_manager_is_auto_mode() {
        let m = BgdlManager::new();
        assert_eq!(m.mode(), MODE_AUTO);
        assert_eq!(m.task_count(), 0);
    }

    #[test]
    fn set_mode_happy_path() {
        let mut m = BgdlManager::new();
        m.set_mode(MODE_ALWAYS_ALLOW).unwrap();
        assert_eq!(m.mode(), MODE_ALWAYS_ALLOW);
        m.set_mode(MODE_AUTO).unwrap();
        assert_eq!(m.mode(), MODE_AUTO);
    }

    #[test]
    fn set_mode_bad_value_rejected() {
        let mut m = BgdlManager::new();
        assert_eq!(m.set_mode(99), Err(errors::PARAM));
        assert_eq!(m.set_mode(-1), Err(errors::PARAM));
    }

    #[test]
    fn register_task_zero_id_rejected() {
        let mut m = BgdlManager::new();
        assert_eq!(m.register_task(sample_task(0, STATE_READY)), Err(errors::PARAM));
    }

    #[test]
    fn register_task_zero_content_size_rejected() {
        let mut m = BgdlManager::new();
        let mut t = sample_task(1, STATE_READY);
        t.content_size = 0;
        assert_eq!(m.register_task(t), Err(errors::PARAM));
    }

    #[test]
    fn register_task_received_larger_than_content_rejected() {
        let mut m = BgdlManager::new();
        let mut t = sample_task(1, STATE_READY);
        t.received_size = 2048;
        assert_eq!(m.register_task(t), Err(errors::PARAM));
    }

    #[test]
    fn register_task_unknown_state_rejected() {
        let mut m = BgdlManager::new();
        assert_eq!(m.register_task(sample_task(1, 99)), Err(errors::PARAM));
    }

    #[test]
    fn register_task_duplicate_id_rejected() {
        let mut m = BgdlManager::new();
        m.register_task(sample_task(1, STATE_READY)).unwrap();
        assert_eq!(m.register_task(sample_task(1, STATE_READY)), Err(errors::PARAM));
    }

    #[test]
    fn register_task_over_capacity_rejected() {
        let mut m = BgdlManager::new();
        for i in 1..=(MAX_TASKS as u32) {
            m.register_task(sample_task(i, STATE_READY)).unwrap();
        }
        assert_eq!(m.register_task(sample_task(999, STATE_READY)), Err(errors::INTERNAL));
    }

    #[test]
    fn unregister_task_unknown_id_rejected() {
        let mut m = BgdlManager::new();
        assert_eq!(m.unregister_task(99), Err(errors::PARAM));
    }

    #[test]
    fn update_progress_happy_path() {
        let mut m = BgdlManager::new();
        m.register_task(sample_task(1, STATE_READY)).unwrap();
        m.update_progress(1, 512, STATE_RUN).unwrap();
        let info = m.info(1).unwrap();
        assert_eq!(info.received_size, 512);
        assert_eq!(info.state, STATE_RUN);
    }

    #[test]
    fn update_progress_received_over_content_rejected() {
        let mut m = BgdlManager::new();
        m.register_task(sample_task(1, STATE_READY)).unwrap();
        assert_eq!(m.update_progress(1, 9999, STATE_RUN), Err(errors::PARAM));
    }

    #[test]
    fn update_progress_unknown_task_rejected() {
        let mut m = BgdlManager::new();
        assert_eq!(m.update_progress(99, 0, STATE_READY), Err(errors::PARAM));
    }

    #[test]
    fn update_progress_bad_state_rejected() {
        let mut m = BgdlManager::new();
        m.register_task(sample_task(1, STATE_READY)).unwrap();
        assert_eq!(m.update_progress(1, 0, 99), Err(errors::PARAM));
    }

    #[test]
    fn info_unknown_task_is_param() {
        let m = BgdlManager::new();
        assert_eq!(m.info(99), Err(errors::PARAM));
    }

    #[test]
    fn info2_mirrors_info() {
        let mut m = BgdlManager::new();
        let mut t = sample_task(1, STATE_RUN);
        t.received_size = 256;
        m.register_task(t).unwrap();
        assert_eq!(m.info(1), m.info2(1));
    }

    #[test]
    fn info_reports_full_progression() {
        let mut m = BgdlManager::new();
        m.register_task(sample_task(1, STATE_READY)).unwrap();
        m.update_progress(1, 256, STATE_RUN).unwrap();
        let info = m.info(1).unwrap();
        assert_eq!(info.received_size, 256);
        assert_eq!(info.content_size, 1024);
        assert_eq!(info.state, STATE_RUN);
    }

    #[test]
    fn unregister_then_info_fails() {
        let mut m = BgdlManager::new();
        m.register_task(sample_task(1, STATE_READY)).unwrap();
        m.unregister_task(1).unwrap();
        assert_eq!(m.info(1), Err(errors::PARAM));
    }

    #[test]
    fn tasks_observable_to_shell() {
        let mut m = BgdlManager::new();
        m.register_task(sample_task(1, STATE_READY)).unwrap();
        m.register_task(sample_task(2, STATE_PAUSE)).unwrap();
        assert_eq!(m.tasks().len(), 2);
    }

    #[test]
    fn full_lifecycle_smoke() {
        let mut m = BgdlManager::new();
        m.set_mode(MODE_ALWAYS_ALLOW).unwrap();
        m.register_task(sample_task(1, STATE_READY)).unwrap();
        // Simulate download progression.
        m.update_progress(1, 256, STATE_RUN).unwrap();
        m.update_progress(1, 512, STATE_RUN).unwrap();
        // Pause then resume.
        m.update_progress(1, 512, STATE_PAUSE).unwrap();
        m.update_progress(1, 768, STATE_RUN).unwrap();
        m.update_progress(1, 1024, STATE_COMPLETE).unwrap();
        let info = m.info2(1).unwrap();
        assert_eq!(info.received_size, info.content_size);
        assert_eq!(info.state, STATE_COMPLETE);
        m.unregister_task(1).unwrap();
    }
}
