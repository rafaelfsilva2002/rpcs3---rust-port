//! Rust port of `rpcs3/Emu/Cell/Modules/cellUsbpspcm.cpp` — PS3 USB PSP
//! communication module (PSPCM) HLE surface.
//!
//! Upstream registers `cellUsbPspcm` with 27 `UNIMPLEMENTED_FUNC` stubs and a
//! 12-entry error enum in the `0x8011_040_` facility. C++ has no observable
//! behaviour beyond always returning `CELL_OK`, but the error codes expose
//! the real lifecycle: init/end, register/unregister, bind (sync / async with
//! wait / poll / cancel), send / recv (sync / async), reset (sync / async),
//! and data-wait (sync / async with cancel).
//!
//! This crate is a behaviour-preserving expansion: it mirrors byte-exact
//! error codes, assembles an inferred FSM from the error-code vocabulary, and
//! exposes enough state (async slots, bind handles, registration table) for
//! integration tests and future real implementations to plug into. All entry
//! points are Rust-side methods returning `Result<(), CellError>`.
//!
//! `no_std` + `alloc`. Single dep: `rpcs3-emu-types`.

#![no_std]

extern crate alloc;

use alloc::vec::Vec;
use rpcs3_emu_types::CellError;

/// Upstream PRX name registered by `DECLARE(ppu_module_manager::cellUsbPspcm)`.
pub const MODULE_NAME: &str = "cellUsbPspcm";

/// 27 FNIDs registered in the exact `REG_FUNC` order for dispatch assertions.
pub const REGISTERED_ENTRY_POINTS: &[&str] = &[
    "cellUsbPspcmInit",
    "cellUsbPspcmEnd",
    "cellUsbPspcmCalcPoolSize",
    "cellUsbPspcmRegister",
    "cellUsbPspcmUnregister",
    "cellUsbPspcmGetAddr",
    "cellUsbPspcmBind",
    "cellUsbPspcmBindAsync",
    "cellUsbPspcmWaitBindAsync",
    "cellUsbPspcmPollBindAsync",
    "cellUsbPspcmCancelBind",
    "cellUsbPspcmClose",
    "cellUsbPspcmSend",
    "cellUsbPspcmSendAsync",
    "cellUsbPspcmWaitSendAsync",
    "cellUsbPspcmPollSendAsync",
    "cellUsbPspcmRecv",
    "cellUsbPspcmRecvAsync",
    "cellUsbPspcmWaitRecvAsync",
    "cellUsbPspcmPollRecvAsync",
    "cellUsbPspcmReset",
    "cellUsbPspcmResetAsync",
    "cellUsbPspcmWaitResetAsync",
    "cellUsbPspcmPollResetAsync",
    "cellUsbPspcmWaitData",
    "cellUsbPspcmPollData",
    "cellUsbPspcmCancelWaitData",
];

// ---------------------------------------------------------------------------
// Error codes — byte-exact `0x8011_0401..0x8011_040C`.
// ---------------------------------------------------------------------------

pub const CELL_USBPSPCM_ERROR_NOT_INITIALIZED: CellError = CellError(0x8011_0401);
pub const CELL_USBPSPCM_ERROR_ALREADY: CellError = CellError(0x8011_0402);
pub const CELL_USBPSPCM_ERROR_INVALID: CellError = CellError(0x8011_0403);
pub const CELL_USBPSPCM_ERROR_NO_MEMORY: CellError = CellError(0x8011_0404);
pub const CELL_USBPSPCM_ERROR_BUSY: CellError = CellError(0x8011_0405);
pub const CELL_USBPSPCM_ERROR_INPROGRESS: CellError = CellError(0x8011_0406);
pub const CELL_USBPSPCM_ERROR_NO_SPACE: CellError = CellError(0x8011_0407);
pub const CELL_USBPSPCM_ERROR_CANCELED: CellError = CellError(0x8011_0408);
pub const CELL_USBPSPCM_ERROR_RESETTING: CellError = CellError(0x8011_0409);
pub const CELL_USBPSPCM_ERROR_RESET_END: CellError = CellError(0x8011_040A);
pub const CELL_USBPSPCM_ERROR_CLOSED: CellError = CellError(0x8011_040B);
pub const CELL_USBPSPCM_ERROR_NO_DATA: CellError = CellError(0x8011_040C);

// ---------------------------------------------------------------------------
// FSM / model.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModuleState {
    Uninit,
    Initialized,
    Finalized,
}

impl Default for ModuleState {
    fn default() -> Self {
        ModuleState::Uninit
    }
}

/// Per-handle state. `Unbound` = just registered; `Binding` = BindAsync in
/// flight; `Bound` = ready for Send/Recv; `Resetting` = Reset in flight;
/// `Closed` = post-`Close` (terminal).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandleState {
    Unbound,
    Binding,
    Bound,
    Resetting,
    Closed,
}

/// State of a per-handle async slot (Bind / Send / Recv / Reset / WaitData).
/// `Idle` = no request pending; `Pending` = async request submitted and still
/// running; `Completed` = result available (next Poll/Wait consumes it);
/// `Canceled` = request was canceled before completion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AsyncSlotState {
    Idle,
    Pending,
    Completed,
    Canceled,
}

