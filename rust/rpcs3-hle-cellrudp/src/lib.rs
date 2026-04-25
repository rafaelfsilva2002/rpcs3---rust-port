//! `rpcs3-hle-cellrudp` — Reliable UDP (libRudp) HLE.
//!
//! Ports `rpcs3/Emu/Cell/Modules/cellRudp.cpp`. PS3 games used libRudp
//! for low-latency delivery-guaranteed P2P (e.g. matchmaking payload
//! sync). The API shape:
//!
//! 1. `Init(allocator)` — register allocator hooks.
//! 2. `CreateContext(socket, event_handler, arg, muxmode)` — allocate
//!    a context over an existing `sys_net` socket.
//! 3. `SetOption(context, option, value)` / `GetOption`.
//! 4. `Bind(context, vport)` — reserve a virtual port.
//! 5. `Read` / `Write` / `Flush` / `Poll`.
//! 6. `TerminateContext(context)` / `End`.
//!
//! The Rust model focuses on lifecycle + option storage + per-context
//! vport map + framed message queue. Real network IO is plug-in.

use rpcs3_emu_types::CellError;

// =====================================================================
// Error codes — byte-exact with cellRudp.h:8-48
// =====================================================================

pub mod errors {
    use rpcs3_emu_types::CellError;

    pub const NOT_INITIALIZED: CellError = CellError(0x8077_0001);
    pub const ALREADY_INITIALIZED: CellError = CellError(0x8077_0002);
    pub const INVALID_CONTEXT_ID: CellError = CellError(0x8077_0003);
    pub const INVALID_ARGUMENT: CellError = CellError(0x8077_0004);
    pub const INVALID_OPTION: CellError = CellError(0x8077_0005);
    pub const INVALID_MUXMODE: CellError = CellError(0x8077_0006);
    pub const MEMORY: CellError = CellError(0x8077_0007);
    pub const INTERNAL: CellError = CellError(0x8077_0008);
    pub const CONN_RESET: CellError = CellError(0x8077_0009);
    pub const CONN_REFUSED: CellError = CellError(0x8077_000a);
    pub const CONN_TIMEOUT: CellError = CellError(0x8077_000b);
    pub const CONN_VERSION_MISMATCH: CellError = CellError(0x8077_000c);
    pub const CONN_TRANSPORT_TYPE_MISMATCH: CellError = CellError(0x8077_000d);
    pub const QUALITY_LEVEL_MISMATCH: CellError = CellError(0x8077_000e);
    pub const THREAD: CellError = CellError(0x8077_000f);
    pub const THREAD_IN_USE: CellError = CellError(0x8077_0010);
    pub const NOT_ACCEPTABLE: CellError = CellError(0x8077_0011);
    pub const MSG_TOO_LARGE: CellError = CellError(0x8077_0012);
    pub const NOT_BOUND: CellError = CellError(0x8077_0013);
    pub const CANCELLED: CellError = CellError(0x8077_0014);
    pub const INVALID_VPORT: CellError = CellError(0x8077_0015);
    pub const WOULDBLOCK: CellError = CellError(0x8077_0016);
    pub const VPORT_IN_USE: CellError = CellError(0x8077_0017);
    pub const VPORT_EXHAUSTED: CellError = CellError(0x8077_0018);
    pub const INVALID_SOCKET: CellError = CellError(0x8077_0019);
    pub const BUFFER_TOO_SMALL: CellError = CellError(0x8077_001a);
    pub const MSG_MALFORMED: CellError = CellError(0x8077_001b);
    pub const ADDR_IN_USE: CellError = CellError(0x8077_001c);
    pub const ALREADY_BOUND: CellError = CellError(0x8077_001d);
    pub const ALREADY_EXISTS: CellError = CellError(0x8077_001e);
    pub const INVALID_POLL_ID: CellError = CellError(0x8077_001f);
    pub const TOO_MANY_CONTEXTS: CellError = CellError(0x8077_0020);
    pub const IN_PROGRESS: CellError = CellError(0x8077_0021);
    pub const NO_EVENT_HANDLER: CellError = CellError(0x8077_0022);
    pub const PAYLOAD_TOO_LARGE: CellError = CellError(0x8077_0023);
    pub const END_OF_DATA: CellError = CellError(0x8077_0024);
    pub const ALREADY_ESTABLISHED: CellError = CellError(0x8077_0025);
    pub const KEEP_ALIVE_FAILURE: CellError = CellError(0x8077_0026);
}

