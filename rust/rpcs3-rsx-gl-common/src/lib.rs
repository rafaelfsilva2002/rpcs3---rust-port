//! `rpcs3-rsx-gl-common` — Rust port of
//! `rpcs3/Emu/RSX/GL/glutils/common.cpp` (thread-local portion).
//!
//! The GL backend marks exactly one thread as the "primary context" to
//! gate operations that can't safely run off-thread. The cpp uses a
//! `thread_local bool` plus `set_/is_primary_context_thread`. We
//! replicate with Rust thread-locals — same semantics, same default
//! (false).
//!
//! The `command_context` / `driver_state` side of the cpp file depends
//! on OpenGL types we don't link; those stay in the frontend.

use std::cell::Cell;

thread_local! {
    static PRIMARY_CONTEXT_THREAD: Cell<bool> = const { Cell::new(false) };
}

/// `set_primary_context_thread(value)` (cpp:9..12).
pub fn set_primary_context_thread(value: bool) {
    PRIMARY_CONTEXT_THREAD.with(|c| c.set(value));
}

/// `is_primary_context_thread()` (cpp:14..17).
#[must_use]
pub fn is_primary_context_thread() -> bool {
    PRIMARY_CONTEXT_THREAD.with(Cell::get)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn default_is_false() {
        assert!(!is_primary_context_thread());
    }

    #[test]
    fn set_and_read() {
        set_primary_context_thread(true);
        assert!(is_primary_context_thread());
        set_primary_context_thread(false);
        assert!(!is_primary_context_thread());
    }

    #[test]
    fn tls_isolation_between_threads() {
        // Main thread sets true.
        set_primary_context_thread(true);
        assert!(is_primary_context_thread());

        // Spawned thread starts with default false.
        let child_saw = thread::spawn(|| is_primary_context_thread()).join().unwrap();
        assert!(!child_saw);

        // Main thread still sees its value.
        assert!(is_primary_context_thread());
        set_primary_context_thread(false);
    }

    #[test]
    fn multiple_toggles() {
        for _ in 0..10 {
            set_primary_context_thread(true);
            assert!(is_primary_context_thread());
            set_primary_context_thread(false);
            assert!(!is_primary_context_thread());
        }
    }
}
