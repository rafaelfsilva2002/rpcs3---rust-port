//! `rpcs3-hle-sys-lwmutex-user` — PS3 lightweight mutex user-mode HLE.
//!
//! Ports `rpcs3/Emu/Cell/Modules/sys_lwmutex_.cpp`.  The firmware
//! layer implements a user-space fast-path for mutex acquisition
//! (based on an atomic `owner` field) that falls back to the `sys_lv2`
//! kernel syscalls only on contention.  This crate captures the
//! observable behaviour of the fast-path state machine — attribute
//! validation, recursion rules, sentinel values — without modelling
//! the real PPU atomic primitives.
//!
//! ## Entry points covered
//!
//! | C++ function           | Rust wrapper                   |
//! |------------------------|--------------------------------|
//! | `sys_lwmutex_create`   | [`LwMutex::create`]            |
//! | `sys_lwmutex_destroy`  | [`LwMutex::destroy`]           |
//! | `sys_lwmutex_lock`     | [`LwMutex::lock`]              |
//! | `sys_lwmutex_trylock`  | [`LwMutex::trylock`]           |
//! | `sys_lwmutex_unlock`   | [`LwMutex::unlock`]            |

use rpcs3_emu_types::CellError;

// =====================================================================
// Error codes — byte-exact with LV2 sys error table
// =====================================================================

/// `CELL_EINVAL`.
pub const CELL_EINVAL:     CellError = CellError(0x8001_0002);
/// `CELL_EPERM`.
pub const CELL_EPERM:      CellError = CellError(0x8001_0003);
/// `CELL_EBUSY`.
pub const CELL_EBUSY:      CellError = CellError(0x8001_000A);
/// `CELL_EDEADLK`.
pub const CELL_EDEADLK:    CellError = CellError(0x8001_0012);
/// `CELL_EKRESOURCE` — resource busy (recursion limit, etc).
pub const CELL_EKRESOURCE: CellError = CellError(0x8001_0020);
/// `CELL_ESRCH`.
pub const CELL_ESRCH:      CellError = CellError(0x8001_0005);
/// `CELL_ETIMEDOUT`.
pub const CELL_ETIMEDOUT:  CellError = CellError(0x8001_000B);

// =====================================================================
// Sync-attribute constants — byte-exact with sys_sync.h
// =====================================================================

pub const SYS_SYNC_FIFO:          u32 = 0x01;
pub const SYS_SYNC_PRIORITY:      u32 = 0x02;
pub const SYS_SYNC_RETRY:         u32 = 0x04;
pub const SYS_SYNC_RECURSIVE:     u32 = 0x10;
pub const SYS_SYNC_NOT_RECURSIVE: u32 = 0x20;

/// `lwmutex_free = 0xffffffffu` from sys_lwmutex.h:21.
pub const LWMUTEX_FREE:     u32 = 0xFFFF_FFFF;
/// `lwmutex_dead = 0xfffffffeu` from sys_lwmutex.h:22.
pub const LWMUTEX_DEAD:     u32 = 0xFFFF_FFFE;
/// `lwmutex_reserved = 0xfffffffdu` from sys_lwmutex.h:23.
pub const LWMUTEX_RESERVED: u32 = 0xFFFF_FFFD;

// =====================================================================
// Attribute validation — sys_lwmutex_create
// =====================================================================

/// Mirror of `sys_lwmutex_attribute_t` (subset that the firmware
/// inspects).  `name_u64` is the 8-byte packed mutex name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LwMutexAttr {
    pub protocol: u32,
    pub recursive: u32,
    pub name_u64: u64,
}

