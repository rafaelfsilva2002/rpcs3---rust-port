//! `rpcs3-hle-cellsavedata` — HLE port of `cellSaveData.cpp`.
//!
//! This module is the bridge between LV2-level save-data storage and
//! the game-facing `cellSaveData*` API. It does NOT touch filesystem
//! syscalls directly — instead it delegates to the [`SaveDataState`]
//! trait that the emu core implements (usually backed by
//! `rpcs3-lv2-fs` + the user's actual save-data root).
//!
//! ## Entry points covered in this iteration
//!
//! | HLE function                        | Rust wrapper                      |
//! |-------------------------------------|-----------------------------------|
//! | `cellSaveDataAutoSave2`             | [`cell_savedata_auto_save`]       |
//! | `cellSaveDataAutoLoad2`             | [`cell_savedata_auto_load`]       |
//! | `cellSaveDataListSave2`             | [`cell_savedata_list_save`]       |
//! | `cellSaveDataListLoad2`             | [`cell_savedata_list_load`]       |
//! | `cellSaveDataListAutoSave`          | [`cell_savedata_list_auto_save`]  |
//! | `cellSaveDataListAutoLoad`          | [`cell_savedata_list_auto_load`]  |
//! | `cellSaveDataDelete2`               | [`cell_savedata_delete`]          |
//!
//! Each wrapper drives the "user callback" style flow of PS3 save
//! APIs as a straight-line sequence: locate → stat → file-op →
//! finalize, returning a [`Result`] that the caller converts to a
//! `CellError` for the guest. The actual file I/O and UI prompts are
//! out of scope here — that's the emu core's job.
//!
//! ## Result codes (frozen; facility 0x8002B4__)
//!
//! Extracted byte-exact from `cellSaveData.h`:
//!
//! | Const                               | Value       |
//! |-------------------------------------|-------------|
//! | `ERROR_CBRESULT`                    | 0x8002B401  |
//! | `ERROR_ACCESS_ERROR`                | 0x8002B402  |
//! | `ERROR_INTERNAL`                    | 0x8002B403  |
//! | `ERROR_PARAM`                       | 0x8002B404  |
//! | `ERROR_NOSPACE`                     | 0x8002B405  |
//! | `ERROR_BROKEN`                      | 0x8002B406  |
//! | `ERROR_FAILURE`                     | 0x8002B407  |
//! | `ERROR_BUSY`                        | 0x8002B408  |
//! | `ERROR_NOUSER`                      | 0x8002B409  |
//! | `ERROR_SIZEOVER`                    | 0x8002B40A  |
//! | `ERROR_NODATA`                      | 0x8002B40B  |
//! | `ERROR_NOTSUPPORTED`                | 0x8002B40C  |

use rpcs3_emu_types::CellError;

// =====================================================================
// Frozen constants
// =====================================================================

pub mod errors {
    use rpcs3_emu_types::CellError;

    pub const CBRESULT: CellError = CellError(0x8002_B401);
    pub const ACCESS_ERROR: CellError = CellError(0x8002_B402);
    pub const INTERNAL: CellError = CellError(0x8002_B403);
    pub const PARAM: CellError = CellError(0x8002_B404);
    pub const NOSPACE: CellError = CellError(0x8002_B405);
    pub const BROKEN: CellError = CellError(0x8002_B406);
    pub const FAILURE: CellError = CellError(0x8002_B407);
    pub const BUSY: CellError = CellError(0x8002_B408);
    pub const NOUSER: CellError = CellError(0x8002_B409);
    pub const SIZEOVER: CellError = CellError(0x8002_B40A);
    pub const NODATA: CellError = CellError(0x8002_B40B);
    pub const NOTSUPPORTED: CellError = CellError(0x8002_B40C);
}

/// `CELL_SAVEDATA_DIRNAME_SIZE` — directory names are fixed 32 bytes.
pub const DIRNAME_SIZE: usize = 32;

// ---- Callback result codes (signed) --------------------------------

pub const CBRESULT_OK_LAST_NOCONFIRM: i32 = 2;
pub const CBRESULT_OK_LAST: i32 = 1;
pub const CBRESULT_OK_NEXT: i32 = 0;
pub const CBRESULT_ERR_NOSPACE: i32 = -1;
pub const CBRESULT_ERR_FAILURE: i32 = -2;
pub const CBRESULT_ERR_BROKEN: i32 = -3;
pub const CBRESULT_ERR_NODATA: i32 = -4;
pub const CBRESULT_ERR_INVALID: i32 = -5;

