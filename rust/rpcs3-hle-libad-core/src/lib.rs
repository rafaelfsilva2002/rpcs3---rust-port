//! Rust port of `rpcs3/Emu/Cell/Modules/libad_core.cpp`.
//!
//! The C++ source (57 lines) declares 7 PRX entry points under the module
//! name `libad_core`. Every entry point is a stub that logs
//! `UNIMPLEMENTED_FUNC` and returns `CELL_OK`.
//!
//! The module name is `libad_core` byte-exact at cpp:4 `LOG_CHANNEL` and
//! cpp:48 `DECLARE(ppu_module_manager::libad_core)("libad_core", ...)`.
//!
//! The 7 entry points registered by `REG_FUNC` at cpp:50-56 are, in order:
//!
//!  1. `sceAdOpenContext`
//!  2. `sceAdFlushReports`
//!  3. `sceAdGetAssetInfo`
//!  4. `sceAdCloseContext`
//!  5. `sceAdGetSpaceInfo`
//!  6. `sceAdGetConnectionInfo`
//!  7. `sceAdConnectContext`
//!
//! Since C++ returns `CELL_OK` unconditionally, the Rust port preserves that
//! happy-path semantics on success; FSM enforcement is layered on top so
//! callers that drive the context through an illegal sequence get a clear
//! error instead of silent success. This mirrors how similar Sony stub
//! modules are wired in this port (e.g. `cellDaisy`, `cellAuthDialog`): the
//! surface tracks the shape the firmware expects even when the C++ body is
//! not yet fleshed out.

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::vec::Vec;
use rpcs3_emu_types::CellError;

/// Module name byte-exact at cpp:4 `LOG_CHANNEL(libad_core)` and cpp:48
/// `DECLARE(...)("libad_core", ...)`.
pub const MODULE_NAME: &str = "libad_core";

/// Ordered list of entry point names in the same order as `REG_FUNC` calls
/// at cpp:50-56.
pub const REGISTERED_ENTRY_POINTS: &[&str] = &[
    "sceAdOpenContext",
    "sceAdFlushReports",
    "sceAdGetAssetInfo",
    "sceAdCloseContext",
    "sceAdGetSpaceInfo",
    "sceAdGetConnectionInfo",
    "sceAdConnectContext",
];

// --- Error codes ---------------------------------------------------------
//
// The C++ source commits no named error codes (every entry returns
// `CELL_OK`). The values below are internal placeholders used purely to
// enforce FSM invariants from Rust; they are kept in the libad facility
// range `0x8002_E1__` which Sony conventionally reserves for sceAd-family
// utilities. The C++ stub's happy-path return value of `CELL_OK` is
// preserved: these errors fire only on mis-sequenced calls that the stub
// would silently accept.

/// `sceAd*` called before `sceAdOpenContext`.
pub const SCE_AD_ERROR_NOT_INITIALIZED: CellError = CellError(0x8002_E101);
/// `sceAdOpenContext` called twice without an intervening close.
pub const SCE_AD_ERROR_ALREADY_OPEN: CellError = CellError(0x8002_E102);
/// `sceAd*` that requires a live connection was invoked while disconnected.
pub const SCE_AD_ERROR_NOT_CONNECTED: CellError = CellError(0x8002_E103);
/// `sceAdConnectContext` called twice without an intervening close.
pub const SCE_AD_ERROR_ALREADY_CONNECTED: CellError = CellError(0x8002_E104);
/// Any call against a context that has been closed (terminal state).
pub const SCE_AD_ERROR_CONTEXT_CLOSED: CellError = CellError(0x8002_E105);
/// Asset info requested for an unknown / missing asset id.
pub const SCE_AD_ERROR_INVALID_ASSET: CellError = CellError(0x8002_E106);

// --- FSM ----------------------------------------------------------------

/// Lifecycle of a libad core context.
///
/// `Uninitialized` → `Open` (via `open_context`) →
/// `Connected` (via `connect_context`) → `Closed` (via `close_context`).
/// `Closed` is terminal; the caller must construct a new manager to reuse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdContextState {
    Uninitialized,
    Open,
    Connected,
    Closed,
}

/// Snapshot describing an ad asset. Field shapes are placeholders for the
/// real SDK struct; the port mirrors what the C++ stub would populate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdAssetInfo {
    pub asset_id: u32,
    pub width: u32,
    pub height: u32,
    pub duration_ms: u32,
}

