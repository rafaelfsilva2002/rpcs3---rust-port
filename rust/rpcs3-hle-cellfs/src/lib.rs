//! `rpcs3-hle-cellfs` — game-facing filesystem HLE wrapper.
//!
//! Ports `rpcs3/Emu/Cell/Modules/cellFs.cpp`. The cellFs API is a
//! thin layer over the `sys_fs_*` syscalls implemented in
//! `rpcs3-lv2-fs`; this crate's job is to translate between the
//! game-visible `cellFs*` calling convention (which uses PS3 Unix
//! flag values like `CELL_FS_O_CREAT = 0o100`) and the already-
//! validated trait API from lv2-fs.
//!
//! ## Entry points covered
//!
//! | HLE function               | Rust wrapper                  |
//! |----------------------------|-------------------------------|
//! | `cellFsOpen`               | [`cell_fs_open`]              |
//! | `cellFsClose`              | [`cell_fs_close`]             |
//! | `cellFsRead`               | [`cell_fs_read`]              |
//! | `cellFsWrite`              | [`cell_fs_write`]             |
//! | `cellFsLseek`              | [`cell_fs_lseek`]             |
//! | `cellFsStat`               | [`cell_fs_stat`]              |
//! | `cellFsMkdir`              | [`cell_fs_mkdir`]             |
//! | `cellFsRmdir`              | [`cell_fs_rmdir`]             |
//! | `cellFsUnlink`             | [`cell_fs_unlink`]            |
//! | `cellFsOpendir`            | [`cell_fs_opendir`]           |
//! | `cellFsReaddir`            | [`cell_fs_readdir`]           |
//! | `cellFsClosedir`           | [`cell_fs_closedir`]          |
//! | `cellFsRename`             | [`cell_fs_rename`] (stub)     |
//! | `cellFsTruncate`           | [`cell_fs_truncate`] (stub)   |
//! | `cellFsChmod`              | [`cell_fs_chmod`] (stub)      |
//!
//! ## Frozen constants (from `sys_fs.h:14-47`, octal)

use rpcs3_emu_types::CellError;
use rpcs3_lv2_fs as lv2;

// =====================================================================
// Constants — cellFs uses PS3 Unix octal flags (different from the
// LV2 bitfield constants in `rpcs3-lv2-fs`).
// =====================================================================

pub const O_RDONLY: u32 = 0o0;
pub const O_WRONLY: u32 = 0o1;
pub const O_RDWR: u32 = 0o2;
pub const O_ACCMODE: u32 = 0o3;
pub const O_CREAT: u32 = 0o100;
pub const O_EXCL: u32 = 0o200;
pub const O_TRUNC: u32 = 0o1000;
pub const O_APPEND: u32 = 0o2000;
pub const O_MSELF: u32 = 0o10000;

pub const SEEK_SET: u32 = 0;
pub const SEEK_CUR: u32 = 1;
pub const SEEK_END: u32 = 2;

pub const S_IFMT: u32 = 0o170000;
pub const S_IFDIR: u32 = 0o040000;
pub const S_IFREG: u32 = 0o100000;
pub const S_IFLNK: u32 = 0o120000;

pub const MAX_FS_PATH_LENGTH: usize = 1024;
pub const MAX_FS_FILE_NAME_LENGTH: usize = 255;

// =====================================================================
// Flag translation (cellFs octal → lv2 bitfield)
// =====================================================================

/// Translate cellFs open-flags to the lv2-fs bitfield. Returns
/// `EINVAL` if unknown bits are set.
#[must_use]
pub fn translate_open_flags(cell_flags: u32) -> Result<u32, CellError> {
    let allowed = O_ACCMODE | O_CREAT | O_EXCL | O_TRUNC | O_APPEND | O_MSELF;
    if cell_flags & !allowed != 0 {
        return Err(CellError::EINVAL);
    }
    let mut out = 0u32;
    match cell_flags & O_ACCMODE {
        O_RDONLY => out |= lv2::O_RDONLY,
        O_WRONLY => out |= lv2::O_WRONLY,
        O_RDWR => out |= lv2::O_RDWR,
        _ => return Err(CellError::EINVAL),
    }
    if cell_flags & O_CREAT != 0 { out |= lv2::O_CREAT; }
    if cell_flags & O_EXCL != 0 { out |= lv2::O_EXCL; }
    if cell_flags & O_TRUNC != 0 { out |= lv2::O_TRUNC; }
    if cell_flags & O_APPEND != 0 { out |= lv2::O_APPEND; }
    Ok(out)
}