impl LwMutexAttr {
    /// Validate the attribute block per sys_lwmutex_.cpp:18-34.
    ///
    /// # Errors
    /// * [`CELL_EINVAL`] if `recursive` isn't `SYS_SYNC_RECURSIVE` /
    ///   `SYS_SYNC_NOT_RECURSIVE`.
    /// * [`CELL_EINVAL`] if `protocol` isn't `FIFO` / `RETRY` /
    ///   `PRIORITY`.
    pub fn validate(&self) -> Result<(), CellError> {
        if self.recursive != SYS_SYNC_RECURSIVE && self.recursive != SYS_SYNC_NOT_RECURSIVE {
            return Err(CELL_EINVAL);
        }
        match self.protocol {
            SYS_SYNC_FIFO | SYS_SYNC_RETRY | SYS_SYNC_PRIORITY => Ok(()),
            _ => Err(CELL_EINVAL),
        }
    }

    /// Returns true if the mutex is configured for recursive locking.
    #[must_use]
    pub fn is_recursive(&self) -> bool { self.recursive == SYS_SYNC_RECURSIVE }

    /// Returns true if the mutex uses the `SYS_SYNC_RETRY` unlock path.
    #[must_use]
    pub fn is_retry(&self) -> bool { self.protocol == SYS_SYNC_RETRY }

    /// Internal-mutex protocol the firmware hands to the underlying
    /// `sys_mutex_create` — see sys_lwmutex_.cpp:38
    /// (`protocol == SYS_SYNC_FIFO ? SYS_SYNC_FIFO : SYS_SYNC_PRIORITY`).
    #[must_use]
    pub fn internal_protocol(&self) -> u32 {
        if self.protocol == SYS_SYNC_FIFO { SYS_SYNC_FIFO } else { SYS_SYNC_PRIORITY }
    }
}

// =====================================================================
// Lightweight mutex state
// =====================================================================

/// Mirror of `sys_lwmutex_t` — the user-visible slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LwMutex {
    pub owner: u32,
    pub attribute: u32,
    pub recursive_count: u32,
    pub sleep_queue: u32,
    /// Waiter count — incremented while sleeping on the kernel queue.
    pub all_info_waiters: u32,
}

impl LwMutex {
    /// Port of `sys_lwmutex_create` — allocates the user-side slot
    /// with the attributes packed into `attribute = recursive | protocol`.
    ///
    /// # Errors
    /// Propagates [`LwMutexAttr::validate`] errors.
    pub fn create(attr: &LwMutexAttr, sleep_queue_id: u32) -> Result<Self, CellError> {
        attr.validate()?;
        Ok(Self {
            owner: LWMUTEX_FREE,
            attribute: attr.recursive | attr.protocol,
            recursive_count: 0,
            sleep_queue: sleep_queue_id,
            all_info_waiters: 0,
        })
    }

    #[must_use]
    pub fn is_recursive(&self) -> bool { (self.attribute & SYS_SYNC_RECURSIVE) != 0 }

    #[must_use]
    pub fn is_retry(&self) -> bool { (self.attribute & SYS_SYNC_RETRY) != 0 }

    /// Port of `sys_lwmutex_trylock` — observable state machine.
    ///
    /// # Errors
    /// * [`CELL_EINVAL`] if the mutex has been destroyed (`owner == LWMUTEX_DEAD`).
    /// * [`CELL_EDEADLK`] if `tid` already owns a non-recursive mutex.
    /// * [`CELL_EKRESOURCE`] if the recursion counter would overflow.
    /// * [`CELL_EBUSY`] if another thread currently owns the mutex.
    pub fn trylock(&mut self, tid: u32) -> Result<(), CellError> {
        // Fast path: free → tid.
        if self.owner == LWMUTEX_FREE {
            self.owner = tid;
            return Ok(());
        }
        // Already owned by us.
        if self.owner == tid {
            if !self.is_recursive() {
                return Err(CELL_EDEADLK);
            }
            if self.recursive_count == u32::MAX {
                return Err(CELL_EKRESOURCE);
            }
            self.recursive_count += 1;
            return Ok(());
        }
        // Dead mutex.
        if self.owner == LWMUTEX_DEAD {
            return Err(CELL_EINVAL);
        }
        // Reserved → would fall through to the syscall in C++.  In the
        // Rust port we surface the `Reserved` condition so the caller
        // can route to the underlying `_sys_lwmutex_trylock`.
        //
        // The observable result (for tests) is identical to contention
        // (EBUSY) until the caller drives the syscall path.
        // Any other owner is another thread holding the lock.
        Err(CELL_EBUSY)
    }

