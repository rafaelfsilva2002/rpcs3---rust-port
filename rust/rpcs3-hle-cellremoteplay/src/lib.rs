//! Rust port of `rpcs3/Emu/Cell/Modules/cellRemotePlay.cpp`.
//!
//! 8 PRX entries under the module name `cellRemotePlay`, the system
//! library PS3 games call when they want to coordinate with a paired
//! PSP running Remote Play. Every body in the C++ is a
//! `todo(...)` stub that returns `CELL_OK`; the one exception is
//! `cellRemotePlayGetComparativeVolume` (cpp:57-67) which writes a
//! `1.0f` default into the caller's buffer before returning.
//!
//! REG_FUNC order at cpp:78-85:
//!
//!  1. `cellRemotePlayGetStatus`
//!  2. `cellRemotePlaySetComparativeVolume`
//!  3. `cellRemotePlayGetPeerInfo`
//!  4. `cellRemotePlayGetSharedMemory`
//!  5. `cellRemotePlayEncryptAllData`
//!  6. `cellRemotePlayStopPeerVideoOut`
//!  7. `cellRemotePlayGetComparativeVolume`
//!  8. `cellRemotePlayBreak`
//!
//! Module name is byte-exact at cpp:5 / cpp:76.

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

use rpcs3_emu_types::CellError;

/// Byte-exact at cpp:5 / cpp:76.
pub const MODULE_NAME: &str = "cellRemotePlay";

/// REG_FUNC order at cpp:78-85.
pub const REGISTERED_ENTRY_POINTS: &[&str] = &[
    "cellRemotePlayGetStatus",
    "cellRemotePlaySetComparativeVolume",
    "cellRemotePlayGetPeerInfo",
    "cellRemotePlayGetSharedMemory",
    "cellRemotePlayEncryptAllData",
    "cellRemotePlayStopPeerVideoOut",
    "cellRemotePlayGetComparativeVolume",
    "cellRemotePlayBreak",
];

// --- Error codes (cellRemotePlay.h) -------------------------------------

/// Byte-exact at `cellRemotePlay.h` — the only error the firmware
/// commits for this module.
pub const CELL_REMOTEPLAY_ERROR_INTERNAL: CellError = CellError(0x8002_9830);

// --- Status codes (cellRemotePlay.h) ------------------------------------
//
// These are the same values the firmware returns via `GetStatus`; see
// cpp:23 stub. The Rust port mirrors the header name-for-name so games
// inspecting the status code against the constant still compile.

pub const CELL_REMOTEPLAY_STATUS_LOADING: u32 = 0;
pub const CELL_REMOTEPLAY_STATUS_WAIT: u32 = 1;
pub const CELL_REMOTEPLAY_STATUS_RUNNING: u32 = 2;
pub const CELL_REMOTEPLAY_STATUS_UNLOADING: u32 = 3;
pub const CELL_REMOTEPLAY_STATUS_FATALERROR: u32 = 4;
pub const CELL_REMOTEPLAY_STATUS_PREMOEND: u32 = 5;

/// `1.0f` — the default comparative volume the firmware hands back when
/// no game-side override has been applied (cpp:63).
pub const DEFAULT_COMPARATIVE_VOLUME: f32 = 1.0;

// --- Status mirror ------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemotePlayStatus {
    Loading,
    Wait,
    Running,
    Unloading,
    FatalError,
    PremoEnd,
}

impl RemotePlayStatus {
    #[must_use]
    pub const fn as_u32(self) -> u32 {
        match self {
            Self::Loading => CELL_REMOTEPLAY_STATUS_LOADING,
            Self::Wait => CELL_REMOTEPLAY_STATUS_WAIT,
            Self::Running => CELL_REMOTEPLAY_STATUS_RUNNING,
            Self::Unloading => CELL_REMOTEPLAY_STATUS_UNLOADING,
            Self::FatalError => CELL_REMOTEPLAY_STATUS_FATALERROR,
            Self::PremoEnd => CELL_REMOTEPLAY_STATUS_PREMOEND,
        }
    }

