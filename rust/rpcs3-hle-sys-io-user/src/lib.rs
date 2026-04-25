//! `rpcs3-hle-sys-io-user` — PS3 I/O subsystem user-mode HLE.
//!
//! Ports `rpcs3/Emu/Cell/Modules/sys_io_.cpp` (253 linhas).  This is
//! the user-mode shim that sits in front of cellPad / cellKb /
//! cellMouse: on `sys_config_start` it spins up a helper PPU thread
//! that receives events from an LV2 event queue and forwards them to
//! the input modules; `sys_config_stop` tears the thread down via a
//! reference-counted `init_ctr`.
//!
//! ## Entry points covered
//!
//! | C++ function                               | Rust wrapper                      |
//! |--------------------------------------------|-----------------------------------|
//! | `sys_config_start`                         | [`LibIoSysConfig::start`]         |
//! | `sys_config_stop`                          | [`LibIoSysConfig::stop`]          |
//! | `sys_config_add_service_listener`          | [`LibIoSysConfig::add_listener`]  |
//! | `sys_config_remove_service_listener`       | [`LibIoSysConfig::remove_listener`] |
//! | `sys_config_register_io_error_handler`     | [`LibIoSysConfig::register_handler`] |
//! | `sys_config_unregister_io_error_handler`   | [`LibIoSysConfig::unregister_handler`] |
//! | `sys_config_register_service`              | [`LibIoSysConfig::register_service`]   |
//! | `sys_config_unregister_service`            | [`LibIoSysConfig::unregister_service`] |

extern crate alloc;

use alloc::string::String;
use rpcs3_emu_types::CellError;

// =====================================================================
// Error codes
// =====================================================================

pub const CELL_EINVAL: CellError = CellError(0x8001_0002);

// =====================================================================
// Constants — byte-exact with sys_io_.cpp
// =====================================================================

/// Event queue depth used by `sys_config_start` (cpp:164
/// `sys_event_queue_create(ppu, queue_id, attr, 0, 0x20)`).
pub const CONFIG_EVENT_QUEUE_DEPTH: u32 = 0x20;

/// Helper PPU thread priority (cpp:168 `prio=512`).
pub const CONFIG_THREAD_PRIORITY: u32 = 512;

/// Helper PPU thread stack size (cpp:168 `stacksize=0x2000`).
pub const CONFIG_THREAD_STACK_SIZE: u32 = 0x2000;

/// Helper PPU thread name (cpp:157 `"_cfg_evt_hndlr"`).
pub const CONFIG_THREAD_NAME: &str = "_cfg_evt_hndlr";

/// `arg1 == 1` is the pad-state notification dispatch (cpp:77).
pub const EVENT_KIND_PAD_NOTIFY: u64 = 1;

// =====================================================================
// LibIoSysConfig — mirror of the fxo singleton
// =====================================================================

/// Mirror of `libio_sys_config` (cpp:16-31).  Shared state guarded by a
/// mutex in C++; the Rust port leaves concurrency to the caller.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LibIoSysConfig {
    /// Reference counter — incremented by `start`, decremented by
    /// `stop`.  Thread + queue only exist when `init_ctr > 0`.
    pub init_ctr: i32,
    /// PPU id of the helper thread (`cfg_evt_hndlr`).
    pub ppu_id: u32,
    /// LV2 event-queue id the helper thread reads from.
    pub queue_id: u32,
    /// Registered service listeners (stubbed in the C++ port).
    pub service_listeners: alloc::vec::Vec<u32>,
    /// Registered services (name → handler).
    pub services: alloc::vec::Vec<(String, u32)>,
    /// Registered io-error handlers.
    pub error_handlers: alloc::vec::Vec<u32>,
}

impl LibIoSysConfig {
    #[must_use]
    pub fn new() -> Self { Self::default() }

