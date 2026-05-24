//! `rpcs3-lv2-event` — event queue / event port syscalls.
//!
//! Ports `rpcs3/Emu/Cell/lv2/sys_event.cpp`. This is LV2's backbone
//! for inter-thread signalling: ports raise `(source, data1..3)`
//! tuples; queues collect them and PPU/SPU threads block on
//! `receive`.
//!
//! ## Scope (iteration 1)
//!
//! * `sys_event_queue_create/destroy/receive/tryreceive/drain`
//! * `sys_event_port_create/destroy/connect_local/disconnect/send`

use rpcs3_emu_types::CellError;

// =====================================================================
// Constants (sys_event.h)
// =====================================================================

/// Queue type — PPU threads wait on it.
pub const QUEUE_PPU: u32 = 1;
/// Queue type — SPU threads wait on it.
pub const QUEUE_SPU: u32 = 2;

/// Protocol — FIFO waiter ordering.
pub const PROTOCOL_FIFO: u32 = 0x01;
/// Protocol — priority waiter ordering.
pub const PROTOCOL_PRIORITY: u32 = 0x02;

/// Port type — local (same process) connections only.
pub const PORT_LOCAL: u32 = 1;
/// Port type — IPC across address spaces (not modelled yet).
pub const PORT_IPC: u32 = 3;

/// Queue destroy mode — wait for waiters (0) or force (1).
pub const QUEUE_DESTROY_WAIT: u32 = 0;
pub const QUEUE_DESTROY_FORCE: u32 = 1;

// =====================================================================
// Types
// =====================================================================

/// `sys_event_t` — the 4-tuple delivered to receivers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Event {
    pub source: u64,
    pub data1: u64,
    pub data2: u64,
    pub data3: u64,
}

/// Attributes for `sys_event_queue_create`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueueAttr {
    pub protocol: u32, // FIFO / PRIORITY
    pub queue_type: u32, // PPU / SPU
}

impl Default for QueueAttr {
    fn default() -> Self {
        Self { protocol: PROTOCOL_FIFO, queue_type: QUEUE_PPU }
    }
}

/// Return of blocking `receive` — mirrors `BlockOutcome` in lv2-sync
/// but carrying the event payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReceiveOutcome {
    /// Got an event immediately.
    Received(Event),
    /// Nothing queued; caller parks and retries when signalled.
    MustBlock,
}

// =====================================================================
// Registry trait (emu core owns concrete impl)
// =====================================================================

pub trait EventRegistry {
    // ---- Queues ------------------------------------------------

    fn queue_create(&mut self, attr: QueueAttr, size: u32) -> Result<u32, CellError>;
    fn queue_destroy(&mut self, id: u32, mode: u32) -> Result<(), CellError>;
    fn queue_receive(&mut self, id: u32) -> Result<ReceiveOutcome, CellError>;
    fn queue_tryreceive(&mut self, id: u32, max: u32) -> Result<Vec<Event>, CellError>;
    fn queue_drain(&mut self, id: u32) -> Result<(), CellError>;

    // ---- Ports -------------------------------------------------

    fn port_create(&mut self, port_type: u32, name: u64) -> Result<u32, CellError>;
    fn port_destroy(&mut self, id: u32) -> Result<(), CellError>;
    fn port_connect_local(&mut self, port: u32, queue: u32) -> Result<(), CellError>;
    fn port_disconnect(&mut self, port: u32) -> Result<(), CellError>;
    fn port_send(&mut self, port: u32, data1: u64, data2: u64, data3: u64)
        -> Result<(), CellError>;
}

// =====================================================================
// Syscalls (thin validating wrappers)
// =====================================================================

/// `sys_event_queue_create(id_out, attr, ipc_key, size)` — ipc_key
/// ignored in local-only mode.
#[must_use]
pub fn sys_event_queue_create<R: EventRegistry + ?Sized>(
    reg: &mut R,
    attr: QueueAttr,
    _ipc_key: u64,
    size: u32,
) -> Result<u32, CellError> {
    if !matches!(attr.protocol, PROTOCOL_FIFO | PROTOCOL_PRIORITY) {
        return Err(CellError::EINVAL);
    }
    if !matches!(attr.queue_type, QUEUE_PPU | QUEUE_SPU) {
        return Err(CellError::EINVAL);
    }
    // C++ enforces 1..=127 queue depth.
    if size == 0 || size > 127 {
        return Err(CellError::EINVAL);
    }
    reg.queue_create(attr, size)
}