// =====================================================================
// Context options (cellRudp.h:51-70)
// =====================================================================

pub const OPTION_MAX_PAYLOAD: i32 = 1;
pub const OPTION_SNDBUF: i32 = 2;
pub const OPTION_RCVBUF: i32 = 3;
pub const OPTION_NODELAY: i32 = 4;
pub const OPTION_DELIVERY_CRITICAL: i32 = 5;
pub const OPTION_ORDER_CRITICAL: i32 = 6;
pub const OPTION_NONBLOCK: i32 = 7;
pub const OPTION_STREAM: i32 = 8;
pub const OPTION_CONNECTION_TIMEOUT: i32 = 9;
pub const OPTION_CLOSE_WAIT_TIMEOUT: i32 = 10;
pub const OPTION_AGGREGATION_TIMEOUT: i32 = 11;
pub const OPTION_LAST_ERROR: i32 = 14;
pub const OPTION_READ_TIMEOUT: i32 = 15;
pub const OPTION_WRITE_TIMEOUT: i32 = 16;
pub const OPTION_FLUSH_TIMEOUT: i32 = 17;
pub const OPTION_KEEP_ALIVE_INTERVAL: i32 = 18;
pub const OPTION_KEEP_ALIVE_TIMEOUT: i32 = 19;

#[must_use]
pub fn is_known_option(opt: i32) -> bool {
    matches!(
        opt,
        OPTION_MAX_PAYLOAD
            | OPTION_SNDBUF
            | OPTION_RCVBUF
            | OPTION_NODELAY
            | OPTION_DELIVERY_CRITICAL
            | OPTION_ORDER_CRITICAL
            | OPTION_NONBLOCK
            | OPTION_STREAM
            | OPTION_CONNECTION_TIMEOUT
            | OPTION_CLOSE_WAIT_TIMEOUT
            | OPTION_AGGREGATION_TIMEOUT
            | OPTION_LAST_ERROR
            | OPTION_READ_TIMEOUT
            | OPTION_WRITE_TIMEOUT
            | OPTION_FLUSH_TIMEOUT
            | OPTION_KEEP_ALIVE_INTERVAL
            | OPTION_KEEP_ALIVE_TIMEOUT
    )
}

// =====================================================================
// Poll event flags (cellRudp.h:73-79)
// =====================================================================

pub const POLL_EV_READ: u32 = 0x0001;
pub const POLL_EV_WRITE: u32 = 0x0002;
pub const POLL_EV_FLUSH: u32 = 0x0004;
pub const POLL_EV_ERROR: u32 = 0x0008;
pub const POLL_EV_ALL_MASK: u32 = 0x000F;

// =====================================================================
// Limits
// =====================================================================

/// Maximum contexts per `init`. The real lib caps at ~256; we pick a
/// conservative value that matches observed game usage.
pub const MAX_CONTEXTS: usize = 256;

/// Virtual-port range exposed to games (1..=0xFFFF). Port 0 is reserved.
pub const MAX_VPORT: u32 = 0xFFFF;

/// Upper bound on `Write` payload size that the model accepts without
/// returning `PAYLOAD_TOO_LARGE`. Real lib is negotiable via
/// `OPTION_MAX_PAYLOAD`.
pub const PAYLOAD_LIMIT: usize = 65_536;

pub type ContextId = u32;
pub type Vport = u32;

// =====================================================================
// MuxMode
// =====================================================================

pub const MUXMODE_MUTED: i32 = 0;
pub const MUXMODE_SINGLE: i32 = 1;
pub const MUXMODE_MULTIPLE: i32 = 2;

#[must_use]
pub fn is_known_muxmode(m: i32) -> bool {
    (MUXMODE_MUTED..=MUXMODE_MULTIPLE).contains(&m)
}

// =====================================================================
// Internal types
// =====================================================================

#[derive(Clone, Debug)]
#[allow(dead_code)] // socket/muxmode/event_handler stored for future IO + dispatch
struct Context {
    id: ContextId,
    socket: i32,
    muxmode: i32,
    event_handler: bool,
    options: std::collections::HashMap<i32, i64>,
    bound_vport: Option<Vport>,
    rx_queue: std::collections::VecDeque<Vec<u8>>,
    last_error: i32,
}

