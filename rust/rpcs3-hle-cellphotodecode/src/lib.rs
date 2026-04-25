//! Rust port of `rpcs3/Emu/Cell/Modules/cellPhotoDecode.cpp` — PS3 photo
//! decoder HLE surface.
//!
//! Upstream registers the `cellPhotoDecodeUtil` PRX with 4 entry points
//! (Initialize, Initialize2, Finalize, FromFile). This crate is a
//! behaviour-faithful, allocation-backed stub: error codes are byte-exact,
//! lifecycle FSM is enforced, the async `funcFinish` callbacks are queued
//! onto a deferred callback list (mirroring `sysutil_register_cb`), and the
//! synchronous `FromFile` path does VFS-style path validation and defers
//! the actual decoding to a pluggable backend trait.
//!
//! The crate is `no_std` + `alloc` and keeps per-entry counters so higher
//! layers (integration tests, tracing) can assert on dispatch order.

#![no_std]

extern crate alloc;

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::mem::{size_of, take};

use rpcs3_emu_types::CellError;

/// PRX module name registered by upstream `DECLARE(ppu_module_manager::cellPhotoDecode)`.
pub const MODULE_NAME: &str = "cellPhotoDecodeUtil";

/// FNIDs registered via `REG_FUNC` — preserved textual order for dispatch assertions.
pub const REGISTERED_ENTRY_POINTS: &[&str] = &[
    "cellPhotoDecodeInitialize",
    "cellPhotoDecodeInitialize2",
    "cellPhotoDecodeFinalize",
    "cellPhotoDecodeFromFile",
];

/// `CELL_PHOTO_DECODE_VERSION_CURRENT` — only value currently accepted by the real firmware.
pub const CELL_PHOTO_DECODE_VERSION_CURRENT: u32 = 0;

/// Sentinel meaning "no dedicated memory container" (caller relies on PRX heap).
pub const SYS_MEMORY_CONTAINER_ID_INVALID: u32 = 0xFFFF_FFFF;

// ---------------------------------------------------------------------------
// Error codes — byte-exact with `CellPhotoDecodeError` (0x8002_C901 .. 0x8002_C906).
// ---------------------------------------------------------------------------

pub const CELL_PHOTO_DECODE_ERROR_BUSY: CellError = CellError(0x8002_C901);
pub const CELL_PHOTO_DECODE_ERROR_INTERNAL: CellError = CellError(0x8002_C902);
pub const CELL_PHOTO_DECODE_ERROR_PARAM: CellError = CellError(0x8002_C903);
pub const CELL_PHOTO_DECODE_ERROR_ACCESS_ERROR: CellError = CellError(0x8002_C904);
pub const CELL_PHOTO_DECODE_ERROR_INITIALIZE: CellError = CellError(0x8002_C905);
pub const CELL_PHOTO_DECODE_ERROR_DECODE: CellError = CellError(0x8002_C906);

// ---------------------------------------------------------------------------
// Wire structs — `#[repr(C)]` layouts mirroring the big-endian PPU memory image.
// ---------------------------------------------------------------------------

/// Mirrors `CellPhotoDecodeSetParam`. Fields are host endian; PPU memory uses BE.
#[repr(C)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CellPhotoDecodeSetParam {
    pub dst_buffer: u32,
    pub width: u16,
    pub height: u16,
    pub reserved1: u32,
    pub reserved2: u32,
}

const _: () = assert!(size_of::<CellPhotoDecodeSetParam>() == 16);

/// Mirrors `CellPhotoDecodeReturnParam`. Upstream `*return_param = {}` zeroes the
/// whole struct before filling `width`/`height` on success.
#[repr(C)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CellPhotoDecodeReturnParam {
    pub width: u16,
    pub height: u16,
    pub reserved1: u32,
    pub reserved2: u32,
}

const _: () = assert!(size_of::<CellPhotoDecodeReturnParam>() == 12);

// ---------------------------------------------------------------------------
// FSM — init/fini lifecycle matching `sysutil_register_cb`-style deferred callbacks.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModuleState {
    Uninit,
    Initialized,
    Finalized,
}