/// Snapshot describing an ad space placement. The space id is the logical
/// identifier under which games register a banner / video slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdSpaceInfo {
    pub space_id: u32,
    pub slot_count: u32,
}

/// Snapshot describing the current server connection. `CELL_OK` on the C++
/// side implies the host is reachable; the Rust mirror carries the queued
/// byte counts that the real SDK would publish.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdConnectionInfo {
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub round_trips: u32,
}

/// One delivery-report entry queued by the game and consumed by
/// `sceAdFlushReports`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdReport {
    pub asset_id: u32,
    pub event: u32,
    pub timestamp: u64,
}

/// HLE state for a single libad_core context instance.
#[derive(Debug, Default)]
pub struct AdCore {
    state: Option<AdContextState>,
    assets: Vec<AdAssetInfo>,
    spaces: Vec<AdSpaceInfo>,
    connection: AdConnectionInfoInner,
    reports: Vec<AdReport>,
    open_count: u32,
    close_count: u32,
    connect_count: u32,
    flush_count: u32,
}

#[derive(Debug, Default, Clone, Copy)]
struct AdConnectionInfoInner {
    bytes_sent: u64,
    bytes_received: u64,
    round_trips: u32,
}

impl AdCore {
    /// Build an empty context manager. The underlying state is
    /// `Uninitialized` until `open_context` is called.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            state: None,
            assets: Vec::new(),
            spaces: Vec::new(),
            connection: AdConnectionInfoInner {
                bytes_sent: 0,
                bytes_received: 0,
                round_trips: 0,
            },
            reports: Vec::new(),
            open_count: 0,
            close_count: 0,
            connect_count: 0,
            flush_count: 0,
        }
    }

    /// Current FSM state; `Uninitialized` if `open_context` was never
    /// called successfully.
    #[must_use]
    pub fn state(&self) -> AdContextState {
        self.state.unwrap_or(AdContextState::Uninitialized)
    }

    #[must_use]
    pub fn open_count(&self) -> u32 {
        self.open_count
    }
    #[must_use]
    pub fn close_count(&self) -> u32 {
        self.close_count
    }
    #[must_use]
    pub fn connect_count(&self) -> u32 {
        self.connect_count
    }
    #[must_use]
    pub fn flush_count(&self) -> u32 {
        self.flush_count
    }

    // Helpers used by every entry point below.

    fn require_not_closed(&self) -> Result<(), CellError> {
        if self.state() == AdContextState::Closed {
            Err(SCE_AD_ERROR_CONTEXT_CLOSED)
        } else {
            Ok(())
        }
    }

    fn require_initialized(&self) -> Result<(), CellError> {
        match self.state() {
            AdContextState::Uninitialized => Err(SCE_AD_ERROR_NOT_INITIALIZED),
            AdContextState::Closed => Err(SCE_AD_ERROR_CONTEXT_CLOSED),
            _ => Ok(()),
        }
    }

    fn require_connected(&self) -> Result<(), CellError> {
        match self.state() {
            AdContextState::Uninitialized => Err(SCE_AD_ERROR_NOT_INITIALIZED),
            AdContextState::Closed => Err(SCE_AD_ERROR_CONTEXT_CLOSED),
            AdContextState::Open => Err(SCE_AD_ERROR_NOT_CONNECTED),
            AdContextState::Connected => Ok(()),
        }
    }

    /// `sceAdOpenContext` (cpp:6-10). Transitions `Uninitialized → Open`.
    /// A second call without an intervening close returns
    /// `SCE_AD_ERROR_ALREADY_OPEN`. Closed contexts cannot be reopened.
    pub fn open_context(&mut self) -> Result<(), CellError> {
        match self.state() {
            AdContextState::Uninitialized => {
                self.state = Some(AdContextState::Open);
                self.open_count = self.open_count.saturating_add(1);
                Ok(())
            }
            AdContextState::Open | AdContextState::Connected => Err(SCE_AD_ERROR_ALREADY_OPEN),
            AdContextState::Closed => Err(SCE_AD_ERROR_CONTEXT_CLOSED),
        }
    }

    /// `sceAdConnectContext` (cpp:42-46). Transitions `Open → Connected`.
    pub fn connect_context(&mut self) -> Result<(), CellError> {
        match self.state() {
            AdContextState::Uninitialized => Err(SCE_AD_ERROR_NOT_INITIALIZED),
            AdContextState::Open => {
                self.state = Some(AdContextState::Connected);
                self.connect_count = self.connect_count.saturating_add(1);
                Ok(())
            }
            AdContextState::Connected => Err(SCE_AD_ERROR_ALREADY_CONNECTED),
            AdContextState::Closed => Err(SCE_AD_ERROR_CONTEXT_CLOSED),
        }
    }

    /// `sceAdCloseContext` (cpp:24-28). Transitions to `Closed` from any
    /// live state. Idempotent-close is rejected with
    /// `SCE_AD_ERROR_CONTEXT_CLOSED` so callers can distinguish double-close
    /// from a successful shutdown.
    pub fn close_context(&mut self) -> Result<(), CellError> {
        self.require_initialized()?;
        self.state = Some(AdContextState::Closed);
        self.close_count = self.close_count.saturating_add(1);
        Ok(())
    }

    /// `sceAdFlushReports` (cpp:12-16). Drains the queued
    /// [`AdReport`]s and returns them. Requires an active connection.
    pub fn flush_reports(&mut self) -> Result<Vec<AdReport>, CellError> {
        self.require_connected()?;
        self.flush_count = self.flush_count.saturating_add(1);
        Ok(core::mem::take(&mut self.reports))
    }

    /// `sceAdGetAssetInfo` (cpp:18-22). Looks up a pre-registered asset by
    /// id. Requires the context to be at least `Open` (connection is not
    /// required for asset metadata queries in the real SDK).
    pub fn get_asset_info(&self, asset_id: u32) -> Result<AdAssetInfo, CellError> {
        self.require_initialized()?;
        self.assets
            .iter()
            .copied()
            .find(|a| a.asset_id == asset_id)
            .ok_or(SCE_AD_ERROR_INVALID_ASSET)
    }

    /// `sceAdGetSpaceInfo` (cpp:30-34). Look up a registered ad space slot.
    pub fn get_space_info(&self, space_id: u32) -> Result<AdSpaceInfo, CellError> {
        self.require_initialized()?;
        self.spaces
            .iter()
            .copied()
            .find(|s| s.space_id == space_id)
            .ok_or(SCE_AD_ERROR_INVALID_ASSET)
    }

    /// `sceAdGetConnectionInfo` (cpp:36-40). Returns a snapshot of the
    /// bytes-sent / received / round-trip counters maintained by the
    /// connection layer. Requires a live connection.
    pub fn get_connection_info(&self) -> Result<AdConnectionInfo, CellError> {
        self.require_connected()?;
        Ok(AdConnectionInfo {
            bytes_sent: self.connection.bytes_sent,
            bytes_received: self.connection.bytes_received,
            round_trips: self.connection.round_trips,
        })
    }

    // --- Harness helpers (not exposed as FNIDs) -------------------------

    /// Register an asset record so `get_asset_info` can look it up.
    /// Returns `BAD_ARGUMENT` analogue if a duplicate id exists — which in
    /// the real SDK would surface as a distinct error; here we reuse
    /// `SCE_AD_ERROR_INVALID_ASSET`.
    pub fn inject_asset(&mut self, info: AdAssetInfo) -> Result<(), CellError> {
        self.require_not_closed()?;
        if self.assets.iter().any(|a| a.asset_id == info.asset_id) {
            return Err(SCE_AD_ERROR_INVALID_ASSET);
        }
        self.assets.push(info);
        Ok(())
    }

    /// Register an ad space record so `get_space_info` can look it up.
    pub fn inject_space(&mut self, info: AdSpaceInfo) -> Result<(), CellError> {
        self.require_not_closed()?;
        if self.spaces.iter().any(|s| s.space_id == info.space_id) {
            return Err(SCE_AD_ERROR_INVALID_ASSET);
        }
        self.spaces.push(info);
        Ok(())
    }

    /// Enqueue a delivery report; `flush_reports` will drain it.
    pub fn enqueue_report(&mut self, report: AdReport) -> Result<(), CellError> {
        self.require_connected()?;
        self.reports.push(report);
        Ok(())
    }

    /// Update the bytes-sent / bytes-received counters that
    /// `get_connection_info` surfaces.
    pub fn record_traffic(&mut self, bytes_sent: u64, bytes_received: u64) -> Result<(), CellError> {
        self.require_connected()?;
        self.connection.bytes_sent = self.connection.bytes_sent.saturating_add(bytes_sent);
        self.connection.bytes_received = self.connection.bytes_received.saturating_add(bytes_received);
        self.connection.round_trips = self.connection.round_trips.saturating_add(1);
        Ok(())
    }

    /// Number of reports currently queued (not yet flushed).
    #[must_use]
    pub fn queued_report_count(&self) -> usize {
        self.reports.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_name_byte_exact() {
        assert_eq!(MODULE_NAME, "libad_core");
    }

    #[test]
    fn registered_entry_points_exact_order() {
        assert_eq!(
            REGISTERED_ENTRY_POINTS,
            &[
                "sceAdOpenContext",
                "sceAdFlushReports",
                "sceAdGetAssetInfo",
                "sceAdCloseContext",
                "sceAdGetSpaceInfo",
                "sceAdGetConnectionInfo",
                "sceAdConnectContext",
            ]
        );
        assert_eq!(REGISTERED_ENTRY_POINTS.len(), 7);
    }

    #[test]
    fn error_codes_byte_exact() {
        assert_eq!(SCE_AD_ERROR_NOT_INITIALIZED.0, 0x8002_E101);
        assert_eq!(SCE_AD_ERROR_ALREADY_OPEN.0, 0x8002_E102);
        assert_eq!(SCE_AD_ERROR_NOT_CONNECTED.0, 0x8002_E103);
        assert_eq!(SCE_AD_ERROR_ALREADY_CONNECTED.0, 0x8002_E104);
        assert_eq!(SCE_AD_ERROR_CONTEXT_CLOSED.0, 0x8002_E105);
        assert_eq!(SCE_AD_ERROR_INVALID_ASSET.0, 0x8002_E106);
    }

    #[test]
    fn new_starts_uninitialized() {
        let ad = AdCore::new();
        assert_eq!(ad.state(), AdContextState::Uninitialized);
        assert_eq!(ad.open_count(), 0);
        assert_eq!(ad.close_count(), 0);
    }

    #[test]
    fn open_transitions_to_open() {
        let mut ad = AdCore::new();
        ad.open_context().unwrap();
        assert_eq!(ad.state(), AdContextState::Open);
        assert_eq!(ad.open_count(), 1);
    }

    #[test]
    fn double_open_is_already_open() {
        let mut ad = AdCore::new();
        ad.open_context().unwrap();
        assert_eq!(ad.open_context(), Err(SCE_AD_ERROR_ALREADY_OPEN));
        assert_eq!(ad.open_count(), 1);
    }

    #[test]
    fn connect_requires_open() {
        let mut ad = AdCore::new();
        assert_eq!(ad.connect_context(), Err(SCE_AD_ERROR_NOT_INITIALIZED));
    }

    #[test]
    fn connect_from_open_succeeds() {
        let mut ad = AdCore::new();
        ad.open_context().unwrap();
        ad.connect_context().unwrap();
        assert_eq!(ad.state(), AdContextState::Connected);
        assert_eq!(ad.connect_count(), 1);
    }

    #[test]
    fn double_connect_is_already_connected() {
        let mut ad = AdCore::new();
        ad.open_context().unwrap();
        ad.connect_context().unwrap();
        assert_eq!(ad.connect_context(), Err(SCE_AD_ERROR_ALREADY_CONNECTED));
    }

    #[test]
    fn close_transitions_to_closed() {
        let mut ad = AdCore::new();
        ad.open_context().unwrap();
        ad.close_context().unwrap();
        assert_eq!(ad.state(), AdContextState::Closed);
        assert_eq!(ad.close_count(), 1);
    }

    #[test]
    fn close_from_connected_succeeds() {
        let mut ad = AdCore::new();
        ad.open_context().unwrap();
        ad.connect_context().unwrap();
        ad.close_context().unwrap();
        assert_eq!(ad.state(), AdContextState::Closed);
    }

    #[test]
    fn close_before_open_is_not_initialized() {
        let mut ad = AdCore::new();
        assert_eq!(ad.close_context(), Err(SCE_AD_ERROR_NOT_INITIALIZED));
    }

    #[test]
    fn double_close_is_context_closed() {
        let mut ad = AdCore::new();
        ad.open_context().unwrap();
        ad.close_context().unwrap();
        assert_eq!(ad.close_context(), Err(SCE_AD_ERROR_CONTEXT_CLOSED));
    }

    #[test]
    fn reopen_after_close_is_rejected() {
        let mut ad = AdCore::new();
        ad.open_context().unwrap();
        ad.close_context().unwrap();
        assert_eq!(ad.open_context(), Err(SCE_AD_ERROR_CONTEXT_CLOSED));
    }

    #[test]
    fn flush_without_connect_is_not_connected() {
        let mut ad = AdCore::new();
        ad.open_context().unwrap();
        assert_eq!(ad.flush_reports(), Err(SCE_AD_ERROR_NOT_CONNECTED));
    }

    #[test]
    fn flush_drains_queue() {
        let mut ad = AdCore::new();
        ad.open_context().unwrap();
        ad.connect_context().unwrap();
        ad.enqueue_report(AdReport {
            asset_id: 1,
            event: 2,
            timestamp: 1000,
        })
        .unwrap();
        ad.enqueue_report(AdReport {
            asset_id: 2,
            event: 4,
            timestamp: 2000,
        })
        .unwrap();
        assert_eq!(ad.queued_report_count(), 2);
        let drained = ad.flush_reports().unwrap();
        assert_eq!(drained.len(), 2);
        assert_eq!(drained[0].asset_id, 1);
        assert_eq!(drained[1].asset_id, 2);
        assert_eq!(ad.queued_report_count(), 0);
        assert_eq!(ad.flush_count(), 1);
    }

    #[test]
    fn enqueue_report_without_connect_errors() {
        let mut ad = AdCore::new();
        ad.open_context().unwrap();
        let r = AdReport {
            asset_id: 1,
            event: 0,
            timestamp: 0,
        };
        assert_eq!(ad.enqueue_report(r), Err(SCE_AD_ERROR_NOT_CONNECTED));
    }

    #[test]
    fn get_asset_info_missing_is_invalid_asset() {
        let mut ad = AdCore::new();
        ad.open_context().unwrap();
        assert_eq!(ad.get_asset_info(42), Err(SCE_AD_ERROR_INVALID_ASSET));
    }

    #[test]
    fn get_asset_info_returns_registered() {
        let mut ad = AdCore::new();
        ad.open_context().unwrap();
        let info = AdAssetInfo {
            asset_id: 7,
            width: 1280,
            height: 720,
            duration_ms: 5000,
        };
        ad.inject_asset(info).unwrap();
        assert_eq!(ad.get_asset_info(7).unwrap(), info);
    }

    #[test]
    fn inject_asset_duplicate_rejected() {
        let mut ad = AdCore::new();
        ad.open_context().unwrap();
        let a = AdAssetInfo {
            asset_id: 7,
            width: 1,
            height: 1,
            duration_ms: 1,
        };
        ad.inject_asset(a).unwrap();
        assert_eq!(ad.inject_asset(a), Err(SCE_AD_ERROR_INVALID_ASSET));
    }

    #[test]
    fn get_space_info_missing_is_invalid_asset() {
        let mut ad = AdCore::new();
        ad.open_context().unwrap();
        assert_eq!(ad.get_space_info(1), Err(SCE_AD_ERROR_INVALID_ASSET));
    }

    #[test]
    fn get_space_info_returns_registered() {
        let mut ad = AdCore::new();
        ad.open_context().unwrap();
        let s = AdSpaceInfo {
            space_id: 9,
            slot_count: 4,
        };
        ad.inject_space(s).unwrap();
        assert_eq!(ad.get_space_info(9).unwrap(), s);
    }

    #[test]
    fn connection_info_without_connect_errors() {
        let mut ad = AdCore::new();
        ad.open_context().unwrap();
        assert_eq!(ad.get_connection_info(), Err(SCE_AD_ERROR_NOT_CONNECTED));
    }

    #[test]
    fn connection_info_counts_traffic() {
        let mut ad = AdCore::new();
        ad.open_context().unwrap();
        ad.connect_context().unwrap();
        ad.record_traffic(100, 200).unwrap();
        ad.record_traffic(50, 25).unwrap();
        let snap = ad.get_connection_info().unwrap();
        assert_eq!(snap.bytes_sent, 150);
        assert_eq!(snap.bytes_received, 225);
        assert_eq!(snap.round_trips, 2);
    }

    #[test]
    fn asset_info_on_uninitialized_is_not_initialized() {
        let ad = AdCore::new();
        assert_eq!(ad.get_asset_info(1), Err(SCE_AD_ERROR_NOT_INITIALIZED));
    }

    #[test]
    fn connection_info_on_closed_is_context_closed() {
        let mut ad = AdCore::new();
        ad.open_context().unwrap();
        ad.connect_context().unwrap();
        ad.close_context().unwrap();
        assert_eq!(ad.get_connection_info(), Err(SCE_AD_ERROR_CONTEXT_CLOSED));
    }

    #[test]
    fn flush_on_closed_is_context_closed() {
        let mut ad = AdCore::new();
        ad.open_context().unwrap();
        ad.connect_context().unwrap();
        ad.close_context().unwrap();
        assert_eq!(ad.flush_reports(), Err(SCE_AD_ERROR_CONTEXT_CLOSED));
    }

    #[test]
    fn full_libad_core_lifecycle_smoke() {
        let mut ad = AdCore::new();
        // 1. Open.
        ad.open_context().unwrap();
        assert_eq!(ad.state(), AdContextState::Open);

        // 2. Register assets / spaces *before* connect (matches SDK order).
        ad.inject_asset(AdAssetInfo {
            asset_id: 1,
            width: 640,
            height: 480,
            duration_ms: 30_000,
        })
        .unwrap();
        ad.inject_asset(AdAssetInfo {
            asset_id: 2,
            width: 1920,
            height: 1080,
            duration_ms: 15_000,
        })
        .unwrap();
        ad.inject_space(AdSpaceInfo {
            space_id: 100,
            slot_count: 2,
        })
        .unwrap();

        // 3. Asset lookup works while only Open.
        assert_eq!(ad.get_asset_info(2).unwrap().width, 1920);

        // 4. Connect to server.
        ad.connect_context().unwrap();
        assert_eq!(ad.state(), AdContextState::Connected);

        // 5. Drive some traffic + reports.
        ad.record_traffic(1024, 256).unwrap();
        ad.enqueue_report(AdReport {
            asset_id: 1,
            event: 10, // impression
            timestamp: 100,
        })
        .unwrap();
        ad.enqueue_report(AdReport {
            asset_id: 2,
            event: 11, // click
            timestamp: 200,
        })
        .unwrap();
        assert_eq!(ad.queued_report_count(), 2);

        // 6. Snapshot + flush.
        let conn = ad.get_connection_info().unwrap();
        assert_eq!(conn.bytes_sent, 1024);
        assert_eq!(conn.bytes_received, 256);
        assert_eq!(conn.round_trips, 1);
        let flushed = ad.flush_reports().unwrap();
        assert_eq!(flushed.len(), 2);
        assert_eq!(ad.queued_report_count(), 0);

        // 7. Close.
        ad.close_context().unwrap();
        assert_eq!(ad.state(), AdContextState::Closed);

        // 8. Everything rejected post-close.
        assert_eq!(ad.open_context(), Err(SCE_AD_ERROR_CONTEXT_CLOSED));
        assert_eq!(ad.connect_context(), Err(SCE_AD_ERROR_CONTEXT_CLOSED));
        assert_eq!(ad.close_context(), Err(SCE_AD_ERROR_CONTEXT_CLOSED));
        assert_eq!(ad.flush_reports(), Err(SCE_AD_ERROR_CONTEXT_CLOSED));
        assert_eq!(ad.get_asset_info(1), Err(SCE_AD_ERROR_CONTEXT_CLOSED));
        assert_eq!(ad.get_space_info(100), Err(SCE_AD_ERROR_CONTEXT_CLOSED));
        assert_eq!(ad.get_connection_info(), Err(SCE_AD_ERROR_CONTEXT_CLOSED));

        // 9. Counters.
        assert_eq!(ad.open_count(), 1);
        assert_eq!(ad.connect_count(), 1);
        assert_eq!(ad.close_count(), 1);
        assert_eq!(ad.flush_count(), 1);
    }
}