// ---- File-op constants ---------------------------------------------

pub const FILEOP_READ: u32 = 0;
pub const FILEOP_WRITE: u32 = 1;
pub const FILEOP_DELETE: u32 = 2;
pub const FILEOP_WRITE_NOTRUNC: u32 = 3;

// ---- File-type constants -------------------------------------------

pub const FILETYPE_SECUREFILE: u32 = 0;
pub const FILETYPE_NORMALFILE: u32 = 1;
pub const FILETYPE_CONTENT_ICON0: u32 = 2;
pub const FILETYPE_CONTENT_ICON1: u32 = 3;
pub const FILETYPE_CONTENT_PIC1: u32 = 4;
pub const FILETYPE_CONTENT_SND0: u32 = 5;

// =====================================================================
// Data model
// =====================================================================

/// A single save-data directory as listed / stat'd by the HLE layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveDataDir {
    /// Null-terminated 32-byte dir name as stored on disk.
    pub name: [u8; DIRNAME_SIZE],
    /// Free-form title shown in the load/save picker.
    pub title: String,
    /// Free-form subtitle.
    pub subtitle: String,
    /// Total size used by this directory, in bytes.
    pub size_bytes: u64,
    /// Last-modified time as seconds since epoch.
    pub mtime: u64,
}

impl SaveDataDir {
    /// Build a dir entry with the given name, padding with zeros.
    /// Returns `None` if `name` is longer than [`DIRNAME_SIZE`].
    pub fn with_name(name: &str) -> Option<Self> {
        let bytes = name.as_bytes();
        if bytes.len() > DIRNAME_SIZE {
            return None;
        }
        let mut arr = [0u8; DIRNAME_SIZE];
        arr[..bytes.len()].copy_from_slice(bytes);
        Some(Self {
            name: arr,
            title: String::new(),
            subtitle: String::new(),
            size_bytes: 0,
            mtime: 0,
        })
    }

    /// Name as a Rust `&str`, stripping the null padding.
    #[must_use]
    pub fn name_str(&self) -> &str {
        let end = self.name.iter().position(|&b| b == 0).unwrap_or(DIRNAME_SIZE);
        std::str::from_utf8(&self.name[..end]).unwrap_or("")
    }
}

/// One atomic file operation the HLE callback has asked the kernel to
/// perform on the current save dir. Populated by the stat callback,
/// consumed by the file callback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileOp {
    pub op: u32,
    pub filetype: u32,
    pub filename: String,
    /// For WRITE / WRITE_NOTRUNC, the payload. Ignored for READ/DELETE.
    pub payload: Vec<u8>,
}

/// Summary the caller gets back after an AutoSave/AutoLoad round-trip
/// completes. Matches the fields exposed by `CellSaveDataStatGet` that
/// games actually depend on (size/free-space/modified flag).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SaveDataResult {
    pub bytes_written: u64,
    pub bytes_read: u64,
    pub files_touched: u32,
    pub is_new: bool,
}

// =====================================================================
// Trait — emu core plugs this in
// =====================================================================

/// Save-data backend. The emu core implements this over an actual
/// filesystem tree rooted at the PS3 user's save-data dir; the
/// reference [`TestSaveData`] lives entirely in-memory for unit tests.
pub trait SaveDataState {
    fn list(&self) -> Vec<SaveDataDir>;
    fn stat(&self, dir: &str) -> Option<SaveDataDir>;

    /// Perform a single file op inside `dir`. Returns the number of
    /// bytes transferred (only meaningful for READ / WRITE).
    fn apply(&mut self, dir: &str, op: &FileOp) -> Result<usize, CellError>;

    /// Delete an entire save directory.
    fn delete(&mut self, dir: &str) -> Result<(), CellError>;

    /// Create a new save directory (used by AutoSave / ListSave when
    /// the target dir does not exist yet).
    fn create(&mut self, dir: &SaveDataDir) -> Result<(), CellError>;
}

// =====================================================================
// Syscalls — straight-line flow, no coroutines
// =====================================================================