    #[must_use]
    pub const fn from_u32(v: u32) -> Option<Self> {
        match v {
            CELL_REMOTEPLAY_STATUS_LOADING => Some(Self::Loading),
            CELL_REMOTEPLAY_STATUS_WAIT => Some(Self::Wait),
            CELL_REMOTEPLAY_STATUS_RUNNING => Some(Self::Running),
            CELL_REMOTEPLAY_STATUS_UNLOADING => Some(Self::Unloading),
            CELL_REMOTEPLAY_STATUS_FATALERROR => Some(Self::FatalError),
            CELL_REMOTEPLAY_STATUS_PREMOEND => Some(Self::PremoEnd),
            _ => None,
        }
    }
}

impl Default for RemotePlayStatus {
    fn default() -> Self {
        Self::Loading
    }
}

// --- Peer info mirror ---------------------------------------------------

/// Subset of `CellRemotePlayPeerInfo` the port surfaces.
/// `cellRemotePlayGetPeerInfo` in the firmware populates a much larger
/// struct; the Rust port keeps only the fields that tests can actually
/// assert against without modelling a vm::ptr buffer.
#[derive(Debug, Default, Clone)]
pub struct PeerInfo {
    pub nickname: alloc::string::String,
    pub psp_ver: u32,
    pub link_state: u32,
}

// --- Manager ------------------------------------------------------------

/// HLE state for the singleton `cellRemotePlay` session.
#[derive(Debug, Default)]
pub struct RemotePlay {
    status: RemotePlayStatus,
    comparative_volume: Option<f32>,
    peer_info: Option<PeerInfo>,
    shared_memory_addr: u32,
    shared_memory_size: u64,
    encrypt_enabled: bool,
    break_requested: bool,
    get_status_calls: u32,
    set_volume_calls: u32,
    get_peer_info_calls: u32,
    get_shared_memory_calls: u32,
    encrypt_all_data_calls: u32,
    stop_peer_video_out_calls: u32,
    get_comparative_volume_calls: u32,
    break_calls: u32,
}

