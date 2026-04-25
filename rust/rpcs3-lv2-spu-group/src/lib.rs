//! `rpcs3-lv2-spu-group` — `sys_spu_thread_group_*` LV2 syscalls.
//!
//! Ports the thread-group half of `rpcs3/Emu/Cell/lv2/sys_spu.cpp`.
//! A SPU thread group packages up to 8 SPU threads that are scheduled
//! as a unit: `start` brings them all to the running state, `join`
//! waits for a well-defined exit cause, `terminate` stops everyone.
//!
//! ## Syscalls covered
//!
//! | LV2 syscall                               | Rust wrapper                         |
//! |-------------------------------------------|--------------------------------------|
//! | `sys_spu_thread_group_create`             | [`sys_spu_thread_group_create`]      |
//! | `sys_spu_thread_group_destroy`            | [`sys_spu_thread_group_destroy`]     |
//! | `sys_spu_thread_group_start`              | [`sys_spu_thread_group_start`]       |
//! | `sys_spu_thread_group_suspend`            | [`sys_spu_thread_group_suspend`]     |
//! | `sys_spu_thread_group_resume`             | [`sys_spu_thread_group_resume`]      |
//! | `sys_spu_thread_group_terminate`          | [`sys_spu_thread_group_terminate`]   |
//! | `sys_spu_thread_group_join`               | [`sys_spu_thread_group_join`]        |
//! | `sys_spu_thread_group_get_priority`       | [`sys_spu_thread_group_get_priority`]|
//! | `sys_spu_thread_group_set_priority`       | [`sys_spu_thread_group_set_priority`]|
//!
//! ## Frozen constants (from `sys_spu.h:17-30`)

use rpcs3_emu_types::CellError;

// =====================================================================
// Group types
// =====================================================================

pub const GROUP_TYPE_NORMAL: i32 = 0x00;
pub const GROUP_TYPE_SYSTEM: i32 = 0x02;
pub const GROUP_TYPE_MEMORY_FROM_CONTAINER: i32 = 0x04;
pub const GROUP_TYPE_NON_CONTEXT: i32 = 0x08;
pub const GROUP_TYPE_EXCLUSIVE_NON_CONTEXT: i32 = 0x18;
pub const GROUP_TYPE_COOPERATE_WITH_SYSTEM: i32 = 0x20;

// =====================================================================
// Join causes (bitmask)
// =====================================================================

pub const JOIN_GROUP_EXIT: u32 = 0x0001;
pub const JOIN_ALL_THREADS_EXIT: u32 = 0x0002;
pub const JOIN_TERMINATED: u32 = 0x0004;

// =====================================================================
// Limits
// =====================================================================

pub const MAX_THREADS_PER_GROUP: u32 = 8;
pub const MIN_PRIORITY: i32 = 16;
pub const MAX_PRIORITY: i32 = 255;

// =====================================================================
// State FSM
// =====================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupState {
    /// Freshly created; threads may be initialised but not running.
    Initialized,
    /// `start` was called; all threads are executing.
    Running,
    /// `suspend` was called; all threads paused.
    Suspended,
    /// `terminate` ran, or all threads naturally exited. `join` can
    /// now drain the exit cause.
    Stopped,
    /// Like `Stopped` but the group has been destroyed via `destroy`.
    Destroyed,
}

// =====================================================================
// Attributes
// =====================================================================

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupAttr {
    pub name: String,
    pub num_threads: u32,
    pub priority: i32,
    pub group_type: i32,
}

fn validate_attr(attr: &GroupAttr) -> Result<(), CellError> {
    if attr.num_threads == 0 || attr.num_threads > MAX_THREADS_PER_GROUP {
        return Err(CellError::EINVAL);
    }
    if attr.priority < MIN_PRIORITY || attr.priority > MAX_PRIORITY {
        return Err(CellError::EINVAL);
    }
    Ok(())
}

// =====================================================================
// Registry trait
// =====================================================================

