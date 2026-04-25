//! Rust port of `rpcs3/Emu/Cell/Modules/cellVideoUpload.cpp`.
//!
//! Single-entry-point module — `cellVideoUploadInitialize` — that the
//! firmware used to drive the PS3 XMB "Upload to YouTube" option
//! (service decommissioned circa 2012). The C++ stub (54 lines)
//! immediately fires **two** deferred sysutil callbacks back to the
//! game: `INITIALIZED` followed by `FINALIZED`, both with `CELL_OK`
//! status. The Rust port preserves that exact event sequence and adds
//! parameter validation so callers that pass malformed input surface a
//! named error.
//!
//! REG_FUNC at cpp:53. Module name byte-exact at cpp:7 / cpp:51.
//!
//! **Facility note**: error codes occupy `0x8002_D0__` (Sony-committed
//! at `cellVideoUpload.h:49-60`). `rpcs3-hle-celldtcpiputility` uses
//! placeholder codes in the same facility sub-range — the two
//! modules do not share state, so they can coexist in the workspace,
//! but a migration would want to reclaim the `DtcpIp` placeholder range
//! before this module lands in the guest's real ABI lookup table.

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use rpcs3_emu_types::CellError;

/// Byte-exact at cpp:7 / cpp:51.
pub const MODULE_NAME: &str = "cellVideoUpload";

/// REG_FUNC at cpp:53.
pub const REGISTERED_ENTRY_POINTS: &[&str] = &["cellVideoUploadInitialize"];

// --- Error codes (byte-exact cellVideoUpload.h:49-60) -------------------

pub const CELL_VIDEO_UPLOAD_ERROR_CANCEL: CellError = CellError(0x8002_D000);
pub const CELL_VIDEO_UPLOAD_ERROR_NETWORK: CellError = CellError(0x8002_D001);
pub const CELL_VIDEO_UPLOAD_ERROR_SERVICE_STOP: CellError = CellError(0x8002_D002);
pub const CELL_VIDEO_UPLOAD_ERROR_SERVICE_BUSY: CellError = CellError(0x8002_D003);
pub const CELL_VIDEO_UPLOAD_ERROR_SERVICE_UNAVAILABLE: CellError = CellError(0x8002_D004);
pub const CELL_VIDEO_UPLOAD_ERROR_SERVICE_QUOTA: CellError = CellError(0x8002_D005);
pub const CELL_VIDEO_UPLOAD_ERROR_ACCOUNT_STOP: CellError = CellError(0x8002_D006);
pub const CELL_VIDEO_UPLOAD_ERROR_OUT_OF_MEMORY: CellError = CellError(0x8002_D020);
pub const CELL_VIDEO_UPLOAD_ERROR_FATAL: CellError = CellError(0x8002_D021);
pub const CELL_VIDEO_UPLOAD_ERROR_INVALID_VALUE: CellError = CellError(0x8002_D022);
pub const CELL_VIDEO_UPLOAD_ERROR_FILE_OPEN: CellError = CellError(0x8002_D023);
pub const CELL_VIDEO_UPLOAD_ERROR_INVALID_STATE: CellError = CellError(0x8002_D024);

// --- Status codes (byte-exact cellVideoUpload.h:63-67) ------------------

pub const CELL_VIDEO_UPLOAD_STATUS_INITIALIZED: i32 = 1;
pub const CELL_VIDEO_UPLOAD_STATUS_FINALIZED: i32 = 2;

// --- Length caps (cellVideoUpload.h:36-44) ------------------------------

pub const CELL_VIDEO_UPLOAD_MAX_FILE_PATH_LEN: usize = 1023;
pub const CELL_VIDEO_UPLOAD_MAX_YOUTUBE_CLIENT_ID_LEN: usize = 64;
pub const CELL_VIDEO_UPLOAD_MAX_YOUTUBE_DEVELOPER_KEY_LEN: usize = 128;
pub const CELL_VIDEO_UPLOAD_MAX_YOUTUBE_TITLE_LEN: usize = 61;
pub const CELL_VIDEO_UPLOAD_MAX_YOUTUBE_DESCRIPTION_LEN: usize = 1024;
pub const CELL_VIDEO_UPLOAD_MAX_YOUTUBE_KEYWORD_LEN: usize = 25;

