//! `rpcs3-hle-cellnetctl` — network connection status HLE.
//!
//! Ports `rpcs3/Emu/Cell/Modules/cellNetCtl.cpp`. Games call
//! `cellNetCtlInit` at boot and query `cellNetCtlGetState` to know
//! if they have an IP yet. Our implementation is a simple state
//! machine over a [`NetCtlBackend`] (what the host reports) —
//! emulators typically want this to always say "connected" via a
//! stub backend, but online PS3 games need real network.
//!
//! ## Entry points
//!
//! | HLE function                     | Rust wrapper                    |
//! |----------------------------------|---------------------------------|
//! | `cellNetCtlInit`                 | [`cell_net_ctl_init`]           |
//! | `cellNetCtlTerm`                 | [`cell_net_ctl_term`]           |
//! | `cellNetCtlGetState`             | [`cell_net_ctl_get_state`]      |
//! | `cellNetCtlAddHandler`           | [`cell_net_ctl_add_handler`]    |
//! | `cellNetCtlDelHandler`           | [`cell_net_ctl_del_handler`]    |
//! | `cellNetCtlGetInfo`              | [`cell_net_ctl_get_info`]       |
//! | `cellNetCtlNetStartDialogLoadAsync`| [`cell_net_ctl_net_start_dialog_load_async`] |

use rpcs3_emu_types::CellError;

// =====================================================================
// Error codes — byte-exact with cellNetCtl.h:8-42
// =====================================================================

pub mod errors {
    use rpcs3_emu_types::CellError;

    pub const NOT_INITIALIZED: CellError = CellError(0x8013_0101);
    pub const NOT_TERMINATED: CellError = CellError(0x8013_0102);
    pub const HANDLER_MAX: CellError = CellError(0x8013_0103);
    pub const ID_NOT_FOUND: CellError = CellError(0x8013_0104);
    pub const INVALID_ID: CellError = CellError(0x8013_0105);
    pub const INVALID_CODE: CellError = CellError(0x8013_0106);
    pub const INVALID_ADDR: CellError = CellError(0x8013_0107);
    pub const NOT_CONNECTED: CellError = CellError(0x8013_0108);
    pub const NOT_AVAIL: CellError = CellError(0x8013_0109);
    pub const INVALID_TYPE: CellError = CellError(0x8013_010A);
    pub const INVALID_SIZE: CellError = CellError(0x8013_010B);
    pub const NET_DISABLED: CellError = CellError(0x8013_0181);
    pub const NET_NOT_CONNECTED: CellError = CellError(0x8013_0182);
    pub const NP_NO_ACCOUNT: CellError = CellError(0x8013_0183);
    pub const NET_CABLE_NOT_CONNECTED: CellError = CellError(0x8013_0186);
    pub const DIALOG_CANCELED: CellError = CellError(0x8013_0190);
    pub const DIALOG_ABORTED: CellError = CellError(0x8013_0191);
}

// =====================================================================
// State / code constants
// =====================================================================

pub const STATE_DISCONNECTED: u32 = 0;
pub const STATE_CONNECTING: u32 = 1;
pub const STATE_IP_OBTAINING: u32 = 2;
pub const STATE_IP_OBTAINED: u32 = 3;

pub const EVENT_CONNECT_REQ: u32 = 0;
pub const EVENT_ESTABLISH: u32 = 1;
pub const EVENT_GET_IP: u32 = 2;
pub const EVENT_LOST: u32 = 3;
pub const EVENT_DISCONNECT_REQ: u32 = 4;
pub const EVENT_ERROR: u32 = 5;

/// Max concurrent state-change handlers the runtime accepts.
pub const MAX_HANDLERS: u32 = 4;

/// Info codes accepted by `cellNetCtlGetInfo`.
pub const INFO_DEVICE: u32 = 1;
pub const INFO_ETHER_ADDR: u32 = 2;
pub const INFO_MTU: u32 = 3;
pub const INFO_LINK: u32 = 4;
pub const INFO_LINK_TYPE: u32 = 5;
pub const INFO_BSSID: u32 = 6;
pub const INFO_SSID: u32 = 7;
pub const INFO_WLAN_SECURITY: u32 = 8;
pub const INFO_IP_CONFIG: u32 = 13;
pub const INFO_DHCP_HOSTNAME: u32 = 14;
pub const INFO_PPPOE_AUTH_NAME: u32 = 15;
pub const INFO_IP_ADDRESS: u32 = 16;
pub const INFO_NETMASK: u32 = 17;
pub const INFO_DEFAULT_ROUTE: u32 = 18;
pub const INFO_PRIMARY_DNS: u32 = 19;
pub const INFO_SECONDARY_DNS: u32 = 20;

