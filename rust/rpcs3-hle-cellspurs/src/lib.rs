//! `rpcs3-hle-cellspurs` — SPU Runtime System (SPURS) HLE layer.
//!
//! Ports the entry-point subset of
//! `rpcs3/Emu/Cell/Modules/cellSpurs.cpp`. SPURS is Sony's
//! high-level SPU scheduling framework: a *SPURS instance* owns a
//! SPU thread group plus up to 16 (or 32) workloads. Games push
//! work units onto workloads; the SPURS kernel running on the SPUs
//! pulls them off and runs them.
//!
//! Iteration 1 covers:
//!
//! * Instance lifecycle: `cellSpursInitialize` / `*InitializeWithAttribute`
//!   / `cellSpursFinalize` / `cellSpursAttributeInitialize` and setter
//!   variants.
//! * Workload lifecycle: `cellSpursAddWorkload` / `*AddWorkload2`
//!   / `cellSpursRemoveWorkload` / `cellSpursGetWorkloadInfo`
//!   / `cellSpursShutdownWorkload` / `cellSpursWaitForWorkloadShutdown`.
//! * Attribute setters enforce byte-exact validation (name length,
//!   priority, SPU count, etc.).
//!
//! Task-level primitives (`cellSpursTaskset*` / `cellSpursJob*`) are
//! out of scope for this iteration — they build on top of workloads.
//!
//! ## Frozen constants (from `cellSpurs.h`)
//!
//! | Const                    | Value |
//! |--------------------------|------:|
//! | `MAX_SPU`                | 8     |
//! | `MAX_WORKLOAD`           | 16    |
//! | `MAX_WORKLOAD2`          | 32    |
//! | `MAX_PRIORITY`           | 16    |
//! | `MAX_TASK`               | 128   |
//! | `MAX_TASK_NAME_LENGTH`   | 32    |

use rpcs3_emu_types::CellError;

// =====================================================================
// Error codes — byte-exact with cellSpurs.h:17-65
// =====================================================================

pub mod errors {
    use rpcs3_emu_types::CellError;

    // Core errors (facility 0x80410700).
    pub const CORE_AGAIN: CellError = CellError(0x8041_0701);
    pub const CORE_INVAL: CellError = CellError(0x8041_0702);
    pub const CORE_NOMEM: CellError = CellError(0x8041_0704);
    pub const CORE_SRCH: CellError = CellError(0x8041_0705);
    pub const CORE_PERM: CellError = CellError(0x8041_0709);
    pub const CORE_BUSY: CellError = CellError(0x8041_070A);
    pub const CORE_STAT: CellError = CellError(0x8041_070F);
    pub const CORE_ALIGN: CellError = CellError(0x8041_0710);
    pub const CORE_NULL_POINTER: CellError = CellError(0x8041_0711);

    // Policy-module errors (facility 0x80410800).
    pub const POLICY_AGAIN: CellError = CellError(0x8041_0801);
    pub const POLICY_INVAL: CellError = CellError(0x8041_0802);
    pub const POLICY_NOMEM: CellError = CellError(0x8041_0804);
    pub const POLICY_SRCH: CellError = CellError(0x8041_0805);
    pub const POLICY_BUSY: CellError = CellError(0x8041_080A);
    pub const POLICY_STAT: CellError = CellError(0x8041_080F);
    pub const POLICY_NULL_POINTER: CellError = CellError(0x8041_0811);

    // Task errors (facility 0x80410900).
    pub const TASK_AGAIN: CellError = CellError(0x8041_0901);
    pub const TASK_INVAL: CellError = CellError(0x8041_0902);
    pub const TASK_NOMEM: CellError = CellError(0x8041_0904);
    pub const TASK_SRCH: CellError = CellError(0x8041_0905);
    pub const TASK_BUSY: CellError = CellError(0x8041_090A);
    pub const TASK_STAT: CellError = CellError(0x8041_090F);
    pub const TASK_FATAL: CellError = CellError(0x8041_0914);
    pub const TASK_SHUTDOWN: CellError = CellError(0x8041_0920);
}

// =====================================================================
// Layout constants
// =====================================================================

pub const MAX_SPU: u32 = 8;
pub const MAX_WORKLOAD: u32 = 16;
pub const MAX_WORKLOAD2: u32 = 32;
pub const MAX_PRIORITY: u32 = 16;
pub const MAX_TASK: u32 = 128;
pub const MAX_TASK_NAME_LENGTH: usize = 32;