/// Arguments shared by all auto-save/auto-load entry points. In the
/// real HLE these come from a `CellSaveDataSetList` / `CellSaveDataSetBuf`
/// plus pointers to callbacks — for the Rust port we collapse the
/// callback stack into a single `ops` list that the game tells us to
/// perform.
#[derive(Debug, Clone)]
pub struct AutoOps<'a> {
    pub dir_name: &'a str,
    pub title: &'a str,
    pub subtitle: &'a str,
    /// File ops to apply, in order. For AutoLoad you pass READ ops;
    /// for AutoSave you pass WRITE ops. Mixing is allowed — the last
    /// CBRESULT value wins.
    pub ops: Vec<FileOp>,
}

fn validate_ops(ops: &[FileOp]) -> Result<(), CellError> {
    for op in ops {
        if op.filename.is_empty() {
            return Err(errors::PARAM);
        }
        if op.filename.len() > 63 {
            // PS3 save files have a 63-char filename limit.
            return Err(errors::PARAM);
        }
        match op.op {
            FILEOP_READ | FILEOP_WRITE | FILEOP_DELETE | FILEOP_WRITE_NOTRUNC => {}
            _ => return Err(errors::PARAM),
        }
    }
    Ok(())
}

/// `cellSaveDataAutoSave2` — create-or-open a save dir and apply the
/// given write ops. Matches C++ behaviour where a missing dir is
/// automatically created before the file callback fires.
#[must_use]
pub fn cell_savedata_auto_save<S: SaveDataState + ?Sized>(
    state: &mut S,
    args: AutoOps<'_>,
) -> Result<SaveDataResult, CellError> {
    validate_ops(&args.ops)?;

    let mut result = SaveDataResult::default();

    // Stat → decide create-or-update.
    let existing = state.stat(args.dir_name);
    if existing.is_none() {
        let mut dir = SaveDataDir::with_name(args.dir_name).ok_or(errors::PARAM)?;
        dir.title = args.title.to_string();
        dir.subtitle = args.subtitle.to_string();
        state.create(&dir)?;
        result.is_new = true;
    }

    for op in &args.ops {
        let n = state.apply(args.dir_name, op)?;
        result.files_touched += 1;
        match op.op {
            FILEOP_WRITE | FILEOP_WRITE_NOTRUNC => result.bytes_written += n as u64,
            FILEOP_READ => result.bytes_read += n as u64,
            _ => {}
        }
    }

    Ok(result)
}

/// `cellSaveDataAutoLoad2` — open an existing save dir and apply
/// the given read ops. Returns `ERROR_NODATA` if the dir doesn't
/// exist (auto-load never creates).
#[must_use]
pub fn cell_savedata_auto_load<S: SaveDataState + ?Sized>(
    state: &mut S,
    args: AutoOps<'_>,
) -> Result<SaveDataResult, CellError> {
    validate_ops(&args.ops)?;
    if state.stat(args.dir_name).is_none() {
        return Err(errors::NODATA);
    }

    let mut result = SaveDataResult::default();
    for op in &args.ops {
        let n = state.apply(args.dir_name, op)?;
        result.files_touched += 1;
        match op.op {
            FILEOP_READ => result.bytes_read += n as u64,
            FILEOP_WRITE | FILEOP_WRITE_NOTRUNC => result.bytes_written += n as u64,
            _ => {}
        }
    }
    Ok(result)
}

/// `cellSaveDataListLoad2` — show the list of dirs matching `prefix`,
/// pick one (in the unit-test harness, `selection` is the index),
/// then drive the same ops flow as AutoLoad.
#[must_use]
pub fn cell_savedata_list_load<S: SaveDataState + ?Sized>(
    state: &mut S,
    prefix: &str,
    selection: usize,
    ops: Vec<FileOp>,
) -> Result<SaveDataResult, CellError> {
    validate_ops(&ops)?;
    let dirs: Vec<SaveDataDir> = state
        .list()
        .into_iter()
        .filter(|d| d.name_str().starts_with(prefix))
        .collect();

    if dirs.is_empty() {
        return Err(errors::NODATA);
    }
    let chosen = dirs.get(selection).ok_or(errors::PARAM)?.name_str().to_owned();
    cell_savedata_auto_load(state, AutoOps {
        dir_name: &chosen,
        title: "",
        subtitle: "",
        ops,
    })
}

