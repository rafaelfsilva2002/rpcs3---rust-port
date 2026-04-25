//! `rpcs3-io-pad-config-types` — Rust port of
//! `rpcs3/Emu/Io/pad_config_types.{h,cpp}`.
//!
//! Enums + a tiny struct shared by every pad handler backend. The cpp
//! declares the enum with conditional variants (`xinput`/`mm` on Windows,
//! `sdl` with HAVE_SDL3, `evdev` with HAVE_LIBEVDEV). We always include
//! all variants here so the wire-format is identical across platforms;
//! the discriminants assume all `#ifdef` branches are active (the most
//! inclusive order).
//!
//! Frozen:
//!
//! - `PadHandler` discriminants in header declaration order.
//! - `MouseMovementMode::Relative=0, Absolute=1` (explicit header values).
//! - `PadInfo` struct (`now_connect: u32`, `system_info: u32`,
//!   `ignore_input: bool`).
//! - `name_for_handler` matches the cpp format strings verbatim.

/// Pad backend identifier. Discriminants preserved in cpp declaration
/// order — `null` is always 0 so zero-initialized configs fall back to
/// "no pad" gracefully.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PadHandler {
    Null = 0,
    Keyboard = 1,
    Ds3 = 2,
    Ds4 = 3,
    DualSense = 4,
    Skateboard = 5,
    Move = 6,
    XInput = 7,
    MMJoystick = 8,
    Sdl = 9,
    Evdev = 10,
}

/// `mouse_movement_mode` (`pad_config_types.h:26..30`).
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseMovementMode {
    Relative = 0,
    Absolute = 1,
}

/// `PadInfo` struct (`pad_config_types.h:32..37`). Three fields with
/// zero defaults.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PadInfo {
    pub now_connect: u32,
    pub system_info: u32,
    pub ignore_input: bool,
}

/// Pretty-print a `PadHandler` exactly like cpp `fmt_class_string`
/// (`pad_config_types.cpp:11..24`).
#[must_use]
pub const fn name_for_handler(h: PadHandler) -> &'static str {
    match h {
        PadHandler::Null => "Null",
        PadHandler::Keyboard => "Keyboard",
        PadHandler::Ds3 => "DualShock 3",
        PadHandler::Ds4 => "DualShock 4",
        PadHandler::DualSense => "DualSense",
        PadHandler::Skateboard => "Skateboard",
        PadHandler::Move => "PS Move",
        PadHandler::XInput => "XInput",
        PadHandler::MMJoystick => "MMJoystick",
        PadHandler::Sdl => "SDL",
        PadHandler::Evdev => "Evdev",
    }
}

/// Pretty-print `MouseMovementMode` (cpp:41..43).
#[must_use]
pub const fn name_for_mouse_mode(m: MouseMovementMode) -> &'static str {
    match m {
        MouseMovementMode::Relative => "Relative",
        MouseMovementMode::Absolute => "Absolute",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pad_handler_discriminants_in_order() {
        assert_eq!(PadHandler::Null as u32, 0);
        assert_eq!(PadHandler::Keyboard as u32, 1);
        assert_eq!(PadHandler::Ds3 as u32, 2);
        assert_eq!(PadHandler::Ds4 as u32, 3);
        assert_eq!(PadHandler::DualSense as u32, 4);
        assert_eq!(PadHandler::Skateboard as u32, 5);
        assert_eq!(PadHandler::Move as u32, 6);
    }

    #[test]
    fn mouse_mode_discriminants() {
        assert_eq!(MouseMovementMode::Relative as i32, 0);
        assert_eq!(MouseMovementMode::Absolute as i32, 1);
    }

    #[test]
    fn pad_info_defaults() {
        let p = PadInfo::default();
        assert_eq!(p.now_connect, 0);
        assert_eq!(p.system_info, 0);
        assert!(!p.ignore_input);
    }

    #[test]
    fn name_for_handler_matches_cpp_strings() {
        assert_eq!(name_for_handler(PadHandler::Null), "Null");
        assert_eq!(name_for_handler(PadHandler::Keyboard), "Keyboard");
        assert_eq!(name_for_handler(PadHandler::Ds3), "DualShock 3");
        assert_eq!(name_for_handler(PadHandler::Ds4), "DualShock 4");
        assert_eq!(name_for_handler(PadHandler::DualSense), "DualSense");
        assert_eq!(name_for_handler(PadHandler::Move), "PS Move");
        assert_eq!(name_for_handler(PadHandler::XInput), "XInput");
        assert_eq!(name_for_handler(PadHandler::MMJoystick), "MMJoystick");
    }

    #[test]
    fn name_for_mouse_mode_matches_cpp() {
        assert_eq!(name_for_mouse_mode(MouseMovementMode::Relative), "Relative");
        assert_eq!(name_for_mouse_mode(MouseMovementMode::Absolute), "Absolute");
    }

    #[test]
    fn pad_info_is_repr_c_size() {
        use core::mem::size_of;
        // 4 (now_connect) + 4 (system_info) + 1 (bool) + 3 padding = 12 bytes
        // with default alignment. The cpp struct has the same layout.
        assert_eq!(size_of::<PadInfo>(), 12);
    }
}