/// Default SPU group priority range [1, MAX_PRIORITY].
pub const MIN_PRIORITY_VAL: u32 = 1;
pub const MAX_PRIORITY_VAL: u32 = MAX_PRIORITY;

// =====================================================================
// Attributes
// =====================================================================

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpursAttribute {
    pub name: String,
    pub num_spus: u32,
    pub spu_priority: u32,
    pub ppu_priority: u32,
    pub exit_if_no_work: bool,
    pub enable_spu_printf: bool,
}

impl Default for SpursAttribute {
    fn default() -> Self {
        Self {
            name: String::new(),
            num_spus: 1,
            spu_priority: 100,
            ppu_priority: 100,
            exit_if_no_work: false,
            enable_spu_printf: false,
        }
    }
}

impl SpursAttribute {
    fn validate(&self) -> Result<(), CellError> {
        if self.num_spus == 0 || self.num_spus > MAX_SPU {
            return Err(errors::CORE_INVAL);
        }
        if self.spu_priority < MIN_PRIORITY_VAL || self.spu_priority > MAX_PRIORITY_VAL {
            return Err(errors::CORE_INVAL);
        }
        if self.ppu_priority < 16 || self.ppu_priority > 255 {
            return Err(errors::CORE_INVAL);
        }
        Ok(())
    }
}

// =====================================================================
// Workloads
// =====================================================================

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkloadAttribute {
    pub name: String,
    pub class: String,
    /// User priority table per SPU (index 0..num_spus-1).
    pub priority: [u8; MAX_SPU as usize],
    pub min_contention: u32,
    pub max_contention: u32,
}

impl Default for WorkloadAttribute {
    fn default() -> Self {
        Self {
            name: String::new(),
            class: String::new(),
            priority: [0; MAX_SPU as usize],
            min_contention: 0,
            max_contention: 1,
        }
    }
}