impl Default for ModuleState {
    fn default() -> Self {
        ModuleState::Uninit
    }
}

/// Records which Initialize variant was invoked — `Initialize` takes `container1`
/// *and* `container2`, while `Initialize2` only takes `container2`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitVariant {
    V1 { container1: u32, container2: u32 },
    V2 { container2: u32 },
}

/// Pending callback enqueued via `sysutil_register_cb`. `funcFinish` cannot run on
/// the caller's stack — upstream always defers; we do the same.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PendingCallback {
    pub func_finish: u32,
    pub result: u32,
    pub userdata: u32,
    pub cause: CallbackCause,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallbackCause {
    Initialize,
    Initialize2,
    Finalize,
}

// ---------------------------------------------------------------------------
// Decode backend — `FromFile` delegates the actual scaling/decoding.
// ---------------------------------------------------------------------------

/// Mirror of `Emu.GetCallbacks().get_scaled_image(...)`. Returning `Ok((w,h))` signals
/// success; `Err(())` means the decode failed (upstream maps to `ERROR_DECODE`).
pub trait PhotoDecodeBackend {
    fn decode_scaled(
        &mut self,
        path: &str,
        requested_width: u16,
        requested_height: u16,
        dst_buffer_addr: u32,
    ) -> Result<(u16, u16), ()>;
}

/// Minimal in-memory backend for tests: returns whatever dimensions were injected
/// (or an injected failure) and records every call.
#[derive(Debug, Default)]
pub struct MockBackend {
    pub calls: Vec<MockDecodeCall>,
    pub next_result: Option<Result<(u16, u16), ()>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MockDecodeCall {
    pub path: String,
    pub width: u16,
    pub height: u16,
    pub dst_buffer_addr: u32,
}

impl PhotoDecodeBackend for MockBackend {
    fn decode_scaled(
        &mut self,
        path: &str,
        requested_width: u16,
        requested_height: u16,
        dst_buffer_addr: u32,
    ) -> Result<(u16, u16), ()> {
        self.calls.push(MockDecodeCall {
            path: path.to_string(),
            width: requested_width,
            height: requested_height,
            dst_buffer_addr,
        });
        self.next_result
            .take()
            .unwrap_or(Ok((requested_width, requested_height)))
    }
}

// ---------------------------------------------------------------------------
// VFS stub — simulates `fs::is_file(vfs::get(vpath))`.
// ---------------------------------------------------------------------------

/// Tests inject known paths here; `FromFile` rejects anything not registered.
#[derive(Debug, Default)]
pub struct VfsRegistry {
    files: Vec<String>,
}

impl VfsRegistry {
    pub fn register_file(&mut self, vpath: &str) {
        let v = vpath.to_string();
        if !self.files.iter().any(|p| p == &v) {
            self.files.push(v);
        }
    }

    pub fn unregister_file(&mut self, vpath: &str) {
        self.files.retain(|p| p != vpath);
    }

    pub fn is_file(&self, vpath: &str) -> bool {
        self.files.iter().any(|p| p == vpath)
    }