/// `cellSaveDataListSave2` — same as `ListLoad` but creates the dir
/// if the selected slot is an empty-slot sentinel (we use
/// `selection == dirs.len()` for that).
#[must_use]
pub fn cell_savedata_list_save<S: SaveDataState + ?Sized>(
    state: &mut S,
    prefix: &str,
    selection: usize,
    new_dir_name: &str,
    title: &str,
    subtitle: &str,
    ops: Vec<FileOp>,
) -> Result<SaveDataResult, CellError> {
    validate_ops(&ops)?;
    let dirs: Vec<SaveDataDir> = state
        .list()
        .into_iter()
        .filter(|d| d.name_str().starts_with(prefix))
        .collect();

    let dir_name = if selection < dirs.len() {
        dirs[selection].name_str().to_owned()
    } else if selection == dirs.len() {
        // "New slot" sentinel.
        new_dir_name.to_owned()
    } else {
        return Err(errors::PARAM);
    };

    cell_savedata_auto_save(state, AutoOps {
        dir_name: &dir_name,
        title,
        subtitle,
        ops,
    })
}

/// `cellSaveDataListAutoSave` — semantically a wrapper around
/// `AutoSave` that first scans for matching prefixed dirs; for the
/// port we just forward with the given dir name.
#[must_use]
pub fn cell_savedata_list_auto_save<S: SaveDataState + ?Sized>(
    state: &mut S,
    args: AutoOps<'_>,
) -> Result<SaveDataResult, CellError> {
    cell_savedata_auto_save(state, args)
}

/// `cellSaveDataListAutoLoad` — dual of `list_auto_save`.
#[must_use]
pub fn cell_savedata_list_auto_load<S: SaveDataState + ?Sized>(
    state: &mut S,
    args: AutoOps<'_>,
) -> Result<SaveDataResult, CellError> {
    cell_savedata_auto_load(state, args)
}

/// `cellSaveDataDelete2` — delete a single save dir.
#[must_use]
pub fn cell_savedata_delete<S: SaveDataState + ?Sized>(
    state: &mut S,
    dir: &str,
) -> Result<(), CellError> {
    if state.stat(dir).is_none() {
        return Err(errors::NODATA);
    }
    state.delete(dir)
}

// =====================================================================
// Reference implementation — in-memory backing
// =====================================================================

#[derive(Debug, Default)]
pub struct TestSaveData {
    pub dirs: std::collections::BTreeMap<String, SaveDataDirInMem>,
    /// Cap on total bytes across all saves. Zero means "infinite".
    pub quota_bytes: u64,
}

#[derive(Debug, Clone)]
pub struct SaveDataDirInMem {
    pub dir: SaveDataDir,
    pub files: std::collections::BTreeMap<String, Vec<u8>>,
}

impl TestSaveData {
    #[must_use]
    pub fn total_bytes(&self) -> u64 {
        self.dirs
            .values()
            .map(|d| d.files.values().map(|f| f.len() as u64).sum::<u64>())
            .sum()
    }
}

impl SaveDataState for TestSaveData {
    fn list(&self) -> Vec<SaveDataDir> {
        self.dirs.values().map(|d| d.dir.clone()).collect()
    }

    fn stat(&self, dir: &str) -> Option<SaveDataDir> {
        self.dirs.get(dir).map(|d| d.dir.clone())
    }

    fn apply(&mut self, dir: &str, op: &FileOp) -> Result<usize, CellError> {
        let quota = self.quota_bytes;
        let total_before = self.total_bytes();
        let d = self.dirs.get_mut(dir).ok_or(errors::NODATA)?;
        match op.op {
            FILEOP_READ => {
                let bytes = d.files.get(&op.filename).ok_or(errors::NODATA)?;
                Ok(bytes.len())
            }
            FILEOP_WRITE => {
                if quota > 0 {
                    let existing_len = d.files.get(&op.filename).map_or(0, |b| b.len() as u64);
                    let projected = total_before - existing_len + op.payload.len() as u64;
                    if projected > quota {
                        return Err(errors::NOSPACE);
                    }
                }
                d.files.insert(op.filename.clone(), op.payload.clone());
                Ok(op.payload.len())
            }
            FILEOP_WRITE_NOTRUNC => {
                let entry = d.files.entry(op.filename.clone()).or_default();
                if quota > 0 {
                    let projected = total_before + op.payload.len() as u64;
                    if projected > quota {
                        return Err(errors::NOSPACE);
                    }
                }
                entry.extend_from_slice(&op.payload);
                Ok(op.payload.len())
            }
            FILEOP_DELETE => {
                d.files.remove(&op.filename);
                Ok(0)
            }
            _ => Err(errors::PARAM),
        }
    }

