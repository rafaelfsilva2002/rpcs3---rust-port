//! Rust port of `rpcs3/Emu/Cell/lv2/sys_interrupt.cpp` — PS3 LV2 interrupt
//! tag/handler syscalls (4 entries, 303 lines C++).
//!
//! 4 entries:
//! * `sys_interrupt_tag_destroy(intrtag)` — destroys an interrupt tag (cpp:102-130).
//!   Validates: tag exists else ESRCH; tag has handler attached else EBUSY.
//! * `_sys_interrupt_thread_establish(ih, intrtag, intrthread, arg1, arg2)` —
//!   binds an interrupt handler to a tag (cpp:132-199). Validation order
//!   preserved EXATA: tag exists → thread exists → thread not running →
//!   tag has no handler → make handler.
//! * `_sys_interrupt_thread_disestablish(ih, r13)` — unbinds + joins (cpp:201-235).
//!   Tries withdraw handler first; if not found, tries to withdraw thread
//!   directly (returns r13=gpr[13]).
//! * `sys_interrupt_thread_eoi()` — end-of-interrupt sentinel (cpp:237-248).
//!   Sets `cpu_flag::ret` + sleep + clears `interrupt_thread_executing`.
//!
//! `no_std` + `alloc`. Single dep: `rpcs3-emu-types`.

#![no_std]

extern crate alloc;

use alloc::vec::Vec;
use rpcs3_emu_types::CellError;

pub const MODULE_NAME: &str = "sys_interrupt";

pub const REGISTERED_ENTRY_POINTS: &[&str] = &[
    "sys_interrupt_tag_destroy",
    "_sys_interrupt_thread_establish",
    "_sys_interrupt_thread_disestablish",
    "sys_interrupt_thread_eoi",
];

pub const CELL_ESRCH: CellError = CellError(0x8001_0005);
pub const CELL_EBUSY: CellError = CellError(0x8001_000A);
pub const CELL_EAGAIN: CellError = CellError(0x8001_000B);
pub const CELL_ESTAT: CellError = CellError(0x8001_000F);

/// Mirror of `lv2_int_tag` — tracks attached handler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IntTag {
    pub id: u32,
    /// `Some(handler_id)` when an interrupt handler is bound.
    pub handler: Option<u32>,
}

/// Mirror of `lv2_int_serv` — interrupt service routine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IntServ {
    pub id: u32,
    pub thread_id: u32,
    pub arg1: u64,
    pub arg2: u64,
    /// Mirrors `interrupt_thread_executing` flag on ppu_thread.
    pub executing: bool,
}

/// Per-thread state needed for interrupt operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThreadState {
    pub id: u32,
    /// Inverse of `cpu_flag::stop` — true means thread has been started.
    pub running: bool,
    /// gpr[13] for TLS save (returned by disestablish).
    pub gpr_13: u64,
    /// Whether the thread is still alive in idm.
    pub alive: bool,
}

/// Outcome of `_sys_interrupt_thread_disestablish`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisestablishOutcome {
    /// Withdrew an interrupt handler — joined and saved tls.
    HandlerJoined { saved_r13: u64 },
    /// Withdrew a raw thread (no handler bound to ih) — saved tls.
    ThreadJoined { saved_r13: u64 },
}

#[derive(Debug, Default)]
pub struct SysInterrupt {
    pub tags: Vec<IntTag>,
    pub servs: Vec<IntServ>,
    pub threads: Vec<ThreadState>,
    pub next_tag_id: u32,
    pub next_serv_id: u32,

    pub tag_destroy_calls: u64,
    pub thread_establish_calls: u64,
    pub thread_disestablish_calls: u64,
    pub thread_eoi_calls: u64,
}

impl SysInterrupt {
    pub fn new() -> Self {
        Self {
            next_tag_id: 1,
            next_serv_id: 1,
            ..Default::default()
        }
    }

    /// Test/scaffold helper: register a thread with the kernel.
    pub fn register_thread(&mut self, id: u32, running: bool, gpr_13: u64) {
        self.threads.push(ThreadState {
            id,
            running,
            gpr_13,
            alive: true,
        });
    }