impl Context {
    fn new(id: ContextId, socket: i32, muxmode: i32, event_handler: bool) -> Self {
        Self {
            id,
            socket,
            muxmode,
            event_handler,
            options: std::collections::HashMap::new(),
            bound_vport: None,
            rx_queue: std::collections::VecDeque::new(),
            last_error: 0,
        }
    }

    fn default_option(opt: i32) -> Option<i64> {
        match opt {
            OPTION_MAX_PAYLOAD => Some(i64::try_from(PAYLOAD_LIMIT).unwrap_or(i64::MAX)),
            OPTION_SNDBUF | OPTION_RCVBUF => Some(64 * 1024),
            OPTION_NODELAY
            | OPTION_DELIVERY_CRITICAL
            | OPTION_ORDER_CRITICAL
            | OPTION_NONBLOCK
            | OPTION_STREAM => Some(0),
            OPTION_CONNECTION_TIMEOUT
            | OPTION_CLOSE_WAIT_TIMEOUT
            | OPTION_AGGREGATION_TIMEOUT
            | OPTION_READ_TIMEOUT
            | OPTION_WRITE_TIMEOUT
            | OPTION_FLUSH_TIMEOUT
            | OPTION_KEEP_ALIVE_INTERVAL
            | OPTION_KEEP_ALIVE_TIMEOUT => Some(30_000_000), // 30s in µs
            OPTION_LAST_ERROR => Some(0),
            _ => None,
        }
    }
}

// =====================================================================
// Manager
// =====================================================================

#[derive(Clone, Debug)]
pub struct RudpManager {
    initialized: bool,
    contexts: Vec<Context>,
    bound_vports: std::collections::HashMap<Vport, ContextId>,
    next_id: ContextId,
    allocator_registered: bool,
}

impl RudpManager {
    #[must_use]
    pub fn new() -> Self {
        Self {
            initialized: false,
            contexts: Vec::new(),
            bound_vports: std::collections::HashMap::new(),
            next_id: 1,
            allocator_registered: false,
        }
    }

    #[must_use]
    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    #[must_use]
    pub fn context_count(&self) -> usize {
        self.contexts.len()
    }

    // ----------------- Lifecycle -----------------

    /// `cellRudpInit(allocator)` — `allocator` is optional; when None
    /// the library falls back to its own arena.
    pub fn init(&mut self, has_allocator: bool) -> Result<(), CellError> {
        if self.initialized {
            return Err(errors::ALREADY_INITIALIZED);
        }
        self.initialized = true;
        self.allocator_registered = has_allocator;
        self.contexts.clear();
        self.bound_vports.clear();
        self.next_id = 1;
        Ok(())
    }

    pub fn end(&mut self) -> Result<(), CellError> {
        if !self.initialized {
            return Err(errors::NOT_INITIALIZED);
        }
        self.initialized = false;
        self.contexts.clear();
        self.bound_vports.clear();
        self.allocator_registered = false;
        Ok(())
    }

    // ----------------- Context management -----------------

    /// `cellRudpCreateContext(socket, event_handler, arg, muxmode)`.
    pub fn create_context(
        &mut self,
        socket: i32,
        event_handler: bool,
        muxmode: i32,
    ) -> Result<ContextId, CellError> {
        self.require_initialized()?;
        if socket < 0 {
            return Err(errors::INVALID_SOCKET);
        }
        if !is_known_muxmode(muxmode) {
            return Err(errors::INVALID_MUXMODE);
        }
        if !event_handler {
            return Err(errors::NO_EVENT_HANDLER);
        }
        if self.contexts.len() >= MAX_CONTEXTS {
            return Err(errors::TOO_MANY_CONTEXTS);
        }
        let id = self.next_id;
        self.next_id = self.next_id.checked_add(1).ok_or(errors::TOO_MANY_CONTEXTS)?;
        self.contexts.push(Context::new(id, socket, muxmode, event_handler));
        Ok(id)
    }

    pub fn terminate_context(&mut self, id: ContextId) -> Result<(), CellError> {
        self.require_initialized()?;
        let idx = self.ctx_idx(id)?;
        if let Some(vport) = self.contexts[idx].bound_vport {
            self.bound_vports.remove(&vport);
        }
        self.contexts.remove(idx);
        Ok(())
    }

    // ----------------- Options -----------------