// =====================================================================
// Backend trait
// =====================================================================

/// What the host reports about network status. Games ask via this.
pub trait NetCtlBackend {
    fn is_connected(&self) -> bool;
    /// IPv4 as big-endian u32, or 0 if disconnected.
    fn local_ipv4(&self) -> u32;
    /// Ethernet MAC, 6 bytes.
    fn ether_addr(&self) -> [u8; 6];
    fn mtu(&self) -> u32 { 1500 }
}

#[derive(Debug, Clone, Copy)]
pub struct OfflineBackend;
impl NetCtlBackend for OfflineBackend {
    fn is_connected(&self) -> bool { false }
    fn local_ipv4(&self) -> u32 { 0 }
    fn ether_addr(&self) -> [u8; 6] { [0; 6] }
}

#[derive(Debug, Clone, Copy)]
pub struct StubConnectedBackend {
    pub ip: u32,
    pub mac: [u8; 6],
}
impl NetCtlBackend for StubConnectedBackend {
    fn is_connected(&self) -> bool { true }
    fn local_ipv4(&self) -> u32 { self.ip }
    fn ether_addr(&self) -> [u8; 6] { self.mac }
}

// =====================================================================
// Manager
// =====================================================================

#[derive(Debug, Default)]
pub struct NetCtlManager {
    initialized: bool,
    handlers: std::collections::BTreeMap<u32, u32>, // id → callback slot
    next_handler_id: u32,
}

// =====================================================================
// Syscalls
// =====================================================================

fn ensure_init(m: &NetCtlManager) -> Result<(), CellError> {
    if m.initialized { Ok(()) } else { Err(errors::NOT_INITIALIZED) }
}

#[must_use]
pub fn cell_net_ctl_init(m: &mut NetCtlManager) -> Result<(), CellError> {
    if m.initialized {
        return Err(errors::NOT_TERMINATED);
    }
    m.initialized = true;
    Ok(())
}

#[must_use]
pub fn cell_net_ctl_term(m: &mut NetCtlManager) -> Result<(), CellError> {
    ensure_init(m)?;
    m.initialized = false;
    m.handlers.clear();
    Ok(())
}

/// Info returned by `cellNetCtlGetInfo`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetInfo {
    IpAddress([u8; 4]),
    Netmask([u8; 4]),
    DefaultRoute([u8; 4]),
    PrimaryDns([u8; 4]),
    SecondaryDns([u8; 4]),
    EtherAddr([u8; 6]),
    Mtu(u32),
    LinkUp,
    LinkDown,
}

#[must_use]
pub fn cell_net_ctl_get_state<B: NetCtlBackend + ?Sized>(
    m: &NetCtlManager,
    backend: &B,
) -> Result<u32, CellError> {
    ensure_init(m)?;
    Ok(if backend.is_connected() {
        STATE_IP_OBTAINED
    } else {
        STATE_DISCONNECTED
    })
}

#[must_use]
pub fn cell_net_ctl_add_handler(
    m: &mut NetCtlManager,
    callback_slot: u32,
) -> Result<u32, CellError> {
    ensure_init(m)?;
    if m.handlers.len() as u32 >= MAX_HANDLERS {
        return Err(errors::HANDLER_MAX);
    }
    m.next_handler_id += 1;
    let id = m.next_handler_id;
    m.handlers.insert(id, callback_slot);
    Ok(id)
}

#[must_use]
pub fn cell_net_ctl_del_handler(m: &mut NetCtlManager, id: u32) -> Result<(), CellError> {
    ensure_init(m)?;
    if m.handlers.remove(&id).is_none() {
        return Err(errors::ID_NOT_FOUND);
    }
    Ok(())
}