impl Default for AsyncSlotState {
    fn default() -> Self {
        AsyncSlotState::Idle
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct AsyncSlot {
    pub state: AsyncSlotState,
}

impl AsyncSlot {
    pub fn is_idle(self) -> bool {
        matches!(self.state, AsyncSlotState::Idle)
    }
}

/// A registered USB PSPCM endpoint / handle record. `addr` is a u32 handle
/// value the real firmware assigns; we hand out sequential values starting at
/// `HANDLE_BASE` so tests can assert deterministic IDs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Handle {
    pub id: u32,
    pub addr: u32,
    pub state: HandleState,
    pub bind_slot: AsyncSlot,
    pub send_slot: AsyncSlot,
    pub recv_slot: AsyncSlot,
    pub reset_slot: AsyncSlot,
    pub data_wait: AsyncSlot,
    pub pending_recv_bytes: u32,
}

pub const HANDLE_BASE: u32 = 0x1000_0000;
pub const MAX_HANDLES: usize = 16;

// ---------------------------------------------------------------------------
// Manager — top-level state + 27 per-entry counters.
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct UsbPspcm {
    state: ModuleState,
    handles: Vec<Handle>,
    next_handle_id: u32,
    next_addr: u32,

    pub init_calls: u64,
    pub end_calls: u64,
    pub calc_pool_size_calls: u64,
    pub register_calls: u64,
    pub unregister_calls: u64,
    pub get_addr_calls: u64,
    pub bind_calls: u64,
    pub bind_async_calls: u64,
    pub wait_bind_async_calls: u64,
    pub poll_bind_async_calls: u64,
    pub cancel_bind_calls: u64,
    pub close_calls: u64,
    pub send_calls: u64,
    pub send_async_calls: u64,
    pub wait_send_async_calls: u64,
    pub poll_send_async_calls: u64,
    pub recv_calls: u64,
    pub recv_async_calls: u64,
    pub wait_recv_async_calls: u64,
    pub poll_recv_async_calls: u64,
    pub reset_calls: u64,
    pub reset_async_calls: u64,
    pub wait_reset_async_calls: u64,
    pub poll_reset_async_calls: u64,
    pub wait_data_calls: u64,
    pub poll_data_calls: u64,
    pub cancel_wait_data_calls: u64,
}

impl UsbPspcm {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn state(&self) -> ModuleState {
        self.state
    }

    pub fn handles(&self) -> &[Handle] {
        &self.handles
    }

    pub fn handle(&self, id: u32) -> Option<&Handle> {
        self.handles.iter().find(|h| h.id == id)
    }

    fn handle_mut(&mut self, id: u32) -> Option<&mut Handle> {
        self.handles.iter_mut().find(|h| h.id == id)
    }

    fn require_initialized(&self) -> Result<(), CellError> {
        match self.state {
            ModuleState::Initialized => Ok(()),
            _ => Err(CELL_USBPSPCM_ERROR_NOT_INITIALIZED),
        }
    }

    fn require_live(&self, id: u32) -> Result<&Handle, CellError> {
        let h = self
            .handle(id)
            .ok_or(CELL_USBPSPCM_ERROR_INVALID)?;
        match h.state {
            HandleState::Closed => Err(CELL_USBPSPCM_ERROR_CLOSED),
            _ => Ok(h),
        }
    }

    /// `cellUsbPspcmInit()` — rejects double-init with `ERROR_ALREADY`.
    pub fn init(&mut self) -> Result<(), CellError> {
        self.init_calls = self.init_calls.saturating_add(1);
        match self.state {
            ModuleState::Initialized => Err(CELL_USBPSPCM_ERROR_ALREADY),
            _ => {
                self.state = ModuleState::Initialized;
                Ok(())
            }
        }
    }

    /// `cellUsbPspcmEnd()` — `NOT_INITIALIZED` when not live; clears handles.
    pub fn end(&mut self) -> Result<(), CellError> {
        self.end_calls = self.end_calls.saturating_add(1);
        self.require_initialized()?;
        self.state = ModuleState::Finalized;
        self.handles.clear();
        Ok(())
    }

    /// `cellUsbPspcmCalcPoolSize()` — returns the pool size the caller must
    /// allocate. Upstream is a stub, so we return a deterministic derived size
    /// from `handle_count` to let tests pin the formula.
    pub fn calc_pool_size(&mut self, handle_count: u32) -> Result<u32, CellError> {
        self.calc_pool_size_calls = self.calc_pool_size_calls.saturating_add(1);
        if handle_count == 0 || handle_count as usize > MAX_HANDLES {
            return Err(CELL_USBPSPCM_ERROR_INVALID);
        }
        // Arbitrary-but-stable formula: 0x200 bytes per handle plus fixed hdr.
        let size = handle_count.checked_mul(0x200).ok_or(CELL_USBPSPCM_ERROR_NO_MEMORY)?;
        Ok(size.checked_add(0x40).ok_or(CELL_USBPSPCM_ERROR_NO_MEMORY)?)
    }

    /// `cellUsbPspcmRegister()` — allocates a new handle in `Unbound` state.
    pub fn register(&mut self) -> Result<u32, CellError> {
        self.register_calls = self.register_calls.saturating_add(1);
        self.require_initialized()?;
        if self.handles.len() >= MAX_HANDLES {
            return Err(CELL_USBPSPCM_ERROR_NO_SPACE);
        }
        let id = HANDLE_BASE.wrapping_add(self.next_handle_id);
        let addr = 0xA000_0000u32.wrapping_add(self.next_addr);
        self.next_handle_id = self.next_handle_id.saturating_add(1);
        self.next_addr = self.next_addr.saturating_add(0x1000);
        self.handles.push(Handle {
            id,
            addr,
            state: HandleState::Unbound,
            bind_slot: AsyncSlot::default(),
            send_slot: AsyncSlot::default(),
            recv_slot: AsyncSlot::default(),
            reset_slot: AsyncSlot::default(),
            data_wait: AsyncSlot::default(),
            pending_recv_bytes: 0,
        });
        Ok(id)
    }

    /// `cellUsbPspcmUnregister()` — removes an idle handle. Rejects if any
    /// async slot is pending (`BUSY`) or if it is currently resetting.
    pub fn unregister(&mut self, id: u32) -> Result<(), CellError> {
        self.unregister_calls = self.unregister_calls.saturating_add(1);
        self.require_initialized()?;
        let h = self.handle(id).ok_or(CELL_USBPSPCM_ERROR_INVALID)?;
        if matches!(h.state, HandleState::Resetting) {
            return Err(CELL_USBPSPCM_ERROR_RESETTING);
        }
        if !(h.bind_slot.is_idle()
            && h.send_slot.is_idle()
            && h.recv_slot.is_idle()
            && h.reset_slot.is_idle()
            && h.data_wait.is_idle())
        {
            return Err(CELL_USBPSPCM_ERROR_BUSY);
        }
        self.handles.retain(|hh| hh.id != id);
        Ok(())
    }

    /// `cellUsbPspcmGetAddr()` — returns the address the firmware assigned.
    pub fn get_addr(&mut self, id: u32) -> Result<u32, CellError> {
        self.get_addr_calls = self.get_addr_calls.saturating_add(1);
        self.require_initialized()?;
        let h = self.require_live(id)?;
        Ok(h.addr)
    }