    pub fn len(&self) -> usize {
        self.files.len()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Manager — holds FSM, callback queue, VFS mock and per-entry counters.
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct PhotoDecode {
    state: ModuleState,
    init_variant: Option<InitVariant>,
    pending: Vec<PendingCallback>,
    vfs: VfsRegistry,

    pub initialize_calls: u64,
    pub initialize2_calls: u64,
    pub finalize_calls: u64,
    pub from_file_calls: u64,
    pub param_error_count: u64,
    pub access_error_count: u64,
    pub decode_error_count: u64,
    pub decode_success_count: u64,
}

impl PhotoDecode {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn state(&self) -> ModuleState {
        self.state
    }

    pub fn init_variant(&self) -> Option<InitVariant> {
        self.init_variant
    }

    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }

    pub fn pending(&self) -> &[PendingCallback] {
        &self.pending
    }

    pub fn vfs_mut(&mut self) -> &mut VfsRegistry {
        &mut self.vfs
    }

    pub fn vfs(&self) -> &VfsRegistry {
        &self.vfs
    }

    /// Drains the deferred callback queue — mirrors the sysutil callback pump.
    pub fn drain_callbacks(&mut self) -> Vec<PendingCallback> {
        take(&mut self.pending)
    }

    /// `cellPhotoDecodeInitialize(version, container1, container2, funcFinish, userdata)`.
    pub fn initialize(
        &mut self,
        version: u32,
        container1: u32,
        container2: u32,
        func_finish: u32,
        userdata: u32,
    ) -> Result<(), CellError> {
        self.initialize_calls = self.initialize_calls.saturating_add(1);
        if version != CELL_PHOTO_DECODE_VERSION_CURRENT || func_finish == 0 {
            self.param_error_count = self.param_error_count.saturating_add(1);
            return Err(CELL_PHOTO_DECODE_ERROR_PARAM);
        }
        // Upstream also validates container sizes, but both branches are currently
        // gated by `&& false`. Preserve the observable behaviour (no rejection).
        self.state = ModuleState::Initialized;
        self.init_variant = Some(InitVariant::V1 {
            container1,
            container2,
        });
        self.pending.push(PendingCallback {
            func_finish,
            result: 0,
            userdata,
            cause: CallbackCause::Initialize,
        });
        Ok(())
    }

    /// `cellPhotoDecodeInitialize2(version, container2, funcFinish, userdata)`.
    pub fn initialize2(
        &mut self,
        version: u32,
        container2: u32,
        func_finish: u32,
        userdata: u32,
    ) -> Result<(), CellError> {
        self.initialize2_calls = self.initialize2_calls.saturating_add(1);
        if version != CELL_PHOTO_DECODE_VERSION_CURRENT || func_finish == 0 {
            self.param_error_count = self.param_error_count.saturating_add(1);
            return Err(CELL_PHOTO_DECODE_ERROR_PARAM);
        }
        self.state = ModuleState::Initialized;
        self.init_variant = Some(InitVariant::V2 { container2 });
        self.pending.push(PendingCallback {
            func_finish,
            result: 0,
            userdata,
            cause: CallbackCause::Initialize2,
        });
        Ok(())
    }

    /// `cellPhotoDecodeFinalize(funcFinish, userdata)`.
    pub fn finalize(&mut self, func_finish: u32, userdata: u32) -> Result<(), CellError> {
        self.finalize_calls = self.finalize_calls.saturating_add(1);
        if func_finish == 0 {
            self.param_error_count = self.param_error_count.saturating_add(1);
            return Err(CELL_PHOTO_DECODE_ERROR_PARAM);
        }
        // Upstream does not gate Finalize on previous Initialize. Preserve that.
        self.state = ModuleState::Finalized;
        self.pending.push(PendingCallback {
            func_finish,
            result: 0,
            userdata,
            cause: CallbackCause::Finalize,
        });
        Ok(())
    }

    /// `cellPhotoDecodeFromFile(srcHddDir, srcHddFile, set_param, return_param)`.
    ///
    /// `src_hdd_dir` / `src_hdd_file` being `None` maps to the upstream null-pointer
    /// check; `set_param` / `return_param` are Rust-side & cannot be null here, so
    /// caller-side bindings would translate null pointers to a `PARAM` before calling.
    pub fn from_file<B: PhotoDecodeBackend>(
        &mut self,
        src_hdd_dir: Option<&str>,
        src_hdd_file: Option<&str>,
        set_param: &CellPhotoDecodeSetParam,
        return_param: &mut CellPhotoDecodeReturnParam,
        backend: &mut B,
    ) -> Result<(), CellError> {
        self.from_file_calls = self.from_file_calls.saturating_add(1);

        let (dir, file) = match (src_hdd_dir, src_hdd_file) {
            (Some(d), Some(f)) => (d, f),
            _ => {
                self.param_error_count = self.param_error_count.saturating_add(1);
                return Err(CELL_PHOTO_DECODE_ERROR_PARAM);
            }
        };

        // Mirror `*return_param = {};` — clear before any success fills it.
        *return_param = CellPhotoDecodeReturnParam::default();

        let vpath = join_vpath(dir, file);
        if !is_allowed_vpath(&vpath) {
            self.access_error_count = self.access_error_count.saturating_add(1);
            return Err(CELL_PHOTO_DECODE_ERROR_ACCESS_ERROR);
        }

        if !self.vfs.is_file(&vpath) {
            self.access_error_count = self.access_error_count.saturating_add(1);
            return Err(CELL_PHOTO_DECODE_ERROR_ACCESS_ERROR);
        }

        match backend.decode_scaled(&vpath, set_param.width, set_param.height, set_param.dst_buffer)
        {
            Ok((w, h)) => {
                return_param.width = w;
                return_param.height = h;
                self.decode_success_count = self.decode_success_count.saturating_add(1);
                Ok(())
            }
            Err(()) => {
                self.decode_error_count = self.decode_error_count.saturating_add(1);
                Err(CELL_PHOTO_DECODE_ERROR_DECODE)
            }
        }
    }
}

/// Joins two VFS path fragments the way upstream does: `"{dir}/{file}"`. We do not
/// normalise `//` sequences because the original format string doesn't either — this
/// keeps the derived `vpath` byte-for-byte comparable.
pub fn join_vpath(dir: &str, file: &str) -> String {
    let mut s = String::with_capacity(dir.len() + 1 + file.len());
    s.push_str(dir);
    s.push('/');
    s.push_str(file);
    s
}

/// Mirrors the prefix whitelist in `cellPhotoDecodeFromFile`. Only these three
/// mount points are accepted; anything else returns `ACCESS_ERROR`.
pub fn is_allowed_vpath(vpath: &str) -> bool {
    vpath.starts_with("/dev_hdd0")
        || vpath.starts_with("/dev_hdd1")
        || vpath.starts_with("/dev_bdvd")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn with_vfs(vpath: &str) -> PhotoDecode {
        let mut m = PhotoDecode::new();
        m.vfs_mut().register_file(vpath);
        m
    }

    #[test]
    fn module_name_and_entries_match_cpp() {
        assert_eq!(MODULE_NAME, "cellPhotoDecodeUtil");
        assert_eq!(REGISTERED_ENTRY_POINTS.len(), 4);
        assert_eq!(REGISTERED_ENTRY_POINTS[0], "cellPhotoDecodeInitialize");
        assert_eq!(REGISTERED_ENTRY_POINTS[1], "cellPhotoDecodeInitialize2");
        assert_eq!(REGISTERED_ENTRY_POINTS[2], "cellPhotoDecodeFinalize");
        assert_eq!(REGISTERED_ENTRY_POINTS[3], "cellPhotoDecodeFromFile");
    }

    #[test]
    fn error_codes_are_byte_exact() {
        assert_eq!(CELL_PHOTO_DECODE_ERROR_BUSY.0, 0x8002_C901);
        assert_eq!(CELL_PHOTO_DECODE_ERROR_INTERNAL.0, 0x8002_C902);
        assert_eq!(CELL_PHOTO_DECODE_ERROR_PARAM.0, 0x8002_C903);
        assert_eq!(CELL_PHOTO_DECODE_ERROR_ACCESS_ERROR.0, 0x8002_C904);
        assert_eq!(CELL_PHOTO_DECODE_ERROR_INITIALIZE.0, 0x8002_C905);
        assert_eq!(CELL_PHOTO_DECODE_ERROR_DECODE.0, 0x8002_C906);
    }

    #[test]
    fn set_param_layout_is_16_bytes() {
        assert_eq!(core::mem::size_of::<CellPhotoDecodeSetParam>(), 16);
        assert_eq!(core::mem::size_of::<CellPhotoDecodeReturnParam>(), 12);
    }

    #[test]
    fn initialize_rejects_wrong_version() {
        let mut m = PhotoDecode::new();
        let err = m.initialize(1, 0xFFFF_FFFF, 0xFFFF_FFFF, 0xDEAD_BEEF, 0).unwrap_err();
        assert_eq!(err, CELL_PHOTO_DECODE_ERROR_PARAM);
        assert_eq!(m.state(), ModuleState::Uninit);
        assert_eq!(m.param_error_count, 1);
        assert_eq!(m.pending_len(), 0);
    }

    #[test]
    fn initialize_rejects_null_func_finish() {
        let mut m = PhotoDecode::new();
        let err = m
            .initialize(CELL_PHOTO_DECODE_VERSION_CURRENT, 0xFFFF_FFFF, 0xFFFF_FFFF, 0, 0)
            .unwrap_err();
        assert_eq!(err, CELL_PHOTO_DECODE_ERROR_PARAM);
        assert_eq!(m.param_error_count, 1);
    }

    #[test]
    fn initialize_queues_callback() {
        let mut m = PhotoDecode::new();
        m.initialize(
            CELL_PHOTO_DECODE_VERSION_CURRENT,
            SYS_MEMORY_CONTAINER_ID_INVALID,
            SYS_MEMORY_CONTAINER_ID_INVALID,
            0x8000_1000,
            0x1234_5678,
        )
        .unwrap();
        assert_eq!(m.state(), ModuleState::Initialized);
        assert_eq!(
            m.init_variant(),
            Some(InitVariant::V1 {
                container1: SYS_MEMORY_CONTAINER_ID_INVALID,
                container2: SYS_MEMORY_CONTAINER_ID_INVALID,
            })
        );
        assert_eq!(m.pending_len(), 1);
        assert_eq!(m.pending()[0].cause, CallbackCause::Initialize);
        assert_eq!(m.pending()[0].userdata, 0x1234_5678);
        assert_eq!(m.pending()[0].result, 0);
    }

    #[test]
    fn initialize2_rejects_wrong_version() {
        let mut m = PhotoDecode::new();
        let err = m.initialize2(42, 0xFFFF_FFFF, 0xDEAD_BEEF, 0).unwrap_err();
        assert_eq!(err, CELL_PHOTO_DECODE_ERROR_PARAM);
    }

    #[test]
    fn initialize2_records_variant_v2() {
        let mut m = PhotoDecode::new();
        m.initialize2(CELL_PHOTO_DECODE_VERSION_CURRENT, 0xAA55, 0x1111, 0x2222)
            .unwrap();
        assert_eq!(m.state(), ModuleState::Initialized);
        assert_eq!(
            m.init_variant(),
            Some(InitVariant::V2 { container2: 0xAA55 })
        );
        assert_eq!(m.pending()[0].cause, CallbackCause::Initialize2);
    }

    #[test]
    fn finalize_rejects_null_callback() {
        let mut m = PhotoDecode::new();
        let err = m.finalize(0, 0).unwrap_err();
        assert_eq!(err, CELL_PHOTO_DECODE_ERROR_PARAM);
        assert_eq!(m.state(), ModuleState::Uninit);
    }

    #[test]
    fn finalize_without_initialize_is_allowed() {
        // Upstream does not gate on prior initialize — it always queues the callback.
        let mut m = PhotoDecode::new();
        m.finalize(0x8000_2000, 0xCAFEBABE).unwrap();
        assert_eq!(m.state(), ModuleState::Finalized);
        assert_eq!(m.pending()[0].cause, CallbackCause::Finalize);
        assert_eq!(m.pending()[0].userdata, 0xCAFEBABE);
    }

    #[test]
    fn drain_callbacks_takes_queue() {
        let mut m = PhotoDecode::new();
        m.initialize(CELL_PHOTO_DECODE_VERSION_CURRENT, 0xFFFF_FFFF, 0xFFFF_FFFF, 0x1000, 0)
            .unwrap();
        m.finalize(0x2000, 0).unwrap();
        let drained = m.drain_callbacks();
        assert_eq!(drained.len(), 2);
        assert_eq!(drained[0].cause, CallbackCause::Initialize);
        assert_eq!(drained[1].cause, CallbackCause::Finalize);
        assert_eq!(m.pending_len(), 0);
    }

    #[test]
    fn from_file_rejects_null_dir_or_file() {
        let mut m = PhotoDecode::new();
        let set = CellPhotoDecodeSetParam::default();
        let mut ret = CellPhotoDecodeReturnParam::default();
        let mut b = MockBackend::default();
        assert_eq!(
            m.from_file(None, Some("p.jpg"), &set, &mut ret, &mut b),
            Err(CELL_PHOTO_DECODE_ERROR_PARAM)
        );
        assert_eq!(
            m.from_file(Some("/dev_hdd0"), None, &set, &mut ret, &mut b),
            Err(CELL_PHOTO_DECODE_ERROR_PARAM)
        );
        assert_eq!(m.from_file_calls, 2);
        assert_eq!(m.param_error_count, 2);
    }

    #[test]
    fn from_file_rejects_non_whitelisted_mount() {
        let mut m = with_vfs("/foo/bar/pic.jpg");
        let set = CellPhotoDecodeSetParam::default();
        let mut ret = CellPhotoDecodeReturnParam {
            width: 0xFFFF,
            height: 0xFFFF,
            reserved1: 0xDEAD_BEEF,
            reserved2: 0xCAFE_BABE,
        };
        let mut b = MockBackend::default();
        let err = m
            .from_file(Some("/foo/bar"), Some("pic.jpg"), &set, &mut ret, &mut b)
            .unwrap_err();
        assert_eq!(err, CELL_PHOTO_DECODE_ERROR_ACCESS_ERROR);
        assert_eq!(m.access_error_count, 1);
        // `return_param` was cleared even though decode failed.
        assert_eq!(ret, CellPhotoDecodeReturnParam::default());
        assert_eq!(b.calls.len(), 0);
    }

    #[test]
    fn from_file_rejects_missing_file() {
        let mut m = PhotoDecode::new(); // vfs empty
        let set = CellPhotoDecodeSetParam::default();
        let mut ret = CellPhotoDecodeReturnParam::default();
        let mut b = MockBackend::default();
        let err = m
            .from_file(Some("/dev_hdd0/photo"), Some("missing.jpg"), &set, &mut ret, &mut b)
            .unwrap_err();
        assert_eq!(err, CELL_PHOTO_DECODE_ERROR_ACCESS_ERROR);
        assert_eq!(b.calls.len(), 0);
    }

    #[test]
    fn from_file_accepts_all_three_prefixes() {
        for (dir, file) in [
            ("/dev_hdd0/photo", "a.jpg"),
            ("/dev_hdd1/images", "b.png"),
            ("/dev_bdvd/PS3_GAME/USRDIR", "c.bmp"),
        ] {
            let vpath = join_vpath(dir, file);
            let mut m = with_vfs(&vpath);
            let set = CellPhotoDecodeSetParam {
                dst_buffer: 0x4000_0000,
                width: 128,
                height: 96,
                reserved1: 0,
                reserved2: 0,
            };
            let mut ret = CellPhotoDecodeReturnParam::default();
            let mut b = MockBackend::default();
            m.from_file(Some(dir), Some(file), &set, &mut ret, &mut b).unwrap();
            assert_eq!(ret.width, 128);
            assert_eq!(ret.height, 96);
            assert_eq!(b.calls.len(), 1);
            assert_eq!(b.calls[0].path, vpath);
            assert_eq!(b.calls[0].dst_buffer_addr, 0x4000_0000);
        }
    }

    #[test]
    fn from_file_maps_backend_error_to_decode() {
        let mut m = with_vfs("/dev_hdd0/pic.jpg");
        let set = CellPhotoDecodeSetParam {
            dst_buffer: 0x4000_0000,
            width: 64,
            height: 64,
            reserved1: 0,
            reserved2: 0,
        };
        let mut ret = CellPhotoDecodeReturnParam::default();
        let mut b = MockBackend {
            next_result: Some(Err(())),
            ..Default::default()
        };
        let err = m
            .from_file(Some("/dev_hdd0"), Some("pic.jpg"), &set, &mut ret, &mut b)
            .unwrap_err();
        assert_eq!(err, CELL_PHOTO_DECODE_ERROR_DECODE);
        assert_eq!(m.decode_error_count, 1);
        assert_eq!(m.decode_success_count, 0);
        assert_eq!(ret, CellPhotoDecodeReturnParam::default());
    }

    #[test]
    fn from_file_uses_backend_reported_dimensions() {
        let mut m = with_vfs("/dev_hdd0/pic.jpg");
        let set = CellPhotoDecodeSetParam {
            dst_buffer: 0x4000_0000,
            width: 1024,
            height: 768,
            reserved1: 0,
            reserved2: 0,
        };
        let mut ret = CellPhotoDecodeReturnParam::default();
        let mut b = MockBackend {
            next_result: Some(Ok((640, 480))),
            ..Default::default()
        };
        m.from_file(Some("/dev_hdd0"), Some("pic.jpg"), &set, &mut ret, &mut b)
            .unwrap();
        // backend down-scaled — `return_param` reports actual dims, not requested.
        assert_eq!(ret.width, 640);
        assert_eq!(ret.height, 480);
        assert_eq!(m.decode_success_count, 1);
    }

    #[test]
    fn join_vpath_inserts_single_slash() {
        assert_eq!(join_vpath("/dev_hdd0/photo", "a.jpg"), "/dev_hdd0/photo/a.jpg");
        // No normalisation — preserves upstream behaviour.
        assert_eq!(join_vpath("/dev_hdd0/", "a.jpg"), "/dev_hdd0//a.jpg");
    }

    #[test]
    fn is_allowed_vpath_checks_three_mounts() {
        assert!(is_allowed_vpath("/dev_hdd0/photo/x.jpg"));
        assert!(is_allowed_vpath("/dev_hdd1/x.jpg"));
        assert!(is_allowed_vpath("/dev_bdvd/x.jpg"));
        assert!(!is_allowed_vpath("/dev_usb000/x.jpg"));
        assert!(!is_allowed_vpath("/app_home/x.jpg"));
        assert!(!is_allowed_vpath(""));
    }

    #[test]
    fn vfs_registry_dedup_and_unregister() {
        let mut v = VfsRegistry::default();
        v.register_file("/dev_hdd0/a.jpg");
        v.register_file("/dev_hdd0/a.jpg");
        v.register_file("/dev_hdd0/b.jpg");
        assert_eq!(v.len(), 2);
        v.unregister_file("/dev_hdd0/a.jpg");
        assert_eq!(v.len(), 1);
        assert!(!v.is_file("/dev_hdd0/a.jpg"));
        assert!(v.is_file("/dev_hdd0/b.jpg"));
    }

    #[test]
    fn from_file_clears_return_param_before_decoding() {
        let mut m = with_vfs("/dev_hdd0/pic.jpg");
        let set = CellPhotoDecodeSetParam {
            dst_buffer: 0x4000_0000,
            width: 256,
            height: 256,
            reserved1: 0,
            reserved2: 0,
        };
        let mut ret = CellPhotoDecodeReturnParam {
            width: 0xAAAA,
            height: 0xBBBB,
            reserved1: 0xDEAD_BEEF,
            reserved2: 0xCAFE_BABE,
        };
        let mut b = MockBackend::default();
        m.from_file(Some("/dev_hdd0"), Some("pic.jpg"), &set, &mut ret, &mut b)
            .unwrap();
        // reserved fields must be zeroed by the `*return_param = {}` step.
        assert_eq!(ret.reserved1, 0);
        assert_eq!(ret.reserved2, 0);
        assert_eq!(ret.width, 256);
        assert_eq!(ret.height, 256);
    }

    #[test]
    fn counters_track_every_entry_independently() {
        let mut m = PhotoDecode::new();
        m.initialize(CELL_PHOTO_DECODE_VERSION_CURRENT, 0xFFFF_FFFF, 0xFFFF_FFFF, 0x1, 0)
            .unwrap();
        m.initialize2(CELL_PHOTO_DECODE_VERSION_CURRENT, 0xFFFF_FFFF, 0x1, 0).unwrap();
        m.finalize(0x1, 0).unwrap();
        let set = CellPhotoDecodeSetParam::default();
        let mut ret = CellPhotoDecodeReturnParam::default();
        let mut b = MockBackend::default();
        let _ = m.from_file(None, None, &set, &mut ret, &mut b);

        assert_eq!(m.initialize_calls, 1);
        assert_eq!(m.initialize2_calls, 1);
        assert_eq!(m.finalize_calls, 1);
        assert_eq!(m.from_file_calls, 1);
        assert_eq!(m.param_error_count, 1); // only the from_file null case
    }

    #[test]
    fn param_error_count_accumulates_across_entries() {
        let mut m = PhotoDecode::new();
        // Bad version x2
        let _ = m.initialize(99, 0xFFFF_FFFF, 0xFFFF_FFFF, 0x1, 0);
        let _ = m.initialize2(99, 0xFFFF_FFFF, 0x1, 0);
        // Null callback x1
        let _ = m.finalize(0, 0);
        assert_eq!(m.param_error_count, 3);
    }

    #[test]
    fn access_error_count_separates_prefix_and_missing_file() {
        let mut m = with_vfs("/dev_hdd0/pic.jpg");
        let set = CellPhotoDecodeSetParam::default();
        let mut ret = CellPhotoDecodeReturnParam::default();
        let mut b = MockBackend::default();
        // bad prefix
        let _ = m.from_file(Some("/foo"), Some("p.jpg"), &set, &mut ret, &mut b);
        // missing file on allowed mount
        let _ = m.from_file(Some("/dev_hdd0"), Some("missing.jpg"), &set, &mut ret, &mut b);
        assert_eq!(m.access_error_count, 2);
        assert_eq!(b.calls.len(), 0);
    }

    #[test]
    fn full_photodecode_lifecycle_smoke() {
        let mut m = PhotoDecode::new();
        m.vfs_mut().register_file("/dev_hdd0/photo/test.jpg");

        // Init (variant 1)
        m.initialize(
            CELL_PHOTO_DECODE_VERSION_CURRENT,
            SYS_MEMORY_CONTAINER_ID_INVALID,
            SYS_MEMORY_CONTAINER_ID_INVALID,
            0x9000_0001,
            0xAA,
        )
        .unwrap();

        // Decode a registered file
        let set = CellPhotoDecodeSetParam {
            dst_buffer: 0x4000_0000,
            width: 320,
            height: 240,
            reserved1: 0,
            reserved2: 0,
        };
        let mut ret = CellPhotoDecodeReturnParam::default();
        let mut backend = MockBackend {
            next_result: Some(Ok((300, 200))),
            ..Default::default()
        };
        m.from_file(
            Some("/dev_hdd0/photo"),
            Some("test.jpg"),
            &set,
            &mut ret,
            &mut backend,
        )
        .unwrap();
        assert_eq!(ret.width, 300);
        assert_eq!(ret.height, 200);

        // Finalize
        m.finalize(0x9000_0002, 0xBB).unwrap();

        // Drain deferred callbacks
        let drained = m.drain_callbacks();
        assert_eq!(drained.len(), 2);
        assert_eq!(drained[0].cause, CallbackCause::Initialize);
        assert_eq!(drained[0].userdata, 0xAA);
        assert_eq!(drained[1].cause, CallbackCause::Finalize);
        assert_eq!(drained[1].userdata, 0xBB);

        // Counters reflect the sequence
        assert_eq!(m.initialize_calls, 1);
        assert_eq!(m.finalize_calls, 1);
        assert_eq!(m.from_file_calls, 1);
        assert_eq!(m.decode_success_count, 1);
        assert_eq!(m.state(), ModuleState::Finalized);
    }
}