#[must_use]
pub fn cell_net_ctl_get_info<B: NetCtlBackend + ?Sized>(
    m: &NetCtlManager,
    backend: &B,
    code: u32,
) -> Result<NetInfo, CellError> {
    ensure_init(m)?;
    if !backend.is_connected() && matches!(code, INFO_IP_ADDRESS | INFO_NETMASK | INFO_DEFAULT_ROUTE | INFO_PRIMARY_DNS | INFO_SECONDARY_DNS) {
        return Err(errors::NOT_CONNECTED);
    }
    match code {
        INFO_IP_ADDRESS => Ok(NetInfo::IpAddress(backend.local_ipv4().to_be_bytes())),
        INFO_NETMASK => Ok(NetInfo::Netmask([255, 255, 255, 0])),
        INFO_DEFAULT_ROUTE => {
            let ip = backend.local_ipv4().to_be_bytes();
            Ok(NetInfo::DefaultRoute([ip[0], ip[1], ip[2], 1]))
        }
        INFO_PRIMARY_DNS => Ok(NetInfo::PrimaryDns([8, 8, 8, 8])),
        INFO_SECONDARY_DNS => Ok(NetInfo::SecondaryDns([8, 8, 4, 4])),
        INFO_ETHER_ADDR => Ok(NetInfo::EtherAddr(backend.ether_addr())),
        INFO_MTU => Ok(NetInfo::Mtu(backend.mtu())),
        INFO_LINK => Ok(if backend.is_connected() { NetInfo::LinkUp } else { NetInfo::LinkDown }),
        _ => Err(errors::INVALID_CODE),
    }
}