    /// `cellUsbPspcmBind()` (synchronous) — transitions `Unbound → Bound`.
    pub fn bind(&mut self, id: u32) -> Result<(), CellError> {
        self.bind_calls = self.bind_calls.saturating_add(1);
        self.require_initialized()?;
        let h = self.require_live_mut(id)?;
        match h.state {
            HandleState::Unbound => {
                if !h.bind_slot.is_idle() {
                    return Err(CELL_USBPSPCM_ERROR_INPROGRESS);
                }
                h.state = HandleState::Bound;
                Ok(())
            }
            HandleState::Bound => Err(CELL_USBPSPCM_ERROR_ALREADY),
            HandleState::Binding => Err(CELL_USBPSPCM_ERROR_INPROGRESS),
            HandleState::Resetting => Err(CELL_USBPSPCM_ERROR_RESETTING),
            HandleState::Closed => Err(CELL_USBPSPCM_ERROR_CLOSED),
        }
    }

    /// `cellUsbPspcmBindAsync()` — starts async bind, slot → Pending.
    pub fn bind_async(&mut self, id: u32) -> Result<(), CellError> {
        self.bind_async_calls = self.bind_async_calls.saturating_add(1);
        self.require_initialized()?;
        let h = self.require_live_mut(id)?;
        match h.state {
            HandleState::Unbound => {
                if !h.bind_slot.is_idle() {
                    return Err(CELL_USBPSPCM_ERROR_INPROGRESS);
                }
                h.state = HandleState::Binding;
                h.bind_slot.state = AsyncSlotState::Pending;
                Ok(())
            }
            HandleState::Bound => Err(CELL_USBPSPCM_ERROR_ALREADY),
            HandleState::Binding => Err(CELL_USBPSPCM_ERROR_INPROGRESS),
            HandleState::Resetting => Err(CELL_USBPSPCM_ERROR_RESETTING),
            HandleState::Closed => Err(CELL_USBPSPCM_ERROR_CLOSED),
        }
    }

    /// `cellUsbPspcmWaitBindAsync()` — blocks until Completed/Canceled.
    pub fn wait_bind_async(&mut self, id: u32) -> Result<(), CellError> {
        self.wait_bind_async_calls = self.wait_bind_async_calls.saturating_add(1);
        self.require_initialized()?;
        self.consume_slot(id, SlotKind::Bind, /*block*/ true)
    }

    /// `cellUsbPspcmPollBindAsync()` — non-blocking; returns INPROGRESS if still Pending.
    pub fn poll_bind_async(&mut self, id: u32) -> Result<(), CellError> {
        self.poll_bind_async_calls = self.poll_bind_async_calls.saturating_add(1);
        self.require_initialized()?;
        self.consume_slot(id, SlotKind::Bind, /*block*/ false)
    }

    /// `cellUsbPspcmCancelBind()` — cancels an in-flight async bind.
    pub fn cancel_bind(&mut self, id: u32) -> Result<(), CellError> {
        self.cancel_bind_calls = self.cancel_bind_calls.saturating_add(1);
        self.require_initialized()?;
        let h = self.require_live_mut(id)?;
        match h.bind_slot.state {
            AsyncSlotState::Pending => {
                h.bind_slot.state = AsyncSlotState::Canceled;
                // After cancel, per-error-code semantics: NOT_INITIALIZED is
                // preserved only at the module level, so we move state back to
                // Unbound (the bind never completed).
                if matches!(h.state, HandleState::Binding) {
                    h.state = HandleState::Unbound;
                }
                Ok(())
            }
            AsyncSlotState::Idle | AsyncSlotState::Completed | AsyncSlotState::Canceled => {
                Err(CELL_USBPSPCM_ERROR_INVALID)
            }
        }
    }

    /// `cellUsbPspcmClose()` — terminal. Rejects if async bind/send/recv live.
    pub fn close(&mut self, id: u32) -> Result<(), CellError> {
        self.close_calls = self.close_calls.saturating_add(1);
        self.require_initialized()?;
        let h = self.require_live_mut(id)?;
        if matches!(h.state, HandleState::Resetting) {
            return Err(CELL_USBPSPCM_ERROR_RESETTING);
        }
        if !(h.bind_slot.is_idle()
            && h.send_slot.is_idle()
            && h.recv_slot.is_idle()
            && h.data_wait.is_idle())
        {
            return Err(CELL_USBPSPCM_ERROR_BUSY);
        }
        h.state = HandleState::Closed;
        Ok(())
    }

    /// `cellUsbPspcmSend()` — synchronous; requires Bound, rejects Reset/Closed.
    pub fn send(&mut self, id: u32, len: u32) -> Result<(), CellError> {
        self.send_calls = self.send_calls.saturating_add(1);
        self.require_initialized()?;
        let h = self.require_live_mut(id)?;
        if matches!(h.state, HandleState::Resetting) {
            return Err(CELL_USBPSPCM_ERROR_RESETTING);
        }
        if !matches!(h.state, HandleState::Bound) {
            return Err(CELL_USBPSPCM_ERROR_INVALID);
        }
        if !h.send_slot.is_idle() {
            return Err(CELL_USBPSPCM_ERROR_INPROGRESS);
        }
        if len == 0 {
            return Err(CELL_USBPSPCM_ERROR_INVALID);
        }
        Ok(())
    }

    /// `cellUsbPspcmSendAsync()`.
    pub fn send_async(&mut self, id: u32, len: u32) -> Result<(), CellError> {
        self.send_async_calls = self.send_async_calls.saturating_add(1);
        self.require_initialized()?;
        let h = self.require_live_mut(id)?;
        if matches!(h.state, HandleState::Resetting) {
            return Err(CELL_USBPSPCM_ERROR_RESETTING);
        }
        if !matches!(h.state, HandleState::Bound) {
            return Err(CELL_USBPSPCM_ERROR_INVALID);
        }
        if len == 0 {
            return Err(CELL_USBPSPCM_ERROR_INVALID);
        }
        if !h.send_slot.is_idle() {
            return Err(CELL_USBPSPCM_ERROR_INPROGRESS);
        }
        h.send_slot.state = AsyncSlotState::Pending;
        Ok(())
    }

