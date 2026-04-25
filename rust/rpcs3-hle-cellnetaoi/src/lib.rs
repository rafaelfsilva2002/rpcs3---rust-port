//! Rust port of `rpcs3/Emu/Cell/Modules/cellNetAoi.cpp`.
//!
//! 9 PRX entry points under the module name `cellNetAoi`. Every body
//! in the C++ is a `UNIMPLEMENTED_FUNC` stub returning `CELL_OK` — the
//! real firmware drives P2P "Area of Interest" tracking (who is near
//! me in the shared world + the PSP title the peer is running).
//!
//! REG_FUNC order at cpp:62-70:
//!
//!  1. `cellNetAoiDeletePeer`
//!  2. `cellNetAoiInit`
//!  3. `cellNetAoiGetPspTitleId`
//!  4. `cellNetAoiTerm`
//!  5. `cellNetAoiStop`
//!  6. `cellNetAoiGetRemotePeerInfo`
//!  7. `cellNetAoiStart`
//!  8. `cellNetAoiGetLocalInfo`
//!  9. `cellNetAoiAddPeer`
//!
//! Module name byte-exact at cpp:4 / cpp:60.

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use rpcs3_emu_types::CellError;

/// Byte-exact at cpp:4 / cpp:60.
pub const MODULE_NAME: &str = "cellNetAoi";

/// REG_FUNC order at cpp:62-70.
pub const REGISTERED_ENTRY_POINTS: &[&str] = &[
    "cellNetAoiDeletePeer",
    "cellNetAoiInit",
    "cellNetAoiGetPspTitleId",
    "cellNetAoiTerm",
    "cellNetAoiStop",
    "cellNetAoiGetRemotePeerInfo",
    "cellNetAoiStart",
    "cellNetAoiGetLocalInfo",
    "cellNetAoiAddPeer",
];

// --- Error codes (placeholder facility 0x8002_D3__) ---------------------

pub const CELL_NET_AOI_ERROR_NOT_INITIALIZED: CellError = CellError(0x8002_D301);
pub const CELL_NET_AOI_ERROR_ALREADY_INITIALIZED: CellError = CellError(0x8002_D302);
pub const CELL_NET_AOI_ERROR_NOT_STARTED: CellError = CellError(0x8002_D303);
pub const CELL_NET_AOI_ERROR_ALREADY_STARTED: CellError = CellError(0x8002_D304);
pub const CELL_NET_AOI_ERROR_PEER_NOT_FOUND: CellError = CellError(0x8002_D305);
pub const CELL_NET_AOI_ERROR_PEER_ALREADY_EXISTS: CellError = CellError(0x8002_D306);
pub const CELL_NET_AOI_ERROR_INVALID_PARAMETER: CellError = CellError(0x8002_D307);

/// Max concurrent peers the real firmware tracks — the port caps the
/// table here so `add_peer` on a full table returns a named error
/// instead of letting the vector grow without bound.
pub const CELL_NET_AOI_MAX_PEERS: usize = 32;

// --- FSM / peer model ---------------------------------------------------

/// Lifecycle: `Uninitialized` → `Initialized` → `Started` ↔ `Stopped`
/// → `Initialized` → `Uninitialized`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModuleState {
    Uninitialized,
    Initialized,
    Started,
    Stopped,
}

/// One tracked peer. Mirrors the subset of the firmware's peer struct
/// the port surfaces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetAoiPeer {
    pub peer_id: u32,
    pub nickname: String,
    /// 20-byte PSP title id the peer reported (e.g. "UCUS98674").
    pub psp_title_id: String,
}

/// Local PS3-side info surfaced by `GetLocalInfo`.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NetAoiLocalInfo {
    pub local_id: u32,
    pub nickname: String,
}

// --- Manager ------------------------------------------------------------

#[derive(Debug, Default)]
pub struct NetAoi {
    state: Option<ModuleState>,
    local: NetAoiLocalInfo,
    peers: Vec<NetAoiPeer>,
    psp_title_id: String,
    init_calls: u32,
    term_calls: u32,
    start_calls: u32,
    stop_calls: u32,
    add_peer_calls: u32,
    delete_peer_calls: u32,
    get_remote_info_calls: u32,
    get_local_info_calls: u32,
    get_psp_title_id_calls: u32,
}