fn validate_path(path: &str) -> Result<(), CellError> {
    if path.is_empty() || path.len() > MAX_FS_PATH_LENGTH {
        return Err(CellError::EINVAL);
    }
    if !path.starts_with('/') {
        return Err(CellError::EINVAL);
    }
    Ok(())
}

// =====================================================================
// Wrappers — delegate to rpcs3-lv2-fs
// =====================================================================

/// `cellFsOpen(path, flags, fd_out, arg, size)`.
#[must_use]
pub fn cell_fs_open<F: lv2::FileSystem + ?Sized>(
    fs: &mut F,
    table: &mut lv2::FdTable,
    path: &str,
    cell_flags: u32,
    _mode: u32,
) -> Result<u32, CellError> {
    validate_path(path)?;
    let lv2_flags = translate_open_flags(cell_flags)?;
    lv2::sys_fs_open(fs, table, path, lv2_flags)
}

#[must_use]
pub fn cell_fs_close<F: lv2::FileSystem + ?Sized>(
    fs: &mut F,
    table: &mut lv2::FdTable,
    fd: u32,
) -> Result<(), CellError> {
    lv2::sys_fs_close(fs, table, fd)
}

#[must_use]
pub fn cell_fs_read<F: lv2::FileSystem + ?Sized>(
    fs: &mut F,
    table: &mut lv2::FdTable,
    fd: u32,
    buf: &mut [u8],
) -> Result<u64, CellError> {
    lv2::sys_fs_read(fs, table, fd, buf)
}

#[must_use]
pub fn cell_fs_write<F: lv2::FileSystem + ?Sized>(
    fs: &mut F,
    table: &mut lv2::FdTable,
    fd: u32,
    buf: &[u8],
) -> Result<u64, CellError> {
    lv2::sys_fs_write(fs, table, fd, buf)
}

#[must_use]
pub fn cell_fs_lseek<F: lv2::FileSystem + ?Sized>(
    fs: &mut F,
    table: &mut lv2::FdTable,
    fd: u32,
    offset: i64,
    whence: u32,
) -> Result<u64, CellError> {
    if whence > SEEK_END {
        return Err(CellError::EINVAL);
    }
    lv2::sys_fs_lseek(fs, table, fd, offset, whence as i32)
}

#[must_use]
pub fn cell_fs_stat<F: lv2::FileSystem + ?Sized>(
    fs: &F,
    path: &str,
) -> Result<lv2::CellFsStat, CellError> {
    validate_path(path)?;
    lv2::sys_fs_stat(fs, path)
}

#[must_use]
pub fn cell_fs_mkdir<F: lv2::FileSystem + ?Sized>(
    fs: &mut F,
    path: &str,
    mode: u32,
) -> Result<(), CellError> {
    validate_path(path)?;
    lv2::sys_fs_mkdir(fs, path, mode)
}

#[must_use]
pub fn cell_fs_rmdir<F: lv2::FileSystem + ?Sized>(
    fs: &mut F,
    path: &str,
) -> Result<(), CellError> {
    validate_path(path)?;
    lv2::sys_fs_rmdir(fs, path)
}

#[must_use]
pub fn cell_fs_unlink<F: lv2::FileSystem + ?Sized>(
    fs: &mut F,
    path: &str,
) -> Result<(), CellError> {
    validate_path(path)?;
    lv2::sys_fs_unlink(fs, path)
}

#[must_use]
pub fn cell_fs_opendir<F: lv2::FileSystem + ?Sized>(
    fs: &mut F,
    table: &mut lv2::FdTable,
    path: &str,
) -> Result<u32, CellError> {
    validate_path(path)?;
    lv2::sys_fs_opendir(fs, table, path)
}

#[must_use]
pub fn cell_fs_readdir<F: lv2::FileSystem + ?Sized>(
    fs: &mut F,
    table: &mut lv2::FdTable,
    dd: u32,
) -> Result<Option<lv2::DirEntry>, CellError> {
    lv2::sys_fs_readdir(fs, table, dd)
}