    pub fn set_option(&mut self, id: ContextId, option: i32, value: i64) -> Result<(), CellError> {
        self.require_initialized()?;
        if !is_known_option(option) {
            return Err(errors::INVALID_OPTION);
        }
        // OPTION_LAST_ERROR is read-only per C++ docs.
        if option == OPTION_LAST_ERROR {
            return Err(errors::INVALID_OPTION);
        }
        // Timeout-class options must be non-negative.
        if matches!(
            option,
            OPTION_CONNECTION_TIMEOUT
                | OPTION_CLOSE_WAIT_TIMEOUT
                | OPTION_AGGREGATION_TIMEOUT
                | OPTION_READ_TIMEOUT
                | OPTION_WRITE_TIMEOUT
                | OPTION_FLUSH_TIMEOUT
                | OPTION_KEEP_ALIVE_INTERVAL
                | OPTION_KEEP_ALIVE_TIMEOUT
        ) && value < 0
        {
            return Err(errors::INVALID_ARGUMENT);
        }
        if option == OPTION_MAX_PAYLOAD {
            if value <= 0 || value > i64::try_from(PAYLOAD_LIMIT).unwrap_or(i64::MAX) {
                return Err(errors::INVALID_ARGUMENT);
            }
        }
        // Boolean-ish options: clamp to 0/1.
        let stored = if matches!(
            option,
            OPTION_NODELAY | OPTION_DELIVERY_CRITICAL | OPTION_ORDER_CRITICAL | OPTION_NONBLOCK | OPTION_STREAM
        ) {
            if value != 0 { 1 } else { 0 }
        } else {
            value
        };
        let idx = self.ctx_idx(id)?;
        self.contexts[idx].options.insert(option, stored);
        Ok(())
    }

    pub fn get_option(&self, id: ContextId, option: i32) -> Result<i64, CellError> {
        self.require_initialized()?;
        if !is_known_option(option) {
            return Err(errors::INVALID_OPTION);
        }
        let idx = self.ctx_idx(id)?;
        if option == OPTION_LAST_ERROR {
            return Ok(i64::from(self.contexts[idx].last_error));
        }
        if let Some(v) = self.contexts[idx].options.get(&option) {
            return Ok(*v);
        }
        Context::default_option(option).ok_or(errors::INVALID_OPTION)
    }

    // ----------------- Bind -----------------

    /// `cellRudpBind(context, vport)`. Port 0 is invalid; duplicate
    /// binds on the same `vport` → `VPORT_IN_USE`.
    pub fn bind(&mut self, id: ContextId, vport: Vport) -> Result<(), CellError> {
        self.require_initialized()?;
        if vport == 0 || vport > MAX_VPORT {
            return Err(errors::INVALID_VPORT);
        }
        let idx = self.ctx_idx(id)?;
        if self.contexts[idx].bound_vport.is_some() {
            return Err(errors::ALREADY_BOUND);
        }
        if self.bound_vports.contains_key(&vport) {
            return Err(errors::VPORT_IN_USE);
        }
        self.contexts[idx].bound_vport = Some(vport);
        self.bound_vports.insert(vport, id);
        Ok(())
    }

    pub fn unbind(&mut self, id: ContextId) -> Result<Vport, CellError> {
        self.require_initialized()?;
        let idx = self.ctx_idx(id)?;
        let vport = self.contexts[idx].bound_vport.take().ok_or(errors::NOT_BOUND)?;
        self.bound_vports.remove(&vport);
        Ok(vport)
    }

    // ----------------- IO -----------------

    /// `cellRudpWrite(context, data)` — the Rust model just queues the
    /// payload if the context is bound; real IO is plugin territory.
    pub fn write(&mut self, id: ContextId, data: &[u8]) -> Result<usize, CellError> {
        self.require_initialized()?;
        let idx = self.ctx_idx(id)?;
        if self.contexts[idx].bound_vport.is_none() {
            return Err(errors::NOT_BOUND);
        }
        let max_payload = self
            .contexts[idx]
            .options
            .get(&OPTION_MAX_PAYLOAD)
            .copied()
            .unwrap_or_else(|| i64::try_from(PAYLOAD_LIMIT).unwrap_or(i64::MAX));
        if data.len() as i64 > max_payload {
            self.contexts[idx].last_error = errors::PAYLOAD_TOO_LARGE.0 as i32;
            return Err(errors::PAYLOAD_TOO_LARGE);
        }
        if data.is_empty() {
            return Err(errors::INVALID_ARGUMENT);
        }
        Ok(data.len())
    }