impl RemotePlay {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            status: RemotePlayStatus::Loading,
            comparative_volume: None,
            peer_info: None,
            shared_memory_addr: 0,
            shared_memory_size: 0,
            encrypt_enabled: false,
            break_requested: false,
            get_status_calls: 0,
            set_volume_calls: 0,
            get_peer_info_calls: 0,
            get_shared_memory_calls: 0,
            encrypt_all_data_calls: 0,
            stop_peer_video_out_calls: 0,
            get_comparative_volume_calls: 0,
            break_calls: 0,
        }
    }

    #[must_use]
    pub fn status(&self) -> RemotePlayStatus {
        self.status
    }

    #[must_use]
    pub fn encrypt_enabled(&self) -> bool {
        self.encrypt_enabled
    }

    #[must_use]
    pub fn break_requested(&self) -> bool {
        self.break_requested
    }

    /// Test-only helper: stage a peer-info blob so `get_peer_info` has
    /// something meaningful to return.
    pub fn inject_peer_info(&mut self, info: PeerInfo) {
        self.peer_info = Some(info);
    }

    /// Test-only helper: stage a shared-memory window so
    /// `get_shared_memory` can report it back.
    pub fn inject_shared_memory(&mut self, addr: u32, size: u64) {
        self.shared_memory_addr = addr;
        self.shared_memory_size = size;
    }

    /// Test-only helper: advance the lifecycle status — the real
    /// firmware drives this from the kernel as the PSP handshake
    /// progresses.
    pub fn set_status(&mut self, status: RemotePlayStatus) {
        self.status = status;
    }

    // --- per-entry counters ---

    #[must_use]
    pub fn get_status_calls(&self) -> u32 {
        self.get_status_calls
    }
    #[must_use]
    pub fn set_volume_calls(&self) -> u32 {
        self.set_volume_calls
    }
    #[must_use]
    pub fn get_peer_info_calls(&self) -> u32 {
        self.get_peer_info_calls
    }
    #[must_use]
    pub fn get_shared_memory_calls(&self) -> u32 {
        self.get_shared_memory_calls
    }
    #[must_use]
    pub fn encrypt_all_data_calls(&self) -> u32 {
        self.encrypt_all_data_calls
    }
    #[must_use]
    pub fn stop_peer_video_out_calls(&self) -> u32 {
        self.stop_peer_video_out_calls
    }
    #[must_use]
    pub fn get_comparative_volume_calls(&self) -> u32 {
        self.get_comparative_volume_calls
    }
    #[must_use]
    pub fn break_calls(&self) -> u32 {
        self.break_calls
    }

    // --- entry points ---

    /// `cellRemotePlayGetStatus` (cpp:21-25). The firmware stub returns
    /// `CELL_OK` without publishing the actual status through the
    /// argument list; the Rust port exposes the stored status as a
    /// regular return value so tests can drive it.
    pub fn get_status(&mut self) -> Result<RemotePlayStatus, CellError> {
        self.get_status_calls = self.get_status_calls.saturating_add(1);
        if self.status == RemotePlayStatus::FatalError {
            return Err(CELL_REMOTEPLAY_ERROR_INTERNAL);
        }
        Ok(self.status)
    }

    /// `cellRemotePlaySetComparativeVolume` (cpp:27-31). The firmware
    /// stub silently accepts any volume; the port rejects `NaN` with
    /// `INTERNAL` since the real driver would choke on it.
    pub fn set_comparative_volume(&mut self, volume: f32) -> Result<(), CellError> {
        self.set_volume_calls = self.set_volume_calls.saturating_add(1);
        if volume.is_nan() {
            return Err(CELL_REMOTEPLAY_ERROR_INTERNAL);
        }
        self.comparative_volume = Some(volume);
        Ok(())
    }

    /// `cellRemotePlayGetPeerInfo` (cpp:33-37).
    pub fn get_peer_info(&mut self) -> Result<Option<PeerInfo>, CellError> {
        self.get_peer_info_calls = self.get_peer_info_calls.saturating_add(1);
        Ok(self.peer_info.clone())
    }

    /// `cellRemotePlayGetSharedMemory` (cpp:39-43). Returns
    /// `(addr, size)` for the shared window the peer-side driver
    /// mapped — `(0, 0)` when nothing has been injected yet.
    pub fn get_shared_memory(&mut self) -> Result<(u32, u64), CellError> {
        self.get_shared_memory_calls = self.get_shared_memory_calls.saturating_add(1);
        Ok((self.shared_memory_addr, self.shared_memory_size))
    }

    /// `cellRemotePlayEncryptAllData` (cpp:45-49). Idempotent toggle —
    /// the firmware stub doesn't distinguish enable vs disable and
    /// neither does the port.
    pub fn encrypt_all_data(&mut self) -> Result<(), CellError> {
        self.encrypt_enabled = true;
        self.encrypt_all_data_calls = self.encrypt_all_data_calls.saturating_add(1);
        Ok(())
    }

    /// `cellRemotePlayStopPeerVideoOut` (cpp:51-55).
    pub fn stop_peer_video_out(&mut self) -> Result<(), CellError> {
        self.stop_peer_video_out_calls = self.stop_peer_video_out_calls.saturating_add(1);
        Ok(())
    }

    /// `cellRemotePlayGetComparativeVolume` (cpp:57-67). Writes the
    /// currently-stored volume (or the `1.0f` default matching cpp:63)
    /// into the caller-supplied `Option`. Passing `None` mimics the
    /// `if (pComparativeAudioVolume)` null check at cpp:61 and skips
    /// the write, as the firmware does.
    pub fn get_comparative_volume(
        &mut self,
        out: Option<&mut f32>,
    ) -> Result<(), CellError> {
        self.get_comparative_volume_calls =
            self.get_comparative_volume_calls.saturating_add(1);
        let v = self.comparative_volume.unwrap_or(DEFAULT_COMPARATIVE_VOLUME);
        if let Some(slot) = out {
            *slot = v;
        }
        Ok(())
    }

    /// `cellRemotePlayBreak` (cpp:69-73). Flips a flag the session
    /// handler watches for. Idempotent.
    pub fn request_break(&mut self) -> Result<(), CellError> {
        self.break_requested = true;
        self.break_calls = self.break_calls.saturating_add(1);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;

    #[test]
    fn module_name_byte_exact() {
        assert_eq!(MODULE_NAME, "cellRemotePlay");
    }

    #[test]
    fn registered_entry_points_exact_order() {
        assert_eq!(REGISTERED_ENTRY_POINTS.len(), 8);
        assert_eq!(REGISTERED_ENTRY_POINTS[0], "cellRemotePlayGetStatus");
        assert_eq!(
            REGISTERED_ENTRY_POINTS[1],
            "cellRemotePlaySetComparativeVolume"
        );
        assert_eq!(
            REGISTERED_ENTRY_POINTS[6],
            "cellRemotePlayGetComparativeVolume"
        );
        assert_eq!(REGISTERED_ENTRY_POINTS[7], "cellRemotePlayBreak");
    }

    #[test]
    fn error_code_byte_exact() {
        assert_eq!(CELL_REMOTEPLAY_ERROR_INTERNAL.0, 0x8002_9830);
    }

    #[test]
    fn status_constants_byte_exact() {
        assert_eq!(CELL_REMOTEPLAY_STATUS_LOADING, 0);
        assert_eq!(CELL_REMOTEPLAY_STATUS_WAIT, 1);
        assert_eq!(CELL_REMOTEPLAY_STATUS_RUNNING, 2);
        assert_eq!(CELL_REMOTEPLAY_STATUS_UNLOADING, 3);
        assert_eq!(CELL_REMOTEPLAY_STATUS_FATALERROR, 4);
        assert_eq!(CELL_REMOTEPLAY_STATUS_PREMOEND, 5);
    }

    #[test]
    fn default_volume_byte_exact() {
        assert_eq!(DEFAULT_COMPARATIVE_VOLUME, 1.0);
    }

    #[test]
    fn status_roundtrip() {
        for v in 0..=5 {
            let s = RemotePlayStatus::from_u32(v).unwrap();
            assert_eq!(s.as_u32(), v);
        }
        assert_eq!(RemotePlayStatus::from_u32(6), None);
    }

    #[test]
    fn starts_in_loading() {
        let rp = RemotePlay::new();
        assert_eq!(rp.status(), RemotePlayStatus::Loading);
    }

    #[test]
    fn get_status_returns_stored() {
        let mut rp = RemotePlay::new();
        rp.set_status(RemotePlayStatus::Running);
        assert_eq!(rp.get_status().unwrap(), RemotePlayStatus::Running);
    }

    #[test]
    fn get_status_fatal_returns_internal() {
        let mut rp = RemotePlay::new();
        rp.set_status(RemotePlayStatus::FatalError);
        assert_eq!(rp.get_status(), Err(CELL_REMOTEPLAY_ERROR_INTERNAL));
    }

    #[test]
    fn set_comparative_volume_stores() {
        let mut rp = RemotePlay::new();
        rp.set_comparative_volume(0.5).unwrap();
        let mut out = 0.0f32;
        rp.get_comparative_volume(Some(&mut out)).unwrap();
        assert_eq!(out, 0.5);
    }

    #[test]
    fn set_comparative_volume_nan_is_internal() {
        let mut rp = RemotePlay::new();
        assert_eq!(
            rp.set_comparative_volume(f32::NAN),
            Err(CELL_REMOTEPLAY_ERROR_INTERNAL)
        );
    }

    #[test]
    fn get_comparative_volume_default_is_one() {
        let mut rp = RemotePlay::new();
        let mut out = 0.0f32;
        rp.get_comparative_volume(Some(&mut out)).unwrap();
        assert_eq!(out, 1.0);
    }

    #[test]
    fn get_comparative_volume_null_ptr_noop() {
        let mut rp = RemotePlay::new();
        // Null pointer — firmware branch cpp:61 skips the write.
        rp.get_comparative_volume(None).unwrap();
        assert_eq!(rp.get_comparative_volume_calls(), 1);
    }

    #[test]
    fn get_peer_info_defaults_to_none() {
        let mut rp = RemotePlay::new();
        assert!(rp.get_peer_info().unwrap().is_none());
    }

    #[test]
    fn get_peer_info_returns_injected_blob() {
        let mut rp = RemotePlay::new();
        rp.inject_peer_info(PeerInfo {
            nickname: "player1".to_string(),
            psp_ver: 0x0680,
            link_state: 3,
        });
        let info = rp.get_peer_info().unwrap().unwrap();
        assert_eq!(info.nickname, "player1");
        assert_eq!(info.psp_ver, 0x0680);
        assert_eq!(info.link_state, 3);
    }

    #[test]
    fn get_shared_memory_defaults_to_zero() {
        let mut rp = RemotePlay::new();
        assert_eq!(rp.get_shared_memory().unwrap(), (0, 0));
    }

    #[test]
    fn get_shared_memory_returns_injected() {
        let mut rp = RemotePlay::new();
        rp.inject_shared_memory(0x4000_0000, 64 * 1024);
        assert_eq!(rp.get_shared_memory().unwrap(), (0x4000_0000, 64 * 1024));
    }

    #[test]
    fn encrypt_all_data_flips_flag() {
        let mut rp = RemotePlay::new();
        assert!(!rp.encrypt_enabled());
        rp.encrypt_all_data().unwrap();
        assert!(rp.encrypt_enabled());
        // Idempotent — second call stays enabled.
        rp.encrypt_all_data().unwrap();
        assert!(rp.encrypt_enabled());
        assert_eq!(rp.encrypt_all_data_calls(), 2);
    }

    #[test]
    fn stop_peer_video_out_counter() {
        let mut rp = RemotePlay::new();
        rp.stop_peer_video_out().unwrap();
        rp.stop_peer_video_out().unwrap();
        assert_eq!(rp.stop_peer_video_out_calls(), 2);
    }

    #[test]
    fn request_break_flips_flag() {
        let mut rp = RemotePlay::new();
        assert!(!rp.break_requested());
        rp.request_break().unwrap();
        assert!(rp.break_requested());
        assert_eq!(rp.break_calls(), 1);
    }

    #[test]
    fn volume_default_persists_until_set() {
        let mut rp = RemotePlay::new();
        let mut v1 = 0.0f32;
        rp.get_comparative_volume(Some(&mut v1)).unwrap();
        rp.set_comparative_volume(0.25).unwrap();
        let mut v2 = 0.0f32;
        rp.get_comparative_volume(Some(&mut v2)).unwrap();
        assert_eq!(v1, 1.0);
        assert_eq!(v2, 0.25);
    }

    #[test]
    fn full_remoteplay_lifecycle_smoke() {
        let mut rp = RemotePlay::new();

        // 1. Session comes up — status progresses from Loading → Wait →
        //    Running as the PSP handshake completes.
        assert_eq!(rp.get_status().unwrap(), RemotePlayStatus::Loading);
        rp.set_status(RemotePlayStatus::Wait);
        assert_eq!(rp.get_status().unwrap(), RemotePlayStatus::Wait);
        rp.set_status(RemotePlayStatus::Running);

        // 2. Game stages peer info + SHM + turns on encryption.
        rp.inject_peer_info(PeerInfo {
            nickname: "vita1".to_string(),
            psp_ver: 0x0680,
            link_state: 3,
        });
        rp.inject_shared_memory(0x5000_0000, 1 << 20);
        rp.encrypt_all_data().unwrap();
        assert!(rp.encrypt_enabled());

        // 3. Game reads back everything it staged.
        let peer = rp.get_peer_info().unwrap().unwrap();
        assert_eq!(peer.nickname, "vita1");
        let (addr, size) = rp.get_shared_memory().unwrap();
        assert_eq!(addr, 0x5000_0000);
        assert_eq!(size, 1 << 20);

        // 4. Volume stays at default until set.
        let mut v = 0.0f32;
        rp.get_comparative_volume(Some(&mut v)).unwrap();
        assert_eq!(v, 1.0);
        rp.set_comparative_volume(0.75).unwrap();
        rp.get_comparative_volume(Some(&mut v)).unwrap();
        assert_eq!(v, 0.75);

        // 5. Teardown — stop peer video, request break, unload.
        rp.stop_peer_video_out().unwrap();
        rp.request_break().unwrap();
        rp.set_status(RemotePlayStatus::Unloading);
        assert_eq!(rp.get_status().unwrap(), RemotePlayStatus::Unloading);

        // 6. Counter trace reflects the dispatch order.
        assert!(rp.get_status_calls() >= 3);
        assert_eq!(rp.set_volume_calls(), 1);
        assert_eq!(rp.get_peer_info_calls(), 1);
        assert_eq!(rp.get_shared_memory_calls(), 1);
        assert_eq!(rp.encrypt_all_data_calls(), 1);
        assert_eq!(rp.stop_peer_video_out_calls(), 1);
        assert!(rp.get_comparative_volume_calls() >= 2);
        assert_eq!(rp.break_calls(), 1);
    }
}