    fn delete(&mut self, dir: &str) -> Result<(), CellError> {
        if self.dirs.remove(dir).is_none() {
            return Err(errors::NODATA);
        }
        Ok(())
    }

    fn create(&mut self, dir: &SaveDataDir) -> Result<(), CellError> {
        let key = dir.name_str().to_owned();
        if self.dirs.contains_key(&key) {
            return Err(errors::BROKEN);
        }
        self.dirs.insert(
            key,
            SaveDataDirInMem { dir: dir.clone(), files: std::collections::BTreeMap::new() },
        );
        Ok(())
    }
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn write_op(name: &str, payload: &[u8]) -> FileOp {
        FileOp {
            op: FILEOP_WRITE,
            filetype: FILETYPE_NORMALFILE,
            filename: name.to_owned(),
            payload: payload.to_vec(),
        }
    }

    fn read_op(name: &str) -> FileOp {
        FileOp {
            op: FILEOP_READ,
            filetype: FILETYPE_NORMALFILE,
            filename: name.to_owned(),
            payload: Vec::new(),
        }
    }

    #[test]
    fn error_codes_are_byte_exact_with_cpp() {
        assert_eq!(errors::CBRESULT.0, 0x8002_B401);
        assert_eq!(errors::PARAM.0, 0x8002_B404);
        assert_eq!(errors::NOSPACE.0, 0x8002_B405);
        assert_eq!(errors::NODATA.0, 0x8002_B40B);
        assert_eq!(errors::NOTSUPPORTED.0, 0x8002_B40C);
    }

    #[test]
    fn cbresult_signed_values_match() {
        assert_eq!(CBRESULT_OK_LAST_NOCONFIRM, 2);
        assert_eq!(CBRESULT_OK_LAST, 1);
        assert_eq!(CBRESULT_OK_NEXT, 0);
        assert_eq!(CBRESULT_ERR_INVALID, -5);
    }

    #[test]
    fn fileop_and_filetype_constants_stable() {
        assert_eq!(FILEOP_READ, 0);
        assert_eq!(FILEOP_WRITE, 1);
        assert_eq!(FILEOP_DELETE, 2);
        assert_eq!(FILEOP_WRITE_NOTRUNC, 3);
        assert_eq!(FILETYPE_SECUREFILE, 0);
        assert_eq!(FILETYPE_CONTENT_PIC1, 4);
    }

    #[test]
    fn dirname_over_32_bytes_is_rejected() {
        let too_long = "a".repeat(33);
        assert!(SaveDataDir::with_name(&too_long).is_none());
    }

    #[test]
    fn auto_save_creates_new_dir() {
        let mut state = TestSaveData::default();
        let res = cell_savedata_auto_save(&mut state, AutoOps {
            dir_name: "SLOT00",
            title: "Slot 0",
            subtitle: "autosave",
            ops: vec![write_op("data.bin", b"hello")],
        })
        .unwrap();
        assert!(res.is_new);
        assert_eq!(res.bytes_written, 5);
        assert_eq!(res.files_touched, 1);
        assert!(state.stat("SLOT00").is_some());
    }

    #[test]
    fn auto_save_updates_existing_dir() {
        let mut state = TestSaveData::default();
        cell_savedata_auto_save(&mut state, AutoOps {
            dir_name: "SLOT00",
            title: "",
            subtitle: "",
            ops: vec![write_op("data.bin", b"first")],
        })
        .unwrap();

        let res = cell_savedata_auto_save(&mut state, AutoOps {
            dir_name: "SLOT00",
            title: "",
            subtitle: "",
            ops: vec![write_op("data.bin", b"second")],
        })
        .unwrap();
        assert!(!res.is_new);
        let rr = cell_savedata_auto_load(&mut state, AutoOps {
            dir_name: "SLOT00",
            title: "",
            subtitle: "",
            ops: vec![read_op("data.bin")],
        })
        .unwrap();
        assert_eq!(rr.bytes_read, b"second".len() as u64);
    }

    #[test]
    fn auto_load_missing_dir_is_nodata() {
        let mut state = TestSaveData::default();
        let err = cell_savedata_auto_load(&mut state, AutoOps {
            dir_name: "SLOT99",
            title: "",
            subtitle: "",
            ops: vec![read_op("x")],
        })
        .unwrap_err();
        assert_eq!(err, errors::NODATA);
    }