pub trait SpuGroupRegistry {
    fn group_create(&mut self, attr: GroupAttr) -> Result<u32, CellError>;
    fn group_destroy(&mut self, id: u32) -> Result<(), CellError>;
    fn group_start(&mut self, id: u32) -> Result<(), CellError>;
    fn group_suspend(&mut self, id: u32) -> Result<(), CellError>;
    fn group_resume(&mut self, id: u32) -> Result<(), CellError>;
    fn group_terminate(&mut self, id: u32, exit_status: i32) -> Result<(), CellError>;
    /// Returns `(cause_bitmask, exit_status)`.
    fn group_join(&mut self, id: u32) -> Result<(u32, i32), CellError>;
    fn group_get_priority(&self, id: u32) -> Result<i32, CellError>;
    fn group_set_priority(&mut self, id: u32, priority: i32) -> Result<(), CellError>;
    /// Mark one thread as exited. Used by the SPU scheduler to
    /// transition the group to `Stopped` when the last thread exits.
    fn thread_exited(&mut self, id: u32, thread_index: u32) -> Result<(), CellError>;
}

// =====================================================================
// Syscalls
// =====================================================================

#[must_use]
pub fn sys_spu_thread_group_create<R: SpuGroupRegistry + ?Sized>(
    reg: &mut R,
    attr: GroupAttr,
) -> Result<u32, CellError> {
    validate_attr(&attr)?;
    reg.group_create(attr)
}

#[must_use]
pub fn sys_spu_thread_group_destroy<R: SpuGroupRegistry + ?Sized>(
    reg: &mut R,
    id: u32,
) -> Result<(), CellError> {
    reg.group_destroy(id)
}

#[must_use]
pub fn sys_spu_thread_group_start<R: SpuGroupRegistry + ?Sized>(
    reg: &mut R,
    id: u32,
) -> Result<(), CellError> {
    reg.group_start(id)
}

#[must_use]
pub fn sys_spu_thread_group_suspend<R: SpuGroupRegistry + ?Sized>(
    reg: &mut R,
    id: u32,
) -> Result<(), CellError> {
    reg.group_suspend(id)
}

#[must_use]
pub fn sys_spu_thread_group_resume<R: SpuGroupRegistry + ?Sized>(
    reg: &mut R,
    id: u32,
) -> Result<(), CellError> {
    reg.group_resume(id)
}

#[must_use]
pub fn sys_spu_thread_group_terminate<R: SpuGroupRegistry + ?Sized>(
    reg: &mut R,
    id: u32,
    exit_status: i32,
) -> Result<(), CellError> {
    reg.group_terminate(id, exit_status)
}

#[must_use]
pub fn sys_spu_thread_group_join<R: SpuGroupRegistry + ?Sized>(
    reg: &mut R,
    id: u32,
) -> Result<(u32, i32), CellError> {
    reg.group_join(id)
}

#[must_use]
pub fn sys_spu_thread_group_get_priority<R: SpuGroupRegistry + ?Sized>(
    reg: &R,
    id: u32,
) -> Result<i32, CellError> {
    reg.group_get_priority(id)
}

#[must_use]
pub fn sys_spu_thread_group_set_priority<R: SpuGroupRegistry + ?Sized>(
    reg: &mut R,
    id: u32,
    priority: i32,
) -> Result<(), CellError> {
    if priority < MIN_PRIORITY || priority > MAX_PRIORITY {
        return Err(CellError::EINVAL);
    }
    reg.group_set_priority(id, priority)
}

// =====================================================================
// Reference implementation
// =====================================================================

#[derive(Debug, Default)]
pub struct TestSpuGroupRegistry {
    next_id: u32,
    groups: std::collections::BTreeMap<u32, Group>,
}

#[derive(Debug)]
struct Group {
    attr: GroupAttr,
    state: GroupState,
    threads_alive: u32,
    join_cause: u32,
    exit_status: i32,
}

impl TestSpuGroupRegistry {
    fn alloc_id(&mut self) -> u32 {
        self.next_id += 1;
        // Match C++ `lv2_spu_group::id_base = 0x04000100` + step 0x100.
        0x0400_0100 + (self.next_id - 1) * 0x100
    }

    #[must_use]
    pub fn state(&self, id: u32) -> Option<GroupState> {
        self.groups.get(&id).map(|g| g.state)
    }
}