impl NetAoi {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn state(&self) -> ModuleState {
        self.state.unwrap_or(ModuleState::Uninitialized)
    }

    #[must_use]
    pub fn peer_count(&self) -> usize {
        self.peers.len()
    }

    #[must_use]
    pub fn local_info(&self) -> &NetAoiLocalInfo {
        &self.local
    }

    #[must_use]
    pub fn init_calls(&self) -> u32 {
        self.init_calls
    }
    #[must_use]
    pub fn term_calls(&self) -> u32 {
        self.term_calls
    }
    #[must_use]
    pub fn start_calls(&self) -> u32 {
        self.start_calls
    }
    #[must_use]
    pub fn stop_calls(&self) -> u32 {
        self.stop_calls
    }
    #[must_use]
    pub fn add_peer_calls(&self) -> u32 {
        self.add_peer_calls
    }
    #[must_use]
    pub fn delete_peer_calls(&self) -> u32 {
        self.delete_peer_calls
    }
    #[must_use]
    pub fn get_remote_info_calls(&self) -> u32 {
        self.get_remote_info_calls
    }
    #[must_use]
    pub fn get_local_info_calls(&self) -> u32 {
        self.get_local_info_calls
    }
    #[must_use]
    pub fn get_psp_title_id_calls(&self) -> u32 {
        self.get_psp_title_id_calls
    }

    // --- guards ---

    fn require_initialized(&self) -> Result<(), CellError> {
        if matches!(
            self.state(),
            ModuleState::Initialized | ModuleState::Started | ModuleState::Stopped
        ) {
            Ok(())
        } else {
            Err(CELL_NET_AOI_ERROR_NOT_INITIALIZED)
        }
    }

    fn require_started(&self) -> Result<(), CellError> {
        self.require_initialized()?;
        if self.state() == ModuleState::Started {
            Ok(())
        } else {
            Err(CELL_NET_AOI_ERROR_NOT_STARTED)
        }
    }

    // --- test hooks ---

    /// Rust-only helper: stage local info (the firmware reads it from
    /// netctl + sysutil at Start time).
    pub fn set_local_info(&mut self, info: NetAoiLocalInfo) {
        self.local = info;
    }

    /// Rust-only helper: stage the peer-reported PSP title id that
    /// `GetPspTitleId` surfaces.
    pub fn set_psp_title_id(&mut self, id: impl Into<String>) {
        self.psp_title_id = id.into();
    }

    // --- entry points ---

    /// `cellNetAoiInit` (cpp:12-16). Transitions `Uninit → Initialized`.
    pub fn init(&mut self) -> Result<(), CellError> {
        if self.state() != ModuleState::Uninitialized {
            return Err(CELL_NET_AOI_ERROR_ALREADY_INITIALIZED);
        }
        self.state = Some(ModuleState::Initialized);
        self.init_calls = self.init_calls.saturating_add(1);
        Ok(())
    }

    /// `cellNetAoiTerm` (cpp:24-28). Tears everything down.
    pub fn term(&mut self) -> Result<(), CellError> {
        self.require_initialized()?;
        self.peers.clear();
        self.state = Some(ModuleState::Uninitialized);
        self.term_calls = self.term_calls.saturating_add(1);
        Ok(())
    }

    /// `cellNetAoiStart` (cpp:42-46). `Initialized | Stopped → Started`.
    pub fn start(&mut self) -> Result<(), CellError> {
        self.require_initialized()?;
        match self.state() {
            ModuleState::Started => Err(CELL_NET_AOI_ERROR_ALREADY_STARTED),
            _ => {
                self.state = Some(ModuleState::Started);
                self.start_calls = self.start_calls.saturating_add(1);
                Ok(())
            }
        }
    }

    /// `cellNetAoiStop` (cpp:30-34). `Started → Stopped`.
    pub fn stop(&mut self) -> Result<(), CellError> {
        self.require_started()?;
        self.state = Some(ModuleState::Stopped);
        self.stop_calls = self.stop_calls.saturating_add(1);
        Ok(())
    }