#[must_use]
pub fn cell_net_ctl_net_start_dialog_load_async(
    m: &NetCtlManager,
) -> Result<(), CellError> {
    ensure_init(m)?;
    Ok(())
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn stub_connected() -> StubConnectedBackend {
        StubConnectedBackend {
            ip: u32::from_be_bytes([192, 168, 1, 42]),
            mac: [0x00, 0xAB, 0xCD, 0xEF, 0x12, 0x34],
        }
    }

    // --- constants ------------------------------------------------

    #[test]
    fn error_codes_match_cpp() {
        assert_eq!(errors::NOT_INITIALIZED.0, 0x8013_0101);
        assert_eq!(errors::NOT_TERMINATED.0, 0x8013_0102);
        assert_eq!(errors::HANDLER_MAX.0, 0x8013_0103);
        assert_eq!(errors::NOT_CONNECTED.0, 0x8013_0108);
        assert_eq!(errors::NET_DISABLED.0, 0x8013_0181);
        assert_eq!(errors::DIALOG_CANCELED.0, 0x8013_0190);
    }

    #[test]
    fn state_constants_match_header() {
        assert_eq!(STATE_DISCONNECTED, 0);
        assert_eq!(STATE_CONNECTING, 1);
        assert_eq!(STATE_IP_OBTAINING, 2);
        assert_eq!(STATE_IP_OBTAINED, 3);
    }

    // --- init / term ----------------------------------------------

    #[test]
    fn init_twice_is_not_terminated() {
        let mut m = NetCtlManager::default();
        cell_net_ctl_init(&mut m).unwrap();
        assert_eq!(cell_net_ctl_init(&mut m).unwrap_err(), errors::NOT_TERMINATED);
    }

    #[test]
    fn term_without_init_is_not_initialized() {
        let mut m = NetCtlManager::default();
        assert_eq!(cell_net_ctl_term(&mut m).unwrap_err(), errors::NOT_INITIALIZED);
    }

    #[test]
    fn term_after_init_allows_reinit() {
        let mut m = NetCtlManager::default();
        cell_net_ctl_init(&mut m).unwrap();
        cell_net_ctl_term(&mut m).unwrap();
        cell_net_ctl_init(&mut m).unwrap();
    }

    // --- get_state ------------------------------------------------

    #[test]
    fn get_state_without_init_is_not_initialized() {
        let m = NetCtlManager::default();
        assert_eq!(
            cell_net_ctl_get_state(&m, &OfflineBackend).unwrap_err(),
            errors::NOT_INITIALIZED,
        );
    }

    #[test]
    fn get_state_offline_backend_disconnected() {
        let mut m = NetCtlManager::default();
        cell_net_ctl_init(&mut m).unwrap();
        assert_eq!(
            cell_net_ctl_get_state(&m, &OfflineBackend).unwrap(),
            STATE_DISCONNECTED,
        );
    }

    #[test]
    fn get_state_connected_backend_ip_obtained() {
        let mut m = NetCtlManager::default();
        cell_net_ctl_init(&mut m).unwrap();
        assert_eq!(
            cell_net_ctl_get_state(&m, &stub_connected()).unwrap(),
            STATE_IP_OBTAINED,
        );
    }

    // --- handlers -------------------------------------------------

    #[test]
    fn add_handler_up_to_max() {
        let mut m = NetCtlManager::default();
        cell_net_ctl_init(&mut m).unwrap();
        for _ in 0..MAX_HANDLERS {
            cell_net_ctl_add_handler(&mut m, 0).unwrap();
        }
        assert_eq!(
            cell_net_ctl_add_handler(&mut m, 0).unwrap_err(),
            errors::HANDLER_MAX,
        );
    }

    #[test]
    fn del_unknown_handler_is_id_not_found() {
        let mut m = NetCtlManager::default();
        cell_net_ctl_init(&mut m).unwrap();
        assert_eq!(
            cell_net_ctl_del_handler(&mut m, 999).unwrap_err(),
            errors::ID_NOT_FOUND,
        );
    }

    #[test]
    fn del_handler_frees_slot() {
        let mut m = NetCtlManager::default();
        cell_net_ctl_init(&mut m).unwrap();
        for _ in 0..MAX_HANDLERS {
            cell_net_ctl_add_handler(&mut m, 0).unwrap();
        }
        // Delete one, should accept a new add.
        cell_net_ctl_del_handler(&mut m, 1).unwrap();
        cell_net_ctl_add_handler(&mut m, 0).unwrap();
    }

    // --- get_info -------------------------------------------------

    #[test]
    fn get_info_ip_disconnected_is_not_connected() {
        let mut m = NetCtlManager::default();
        cell_net_ctl_init(&mut m).unwrap();
        assert_eq!(
            cell_net_ctl_get_info(&m, &OfflineBackend, INFO_IP_ADDRESS).unwrap_err(),
            errors::NOT_CONNECTED,
        );
    }

    #[test]
    fn get_info_ip_connected_returns_bytes() {
        let mut m = NetCtlManager::default();
        cell_net_ctl_init(&mut m).unwrap();
        match cell_net_ctl_get_info(&m, &stub_connected(), INFO_IP_ADDRESS).unwrap() {
            NetInfo::IpAddress(bytes) => assert_eq!(bytes, [192, 168, 1, 42]),
            other => panic!("expected IpAddress, got {other:?}"),
        }
    }

    #[test]
    fn get_info_default_route_uses_subnet_gateway() {
        let mut m = NetCtlManager::default();
        cell_net_ctl_init(&mut m).unwrap();
        match cell_net_ctl_get_info(&m, &stub_connected(), INFO_DEFAULT_ROUTE).unwrap() {
            NetInfo::DefaultRoute(bytes) => assert_eq!(bytes, [192, 168, 1, 1]),
            other => panic!("expected DefaultRoute, got {other:?}"),
        }
    }

    #[test]
    fn get_info_ether_addr_from_backend() {
        let mut m = NetCtlManager::default();
        cell_net_ctl_init(&mut m).unwrap();
        match cell_net_ctl_get_info(&m, &stub_connected(), INFO_ETHER_ADDR).unwrap() {
            NetInfo::EtherAddr(mac) => assert_eq!(mac, [0x00, 0xAB, 0xCD, 0xEF, 0x12, 0x34]),
            other => panic!("expected EtherAddr, got {other:?}"),
        }
    }

    #[test]
    fn get_info_mtu_default_1500() {
        let mut m = NetCtlManager::default();
        cell_net_ctl_init(&mut m).unwrap();
        match cell_net_ctl_get_info(&m, &stub_connected(), INFO_MTU).unwrap() {
            NetInfo::Mtu(v) => assert_eq!(v, 1500),
            other => panic!("expected Mtu, got {other:?}"),
        }
    }

    #[test]
    fn get_info_primary_dns_is_google() {
        let mut m = NetCtlManager::default();
        cell_net_ctl_init(&mut m).unwrap();
        match cell_net_ctl_get_info(&m, &stub_connected(), INFO_PRIMARY_DNS).unwrap() {
            NetInfo::PrimaryDns(bytes) => assert_eq!(bytes, [8, 8, 8, 8]),
            other => panic!("expected PrimaryDns, got {other:?}"),
        }
    }

    #[test]
    fn get_info_invalid_code_is_invalid_code() {
        let mut m = NetCtlManager::default();
        cell_net_ctl_init(&mut m).unwrap();
        assert_eq!(
            cell_net_ctl_get_info(&m, &stub_connected(), 99).unwrap_err(),
            errors::INVALID_CODE,
        );
    }

    // --- dialog stub ----------------------------------------------

    #[test]
    fn dialog_start_is_noop_when_initialized() {
        let mut m = NetCtlManager::default();
        cell_net_ctl_init(&mut m).unwrap();
        cell_net_ctl_net_start_dialog_load_async(&m).unwrap();
    }
}