    /// Port of `sys_config_start`.  Increments `init_ctr`; on the
    /// first call, spawns the helper thread + event queue.  Returns
    /// `StartOutcome` so the caller can drive the real syscalls.
    #[must_use]
    pub fn start(&mut self, mock_queue_id: u32, mock_ppu_id: u32) -> StartOutcome {
        let first = self.init_ctr == 0;
        self.init_ctr = self.init_ctr.saturating_add(1);
        if first {
            self.queue_id = mock_queue_id;
            self.ppu_id = mock_ppu_id;
            StartOutcome::FirstStart {
                queue_id: mock_queue_id,
                ppu_id: mock_ppu_id,
            }
        } else {
            StartOutcome::AlreadyStarted { init_ctr: self.init_ctr }
        }
    }

    /// Port of `sys_config_stop`.  Decrements `init_ctr`; when it
    /// reaches zero, tears down the helper thread + queue.
    #[must_use]
    pub fn stop(&mut self) -> StopOutcome {
        if self.init_ctr <= 0 {
            // C++ has a TODO comment for this path — it still returns
            // CELL_OK though, so surface as `NotStarted`.
            return StopOutcome::NotStarted;
        }
        self.init_ctr -= 1;
        if self.init_ctr == 0 {
            let out = StopOutcome::LastStop {
                queue_id: self.queue_id,
                ppu_id: self.ppu_id,
            };
            self.queue_id = 0;
            self.ppu_id = 0;
            out
        } else {
            StopOutcome::StillActive { init_ctr: self.init_ctr }
        }
    }

    /// Port of `send_sys_io_connect_event` (cpp:123-142).  Only enqueues
    /// the event when `init_ctr > 0` — otherwise the event is dropped
    /// (cellPad_NotifyStateChange is called instead in the C++ fast
    /// path, which the Rust port omits since it's not in the module).
    #[must_use]
    pub fn send_connect_event(&self, index: u64, state: u64) -> Option<ConfigEvent> {
        if self.init_ctr > 0 {
            Some(ConfigEvent { source: 0, arg1: 1, arg2: index, arg3: state })
        } else {
            None
        }
    }

    /// Port of the pad-dispatch branch in `config_event_entry`
    /// (cpp:77-89).  Returns `true` if the event should be forwarded
    /// to `cellPad_NotifyStateChange`, `false` otherwise.
    #[must_use]
    pub fn should_dispatch_pad(event: &ConfigEvent) -> bool {
        event.arg1 == EVENT_KIND_PAD_NOTIFY
    }

    // ---- service listeners / handlers / services ---------------------

    /// Port of `sys_config_add_service_listener`.  The firmware stub
    /// just returns `CELL_OK`; the Rust port stores the handle so
    /// tests can inspect it.
    pub fn add_listener(&mut self, listener: u32) -> Result<(), CellError> {
        if self.service_listeners.contains(&listener) && listener != 0 {
            return Err(CELL_EINVAL);
        }
        self.service_listeners.push(listener);
        Ok(())
    }

    /// Port of `sys_config_remove_service_listener`.
    pub fn remove_listener(&mut self, listener: u32) -> Result<(), CellError> {
        let pos = self.service_listeners.iter().position(|&l| l == listener)
            .ok_or(CELL_EINVAL)?;
        self.service_listeners.swap_remove(pos);
        Ok(())
    }

    /// Port of `sys_config_register_io_error_handler`.
    pub fn register_handler(&mut self, handler: u32) -> Result<(), CellError> {
        if self.error_handlers.contains(&handler) && handler != 0 {
            return Err(CELL_EINVAL);
        }
        self.error_handlers.push(handler);
        Ok(())
    }

    /// Port of `sys_config_unregister_io_error_handler`.
    pub fn unregister_handler(&mut self, handler: u32) -> Result<(), CellError> {
        let pos = self.error_handlers.iter().position(|&h| h == handler)
            .ok_or(CELL_EINVAL)?;
        self.error_handlers.swap_remove(pos);
        Ok(())
    }

    /// Port of `sys_config_register_service`.
    pub fn register_service(&mut self, name: &str, handler: u32) -> Result<(), CellError> {
        if self.services.iter().any(|(n, _)| n == name) {
            return Err(CELL_EINVAL);
        }
        self.services.push((String::from(name), handler));
        Ok(())
    }