    /// Test hook: inject a received message into a context's rx queue.
    pub fn inject_recv(&mut self, id: ContextId, data: &[u8]) -> Result<(), CellError> {
        self.require_initialized()?;
        let idx = self.ctx_idx(id)?;
        if self.contexts[idx].bound_vport.is_none() {
            return Err(errors::NOT_BOUND);
        }
        self.contexts[idx].rx_queue.push_back(data.to_vec());
        Ok(())
    }

    pub fn read(&mut self, id: ContextId, out: &mut [u8]) -> Result<usize, CellError> {
        self.require_initialized()?;
        let idx = self.ctx_idx(id)?;
        if self.contexts[idx].bound_vport.is_none() {
            return Err(errors::NOT_BOUND);
        }
        let nonblock = self.contexts[idx].options.get(&OPTION_NONBLOCK).copied().unwrap_or(0) != 0;
        let Some(msg) = self.contexts[idx].rx_queue.pop_front() else {
            return Err(if nonblock { errors::WOULDBLOCK } else { errors::END_OF_DATA });
        };
        if out.len() < msg.len() {
            // Real lib re-queues the message on buffer-too-small.
            self.contexts[idx].rx_queue.push_front(msg);
            return Err(errors::BUFFER_TOO_SMALL);
        }
        out[..msg.len()].copy_from_slice(&msg);
        Ok(msg.len())
    }

    pub fn flush(&mut self, id: ContextId) -> Result<(), CellError> {
        self.require_initialized()?;
        let idx = self.ctx_idx(id)?;
        if self.contexts[idx].bound_vport.is_none() {
            return Err(errors::NOT_BOUND);
        }
        Ok(())
    }

    pub fn poll(&self, id: ContextId, event_mask: u32) -> Result<u32, CellError> {
        self.require_initialized()?;
        if (event_mask & !POLL_EV_ALL_MASK) != 0 {
            return Err(errors::INVALID_ARGUMENT);
        }
        let idx = self.ctx_idx(id)?;
        let ctx = &self.contexts[idx];
        let mut ready = 0u32;
        if (event_mask & POLL_EV_READ) != 0 && !ctx.rx_queue.is_empty() {
            ready |= POLL_EV_READ;
        }
        if (event_mask & POLL_EV_WRITE) != 0 && ctx.bound_vport.is_some() {
            ready |= POLL_EV_WRITE;
        }
        if (event_mask & POLL_EV_FLUSH) != 0 {
            ready |= POLL_EV_FLUSH;
        }
        if (event_mask & POLL_EV_ERROR) != 0 && ctx.last_error != 0 {
            ready |= POLL_EV_ERROR;
        }
        Ok(ready)
    }

    // ----------------- Helpers -----------------

    fn require_initialized(&self) -> Result<(), CellError> {
        if self.initialized { Ok(()) } else { Err(errors::NOT_INITIALIZED) }
    }

    fn ctx_idx(&self, id: ContextId) -> Result<usize, CellError> {
        self.contexts.iter().position(|c| c.id == id).ok_or(errors::INVALID_CONTEXT_ID)
    }
}

impl Default for RudpManager {
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

    fn initialized() -> RudpManager {
        let mut m = RudpManager::new();
        m.init(true).unwrap();
        m
    }

    fn create_ctx(m: &mut RudpManager) -> ContextId {
        m.create_context(1, true, MUXMODE_SINGLE).unwrap()
    }

    #[test]
    fn error_codes_byte_exact() {
        assert_eq!(errors::NOT_INITIALIZED.0, 0x8077_0001);
        assert_eq!(errors::ALREADY_INITIALIZED.0, 0x8077_0002);
        assert_eq!(errors::INVALID_CONTEXT_ID.0, 0x8077_0003);
        assert_eq!(errors::INVALID_ARGUMENT.0, 0x8077_0004);
        assert_eq!(errors::INVALID_OPTION.0, 0x8077_0005);
        assert_eq!(errors::INVALID_MUXMODE.0, 0x8077_0006);
        assert_eq!(errors::MEMORY.0, 0x8077_0007);
        assert_eq!(errors::MSG_TOO_LARGE.0, 0x8077_0012);
        assert_eq!(errors::INVALID_VPORT.0, 0x8077_0015);
        assert_eq!(errors::WOULDBLOCK.0, 0x8077_0016);
        assert_eq!(errors::VPORT_IN_USE.0, 0x8077_0017);
        assert_eq!(errors::TOO_MANY_CONTEXTS.0, 0x8077_0020);
        assert_eq!(errors::END_OF_DATA.0, 0x8077_0024);
        assert_eq!(errors::KEEP_ALIVE_FAILURE.0, 0x8077_0026);
    }