/// `sys_event_queue_destroy(id, mode)`.
#[must_use]
pub fn sys_event_queue_destroy<R: EventRegistry + ?Sized>(
    reg: &mut R,
    id: u32,
    mode: u32,
) -> Result<(), CellError> {
    if !matches!(mode, QUEUE_DESTROY_WAIT | QUEUE_DESTROY_FORCE) {
        return Err(CellError::EINVAL);
    }
    reg.queue_destroy(id, mode)
}

/// `sys_event_queue_receive(id, event_out, timeout)` — blocking.
/// Emu core reads `ReceiveOutcome`: if `MustBlock`, parks the thread.
#[must_use]
pub fn sys_event_queue_receive<R: EventRegistry + ?Sized>(
    reg: &mut R,
    id: u32,
    _timeout_us: u64,
) -> Result<ReceiveOutcome, CellError> {
    reg.queue_receive(id)
}

/// `sys_event_queue_tryreceive(id, event_out_array, size, count_out)`
/// non-blocking batch up to `size` events; returns the list actually
/// drained (may be shorter).
#[must_use]
pub fn sys_event_queue_tryreceive<R: EventRegistry + ?Sized>(
    reg: &mut R,
    id: u32,
    size: u32,
) -> Result<Vec<Event>, CellError> {
    reg.queue_tryreceive(id, size)
}

/// `sys_event_queue_drain(id)` — drop all pending events.
#[must_use]
pub fn sys_event_queue_drain<R: EventRegistry + ?Sized>(
    reg: &mut R,
    id: u32,
) -> Result<(), CellError> {
    reg.queue_drain(id)
}

/// `sys_event_port_create(id_out, port_type, name)`.
#[must_use]
pub fn sys_event_port_create<R: EventRegistry + ?Sized>(
    reg: &mut R,
    port_type: u32,
    name: u64,
) -> Result<u32, CellError> {
    if !matches!(port_type, PORT_LOCAL | PORT_IPC) {
        return Err(CellError::EINVAL);
    }
    reg.port_create(port_type, name)
}

/// `sys_event_port_destroy(id)`.
#[must_use]
pub fn sys_event_port_destroy<R: EventRegistry + ?Sized>(
    reg: &mut R,
    id: u32,
) -> Result<(), CellError> {
    reg.port_destroy(id)
}

/// `sys_event_port_connect_local(port, queue)`.
#[must_use]
pub fn sys_event_port_connect_local<R: EventRegistry + ?Sized>(
    reg: &mut R,
    port: u32,
    queue: u32,
) -> Result<(), CellError> {
    reg.port_connect_local(port, queue)
}

/// `sys_event_port_disconnect(port)`.
#[must_use]
pub fn sys_event_port_disconnect<R: EventRegistry + ?Sized>(
    reg: &mut R,
    port: u32,
) -> Result<(), CellError> {
    reg.port_disconnect(port)
}

/// `sys_event_port_send(port, data1, data2, data3)`.
#[must_use]
pub fn sys_event_port_send<R: EventRegistry + ?Sized>(
    reg: &mut R,
    port: u32,
    data1: u64,
    data2: u64,
    data3: u64,
) -> Result<(), CellError> {
    reg.port_send(port, data1, data2, data3)
}