    /// Test/scaffold helper: create a new interrupt tag (no handler).
    pub fn alloc_tag(&mut self) -> u32 {
        let id = self.next_tag_id;
        self.next_tag_id = self.next_tag_id.wrapping_add(1);
        self.tags.push(IntTag { id, handler: None });
        id
    }

    fn find_tag(&self, id: u32) -> Option<usize> {
        self.tags.iter().position(|t| t.id == id)
    }

    fn find_serv(&self, id: u32) -> Option<usize> {
        self.servs.iter().position(|s| s.id == id)
    }

    fn find_thread(&self, id: u32) -> Option<usize> {
        self.threads.iter().position(|t| t.id == id && t.alive)
    }

    /// `sys_interrupt_tag_destroy(intrtag)` — cpp:102-130.
    /// EBUSY if handler attached, ESRCH if tag missing.
    pub fn tag_destroy(&mut self, intrtag: u32) -> Result<(), CellError> {
        self.tag_destroy_calls = self.tag_destroy_calls.saturating_add(1);
        let pos = self.find_tag(intrtag).ok_or(CELL_ESRCH)?;
        if self.tags[pos].handler.is_some() {
            return Err(CELL_EBUSY);
        }
        self.tags.remove(pos);
        Ok(())
    }

    /// `_sys_interrupt_thread_establish(ih, intrtag, intrthread, arg1, arg2)`
    /// — cpp:132-199. Validation order EXATA: tag → thread → !running → !handler.
    pub fn thread_establish(
        &mut self,
        intrtag: u32,
        intrthread: u32,
        arg1: u64,
        arg2: u64,
        ih_out: Option<&mut u32>,
    ) -> Result<(), CellError> {
        self.thread_establish_calls = self.thread_establish_calls.saturating_add(1);
        // (1) Tag must exist.
        let tag_pos = self.find_tag(intrtag).ok_or(CELL_ESRCH)?;
        // (2) Thread must exist.
        let thread_pos = self.find_thread(intrthread).ok_or(CELL_ESRCH)?;
        // (3) Thread must NOT already be running (else already-established).
        if self.threads[thread_pos].running {
            return Err(CELL_EAGAIN);
        }
        // (4) Tag must NOT already have a handler.
        if self.tags[tag_pos].handler.is_some() {
            return Err(CELL_ESTAT);
        }
        // (5) Make handler + bind to tag + start thread.
        let serv_id = self.next_serv_id;
        self.next_serv_id = self.next_serv_id.wrapping_add(1);
        self.servs.push(IntServ {
            id: serv_id,
            thread_id: intrthread,
            arg1,
            arg2,
            executing: false,
        });
        self.tags[tag_pos].handler = Some(serv_id);
        self.threads[thread_pos].running = true;
        if let Some(slot) = ih_out {
            *slot = serv_id;
        }
        Ok(())
    }

    /// `_sys_interrupt_thread_disestablish(ih, r13)` — cpp:201-235.
    /// Tries withdraw handler first; if not found, withdraws thread directly.
    pub fn thread_disestablish(&mut self, ih: u32) -> Result<DisestablishOutcome, CellError> {
        self.thread_disestablish_calls = self.thread_disestablish_calls.saturating_add(1);

        // First path: withdraw handler.
        if let Some(serv_pos) = self.find_serv(ih) {
            let serv = self.servs[serv_pos];
            // Find the thread to recover gpr[13].
            let r13 = self
                .threads
                .iter()
                .find(|t| t.id == serv.thread_id && t.alive)
                .map(|t| t.gpr_13)
                .unwrap_or(0);
            self.servs.remove(serv_pos);
            // Unbind from any tag pointing here.
            for t in self.tags.iter_mut() {
                if t.handler == Some(ih) {
                    t.handler = None;
                }
            }
            // Mark thread finished (cpp:97 `thread_state::finished`).
            for t in self.threads.iter_mut() {
                if t.id == serv.thread_id {
                    t.alive = false;
                }
            }
            return Ok(DisestablishOutcome::HandlerJoined { saved_r13: r13 });
        }

        // Second path: withdraw raw thread directly.
        if let Some(thread_pos) = self.find_thread(ih) {
            let r13 = self.threads[thread_pos].gpr_13;
            self.threads[thread_pos].alive = false;
            return Ok(DisestablishOutcome::ThreadJoined { saved_r13: r13 });
        }

        Err(CELL_ESRCH)
    }