    #[test]
    fn option_constants_stable() {
        assert_eq!(OPTION_MAX_PAYLOAD, 1);
        assert_eq!(OPTION_SNDBUF, 2);
        assert_eq!(OPTION_RCVBUF, 3);
        assert_eq!(OPTION_NODELAY, 4);
        assert_eq!(OPTION_NONBLOCK, 7);
        assert_eq!(OPTION_STREAM, 8);
        assert_eq!(OPTION_LAST_ERROR, 14);
        assert_eq!(OPTION_KEEP_ALIVE_TIMEOUT, 19);
    }

    #[test]
    fn poll_ev_bits_stable() {
        assert_eq!(POLL_EV_READ, 0x0001);
        assert_eq!(POLL_EV_WRITE, 0x0002);
        assert_eq!(POLL_EV_FLUSH, 0x0004);
        assert_eq!(POLL_EV_ERROR, 0x0008);
        assert_eq!(POLL_EV_ALL_MASK, 0x000F);
    }

    #[test]
    fn muxmode_constants_stable() {
        assert_eq!(MUXMODE_MUTED, 0);
        assert_eq!(MUXMODE_SINGLE, 1);
        assert_eq!(MUXMODE_MULTIPLE, 2);
        assert!(is_known_muxmode(MUXMODE_MUTED));
        assert!(!is_known_muxmode(99));
    }

    #[test]
    fn init_happy_path() {
        let mut m = RudpManager::new();
        m.init(true).unwrap();
        assert!(m.is_initialized());
    }

    #[test]
    fn init_twice_is_already_initialized() {
        let mut m = initialized();
        assert_eq!(m.init(true), Err(errors::ALREADY_INITIALIZED));
    }

    #[test]
    fn end_without_init_is_not_initialized() {
        let mut m = RudpManager::new();
        assert_eq!(m.end(), Err(errors::NOT_INITIALIZED));
    }

    #[test]
    fn init_works_without_allocator() {
        let mut m = RudpManager::new();
        m.init(false).unwrap();
        assert!(m.is_initialized());
    }

    #[test]
    fn create_context_without_init_rejected() {
        let mut m = RudpManager::new();
        assert_eq!(m.create_context(1, true, MUXMODE_SINGLE), Err(errors::NOT_INITIALIZED));
    }

    #[test]
    fn create_context_negative_socket_rejected() {
        let mut m = initialized();
        assert_eq!(m.create_context(-1, true, MUXMODE_SINGLE), Err(errors::INVALID_SOCKET));
    }

    #[test]
    fn create_context_bad_muxmode_rejected() {
        let mut m = initialized();
        assert_eq!(m.create_context(1, true, 99), Err(errors::INVALID_MUXMODE));
    }

    #[test]
    fn create_context_missing_handler_rejected() {
        let mut m = initialized();
        assert_eq!(m.create_context(1, false, MUXMODE_SINGLE), Err(errors::NO_EVENT_HANDLER));
    }

    #[test]
    fn create_context_increments_id() {
        let mut m = initialized();
        let a = create_ctx(&mut m);
        let b = create_ctx(&mut m);
        assert_eq!(b, a + 1);
        assert_eq!(m.context_count(), 2);
    }

    #[test]
    fn terminate_context_bad_id_rejected() {
        let mut m = initialized();
        assert_eq!(m.terminate_context(999), Err(errors::INVALID_CONTEXT_ID));
    }

    #[test]
    fn terminate_context_unbinds_vport() {
        let mut m = initialized();
        let id = create_ctx(&mut m);
        m.bind(id, 5000).unwrap();
        m.terminate_context(id).unwrap();
        // Vport should be free again.
        let id2 = create_ctx(&mut m);
        m.bind(id2, 5000).unwrap();
    }

    #[test]
    fn set_get_option_round_trip() {
        let mut m = initialized();
        let id = create_ctx(&mut m);
        m.set_option(id, OPTION_SNDBUF, 128_000).unwrap();
        assert_eq!(m.get_option(id, OPTION_SNDBUF), Ok(128_000));
    }

