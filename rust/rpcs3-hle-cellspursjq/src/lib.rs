//! Rust port of `rpcs3/Emu/Cell/Modules/cellSpursJq.cpp` — PS3 SPURS
//! Job Queue HLE surface.
//!
//! Upstream ships 63 all-stub entries. Three families visible in the names:
//!
//! * **JobQueue lifecycle + attribute setup** — 8× `cellSpursJobQueueAttribute*`,
//!   plus `SetWaitingMode`, `Shutdown`, 2× `Create*`, `Join`, 10× push
//!   variants (JobList/Job/Job2/PushAndRelease/Body/Sync/Flush/AllocateDescriptor),
//!   5× Get* accessors (Spurs/HandleCount/Error/MaxSize/JobQueueId), 2× Size
//!   getter, Open/Close, 3× Semaphore ops + Initialize, SendSignal.
//!
//! * **JobQueuePort** — Get/Push* variants mirroring the JobQueue push set,
//!   Sync/TrySync, Initialize/InitializeWithDescriptorBuffer, Finalize, and
//!   3× CopyPush* variants.
//!
//! * **JobQueuePort2** — fresh family with Create/Destroy/AllocateDescriptor
//!   and the same Push*/Sync set.
//!
//! Plus 3× SetException/SetException2/UnsetException event handlers.
//!
//! `no_std` + `alloc`. Single dep: `rpcs3-emu-types`.

#![no_std]

extern crate alloc;

use rpcs3_emu_types::CellError;

pub const MODULE_NAME: &str = "cellSpursJq";

/// 63 FNIDs in exact `REG_FUNC` order (cpp:386-448).
pub const REGISTERED_ENTRY_POINTS: &[&str] = &[
    "cellSpursJobQueueAttributeInitialize",
    "cellSpursJobQueueAttributeSetMaxGrab",
    "cellSpursJobQueueAttributeSetSubmitWithEntryLock",
    "cellSpursJobQueueAttributeSetDoBusyWaiting",
    "cellSpursJobQueueAttributeSetIsHaltOnError",
    "cellSpursJobQueueAttributeSetIsJobTypeMemoryCheck",
    "cellSpursJobQueueAttributeSetMaxSizeJobDescriptor",
    "cellSpursJobQueueAttributeSetGrabParameters",
    "cellSpursJobQueueSetWaitingMode",
    "cellSpursShutdownJobQueue",
    "_cellSpursCreateJobQueueWithJobDescriptorPool",
    "_cellSpursCreateJobQueue",
    "cellSpursJoinJobQueue",
    "_cellSpursJobQueuePushJobListBody",
    "_cellSpursJobQueuePushJobBody2",
    "_cellSpursJobQueuePushJob2Body",
    "_cellSpursJobQueuePushAndReleaseJobBody",
    "_cellSpursJobQueuePushJobBody",
    "_cellSpursJobQueuePushBody",
    "_cellSpursJobQueueAllocateJobDescriptorBody",
    "_cellSpursJobQueuePushSync",
    "_cellSpursJobQueuePushFlush",
    "cellSpursJobQueueGetSpurs",
    "cellSpursJobQueueGetHandleCount",
    "cellSpursJobQueueGetError",
    "cellSpursJobQueueGetMaxSizeJobDescriptor",
    "cellSpursGetJobQueueId",
    "cellSpursJobQueueGetSuspendedJobSize",
    "cellSpursJobQueueClose",
    "cellSpursJobQueueOpen",
    "cellSpursJobQueueSemaphoreTryAcquire",
    "cellSpursJobQueueSemaphoreAcquire",
    "cellSpursJobQueueSemaphoreInitialize",
    "cellSpursJobQueueSendSignal",
    "cellSpursJobQueuePortGetJobQueue",
    "_cellSpursJobQueuePortPushSync",
    "_cellSpursJobQueuePortPushFlush",
    "_cellSpursJobQueuePortPushJobListBody",
    "_cellSpursJobQueuePortPushJobBody",
    "_cellSpursJobQueuePortPushJobBody2",
    "_cellSpursJobQueuePortPushBody",
    "cellSpursJobQueuePortTrySync",
    "cellSpursJobQueuePortSync",
    "cellSpursJobQueuePortInitialize",
    "cellSpursJobQueuePortInitializeWithDescriptorBuffer",
    "cellSpursJobQueuePortFinalize",
    "_cellSpursJobQueuePortCopyPushJobBody",
    "_cellSpursJobQueuePortCopyPushJobBody2",
    "_cellSpursJobQueuePortCopyPushBody",
    "cellSpursJobQueuePort2GetJobQueue",
    "cellSpursJobQueuePort2PushSync",
    "cellSpursJobQueuePort2PushFlush",
    "_cellSpursJobQueuePort2PushJobListBody",
    "cellSpursJobQueuePort2Sync",
    "cellSpursJobQueuePort2Create",
    "cellSpursJobQueuePort2Destroy",
    "cellSpursJobQueuePort2AllocateJobDescriptor",
    "_cellSpursJobQueuePort2PushAndReleaseJobBody",
    "_cellSpursJobQueuePort2CopyPushJobBody",
    "_cellSpursJobQueuePort2PushJobBody",
    "cellSpursJobQueueSetExceptionEventHandler",
    "cellSpursJobQueueSetExceptionEventHandler2",
    "cellSpursJobQueueUnsetExceptionEventHandler",
];