    pub fn wait_send_async(&mut self, id: u32) -> Result<(), CellError> {
        self.wait_send_async_calls = self.wait_send_async_calls.saturating_add(1);
        self.require_initialized()?;
        self.consume_slot(id, SlotKind::Send, true)
    }

    pub fn poll_send_async(&mut self, id: u32) -> Result<(), CellError> {
        self.poll_send_async_calls = self.poll_send_async_calls.saturating_add(1);
        self.require_initialized()?;
        self.consume_slot(id, SlotKind::Send, false)
    }

    /// `cellUsbPspcmRecv()` — requires Bound and some pending_recv_bytes, else NO_DATA.
    pub fn recv(&mut self, id: u32, max_len: u32) -> Result<u32, CellError> {
        self.recv_calls = self.recv_calls.saturating_add(1);
        self.require_initialized()?;
        let h = self.require_live_mut(id)?;
        if matches!(h.state, HandleState::Resetting) {
            return Err(CELL_USBPSPCM_ERROR_RESETTING);
        }
        if !matches!(h.state, HandleState::Bound) {
            return Err(CELL_USBPSPCM_ERROR_INVALID);
        }
        if max_len == 0 {
            return Err(CELL_USBPSPCM_ERROR_INVALID);
        }
        if !h.recv_slot.is_idle() {
            return Err(CELL_USBPSPCM_ERROR_INPROGRESS);
        }
        if h.pending_recv_bytes == 0 {
            return Err(CELL_USBPSPCM_ERROR_NO_DATA);
        }
        let take = core::cmp::min(h.pending_recv_bytes, max_len);
        h.pending_recv_bytes = h.pending_recv_bytes.saturating_sub(take);
        Ok(take)
    }

    pub fn recv_async(&mut self, id: u32, max_len: u32) -> Result<(), CellError> {
        self.recv_async_calls = self.recv_async_calls.saturating_add(1);
        self.require_initialized()?;
        let h = self.require_live_mut(id)?;
        if matches!(h.state, HandleState::Resetting) {
            return Err(CELL_USBPSPCM_ERROR_RESETTING);
        }
        if !matches!(h.state, HandleState::Bound) {
            return Err(CELL_USBPSPCM_ERROR_INVALID);
        }
        if max_len == 0 {
            return Err(CELL_USBPSPCM_ERROR_INVALID);
        }
        if !h.recv_slot.is_idle() {
            return Err(CELL_USBPSPCM_ERROR_INPROGRESS);
        }
        h.recv_slot.state = AsyncSlotState::Pending;
        Ok(())
    }

    pub fn wait_recv_async(&mut self, id: u32) -> Result<(), CellError> {
        self.wait_recv_async_calls = self.wait_recv_async_calls.saturating_add(1);
        self.require_initialized()?;
        self.consume_slot(id, SlotKind::Recv, true)
    }

    pub fn poll_recv_async(&mut self, id: u32) -> Result<(), CellError> {
        self.poll_recv_async_calls = self.poll_recv_async_calls.saturating_add(1);
        self.require_initialized()?;
        self.consume_slot(id, SlotKind::Recv, false)
    }

    /// `cellUsbPspcmReset()` — synchronous reset → state back to Unbound.
    pub fn reset(&mut self, id: u32) -> Result<(), CellError> {
        self.reset_calls = self.reset_calls.saturating_add(1);
        self.require_initialized()?;
        let h = self.require_live_mut(id)?;
        match h.state {
            HandleState::Resetting => Err(CELL_USBPSPCM_ERROR_RESETTING),
            HandleState::Closed => Err(CELL_USBPSPCM_ERROR_CLOSED),
            _ => {
                if !h.reset_slot.is_idle() {
                    return Err(CELL_USBPSPCM_ERROR_INPROGRESS);
                }
                h.state = HandleState::Unbound;
                h.pending_recv_bytes = 0;
                Ok(())
            }
        }
    }

    pub fn reset_async(&mut self, id: u32) -> Result<(), CellError> {
        self.reset_async_calls = self.reset_async_calls.saturating_add(1);
        self.require_initialized()?;
        let h = self.require_live_mut(id)?;
        match h.state {
            HandleState::Resetting => Err(CELL_USBPSPCM_ERROR_RESETTING),
            HandleState::Closed => Err(CELL_USBPSPCM_ERROR_CLOSED),
            _ => {
                if !h.reset_slot.is_idle() {
                    return Err(CELL_USBPSPCM_ERROR_INPROGRESS);
                }
                h.state = HandleState::Resetting;
                h.reset_slot.state = AsyncSlotState::Pending;
                Ok(())
            }
        }
    }

    pub fn wait_reset_async(&mut self, id: u32) -> Result<(), CellError> {
        self.wait_reset_async_calls = self.wait_reset_async_calls.saturating_add(1);
        self.require_initialized()?;
        self.consume_slot(id, SlotKind::Reset, true)
    }

    pub fn poll_reset_async(&mut self, id: u32) -> Result<(), CellError> {
        self.poll_reset_async_calls = self.poll_reset_async_calls.saturating_add(1);
        self.require_initialized()?;
        self.consume_slot(id, SlotKind::Reset, false)
    }

    /// `cellUsbPspcmWaitData()` — blocks until pending_recv_bytes > 0.
    /// Slot is marked Pending so `CancelWaitData` can short-circuit it.
    pub fn wait_data(&mut self, id: u32) -> Result<(), CellError> {
        self.wait_data_calls = self.wait_data_calls.saturating_add(1);
        self.require_initialized()?;
        let h = self.require_live_mut(id)?;
        if matches!(h.state, HandleState::Resetting) {
            return Err(CELL_USBPSPCM_ERROR_RESETTING);
        }
        if !matches!(h.state, HandleState::Bound) {
            return Err(CELL_USBPSPCM_ERROR_INVALID);
        }
        match h.data_wait.state {
            AsyncSlotState::Pending => Err(CELL_USBPSPCM_ERROR_INPROGRESS),
            AsyncSlotState::Canceled => {
                h.data_wait.state = AsyncSlotState::Idle;
                Err(CELL_USBPSPCM_ERROR_CANCELED)
            }
            AsyncSlotState::Completed => {
                h.data_wait.state = AsyncSlotState::Idle;
                if h.pending_recv_bytes == 0 {
                    Err(CELL_USBPSPCM_ERROR_NO_DATA)
                } else {
                    Ok(())
                }
            }
            AsyncSlotState::Idle => {
                if h.pending_recv_bytes > 0 {
                    Ok(())
                } else {
                    h.data_wait.state = AsyncSlotState::Pending;
                    Err(CELL_USBPSPCM_ERROR_INPROGRESS)
                }
            }
        }
    }

