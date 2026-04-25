//! `rpcs3-ipc-config` — Rust port of `rpcs3/Emu/IPC_config.cpp` + `.h`.
//!
//! Tiny config node: whether RPCS3's IPC server is enabled + which TCP
//! port to bind. Frontends use the port to implement "remote control"
//! APIs — mutating a running instance from QA tooling or scripts.
//!
//! Frozen from the cpp header:
//!
//! - Default: server **disabled**, port `28012`.
//! - Valid port range: `1025..=65535` (`cfg::_int<1025, 65535>`).
//! - Config file name: `ipc.yml` inside the config dir.
//! - Accessors preserve the cpp get/set split so frontends can wire
//!   telemetry or CLI switches.

/// File name inside `config_dir()` (cpp:45).
pub const CONFIG_FILE_NAME: &str = "ipc.yml";

/// Default port when no config file exists (cpp `cfg::_int<1025, 65535> ipc_port{..., 28012}`).
pub const DEFAULT_PORT: u16 = 28012;

/// Inclusive port bounds (cpp `cfg::_int<1025, 65535>`).
pub const MIN_PORT: u16 = 1025;
pub const MAX_PORT: u16 = 65535;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IpcConfig {
    pub server_enabled: bool,
    pub port: u16,
}

impl Default for IpcConfig {
    fn default() -> Self {
        Self { server_enabled: false, port: DEFAULT_PORT }
    }
}

impl IpcConfig {
    /// `set_port(port)` (cpp:63..66) — accepts a port if within range,
    /// rejects otherwise. Returns whether the value was applied.
    pub fn set_port(&mut self, port: u16) -> bool {
        if (MIN_PORT..=MAX_PORT).contains(&port) {
            self.port = port;
            true
        } else {
            false
        }
    }

    /// `set_server_enabled(bool)` (cpp:58..61).
    pub fn set_server_enabled(&mut self, enabled: bool) {
        self.server_enabled = enabled;
    }
}

/// Clamp an arbitrary port value into the valid range (useful for UI
/// slider bindings where overflow/underflow may happen).
#[must_use]
pub const fn clamp_port(port: i32) -> u16 {
    if port < MIN_PORT as i32 {
        MIN_PORT
    } else if port > MAX_PORT as i32 {
        MAX_PORT
    } else {
        port as u16
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_cpp_header() {
        let c = IpcConfig::default();
        assert!(!c.server_enabled);
        assert_eq!(c.port, 28012);
    }

    #[test]
    fn port_bounds_are_1025_to_65535_inclusive() {
        assert_eq!(MIN_PORT, 1025);
        assert_eq!(MAX_PORT, 65535);
    }

    #[test]
    fn config_file_name_constant() {
        assert_eq!(CONFIG_FILE_NAME, "ipc.yml");
    }

    #[test]
    fn set_port_accepts_valid_and_rejects_invalid() {
        let mut c = IpcConfig::default();
        assert!(c.set_port(1025));
        assert_eq!(c.port, 1025);
        assert!(c.set_port(65535));
        assert_eq!(c.port, 65535);
        assert!(c.set_port(28012));
        assert_eq!(c.port, 28012);
        // Out of range — no mutation.
        assert!(!c.set_port(1024));
        assert_eq!(c.port, 28012);
        assert!(!c.set_port(0));
        assert_eq!(c.port, 28012);
    }

    #[test]
    fn set_server_enabled_flips_bit() {
        let mut c = IpcConfig::default();
        c.set_server_enabled(true);
        assert!(c.server_enabled);
        c.set_server_enabled(false);
        assert!(!c.server_enabled);
    }

    #[test]
    fn clamp_port_pins_out_of_range() {
        assert_eq!(clamp_port(0), MIN_PORT);
        assert_eq!(clamp_port(-100), MIN_PORT);
        assert_eq!(clamp_port(1024), MIN_PORT);
        assert_eq!(clamp_port(1025), 1025);
        assert_eq!(clamp_port(28012), 28012);
        assert_eq!(clamp_port(65535), MAX_PORT);
        assert_eq!(clamp_port(100_000), MAX_PORT);
    }
}