impl SpuGroupRegistry for TestSpuGroupRegistry {
    fn group_create(&mut self, attr: GroupAttr) -> Result<u32, CellError> {
        let id = self.alloc_id();
        self.groups.insert(
            id,
            Group {
                threads_alive: attr.num_threads,
                attr,
                state: GroupState::Initialized,
                join_cause: 0,
                exit_status: 0,
            },
        );
        Ok(id)
    }

    fn group_destroy(&mut self, id: u32) -> Result<(), CellError> {
        let g = self.groups.get(&id).ok_or(CellError::ESRCH)?;
        if g.state == GroupState::Running || g.state == GroupState::Suspended {
            return Err(CellError::EBUSY);
        }
        self.groups.remove(&id);
        Ok(())
    }

    fn group_start(&mut self, id: u32) -> Result<(), CellError> {
        let g = self.groups.get_mut(&id).ok_or(CellError::ESRCH)?;
        match g.state {
            GroupState::Initialized => {
                g.state = GroupState::Running;
                Ok(())
            }
            GroupState::Running | GroupState::Suspended => Err(CellError::ESTAT),
            GroupState::Stopped | GroupState::Destroyed => Err(CellError::ESTAT),
        }
    }

    fn group_suspend(&mut self, id: u32) -> Result<(), CellError> {
        let g = self.groups.get_mut(&id).ok_or(CellError::ESRCH)?;
        if g.state != GroupState::Running {
            return Err(CellError::ESTAT);
        }
        g.state = GroupState::Suspended;
        Ok(())
    }

    fn group_resume(&mut self, id: u32) -> Result<(), CellError> {
        let g = self.groups.get_mut(&id).ok_or(CellError::ESRCH)?;
        if g.state != GroupState::Suspended {
            return Err(CellError::ESTAT);
        }
        g.state = GroupState::Running;
        Ok(())
    }

    fn group_terminate(&mut self, id: u32, exit_status: i32) -> Result<(), CellError> {
        let g = self.groups.get_mut(&id).ok_or(CellError::ESRCH)?;
        match g.state {
            GroupState::Running | GroupState::Suspended => {
                g.state = GroupState::Stopped;
                g.exit_status = exit_status;
                g.join_cause = JOIN_TERMINATED;
                g.threads_alive = 0;
                Ok(())
            }
            _ => Err(CellError::ESTAT),
        }
    }

    fn group_join(&mut self, id: u32) -> Result<(u32, i32), CellError> {
        let g = self.groups.get(&id).ok_or(CellError::ESRCH)?;
        match g.state {
            GroupState::Stopped => Ok((g.join_cause, g.exit_status)),
            _ => Err(CellError::ESTAT),
        }
    }

    fn group_get_priority(&self, id: u32) -> Result<i32, CellError> {
        Ok(self.groups.get(&id).ok_or(CellError::ESRCH)?.attr.priority)
    }

    fn group_set_priority(&mut self, id: u32, priority: i32) -> Result<(), CellError> {
        let g = self.groups.get_mut(&id).ok_or(CellError::ESRCH)?;
        g.attr.priority = priority;
        Ok(())
    }

    fn thread_exited(&mut self, id: u32, _thread_index: u32) -> Result<(), CellError> {
        let g = self.groups.get_mut(&id).ok_or(CellError::ESRCH)?;
        if g.threads_alive == 0 {
            return Err(CellError::ESTAT);
        }
        g.threads_alive -= 1;
        if g.threads_alive == 0 {
            g.state = GroupState::Stopped;
            // Natural exit: all-threads-exit bit set, not terminated.
            g.join_cause |= JOIN_ALL_THREADS_EXIT;
        }
        Ok(())
    }
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn attr() -> GroupAttr {
        GroupAttr {
            name: "test".to_owned(),
            num_threads: 2,
            priority: 100,
            group_type: GROUP_TYPE_NORMAL,
        }
    }

    fn setup() -> (TestSpuGroupRegistry, u32) {
        let mut reg = TestSpuGroupRegistry::default();
        let id = sys_spu_thread_group_create(&mut reg, attr()).unwrap();
        (reg, id)
    }

    // --- constants ------------------------------------------------