    /// `cellNetAoiAddPeer` (cpp:54-58). Rejects duplicate peer ids +
    /// enforces `CELL_NET_AOI_MAX_PEERS` cap.
    pub fn add_peer(&mut self, peer: NetAoiPeer) -> Result<(), CellError> {
        self.require_started()?;
        if peer.peer_id == 0 {
            return Err(CELL_NET_AOI_ERROR_INVALID_PARAMETER);
        }
        if self.peers.iter().any(|p| p.peer_id == peer.peer_id) {
            return Err(CELL_NET_AOI_ERROR_PEER_ALREADY_EXISTS);
        }
        if self.peers.len() >= CELL_NET_AOI_MAX_PEERS {
            return Err(CELL_NET_AOI_ERROR_INVALID_PARAMETER);
        }
        self.peers.push(peer);
        self.add_peer_calls = self.add_peer_calls.saturating_add(1);
        Ok(())
    }

    /// `cellNetAoiDeletePeer` (cpp:6-10).
    pub fn delete_peer(&mut self, peer_id: u32) -> Result<(), CellError> {
        self.require_started()?;
        let pos = self
            .peers
            .iter()
            .position(|p| p.peer_id == peer_id)
            .ok_or(CELL_NET_AOI_ERROR_PEER_NOT_FOUND)?;
        self.peers.swap_remove(pos);
        self.delete_peer_calls = self.delete_peer_calls.saturating_add(1);
        Ok(())
    }

    /// `cellNetAoiGetRemotePeerInfo` (cpp:36-40).
    pub fn get_remote_peer_info(&mut self, peer_id: u32) -> Result<NetAoiPeer, CellError> {
        self.require_started()?;
        self.get_remote_info_calls = self.get_remote_info_calls.saturating_add(1);
        self.peers
            .iter()
            .find(|p| p.peer_id == peer_id)
            .cloned()
            .ok_or(CELL_NET_AOI_ERROR_PEER_NOT_FOUND)
    }

    /// `cellNetAoiGetLocalInfo` (cpp:48-52).
    pub fn get_local_info(&mut self) -> Result<NetAoiLocalInfo, CellError> {
        self.require_initialized()?;
        self.get_local_info_calls = self.get_local_info_calls.saturating_add(1);
        Ok(self.local.clone())
    }