/// Size of the `pResultURL` buffer the firmware hands to the callback
/// (cpp:40 `vm::var<char[]>(128)`). 128 bytes.
pub const CELL_VIDEO_UPLOAD_RESULT_URL_LEN: usize = 128;

// --- Data mirrors -------------------------------------------------------

/// Subset of `CellVideoUploadParam::u::youtube` (cellVideoUpload.h:17-28).
#[derive(Debug, Default, Clone)]
pub struct YoutubeUploadFields {
    pub client_id: String,
    pub developer_key: String,
    pub title_utf8: String,
    pub description_utf8: String,
    pub keyword_1_utf8: String,
    pub keyword_2_utf8: String,
    pub keyword_3_utf8: String,
    pub is_private: u8,
    pub rating: u8,
}

/// Mirror of `CellVideoUploadOption` (cellVideoUpload.h:5-9).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CellVideoUploadOption {
    pub r#type: i32,
    pub value: u64,
}

/// Mirror of `CellVideoUploadParam` (cellVideoUpload.h:11-32). The
/// union over site-specific fields is flattened — `u.youtube` is the
/// only declared variant.
#[derive(Debug, Default, Clone)]
pub struct CellVideoUploadParam {
    pub site_id: i32,
    pub file_path: String,
    pub youtube: YoutubeUploadFields,
    pub options: Vec<CellVideoUploadOption>,
}

/// One deferred callback the stub fires via `sysutil_register_cb`
/// (cpp:38-46). Tests drain these via [`VideoUpload::drain_callbacks`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadCallback {
    pub status: i32,
    pub error_code: CellError,
    /// The `pResultURL` buffer the firmware publishes — empty in the
    /// stub (matches the zero-initialised `vm::var<char[]>` at cpp:40).
    pub result_url: String,
    pub userdata: u64,
}

// --- Validation ---------------------------------------------------------

fn require_len(s: &str, cap: usize) -> Result<(), CellError> {
    if s.len() > cap {
        Err(CELL_VIDEO_UPLOAD_ERROR_INVALID_VALUE)
    } else {
        Ok(())
    }
}

/// Validate a `CellVideoUploadParam`: non-empty file path within cap,
/// all YouTube fields within their caps, rating ≤ 5.
pub fn validate_param(param: &CellVideoUploadParam) -> Result<(), CellError> {
    if param.file_path.is_empty() {
        return Err(CELL_VIDEO_UPLOAD_ERROR_FILE_OPEN);
    }
    require_len(&param.file_path, CELL_VIDEO_UPLOAD_MAX_FILE_PATH_LEN)?;
    require_len(
        &param.youtube.client_id,
        CELL_VIDEO_UPLOAD_MAX_YOUTUBE_CLIENT_ID_LEN,
    )?;
    require_len(
        &param.youtube.developer_key,
        CELL_VIDEO_UPLOAD_MAX_YOUTUBE_DEVELOPER_KEY_LEN,
    )?;
    require_len(
        &param.youtube.title_utf8,
        CELL_VIDEO_UPLOAD_MAX_YOUTUBE_TITLE_LEN,
    )?;
    require_len(
        &param.youtube.description_utf8,
        CELL_VIDEO_UPLOAD_MAX_YOUTUBE_DESCRIPTION_LEN,
    )?;
    for keyword in [
        &param.youtube.keyword_1_utf8,
        &param.youtube.keyword_2_utf8,
        &param.youtube.keyword_3_utf8,
    ] {
        require_len(keyword, CELL_VIDEO_UPLOAD_MAX_YOUTUBE_KEYWORD_LEN)?;
    }
    if param.youtube.rating > 5 {
        return Err(CELL_VIDEO_UPLOAD_ERROR_INVALID_VALUE);
    }
    if param.youtube.is_private > 1 {
        return Err(CELL_VIDEO_UPLOAD_ERROR_INVALID_VALUE);
    }
    Ok(())
}