// =====================================================================
// Tests — with an in-memory reference registry
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, VecDeque};

    #[allow(dead_code)]
    struct Queue {
        attr: QueueAttr,
        size: u32,
        pending: VecDeque<Event>,
    }

    struct Port {
        port_type: u32,
        name: u64,
        /// Queue this port is connected to (local mode only).
        connected_queue: Option<u32>,
    }

    #[derive(Default)]
    struct TestRegistry {
        queues: HashMap<u32, Queue>,
        ports: HashMap<u32, Port>,
        next_id: u32,
    }

    impl TestRegistry {
        fn alloc_id(&mut self) -> u32 {
            self.next_id += 1;
            self.next_id
        }
    }

    impl EventRegistry for TestRegistry {
        fn queue_create(&mut self, attr: QueueAttr, size: u32) -> Result<u32, CellError> {
            let id = self.alloc_id();
            self.queues.insert(
                id,
                Queue { attr, size, pending: VecDeque::new() },
            );
            Ok(id)
        }
        fn queue_destroy(&mut self, id: u32, mode: u32) -> Result<(), CellError> {
            let q = self.queues.get(&id).ok_or(CellError::ESRCH)?;
            if !q.pending.is_empty() && mode == QUEUE_DESTROY_WAIT {
                return Err(CellError::EBUSY);
            }
            self.queues.remove(&id);
            Ok(())
        }
        fn queue_receive(&mut self, id: u32) -> Result<ReceiveOutcome, CellError> {
            let q = self.queues.get_mut(&id).ok_or(CellError::ESRCH)?;
            match q.pending.pop_front() {
                Some(ev) => Ok(ReceiveOutcome::Received(ev)),
                None => Ok(ReceiveOutcome::MustBlock),
            }
        }
        fn queue_tryreceive(&mut self, id: u32, max: u32) -> Result<Vec<Event>, CellError> {
            let q = self.queues.get_mut(&id).ok_or(CellError::ESRCH)?;
            let n = (max as usize).min(q.pending.len());
            Ok((0..n).map(|_| q.pending.pop_front().unwrap()).collect())
        }
        fn queue_drain(&mut self, id: u32) -> Result<(), CellError> {
            let q = self.queues.get_mut(&id).ok_or(CellError::ESRCH)?;
            q.pending.clear();
            Ok(())
        }
        fn port_create(&mut self, port_type: u32, name: u64) -> Result<u32, CellError> {
            let id = self.alloc_id();
            self.ports.insert(id, Port { port_type, name, connected_queue: None });
            Ok(id)
        }
        fn port_destroy(&mut self, id: u32) -> Result<(), CellError> {
            let p = self.ports.get(&id).ok_or(CellError::ESRCH)?;
            if p.connected_queue.is_some() {
                return Err(CellError::EISCONN);
            }
            self.ports.remove(&id);
            Ok(())
        }
        fn port_connect_local(&mut self, port: u32, queue: u32) -> Result<(), CellError> {
            if !self.queues.contains_key(&queue) {
                return Err(CellError::ESRCH);
            }
            let p = self.ports.get_mut(&port).ok_or(CellError::ESRCH)?;
            if p.port_type != PORT_LOCAL {
                return Err(CellError::EINVAL);
            }
            if p.connected_queue.is_some() {
                return Err(CellError::EISCONN);
            }
            p.connected_queue = Some(queue);
            Ok(())
        }
        fn port_disconnect(&mut self, port: u32) -> Result<(), CellError> {
            let p = self.ports.get_mut(&port).ok_or(CellError::ESRCH)?;
            if p.connected_queue.is_none() {
                return Err(CellError::ENOTCONN);
            }
            p.connected_queue = None;
            Ok(())
        }
        fn port_send(
            &mut self,
            port: u32,
            data1: u64,
            data2: u64,
            data3: u64,
        ) -> Result<(), CellError> {
            let p = self.ports.get(&port).ok_or(CellError::ESRCH)?;
            let queue = p.connected_queue.ok_or(CellError::ENOTCONN)?;
            let source = p.name; // port name serves as event source
            let q = self.queues.get_mut(&queue).ok_or(CellError::ESRCH)?;
            if q.pending.len() as u32 >= q.size {
                return Err(CellError::EBUSY); // queue full
            }
            q.pending.push_back(Event { source, data1, data2, data3 });
            Ok(())
        }
    }

    // -- Queue lifecycle -----------------------------------------

    #[test]
    fn queue_create_returns_id() {
        let mut r = TestRegistry::default();
        let id = sys_event_queue_create(&mut r, QueueAttr::default(), 0, 16).unwrap();
        assert!(id > 0);
    }

    #[test]
    fn queue_create_rejects_bad_protocol() {
        let mut r = TestRegistry::default();
        assert_eq!(
            sys_event_queue_create(
                &mut r,
                QueueAttr { protocol: 0x99, queue_type: QUEUE_PPU },
                0,
                16,
            ),
            Err(CellError::EINVAL)
        );
    }

    #[test]
    fn queue_create_rejects_zero_or_oversized() {
        let mut r = TestRegistry::default();
        assert_eq!(
            sys_event_queue_create(&mut r, QueueAttr::default(), 0, 0),
            Err(CellError::EINVAL)
        );
        assert_eq!(
            sys_event_queue_create(&mut r, QueueAttr::default(), 0, 200),
            Err(CellError::EINVAL)
        );
    }

    #[test]
    fn queue_destroy_bad_mode_is_einval() {
        let mut r = TestRegistry::default();
        let id = sys_event_queue_create(&mut r, QueueAttr::default(), 0, 8).unwrap();
        assert_eq!(
            sys_event_queue_destroy(&mut r, id, 99),
            Err(CellError::EINVAL)
        );
    }

    #[test]
    fn queue_destroy_unknown_is_esrch() {
        let mut r = TestRegistry::default();
        assert_eq!(
            sys_event_queue_destroy(&mut r, 999, QUEUE_DESTROY_WAIT),
            Err(CellError::ESRCH)
        );
    }

    // -- Port + Send + Receive -----------------------------------

    #[test]
    fn port_send_delivers_event_to_connected_queue() {
        let mut r = TestRegistry::default();
        let qid = sys_event_queue_create(&mut r, QueueAttr::default(), 0, 8).unwrap();
        let pid = sys_event_port_create(&mut r, PORT_LOCAL, 0xCAFE).unwrap();
        sys_event_port_connect_local(&mut r, pid, qid).unwrap();
        sys_event_port_send(&mut r, pid, 1, 2, 3).unwrap();

        let out = sys_event_queue_receive(&mut r, qid, 0).unwrap();
        match out {
            ReceiveOutcome::Received(ev) => {
                assert_eq!(ev.source, 0xCAFE);
                assert_eq!(ev.data1, 1);
                assert_eq!(ev.data2, 2);
                assert_eq!(ev.data3, 3);
            }
            _ => panic!("expected Received"),
        }
    }

    #[test]
    fn receive_on_empty_queue_returns_mustblock() {
        let mut r = TestRegistry::default();
        let qid = sys_event_queue_create(&mut r, QueueAttr::default(), 0, 8).unwrap();
        assert_eq!(
            sys_event_queue_receive(&mut r, qid, 0),
            Ok(ReceiveOutcome::MustBlock)
        );
    }

    #[test]
    fn tryreceive_drains_up_to_size_events() {
        let mut r = TestRegistry::default();
        let qid = sys_event_queue_create(&mut r, QueueAttr::default(), 0, 8).unwrap();
        let pid = sys_event_port_create(&mut r, PORT_LOCAL, 1).unwrap();
        sys_event_port_connect_local(&mut r, pid, qid).unwrap();
        for i in 0..4u64 {
            sys_event_port_send(&mut r, pid, i, 0, 0).unwrap();
        }
        let got = sys_event_queue_tryreceive(&mut r, qid, 2).unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].data1, 0);
        assert_eq!(got[1].data1, 1);
    }

    #[test]
    fn drain_removes_all_pending() {
        let mut r = TestRegistry::default();
        let qid = sys_event_queue_create(&mut r, QueueAttr::default(), 0, 8).unwrap();
        let pid = sys_event_port_create(&mut r, PORT_LOCAL, 1).unwrap();
        sys_event_port_connect_local(&mut r, pid, qid).unwrap();
        for _ in 0..3 {
            sys_event_port_send(&mut r, pid, 0, 0, 0).unwrap();
        }
        sys_event_queue_drain(&mut r, qid).unwrap();
        assert_eq!(
            sys_event_queue_receive(&mut r, qid, 0),
            Ok(ReceiveOutcome::MustBlock)
        );
    }

    #[test]
    fn queue_full_send_is_ebusy() {
        let mut r = TestRegistry::default();
        let qid = sys_event_queue_create(&mut r, QueueAttr::default(), 0, 2).unwrap();
        let pid = sys_event_port_create(&mut r, PORT_LOCAL, 1).unwrap();
        sys_event_port_connect_local(&mut r, pid, qid).unwrap();
        sys_event_port_send(&mut r, pid, 0, 0, 0).unwrap();
        sys_event_port_send(&mut r, pid, 0, 0, 0).unwrap();
        assert_eq!(
            sys_event_port_send(&mut r, pid, 0, 0, 0),
            Err(CellError::EBUSY)
        );
    }

    // -- Port state transitions ----------------------------------

    #[test]
    fn port_create_rejects_bad_type() {
        let mut r = TestRegistry::default();
        assert_eq!(
            sys_event_port_create(&mut r, 99, 0),
            Err(CellError::EINVAL)
        );
    }

    #[test]
    fn port_double_connect_is_eisconn() {
        let mut r = TestRegistry::default();
        let q1 = sys_event_queue_create(&mut r, QueueAttr::default(), 0, 4).unwrap();
        let q2 = sys_event_queue_create(&mut r, QueueAttr::default(), 0, 4).unwrap();
        let p = sys_event_port_create(&mut r, PORT_LOCAL, 0).unwrap();
        sys_event_port_connect_local(&mut r, p, q1).unwrap();
        assert_eq!(
            sys_event_port_connect_local(&mut r, p, q2),
            Err(CellError::EISCONN)
        );
    }

    #[test]
    fn port_disconnect_then_reconnect_works() {
        let mut r = TestRegistry::default();
        let q1 = sys_event_queue_create(&mut r, QueueAttr::default(), 0, 4).unwrap();
        let q2 = sys_event_queue_create(&mut r, QueueAttr::default(), 0, 4).unwrap();
        let p = sys_event_port_create(&mut r, PORT_LOCAL, 0).unwrap();
        sys_event_port_connect_local(&mut r, p, q1).unwrap();
        sys_event_port_disconnect(&mut r, p).unwrap();
        sys_event_port_connect_local(&mut r, p, q2).unwrap();
    }

    #[test]
    fn port_disconnect_when_not_connected_is_enotconn() {
        let mut r = TestRegistry::default();
        let p = sys_event_port_create(&mut r, PORT_LOCAL, 0).unwrap();
        assert_eq!(
            sys_event_port_disconnect(&mut r, p),
            Err(CellError::ENOTCONN)
        );
    }

    #[test]
    fn send_on_disconnected_port_is_enotconn() {
        let mut r = TestRegistry::default();
        let p = sys_event_port_create(&mut r, PORT_LOCAL, 0).unwrap();
        assert_eq!(
            sys_event_port_send(&mut r, p, 0, 0, 0),
            Err(CellError::ENOTCONN)
        );
    }

    #[test]
    fn port_destroy_while_connected_is_eisconn() {
        let mut r = TestRegistry::default();
        let q = sys_event_queue_create(&mut r, QueueAttr::default(), 0, 4).unwrap();
        let p = sys_event_port_create(&mut r, PORT_LOCAL, 0).unwrap();
        sys_event_port_connect_local(&mut r, p, q).unwrap();
        assert_eq!(
            sys_event_port_destroy(&mut r, p),
            Err(CellError::EISCONN)
        );
    }

    // -- Constants frozen ---------------------------------------

    #[test]
    fn constants_frozen() {
        assert_eq!(QUEUE_PPU, 1);
        assert_eq!(QUEUE_SPU, 2);
        assert_eq!(PROTOCOL_FIFO, 0x01);
        assert_eq!(PROTOCOL_PRIORITY, 0x02);
        assert_eq!(PORT_LOCAL, 1);
        assert_eq!(PORT_IPC, 3);
        assert_eq!(QUEUE_DESTROY_WAIT, 0);
        assert_eq!(QUEUE_DESTROY_FORCE, 1);
    }
}