    /// `cellUsbPspcmPollData()` — non-blocking variant.
    pub fn poll_data(&mut self, id: u32) -> Result<u32, CellError> {
        self.poll_data_calls = self.poll_data_calls.saturating_add(1);
        self.require_initialized()?;
        let h = self.require_live_mut(id)?;
        if matches!(h.state, HandleState::Resetting) {
            return Err(CELL_USBPSPCM_ERROR_RESETTING);
        }
        if !matches!(h.state, HandleState::Bound) {
            return Err(CELL_USBPSPCM_ERROR_INVALID);
        }
        if h.pending_recv_bytes == 0 {
            Err(CELL_USBPSPCM_ERROR_NO_DATA)
        } else {
            Ok(h.pending_recv_bytes)
        }
    }

    /// `cellUsbPspcmCancelWaitData()` — wakes a blocked WaitData.
    pub fn cancel_wait_data(&mut self, id: u32) -> Result<(), CellError> {
        self.cancel_wait_data_calls = self.cancel_wait_data_calls.saturating_add(1);
        self.require_initialized()?;
        let h = self.require_live_mut(id)?;
        if !matches!(h.data_wait.state, AsyncSlotState::Pending) {
            return Err(CELL_USBPSPCM_ERROR_INVALID);
        }
        h.data_wait.state = AsyncSlotState::Canceled;
        Ok(())
    }

    // -- test hooks -----------------------------------------------------

    /// Simulate the async bind driver completing the bind transition.
    pub fn inject_bind_complete(&mut self, id: u32) {
        if let Some(h) = self.handle_mut(id) {
            if matches!(h.bind_slot.state, AsyncSlotState::Pending) {
                h.bind_slot.state = AsyncSlotState::Completed;
                if matches!(h.state, HandleState::Binding) {
                    h.state = HandleState::Bound;
                }
            }
        }
    }

    /// Simulate the async send driver completing.
    pub fn inject_send_complete(&mut self, id: u32) {
        if let Some(h) = self.handle_mut(id) {
            if matches!(h.send_slot.state, AsyncSlotState::Pending) {
                h.send_slot.state = AsyncSlotState::Completed;
            }
        }
    }

    /// Simulate the async recv driver completing with `bytes` bytes available.
    pub fn inject_recv_complete(&mut self, id: u32, bytes: u32) {
        if let Some(h) = self.handle_mut(id) {
            if matches!(h.recv_slot.state, AsyncSlotState::Pending) {
                h.recv_slot.state = AsyncSlotState::Completed;
                h.pending_recv_bytes = h.pending_recv_bytes.saturating_add(bytes);
            }
        }
    }

    /// Simulate the async reset driver completing → state Unbound, clear buffers.
    pub fn inject_reset_complete(&mut self, id: u32) {
        if let Some(h) = self.handle_mut(id) {
            if matches!(h.reset_slot.state, AsyncSlotState::Pending) {
                h.reset_slot.state = AsyncSlotState::Completed;
                if matches!(h.state, HandleState::Resetting) {
                    h.state = HandleState::Unbound;
                    h.pending_recv_bytes = 0;
                }
            }
        }
    }

    /// Simulate data arriving on a bound handle, waking a pending WaitData.
    pub fn inject_data_ready(&mut self, id: u32, bytes: u32) {
        if let Some(h) = self.handle_mut(id) {
            h.pending_recv_bytes = h.pending_recv_bytes.saturating_add(bytes);
            if matches!(h.data_wait.state, AsyncSlotState::Pending) {
                h.data_wait.state = AsyncSlotState::Completed;
            }
        }
    }

    // -- internals ------------------------------------------------------

    fn require_live_mut(&mut self, id: u32) -> Result<&mut Handle, CellError> {
        let h = self
            .handle_mut(id)
            .ok_or(CELL_USBPSPCM_ERROR_INVALID)?;
        match h.state {
            HandleState::Closed => Err(CELL_USBPSPCM_ERROR_CLOSED),
            _ => Ok(h),
        }
    }