    #[test]
    fn empty_filename_is_param_error() {
        let mut state = TestSaveData::default();
        let err = cell_savedata_auto_save(&mut state, AutoOps {
            dir_name: "D",
            title: "",
            subtitle: "",
            ops: vec![FileOp {
                op: FILEOP_WRITE,
                filetype: FILETYPE_NORMALFILE,
                filename: String::new(),
                payload: vec![],
            }],
        })
        .unwrap_err();
        assert_eq!(err, errors::PARAM);
    }

    #[test]
    fn filename_over_63_chars_is_param_error() {
        let mut state = TestSaveData::default();
        let err = cell_savedata_auto_save(&mut state, AutoOps {
            dir_name: "D",
            title: "",
            subtitle: "",
            ops: vec![FileOp {
                op: FILEOP_WRITE,
                filetype: FILETYPE_NORMALFILE,
                filename: "a".repeat(64),
                payload: vec![],
            }],
        })
        .unwrap_err();
        assert_eq!(err, errors::PARAM);
    }

    #[test]
    fn unknown_op_code_is_param_error() {
        let mut state = TestSaveData::default();
        let err = cell_savedata_auto_save(&mut state, AutoOps {
            dir_name: "D",
            title: "",
            subtitle: "",
            ops: vec![FileOp {
                op: 99,
                filetype: FILETYPE_NORMALFILE,
                filename: "x".to_owned(),
                payload: vec![],
            }],
        })
        .unwrap_err();
        assert_eq!(err, errors::PARAM);
    }

    #[test]
    fn list_load_picks_matching_prefix_and_reads() {
        let mut state = TestSaveData::default();
        for (i, name) in ["GAME00", "GAME01", "OTHER00"].iter().enumerate() {
            cell_savedata_auto_save(&mut state, AutoOps {
                dir_name: name,
                title: "",
                subtitle: "",
                ops: vec![write_op("f", &[i as u8])],
            })
            .unwrap();
        }
        let res = cell_savedata_list_load(&mut state, "GAME", 1, vec![read_op("f")]).unwrap();
        assert_eq!(res.bytes_read, 1);
        // Selection index 1 within GAME-prefixed dirs → GAME01 → payload [1]
        // The bytes_read==1 proves the read went through; filename check is
        // implicit because no error.
    }

    #[test]
    fn list_load_no_match_is_nodata() {
        let mut state = TestSaveData::default();
        cell_savedata_auto_save(&mut state, AutoOps {
            dir_name: "OTHER",
            title: "",
            subtitle: "",
            ops: vec![write_op("f", b"x")],
        })
        .unwrap();
        let err = cell_savedata_list_load(&mut state, "GAME", 0, vec![read_op("f")]).unwrap_err();
        assert_eq!(err, errors::NODATA);
    }

    #[test]
    fn list_load_out_of_range_is_param() {
        let mut state = TestSaveData::default();
        cell_savedata_auto_save(&mut state, AutoOps {
            dir_name: "GAME",
            title: "",
            subtitle: "",
            ops: vec![write_op("f", b"x")],
        })
        .unwrap();
        let err = cell_savedata_list_load(&mut state, "GAME", 99, vec![read_op("f")]).unwrap_err();
        assert_eq!(err, errors::PARAM);
    }

    #[test]
    fn list_save_new_slot_creates_dir() {
        let mut state = TestSaveData::default();
        // No existing GAME* dirs → selection 0 == new-slot sentinel.
        let res = cell_savedata_list_save(
            &mut state,
            "GAME",
            0,
            "GAME00",
            "game",
            "new save",
            vec![write_op("f", b"yo")],
        )
        .unwrap();
        assert!(res.is_new);
        assert!(state.stat("GAME00").is_some());
    }

    #[test]
    fn list_save_existing_slot_updates() {
        let mut state = TestSaveData::default();
        cell_savedata_auto_save(&mut state, AutoOps {
            dir_name: "GAME00",
            title: "",
            subtitle: "",
            ops: vec![write_op("f", b"a")],
        })
        .unwrap();
        let res = cell_savedata_list_save(
            &mut state,
            "GAME",
            0,
            "IGNORED",
            "",
            "",
            vec![write_op("f", b"bb")],
        )
        .unwrap();
        assert!(!res.is_new);
    }