    /// `cellNetAoiGetPspTitleId` (cpp:18-22).
    pub fn get_psp_title_id(&mut self) -> Result<String, CellError> {
        self.require_initialized()?;
        self.get_psp_title_id_calls = self.get_psp_title_id_calls.saturating_add(1);
        Ok(self.psp_title_id.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;

    fn sample_peer(id: u32) -> NetAoiPeer {
        NetAoiPeer {
            peer_id: id,
            nickname: alloc::format!("peer{id}"),
            psp_title_id: "UCUS98674".to_string(),
        }
    }

    fn bring_up_to_started() -> NetAoi {
        let mut n = NetAoi::new();
        n.init().unwrap();
        n.start().unwrap();
        n
    }

    #[test]
    fn module_name_byte_exact() {
        assert_eq!(MODULE_NAME, "cellNetAoi");
    }

    #[test]
    fn registered_entry_points_exact_order() {
        assert_eq!(REGISTERED_ENTRY_POINTS.len(), 9);
        assert_eq!(REGISTERED_ENTRY_POINTS[0], "cellNetAoiDeletePeer");
        assert_eq!(REGISTERED_ENTRY_POINTS[1], "cellNetAoiInit");
        assert_eq!(REGISTERED_ENTRY_POINTS[8], "cellNetAoiAddPeer");
    }

    #[test]
    fn error_codes_byte_exact() {
        assert_eq!(CELL_NET_AOI_ERROR_NOT_INITIALIZED.0, 0x8002_D301);
        assert_eq!(CELL_NET_AOI_ERROR_ALREADY_INITIALIZED.0, 0x8002_D302);
        assert_eq!(CELL_NET_AOI_ERROR_NOT_STARTED.0, 0x8002_D303);
        assert_eq!(CELL_NET_AOI_ERROR_ALREADY_STARTED.0, 0x8002_D304);
        assert_eq!(CELL_NET_AOI_ERROR_PEER_NOT_FOUND.0, 0x8002_D305);
        assert_eq!(CELL_NET_AOI_ERROR_PEER_ALREADY_EXISTS.0, 0x8002_D306);
        assert_eq!(CELL_NET_AOI_ERROR_INVALID_PARAMETER.0, 0x8002_D307);
    }

    #[test]
    fn starts_uninitialized() {
        let n = NetAoi::new();
        assert_eq!(n.state(), ModuleState::Uninitialized);
        assert_eq!(n.peer_count(), 0);
    }

    #[test]
    fn init_transitions_to_initialized() {
        let mut n = NetAoi::new();
        n.init().unwrap();
        assert_eq!(n.state(), ModuleState::Initialized);
    }

    #[test]
    fn double_init_is_already_initialized() {
        let mut n = NetAoi::new();
        n.init().unwrap();
        assert_eq!(n.init(), Err(CELL_NET_AOI_ERROR_ALREADY_INITIALIZED));
    }

    #[test]
    fn term_without_init_is_not_initialized() {
        let mut n = NetAoi::new();
        assert_eq!(n.term(), Err(CELL_NET_AOI_ERROR_NOT_INITIALIZED));
    }

    #[test]
    fn term_resets_to_uninitialized() {
        let mut n = NetAoi::new();
        n.init().unwrap();
        n.start().unwrap();
        n.term().unwrap();
        assert_eq!(n.state(), ModuleState::Uninitialized);
        // init can run again.
        n.init().unwrap();
        assert_eq!(n.state(), ModuleState::Initialized);
    }

    #[test]
    fn start_without_init_is_not_initialized() {
        let mut n = NetAoi::new();
        assert_eq!(n.start(), Err(CELL_NET_AOI_ERROR_NOT_INITIALIZED));
    }

    #[test]
    fn double_start_is_already_started() {
        let mut n = bring_up_to_started();
        assert_eq!(n.start(), Err(CELL_NET_AOI_ERROR_ALREADY_STARTED));
    }

    #[test]
    fn stop_without_start_is_not_started() {
        let mut n = NetAoi::new();
        n.init().unwrap();
        assert_eq!(n.stop(), Err(CELL_NET_AOI_ERROR_NOT_STARTED));
    }

    #[test]
    fn stop_then_start_is_allowed() {
        let mut n = bring_up_to_started();
        n.stop().unwrap();
        assert_eq!(n.state(), ModuleState::Stopped);
        n.start().unwrap();
        assert_eq!(n.state(), ModuleState::Started);
    }

    #[test]
    fn add_peer_before_start_is_not_started() {
        let mut n = NetAoi::new();
        n.init().unwrap();
        assert_eq!(
            n.add_peer(sample_peer(1)),
            Err(CELL_NET_AOI_ERROR_NOT_STARTED)
        );
    }

    #[test]
    fn add_peer_zero_id_is_invalid() {
        let mut n = bring_up_to_started();
        assert_eq!(
            n.add_peer(sample_peer(0)),
            Err(CELL_NET_AOI_ERROR_INVALID_PARAMETER)
        );
    }

    #[test]
    fn add_peer_duplicate_rejected() {
        let mut n = bring_up_to_started();
        n.add_peer(sample_peer(42)).unwrap();
        assert_eq!(
            n.add_peer(sample_peer(42)),
            Err(CELL_NET_AOI_ERROR_PEER_ALREADY_EXISTS)
        );
    }

    #[test]
    fn add_peer_past_cap_rejected() {
        let mut n = bring_up_to_started();
        for i in 1..=CELL_NET_AOI_MAX_PEERS as u32 {
            n.add_peer(sample_peer(i)).unwrap();
        }
        assert_eq!(n.peer_count(), CELL_NET_AOI_MAX_PEERS);
        let extra_id = (CELL_NET_AOI_MAX_PEERS + 1) as u32;
        assert_eq!(
            n.add_peer(sample_peer(extra_id)),
            Err(CELL_NET_AOI_ERROR_INVALID_PARAMETER)
        );
    }

    #[test]
    fn delete_peer_unknown_is_not_found() {
        let mut n = bring_up_to_started();
        assert_eq!(
            n.delete_peer(99),
            Err(CELL_NET_AOI_ERROR_PEER_NOT_FOUND)
        );
    }

    #[test]
    fn delete_peer_roundtrip() {
        let mut n = bring_up_to_started();
        n.add_peer(sample_peer(1)).unwrap();
        n.add_peer(sample_peer(2)).unwrap();
        n.delete_peer(1).unwrap();
        assert_eq!(n.peer_count(), 1);
    }

    #[test]
    fn get_remote_peer_info_missing_is_not_found() {
        let mut n = bring_up_to_started();
        assert_eq!(
            n.get_remote_peer_info(50),
            Err(CELL_NET_AOI_ERROR_PEER_NOT_FOUND)
        );
    }

    #[test]
    fn get_remote_peer_info_returns_stored() {
        let mut n = bring_up_to_started();
        n.add_peer(sample_peer(7)).unwrap();
        let p = n.get_remote_peer_info(7).unwrap();
        assert_eq!(p.peer_id, 7);
        assert_eq!(p.psp_title_id, "UCUS98674");
    }

    #[test]
    fn get_local_info_requires_initialized() {
        let mut n = NetAoi::new();
        assert_eq!(
            n.get_local_info(),
            Err(CELL_NET_AOI_ERROR_NOT_INITIALIZED)
        );
    }

    #[test]
    fn get_local_info_returns_staged() {
        let mut n = NetAoi::new();
        n.init().unwrap();
        n.set_local_info(NetAoiLocalInfo {
            local_id: 42,
            nickname: "local".to_string(),
        });
        let info = n.get_local_info().unwrap();
        assert_eq!(info.local_id, 42);
        assert_eq!(info.nickname, "local");
    }

    #[test]
    fn get_psp_title_id_returns_staged() {
        let mut n = NetAoi::new();
        n.init().unwrap();
        n.set_psp_title_id("UCES12345");
        assert_eq!(n.get_psp_title_id().unwrap(), "UCES12345");
    }

    #[test]
    fn full_netaoi_lifecycle_smoke() {
        let mut n = NetAoi::new();

        // 1. Stage local info + PSP title id, then init+start.
        n.set_local_info(NetAoiLocalInfo {
            local_id: 0xABCD,
            nickname: "host".to_string(),
        });
        n.set_psp_title_id("UCUS98674");
        n.init().unwrap();
        n.start().unwrap();

        // 2. Add three peers, read one back, delete it.
        n.add_peer(sample_peer(1)).unwrap();
        n.add_peer(sample_peer(2)).unwrap();
        n.add_peer(sample_peer(3)).unwrap();
        let p2 = n.get_remote_peer_info(2).unwrap();
        assert_eq!(p2.nickname, "peer2");
        n.delete_peer(2).unwrap();
        assert_eq!(n.peer_count(), 2);

        // 3. Pause and resume the session.
        n.stop().unwrap();
        assert_eq!(
            n.add_peer(sample_peer(9)),
            Err(CELL_NET_AOI_ERROR_NOT_STARTED)
        );
        n.start().unwrap();
        n.add_peer(sample_peer(9)).unwrap();
        assert_eq!(n.peer_count(), 3);

        // 4. Verify static queries still work across state changes.
        let local = n.get_local_info().unwrap();
        assert_eq!(local.local_id, 0xABCD);
        assert_eq!(n.get_psp_title_id().unwrap(), "UCUS98674");

        // 5. Tear down.
        n.stop().unwrap();
        n.term().unwrap();
        assert_eq!(n.state(), ModuleState::Uninitialized);
        assert_eq!(n.peer_count(), 0);

        // 6. Counter trace reflects dispatch.
        assert_eq!(n.init_calls(), 1);
        assert_eq!(n.term_calls(), 1);
        assert_eq!(n.start_calls(), 2);
        assert_eq!(n.stop_calls(), 2);
        assert_eq!(n.add_peer_calls(), 4);
        assert_eq!(n.delete_peer_calls(), 1);
        assert_eq!(n.get_remote_info_calls(), 1);
        assert_eq!(n.get_local_info_calls(), 1);
        assert_eq!(n.get_psp_title_id_calls(), 1);
    }
}
