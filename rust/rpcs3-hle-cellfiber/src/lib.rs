//! `rpcs3-hle-cellfiber` — PPU fiber (coroutine) HLE.
//!
//! Ports `rpcs3/Emu/Cell/Modules/cellFiber.cpp`. cellFiber exposes
//! cooperative coroutines to PPU code: the game creates a
//! `CellFiberPpuScheduler`, attaches fibers, and calls `Run` / `Yield`
//! to multiplex cooperatively on a single PPU thread. Beyond the
//! scheduler there is a lower-level `Context` primitive and a
//! `WorkerControl` helper.
//!
//! ## Entry points covered
//!
//! | HLE function                                    | Rust wrapper                              |
//! |-------------------------------------------------|-------------------------------------------|
//! | `cellFiberPpuInitialize`                        | [`Fiber::initialize`]                     |
//! | `cellFiberPpuSchedulerAttributeInitialize`      | [`SchedulerAttribute::initialize`]        |
//! | `cellFiberPpuInitializeScheduler`               | [`Fiber::init_scheduler`]                 |
//! | `cellFiberPpuFinalizeScheduler`                 | [`Fiber::finalize_scheduler`]             |
//! | `cellFiberPpuRunFibers`                         | [`Fiber::run_fibers`]                     |
//! | `cellFiberPpuCheckFlags`                        | [`Fiber::check_flags`]                    |
//! | `cellFiberPpuAttributeInitialize`               | [`FiberAttribute::initialize`]            |
//! | `cellFiberPpuCreateFiber`                       | [`Fiber::create_fiber`]                   |
//! | `cellFiberPpuExit`                              | [`Fiber::exit_fiber`]                     |
//! | `cellFiberPpuYield`                             | [`Fiber::yield_current`]                  |
//! | `cellFiberPpuJoinFiber`                         | [`Fiber::join`]                           |
//! | `cellFiberPpuContextInitialize`                 | [`Fiber::context_init`]                   |

use rpcs3_emu_types::CellError;

// =====================================================================
// Error codes — byte-exact with cellFiber.h:6-19
// =====================================================================

pub mod errors {
    use rpcs3_emu_types::CellError;

    pub const AGAIN: CellError = CellError(0x8076_0001);
    pub const INVAL: CellError = CellError(0x8076_0002);
    pub const NOMEM: CellError = CellError(0x8076_0004);
    pub const DEADLK: CellError = CellError(0x8076_0008);
    pub const PERM: CellError = CellError(0x8076_0009);
    pub const BUSY: CellError = CellError(0x8076_000A);
    pub const ABORT: CellError = CellError(0x8076_000C);
    pub const STAT: CellError = CellError(0x8076_000F);
    pub const ALIGN: CellError = CellError(0x8076_0010);
    pub const NULL_POINTER: CellError = CellError(0x8076_0011);
    pub const NOSYSINIT: CellError = CellError(0x8076_0020);
}

// =====================================================================
// Alignment / size constants — byte-exact with cellFiber.h:25-117
// =====================================================================

pub const SCHEDULER_SIZE: usize = 512;
pub const SCHEDULER_ALIGN: u64 = 128;

pub const SCHEDULER_ATTRIBUTE_SIZE: usize = 256;
pub const SCHEDULER_ATTRIBUTE_ALIGN: u64 = 8;

pub const FIBER_SIZE: usize = 896;
pub const FIBER_ALIGN: u64 = 128;

pub const FIBER_ATTRIBUTE_SIZE: usize = 256;
pub const FIBER_ATTRIBUTE_ALIGN: u64 = 8;

pub const CONTEXT_SIZE: usize = 640;
pub const CONTEXT_ALIGN: u64 = 16;

pub const CONTEXT_ATTRIBUTE_SIZE: usize = 128;
pub const CONTEXT_ATTRIBUTE_ALIGN: u64 = 8;

pub const WORKER_CONTROL_SIZE: usize = 768;
pub const WORKER_CONTROL_ALIGN: u64 = 128;

pub const WORKER_CONTROL_ATTRIBUTE_SIZE: usize = 384;
pub const WORKER_CONTROL_ATTRIBUTE_ALIGN: u64 = 8;