#[must_use]
pub fn cell_fs_closedir<F: lv2::FileSystem + ?Sized>(
    fs: &mut F,
    table: &mut lv2::FdTable,
    dd: u32,
) -> Result<(), CellError> {
    lv2::sys_fs_closedir(fs, table, dd)
}

/// `cellFsRename(from, to)`. Stub — not yet wired through lv2-fs.
#[must_use]
pub fn cell_fs_rename<F: lv2::FileSystem + ?Sized>(
    _fs: &mut F,
    from: &str,
    to: &str,
) -> Result<(), CellError> {
    validate_path(from)?;
    validate_path(to)?;
    Err(CellError::ENOSYS)
}

#[must_use]
pub fn cell_fs_truncate<F: lv2::FileSystem + ?Sized>(
    _fs: &mut F,
    path: &str,
    _size: u64,
) -> Result<(), CellError> {
    validate_path(path)?;
    Err(CellError::ENOSYS)
}

#[must_use]
pub fn cell_fs_chmod<F: lv2::FileSystem + ?Sized>(
    _fs: &mut F,
    path: &str,
    _mode: u32,
) -> Result<(), CellError> {
    validate_path(path)?;
    Err(CellError::ENOSYS)
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // --- flag translation ----------------------------------------

    #[test]
    fn translate_rdonly_maps_cleanly() {
        assert_eq!(translate_open_flags(O_RDONLY).unwrap(), lv2::O_RDONLY);
    }

    #[test]
    fn translate_wronly_create_trunc() {
        let out = translate_open_flags(O_WRONLY | O_CREAT | O_TRUNC).unwrap();
        assert_eq!(out, lv2::O_WRONLY | lv2::O_CREAT | lv2::O_TRUNC);
    }

    #[test]
    fn translate_rdwr_append() {
        let out = translate_open_flags(O_RDWR | O_APPEND).unwrap();
        assert_eq!(out, lv2::O_RDWR | lv2::O_APPEND);
    }

    #[test]
    fn translate_rejects_unknown_bits() {
        let err = translate_open_flags(0x4000_0000 | O_RDONLY).unwrap_err();
        assert_eq!(err, CellError::EINVAL);
    }

    #[test]
    fn translate_rejects_access_mode_3() {
        let err = translate_open_flags(O_ACCMODE).unwrap_err();
        assert_eq!(err, CellError::EINVAL);
    }

    // --- path validation ------------------------------------------

    #[test]
    fn validate_path_rejects_empty() {
        assert_eq!(validate_path("").unwrap_err(), CellError::EINVAL);
    }

    #[test]
    fn validate_path_rejects_relative() {
        assert_eq!(validate_path("foo.txt").unwrap_err(), CellError::EINVAL);
    }

    #[test]
    fn validate_path_rejects_overlong() {
        let long = format!("/{}", "x".repeat(MAX_FS_PATH_LENGTH));
        assert_eq!(validate_path(&long).unwrap_err(), CellError::EINVAL);
    }

    #[test]
    fn validate_path_accepts_abs_slash() {
        validate_path("/dev_hdd0/foo/bar.bin").unwrap();
    }

    // --- constants ------------------------------------------------

    #[test]
    fn octal_constants_match_sys_fs_h() {
        assert_eq!(O_RDONLY, 0);
        assert_eq!(O_WRONLY, 1);
        assert_eq!(O_RDWR, 2);
        assert_eq!(O_CREAT, 0o100);
        assert_eq!(O_EXCL, 0o200);
        assert_eq!(O_TRUNC, 0o1000);
        assert_eq!(O_APPEND, 0o2000);
    }

    #[test]
    fn seek_constants_match() {
        assert_eq!(SEEK_SET, 0);
        assert_eq!(SEEK_CUR, 1);
        assert_eq!(SEEK_END, 2);
    }

    #[test]
    fn stat_mode_bits_match() {
        assert_eq!(S_IFDIR, 0o040000);
        assert_eq!(S_IFREG, 0o100000);
        assert_eq!(S_IFLNK, 0o120000);
    }

    #[test]
    fn max_path_length_is_1024() {
        assert_eq!(MAX_FS_PATH_LENGTH, 1024);
        assert_eq!(MAX_FS_FILE_NAME_LENGTH, 255);
    }

    // --- minimal test-only FileSystem impl ------------------------

    #[derive(Default)]
    struct NullFs;
    impl lv2::FileSystem for NullFs {
        fn open(&mut self, _: &str, _: u32) -> Result<lv2::FileObject, CellError> {
            Err(CellError::ENOENT)
        }
        fn close(&mut self, _: &mut lv2::FileObject) -> Result<(), CellError> { Ok(()) }
        fn read(&mut self, _: &mut lv2::FileObject, _: &mut [u8]) -> Result<u64, CellError> { Ok(0) }
        fn write(&mut self, _: &mut lv2::FileObject, _: &[u8]) -> Result<u64, CellError> { Ok(0) }
        fn seek(&mut self, _: &mut lv2::FileObject, _: i64, _: lv2::Whence) -> Result<u64, CellError> { Ok(0) }
        fn stat(&self, _: &str) -> Result<lv2::CellFsStat, CellError> {
            Err(CellError::ENOENT)
        }
        fn mkdir(&mut self, _: &str, _: u32) -> Result<(), CellError> { Ok(()) }
        fn rmdir(&mut self, _: &str) -> Result<(), CellError> { Ok(()) }
        fn unlink(&mut self, _: &str) -> Result<(), CellError> { Ok(()) }
        fn opendir(&mut self, _: &str) -> Result<lv2::DirObject, CellError> {
            Err(CellError::ENOENT)
        }
        fn closedir(&mut self, _: &mut lv2::DirObject) -> Result<(), CellError> { Ok(()) }
        fn readdir(&mut self, _: &mut lv2::DirObject) -> Result<Option<lv2::DirEntry>, CellError> {
            Ok(None)
        }
    }

    // --- stub errors ----------------------------------------------

    #[test]
    fn truncate_stub_returns_enosys() {
        let mut fs = NullFs;
        assert_eq!(
            cell_fs_truncate(&mut fs, "/foo", 0).unwrap_err(),
            CellError::ENOSYS,
        );
    }

    #[test]
    fn rename_stub_returns_enosys() {
        let mut fs = NullFs;
        assert_eq!(
            cell_fs_rename(&mut fs, "/a", "/b").unwrap_err(),
            CellError::ENOSYS,
        );
    }

    #[test]
    fn chmod_stub_returns_enosys() {
        let mut fs = NullFs;
        assert_eq!(
            cell_fs_chmod(&mut fs, "/a", 0o644).unwrap_err(),
            CellError::ENOSYS,
        );
    }

    #[test]
    fn truncate_rejects_empty_path_first() {
        let mut fs = NullFs;
        assert_eq!(
            cell_fs_truncate(&mut fs, "", 0).unwrap_err(),
            CellError::EINVAL,
        );
    }

    // --- path validation happens before delegating ----------------

    #[test]
    fn open_with_empty_path_rejects_before_hitting_fs() {
        let mut fs = NullFs;
        let mut table = lv2::FdTable::default();
        assert_eq!(
            cell_fs_open(&mut fs, &mut table, "", O_RDONLY, 0).unwrap_err(),
            CellError::EINVAL,
        );
    }

    #[test]
    fn lseek_rejects_bad_whence_before_fs_call() {
        let mut fs = NullFs;
        let mut table = lv2::FdTable::default();
        assert_eq!(
            cell_fs_lseek(&mut fs, &mut table, 0, 0, 99).unwrap_err(),
            CellError::EINVAL,
        );
    }

    #[test]
    fn mkdir_with_relative_path_rejects_before_fs() {
        let mut fs = NullFs;
        assert_eq!(
            cell_fs_mkdir(&mut fs, "relative", 0o755).unwrap_err(),
            CellError::EINVAL,
        );
    }

    #[test]
    fn unlink_with_empty_path_rejects_before_fs() {
        let mut fs = NullFs;
        assert_eq!(
            cell_fs_unlink(&mut fs, "").unwrap_err(),
            CellError::EINVAL,
        );
    }

    #[test]
    fn opendir_validates_path_first() {
        let mut fs = NullFs;
        let mut table = lv2::FdTable::default();
        assert_eq!(
            cell_fs_opendir(&mut fs, &mut table, "bad").unwrap_err(),
            CellError::EINVAL,
        );
    }
}
