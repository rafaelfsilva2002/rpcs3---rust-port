//! `rpcs3-np-upnp-config` — Rust port of `rpcs3/Emu/NP/upnp_config.cpp` + `.h`.
//!
//! Tiny single-field config: the UPnP Internet Gateway Device URL that
//! RPCS3 uses to open ports for PSN/RPCN connectivity. Default is empty
//! (auto-discover on startup).
//!
//! Frozen:
//!
//! - Default value: empty string `""` (cpp `cfg::string device_url{..., ""}`).
//! - Config file name: `upnp.yml` (cpp:54).

pub const CONFIG_FILE_NAME: &str = "upnp.yml";
pub const DEFAULT_DEVICE_URL: &str = "";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UpnpConfig {
    pub device_url: String,
}

impl UpnpConfig {
    /// `get_device_url()` (cpp:42..45).
    pub fn device_url(&self) -> &str {
        &self.device_url
    }

    /// `set_device_url(url)` (cpp:47..50). Copies into the field
    /// unconditionally — caller is responsible for validation.
    pub fn set_device_url(&mut self, url: &str) {
        self.device_url = url.to_string();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_device_url_is_empty() {
        let c = UpnpConfig::default();
        assert_eq!(c.device_url(), DEFAULT_DEVICE_URL);
        assert_eq!(c.device_url(), "");
    }

    #[test]
    fn config_file_name() {
        assert_eq!(CONFIG_FILE_NAME, "upnp.yml");
    }

    #[test]
    fn set_and_get_url() {
        let mut c = UpnpConfig::default();
        c.set_device_url("http://192.168.1.1:1900/gate.xml");
        assert_eq!(c.device_url(), "http://192.168.1.1:1900/gate.xml");
    }

    #[test]
    fn set_empty_accepted() {
        let mut c = UpnpConfig::default();
        c.set_device_url("http://x/");
        c.set_device_url("");
        assert_eq!(c.device_url(), "");
    }
}