pub const NAME_MAX_LENGTH: usize = 31; // name[32] incl. NUL

// =====================================================================
// Attributes
// =====================================================================

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SchedulerAttribute {
    pub auto_check_flags: bool,
    pub debugger_support: bool,
    pub auto_check_flags_interval_usec: u32,
}

impl SchedulerAttribute {
    /// `cellFiberPpuSchedulerAttributeInitialize`. Sets library defaults.
    #[must_use]
    pub fn initialize() -> Self {
        Self {
            auto_check_flags: false,
            debugger_support: false,
            auto_check_flags_interval_usec: 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FiberAttribute {
    pub name: String,
    pub on_exit_callback: bool, // whether a callback is registered
    pub on_exit_callback_arg: u64,
}

impl FiberAttribute {
    #[must_use]
    pub fn initialize() -> Self {
        Self { name: String::new(), on_exit_callback: false, on_exit_callback_arg: 0 }
    }

    fn validate(&self) -> Result<(), CellError> {
        if self.name.len() > NAME_MAX_LENGTH {
            return Err(errors::INVAL);
        }
        if self.name.bytes().any(|b| b < 0x20 || b == 0x7F) {
            return Err(errors::INVAL);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContextAttribute {
    pub name: String,
    pub debugger_support: bool,
}

impl ContextAttribute {
    #[must_use]
    pub fn initialize() -> Self {
        Self { name: String::new(), debugger_support: false }
    }

    fn validate(&self) -> Result<(), CellError> {
        if self.name.len() > NAME_MAX_LENGTH {
            return Err(errors::INVAL);
        }
        if self.name.bytes().any(|b| b < 0x20 || b == 0x7F) {
            return Err(errors::INVAL);
        }
        Ok(())
    }
}

// =====================================================================
// Internal state
// =====================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FiberState {
    Ready,
    Running,
    Yielded,
    Exited,
}

#[derive(Clone, Debug)]
struct FiberEntry {
    id: u32,
    #[allow(dead_code)]
    entry_arg: u64,
    state: FiberState,
    exit_code: s32_helper::S32,
    name: String,
    attr: FiberAttribute,
    on_exit_fired: bool,
}

mod s32_helper {
    pub type S32 = i32;
}

#[derive(Clone, Debug)]
struct Scheduler {
    id: u32,
    attr: SchedulerAttribute,
    flags: u32,
    fibers: Vec<FiberEntry>,
    current_fiber: Option<u32>,
    next_fiber_id: u32,
}

#[derive(Clone, Debug)]
struct ContextEntry {
    id: u32,
    #[allow(dead_code)]
    attr: ContextAttribute,
}

#[derive(Clone, Debug, Default)]
pub struct Fiber {
    initialized: bool,
    schedulers: Vec<Scheduler>,
    contexts: Vec<ContextEntry>,
    next_scheduler_id: u32,
    next_context_id: u32,
}

pub const MAX_SCHEDULERS: usize = 16;
pub const MAX_FIBERS_PER_SCHEDULER: usize = 256;
pub const MAX_CONTEXTS: usize = 256;

impl Fiber {
    #[must_use]
    pub fn new() -> Self {
        Self { next_scheduler_id: 1, next_context_id: 1, ..Default::default() }
    }

    #[must_use]
    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    // ----------------- Lifecycle -----------------

    pub fn initialize(&mut self, pool_addr: u64, pool_size: u32) -> Result<(), CellError> {
        if self.initialized {
            return Err(errors::BUSY);
        }
        if pool_addr == 0 {
            return Err(errors::NULL_POINTER);
        }
        if pool_addr % 128 != 0 {
            return Err(errors::ALIGN);
        }
        if pool_size < (SCHEDULER_SIZE as u32) {
            return Err(errors::NOMEM);
        }
        self.initialized = true;
        Ok(())
    }

    fn require_sysinit(&self) -> Result<(), CellError> {
        if self.initialized { Ok(()) } else { Err(errors::NOSYSINIT) }
    }

    // ----------------- Scheduler -----------------

    pub fn init_scheduler(&mut self, addr: u64, attr: &SchedulerAttribute) -> Result<u32, CellError> {
        self.require_sysinit()?;
        validate_alignment(addr, SCHEDULER_ALIGN)?;
        if attr.auto_check_flags && attr.auto_check_flags_interval_usec == 0 {
            return Err(errors::INVAL);
        }
        if self.schedulers.len() >= MAX_SCHEDULERS {
            return Err(errors::NOMEM);
        }
        let id = self.next_scheduler_id;
        self.next_scheduler_id = self.next_scheduler_id.checked_add(1).ok_or(errors::NOMEM)?;
        self.schedulers.push(Scheduler {
            id,
            attr: attr.clone(),
            flags: 0,
            fibers: Vec::new(),
            current_fiber: None,
            next_fiber_id: 1,
        });
        Ok(id)
    }

    pub fn finalize_scheduler(&mut self, scheduler_id: u32) -> Result<(), CellError> {
        self.require_sysinit()?;
        let idx = self.sched_idx(scheduler_id)?;
        if self.schedulers[idx].fibers.iter().any(|f| f.state != FiberState::Exited) {
            return Err(errors::BUSY);
        }
        self.schedulers.remove(idx);
        Ok(())
    }

    /// `cellFiberPpuRunFibers(scheduler)`. Walks the ready-list and
    /// marks each fiber as Running → Yielded (simulated cooperative
    /// schedule). Tests assert the fairness sequence.
    pub fn run_fibers(&mut self, scheduler_id: u32) -> Result<u32, CellError> {
        self.require_sysinit()?;
        let idx = self.sched_idx(scheduler_id)?;
        let mut ran = 0u32;
        for fiber in self.schedulers[idx].fibers.iter_mut() {
            if fiber.state == FiberState::Ready || fiber.state == FiberState::Yielded {
                fiber.state = FiberState::Running;
                ran += 1;
                // Cooperative: end of slice returns to yielded state.
                fiber.state = FiberState::Yielded;
            }
        }
        if ran == 0 && self.schedulers[idx].fibers.is_empty() {
            return Err(errors::STAT);
        }
        Ok(ran)
    }

    pub fn check_flags(&self, scheduler_id: u32) -> Result<u32, CellError> {
        self.require_sysinit()?;
        let idx = self.sched_idx(scheduler_id)?;
        Ok(self.schedulers[idx].flags)
    }

    pub fn raise_flag(&mut self, scheduler_id: u32, flag: u32) -> Result<(), CellError> {
        self.require_sysinit()?;
        let idx = self.sched_idx(scheduler_id)?;
        self.schedulers[idx].flags |= flag;
        Ok(())
    }

    // ----------------- Fibers -----------------

    pub fn create_fiber(
        &mut self,
        scheduler_id: u32,
        fiber_addr: u64,
        entry_arg: u64,
        priority: i32,
        attr: &FiberAttribute,
    ) -> Result<u32, CellError> {
        self.require_sysinit()?;
        validate_alignment(fiber_addr, FIBER_ALIGN)?;
        if !(0..=1000).contains(&priority) {
            return Err(errors::INVAL);
        }
        attr.validate()?;
        let idx = self.sched_idx(scheduler_id)?;
        if self.schedulers[idx].fibers.len() >= MAX_FIBERS_PER_SCHEDULER {
            return Err(errors::NOMEM);
        }
        let fid = self.schedulers[idx].next_fiber_id;
        self.schedulers[idx].next_fiber_id = self.schedulers[idx]
            .next_fiber_id
            .checked_add(1)
            .ok_or(errors::NOMEM)?;
        let name = attr.name.clone();
        self.schedulers[idx].fibers.push(FiberEntry {
            id: fid,
            entry_arg,
            state: FiberState::Ready,
            exit_code: 0,
            name,
            attr: attr.clone(),
            on_exit_fired: false,
        });
        Ok(fid)
    }

    pub fn exit_fiber(&mut self, scheduler_id: u32, fiber_id: u32, exit_code: i32) -> Result<(), CellError> {
        self.require_sysinit()?;
        let idx = self.sched_idx(scheduler_id)?;
        let fiber = self.fiber_mut(idx, fiber_id)?;
        if fiber.state == FiberState::Exited {
            return Err(errors::STAT);
        }
        fiber.state = FiberState::Exited;
        fiber.exit_code = exit_code;
        // Fire on-exit callback if registered.
        if fiber.attr.on_exit_callback {
            fiber.on_exit_fired = true;
        }
        Ok(())
    }

    pub fn yield_current(&mut self, scheduler_id: u32, fiber_id: u32) -> Result<(), CellError> {
        self.require_sysinit()?;
        let idx = self.sched_idx(scheduler_id)?;
        let fiber = self.fiber_mut(idx, fiber_id)?;
        match fiber.state {
            FiberState::Running | FiberState::Ready => {
                fiber.state = FiberState::Yielded;
                Ok(())
            }
            FiberState::Yielded => Err(errors::STAT),
            FiberState::Exited => Err(errors::STAT),
        }
    }

    /// `cellFiberPpuJoinFiber(fiber, exitCode_out)`. Returns the exit
    /// code of a terminated fiber; errors if still running.
    pub fn join(&self, scheduler_id: u32, fiber_id: u32) -> Result<i32, CellError> {
        self.require_sysinit()?;
        let idx = self.sched_idx(scheduler_id)?;
        let fiber = self.schedulers[idx]
            .fibers
            .iter()
            .find(|f| f.id == fiber_id)
            .ok_or(errors::STAT)?;
        match fiber.state {
            FiberState::Exited => Ok(fiber.exit_code),
            _ => Err(errors::BUSY),
        }
    }

    pub fn fiber_state(&self, scheduler_id: u32, fiber_id: u32) -> Result<FiberState, CellError> {
        self.require_sysinit()?;
        let idx = self.sched_idx(scheduler_id)?;
        let fiber = self.schedulers[idx]
            .fibers
            .iter()
            .find(|f| f.id == fiber_id)
            .ok_or(errors::STAT)?;
        Ok(fiber.state)
    }

    #[must_use]
    pub fn fiber_on_exit_fired(&self, scheduler_id: u32, fiber_id: u32) -> bool {
        let Ok(idx) = self.sched_idx(scheduler_id) else { return false };
        self.schedulers[idx]
            .fibers
            .iter()
            .find(|f| f.id == fiber_id)
            .is_some_and(|f| f.on_exit_fired)
    }

    #[must_use]
    pub fn fiber_name(&self, scheduler_id: u32, fiber_id: u32) -> Option<&str> {
        let idx = self.sched_idx(scheduler_id).ok()?;
        Some(self.schedulers[idx].fibers.iter().find(|f| f.id == fiber_id)?.name.as_str())
    }

    // ----------------- Contexts (lower-level fibers) -----------------

    pub fn context_init(&mut self, addr: u64, attr: &ContextAttribute) -> Result<u32, CellError> {
        self.require_sysinit()?;
        validate_alignment(addr, CONTEXT_ALIGN)?;
        attr.validate()?;
        if self.contexts.len() >= MAX_CONTEXTS {
            return Err(errors::NOMEM);
        }
        let id = self.next_context_id;
        self.next_context_id = self.next_context_id.checked_add(1).ok_or(errors::NOMEM)?;
        self.contexts.push(ContextEntry { id, attr: attr.clone() });
        Ok(id)
    }

    pub fn context_finalize(&mut self, context_id: u32) -> Result<(), CellError> {
        self.require_sysinit()?;
        let idx = self.contexts.iter().position(|c| c.id == context_id).ok_or(errors::STAT)?;
        self.contexts.remove(idx);
        Ok(())
    }

    #[must_use]
    pub fn context_count(&self) -> usize {
        self.contexts.len()
    }

    // ----------------- Helpers -----------------

    fn sched_idx(&self, id: u32) -> Result<usize, CellError> {
        self.schedulers.iter().position(|s| s.id == id).ok_or(errors::STAT)
    }

    fn fiber_mut(&mut self, sched_idx: usize, fiber_id: u32) -> Result<&mut FiberEntry, CellError> {
        self.schedulers[sched_idx]
            .fibers
            .iter_mut()
            .find(|f| f.id == fiber_id)
            .ok_or(errors::STAT)
    }
}

fn validate_alignment(addr: u64, align: u64) -> Result<(), CellError> {
    if addr == 0 {
        return Err(errors::NULL_POINTER);
    }
    if addr % align != 0 {
        return Err(errors::ALIGN);
    }
    Ok(())
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    const POOL: u64 = 0x2000_0080;
    const SCHED_ADDR: u64 = 0x2000_0100; // 128-byte aligned
    const FIBER_ADDR: u64 = 0x2000_0200; // 128-byte aligned
    const CTX_ADDR: u64 = 0x2000_0300; // 16-byte aligned

    fn init_fiber() -> Fiber {
        let mut f = Fiber::new();
        f.initialize(POOL, 4 * 1024).unwrap();
        f
    }

    #[test]
    fn error_codes_byte_exact() {
        assert_eq!(errors::AGAIN.0, 0x8076_0001);
        assert_eq!(errors::INVAL.0, 0x8076_0002);
        assert_eq!(errors::NOMEM.0, 0x8076_0004);
        assert_eq!(errors::DEADLK.0, 0x8076_0008);
        assert_eq!(errors::PERM.0, 0x8076_0009);
        assert_eq!(errors::BUSY.0, 0x8076_000A);
        assert_eq!(errors::ABORT.0, 0x8076_000C);
        assert_eq!(errors::STAT.0, 0x8076_000F);
        assert_eq!(errors::ALIGN.0, 0x8076_0010);
        assert_eq!(errors::NULL_POINTER.0, 0x8076_0011);
        assert_eq!(errors::NOSYSINIT.0, 0x8076_0020);
    }

    #[test]
    fn size_constants_stable() {
        assert_eq!(SCHEDULER_SIZE, 512);
        assert_eq!(SCHEDULER_ALIGN, 128);
        assert_eq!(SCHEDULER_ATTRIBUTE_SIZE, 256);
        assert_eq!(FIBER_SIZE, 896);
        assert_eq!(FIBER_ALIGN, 128);
        assert_eq!(FIBER_ATTRIBUTE_SIZE, 256);
        assert_eq!(CONTEXT_SIZE, 640);
        assert_eq!(CONTEXT_ALIGN, 16);
        assert_eq!(CONTEXT_ATTRIBUTE_SIZE, 128);
        assert_eq!(WORKER_CONTROL_SIZE, 768);
        assert_eq!(WORKER_CONTROL_ALIGN, 128);
        assert_eq!(WORKER_CONTROL_ATTRIBUTE_SIZE, 384);
        assert_eq!(NAME_MAX_LENGTH, 31);
    }

    #[test]
    fn initialize_requires_aligned_pool() {
        let mut f = Fiber::new();
        assert_eq!(f.initialize(POOL + 1, 4096), Err(errors::ALIGN));
    }

    #[test]
    fn initialize_null_pool_rejected() {
        let mut f = Fiber::new();
        assert_eq!(f.initialize(0, 4096), Err(errors::NULL_POINTER));
    }

    #[test]
    fn initialize_pool_too_small_rejected() {
        let mut f = Fiber::new();
        assert_eq!(f.initialize(POOL, 16), Err(errors::NOMEM));
    }

    #[test]
    fn initialize_twice_rejected() {
        let mut f = init_fiber();
        assert_eq!(f.initialize(POOL, 4096), Err(errors::BUSY));
    }

    #[test]
    fn operations_without_sysinit_rejected() {
        let mut f = Fiber::new();
        assert_eq!(f.init_scheduler(SCHED_ADDR, &SchedulerAttribute::initialize()), Err(errors::NOSYSINIT));
        assert_eq!(f.finalize_scheduler(1), Err(errors::NOSYSINIT));
        assert_eq!(f.run_fibers(1), Err(errors::NOSYSINIT));
        assert_eq!(f.check_flags(1), Err(errors::NOSYSINIT));
        assert_eq!(f.context_init(CTX_ADDR, &ContextAttribute::initialize()), Err(errors::NOSYSINIT));
    }

    #[test]
    fn scheduler_attribute_initialize_defaults() {
        let a = SchedulerAttribute::initialize();
        assert!(!a.auto_check_flags);
        assert!(!a.debugger_support);
        assert_eq!(a.auto_check_flags_interval_usec, 0);
    }

    #[test]
    fn init_scheduler_happy_path() {
        let mut f = init_fiber();
        let id = f.init_scheduler(SCHED_ADDR, &SchedulerAttribute::initialize()).unwrap();
        assert_eq!(id, 1);
    }

    #[test]
    fn init_scheduler_alignment_rejected() {
        let mut f = init_fiber();
        assert_eq!(
            f.init_scheduler(SCHED_ADDR + 1, &SchedulerAttribute::initialize()),
            Err(errors::ALIGN)
        );
    }

    #[test]
    fn init_scheduler_auto_check_without_interval_rejected() {
        let mut f = init_fiber();
        let mut a = SchedulerAttribute::initialize();
        a.auto_check_flags = true;
        a.auto_check_flags_interval_usec = 0;
        assert_eq!(f.init_scheduler(SCHED_ADDR, &a), Err(errors::INVAL));
    }

    #[test]
    fn init_scheduler_exceeds_max_rejected() {
        let mut f = init_fiber();
        for i in 0..MAX_SCHEDULERS {
            let addr = SCHED_ADDR + (i as u64) * SCHEDULER_ALIGN;
            f.init_scheduler(addr, &SchedulerAttribute::initialize()).unwrap();
        }
        assert_eq!(
            f.init_scheduler(SCHED_ADDR + MAX_SCHEDULERS as u64 * SCHEDULER_ALIGN, &SchedulerAttribute::initialize()),
            Err(errors::NOMEM)
        );
    }

    #[test]
    fn finalize_scheduler_unknown_id_rejected() {
        let mut f = init_fiber();
        assert_eq!(f.finalize_scheduler(999), Err(errors::STAT));
    }

    #[test]
    fn finalize_scheduler_with_live_fiber_busy() {
        let mut f = init_fiber();
        let s = f.init_scheduler(SCHED_ADDR, &SchedulerAttribute::initialize()).unwrap();
        f.create_fiber(s, FIBER_ADDR, 0, 10, &FiberAttribute::initialize()).unwrap();
        assert_eq!(f.finalize_scheduler(s), Err(errors::BUSY));
    }

    #[test]
    fn create_fiber_alignment_rejected() {
        let mut f = init_fiber();
        let s = f.init_scheduler(SCHED_ADDR, &SchedulerAttribute::initialize()).unwrap();
        assert_eq!(
            f.create_fiber(s, FIBER_ADDR + 1, 0, 10, &FiberAttribute::initialize()),
            Err(errors::ALIGN)
        );
    }

    #[test]
    fn create_fiber_bad_priority_rejected() {
        let mut f = init_fiber();
        let s = f.init_scheduler(SCHED_ADDR, &SchedulerAttribute::initialize()).unwrap();
        assert_eq!(
            f.create_fiber(s, FIBER_ADDR, 0, -1, &FiberAttribute::initialize()),
            Err(errors::INVAL)
        );
        assert_eq!(
            f.create_fiber(s, FIBER_ADDR, 0, 1001, &FiberAttribute::initialize()),
            Err(errors::INVAL)
        );
    }

    #[test]
    fn create_fiber_bad_name_rejected() {
        let mut f = init_fiber();
        let s = f.init_scheduler(SCHED_ADDR, &SchedulerAttribute::initialize()).unwrap();
        let mut a = FiberAttribute::initialize();
        a.name = "a".repeat(NAME_MAX_LENGTH + 1);
        assert_eq!(f.create_fiber(s, FIBER_ADDR, 0, 10, &a), Err(errors::INVAL));
        a.name = "control\x01char".into();
        assert_eq!(f.create_fiber(s, FIBER_ADDR, 0, 10, &a), Err(errors::INVAL));
    }

    #[test]
    fn create_fiber_records_name() {
        let mut f = init_fiber();
        let s = f.init_scheduler(SCHED_ADDR, &SchedulerAttribute::initialize()).unwrap();
        let mut a = FiberAttribute::initialize();
        a.name = "worker".into();
        let fid = f.create_fiber(s, FIBER_ADDR, 0, 10, &a).unwrap();
        assert_eq!(f.fiber_name(s, fid), Some("worker"));
    }

    #[test]
    fn run_fibers_transitions_ready_to_yielded() {
        let mut f = init_fiber();
        let s = f.init_scheduler(SCHED_ADDR, &SchedulerAttribute::initialize()).unwrap();
        let fid = f.create_fiber(s, FIBER_ADDR, 0, 10, &FiberAttribute::initialize()).unwrap();
        assert_eq!(f.fiber_state(s, fid), Ok(FiberState::Ready));
        let ran = f.run_fibers(s).unwrap();
        assert_eq!(ran, 1);
        assert_eq!(f.fiber_state(s, fid), Ok(FiberState::Yielded));
    }

    #[test]
    fn run_fibers_empty_scheduler_is_stat() {
        let mut f = init_fiber();
        let s = f.init_scheduler(SCHED_ADDR, &SchedulerAttribute::initialize()).unwrap();
        assert_eq!(f.run_fibers(s), Err(errors::STAT));
    }

    #[test]
    fn run_fibers_skips_exited() {
        let mut f = init_fiber();
        let s = f.init_scheduler(SCHED_ADDR, &SchedulerAttribute::initialize()).unwrap();
        let a = f.create_fiber(s, FIBER_ADDR, 0, 10, &FiberAttribute::initialize()).unwrap();
        let b = f.create_fiber(s, FIBER_ADDR + FIBER_ALIGN, 0, 10, &FiberAttribute::initialize()).unwrap();
        f.exit_fiber(s, a, 0).unwrap();
        let ran = f.run_fibers(s).unwrap();
        assert_eq!(ran, 1);
        assert_eq!(f.fiber_state(s, b), Ok(FiberState::Yielded));
    }

    #[test]
    fn exit_fiber_records_exit_code() {
        let mut f = init_fiber();
        let s = f.init_scheduler(SCHED_ADDR, &SchedulerAttribute::initialize()).unwrap();
        let fid = f.create_fiber(s, FIBER_ADDR, 0, 10, &FiberAttribute::initialize()).unwrap();
        f.exit_fiber(s, fid, 42).unwrap();
        assert_eq!(f.join(s, fid), Ok(42));
        assert_eq!(f.fiber_state(s, fid), Ok(FiberState::Exited));
    }

    #[test]
    fn exit_fiber_twice_rejected() {
        let mut f = init_fiber();
        let s = f.init_scheduler(SCHED_ADDR, &SchedulerAttribute::initialize()).unwrap();
        let fid = f.create_fiber(s, FIBER_ADDR, 0, 10, &FiberAttribute::initialize()).unwrap();
        f.exit_fiber(s, fid, 0).unwrap();
        assert_eq!(f.exit_fiber(s, fid, 0), Err(errors::STAT));
    }

    #[test]
    fn exit_fiber_fires_on_exit_callback() {
        let mut f = init_fiber();
        let s = f.init_scheduler(SCHED_ADDR, &SchedulerAttribute::initialize()).unwrap();
        let mut a = FiberAttribute::initialize();
        a.on_exit_callback = true;
        a.on_exit_callback_arg = 0xDEADBEEF;
        let fid = f.create_fiber(s, FIBER_ADDR, 0, 10, &a).unwrap();
        f.exit_fiber(s, fid, 0).unwrap();
        assert!(f.fiber_on_exit_fired(s, fid));
    }

    #[test]
    fn join_running_fiber_is_busy() {
        let mut f = init_fiber();
        let s = f.init_scheduler(SCHED_ADDR, &SchedulerAttribute::initialize()).unwrap();
        let fid = f.create_fiber(s, FIBER_ADDR, 0, 10, &FiberAttribute::initialize()).unwrap();
        assert_eq!(f.join(s, fid), Err(errors::BUSY));
    }

    #[test]
    fn yield_ready_or_running_ok() {
        let mut f = init_fiber();
        let s = f.init_scheduler(SCHED_ADDR, &SchedulerAttribute::initialize()).unwrap();
        let fid = f.create_fiber(s, FIBER_ADDR, 0, 10, &FiberAttribute::initialize()).unwrap();
        f.yield_current(s, fid).unwrap();
        assert_eq!(f.fiber_state(s, fid), Ok(FiberState::Yielded));
    }

    #[test]
    fn yield_already_yielded_rejected() {
        let mut f = init_fiber();
        let s = f.init_scheduler(SCHED_ADDR, &SchedulerAttribute::initialize()).unwrap();
        let fid = f.create_fiber(s, FIBER_ADDR, 0, 10, &FiberAttribute::initialize()).unwrap();
        f.yield_current(s, fid).unwrap();
        assert_eq!(f.yield_current(s, fid), Err(errors::STAT));
    }

    #[test]
    fn yield_exited_rejected() {
        let mut f = init_fiber();
        let s = f.init_scheduler(SCHED_ADDR, &SchedulerAttribute::initialize()).unwrap();
        let fid = f.create_fiber(s, FIBER_ADDR, 0, 10, &FiberAttribute::initialize()).unwrap();
        f.exit_fiber(s, fid, 0).unwrap();
        assert_eq!(f.yield_current(s, fid), Err(errors::STAT));
    }

    #[test]
    fn check_flags_and_raise() {
        let mut f = init_fiber();
        let s = f.init_scheduler(SCHED_ADDR, &SchedulerAttribute::initialize()).unwrap();
        assert_eq!(f.check_flags(s), Ok(0));
        f.raise_flag(s, 0x4).unwrap();
        f.raise_flag(s, 0x1).unwrap();
        assert_eq!(f.check_flags(s), Ok(0x5));
    }

    #[test]
    fn context_init_alignment_rejected() {
        let mut f = init_fiber();
        assert_eq!(
            f.context_init(CTX_ADDR + 1, &ContextAttribute::initialize()),
            Err(errors::ALIGN)
        );
    }

    #[test]
    fn context_init_happy_path() {
        let mut f = init_fiber();
        let id = f.context_init(CTX_ADDR, &ContextAttribute::initialize()).unwrap();
        assert_eq!(id, 1);
        assert_eq!(f.context_count(), 1);
    }

    #[test]
    fn context_finalize_unknown_rejected() {
        let mut f = init_fiber();
        assert_eq!(f.context_finalize(999), Err(errors::STAT));
    }

    #[test]
    fn context_bad_name_rejected() {
        let mut f = init_fiber();
        let mut a = ContextAttribute::initialize();
        a.name = "x".repeat(NAME_MAX_LENGTH + 1);
        assert_eq!(f.context_init(CTX_ADDR, &a), Err(errors::INVAL));
    }

    #[test]
    fn fiber_attribute_validate_accepts_empty_name() {
        FiberAttribute::initialize().validate().unwrap();
    }

    #[test]
    fn full_fiber_lifecycle_smoke() {
        let mut f = init_fiber();
        let sched = f
            .init_scheduler(
                SCHED_ADDR,
                &SchedulerAttribute {
                    auto_check_flags: true,
                    debugger_support: true,
                    auto_check_flags_interval_usec: 1000,
                },
            )
            .unwrap();

        let mut worker_attr = FiberAttribute::initialize();
        worker_attr.name = "worker".into();
        worker_attr.on_exit_callback = true;
        let a = f.create_fiber(sched, FIBER_ADDR, 0xAA, 10, &worker_attr).unwrap();
        let b = f.create_fiber(sched, FIBER_ADDR + FIBER_ALIGN, 0xBB, 20, &FiberAttribute::initialize()).unwrap();

        // Cooperative schedule round 1: both run and yield.
        assert_eq!(f.run_fibers(sched), Ok(2));
        assert_eq!(f.fiber_state(sched, a), Ok(FiberState::Yielded));
        assert_eq!(f.fiber_state(sched, b), Ok(FiberState::Yielded));

        // First fiber exits with code; join sees the value.
        f.exit_fiber(sched, a, 7).unwrap();
        assert!(f.fiber_on_exit_fired(sched, a));
        assert_eq!(f.join(sched, a), Ok(7));

        // Flag signaling.
        f.raise_flag(sched, 0x10).unwrap();
        assert_eq!(f.check_flags(sched), Ok(0x10));

        // Cleanup.
        f.exit_fiber(sched, b, 0).unwrap();
        f.finalize_scheduler(sched).unwrap();

        // Contexts: separate allocator.
        let c = f.context_init(CTX_ADDR, &ContextAttribute::initialize()).unwrap();
        f.context_finalize(c).unwrap();
    }
}