    #[test]
    fn group_type_constants_match_cpp() {
        assert_eq!(GROUP_TYPE_NORMAL, 0x00);
        assert_eq!(GROUP_TYPE_SYSTEM, 0x02);
        assert_eq!(GROUP_TYPE_MEMORY_FROM_CONTAINER, 0x04);
        assert_eq!(GROUP_TYPE_NON_CONTEXT, 0x08);
        assert_eq!(GROUP_TYPE_EXCLUSIVE_NON_CONTEXT, 0x18);
        assert_eq!(GROUP_TYPE_COOPERATE_WITH_SYSTEM, 0x20);
    }

    #[test]
    fn join_cause_bitmask_matches_cpp() {
        assert_eq!(JOIN_GROUP_EXIT, 0x0001);
        assert_eq!(JOIN_ALL_THREADS_EXIT, 0x0002);
        assert_eq!(JOIN_TERMINATED, 0x0004);
    }

    #[test]
    fn id_base_is_0x04000100_with_step_0x100() {
        let (_reg, id) = setup();
        assert_eq!(id, 0x0400_0100);
        let mut reg = TestSpuGroupRegistry::default();
        sys_spu_thread_group_create(&mut reg, attr()).unwrap();
        let id2 = sys_spu_thread_group_create(&mut reg, attr()).unwrap();
        assert_eq!(id2, 0x0400_0200);
    }

    // --- validation -----------------------------------------------

    #[test]
    fn create_rejects_zero_threads() {
        let mut reg = TestSpuGroupRegistry::default();
        let bad = GroupAttr { num_threads: 0, ..attr() };
        assert_eq!(
            sys_spu_thread_group_create(&mut reg, bad).unwrap_err(),
            CellError::EINVAL,
        );
    }

    #[test]
    fn create_rejects_more_than_8_threads() {
        let mut reg = TestSpuGroupRegistry::default();
        let bad = GroupAttr { num_threads: 9, ..attr() };
        assert_eq!(
            sys_spu_thread_group_create(&mut reg, bad).unwrap_err(),
            CellError::EINVAL,
        );
    }

    #[test]
    fn create_rejects_priority_out_of_range() {
        let mut reg = TestSpuGroupRegistry::default();
        let bad = GroupAttr { priority: 15, ..attr() };
        assert_eq!(
            sys_spu_thread_group_create(&mut reg, bad).unwrap_err(),
            CellError::EINVAL,
        );
        let bad = GroupAttr { priority: 256, ..attr() };
        assert_eq!(
            sys_spu_thread_group_create(&mut reg, bad).unwrap_err(),
            CellError::EINVAL,
        );
    }

    // --- FSM ------------------------------------------------------

    #[test]
    fn start_moves_to_running() {
        let (mut reg, id) = setup();
        sys_spu_thread_group_start(&mut reg, id).unwrap();
        assert_eq!(reg.state(id), Some(GroupState::Running));
    }

    #[test]
    fn start_twice_is_estat() {
        let (mut reg, id) = setup();
        sys_spu_thread_group_start(&mut reg, id).unwrap();
        assert_eq!(
            sys_spu_thread_group_start(&mut reg, id).unwrap_err(),
            CellError::ESTAT,
        );
    }

    #[test]
    fn suspend_resume_flip_state() {
        let (mut reg, id) = setup();
        sys_spu_thread_group_start(&mut reg, id).unwrap();
        sys_spu_thread_group_suspend(&mut reg, id).unwrap();
        assert_eq!(reg.state(id), Some(GroupState::Suspended));
        sys_spu_thread_group_resume(&mut reg, id).unwrap();
        assert_eq!(reg.state(id), Some(GroupState::Running));
    }

    #[test]
    fn suspend_when_not_running_is_estat() {
        let (mut reg, id) = setup();
        assert_eq!(
            sys_spu_thread_group_suspend(&mut reg, id).unwrap_err(),
            CellError::ESTAT,
        );
    }

    #[test]
    fn resume_when_not_suspended_is_estat() {
        let (mut reg, id) = setup();
        sys_spu_thread_group_start(&mut reg, id).unwrap();
        assert_eq!(
            sys_spu_thread_group_resume(&mut reg, id).unwrap_err(),
            CellError::ESTAT,
        );
    }

    // --- terminate / join -----------------------------------------

