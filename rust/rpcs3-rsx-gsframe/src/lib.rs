//! `rpcs3-rsx-gsframe` — Rust port of `rpcs3/Emu/RSX/GSFrameBase.cpp`.
//!
//! Two globals the game-window plumbing exposes: whether the focus is
//! currently inside the emulator window, and a helper that combines that
//! with the "background input allowed" config flag.
//!
//! Frozen:
//!
//! - `g_game_window_focused` initial value `false` (cpp:5).
//! - `is_input_allowed()` predicate: focused OR background-input cfg
//!   enabled (cpp:7..10).

use std::sync::atomic::{AtomicBool, Ordering};

/// Global focus tracker. The C++ version is `atomic_t<bool>` with
/// default `false`.
pub static GAME_WINDOW_FOCUSED: AtomicBool = AtomicBool::new(false);

/// `is_input_allowed(background_input_enabled)` (cpp:7..10).
/// Takes the config flag explicitly so the crate stays free of the RPCS3
/// config stack. Returns true when either the window is focused or the
/// config allows background input.
#[must_use]
pub fn is_input_allowed(background_input_enabled: bool) -> bool {
    GAME_WINDOW_FOCUSED.load(Ordering::SeqCst) || background_input_enabled
}

/// Convenience setters for tests and frontends driving focus state.
pub fn set_game_window_focused(focused: bool) {
    GAME_WINDOW_FOCUSED.store(focused, Ordering::SeqCst);
}

#[must_use]
pub fn get_game_window_focused() -> bool {
    GAME_WINDOW_FOCUSED.load(Ordering::SeqCst)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // The global is shared across tests — serialize to avoid interference.
    static LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn focus_starts_false_and_toggles() {
        let _guard = LOCK.lock().unwrap();
        set_game_window_focused(false);
        assert!(!get_game_window_focused());
        set_game_window_focused(true);
        assert!(get_game_window_focused());
        set_game_window_focused(false);
    }

    #[test]
    fn is_input_allowed_when_focused() {
        let _guard = LOCK.lock().unwrap();
        set_game_window_focused(true);
        assert!(is_input_allowed(false));
        assert!(is_input_allowed(true));
        set_game_window_focused(false);
    }

    #[test]
    fn is_input_allowed_when_background_cfg_enabled() {
        let _guard = LOCK.lock().unwrap();
        set_game_window_focused(false);
        assert!(is_input_allowed(true));
        assert!(!is_input_allowed(false));
    }

    #[test]
    fn is_input_allowed_neither_flag_set_returns_false() {
        let _guard = LOCK.lock().unwrap();
        set_game_window_focused(false);
        assert!(!is_input_allowed(false));
    }
}