// ---------------------------------------------------------------------------
// Placeholder error codes — upstream has no enum. Facility `0x8061_5B__`
// unused by any ported crate.
// ---------------------------------------------------------------------------

pub const CELL_SPURS_JQ_ERROR_NULL_POINTER: CellError = CellError(0x8061_5B01);
pub const CELL_SPURS_JQ_ERROR_NOT_INITIALIZED: CellError = CellError(0x8061_5B02);
pub const CELL_SPURS_JQ_ERROR_ALREADY_INITIALIZED: CellError = CellError(0x8061_5B03);
pub const CELL_SPURS_JQ_ERROR_BUSY: CellError = CellError(0x8061_5B04);
pub const CELL_SPURS_JQ_ERROR_JOIN: CellError = CellError(0x8061_5B05);
pub const CELL_SPURS_JQ_ERROR_FULL: CellError = CellError(0x8061_5B06);
pub const CELL_SPURS_JQ_ERROR_EMPTY: CellError = CellError(0x8061_5B07);

// ---------------------------------------------------------------------------
// Inferred FSM.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobQueueState {
    Uninit,
    Created,
    Open,
    Joined,
    Shutdown,
}

impl Default for JobQueueState {
    fn default() -> Self {
        JobQueueState::Uninit
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortState {
    Uninit,
    Initialized,
    Finalized,
}

impl Default for PortState {
    fn default() -> Self {
        PortState::Uninit
    }
}

// ---------------------------------------------------------------------------
// Manager.
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct SpursJq {
    pub job_queue: JobQueueState,
    pub port: PortState,
    pub port2: PortState,
    pub attribute_initialized: bool,
    pub handle_count: u32,
    pub suspended_job_size: u32,
    pub pending_pushes: u32,
    pub semaphore_initialized: bool,
    pub exception_handler_set: bool,

    // 63 per-entry counters.
    pub attribute_initialize_calls: u64,
    pub attribute_set_max_grab_calls: u64,
    pub attribute_set_submit_with_entry_lock_calls: u64,
    pub attribute_set_do_busy_waiting_calls: u64,
    pub attribute_set_is_halt_on_error_calls: u64,
    pub attribute_set_is_job_type_memory_check_calls: u64,
    pub attribute_set_max_size_job_descriptor_calls: u64,
    pub attribute_set_grab_parameters_calls: u64,
    pub set_waiting_mode_calls: u64,
    pub shutdown_job_queue_calls: u64,
    pub create_job_queue_with_pool_calls: u64,
    pub create_job_queue_calls: u64,
    pub join_job_queue_calls: u64,
    pub push_job_list_body_calls: u64,
    pub push_job_body2_calls: u64,
    pub push_job2_body_calls: u64,
    pub push_and_release_job_body_calls: u64,
    pub push_job_body_calls: u64,
    pub push_body_calls: u64,
    pub allocate_job_descriptor_body_calls: u64,
    pub push_sync_calls: u64,
    pub push_flush_calls: u64,
    pub get_spurs_calls: u64,
    pub get_handle_count_calls: u64,
    pub get_error_calls: u64,
    pub get_max_size_job_descriptor_calls: u64,
    pub get_job_queue_id_calls: u64,
    pub get_suspended_job_size_calls: u64,
    pub close_calls: u64,
    pub open_calls: u64,
    pub semaphore_try_acquire_calls: u64,
    pub semaphore_acquire_calls: u64,
    pub semaphore_initialize_calls: u64,
    pub send_signal_calls: u64,
    pub port_get_job_queue_calls: u64,
    pub port_push_sync_calls: u64,
    pub port_push_flush_calls: u64,
    pub port_push_job_list_body_calls: u64,
    pub port_push_job_body_calls: u64,
    pub port_push_job_body2_calls: u64,
    pub port_push_body_calls: u64,
    pub port_try_sync_calls: u64,
    pub port_sync_calls: u64,
    pub port_initialize_calls: u64,
    pub port_initialize_with_descriptor_buffer_calls: u64,
    pub port_finalize_calls: u64,
    pub port_copy_push_job_body_calls: u64,
    pub port_copy_push_job_body2_calls: u64,
    pub port_copy_push_body_calls: u64,
    pub port2_get_job_queue_calls: u64,
    pub port2_push_sync_calls: u64,
    pub port2_push_flush_calls: u64,
    pub port2_push_job_list_body_calls: u64,
    pub port2_sync_calls: u64,
    pub port2_create_calls: u64,
    pub port2_destroy_calls: u64,
    pub port2_allocate_job_descriptor_calls: u64,
    pub port2_push_and_release_job_body_calls: u64,
    pub port2_copy_push_job_body_calls: u64,
    pub port2_push_job_body_calls: u64,
    pub set_exception_event_handler_calls: u64,
    pub set_exception_event_handler2_calls: u64,
    pub unset_exception_event_handler_calls: u64,
}

impl SpursJq {
    pub fn new() -> Self {
        Self::default()
    }

    // -- Attribute setters -----------------------------------------------

    pub fn attribute_initialize(&mut self) -> Result<(), CellError> {
        self.attribute_initialize_calls = self.attribute_initialize_calls.saturating_add(1);
        self.attribute_initialized = true;
        Ok(())
    }

    pub fn attribute_set_max_grab(&mut self) -> Result<(), CellError> {
        self.attribute_set_max_grab_calls = self.attribute_set_max_grab_calls.saturating_add(1);
        Ok(())
    }

    pub fn attribute_set_submit_with_entry_lock(&mut self) -> Result<(), CellError> {
        self.attribute_set_submit_with_entry_lock_calls = self
            .attribute_set_submit_with_entry_lock_calls
            .saturating_add(1);
        Ok(())
    }

    pub fn attribute_set_do_busy_waiting(&mut self) -> Result<(), CellError> {
        self.attribute_set_do_busy_waiting_calls =
            self.attribute_set_do_busy_waiting_calls.saturating_add(1);
        Ok(())
    }

    pub fn attribute_set_is_halt_on_error(&mut self) -> Result<(), CellError> {
        self.attribute_set_is_halt_on_error_calls = self
            .attribute_set_is_halt_on_error_calls
            .saturating_add(1);
        Ok(())
    }

    pub fn attribute_set_is_job_type_memory_check(&mut self) -> Result<(), CellError> {
        self.attribute_set_is_job_type_memory_check_calls = self
            .attribute_set_is_job_type_memory_check_calls
            .saturating_add(1);
        Ok(())
    }

    pub fn attribute_set_max_size_job_descriptor(&mut self) -> Result<(), CellError> {
        self.attribute_set_max_size_job_descriptor_calls = self
            .attribute_set_max_size_job_descriptor_calls
            .saturating_add(1);
        Ok(())
    }

    pub fn attribute_set_grab_parameters(&mut self) -> Result<(), CellError> {
        self.attribute_set_grab_parameters_calls = self
            .attribute_set_grab_parameters_calls
            .saturating_add(1);
        Ok(())
    }

    pub fn set_waiting_mode(&mut self) -> Result<(), CellError> {
        self.set_waiting_mode_calls = self.set_waiting_mode_calls.saturating_add(1);
        Ok(())
    }

    // -- Lifecycle -------------------------------------------------------

    pub fn shutdown_job_queue(&mut self) -> Result<(), CellError> {
        self.shutdown_job_queue_calls = self.shutdown_job_queue_calls.saturating_add(1);
        self.job_queue = JobQueueState::Shutdown;
        Ok(())
    }

    pub fn create_job_queue_with_pool(&mut self) -> Result<(), CellError> {
        self.create_job_queue_with_pool_calls =
            self.create_job_queue_with_pool_calls.saturating_add(1);
        self.job_queue = JobQueueState::Created;
        Ok(())
    }

    pub fn create_job_queue(&mut self) -> Result<(), CellError> {
        self.create_job_queue_calls = self.create_job_queue_calls.saturating_add(1);
        self.job_queue = JobQueueState::Created;
        Ok(())
    }

    pub fn join_job_queue(&mut self) -> Result<(), CellError> {
        self.join_job_queue_calls = self.join_job_queue_calls.saturating_add(1);
        self.job_queue = JobQueueState::Joined;
        Ok(())
    }

    // -- Push variants (10) ----------------------------------------------

    pub fn push_job_list_body(&mut self) -> Result<(), CellError> {
        self.push_job_list_body_calls = self.push_job_list_body_calls.saturating_add(1);
        self.pending_pushes = self.pending_pushes.saturating_add(1);
        Ok(())
    }

    pub fn push_job_body2(&mut self) -> Result<(), CellError> {
        self.push_job_body2_calls = self.push_job_body2_calls.saturating_add(1);
        self.pending_pushes = self.pending_pushes.saturating_add(1);
        Ok(())
    }

    pub fn push_job2_body(&mut self) -> Result<(), CellError> {
        self.push_job2_body_calls = self.push_job2_body_calls.saturating_add(1);
        self.pending_pushes = self.pending_pushes.saturating_add(1);
        Ok(())
    }

    pub fn push_and_release_job_body(&mut self) -> Result<(), CellError> {
        self.push_and_release_job_body_calls =
            self.push_and_release_job_body_calls.saturating_add(1);
        self.pending_pushes = self.pending_pushes.saturating_add(1);
        Ok(())
    }

    pub fn push_job_body(&mut self) -> Result<(), CellError> {
        self.push_job_body_calls = self.push_job_body_calls.saturating_add(1);
        self.pending_pushes = self.pending_pushes.saturating_add(1);
        Ok(())
    }

    pub fn push_body(&mut self) -> Result<(), CellError> {
        self.push_body_calls = self.push_body_calls.saturating_add(1);
        self.pending_pushes = self.pending_pushes.saturating_add(1);
        Ok(())
    }

    pub fn allocate_job_descriptor_body(&mut self) -> Result<(), CellError> {
        self.allocate_job_descriptor_body_calls =
            self.allocate_job_descriptor_body_calls.saturating_add(1);
        Ok(())
    }

    pub fn push_sync(&mut self) -> Result<(), CellError> {
        self.push_sync_calls = self.push_sync_calls.saturating_add(1);
        self.pending_pushes = 0;
        Ok(())
    }

    pub fn push_flush(&mut self) -> Result<(), CellError> {
        self.push_flush_calls = self.push_flush_calls.saturating_add(1);
        self.pending_pushes = 0;
        Ok(())
    }

    // -- Getters ---------------------------------------------------------

    pub fn get_spurs(&mut self) -> Result<(), CellError> {
        self.get_spurs_calls = self.get_spurs_calls.saturating_add(1);
        Ok(())
    }

    pub fn get_handle_count(&mut self) -> Result<(), CellError> {
        self.get_handle_count_calls = self.get_handle_count_calls.saturating_add(1);
        Ok(())
    }

    pub fn get_error(&mut self) -> Result<(), CellError> {
        self.get_error_calls = self.get_error_calls.saturating_add(1);
        Ok(())
    }

    pub fn get_max_size_job_descriptor(&mut self) -> Result<(), CellError> {
        self.get_max_size_job_descriptor_calls =
            self.get_max_size_job_descriptor_calls.saturating_add(1);
        Ok(())
    }

    pub fn get_job_queue_id(&mut self) -> Result<(), CellError> {
        self.get_job_queue_id_calls = self.get_job_queue_id_calls.saturating_add(1);
        Ok(())
    }

    pub fn get_suspended_job_size(&mut self) -> Result<(), CellError> {
        self.get_suspended_job_size_calls =
            self.get_suspended_job_size_calls.saturating_add(1);
        Ok(())
    }

    pub fn close(&mut self) -> Result<(), CellError> {
        self.close_calls = self.close_calls.saturating_add(1);
        if matches!(self.job_queue, JobQueueState::Open) {
            self.job_queue = JobQueueState::Created;
        }
        Ok(())
    }

    pub fn open(&mut self) -> Result<(), CellError> {
        self.open_calls = self.open_calls.saturating_add(1);
        self.job_queue = JobQueueState::Open;
        Ok(())
    }

    // -- Semaphore -------------------------------------------------------

    pub fn semaphore_try_acquire(&mut self) -> Result<(), CellError> {
        self.semaphore_try_acquire_calls = self.semaphore_try_acquire_calls.saturating_add(1);
        Ok(())
    }

    pub fn semaphore_acquire(&mut self) -> Result<(), CellError> {
        self.semaphore_acquire_calls = self.semaphore_acquire_calls.saturating_add(1);
        Ok(())
    }

    pub fn semaphore_initialize(&mut self) -> Result<(), CellError> {
        self.semaphore_initialize_calls = self.semaphore_initialize_calls.saturating_add(1);
        self.semaphore_initialized = true;
        Ok(())
    }

    pub fn send_signal(&mut self) -> Result<(), CellError> {
        self.send_signal_calls = self.send_signal_calls.saturating_add(1);
        Ok(())
    }

    // -- Port ------------------------------------------------------------

    pub fn port_get_job_queue(&mut self) -> Result<(), CellError> {
        self.port_get_job_queue_calls = self.port_get_job_queue_calls.saturating_add(1);
        Ok(())
    }

    pub fn port_push_sync(&mut self) -> Result<(), CellError> {
        self.port_push_sync_calls = self.port_push_sync_calls.saturating_add(1);
        Ok(())
    }

    pub fn port_push_flush(&mut self) -> Result<(), CellError> {
        self.port_push_flush_calls = self.port_push_flush_calls.saturating_add(1);
        Ok(())
    }

    pub fn port_push_job_list_body(&mut self) -> Result<(), CellError> {
        self.port_push_job_list_body_calls =
            self.port_push_job_list_body_calls.saturating_add(1);
        Ok(())
    }

    pub fn port_push_job_body(&mut self) -> Result<(), CellError> {
        self.port_push_job_body_calls = self.port_push_job_body_calls.saturating_add(1);
        Ok(())
    }

    pub fn port_push_job_body2(&mut self) -> Result<(), CellError> {
        self.port_push_job_body2_calls = self.port_push_job_body2_calls.saturating_add(1);
        Ok(())
    }

    pub fn port_push_body(&mut self) -> Result<(), CellError> {
        self.port_push_body_calls = self.port_push_body_calls.saturating_add(1);
        Ok(())
    }

    pub fn port_try_sync(&mut self) -> Result<(), CellError> {
        self.port_try_sync_calls = self.port_try_sync_calls.saturating_add(1);
        Ok(())
    }

    pub fn port_sync(&mut self) -> Result<(), CellError> {
        self.port_sync_calls = self.port_sync_calls.saturating_add(1);
        Ok(())
    }

    pub fn port_initialize(&mut self) -> Result<(), CellError> {
        self.port_initialize_calls = self.port_initialize_calls.saturating_add(1);
        self.port = PortState::Initialized;
        Ok(())
    }

    pub fn port_initialize_with_descriptor_buffer(&mut self) -> Result<(), CellError> {
        self.port_initialize_with_descriptor_buffer_calls = self
            .port_initialize_with_descriptor_buffer_calls
            .saturating_add(1);
        self.port = PortState::Initialized;
        Ok(())
    }

    pub fn port_finalize(&mut self) -> Result<(), CellError> {
        self.port_finalize_calls = self.port_finalize_calls.saturating_add(1);
        self.port = PortState::Finalized;
        Ok(())
    }

    pub fn port_copy_push_job_body(&mut self) -> Result<(), CellError> {
        self.port_copy_push_job_body_calls =
            self.port_copy_push_job_body_calls.saturating_add(1);
        Ok(())
    }

    pub fn port_copy_push_job_body2(&mut self) -> Result<(), CellError> {
        self.port_copy_push_job_body2_calls =
            self.port_copy_push_job_body2_calls.saturating_add(1);
        Ok(())
    }

    pub fn port_copy_push_body(&mut self) -> Result<(), CellError> {
        self.port_copy_push_body_calls = self.port_copy_push_body_calls.saturating_add(1);
        Ok(())
    }

    // -- Port2 -----------------------------------------------------------

    pub fn port2_get_job_queue(&mut self) -> Result<(), CellError> {
        self.port2_get_job_queue_calls = self.port2_get_job_queue_calls.saturating_add(1);
        Ok(())
    }

    pub fn port2_push_sync(&mut self) -> Result<(), CellError> {
        self.port2_push_sync_calls = self.port2_push_sync_calls.saturating_add(1);
        Ok(())
    }

    pub fn port2_push_flush(&mut self) -> Result<(), CellError> {
        self.port2_push_flush_calls = self.port2_push_flush_calls.saturating_add(1);
        Ok(())
    }

    pub fn port2_push_job_list_body(&mut self) -> Result<(), CellError> {
        self.port2_push_job_list_body_calls =
            self.port2_push_job_list_body_calls.saturating_add(1);
        Ok(())
    }

    pub fn port2_sync(&mut self) -> Result<(), CellError> {
        self.port2_sync_calls = self.port2_sync_calls.saturating_add(1);
        Ok(())
    }

    pub fn port2_create(&mut self) -> Result<(), CellError> {
        self.port2_create_calls = self.port2_create_calls.saturating_add(1);
        self.port2 = PortState::Initialized;
        Ok(())
    }

    pub fn port2_destroy(&mut self) -> Result<(), CellError> {
        self.port2_destroy_calls = self.port2_destroy_calls.saturating_add(1);
        self.port2 = PortState::Finalized;
        Ok(())
    }

    pub fn port2_allocate_job_descriptor(&mut self) -> Result<(), CellError> {
        self.port2_allocate_job_descriptor_calls =
            self.port2_allocate_job_descriptor_calls.saturating_add(1);
        Ok(())
    }

    pub fn port2_push_and_release_job_body(&mut self) -> Result<(), CellError> {
        self.port2_push_and_release_job_body_calls = self
            .port2_push_and_release_job_body_calls
            .saturating_add(1);
        Ok(())
    }

    pub fn port2_copy_push_job_body(&mut self) -> Result<(), CellError> {
        self.port2_copy_push_job_body_calls =
            self.port2_copy_push_job_body_calls.saturating_add(1);
        Ok(())
    }

    pub fn port2_push_job_body(&mut self) -> Result<(), CellError> {
        self.port2_push_job_body_calls = self.port2_push_job_body_calls.saturating_add(1);
        Ok(())
    }

    // -- Exception handlers ----------------------------------------------

    pub fn set_exception_event_handler(&mut self) -> Result<(), CellError> {
        self.set_exception_event_handler_calls =
            self.set_exception_event_handler_calls.saturating_add(1);
        self.exception_handler_set = true;
        Ok(())
    }

    pub fn set_exception_event_handler2(&mut self) -> Result<(), CellError> {
        self.set_exception_event_handler2_calls =
            self.set_exception_event_handler2_calls.saturating_add(1);
        self.exception_handler_set = true;
        Ok(())
    }

    pub fn unset_exception_event_handler(&mut self) -> Result<(), CellError> {
        self.unset_exception_event_handler_calls =
            self.unset_exception_event_handler_calls.saturating_add(1);
        self.exception_handler_set = false;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_and_entry_count() {
        assert_eq!(MODULE_NAME, "cellSpursJq");
        assert_eq!(REGISTERED_ENTRY_POINTS.len(), 63);
        assert_eq!(REGISTERED_ENTRY_POINTS[0], "cellSpursJobQueueAttributeInitialize");
        assert_eq!(REGISTERED_ENTRY_POINTS[10], "_cellSpursCreateJobQueueWithJobDescriptorPool");
        assert_eq!(REGISTERED_ENTRY_POINTS[34], "cellSpursJobQueuePortGetJobQueue");
        assert_eq!(REGISTERED_ENTRY_POINTS[49], "cellSpursJobQueuePort2GetJobQueue");
        assert_eq!(REGISTERED_ENTRY_POINTS[62], "cellSpursJobQueueUnsetExceptionEventHandler");
    }

    #[test]
    fn placeholder_error_codes_byte_exact() {
        assert_eq!(CELL_SPURS_JQ_ERROR_NULL_POINTER.0, 0x8061_5B01);
        assert_eq!(CELL_SPURS_JQ_ERROR_NOT_INITIALIZED.0, 0x8061_5B02);
        assert_eq!(CELL_SPURS_JQ_ERROR_ALREADY_INITIALIZED.0, 0x8061_5B03);
        assert_eq!(CELL_SPURS_JQ_ERROR_BUSY.0, 0x8061_5B04);
        assert_eq!(CELL_SPURS_JQ_ERROR_JOIN.0, 0x8061_5B05);
        assert_eq!(CELL_SPURS_JQ_ERROR_FULL.0, 0x8061_5B06);
        assert_eq!(CELL_SPURS_JQ_ERROR_EMPTY.0, 0x8061_5B07);
    }

    #[test]
    fn default_state() {
        let m = SpursJq::new();
        assert_eq!(m.job_queue, JobQueueState::Uninit);
        assert_eq!(m.port, PortState::Uninit);
        assert_eq!(m.port2, PortState::Uninit);
        assert!(!m.attribute_initialized);
        assert!(!m.semaphore_initialized);
        assert!(!m.exception_handler_set);
        assert_eq!(m.pending_pushes, 0);
    }

    #[test]
    fn attribute_initialize_sets_flag() {
        let mut m = SpursJq::new();
        m.attribute_initialize().unwrap();
        assert!(m.attribute_initialized);
    }

    #[test]
    fn attribute_setters_all_tracked() {
        let mut m = SpursJq::new();
        m.attribute_set_max_grab().unwrap();
        m.attribute_set_submit_with_entry_lock().unwrap();
        m.attribute_set_do_busy_waiting().unwrap();
        m.attribute_set_is_halt_on_error().unwrap();
        m.attribute_set_is_job_type_memory_check().unwrap();
        m.attribute_set_max_size_job_descriptor().unwrap();
        m.attribute_set_grab_parameters().unwrap();
        m.set_waiting_mode().unwrap();
        assert_eq!(m.attribute_set_max_grab_calls, 1);
        assert_eq!(m.attribute_set_submit_with_entry_lock_calls, 1);
        assert_eq!(m.attribute_set_do_busy_waiting_calls, 1);
        assert_eq!(m.attribute_set_is_halt_on_error_calls, 1);
        assert_eq!(m.attribute_set_is_job_type_memory_check_calls, 1);
        assert_eq!(m.attribute_set_max_size_job_descriptor_calls, 1);
        assert_eq!(m.attribute_set_grab_parameters_calls, 1);
        assert_eq!(m.set_waiting_mode_calls, 1);
    }

    #[test]
    fn create_transitions_to_created() {
        let mut m = SpursJq::new();
        m.create_job_queue().unwrap();
        assert_eq!(m.job_queue, JobQueueState::Created);
    }

    #[test]
    fn create_with_pool_also_sets_created() {
        let mut m = SpursJq::new();
        m.create_job_queue_with_pool().unwrap();
        assert_eq!(m.job_queue, JobQueueState::Created);
    }

    #[test]
    fn open_close_toggles_created_open() {
        let mut m = SpursJq::new();
        m.create_job_queue().unwrap();
        m.open().unwrap();
        assert_eq!(m.job_queue, JobQueueState::Open);
        m.close().unwrap();
        assert_eq!(m.job_queue, JobQueueState::Created);
    }

    #[test]
    fn close_when_not_open_is_noop() {
        let mut m = SpursJq::new();
        m.close().unwrap();
        assert_eq!(m.job_queue, JobQueueState::Uninit);
    }

    #[test]
    fn join_transitions_to_joined() {
        let mut m = SpursJq::new();
        m.create_job_queue().unwrap();
        m.join_job_queue().unwrap();
        assert_eq!(m.job_queue, JobQueueState::Joined);
    }

    #[test]
    fn shutdown_transitions_to_shutdown() {
        let mut m = SpursJq::new();
        m.create_job_queue().unwrap();
        m.shutdown_job_queue().unwrap();
        assert_eq!(m.job_queue, JobQueueState::Shutdown);
    }

    #[test]
    fn push_variants_increment_pending() {
        let mut m = SpursJq::new();
        m.push_job_body().unwrap();
        m.push_job_body2().unwrap();
        m.push_job2_body().unwrap();
        m.push_body().unwrap();
        m.push_job_list_body().unwrap();
        m.push_and_release_job_body().unwrap();
        assert_eq!(m.pending_pushes, 6);
        // Sync and Flush both drain.
        m.push_sync().unwrap();
        assert_eq!(m.pending_pushes, 0);
        m.push_job_body().unwrap();
        m.push_flush().unwrap();
        assert_eq!(m.pending_pushes, 0);
    }

    #[test]
    fn allocate_descriptor_does_not_push() {
        let mut m = SpursJq::new();
        m.allocate_job_descriptor_body().unwrap();
        assert_eq!(m.pending_pushes, 0);
    }

    #[test]
    fn semaphore_initialize_flag() {
        let mut m = SpursJq::new();
        m.semaphore_initialize().unwrap();
        assert!(m.semaphore_initialized);
        m.semaphore_try_acquire().unwrap();
        m.semaphore_acquire().unwrap();
        m.send_signal().unwrap();
        assert_eq!(m.semaphore_try_acquire_calls, 1);
        assert_eq!(m.semaphore_acquire_calls, 1);
        assert_eq!(m.send_signal_calls, 1);
    }

    #[test]
    fn port_lifecycle() {
        let mut m = SpursJq::new();
        m.port_initialize().unwrap();
        assert_eq!(m.port, PortState::Initialized);
        m.port_finalize().unwrap();
        assert_eq!(m.port, PortState::Finalized);
    }

    #[test]
    fn port_initialize_with_descriptor_buffer_also_sets_initialized() {
        let mut m = SpursJq::new();
        m.port_initialize_with_descriptor_buffer().unwrap();
        assert_eq!(m.port, PortState::Initialized);
    }

    #[test]
    fn port2_lifecycle() {
        let mut m = SpursJq::new();
        m.port2_create().unwrap();
        assert_eq!(m.port2, PortState::Initialized);
        m.port2_destroy().unwrap();
        assert_eq!(m.port2, PortState::Finalized);
    }

    #[test]
    fn port_and_port2_independent() {
        let mut m = SpursJq::new();
        m.port_initialize().unwrap();
        assert_eq!(m.port, PortState::Initialized);
        assert_eq!(m.port2, PortState::Uninit);
        m.port2_create().unwrap();
        m.port_finalize().unwrap();
        assert_eq!(m.port, PortState::Finalized);
        assert_eq!(m.port2, PortState::Initialized);
    }

    #[test]
    fn port_push_entries_tracked() {
        let mut m = SpursJq::new();
        m.port_push_sync().unwrap();
        m.port_push_flush().unwrap();
        m.port_push_job_list_body().unwrap();
        m.port_push_job_body().unwrap();
        m.port_push_job_body2().unwrap();
        m.port_push_body().unwrap();
        m.port_copy_push_job_body().unwrap();
        m.port_copy_push_job_body2().unwrap();
        m.port_copy_push_body().unwrap();
        m.port_try_sync().unwrap();
        m.port_sync().unwrap();
        m.port_get_job_queue().unwrap();
        assert_eq!(m.port_push_sync_calls, 1);
        assert_eq!(m.port_push_flush_calls, 1);
        assert_eq!(m.port_copy_push_job_body_calls, 1);
        assert_eq!(m.port_copy_push_body_calls, 1);
        assert_eq!(m.port_get_job_queue_calls, 1);
    }

    #[test]
    fn port2_entries_tracked() {
        let mut m = SpursJq::new();
        m.port2_get_job_queue().unwrap();
        m.port2_push_sync().unwrap();
        m.port2_push_flush().unwrap();
        m.port2_push_job_list_body().unwrap();
        m.port2_sync().unwrap();
        m.port2_allocate_job_descriptor().unwrap();
        m.port2_push_and_release_job_body().unwrap();
        m.port2_copy_push_job_body().unwrap();
        m.port2_push_job_body().unwrap();
        assert_eq!(m.port2_push_sync_calls, 1);
        assert_eq!(m.port2_sync_calls, 1);
        assert_eq!(m.port2_allocate_job_descriptor_calls, 1);
    }

    #[test]
    fn exception_handler_set_unset() {
        let mut m = SpursJq::new();
        m.set_exception_event_handler().unwrap();
        assert!(m.exception_handler_set);
        m.unset_exception_event_handler().unwrap();
        assert!(!m.exception_handler_set);
        m.set_exception_event_handler2().unwrap();
        assert!(m.exception_handler_set);
    }

    #[test]
    fn getters_all_tracked() {
        let mut m = SpursJq::new();
        m.get_spurs().unwrap();
        m.get_handle_count().unwrap();
        m.get_error().unwrap();
        m.get_max_size_job_descriptor().unwrap();
        m.get_job_queue_id().unwrap();
        m.get_suspended_job_size().unwrap();
        assert_eq!(m.get_spurs_calls, 1);
        assert_eq!(m.get_handle_count_calls, 1);
        assert_eq!(m.get_error_calls, 1);
        assert_eq!(m.get_max_size_job_descriptor_calls, 1);
        assert_eq!(m.get_job_queue_id_calls, 1);
        assert_eq!(m.get_suspended_job_size_calls, 1);
    }

    #[test]
    fn full_spurs_jq_lifecycle_smoke() {
        let mut m = SpursJq::new();

        // Attribute config
        m.attribute_initialize().unwrap();
        m.attribute_set_max_grab().unwrap();
        m.attribute_set_do_busy_waiting().unwrap();

        // Create job queue
        m.create_job_queue().unwrap();
        m.open().unwrap();

        // Port init + push
        m.port_initialize().unwrap();
        m.port_push_job_body().unwrap();
        m.port_push_job_body2().unwrap();
        m.port_sync().unwrap();

        // Port2 variant
        m.port2_create().unwrap();
        m.port2_push_job_body().unwrap();
        m.port2_sync().unwrap();

        // Semaphore + exception handler
        m.semaphore_initialize().unwrap();
        m.send_signal().unwrap();
        m.set_exception_event_handler().unwrap();

        // Push jobs directly
        m.push_job_body().unwrap();
        m.push_job_body().unwrap();
        m.push_sync().unwrap();
        assert_eq!(m.pending_pushes, 0);

        // Tear down
        m.unset_exception_event_handler().unwrap();
        m.port2_destroy().unwrap();
        m.port_finalize().unwrap();
        m.close().unwrap();
        m.join_job_queue().unwrap();
        m.shutdown_job_queue().unwrap();

        assert_eq!(m.job_queue, JobQueueState::Shutdown);
        assert_eq!(m.port, PortState::Finalized);
        assert_eq!(m.port2, PortState::Finalized);
    }
}