    /// Shared Wait/Poll consumer. `block=true` mirrors `WaitXxx` (returns
    /// `INPROGRESS` if Pending and caller would have to wait); we don't spin,
    /// so Pending is reported the same in both variants — tests inject the
    /// completion explicitly. The helper distinguishes Canceled → `CANCELED`
    /// and Completed → `Ok(())`, and resets the slot to Idle after consumption.
    fn consume_slot(
        &mut self,
        id: u32,
        kind: SlotKind,
        _block: bool,
    ) -> Result<(), CellError> {
        let h = self.require_live_mut(id)?;
        let slot = match kind {
            SlotKind::Bind => &mut h.bind_slot,
            SlotKind::Send => &mut h.send_slot,
            SlotKind::Recv => &mut h.recv_slot,
            SlotKind::Reset => &mut h.reset_slot,
        };
        match slot.state {
            AsyncSlotState::Idle => Err(CELL_USBPSPCM_ERROR_INVALID),
            AsyncSlotState::Pending => Err(CELL_USBPSPCM_ERROR_INPROGRESS),
            AsyncSlotState::Canceled => {
                slot.state = AsyncSlotState::Idle;
                Err(CELL_USBPSPCM_ERROR_CANCELED)
            }
            AsyncSlotState::Completed => {
                slot.state = AsyncSlotState::Idle;
                // Reset-completed additionally signals RESET_END one time.
                match kind {
                    SlotKind::Reset => Err(CELL_USBPSPCM_ERROR_RESET_END),
                    _ => Ok(()),
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SlotKind {
    Bind,
    Send,
    Recv,
    Reset,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn init_and_register(n: usize) -> (UsbPspcm, Vec<u32>) {
        let mut m = UsbPspcm::new();
        m.init().unwrap();
        let mut ids = Vec::with_capacity(n);
        for _ in 0..n {
            ids.push(m.register().unwrap());
        }
        (m, ids)
    }

    #[test]
    fn module_name_and_entries_match_cpp() {
        assert_eq!(MODULE_NAME, "cellUsbPspcm");
        assert_eq!(REGISTERED_ENTRY_POINTS.len(), 27);
        assert_eq!(REGISTERED_ENTRY_POINTS[0], "cellUsbPspcmInit");
        assert_eq!(REGISTERED_ENTRY_POINTS[26], "cellUsbPspcmCancelWaitData");
    }

    #[test]
    fn error_codes_byte_exact() {
        assert_eq!(CELL_USBPSPCM_ERROR_NOT_INITIALIZED.0, 0x8011_0401);
        assert_eq!(CELL_USBPSPCM_ERROR_ALREADY.0, 0x8011_0402);
        assert_eq!(CELL_USBPSPCM_ERROR_INVALID.0, 0x8011_0403);
        assert_eq!(CELL_USBPSPCM_ERROR_NO_MEMORY.0, 0x8011_0404);
        assert_eq!(CELL_USBPSPCM_ERROR_BUSY.0, 0x8011_0405);
        assert_eq!(CELL_USBPSPCM_ERROR_INPROGRESS.0, 0x8011_0406);
        assert_eq!(CELL_USBPSPCM_ERROR_NO_SPACE.0, 0x8011_0407);
        assert_eq!(CELL_USBPSPCM_ERROR_CANCELED.0, 0x8011_0408);
        assert_eq!(CELL_USBPSPCM_ERROR_RESETTING.0, 0x8011_0409);
        assert_eq!(CELL_USBPSPCM_ERROR_RESET_END.0, 0x8011_040A);
        assert_eq!(CELL_USBPSPCM_ERROR_CLOSED.0, 0x8011_040B);
        assert_eq!(CELL_USBPSPCM_ERROR_NO_DATA.0, 0x8011_040C);
    }

    #[test]
    fn init_double_init_rejected() {
        let mut m = UsbPspcm::new();
        m.init().unwrap();
        assert_eq!(m.init(), Err(CELL_USBPSPCM_ERROR_ALREADY));
    }

    #[test]
    fn end_without_init_fails() {
        let mut m = UsbPspcm::new();
        assert_eq!(m.end(), Err(CELL_USBPSPCM_ERROR_NOT_INITIALIZED));
    }

    #[test]
    fn register_before_init_fails() {
        let mut m = UsbPspcm::new();
        assert_eq!(m.register(), Err(CELL_USBPSPCM_ERROR_NOT_INITIALIZED));
    }

    #[test]
    fn register_allocates_monotonic_ids_and_addrs() {
        let (m, ids) = init_and_register(3);
        assert_eq!(ids[0], HANDLE_BASE);
        assert_eq!(ids[1], HANDLE_BASE + 1);
        assert_eq!(ids[2], HANDLE_BASE + 2);
        let addrs: Vec<u32> = m.handles().iter().map(|h| h.addr).collect();
        assert_eq!(addrs, [0xA000_0000, 0xA000_1000, 0xA000_2000]);
    }

    #[test]
    fn register_overflow_returns_no_space() {
        let mut m = UsbPspcm::new();
        m.init().unwrap();
        for _ in 0..MAX_HANDLES {
            m.register().unwrap();
        }
        assert_eq!(m.register(), Err(CELL_USBPSPCM_ERROR_NO_SPACE));
    }

    #[test]
    fn calc_pool_size_formula() {
        let mut m = UsbPspcm::new();
        assert_eq!(m.calc_pool_size(1).unwrap(), 0x240);
        assert_eq!(m.calc_pool_size(4).unwrap(), 0x840);
        assert_eq!(m.calc_pool_size(0), Err(CELL_USBPSPCM_ERROR_INVALID));
        assert_eq!(
            m.calc_pool_size((MAX_HANDLES + 1) as u32),
            Err(CELL_USBPSPCM_ERROR_INVALID)
        );
    }

    #[test]
    fn bind_sync_transitions_unbound_to_bound() {
        let (mut m, ids) = init_and_register(1);
        m.bind(ids[0]).unwrap();
        assert_eq!(m.handle(ids[0]).unwrap().state, HandleState::Bound);
    }

    #[test]
    fn bind_twice_returns_already() {
        let (mut m, ids) = init_and_register(1);
        m.bind(ids[0]).unwrap();
        assert_eq!(m.bind(ids[0]), Err(CELL_USBPSPCM_ERROR_ALREADY));
    }

    #[test]
    fn bind_async_wait_poll_complete_flow() {
        let (mut m, ids) = init_and_register(1);
        m.bind_async(ids[0]).unwrap();
        assert_eq!(m.handle(ids[0]).unwrap().state, HandleState::Binding);
        // Still pending.
        assert_eq!(m.poll_bind_async(ids[0]), Err(CELL_USBPSPCM_ERROR_INPROGRESS));
        assert_eq!(m.wait_bind_async(ids[0]), Err(CELL_USBPSPCM_ERROR_INPROGRESS));
        // Simulate completion.
        m.inject_bind_complete(ids[0]);
        assert_eq!(m.handle(ids[0]).unwrap().state, HandleState::Bound);
        m.wait_bind_async(ids[0]).unwrap();
        // Slot idle after consumption.
        assert_eq!(m.wait_bind_async(ids[0]), Err(CELL_USBPSPCM_ERROR_INVALID));
    }

    #[test]
    fn cancel_bind_reverts_state_and_flags_slot() {
        let (mut m, ids) = init_and_register(1);
        m.bind_async(ids[0]).unwrap();
        m.cancel_bind(ids[0]).unwrap();
        assert_eq!(m.handle(ids[0]).unwrap().state, HandleState::Unbound);
        assert_eq!(m.poll_bind_async(ids[0]), Err(CELL_USBPSPCM_ERROR_CANCELED));
    }

    #[test]
    fn cancel_bind_without_pending_is_invalid() {
        let (mut m, ids) = init_and_register(1);
        assert_eq!(m.cancel_bind(ids[0]), Err(CELL_USBPSPCM_ERROR_INVALID));
    }

    #[test]
    fn send_requires_bound_and_rejects_empty() {
        let (mut m, ids) = init_and_register(1);
        assert_eq!(m.send(ids[0], 10), Err(CELL_USBPSPCM_ERROR_INVALID)); // Unbound
        m.bind(ids[0]).unwrap();
        assert_eq!(m.send(ids[0], 0), Err(CELL_USBPSPCM_ERROR_INVALID));
        m.send(ids[0], 64).unwrap();
    }

    #[test]
    fn recv_on_empty_returns_no_data() {
        let (mut m, ids) = init_and_register(1);
        m.bind(ids[0]).unwrap();
        assert_eq!(m.recv(ids[0], 128), Err(CELL_USBPSPCM_ERROR_NO_DATA));
    }

    #[test]
    fn recv_consumes_pending_bytes_up_to_max_len() {
        let (mut m, ids) = init_and_register(1);
        m.bind(ids[0]).unwrap();
        // Inject via the data-ready hook.
        m.inject_data_ready(ids[0], 300);
        assert_eq!(m.recv(ids[0], 100).unwrap(), 100);
        assert_eq!(m.handle(ids[0]).unwrap().pending_recv_bytes, 200);
        assert_eq!(m.recv(ids[0], 1000).unwrap(), 200);
        assert_eq!(m.handle(ids[0]).unwrap().pending_recv_bytes, 0);
    }

    #[test]
    fn recv_async_wait_complete_flow() {
        let (mut m, ids) = init_and_register(1);
        m.bind(ids[0]).unwrap();
        m.recv_async(ids[0], 256).unwrap();
        assert_eq!(m.poll_recv_async(ids[0]), Err(CELL_USBPSPCM_ERROR_INPROGRESS));
        m.inject_recv_complete(ids[0], 128);
        m.wait_recv_async(ids[0]).unwrap();
        // Slot cleared; pending bytes recorded.
        assert_eq!(m.handle(ids[0]).unwrap().pending_recv_bytes, 128);
    }

    #[test]
    fn send_async_wait_complete_flow() {
        let (mut m, ids) = init_and_register(1);
        m.bind(ids[0]).unwrap();
        m.send_async(ids[0], 64).unwrap();
        assert_eq!(m.poll_send_async(ids[0]), Err(CELL_USBPSPCM_ERROR_INPROGRESS));
        m.inject_send_complete(ids[0]);
        m.wait_send_async(ids[0]).unwrap();
    }

    #[test]
    fn reset_async_surfaces_reset_end_code() {
        let (mut m, ids) = init_and_register(1);
        m.bind(ids[0]).unwrap();
        m.reset_async(ids[0]).unwrap();
        assert_eq!(m.handle(ids[0]).unwrap().state, HandleState::Resetting);
        // Operations during reset: all RESETTING.
        assert_eq!(m.send(ids[0], 1), Err(CELL_USBPSPCM_ERROR_RESETTING));
        assert_eq!(m.recv(ids[0], 1), Err(CELL_USBPSPCM_ERROR_RESETTING));
        m.inject_reset_complete(ids[0]);
        // Wait returns RESET_END one time after completion.
        assert_eq!(
            m.wait_reset_async(ids[0]),
            Err(CELL_USBPSPCM_ERROR_RESET_END)
        );
        // Second time, slot is Idle — INVALID.
        assert_eq!(
            m.wait_reset_async(ids[0]),
            Err(CELL_USBPSPCM_ERROR_INVALID)
        );
        assert_eq!(m.handle(ids[0]).unwrap().state, HandleState::Unbound);
    }

    #[test]
    fn reset_sync_rejects_while_resetting() {
        let (mut m, ids) = init_and_register(1);
        m.bind(ids[0]).unwrap();
        m.reset_async(ids[0]).unwrap();
        assert_eq!(m.reset(ids[0]), Err(CELL_USBPSPCM_ERROR_RESETTING));
    }

    #[test]
    fn wait_data_canceled_via_cancel() {
        let (mut m, ids) = init_and_register(1);
        m.bind(ids[0]).unwrap();
        // No data yet — wait returns INPROGRESS and arms the slot.
        assert_eq!(m.wait_data(ids[0]), Err(CELL_USBPSPCM_ERROR_INPROGRESS));
        m.cancel_wait_data(ids[0]).unwrap();
        assert_eq!(m.wait_data(ids[0]), Err(CELL_USBPSPCM_ERROR_CANCELED));
        // Next call with no data re-arms as pending.
        assert_eq!(m.wait_data(ids[0]), Err(CELL_USBPSPCM_ERROR_INPROGRESS));
    }

    #[test]
    fn wait_data_returns_ok_when_data_already_buffered() {
        let (mut m, ids) = init_and_register(1);
        m.bind(ids[0]).unwrap();
        m.inject_data_ready(ids[0], 32);
        m.wait_data(ids[0]).unwrap();
    }

    #[test]
    fn poll_data_returns_pending_bytes_or_no_data() {
        let (mut m, ids) = init_and_register(1);
        m.bind(ids[0]).unwrap();
        assert_eq!(m.poll_data(ids[0]), Err(CELL_USBPSPCM_ERROR_NO_DATA));
        m.inject_data_ready(ids[0], 77);
        assert_eq!(m.poll_data(ids[0]).unwrap(), 77);
    }

    #[test]
    fn close_rejects_if_async_pending() {
        let (mut m, ids) = init_and_register(1);
        m.bind(ids[0]).unwrap();
        m.recv_async(ids[0], 64).unwrap();
        assert_eq!(m.close(ids[0]), Err(CELL_USBPSPCM_ERROR_BUSY));
        m.inject_recv_complete(ids[0], 64);
        // Still Pending? No — inject moved to Completed, but slot isn't Idle
        // until consumed. Close still sees non-idle slot → BUSY.
        assert_eq!(m.close(ids[0]), Err(CELL_USBPSPCM_ERROR_BUSY));
        m.wait_recv_async(ids[0]).unwrap();
        m.close(ids[0]).unwrap();
        assert_eq!(m.handle(ids[0]).unwrap().state, HandleState::Closed);
    }

    #[test]
    fn close_then_operations_return_closed() {
        let (mut m, ids) = init_and_register(1);
        m.bind(ids[0]).unwrap();
        m.close(ids[0]).unwrap();
        assert_eq!(m.bind(ids[0]), Err(CELL_USBPSPCM_ERROR_CLOSED));
        assert_eq!(m.send(ids[0], 1), Err(CELL_USBPSPCM_ERROR_CLOSED));
        assert_eq!(m.reset(ids[0]), Err(CELL_USBPSPCM_ERROR_CLOSED));
    }

    #[test]
    fn unregister_rejects_while_busy_and_resetting() {
        let (mut m, ids) = init_and_register(2);
        m.bind(ids[0]).unwrap();
        m.recv_async(ids[0], 16).unwrap();
        assert_eq!(m.unregister(ids[0]), Err(CELL_USBPSPCM_ERROR_BUSY));
        // Finish recv, unregister now allowed.
        m.inject_recv_complete(ids[0], 16);
        m.wait_recv_async(ids[0]).unwrap();
        m.unregister(ids[0]).unwrap();
        assert_eq!(m.handles().len(), 1);

        // Resetting blocks unregister with RESETTING.
        m.bind(ids[1]).unwrap();
        m.reset_async(ids[1]).unwrap();
        assert_eq!(m.unregister(ids[1]), Err(CELL_USBPSPCM_ERROR_RESETTING));
    }

    #[test]
    fn get_addr_requires_init_and_live_handle() {
        let mut m = UsbPspcm::new();
        assert_eq!(m.get_addr(0), Err(CELL_USBPSPCM_ERROR_NOT_INITIALIZED));
        m.init().unwrap();
        assert_eq!(m.get_addr(0xDEAD), Err(CELL_USBPSPCM_ERROR_INVALID));
        let id = m.register().unwrap();
        assert_eq!(m.get_addr(id).unwrap(), 0xA000_0000);
        m.bind(id).unwrap();
        m.close(id).unwrap();
        assert_eq!(m.get_addr(id), Err(CELL_USBPSPCM_ERROR_CLOSED));
    }

    #[test]
    fn end_clears_handles() {
        let (mut m, _ids) = init_and_register(3);
        m.end().unwrap();
        assert_eq!(m.handles().len(), 0);
        assert_eq!(m.state(), ModuleState::Finalized);
        // After end, any entry returns NOT_INITIALIZED.
        assert_eq!(m.register(), Err(CELL_USBPSPCM_ERROR_NOT_INITIALIZED));
        assert_eq!(m.bind(HANDLE_BASE), Err(CELL_USBPSPCM_ERROR_NOT_INITIALIZED));
    }

    #[test]
    fn counters_track_every_entry() {
        let (mut m, ids) = init_and_register(1);
        let _ = m.calc_pool_size(1);
        let _ = m.get_addr(ids[0]);
        m.bind(ids[0]).unwrap();
        m.send(ids[0], 1).unwrap();
        let _ = m.recv(ids[0], 1);
        let _ = m.poll_data(ids[0]);
        m.send_async(ids[0], 1).unwrap();
        m.inject_send_complete(ids[0]);
        m.wait_send_async(ids[0]).unwrap();
        m.recv_async(ids[0], 1).unwrap();
        m.inject_recv_complete(ids[0], 1);
        m.poll_recv_async(ids[0]).unwrap();
        m.reset_async(ids[0]).unwrap();
        m.inject_reset_complete(ids[0]);
        let _ = m.poll_reset_async(ids[0]);
        let _ = m.wait_data(ids[0]);
        let _ = m.cancel_wait_data(ids[0]);
        assert!(m.init_calls >= 1);
        assert!(m.register_calls >= 1);
        assert!(m.calc_pool_size_calls >= 1);
        assert!(m.get_addr_calls >= 1);
        assert!(m.bind_calls >= 1);
        assert!(m.send_calls >= 1);
        assert!(m.recv_calls >= 1);
        assert!(m.poll_data_calls >= 1);
        assert!(m.send_async_calls >= 1);
        assert!(m.wait_send_async_calls >= 1);
        assert!(m.recv_async_calls >= 1);
        assert!(m.poll_recv_async_calls >= 1);
        assert!(m.reset_async_calls >= 1);
        assert!(m.poll_reset_async_calls >= 1);
        assert!(m.wait_data_calls >= 1);
        assert!(m.cancel_wait_data_calls >= 1);
    }

    #[test]
    fn invalid_handle_id_returns_invalid() {
        let mut m = UsbPspcm::new();
        m.init().unwrap();
        assert_eq!(m.bind(0xDEAD), Err(CELL_USBPSPCM_ERROR_INVALID));
        assert_eq!(m.send(0xDEAD, 1), Err(CELL_USBPSPCM_ERROR_INVALID));
        assert_eq!(m.close(0xDEAD), Err(CELL_USBPSPCM_ERROR_INVALID));
    }

    #[test]
    fn full_usbpspcm_lifecycle_smoke() {
        let mut m = UsbPspcm::new();
        m.init().unwrap();
        let id = m.register().unwrap();
        assert_eq!(m.get_addr(id).unwrap(), 0xA000_0000);

        // Async bind path.
        m.bind_async(id).unwrap();
        assert_eq!(m.poll_bind_async(id), Err(CELL_USBPSPCM_ERROR_INPROGRESS));
        m.inject_bind_complete(id);
        m.poll_bind_async(id).unwrap();

        // Send sync.
        m.send(id, 128).unwrap();

        // Recv async path.
        m.recv_async(id, 256).unwrap();
        m.inject_recv_complete(id, 200);
        m.wait_recv_async(id).unwrap();
        assert_eq!(m.poll_data(id).unwrap(), 200);
        assert_eq!(m.recv(id, 256).unwrap(), 200);

        // Reset.
        m.reset_async(id).unwrap();
        m.inject_reset_complete(id);
        assert_eq!(m.wait_reset_async(id), Err(CELL_USBPSPCM_ERROR_RESET_END));
        assert_eq!(m.handle(id).unwrap().state, HandleState::Unbound);

        // Close + end.
        m.bind(id).unwrap();
        m.close(id).unwrap();
        m.end().unwrap();
        assert_eq!(m.state(), ModuleState::Finalized);
    }
}