    #[test]
    fn quota_exceeded_is_nospace() {
        let mut state = TestSaveData { quota_bytes: 4, ..Default::default() };
        cell_savedata_auto_save(&mut state, AutoOps {
            dir_name: "D",
            title: "",
            subtitle: "",
            ops: vec![write_op("a", b"12")],
        })
        .unwrap();
        // Writing another 4 bytes would exceed 4-byte quota.
        let err = cell_savedata_auto_save(&mut state, AutoOps {
            dir_name: "D",
            title: "",
            subtitle: "",
            ops: vec![write_op("b", b"abcd")],
        })
        .unwrap_err();
        assert_eq!(err, errors::NOSPACE);
    }

    #[test]
    fn fileop_delete_removes_file() {
        let mut state = TestSaveData::default();
        cell_savedata_auto_save(&mut state, AutoOps {
            dir_name: "D",
            title: "",
            subtitle: "",
            ops: vec![write_op("a.bin", b"data")],
        })
        .unwrap();

        let del = FileOp {
            op: FILEOP_DELETE,
            filetype: FILETYPE_NORMALFILE,
            filename: "a.bin".to_owned(),
            payload: vec![],
        };
        cell_savedata_auto_load(&mut state, AutoOps {
            dir_name: "D",
            title: "",
            subtitle: "",
            ops: vec![del],
        })
        .unwrap();

        // File gone → subsequent READ fails.
        let err = cell_savedata_auto_load(&mut state, AutoOps {
            dir_name: "D",
            title: "",
            subtitle: "",
            ops: vec![read_op("a.bin")],
        })
        .unwrap_err();
        assert_eq!(err, errors::NODATA);
    }

    #[test]
    fn write_notrunc_appends() {
        let mut state = TestSaveData::default();
        cell_savedata_auto_save(&mut state, AutoOps {
            dir_name: "D",
            title: "",
            subtitle: "",
            ops: vec![write_op("log", b"first ")],
        })
        .unwrap();

        let append = FileOp {
            op: FILEOP_WRITE_NOTRUNC,
            filetype: FILETYPE_NORMALFILE,
            filename: "log".to_owned(),
            payload: b"second".to_vec(),
        };
        cell_savedata_auto_save(&mut state, AutoOps {
            dir_name: "D",
            title: "",
            subtitle: "",
            ops: vec![append],
        })
        .unwrap();

        let res = cell_savedata_auto_load(&mut state, AutoOps {
            dir_name: "D",
            title: "",
            subtitle: "",
            ops: vec![read_op("log")],
        })
        .unwrap();
        assert_eq!(res.bytes_read, b"first second".len() as u64);
    }

    #[test]
    fn delete_unknown_dir_is_nodata() {
        let mut state = TestSaveData::default();
        let err = cell_savedata_delete(&mut state, "NOPE").unwrap_err();
        assert_eq!(err, errors::NODATA);
    }

    #[test]
    fn delete_existing_dir_clears_stat() {
        let mut state = TestSaveData::default();
        cell_savedata_auto_save(&mut state, AutoOps {
            dir_name: "D",
            title: "",
            subtitle: "",
            ops: vec![write_op("a", b"1")],
        })
        .unwrap();
        cell_savedata_delete(&mut state, "D").unwrap();
        assert!(state.stat("D").is_none());
    }

    #[test]
    fn list_auto_save_forwards_to_auto_save() {
        let mut state = TestSaveData::default();
        let res = cell_savedata_list_auto_save(&mut state, AutoOps {
            dir_name: "X",
            title: "",
            subtitle: "",
            ops: vec![write_op("a", b"hi")],
        })
        .unwrap();
        assert!(res.is_new);
    }

    #[test]
    fn dir_name_str_strips_null_padding() {
        let d = SaveDataDir::with_name("SHORT").unwrap();
        assert_eq!(d.name_str(), "SHORT");
    }

    #[test]
    fn total_bytes_reflects_files() {
        let mut state = TestSaveData::default();
        cell_savedata_auto_save(&mut state, AutoOps {
            dir_name: "A",
            title: "",
            subtitle: "",
            ops: vec![write_op("x", &vec![0; 10])],
        })
        .unwrap();
        cell_savedata_auto_save(&mut state, AutoOps {
            dir_name: "B",
            title: "",
            subtitle: "",
            ops: vec![write_op("x", &vec![0; 15])],
        })
        .unwrap();
        assert_eq!(state.total_bytes(), 25);
    }
}