    #[test]
    fn terminate_stops_and_sets_cause_and_status() {
        let (mut reg, id) = setup();
        sys_spu_thread_group_start(&mut reg, id).unwrap();
        sys_spu_thread_group_terminate(&mut reg, id, -42).unwrap();
        assert_eq!(reg.state(id), Some(GroupState::Stopped));
        let (cause, status) = sys_spu_thread_group_join(&mut reg, id).unwrap();
        assert_eq!(cause, JOIN_TERMINATED);
        assert_eq!(status, -42);
    }

    #[test]
    fn natural_exit_sets_all_threads_exit() {
        let (mut reg, id) = setup();
        sys_spu_thread_group_start(&mut reg, id).unwrap();
        reg.thread_exited(id, 0).unwrap();
        assert_eq!(reg.state(id), Some(GroupState::Running), "1/2 still alive");
        reg.thread_exited(id, 1).unwrap();
        assert_eq!(reg.state(id), Some(GroupState::Stopped));

        let (cause, _status) = sys_spu_thread_group_join(&mut reg, id).unwrap();
        assert_eq!(cause & JOIN_ALL_THREADS_EXIT, JOIN_ALL_THREADS_EXIT);
    }

    #[test]
    fn join_before_stop_is_estat() {
        let (mut reg, id) = setup();
        sys_spu_thread_group_start(&mut reg, id).unwrap();
        assert_eq!(
            sys_spu_thread_group_join(&mut reg, id).unwrap_err(),
            CellError::ESTAT,
        );
    }

    // --- priority -------------------------------------------------

    #[test]
    fn get_set_priority_round_trips() {
        let (mut reg, id) = setup();
        assert_eq!(sys_spu_thread_group_get_priority(&reg, id).unwrap(), 100);
        sys_spu_thread_group_set_priority(&mut reg, id, 200).unwrap();
        assert_eq!(sys_spu_thread_group_get_priority(&reg, id).unwrap(), 200);
    }

    #[test]
    fn set_priority_out_of_range_is_einval() {
        let (mut reg, id) = setup();
        assert_eq!(
            sys_spu_thread_group_set_priority(&mut reg, id, 1).unwrap_err(),
            CellError::EINVAL,
        );
    }

    // --- destroy --------------------------------------------------

    #[test]
    fn destroy_while_running_is_ebusy() {
        let (mut reg, id) = setup();
        sys_spu_thread_group_start(&mut reg, id).unwrap();
        assert_eq!(
            sys_spu_thread_group_destroy(&mut reg, id).unwrap_err(),
            CellError::EBUSY,
        );
    }

    #[test]
    fn destroy_after_join_succeeds() {
        let (mut reg, id) = setup();
        sys_spu_thread_group_start(&mut reg, id).unwrap();
        sys_spu_thread_group_terminate(&mut reg, id, 0).unwrap();
        sys_spu_thread_group_join(&mut reg, id).unwrap();
        sys_spu_thread_group_destroy(&mut reg, id).unwrap();
        assert_eq!(reg.state(id), None);
    }

    #[test]
    fn destroy_initialized_group_succeeds() {
        let (mut reg, id) = setup();
        sys_spu_thread_group_destroy(&mut reg, id).unwrap();
        assert_eq!(
            sys_spu_thread_group_destroy(&mut reg, id).unwrap_err(),
            CellError::ESRCH,
        );
    }

    #[test]
    fn unknown_id_is_esrch_across_ops() {
        let mut reg = TestSpuGroupRegistry::default();
        let bogus = 0x0400_DEAD;
        assert_eq!(sys_spu_thread_group_start(&mut reg, bogus).unwrap_err(), CellError::ESRCH);
        assert_eq!(sys_spu_thread_group_destroy(&mut reg, bogus).unwrap_err(), CellError::ESRCH);
        assert_eq!(sys_spu_thread_group_join(&mut reg, bogus).unwrap_err(), CellError::ESRCH);
        assert_eq!(sys_spu_thread_group_get_priority(&reg, bogus).unwrap_err(), CellError::ESRCH);
    }

    #[test]
    fn terminate_before_start_is_estat() {
        let (mut reg, id) = setup();
        assert_eq!(
            sys_spu_thread_group_terminate(&mut reg, id, 0).unwrap_err(),
            CellError::ESTAT,
        );
    }
}