    /// Port of `sys_config_unregister_service`.
    pub fn unregister_service(&mut self, name: &str) -> Result<(), CellError> {
        let pos = self.services.iter().position(|(n, _)| n == name)
            .ok_or(CELL_EINVAL)?;
        self.services.swap_remove(pos);
        Ok(())
    }
}

/// Outcome of [`LibIoSysConfig::start`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartOutcome {
    /// First call — caller must create the LV2 event queue + helper
    /// PPU thread with the returned ids.
    FirstStart { queue_id: u32, ppu_id: u32 },
    /// Subsequent call — ref-count bumped.
    AlreadyStarted { init_ctr: i32 },
}

/// Outcome of [`LibIoSysConfig::stop`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopOutcome {
    /// Ref-count dropped to zero — caller must destroy the queue + join
    /// the helper thread with the ids returned.
    LastStop { queue_id: u32, ppu_id: u32 },
    /// Ref-count still > 0 — tear-down deferred.
    StillActive { init_ctr: i32 },
    /// Stop called while already at zero — C++ has a TODO; firmware
    /// silently returns `CELL_OK`.
    NotStarted,
}

/// Event payload enqueued by `send_sys_io_connect_event` and consumed
/// by the helper thread in `config_event_entry`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConfigEvent {
    pub source: u64,
    pub arg1: u64,
    pub arg2: u64,
    pub arg3: u64,
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ---- constants ---------------------------------------------------

    #[test]
    fn cell_einval_byte_exact() {
        assert_eq!(CELL_EINVAL.0, 0x8001_0002);
    }

    #[test]
    fn config_queue_depth_byte_exact() {
        // cpp:164 → 0x20.
        assert_eq!(CONFIG_EVENT_QUEUE_DEPTH, 0x20);
    }

    #[test]
    fn thread_priority_byte_exact() {
        assert_eq!(CONFIG_THREAD_PRIORITY, 512);
    }

    #[test]
    fn thread_stack_size_byte_exact() {
        assert_eq!(CONFIG_THREAD_STACK_SIZE, 0x2000);
    }

    #[test]
    fn thread_name_byte_exact() {
        // cpp:157 — "_cfg_evt_hndlr".
        assert_eq!(CONFIG_THREAD_NAME, "_cfg_evt_hndlr");
    }

    #[test]
    fn event_kind_pad_notify_is_1() {
        // cpp:77 `arg1 == 1` branch.
        assert_eq!(EVENT_KIND_PAD_NOTIFY, 1);
    }

    // ---- start / stop ref-counting ----------------------------------

    #[test]
    fn start_first_call_spawns_thread_and_queue() {
        let mut cfg = LibIoSysConfig::new();
        let out = cfg.start(0x4000, 0x5000);
        assert_eq!(out, StartOutcome::FirstStart {
            queue_id: 0x4000,
            ppu_id: 0x5000,
        });
        assert_eq!(cfg.init_ctr, 1);
        assert_eq!(cfg.queue_id, 0x4000);
        assert_eq!(cfg.ppu_id, 0x5000);
    }

    #[test]
    fn start_second_call_bumps_refcount() {
        let mut cfg = LibIoSysConfig::new();
        cfg.start(0x4000, 0x5000);
        let out = cfg.start(0xDEAD, 0xBEEF);
        assert_eq!(out, StartOutcome::AlreadyStarted { init_ctr: 2 });
        // Subsequent calls do NOT change the stored ids.
        assert_eq!(cfg.queue_id, 0x4000);
        assert_eq!(cfg.ppu_id, 0x5000);
    }

    #[test]
    fn stop_before_start_is_not_started() {
        let mut cfg = LibIoSysConfig::new();
        assert_eq!(cfg.stop(), StopOutcome::NotStarted);
    }

    #[test]
    fn stop_refcount_greater_than_one_is_still_active() {
        let mut cfg = LibIoSysConfig::new();
        cfg.start(0x4000, 0x5000);
        cfg.start(0x4000, 0x5000);
        let out = cfg.stop();
        assert_eq!(out, StopOutcome::StillActive { init_ctr: 1 });
        // Ids still present.
        assert_eq!(cfg.queue_id, 0x4000);
    }

    #[test]
    fn stop_last_call_tears_down() {
        let mut cfg = LibIoSysConfig::new();
        cfg.start(0x4000, 0x5000);
        let out = cfg.stop();
        assert_eq!(out, StopOutcome::LastStop {
            queue_id: 0x4000,
            ppu_id: 0x5000,
        });
        assert_eq!(cfg.init_ctr, 0);
        assert_eq!(cfg.queue_id, 0);
        assert_eq!(cfg.ppu_id, 0);
    }

    #[test]
    fn start_stop_roundtrip() {
        let mut cfg = LibIoSysConfig::new();
        for _ in 0..3 {
            cfg.start(0x4000, 0x5000);
        }
        assert_eq!(cfg.init_ctr, 3);
        assert!(matches!(cfg.stop(), StopOutcome::StillActive { init_ctr: 2 }));
        assert!(matches!(cfg.stop(), StopOutcome::StillActive { init_ctr: 1 }));
        assert!(matches!(cfg.stop(), StopOutcome::LastStop { .. }));
        assert_eq!(cfg.init_ctr, 0);
    }

    // ---- send_connect_event -----------------------------------------

    #[test]
    fn send_connect_event_drops_when_not_started() {
        let cfg = LibIoSysConfig::new();
        assert!(cfg.send_connect_event(0, 1).is_none());
    }

    #[test]
    fn send_connect_event_emits_when_active() {
        let mut cfg = LibIoSysConfig::new();
        cfg.start(0x4000, 0x5000);
        let event = cfg.send_connect_event(3, 0xA5).unwrap();
        assert_eq!(event.arg1, EVENT_KIND_PAD_NOTIFY);
        assert_eq!(event.arg2, 3);
        assert_eq!(event.arg3, 0xA5);
    }

    #[test]
    fn should_dispatch_pad_for_kind_1() {
        let event = ConfigEvent { source: 0, arg1: 1, arg2: 0, arg3: 0 };
        assert!(LibIoSysConfig::should_dispatch_pad(&event));
    }

    #[test]
    fn should_not_dispatch_pad_for_other_kind() {
        let event = ConfigEvent { source: 0, arg1: 2, arg2: 0, arg3: 0 };
        assert!(!LibIoSysConfig::should_dispatch_pad(&event));
        let event = ConfigEvent { source: 0, arg1: 0, arg2: 0, arg3: 0 };
        assert!(!LibIoSysConfig::should_dispatch_pad(&event));
    }

    // ---- service listeners ------------------------------------------

    #[test]
    fn add_listener_stores_handle() {
        let mut cfg = LibIoSysConfig::new();
        cfg.add_listener(0x1234).unwrap();
        assert_eq!(cfg.service_listeners, alloc::vec![0x1234]);
    }

    #[test]
    fn add_duplicate_listener_is_einval() {
        let mut cfg = LibIoSysConfig::new();
        cfg.add_listener(0x1234).unwrap();
        assert_eq!(cfg.add_listener(0x1234).unwrap_err(), CELL_EINVAL);
    }

    #[test]
    fn remove_listener_clears_handle() {
        let mut cfg = LibIoSysConfig::new();
        cfg.add_listener(0x1234).unwrap();
        cfg.remove_listener(0x1234).unwrap();
        assert!(cfg.service_listeners.is_empty());
    }

    #[test]
    fn remove_unknown_listener_is_einval() {
        let mut cfg = LibIoSysConfig::new();
        assert_eq!(cfg.remove_listener(0xABCD).unwrap_err(), CELL_EINVAL);
    }

    // ---- io-error handlers ------------------------------------------

    #[test]
    fn register_handler_stores_handle() {
        let mut cfg = LibIoSysConfig::new();
        cfg.register_handler(0x5555).unwrap();
        assert_eq!(cfg.error_handlers, alloc::vec![0x5555]);
    }

    #[test]
    fn register_duplicate_handler_is_einval() {
        let mut cfg = LibIoSysConfig::new();
        cfg.register_handler(0x5555).unwrap();
        assert_eq!(cfg.register_handler(0x5555).unwrap_err(), CELL_EINVAL);
    }

    #[test]
    fn unregister_handler_removes() {
        let mut cfg = LibIoSysConfig::new();
        cfg.register_handler(0x5555).unwrap();
        cfg.unregister_handler(0x5555).unwrap();
        assert!(cfg.error_handlers.is_empty());
    }

    #[test]
    fn unregister_unknown_handler_is_einval() {
        let mut cfg = LibIoSysConfig::new();
        assert_eq!(cfg.unregister_handler(0xCAFE).unwrap_err(), CELL_EINVAL);
    }

    // ---- services ---------------------------------------------------

    #[test]
    fn register_service_stores_pair() {
        let mut cfg = LibIoSysConfig::new();
        cfg.register_service("pad", 0x9000).unwrap();
        assert_eq!(cfg.services.len(), 1);
        assert_eq!(cfg.services[0].0, "pad");
        assert_eq!(cfg.services[0].1, 0x9000);
    }

    #[test]
    fn register_duplicate_service_is_einval() {
        let mut cfg = LibIoSysConfig::new();
        cfg.register_service("pad", 0x9000).unwrap();
        assert_eq!(
            cfg.register_service("pad", 0xDEAD).unwrap_err(),
            CELL_EINVAL,
        );
    }

    #[test]
    fn unregister_service_removes() {
        let mut cfg = LibIoSysConfig::new();
        cfg.register_service("pad", 0x9000).unwrap();
        cfg.unregister_service("pad").unwrap();
        assert!(cfg.services.is_empty());
    }

    #[test]
    fn unregister_unknown_service_is_einval() {
        let mut cfg = LibIoSysConfig::new();
        assert_eq!(cfg.unregister_service("ghost").unwrap_err(), CELL_EINVAL);
    }

    #[test]
    fn multiple_services_can_coexist() {
        let mut cfg = LibIoSysConfig::new();
        cfg.register_service("pad", 0x9000).unwrap();
        cfg.register_service("kb",  0xA000).unwrap();
        cfg.register_service("mouse", 0xB000).unwrap();
        assert_eq!(cfg.services.len(), 3);
    }

    // ---- full smoke -------------------------------------------------

    #[test]
    fn full_sys_io_lifecycle_smoke() {
        let mut cfg = LibIoSysConfig::new();

        // 1. First start — allocates thread + queue.
        let out = cfg.start(0x4000, 0x5000);
        assert!(matches!(out, StartOutcome::FirstStart { queue_id: 0x4000, ppu_id: 0x5000 }));

        // 2. Register a couple services.
        cfg.register_service("pad",   0x9000).unwrap();
        cfg.register_service("mouse", 0xA000).unwrap();
        assert_eq!(cfg.services.len(), 2);

        // 3. Send a pad-connect event — should enqueue.
        let event = cfg.send_connect_event(0, 0xA5).unwrap();
        assert!(LibIoSysConfig::should_dispatch_pad(&event));

        // 4. Register + remove an io-error handler.
        cfg.register_handler(0x5555).unwrap();
        cfg.unregister_handler(0x5555).unwrap();
        assert!(cfg.error_handlers.is_empty());

        // 5. Second start — ref-count bumps.
        let out = cfg.start(0xDEAD, 0xBEEF);
        assert!(matches!(out, StartOutcome::AlreadyStarted { init_ctr: 2 }));

        // 6. Stop once → still active.
        assert!(matches!(cfg.stop(), StopOutcome::StillActive { init_ctr: 1 }));

        // 7. Stop again → tear down.
        let out = cfg.stop();
        assert!(matches!(out, StopOutcome::LastStop { queue_id: 0x4000, ppu_id: 0x5000 }));
        assert_eq!(cfg.init_ctr, 0);

        // 8. Events are dropped once torn down.
        assert!(cfg.send_connect_event(0, 0xA5).is_none());

        // 9. Stop again → NotStarted.
        assert_eq!(cfg.stop(), StopOutcome::NotStarted);
    }
}