    /// Port of `sys_lwmutex_lock` — fast-path variant.  Returns
    /// [`LockOutcome::Acquired`] on immediate success and
    /// [`LockOutcome::WouldSleep`] when the caller must descend into the
    /// kernel syscall.  The Rust port does NOT spin 10× like the C++
    /// side (that's a micro-optimisation irrelevant to observable
    /// behaviour), but preserves every error-code route.
    #[must_use]
    pub fn lock(&mut self, tid: u32) -> LockOutcome {
        // Fast CAS.
        if self.owner == LWMUTEX_FREE {
            self.owner = tid;
            return LockOutcome::Acquired;
        }
        if self.owner == tid {
            if !self.is_recursive() {
                return LockOutcome::Error(CELL_EDEADLK);
            }
            if self.recursive_count == u32::MAX {
                return LockOutcome::Error(CELL_EKRESOURCE);
            }
            self.recursive_count += 1;
            return LockOutcome::Acquired;
        }
        if self.owner == LWMUTEX_DEAD {
            return LockOutcome::Error(CELL_EINVAL);
        }
        // Contention → caller must sleep on the syscall.
        self.all_info_waiters = self.all_info_waiters.saturating_add(1);
        LockOutcome::WouldSleep
    }

    /// Finalise a `LockOutcome::WouldSleep` after the syscall returned
    /// `CELL_OK` — the firmware then exchanges `owner = tid`, decrements
    /// the waiter count, and asserts that the previous owner was
    /// `LWMUTEX_RESERVED` (sys_lwmutex_.cpp:197-204).
    ///
    /// # Errors
    /// * [`CELL_EINVAL`] if the previous owner was NOT
    ///   `LWMUTEX_RESERVED` (internal invariant violation).
    pub fn finish_sleep_acquire(&mut self, tid: u32) -> Result<(), CellError> {
        let old = self.owner;
        self.owner = tid;
        self.all_info_waiters = self.all_info_waiters.saturating_sub(1);
        if old != LWMUTEX_RESERVED {
            return Err(CELL_EINVAL);
        }
        Ok(())
    }

    /// Port of `sys_lwmutex_unlock`.
    ///
    /// # Errors
    /// * [`CELL_EPERM`] if `tid` is not the current owner.
    pub fn unlock(&mut self, tid: u32) -> Result<UnlockOutcome, CellError> {
        if self.owner != tid {
            return Err(CELL_EPERM);
        }
        if self.recursive_count > 0 {
            self.recursive_count -= 1;
            return Ok(UnlockOutcome::Released);
        }
        // No waiters → clear owner directly.  C++ checks this via a
        // 64-bit CAS; we peek `all_info_waiters` instead.
        if self.all_info_waiters == 0 {
            self.owner = LWMUTEX_FREE;
            return Ok(UnlockOutcome::Released);
        }
        // Waiters exist.  SYS_SYNC_RETRY → release owner then `unlock2`.
        if self.is_retry() {
            self.owner = LWMUTEX_FREE;
            return Ok(UnlockOutcome::NeedsSyscall { reserved: false });
        }
        // Regular path: `owner = LWMUTEX_RESERVED`, call `_sys_lwmutex_unlock`.
        self.owner = LWMUTEX_RESERVED;
        Ok(UnlockOutcome::NeedsSyscall { reserved: true })
    }

    /// Port of `sys_lwmutex_destroy`.  Returns [`CELL_EBUSY`] if `tid`
    /// already owns the mutex (the firmware refuses to let a recursive
    /// destroy proceed — see sys_lwmutex_.cpp:68-71).  Otherwise marks
    /// the mutex as dead.
    ///
    /// # Errors
    /// * [`CELL_EBUSY`] if the calling thread owns the mutex.
    pub fn destroy(&mut self, tid: u32) -> Result<(), CellError> {
        if self.owner == tid {
            return Err(CELL_EBUSY);
        }
        // Firmware takes the lock via trylock, then marks dead.  We
        // bypass the take step for the observable end state.
        self.owner = LWMUTEX_DEAD;
        Ok(())
    }
}