    #[test]
    fn get_option_returns_defaults() {
        let mut m = initialized();
        let id = create_ctx(&mut m);
        assert_eq!(m.get_option(id, OPTION_RCVBUF), Ok(64 * 1024));
        assert_eq!(m.get_option(id, OPTION_NODELAY), Ok(0));
    }

    #[test]
    fn set_option_unknown_rejected() {
        let mut m = initialized();
        let id = create_ctx(&mut m);
        assert_eq!(m.set_option(id, 999, 1), Err(errors::INVALID_OPTION));
    }

    #[test]
    fn set_option_last_error_is_readonly() {
        let mut m = initialized();
        let id = create_ctx(&mut m);
        assert_eq!(m.set_option(id, OPTION_LAST_ERROR, 0), Err(errors::INVALID_OPTION));
    }

    #[test]
    fn set_option_timeout_negative_rejected() {
        let mut m = initialized();
        let id = create_ctx(&mut m);
        assert_eq!(m.set_option(id, OPTION_READ_TIMEOUT, -1), Err(errors::INVALID_ARGUMENT));
    }

    #[test]
    fn set_option_max_payload_zero_rejected() {
        let mut m = initialized();
        let id = create_ctx(&mut m);
        assert_eq!(m.set_option(id, OPTION_MAX_PAYLOAD, 0), Err(errors::INVALID_ARGUMENT));
    }

    #[test]
    fn set_option_boolean_clamps_to_zero_one() {
        let mut m = initialized();
        let id = create_ctx(&mut m);
        m.set_option(id, OPTION_NONBLOCK, 42).unwrap();
        assert_eq!(m.get_option(id, OPTION_NONBLOCK), Ok(1));
        m.set_option(id, OPTION_NONBLOCK, 0).unwrap();
        assert_eq!(m.get_option(id, OPTION_NONBLOCK), Ok(0));
    }

    #[test]
    fn bind_happy_path() {
        let mut m = initialized();
        let id = create_ctx(&mut m);
        m.bind(id, 1234).unwrap();
    }

    #[test]
    fn bind_vport_zero_rejected() {
        let mut m = initialized();
        let id = create_ctx(&mut m);
        assert_eq!(m.bind(id, 0), Err(errors::INVALID_VPORT));
    }

    #[test]
    fn bind_vport_over_max_rejected() {
        let mut m = initialized();
        let id = create_ctx(&mut m);
        assert_eq!(m.bind(id, 0x10000), Err(errors::INVALID_VPORT));
    }

    #[test]
    fn bind_same_context_twice_is_already_bound() {
        let mut m = initialized();
        let id = create_ctx(&mut m);
        m.bind(id, 1234).unwrap();
        assert_eq!(m.bind(id, 5678), Err(errors::ALREADY_BOUND));
    }

    #[test]
    fn bind_conflict_is_vport_in_use() {
        let mut m = initialized();
        let a = create_ctx(&mut m);
        let b = create_ctx(&mut m);
        m.bind(a, 1234).unwrap();
        assert_eq!(m.bind(b, 1234), Err(errors::VPORT_IN_USE));
    }

    #[test]
    fn unbind_happy_path() {
        let mut m = initialized();
        let id = create_ctx(&mut m);
        m.bind(id, 1234).unwrap();
        assert_eq!(m.unbind(id), Ok(1234));
    }

    #[test]
    fn unbind_without_bind_is_not_bound() {
        let mut m = initialized();
        let id = create_ctx(&mut m);
        assert_eq!(m.unbind(id), Err(errors::NOT_BOUND));
    }

    #[test]
    fn write_without_bind_is_not_bound() {
        let mut m = initialized();
        let id = create_ctx(&mut m);
        assert_eq!(m.write(id, &[1, 2, 3]), Err(errors::NOT_BOUND));
    }

    #[test]
    fn write_empty_data_is_invalid_argument() {
        let mut m = initialized();
        let id = create_ctx(&mut m);
        m.bind(id, 1234).unwrap();
        assert_eq!(m.write(id, &[]), Err(errors::INVALID_ARGUMENT));
    }

    #[test]
    fn write_oversize_rejected_with_payload_too_large() {
        let mut m = initialized();
        let id = create_ctx(&mut m);
        m.bind(id, 1234).unwrap();
        m.set_option(id, OPTION_MAX_PAYLOAD, 16).unwrap();
        let data = [0u8; 32];
        assert_eq!(m.write(id, &data), Err(errors::PAYLOAD_TOO_LARGE));
    }

