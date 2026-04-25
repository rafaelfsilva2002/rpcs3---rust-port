//! `rpcs3-io-interception` — Rust port of `rpcs3/Emu/Io/interception.cpp`.
//!
//! Global "is this input source intercepted by the OSD/debugger?"
//! toggles plus the mouse+keyboard active-device tracker. The C++ version
//! uses atomics because several threads poll these flags every frame;
//! we expose the same shape via `AtomicBool` + an `AtomicU32` cast of
//! the enum so frontends can share state across threads identically.
//!
//! Frozen:
//!
//! - `ActiveMouseAndKeyboard` enum (Emulated / Pad) with matching
//!   positional discriminants (cpp:18..21).
//! - Per-device interception flags: pads / keyboards / mice.
//! - `set_intercepted(pads, keyboards, mice)` 3-arg + 1-arg (all) overload
//!   semantics (cpp:34..58).
//! - `toggle_mouse_and_keyboard` XOR swap (cpp:83..88).
//! - `set_mouse_and_keyboard` early-out when value unchanged (cpp:76..80).

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

/// Currently routed mouse/keyboard source.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveMouseAndKeyboard {
    /// RPCS3 emulates libkeyboard/libmouse; host events go through.
    Emulated = 0,
    /// The source is treated as a pad; no M+KB APIs see events.
    Pad = 1,
}

impl ActiveMouseAndKeyboard {
    pub fn from_u32(v: u32) -> Self {
        match v {
            1 => Self::Pad,
            _ => Self::Emulated,
        }
    }
}

/// Global interception state. Fields are atomic to match the cpp
/// `atomic_t<...>`.
pub struct InterceptionState {
    pub pads: AtomicBool,
    pub keyboards: AtomicBool,
    pub mice: AtomicBool,
    /// Stored as `u32` to allow `Atomic*` ops; convert via
    /// `ActiveMouseAndKeyboard::from_u32`.
    pub active_mkb: AtomicU32,
}

impl InterceptionState {
    pub const fn new() -> Self {
        Self {
            pads: AtomicBool::new(false),
            keyboards: AtomicBool::new(false),
            mice: AtomicBool::new(false),
            // cpp:29 initializes to `emulated`.
            active_mkb: AtomicU32::new(ActiveMouseAndKeyboard::Emulated as u32),
        }
    }

    /// `SetIntercepted(pads, keyboards, mice)` (cpp:34..53).
    pub fn set_intercepted_individual(&self, pads: bool, keyboards: bool, mice: bool) {
        self.pads.store(pads, Ordering::SeqCst);
        self.keyboards.store(keyboards, Ordering::SeqCst);
        self.mice.store(mice, Ordering::SeqCst);
    }

    /// `SetIntercepted(all_intercepted)` single-value overload (cpp:55..58).
    pub fn set_intercepted_all(&self, all: bool) {
        self.set_intercepted_individual(all, all, all);
    }

    /// `set_mouse_and_keyboard(device)` (cpp:71..81). Returns `true` if
    /// the value changed (callers use this to trigger overlay updates).
    pub fn set_mouse_and_keyboard(&self, device: ActiveMouseAndKeyboard) -> bool {
        let new_val = device as u32;
        let old_val = self.active_mkb.swap(new_val, Ordering::SeqCst);
        old_val != new_val
    }

    /// `toggle_mouse_and_keyboard` (cpp:83..88). Returns the new value.
    pub fn toggle_mouse_and_keyboard(&self) -> ActiveMouseAndKeyboard {
        let old = ActiveMouseAndKeyboard::from_u32(self.active_mkb.load(Ordering::SeqCst));
        let new = match old {
            ActiveMouseAndKeyboard::Emulated => ActiveMouseAndKeyboard::Pad,
            ActiveMouseAndKeyboard::Pad => ActiveMouseAndKeyboard::Emulated,
        };
        self.active_mkb.store(new as u32, Ordering::SeqCst);
        new
    }

    /// Snapshot of all flags (useful for tests / telemetry).
    #[must_use]
    pub fn snapshot(&self) -> InterceptionSnapshot {
        InterceptionSnapshot {
            pads: self.pads.load(Ordering::SeqCst),
            keyboards: self.keyboards.load(Ordering::SeqCst),
            mice: self.mice.load(Ordering::SeqCst),
            active_mkb: ActiveMouseAndKeyboard::from_u32(self.active_mkb.load(Ordering::SeqCst)),
        }
    }
}

impl Default for InterceptionState {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InterceptionSnapshot {
    pub pads: bool,
    pub keyboards: bool,
    pub mice: bool,
    pub active_mkb: ActiveMouseAndKeyboard,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_mkb_discriminants() {
        assert_eq!(ActiveMouseAndKeyboard::Emulated as u32, 0);
        assert_eq!(ActiveMouseAndKeyboard::Pad as u32, 1);
    }

    #[test]
    fn from_u32_roundtrip() {
        assert_eq!(ActiveMouseAndKeyboard::from_u32(0), ActiveMouseAndKeyboard::Emulated);
        assert_eq!(ActiveMouseAndKeyboard::from_u32(1), ActiveMouseAndKeyboard::Pad);
        // Out-of-range collapses to Emulated (safe default).
        assert_eq!(ActiveMouseAndKeyboard::from_u32(99), ActiveMouseAndKeyboard::Emulated);
    }

    #[test]
    fn default_state_is_all_false_emulated() {
        let s = InterceptionState::new();
        let snap = s.snapshot();
        assert!(!snap.pads);
        assert!(!snap.keyboards);
        assert!(!snap.mice);
        assert_eq!(snap.active_mkb, ActiveMouseAndKeyboard::Emulated);
    }

    #[test]
    fn set_intercepted_individual_flips_each() {
        let s = InterceptionState::new();
        s.set_intercepted_individual(true, false, true);
        let snap = s.snapshot();
        assert!(snap.pads);
        assert!(!snap.keyboards);
        assert!(snap.mice);
    }

    #[test]
    fn set_intercepted_all_flips_everything() {
        let s = InterceptionState::new();
        s.set_intercepted_all(true);
        let snap = s.snapshot();
        assert!(snap.pads);
        assert!(snap.keyboards);
        assert!(snap.mice);
        s.set_intercepted_all(false);
        let snap = s.snapshot();
        assert!(!snap.pads);
        assert!(!snap.keyboards);
        assert!(!snap.mice);
    }

    #[test]
    fn set_mkb_signals_change() {
        let s = InterceptionState::new();
        // Same value → no change.
        assert!(!s.set_mouse_and_keyboard(ActiveMouseAndKeyboard::Emulated));
        // Different value → change.
        assert!(s.set_mouse_and_keyboard(ActiveMouseAndKeyboard::Pad));
        // Same again.
        assert!(!s.set_mouse_and_keyboard(ActiveMouseAndKeyboard::Pad));
    }

    #[test]
    fn toggle_mkb_xor_swap() {
        let s = InterceptionState::new();
        assert_eq!(s.toggle_mouse_and_keyboard(), ActiveMouseAndKeyboard::Pad);
        assert_eq!(s.toggle_mouse_and_keyboard(), ActiveMouseAndKeyboard::Emulated);
        assert_eq!(s.toggle_mouse_and_keyboard(), ActiveMouseAndKeyboard::Pad);
    }

    #[test]
    fn snapshot_independent_of_state_mutation() {
        let s = InterceptionState::new();
        let before = s.snapshot();
        s.set_intercepted_all(true);
        let after = s.snapshot();
        assert_eq!(before.pads, false);
        assert_eq!(after.pads, true);
    }
}