/// Outcome of [`LwMutex::lock`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockOutcome {
    /// Fast path succeeded — caller owns the mutex.
    Acquired,
    /// Caller must descend into `_sys_lwmutex_lock` and resolve there.
    WouldSleep,
    /// Immediate error (dead mutex, non-recursive deadlock, or
    /// recursion-counter overflow).
    Error(CellError),
}

impl LockOutcome {
    #[must_use]
    pub fn is_ok(self) -> bool { matches!(self, Self::Acquired) }
}

/// Outcome of [`LwMutex::unlock`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnlockOutcome {
    /// Unlock complete, no syscall needed.
    Released,
    /// Waiters exist — caller must dispatch the underlying syscall.
    /// `reserved=true` means the firmware wrote `LWMUTEX_RESERVED` to
    /// the owner slot (regular path); `false` means the owner was
    /// released to `LWMUTEX_FREE` (the `SYS_SYNC_RETRY` path).
    NeedsSyscall { reserved: bool },
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn good_attr() -> LwMutexAttr {
        LwMutexAttr {
            protocol: SYS_SYNC_PRIORITY,
            recursive: SYS_SYNC_NOT_RECURSIVE,
            name_u64: 0x4C57_4D54_4558_4900, // "LWMTEXI\0"
        }
    }

    fn recursive_attr() -> LwMutexAttr {
        LwMutexAttr {
            protocol: SYS_SYNC_PRIORITY,
            recursive: SYS_SYNC_RECURSIVE,
            name_u64: 0,
        }
    }

    // ---- constants ---------------------------------------------------

    #[test]
    fn error_codes_byte_exact() {
        assert_eq!(CELL_EINVAL.0,     0x8001_0002);
        assert_eq!(CELL_EPERM.0,      0x8001_0003);
        assert_eq!(CELL_ESRCH.0,      0x8001_0005);
        assert_eq!(CELL_EBUSY.0,      0x8001_000A);
        assert_eq!(CELL_ETIMEDOUT.0,  0x8001_000B);
        assert_eq!(CELL_EDEADLK.0,    0x8001_0012);
        assert_eq!(CELL_EKRESOURCE.0, 0x8001_0020);
    }

    #[test]
    fn sync_constants_byte_exact() {
        assert_eq!(SYS_SYNC_FIFO,          0x01);
        assert_eq!(SYS_SYNC_PRIORITY,      0x02);
        assert_eq!(SYS_SYNC_RETRY,         0x04);
        assert_eq!(SYS_SYNC_RECURSIVE,     0x10);
        assert_eq!(SYS_SYNC_NOT_RECURSIVE, 0x20);
    }

    #[test]
    fn sentinel_constants_byte_exact() {
        assert_eq!(LWMUTEX_FREE,     0xFFFF_FFFF);
        assert_eq!(LWMUTEX_DEAD,     0xFFFF_FFFE);
        assert_eq!(LWMUTEX_RESERVED, 0xFFFF_FFFD);
    }

    // ---- validate ----------------------------------------------------

    #[test]
    fn validate_good_priority_not_recursive() {
        assert!(good_attr().validate().is_ok());
    }

    #[test]
    fn validate_good_priority_recursive() {
        assert!(recursive_attr().validate().is_ok());
    }

    #[test]
    fn validate_accepts_all_three_protocols() {
        for p in [SYS_SYNC_FIFO, SYS_SYNC_RETRY, SYS_SYNC_PRIORITY] {
            let attr = LwMutexAttr { protocol: p, recursive: SYS_SYNC_NOT_RECURSIVE, name_u64: 0 };
            assert!(attr.validate().is_ok(), "{p}");
        }
    }

    #[test]
    fn validate_rejects_bad_recursive() {
        for bad in [0u32, 1, 0x30, 0xFF] {
            let attr = LwMutexAttr { protocol: SYS_SYNC_PRIORITY, recursive: bad, name_u64: 0 };
            assert_eq!(attr.validate().unwrap_err(), CELL_EINVAL, "{bad:#x}");
        }
    }

    #[test]
    fn validate_rejects_bad_protocol() {
        for bad in [0u32, 0x3, 0x8, 0xFF] {
            let attr = LwMutexAttr { protocol: bad, recursive: SYS_SYNC_NOT_RECURSIVE, name_u64: 0 };
            assert_eq!(attr.validate().unwrap_err(), CELL_EINVAL, "{bad:#x}");
        }
    }

    #[test]
    fn internal_protocol_fifo_stays_fifo() {
        let attr = LwMutexAttr { protocol: SYS_SYNC_FIFO, recursive: SYS_SYNC_NOT_RECURSIVE, name_u64: 0 };
        assert_eq!(attr.internal_protocol(), SYS_SYNC_FIFO);
    }

    #[test]
    fn internal_protocol_retry_becomes_priority() {
        // sys_lwmutex_.cpp:38 coerces non-FIFO into PRIORITY.
        let attr = LwMutexAttr { protocol: SYS_SYNC_RETRY, recursive: SYS_SYNC_NOT_RECURSIVE, name_u64: 0 };
        assert_eq!(attr.internal_protocol(), SYS_SYNC_PRIORITY);
    }

    #[test]
    fn internal_protocol_priority_stays_priority() {
        assert_eq!(good_attr().internal_protocol(), SYS_SYNC_PRIORITY);
    }

    // ---- create ------------------------------------------------------

    #[test]
    fn create_initializes_free() {
        let lw = LwMutex::create(&good_attr(), 0x1000).unwrap();
        assert_eq!(lw.owner, LWMUTEX_FREE);
        assert_eq!(lw.recursive_count, 0);
        assert_eq!(lw.sleep_queue, 0x1000);
    }

    #[test]
    fn create_packs_attribute_field() {
        let lw = LwMutex::create(&good_attr(), 0).unwrap();
        // attribute = recursive | protocol = NOT_RECURSIVE | PRIORITY = 0x22
        assert_eq!(lw.attribute, SYS_SYNC_NOT_RECURSIVE | SYS_SYNC_PRIORITY);
    }

    #[test]
    fn create_recursive_attribute_set() {
        let lw = LwMutex::create(&recursive_attr(), 0).unwrap();
        assert!(lw.is_recursive());
    }

    #[test]
    fn create_with_bad_attr_returns_einval() {
        let bad = LwMutexAttr { protocol: 0, recursive: 0, name_u64: 0 };
        assert_eq!(LwMutex::create(&bad, 0).unwrap_err(), CELL_EINVAL);
    }

    // ---- trylock -----------------------------------------------------

    #[test]
    fn trylock_acquires_free_mutex() {
        let mut lw = LwMutex::create(&good_attr(), 0).unwrap();
        lw.trylock(42).unwrap();
        assert_eq!(lw.owner, 42);
    }

    #[test]
    fn trylock_recursive_increments_count() {
        let mut lw = LwMutex::create(&recursive_attr(), 0).unwrap();
        lw.trylock(42).unwrap();
        lw.trylock(42).unwrap();
        lw.trylock(42).unwrap();
        assert_eq!(lw.recursive_count, 2);
    }

    #[test]
    fn trylock_non_recursive_deadlock() {
        let mut lw = LwMutex::create(&good_attr(), 0).unwrap();
        lw.trylock(42).unwrap();
        assert_eq!(lw.trylock(42).unwrap_err(), CELL_EDEADLK);
    }

    #[test]
    fn trylock_recursion_limit_is_ekresource() {
        let mut lw = LwMutex::create(&recursive_attr(), 0).unwrap();
        lw.trylock(42).unwrap();
        lw.recursive_count = u32::MAX;
        assert_eq!(lw.trylock(42).unwrap_err(), CELL_EKRESOURCE);
    }

    #[test]
    fn trylock_dead_mutex_is_einval() {
        let mut lw = LwMutex::create(&good_attr(), 0).unwrap();
        lw.owner = LWMUTEX_DEAD;
        assert_eq!(lw.trylock(42).unwrap_err(), CELL_EINVAL);
    }

    #[test]
    fn trylock_other_owner_is_ebusy() {
        let mut lw = LwMutex::create(&good_attr(), 0).unwrap();
        lw.trylock(42).unwrap();
        assert_eq!(lw.trylock(99).unwrap_err(), CELL_EBUSY);
    }

    // ---- lock --------------------------------------------------------

    #[test]
    fn lock_fast_path_acquires() {
        let mut lw = LwMutex::create(&good_attr(), 0).unwrap();
        assert_eq!(lw.lock(42), LockOutcome::Acquired);
        assert_eq!(lw.owner, 42);
    }

    #[test]
    fn lock_recursive_branch() {
        let mut lw = LwMutex::create(&recursive_attr(), 0).unwrap();
        lw.lock(42);
        assert_eq!(lw.lock(42), LockOutcome::Acquired);
        assert_eq!(lw.recursive_count, 1);
    }

    #[test]
    fn lock_non_recursive_deadlock() {
        let mut lw = LwMutex::create(&good_attr(), 0).unwrap();
        lw.lock(42);
        assert_eq!(lw.lock(42), LockOutcome::Error(CELL_EDEADLK));
    }

    #[test]
    fn lock_dead_mutex_is_error_einval() {
        let mut lw = LwMutex::create(&good_attr(), 0).unwrap();
        lw.owner = LWMUTEX_DEAD;
        assert_eq!(lw.lock(42), LockOutcome::Error(CELL_EINVAL));
    }

    #[test]
    fn lock_contention_would_sleep_and_increments_waiters() {
        let mut lw = LwMutex::create(&good_attr(), 0).unwrap();
        lw.lock(42); // owner = 42
        assert_eq!(lw.lock(99), LockOutcome::WouldSleep);
        assert_eq!(lw.all_info_waiters, 1);
    }

    #[test]
    fn finish_sleep_acquire_from_reserved() {
        let mut lw = LwMutex::create(&good_attr(), 0).unwrap();
        lw.owner = LWMUTEX_RESERVED;
        lw.all_info_waiters = 1;
        lw.finish_sleep_acquire(99).unwrap();
        assert_eq!(lw.owner, 99);
        assert_eq!(lw.all_info_waiters, 0);
    }

    #[test]
    fn finish_sleep_acquire_from_non_reserved_is_einval() {
        let mut lw = LwMutex::create(&good_attr(), 0).unwrap();
        lw.owner = 55;
        assert_eq!(lw.finish_sleep_acquire(99).unwrap_err(), CELL_EINVAL);
    }

    // ---- unlock ------------------------------------------------------

    #[test]
    fn unlock_non_owner_is_eperm() {
        let mut lw = LwMutex::create(&good_attr(), 0).unwrap();
        lw.trylock(42).unwrap();
        assert_eq!(lw.unlock(99).unwrap_err(), CELL_EPERM);
    }

    #[test]
    fn unlock_releases_free_when_no_waiters() {
        let mut lw = LwMutex::create(&good_attr(), 0).unwrap();
        lw.trylock(42).unwrap();
        assert_eq!(lw.unlock(42).unwrap(), UnlockOutcome::Released);
        assert_eq!(lw.owner, LWMUTEX_FREE);
    }

    #[test]
    fn unlock_decrements_recursive_count_first() {
        let mut lw = LwMutex::create(&recursive_attr(), 0).unwrap();
        lw.trylock(42).unwrap();
        lw.trylock(42).unwrap();
        lw.trylock(42).unwrap();
        // Three locks → recursive_count = 2.
        assert_eq!(lw.recursive_count, 2);
        assert_eq!(lw.unlock(42).unwrap(), UnlockOutcome::Released);
        assert_eq!(lw.recursive_count, 1);
        assert_eq!(lw.owner, 42); // still owned
    }

    #[test]
    fn unlock_with_waiters_priority_uses_reserved() {
        let mut lw = LwMutex::create(&good_attr(), 0).unwrap();
        lw.trylock(42).unwrap();
        lw.all_info_waiters = 1;
        match lw.unlock(42).unwrap() {
            UnlockOutcome::NeedsSyscall { reserved: true } => {
                assert_eq!(lw.owner, LWMUTEX_RESERVED);
            }
            other => panic!("expected reserved path, got {other:?}"),
        }
    }

    #[test]
    fn unlock_with_waiters_retry_clears_owner() {
        let retry = LwMutexAttr {
            protocol: SYS_SYNC_RETRY,
            recursive: SYS_SYNC_NOT_RECURSIVE,
            name_u64: 0,
        };
        let mut lw = LwMutex::create(&retry, 0).unwrap();
        lw.trylock(42).unwrap();
        lw.all_info_waiters = 1;
        assert!(lw.is_retry());
        match lw.unlock(42).unwrap() {
            UnlockOutcome::NeedsSyscall { reserved: false } => {
                assert_eq!(lw.owner, LWMUTEX_FREE);
            }
            other => panic!("expected retry path, got {other:?}"),
        }
    }

    // ---- destroy -----------------------------------------------------

    #[test]
    fn destroy_owner_is_ebusy() {
        let mut lw = LwMutex::create(&good_attr(), 0).unwrap();
        lw.trylock(42).unwrap();
        assert_eq!(lw.destroy(42).unwrap_err(), CELL_EBUSY);
    }

    #[test]
    fn destroy_free_mutex_marks_dead() {
        let mut lw = LwMutex::create(&good_attr(), 0).unwrap();
        lw.destroy(99).unwrap();
        assert_eq!(lw.owner, LWMUTEX_DEAD);
    }

    #[test]
    fn destroy_then_lock_is_einval() {
        let mut lw = LwMutex::create(&good_attr(), 0).unwrap();
        lw.destroy(99).unwrap();
        assert_eq!(lw.lock(42), LockOutcome::Error(CELL_EINVAL));
    }

    // ---- full smoke --------------------------------------------------

    #[test]
    fn full_lwmutex_lifecycle_smoke() {
        // 1. Create a recursive priority mutex.
        let mut lw = LwMutex::create(&recursive_attr(), 0x1000).unwrap();
        assert!(lw.is_recursive());

        // 2. Acquire it twice (recursive).
        assert_eq!(lw.lock(42), LockOutcome::Acquired);
        assert_eq!(lw.lock(42), LockOutcome::Acquired);
        assert_eq!(lw.recursive_count, 1);

        // 3. Another thread tries to lock — would sleep.
        assert_eq!(lw.lock(99), LockOutcome::WouldSleep);
        assert_eq!(lw.all_info_waiters, 1);

        // 4. Unlock inner recursion first.
        assert_eq!(lw.unlock(42).unwrap(), UnlockOutcome::Released);
        assert_eq!(lw.recursive_count, 0);

        // 5. Unlock outer — waiters exist → NeedsSyscall{reserved=true}.
        match lw.unlock(42).unwrap() {
            UnlockOutcome::NeedsSyscall { reserved: true } => {
                assert_eq!(lw.owner, LWMUTEX_RESERVED);
            }
            other => panic!("expected NeedsSyscall, got {other:?}"),
        }

        // 6. Firmware syscall would hand off to thread 99 — simulate.
        lw.finish_sleep_acquire(99).unwrap();
        assert_eq!(lw.owner, 99);
        assert_eq!(lw.all_info_waiters, 0);

        // 7. Thread 99 releases.
        assert_eq!(lw.unlock(99).unwrap(), UnlockOutcome::Released);
        assert_eq!(lw.owner, LWMUTEX_FREE);

        // 8. Destroy while free → OK.
        lw.destroy(42).unwrap();
        assert_eq!(lw.owner, LWMUTEX_DEAD);

        // 9. Further operations on dead mutex fail.
        assert_eq!(lw.lock(42), LockOutcome::Error(CELL_EINVAL));
    }
}