fn validate_workload_attr(attr: &WorkloadAttribute, num_spus: u32) -> Result<(), CellError> {
    if attr.name.len() > 64 {
        return Err(errors::POLICY_INVAL);
    }
    if attr.max_contention == 0 || attr.max_contention > num_spus {
        return Err(errors::POLICY_INVAL);
    }
    if attr.min_contention > attr.max_contention {
        return Err(errors::POLICY_INVAL);
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkloadState {
    Runnable,
    Shutdown,
    Removable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkloadInfo {
    pub id: u32,
    pub name: String,
    pub state: WorkloadState,
    pub class: String,
}

// =====================================================================
// Registry (SPURS instances)
// =====================================================================

pub trait SpursRegistry {
    fn spurs_initialize(&mut self, attr: SpursAttribute) -> Result<u32, CellError>;
    fn spurs_finalize(&mut self, spurs_id: u32) -> Result<(), CellError>;
    fn spurs_add_workload(
        &mut self,
        spurs_id: u32,
        attr: WorkloadAttribute,
    ) -> Result<u32, CellError>;
    fn spurs_remove_workload(&mut self, spurs_id: u32, workload_id: u32) -> Result<(), CellError>;
    fn spurs_shutdown_workload(
        &mut self,
        spurs_id: u32,
        workload_id: u32,
    ) -> Result<(), CellError>;
    fn spurs_wait_for_workload_shutdown(
        &mut self,
        spurs_id: u32,
        workload_id: u32,
    ) -> Result<(), CellError>;
    fn spurs_get_workload_info(
        &self,
        spurs_id: u32,
        workload_id: u32,
    ) -> Result<WorkloadInfo, CellError>;
}

// =====================================================================
// Syscalls (HLE wrappers)
// =====================================================================

/// `cellSpursAttributeInitialize(attr_out, num_spus, spu_priority,
/// ppu_priority, exit_if_no_work)` — populate an attribute struct.
#[must_use]
pub fn cell_spurs_attribute_initialize(
    num_spus: u32,
    spu_priority: u32,
    ppu_priority: u32,
    exit_if_no_work: bool,
) -> Result<SpursAttribute, CellError> {
    let attr = SpursAttribute {
        num_spus,
        spu_priority,
        ppu_priority,
        exit_if_no_work,
        ..SpursAttribute::default()
    };
    attr.validate()?;
    Ok(attr)
}

/// `cellSpursAttributeSetNamePrefix(attr, prefix, prefix_len)`.
#[must_use]
pub fn cell_spurs_attribute_set_name_prefix(
    attr: &mut SpursAttribute,
    name: &str,
) -> Result<(), CellError> {
    if name.len() > 15 {
        // CELL_SPURS_NAME_MAX_LENGTH = 15.
        return Err(errors::CORE_INVAL);
    }
    attr.name = name.to_owned();
    Ok(())
}

/// `cellSpursAttributeEnableSpuPrintfIfAvailable(attr)`.
#[must_use]
pub fn cell_spurs_attribute_enable_spu_printf(
    attr: &mut SpursAttribute,
) -> Result<(), CellError> {
    attr.enable_spu_printf = true;
    Ok(())
}

/// `cellSpursInitializeWithAttribute(spurs_out, attr)` — create an
/// instance. Caller would normally pass a pre-allocated 4KB aligned
/// buffer; we abstract that away via the registry.
#[must_use]
pub fn cell_spurs_initialize_with_attribute<R: SpursRegistry + ?Sized>(
    reg: &mut R,
    attr: SpursAttribute,
) -> Result<u32, CellError> {
    attr.validate()?;
    reg.spurs_initialize(attr)
}

/// Convenience: `cellSpursInitialize(spurs_out, num_spus, spu_priority,
/// ppu_priority, exit_if_no_work)` — builds the attribute on the fly.
#[must_use]
pub fn cell_spurs_initialize<R: SpursRegistry + ?Sized>(
    reg: &mut R,
    num_spus: u32,
    spu_priority: u32,
    ppu_priority: u32,
    exit_if_no_work: bool,
) -> Result<u32, CellError> {
    let attr = cell_spurs_attribute_initialize(num_spus, spu_priority, ppu_priority, exit_if_no_work)?;
    reg.spurs_initialize(attr)
}

/// `cellSpursFinalize(spurs)`.
#[must_use]
pub fn cell_spurs_finalize<R: SpursRegistry + ?Sized>(
    reg: &mut R,
    spurs_id: u32,
) -> Result<(), CellError> {
    reg.spurs_finalize(spurs_id)
}

/// `cellSpursAddWorkload(spurs, wid_out, attr)`.
#[must_use]
pub fn cell_spurs_add_workload<R: SpursRegistry + ?Sized>(
    reg: &mut R,
    spurs_id: u32,
    attr: WorkloadAttribute,
) -> Result<u32, CellError> {
    reg.spurs_add_workload(spurs_id, attr)
}

/// `cellSpursRemoveWorkload(spurs, wid)` — workload must be in the
/// shutdown state (games call `ShutdownWorkload` first).
#[must_use]
pub fn cell_spurs_remove_workload<R: SpursRegistry + ?Sized>(
    reg: &mut R,
    spurs_id: u32,
    workload_id: u32,
) -> Result<(), CellError> {
    reg.spurs_remove_workload(spurs_id, workload_id)
}

/// `cellSpursShutdownWorkload(spurs, wid)`.
#[must_use]
pub fn cell_spurs_shutdown_workload<R: SpursRegistry + ?Sized>(
    reg: &mut R,
    spurs_id: u32,
    workload_id: u32,
) -> Result<(), CellError> {
    reg.spurs_shutdown_workload(spurs_id, workload_id)
}

/// `cellSpursWaitForWorkloadShutdown(spurs, wid)`.
#[must_use]
pub fn cell_spurs_wait_for_workload_shutdown<R: SpursRegistry + ?Sized>(
    reg: &mut R,
    spurs_id: u32,
    workload_id: u32,
) -> Result<(), CellError> {
    reg.spurs_wait_for_workload_shutdown(spurs_id, workload_id)
}

/// `cellSpursGetWorkloadInfo(spurs, wid, info_out)`.
#[must_use]
pub fn cell_spurs_get_workload_info<R: SpursRegistry + ?Sized>(
    reg: &R,
    spurs_id: u32,
    workload_id: u32,
) -> Result<WorkloadInfo, CellError> {
    reg.spurs_get_workload_info(spurs_id, workload_id)
}

// =====================================================================
// Reference registry
// =====================================================================

#[derive(Debug, Default)]
pub struct TestSpursRegistry {
    next_id: u32,
    instances: std::collections::BTreeMap<u32, Instance>,
}

#[derive(Debug)]
struct Instance {
    attr: SpursAttribute,
    next_wid: u32,
    workloads: std::collections::BTreeMap<u32, Workload>,
}

#[derive(Debug)]
struct Workload {
    attr: WorkloadAttribute,
    state: WorkloadState,
}

impl TestSpursRegistry {
    fn alloc_id(&mut self) -> u32 {
        self.next_id += 1;
        self.next_id
    }

    #[must_use]
    pub fn workload_count(&self, spurs_id: u32) -> Option<usize> {
        self.instances.get(&spurs_id).map(|i| i.workloads.len())
    }
}

impl SpursRegistry for TestSpursRegistry {
    fn spurs_initialize(&mut self, attr: SpursAttribute) -> Result<u32, CellError> {
        let id = self.alloc_id();
        self.instances.insert(
            id,
            Instance {
                attr,
                next_wid: 0,
                workloads: std::collections::BTreeMap::new(),
            },
        );
        Ok(id)
    }

    fn spurs_finalize(&mut self, spurs_id: u32) -> Result<(), CellError> {
        let inst = self.instances.get(&spurs_id).ok_or(errors::CORE_SRCH)?;
        if !inst.workloads.is_empty() {
            return Err(errors::CORE_BUSY);
        }
        self.instances.remove(&spurs_id);
        Ok(())
    }

    fn spurs_add_workload(
        &mut self,
        spurs_id: u32,
        attr: WorkloadAttribute,
    ) -> Result<u32, CellError> {
        let inst = self.instances.get_mut(&spurs_id).ok_or(errors::CORE_SRCH)?;
        validate_workload_attr(&attr, inst.attr.num_spus)?;
        if inst.workloads.len() as u32 >= MAX_WORKLOAD {
            return Err(errors::POLICY_AGAIN);
        }
        let wid = inst.next_wid;
        inst.next_wid += 1;
        inst.workloads.insert(
            wid,
            Workload { attr, state: WorkloadState::Runnable },
        );
        Ok(wid)
    }

    fn spurs_remove_workload(&mut self, spurs_id: u32, wid: u32) -> Result<(), CellError> {
        let inst = self.instances.get_mut(&spurs_id).ok_or(errors::CORE_SRCH)?;
        let wk = inst.workloads.get(&wid).ok_or(errors::POLICY_SRCH)?;
        if wk.state != WorkloadState::Shutdown && wk.state != WorkloadState::Removable {
            return Err(errors::POLICY_STAT);
        }
        inst.workloads.remove(&wid);
        Ok(())
    }

    fn spurs_shutdown_workload(&mut self, spurs_id: u32, wid: u32) -> Result<(), CellError> {
        let inst = self.instances.get_mut(&spurs_id).ok_or(errors::CORE_SRCH)?;
        let wk = inst.workloads.get_mut(&wid).ok_or(errors::POLICY_SRCH)?;
        if wk.state != WorkloadState::Runnable {
            return Err(errors::POLICY_STAT);
        }
        wk.state = WorkloadState::Shutdown;
        Ok(())
    }

    fn spurs_wait_for_workload_shutdown(
        &mut self,
        spurs_id: u32,
        wid: u32,
    ) -> Result<(), CellError> {
        let inst = self.instances.get_mut(&spurs_id).ok_or(errors::CORE_SRCH)?;
        let wk = inst.workloads.get_mut(&wid).ok_or(errors::POLICY_SRCH)?;
        if wk.state != WorkloadState::Shutdown {
            return Err(errors::POLICY_STAT);
        }
        // In a real impl this waits on the SPU side; in our model the
        // wait completes immediately by moving to Removable.
        wk.state = WorkloadState::Removable;
        Ok(())
    }

    fn spurs_get_workload_info(
        &self,
        spurs_id: u32,
        wid: u32,
    ) -> Result<WorkloadInfo, CellError> {
        let inst = self.instances.get(&spurs_id).ok_or(errors::CORE_SRCH)?;
        let wk = inst.workloads.get(&wid).ok_or(errors::POLICY_SRCH)?;
        Ok(WorkloadInfo {
            id: wid,
            name: wk.attr.name.clone(),
            state: wk.state,
            class: wk.attr.class.clone(),
        })
    }
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn default_attr() -> SpursAttribute {
        SpursAttribute {
            num_spus: 2,
            spu_priority: 10,
            ppu_priority: 100,
            ..SpursAttribute::default()
        }
    }

    fn default_wk_attr() -> WorkloadAttribute {
        WorkloadAttribute {
            name: "wk".into(),
            class: "test".into(),
            priority: [1, 1, 0, 0, 0, 0, 0, 0],
            min_contention: 0,
            max_contention: 2,
        }
    }

    // --- constants ------------------------------------------------

    #[test]
    fn error_codes_byte_exact_vs_cellSpurs_h() {
        assert_eq!(errors::CORE_AGAIN.0, 0x8041_0701);
        assert_eq!(errors::CORE_BUSY.0, 0x8041_070A);
        assert_eq!(errors::POLICY_SRCH.0, 0x8041_0805);
        assert_eq!(errors::POLICY_STAT.0, 0x8041_080F);
        assert_eq!(errors::TASK_FATAL.0, 0x8041_0914);
        assert_eq!(errors::TASK_SHUTDOWN.0, 0x8041_0920);
    }

    #[test]
    fn layout_constants_frozen() {
        assert_eq!(MAX_SPU, 8);
        assert_eq!(MAX_WORKLOAD, 16);
        assert_eq!(MAX_WORKLOAD2, 32);
        assert_eq!(MAX_PRIORITY, 16);
        assert_eq!(MAX_TASK, 128);
        assert_eq!(MAX_TASK_NAME_LENGTH, 32);
    }

    // --- attribute validation -------------------------------------

    #[test]
    fn attr_init_rejects_zero_spus() {
        assert_eq!(
            cell_spurs_attribute_initialize(0, 10, 100, false).unwrap_err(),
            errors::CORE_INVAL,
        );
    }

    #[test]
    fn attr_init_rejects_more_than_8_spus() {
        assert_eq!(
            cell_spurs_attribute_initialize(9, 10, 100, false).unwrap_err(),
            errors::CORE_INVAL,
        );
    }

    #[test]
    fn attr_init_rejects_bad_spu_priority() {
        assert_eq!(
            cell_spurs_attribute_initialize(1, 0, 100, false).unwrap_err(),
            errors::CORE_INVAL,
        );
        assert_eq!(
            cell_spurs_attribute_initialize(1, 17, 100, false).unwrap_err(),
            errors::CORE_INVAL,
        );
    }

    #[test]
    fn attr_init_rejects_ppu_priority_out_of_range() {
        assert_eq!(
            cell_spurs_attribute_initialize(1, 10, 15, false).unwrap_err(),
            errors::CORE_INVAL,
        );
        assert_eq!(
            cell_spurs_attribute_initialize(1, 10, 256, false).unwrap_err(),
            errors::CORE_INVAL,
        );
    }

    #[test]
    fn attr_set_name_prefix_enforces_15_char_max() {
        let mut attr = default_attr();
        assert_eq!(
            cell_spurs_attribute_set_name_prefix(&mut attr, "0123456789abcdef").unwrap_err(),
            errors::CORE_INVAL,
        );
        cell_spurs_attribute_set_name_prefix(&mut attr, "shortname").unwrap();
        assert_eq!(attr.name, "shortname");
    }

    // --- instance lifecycle ---------------------------------------

    #[test]
    fn initialize_and_finalize_round_trip() {
        let mut reg = TestSpursRegistry::default();
        let id = cell_spurs_initialize_with_attribute(&mut reg, default_attr()).unwrap();
        assert!(reg.instances.contains_key(&id));
        cell_spurs_finalize(&mut reg, id).unwrap();
        assert!(!reg.instances.contains_key(&id));
    }

    #[test]
    fn finalize_unknown_is_srch() {
        let mut reg = TestSpursRegistry::default();
        assert_eq!(cell_spurs_finalize(&mut reg, 42).unwrap_err(), errors::CORE_SRCH);
    }

    // --- workload lifecycle ---------------------------------------

    #[test]
    fn add_workload_happy_path() {
        let mut reg = TestSpursRegistry::default();
        let sid = cell_spurs_initialize_with_attribute(&mut reg, default_attr()).unwrap();
        let wid = cell_spurs_add_workload(&mut reg, sid, default_wk_attr()).unwrap();
        assert_eq!(wid, 0);
        let info = cell_spurs_get_workload_info(&reg, sid, wid).unwrap();
        assert_eq!(info.state, WorkloadState::Runnable);
        assert_eq!(info.class, "test");
    }

    #[test]
    fn workload_attr_rejects_zero_max_contention() {
        let mut reg = TestSpursRegistry::default();
        let sid = cell_spurs_initialize_with_attribute(&mut reg, default_attr()).unwrap();
        let bad = WorkloadAttribute { max_contention: 0, ..default_wk_attr() };
        assert_eq!(
            cell_spurs_add_workload(&mut reg, sid, bad).unwrap_err(),
            errors::POLICY_INVAL,
        );
    }

    #[test]
    fn workload_attr_rejects_contention_exceeding_num_spus() {
        let mut reg = TestSpursRegistry::default();
        let sid = cell_spurs_initialize_with_attribute(&mut reg, default_attr()).unwrap();
        let bad = WorkloadAttribute { max_contention: 99, ..default_wk_attr() };
        assert_eq!(
            cell_spurs_add_workload(&mut reg, sid, bad).unwrap_err(),
            errors::POLICY_INVAL,
        );
    }

    #[test]
    fn add_workload_beyond_max_is_again() {
        let mut reg = TestSpursRegistry::default();
        let sid = cell_spurs_initialize_with_attribute(&mut reg, default_attr()).unwrap();
        for _ in 0..MAX_WORKLOAD {
            cell_spurs_add_workload(&mut reg, sid, default_wk_attr()).unwrap();
        }
        assert_eq!(
            cell_spurs_add_workload(&mut reg, sid, default_wk_attr()).unwrap_err(),
            errors::POLICY_AGAIN,
        );
    }

    #[test]
    fn remove_runnable_workload_is_stat() {
        let mut reg = TestSpursRegistry::default();
        let sid = cell_spurs_initialize_with_attribute(&mut reg, default_attr()).unwrap();
        let wid = cell_spurs_add_workload(&mut reg, sid, default_wk_attr()).unwrap();
        assert_eq!(
            cell_spurs_remove_workload(&mut reg, sid, wid).unwrap_err(),
            errors::POLICY_STAT,
        );
    }

    #[test]
    fn workload_full_lifecycle_shutdown_wait_remove() {
        let mut reg = TestSpursRegistry::default();
        let sid = cell_spurs_initialize_with_attribute(&mut reg, default_attr()).unwrap();
        let wid = cell_spurs_add_workload(&mut reg, sid, default_wk_attr()).unwrap();

        cell_spurs_shutdown_workload(&mut reg, sid, wid).unwrap();
        let info = cell_spurs_get_workload_info(&reg, sid, wid).unwrap();
        assert_eq!(info.state, WorkloadState::Shutdown);

        cell_spurs_wait_for_workload_shutdown(&mut reg, sid, wid).unwrap();
        let info = cell_spurs_get_workload_info(&reg, sid, wid).unwrap();
        assert_eq!(info.state, WorkloadState::Removable);

        cell_spurs_remove_workload(&mut reg, sid, wid).unwrap();
        assert_eq!(reg.workload_count(sid), Some(0));
    }

    #[test]
    fn shutdown_twice_is_stat() {
        let mut reg = TestSpursRegistry::default();
        let sid = cell_spurs_initialize_with_attribute(&mut reg, default_attr()).unwrap();
        let wid = cell_spurs_add_workload(&mut reg, sid, default_wk_attr()).unwrap();
        cell_spurs_shutdown_workload(&mut reg, sid, wid).unwrap();
        assert_eq!(
            cell_spurs_shutdown_workload(&mut reg, sid, wid).unwrap_err(),
            errors::POLICY_STAT,
        );
    }

    #[test]
    fn wait_before_shutdown_is_stat() {
        let mut reg = TestSpursRegistry::default();
        let sid = cell_spurs_initialize_with_attribute(&mut reg, default_attr()).unwrap();
        let wid = cell_spurs_add_workload(&mut reg, sid, default_wk_attr()).unwrap();
        assert_eq!(
            cell_spurs_wait_for_workload_shutdown(&mut reg, sid, wid).unwrap_err(),
            errors::POLICY_STAT,
        );
    }

    #[test]
    fn finalize_with_workloads_still_running_is_busy() {
        let mut reg = TestSpursRegistry::default();
        let sid = cell_spurs_initialize_with_attribute(&mut reg, default_attr()).unwrap();
        cell_spurs_add_workload(&mut reg, sid, default_wk_attr()).unwrap();
        assert_eq!(cell_spurs_finalize(&mut reg, sid).unwrap_err(), errors::CORE_BUSY);
    }

    #[test]
    fn unknown_workload_on_lookup_is_policy_srch() {
        let mut reg = TestSpursRegistry::default();
        let sid = cell_spurs_initialize_with_attribute(&mut reg, default_attr()).unwrap();
        assert_eq!(
            cell_spurs_get_workload_info(&reg, sid, 99).unwrap_err(),
            errors::POLICY_SRCH,
        );
    }

    #[test]
    fn enable_spu_printf_flips_flag() {
        let mut attr = default_attr();
        assert!(!attr.enable_spu_printf);
        cell_spurs_attribute_enable_spu_printf(&mut attr).unwrap();
        assert!(attr.enable_spu_printf);
    }
}