    #[test]
    fn write_happy_path_returns_length() {
        let mut m = initialized();
        let id = create_ctx(&mut m);
        m.bind(id, 1234).unwrap();
        assert_eq!(m.write(id, &[1, 2, 3, 4]), Ok(4));
    }

    #[test]
    fn read_empty_queue_blocking_is_end_of_data() {
        let mut m = initialized();
        let id = create_ctx(&mut m);
        m.bind(id, 1234).unwrap();
        let mut buf = [0u8; 16];
        assert_eq!(m.read(id, &mut buf), Err(errors::END_OF_DATA));
    }

    #[test]
    fn read_empty_queue_nonblocking_is_wouldblock() {
        let mut m = initialized();
        let id = create_ctx(&mut m);
        m.bind(id, 1234).unwrap();
        m.set_option(id, OPTION_NONBLOCK, 1).unwrap();
        let mut buf = [0u8; 16];
        assert_eq!(m.read(id, &mut buf), Err(errors::WOULDBLOCK));
    }

    #[test]
    fn read_happy_path_returns_bytes() {
        let mut m = initialized();
        let id = create_ctx(&mut m);
        m.bind(id, 1234).unwrap();
        m.inject_recv(id, &[10, 20, 30]).unwrap();
        let mut buf = [0u8; 8];
        assert_eq!(m.read(id, &mut buf), Ok(3));
        assert_eq!(&buf[..3], &[10, 20, 30]);
    }

    #[test]
    fn read_buffer_too_small_requeues() {
        let mut m = initialized();
        let id = create_ctx(&mut m);
        m.bind(id, 1234).unwrap();
        m.inject_recv(id, &[1, 2, 3, 4, 5]).unwrap();
        let mut small = [0u8; 2];
        assert_eq!(m.read(id, &mut small), Err(errors::BUFFER_TOO_SMALL));
        // The message should still be in the queue.
        let mut big = [0u8; 8];
        assert_eq!(m.read(id, &mut big), Ok(5));
    }

    #[test]
    fn flush_without_bind_is_not_bound() {
        let mut m = initialized();
        let id = create_ctx(&mut m);
        assert_eq!(m.flush(id), Err(errors::NOT_BOUND));
    }

    #[test]
    fn poll_bad_mask_rejected() {
        let mut m = initialized();
        let id = create_ctx(&mut m);
        assert_eq!(m.poll(id, 0xF0), Err(errors::INVALID_ARGUMENT));
    }

    #[test]
    fn poll_reports_read_when_queue_has_data() {
        let mut m = initialized();
        let id = create_ctx(&mut m);
        m.bind(id, 1234).unwrap();
        m.inject_recv(id, &[7]).unwrap();
        let r = m.poll(id, POLL_EV_READ | POLL_EV_WRITE).unwrap();
        assert!(r & POLL_EV_READ != 0);
        assert!(r & POLL_EV_WRITE != 0);
    }

    #[test]
    fn poll_write_requires_bind() {
        let mut m = initialized();
        let id = create_ctx(&mut m);
        let r = m.poll(id, POLL_EV_WRITE).unwrap();
        assert_eq!(r, 0);
    }

    #[test]
    fn full_lifecycle_smoke() {
        let mut m = RudpManager::new();
        m.init(true).unwrap();
        let a = m.create_context(10, true, MUXMODE_SINGLE).unwrap();
        let b = m.create_context(11, true, MUXMODE_MULTIPLE).unwrap();
        m.set_option(a, OPTION_NODELAY, 1).unwrap();
        m.set_option(a, OPTION_NONBLOCK, 1).unwrap();
        m.bind(a, 5000).unwrap();
        m.bind(b, 5001).unwrap();
        m.write(a, &[0xDE, 0xAD, 0xBE, 0xEF]).unwrap();
        m.inject_recv(b, &[0xCA, 0xFE]).unwrap();
        let mut rx = [0u8; 8];
        assert_eq!(m.read(b, &mut rx), Ok(2));
        m.flush(a).unwrap();
        assert!(m.poll(b, POLL_EV_READ | POLL_EV_WRITE).unwrap() & POLL_EV_WRITE != 0);
        m.terminate_context(a).unwrap();
        m.terminate_context(b).unwrap();
        m.end().unwrap();
    }
}