// --- Manager ------------------------------------------------------------

#[derive(Debug, Default)]
pub struct VideoUpload {
    initialized: bool,
    callback_addr: u64,
    userdata: u64,
    pending_callbacks: Vec<UploadCallback>,
    init_calls: u32,
}

impl VideoUpload {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            initialized: false,
            callback_addr: 0,
            userdata: 0,
            pending_callbacks: Vec::new(),
            init_calls: 0,
        }
    }

    #[must_use]
    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    #[must_use]
    pub fn init_calls(&self) -> u32 {
        self.init_calls
    }

    #[must_use]
    pub fn pending_callback_count(&self) -> usize {
        self.pending_callbacks.len()
    }

    pub fn drain_callbacks(&mut self) -> Vec<UploadCallback> {
        core::mem::take(&mut self.pending_callbacks)
    }

    /// `cellVideoUploadInitialize` (cpp:34-49). Validates the param,
    /// queues the `INITIALIZED → FINALIZED` callback pair that the
    /// firmware stub publishes via `sysutil_register_cb`.
    ///
    /// `callback_addr == 0` is rejected — a null callback has no useful
    /// semantics and the stub would immediately SEGV when it tried to
    /// invoke `cb(...)` at cpp:42.
    pub fn initialize(
        &mut self,
        param: Option<&CellVideoUploadParam>,
        callback_addr: u64,
        userdata: u64,
    ) -> Result<(), CellError> {
        if self.initialized {
            return Err(CELL_VIDEO_UPLOAD_ERROR_INVALID_STATE);
        }
        let Some(p) = param else {
            return Err(CELL_VIDEO_UPLOAD_ERROR_INVALID_VALUE);
        };
        if callback_addr == 0 {
            return Err(CELL_VIDEO_UPLOAD_ERROR_INVALID_VALUE);
        }
        validate_param(p)?;
        self.initialized = true;
        self.callback_addr = callback_addr;
        self.userdata = userdata;
        let empty_url = String::new();
        // Firmware fires INITIALIZED then FINALIZED back-to-back
        // (cpp:42-43) — port mirrors that exactly.
        self.pending_callbacks.push(UploadCallback {
            status: CELL_VIDEO_UPLOAD_STATUS_INITIALIZED,
            error_code: CellError::OK,
            result_url: empty_url.clone(),
            userdata,
        });
        self.pending_callbacks.push(UploadCallback {
            status: CELL_VIDEO_UPLOAD_STATUS_FINALIZED,
            error_code: CellError::OK,
            result_url: empty_url,
            userdata,
        });
        self.init_calls = self.init_calls.saturating_add(1);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;

    fn sample_param() -> CellVideoUploadParam {
        CellVideoUploadParam {
            site_id: 1,
            file_path: "/dev_hdd0/video/capture.mp4".to_string(),
            youtube: YoutubeUploadFields {
                client_id: "client-id".to_string(),
                developer_key: "dev-key".to_string(),
                title_utf8: "my video".to_string(),
                description_utf8: "description".to_string(),
                keyword_1_utf8: "kw1".to_string(),
                keyword_2_utf8: "kw2".to_string(),
                keyword_3_utf8: "kw3".to_string(),
                is_private: 0,
                rating: 3,
            },
            options: Vec::new(),
        }
    }

    #[test]
    fn module_name_byte_exact() {
        assert_eq!(MODULE_NAME, "cellVideoUpload");
    }

    #[test]
    fn registered_entry_points_exact_order() {
        assert_eq!(REGISTERED_ENTRY_POINTS, &["cellVideoUploadInitialize"]);
    }

    #[test]
    fn error_codes_byte_exact() {
        assert_eq!(CELL_VIDEO_UPLOAD_ERROR_CANCEL.0, 0x8002_D000);
        assert_eq!(CELL_VIDEO_UPLOAD_ERROR_NETWORK.0, 0x8002_D001);
        assert_eq!(CELL_VIDEO_UPLOAD_ERROR_SERVICE_STOP.0, 0x8002_D002);
        assert_eq!(CELL_VIDEO_UPLOAD_ERROR_SERVICE_BUSY.0, 0x8002_D003);
        assert_eq!(CELL_VIDEO_UPLOAD_ERROR_SERVICE_UNAVAILABLE.0, 0x8002_D004);
        assert_eq!(CELL_VIDEO_UPLOAD_ERROR_SERVICE_QUOTA.0, 0x8002_D005);
        assert_eq!(CELL_VIDEO_UPLOAD_ERROR_ACCOUNT_STOP.0, 0x8002_D006);
        assert_eq!(CELL_VIDEO_UPLOAD_ERROR_OUT_OF_MEMORY.0, 0x8002_D020);
        assert_eq!(CELL_VIDEO_UPLOAD_ERROR_FATAL.0, 0x8002_D021);
        assert_eq!(CELL_VIDEO_UPLOAD_ERROR_INVALID_VALUE.0, 0x8002_D022);
        assert_eq!(CELL_VIDEO_UPLOAD_ERROR_FILE_OPEN.0, 0x8002_D023);
        assert_eq!(CELL_VIDEO_UPLOAD_ERROR_INVALID_STATE.0, 0x8002_D024);
    }

    #[test]
    fn status_constants_byte_exact() {
        assert_eq!(CELL_VIDEO_UPLOAD_STATUS_INITIALIZED, 1);
        assert_eq!(CELL_VIDEO_UPLOAD_STATUS_FINALIZED, 2);
    }

    #[test]
    fn length_caps_byte_exact() {
        assert_eq!(CELL_VIDEO_UPLOAD_MAX_FILE_PATH_LEN, 1023);
        assert_eq!(CELL_VIDEO_UPLOAD_MAX_YOUTUBE_CLIENT_ID_LEN, 64);
        assert_eq!(CELL_VIDEO_UPLOAD_MAX_YOUTUBE_DEVELOPER_KEY_LEN, 128);
        assert_eq!(CELL_VIDEO_UPLOAD_MAX_YOUTUBE_TITLE_LEN, 61);
        assert_eq!(CELL_VIDEO_UPLOAD_MAX_YOUTUBE_DESCRIPTION_LEN, 1024);
        assert_eq!(CELL_VIDEO_UPLOAD_MAX_YOUTUBE_KEYWORD_LEN, 25);
        assert_eq!(CELL_VIDEO_UPLOAD_RESULT_URL_LEN, 128);
    }

    #[test]
    fn error_code_gap_structure_preserved() {
        // The C++ header has a deliberate gap at 0x07..0x1F, mirroring
        // the Sony convention of reserving "service" errors in the low
        // range and "library" errors higher up.
        assert!(
            CELL_VIDEO_UPLOAD_ERROR_OUT_OF_MEMORY.0
                > CELL_VIDEO_UPLOAD_ERROR_ACCOUNT_STOP.0 + 1
        );
        assert_eq!(
            CELL_VIDEO_UPLOAD_ERROR_OUT_OF_MEMORY.0
                - CELL_VIDEO_UPLOAD_ERROR_ACCOUNT_STOP.0,
            0x1A
        );
    }

    #[test]
    fn initialize_happy_path_queues_both_callbacks() {
        let mut vu = VideoUpload::new();
        let p = sample_param();
        vu.initialize(Some(&p), 0xDEAD_BEEF, 0xCAFE_FACE).unwrap();
        assert!(vu.is_initialized());
        let cbs = vu.drain_callbacks();
        assert_eq!(cbs.len(), 2);
        assert_eq!(cbs[0].status, CELL_VIDEO_UPLOAD_STATUS_INITIALIZED);
        assert_eq!(cbs[0].error_code, CellError::OK);
        assert_eq!(cbs[0].userdata, 0xCAFE_FACE);
        assert!(cbs[0].result_url.is_empty());
        assert_eq!(cbs[1].status, CELL_VIDEO_UPLOAD_STATUS_FINALIZED);
        assert_eq!(cbs[1].error_code, CellError::OK);
    }

    #[test]
    fn initialize_null_param_is_invalid_value() {
        let mut vu = VideoUpload::new();
        assert_eq!(
            vu.initialize(None, 1, 0),
            Err(CELL_VIDEO_UPLOAD_ERROR_INVALID_VALUE)
        );
    }

    #[test]
    fn initialize_null_callback_is_invalid_value() {
        let mut vu = VideoUpload::new();
        let p = sample_param();
        assert_eq!(
            vu.initialize(Some(&p), 0, 0),
            Err(CELL_VIDEO_UPLOAD_ERROR_INVALID_VALUE)
        );
    }

    #[test]
    fn initialize_twice_is_invalid_state() {
        let mut vu = VideoUpload::new();
        let p = sample_param();
        vu.initialize(Some(&p), 1, 0).unwrap();
        assert_eq!(
            vu.initialize(Some(&p), 1, 0),
            Err(CELL_VIDEO_UPLOAD_ERROR_INVALID_STATE)
        );
    }

    #[test]
    fn initialize_empty_file_path_is_file_open() {
        let mut vu = VideoUpload::new();
        let mut p = sample_param();
        p.file_path = String::new();
        assert_eq!(
            vu.initialize(Some(&p), 1, 0),
            Err(CELL_VIDEO_UPLOAD_ERROR_FILE_OPEN)
        );
    }

    #[test]
    fn initialize_oversize_file_path_is_invalid_value() {
        let mut vu = VideoUpload::new();
        let mut p = sample_param();
        p.file_path = "x".repeat(CELL_VIDEO_UPLOAD_MAX_FILE_PATH_LEN + 1);
        assert_eq!(
            vu.initialize(Some(&p), 1, 0),
            Err(CELL_VIDEO_UPLOAD_ERROR_INVALID_VALUE)
        );
    }

    #[test]
    fn initialize_oversize_title_is_invalid_value() {
        let mut vu = VideoUpload::new();
        let mut p = sample_param();
        p.youtube.title_utf8 = "t".repeat(CELL_VIDEO_UPLOAD_MAX_YOUTUBE_TITLE_LEN + 1);
        assert_eq!(
            vu.initialize(Some(&p), 1, 0),
            Err(CELL_VIDEO_UPLOAD_ERROR_INVALID_VALUE)
        );
    }

    #[test]
    fn initialize_oversize_description_is_invalid_value() {
        let mut vu = VideoUpload::new();
        let mut p = sample_param();
        p.youtube.description_utf8 =
            "d".repeat(CELL_VIDEO_UPLOAD_MAX_YOUTUBE_DESCRIPTION_LEN + 1);
        assert_eq!(
            vu.initialize(Some(&p), 1, 0),
            Err(CELL_VIDEO_UPLOAD_ERROR_INVALID_VALUE)
        );
    }

    #[test]
    fn initialize_oversize_keyword_2_is_invalid_value() {
        let mut vu = VideoUpload::new();
        let mut p = sample_param();
        p.youtube.keyword_2_utf8 =
            "k".repeat(CELL_VIDEO_UPLOAD_MAX_YOUTUBE_KEYWORD_LEN + 1);
        assert_eq!(
            vu.initialize(Some(&p), 1, 0),
            Err(CELL_VIDEO_UPLOAD_ERROR_INVALID_VALUE)
        );
    }

    #[test]
    fn initialize_rating_over_5_is_invalid_value() {
        let mut vu = VideoUpload::new();
        let mut p = sample_param();
        p.youtube.rating = 6;
        assert_eq!(
            vu.initialize(Some(&p), 1, 0),
            Err(CELL_VIDEO_UPLOAD_ERROR_INVALID_VALUE)
        );
    }

    #[test]
    fn initialize_is_private_non_bool_is_invalid_value() {
        let mut vu = VideoUpload::new();
        let mut p = sample_param();
        p.youtube.is_private = 2;
        assert_eq!(
            vu.initialize(Some(&p), 1, 0),
            Err(CELL_VIDEO_UPLOAD_ERROR_INVALID_VALUE)
        );
    }

    #[test]
    fn initialize_boundary_lengths_accepted() {
        let mut vu = VideoUpload::new();
        let p = CellVideoUploadParam {
            site_id: 0,
            file_path: "/f.mp4".to_string(),
            youtube: YoutubeUploadFields {
                client_id: "a".repeat(CELL_VIDEO_UPLOAD_MAX_YOUTUBE_CLIENT_ID_LEN),
                developer_key: "b".repeat(CELL_VIDEO_UPLOAD_MAX_YOUTUBE_DEVELOPER_KEY_LEN),
                title_utf8: "c".repeat(CELL_VIDEO_UPLOAD_MAX_YOUTUBE_TITLE_LEN),
                description_utf8: "d"
                    .repeat(CELL_VIDEO_UPLOAD_MAX_YOUTUBE_DESCRIPTION_LEN),
                keyword_1_utf8: "e".repeat(CELL_VIDEO_UPLOAD_MAX_YOUTUBE_KEYWORD_LEN),
                keyword_2_utf8: String::new(),
                keyword_3_utf8: String::new(),
                is_private: 1,
                rating: 5,
            },
            options: Vec::new(),
        };
        vu.initialize(Some(&p), 0x8000_0000, 0).unwrap();
    }

    #[test]
    fn drain_callbacks_is_fifo() {
        let mut vu = VideoUpload::new();
        let p = sample_param();
        vu.initialize(Some(&p), 1, 0x1234).unwrap();
        let cbs = vu.drain_callbacks();
        assert_eq!(cbs[0].status, CELL_VIDEO_UPLOAD_STATUS_INITIALIZED);
        assert_eq!(cbs[1].status, CELL_VIDEO_UPLOAD_STATUS_FINALIZED);
    }

    #[test]
    fn drain_callbacks_empties_queue() {
        let mut vu = VideoUpload::new();
        let p = sample_param();
        vu.initialize(Some(&p), 1, 0).unwrap();
        assert_eq!(vu.pending_callback_count(), 2);
        let _ = vu.drain_callbacks();
        assert_eq!(vu.pending_callback_count(), 0);
    }

    #[test]
    fn init_calls_counter_increments() {
        let mut vu = VideoUpload::new();
        let p = sample_param();
        vu.initialize(Some(&p), 1, 0).unwrap();
        assert_eq!(vu.init_calls(), 1);
    }

    #[test]
    fn validate_param_standalone_happy_path() {
        let p = sample_param();
        assert!(validate_param(&p).is_ok());
    }

    #[test]
    fn full_videoupload_lifecycle_smoke() {
        let mut vu = VideoUpload::new();
        let p = sample_param();

        // 1. First initialize succeeds + queues both deferred callbacks.
        vu.initialize(Some(&p), 0x8000_0000, 0xABCD_EF01).unwrap();
        assert_eq!(vu.pending_callback_count(), 2);

        // 2. Re-initialize blocked.
        assert_eq!(
            vu.initialize(Some(&p), 0x8000_0000, 0),
            Err(CELL_VIDEO_UPLOAD_ERROR_INVALID_STATE)
        );

        // 3. Drain — both statuses arrive with CELL_OK.
        let cbs = vu.drain_callbacks();
        assert_eq!(cbs.len(), 2);
        assert_eq!(cbs[0].status, CELL_VIDEO_UPLOAD_STATUS_INITIALIZED);
        assert_eq!(cbs[1].status, CELL_VIDEO_UPLOAD_STATUS_FINALIZED);
        assert!(cbs.iter().all(|c| c.error_code == CellError::OK));
        assert!(cbs.iter().all(|c| c.userdata == 0xABCD_EF01));
    }
}