    /// `sys_interrupt_thread_eoi()` — cpp:237-248. Models the side-effect of
    /// clearing the `interrupt_thread_executing` flag for the calling thread.
    /// In a real port this would be wired via thread context lookup; here we
    /// expose it as a `clear_executing(thread_id)` helper.
    pub fn thread_eoi(&mut self, calling_thread_id: u32) -> Result<(), CellError> {
        self.thread_eoi_calls = self.thread_eoi_calls.saturating_add(1);
        for s in self.servs.iter_mut() {
            if s.thread_id == calling_thread_id {
                s.executing = false;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_and_entries() {
        assert_eq!(MODULE_NAME, "sys_interrupt");
        assert_eq!(REGISTERED_ENTRY_POINTS.len(), 4);
    }

    #[test]
    fn errors_byte_exact() {
        assert_eq!(CELL_ESRCH.0, 0x8001_0005);
        assert_eq!(CELL_EBUSY.0, 0x8001_000A);
        assert_eq!(CELL_EAGAIN.0, 0x8001_000B);
        assert_eq!(CELL_ESTAT.0, 0x8001_000F);
    }

    #[test]
    fn tag_destroy_unknown_esrch() {
        let mut m = SysInterrupt::new();
        assert_eq!(m.tag_destroy(99), Err(CELL_ESRCH));
    }

    #[test]
    fn tag_destroy_with_handler_ebusy() {
        let mut m = SysInterrupt::new();
        let tag = m.alloc_tag();
        m.register_thread(100, false, 0);
        m.thread_establish(tag, 100, 0, 0, None).unwrap();
        // Tag now has handler — destroy fails.
        assert_eq!(m.tag_destroy(tag), Err(CELL_EBUSY));
    }

    #[test]
    fn tag_destroy_unbound_succeeds() {
        let mut m = SysInterrupt::new();
        let tag = m.alloc_tag();
        m.tag_destroy(tag).unwrap();
        assert!(m.tags.is_empty());
    }

    #[test]
    fn establish_unknown_tag_esrch() {
        let mut m = SysInterrupt::new();
        m.register_thread(100, false, 0);
        assert_eq!(
            m.thread_establish(99, 100, 0, 0, None),
            Err(CELL_ESRCH)
        );
    }

    #[test]
    fn establish_unknown_thread_esrch() {
        let mut m = SysInterrupt::new();
        let tag = m.alloc_tag();
        assert_eq!(
            m.thread_establish(tag, 99, 0, 0, None),
            Err(CELL_ESRCH)
        );
    }

    #[test]
    fn establish_running_thread_eagain() {
        // cpp:163-167 — if interrupt thread is running, already established elsewhere.
        let mut m = SysInterrupt::new();
        let tag = m.alloc_tag();
        m.register_thread(100, true, 0); // already running
        assert_eq!(
            m.thread_establish(tag, 100, 0, 0, None),
            Err(CELL_EAGAIN)
        );
    }

    #[test]
    fn establish_already_handled_estat() {
        // cpp:170-174 — tag already has a handler.
        let mut m = SysInterrupt::new();
        let tag = m.alloc_tag();
        m.register_thread(100, false, 0);
        m.register_thread(101, false, 0);
        m.thread_establish(tag, 100, 0, 0, None).unwrap();
        // Second handler on same tag → ESTAT.
        assert_eq!(
            m.thread_establish(tag, 101, 0, 0, None),
            Err(CELL_ESTAT)
        );
    }

    #[test]
    fn establish_succeeds_and_writes_ih() {
        let mut m = SysInterrupt::new();
        let tag = m.alloc_tag();
        m.register_thread(100, false, 0xCAFE_BABE);
        let mut ih = 0u32;
        m.thread_establish(tag, 100, 0xAA, 0xBB, Some(&mut ih)).unwrap();
        assert_eq!(ih, 1);
        // Tag now has handler.
        let t = &m.tags[0];
        assert_eq!(t.handler, Some(1));
        // Thread now running.
        assert!(m.threads[0].running);
        // Serv stored args.
        let s = &m.servs[0];
        assert_eq!(s.thread_id, 100);
        assert_eq!(s.arg1, 0xAA);
        assert_eq!(s.arg2, 0xBB);
    }

    #[test]
    fn disestablish_handler_path_returns_r13() {
        let mut m = SysInterrupt::new();
        let tag = m.alloc_tag();
        m.register_thread(100, false, 0xDEAD_BEEF);
        let mut ih = 0u32;
        m.thread_establish(tag, 100, 0, 0, Some(&mut ih)).unwrap();
        let outcome = m.thread_disestablish(ih).unwrap();
        assert_eq!(outcome, DisestablishOutcome::HandlerJoined { saved_r13: 0xDEAD_BEEF });
        // Handler removed.
        assert!(m.servs.is_empty());
        // Tag unbound.
        assert!(m.tags[0].handler.is_none());
        // Thread marked dead.
        assert!(!m.threads[0].alive);
    }

    #[test]
    fn disestablish_thread_path_when_no_handler() {
        // cpp:213-221 — if no handler with that ih, try withdraw raw thread.
        let mut m = SysInterrupt::new();
        m.register_thread(50, false, 0xABCD_1234);
        let outcome = m.thread_disestablish(50).unwrap();
        assert_eq!(outcome, DisestablishOutcome::ThreadJoined { saved_r13: 0xABCD_1234 });
        assert!(!m.threads[0].alive);
    }

    #[test]
    fn disestablish_unknown_esrch() {
        let mut m = SysInterrupt::new();
        assert_eq!(m.thread_disestablish(99), Err(CELL_ESRCH));
    }

    #[test]
    fn eoi_clears_executing_flag() {
        let mut m = SysInterrupt::new();
        let tag = m.alloc_tag();
        m.register_thread(100, false, 0);
        m.thread_establish(tag, 100, 0, 0, None).unwrap();
        // Simulate interrupt firing.
        m.servs[0].executing = true;
        m.thread_eoi(100).unwrap();
        assert!(!m.servs[0].executing);
    }

    #[test]
    fn eoi_no_op_for_unknown_thread() {
        let mut m = SysInterrupt::new();
        m.thread_eoi(999).unwrap();
        assert_eq!(m.thread_eoi_calls, 1);
    }

    #[test]
    fn full_interrupt_lifecycle_smoke() {
        let mut m = SysInterrupt::new();
        // Setup: 1 tag + 1 thread (created stopped).
        let tag = m.alloc_tag();
        m.register_thread(100, false, 0xCAFE_F00D);

        // Establish handler.
        let mut ih = 0u32;
        m.thread_establish(tag, 100, 0x11, 0x22, Some(&mut ih)).unwrap();
        assert_eq!(ih, 1);
        // Thread running, tag bound.
        assert!(m.threads[0].running);
        assert_eq!(m.tags[0].handler, Some(ih));
        // Cannot destroy tag — busy.
        assert_eq!(m.tag_destroy(tag), Err(CELL_EBUSY));

        // Simulate interrupt firing + EOI.
        m.servs[0].executing = true;
        m.thread_eoi(100).unwrap();
        assert!(!m.servs[0].executing);

        // Disestablish handler.
        let outcome = m.thread_disestablish(ih).unwrap();
        assert_eq!(outcome, DisestablishOutcome::HandlerJoined { saved_r13: 0xCAFE_F00D });

        // Now tag can be destroyed.
        m.tag_destroy(tag).unwrap();
        assert!(m.tags.is_empty());
    }
}
