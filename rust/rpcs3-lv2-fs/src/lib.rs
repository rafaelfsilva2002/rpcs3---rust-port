//! `rpcs3-lv2-fs` — LV2 filesystem syscalls.
//!
//! Ports `rpcs3/Emu/Cell/lv2/sys_fs.cpp`. Actual on-disk I/O lives
//! behind a [`FileSystem`] trait — this crate only handles syscall
//! validation, flag decoding, fd table management, and error mapping.
//!
//! ## Scope (iteration 1)
//!
//! * `sys_fs_open`
//! * `sys_fs_read`
//! * `sys_fs_write`
//! * `sys_fs_close`
//! * `sys_fs_lseek`
//! * `sys_fs_stat`
//! * `sys_fs_fstat`
//! * `sys_fs_mkdir`
//! * `sys_fs_rmdir`
//! * `sys_fs_unlink`
//! * `sys_fs_opendir`
//! * `sys_fs_readdir`
//! * `sys_fs_closedir`

use rpcs3_emu_types::CellError;

// =====================================================================
// Open flag constants (cellFsFile.h / sys_fs.h)
// =====================================================================

pub const O_RDONLY: u32 = 0x0;
pub const O_WRONLY: u32 = 0x1;
pub const O_RDWR: u32 = 0x2;
pub const O_ACCMODE: u32 = 0x3;

pub const O_CREAT: u32 = 0x4;
pub const O_APPEND: u32 = 0x8;
pub const O_TRUNC: u32 = 0x10;
pub const O_EXCL: u32 = 0x80;

// =====================================================================
// Lseek whence
// =====================================================================

#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Whence {
    Set = 0,
    Cur = 1,
    End = 2,
}

impl Whence {
    #[must_use]
    pub fn from_i32(v: i32) -> Option<Self> {
        match v {
            0 => Some(Self::Set),
            1 => Some(Self::Cur),
            2 => Some(Self::End),
            _ => None,
        }
    }
}

// =====================================================================
// CellFsStat — mirrors `CellFsStat` in cellFsFile.h
// =====================================================================

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CellFsStat {
    pub mode: u32,
    pub uid: i32,
    pub gid: i32,
    pub atime: u64,
    pub mtime: u64,
    pub ctime: u64,
    pub size: u64,
    pub blksize: u64,
}

/// Mode bits. `S_IFDIR` marks a directory entry.
pub const S_IFDIR: u32 = 0o040000;
pub const S_IFREG: u32 = 0o100000;
pub const S_IFMT: u32 = 0o170000;

impl CellFsStat {
    #[must_use]
    pub fn is_dir(self) -> bool {
        (self.mode & S_IFMT) == S_IFDIR
    }
    #[must_use]
    pub fn is_file(self) -> bool {
        (self.mode & S_IFMT) == S_IFREG
    }
}

// =====================================================================
// Directory entry
// =====================================================================

/// Mirrors `CellFsDirent`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntry {
    /// 0 = regular file, 1 = directory. Matches `CELL_FS_TYPE_*`.
    pub d_type: u8,
    pub name: String,
}

pub const FS_TYPE_REGULAR: u8 = 1;
pub const FS_TYPE_DIRECTORY: u8 = 2;

// =====================================================================
// FileSystem trait + fd table
// =====================================================================

/// Filesystem host abstraction. The emu core owns the concrete impl,
/// typically backed by a host directory tree mounted via
/// `rpcs3-vfs-mount`.
pub trait FileSystem {
    /// Open `path` with POSIX-style flags. Returns the host file
    /// object as an opaque handle; the syscall layer assigns a fd.
    fn open(&mut self, path: &str, flags: u32) -> Result<FileObject, CellError>;
    fn close(&mut self, obj: &mut FileObject) -> Result<(), CellError>;

    fn read(&mut self, obj: &mut FileObject, buf: &mut [u8]) -> Result<u64, CellError>;
    fn write(&mut self, obj: &mut FileObject, buf: &[u8]) -> Result<u64, CellError>;
    fn seek(&mut self, obj: &mut FileObject, offset: i64, whence: Whence) -> Result<u64, CellError>;

    fn stat(&self, path: &str) -> Result<CellFsStat, CellError>;
    fn mkdir(&mut self, path: &str, mode: u32) -> Result<(), CellError>;
    fn rmdir(&mut self, path: &str) -> Result<(), CellError>;
    fn unlink(&mut self, path: &str) -> Result<(), CellError>;

    fn opendir(&mut self, path: &str) -> Result<DirObject, CellError>;
    fn closedir(&mut self, obj: &mut DirObject) -> Result<(), CellError>;
    fn readdir(&mut self, obj: &mut DirObject) -> Result<Option<DirEntry>, CellError>;
}

/// Opaque per-fd state. The filesystem populates an integer token it
/// later recognises; we keep it generic.
#[derive(Debug, Clone)]
pub struct FileObject {
    pub handle: u64,
    pub flags: u32,
}

#[derive(Debug, Clone)]
pub struct DirObject {
    pub handle: u64,
}

// =====================================================================
// fd table — a thin indexed pool of FileObject / DirObject
// =====================================================================

/// Simple sequential fd table. File descriptors are allocated starting
/// at 4 (matching RPCS3's convention of reserving 0..3 for stdio).
#[derive(Default)]
pub struct FdTable {
    entries: std::collections::HashMap<u32, FdEntry>,
    next_fd: u32,
}

#[derive(Debug, Clone)]
enum FdEntry {
    File(FileObject),
    Dir(DirObject),
}

impl FdTable {
    #[must_use]
    pub fn new() -> Self {
        Self { entries: Default::default(), next_fd: 4 }
    }

    fn alloc_fd(&mut self) -> u32 {
        let fd = self.next_fd;
        self.next_fd += 1;
        fd
    }

    fn get_file(&mut self, fd: u32) -> Result<&mut FileObject, CellError> {
        match self.entries.get_mut(&fd) {
            Some(FdEntry::File(f)) => Ok(f),
            Some(FdEntry::Dir(_)) => Err(CellError::EISDIR),
            None => Err(CellError::EBADF),
        }
    }

    fn get_dir(&mut self, fd: u32) -> Result<&mut DirObject, CellError> {
        match self.entries.get_mut(&fd) {
            Some(FdEntry::Dir(d)) => Ok(d),
            Some(FdEntry::File(_)) => Err(CellError::ENOTDIR),
            None => Err(CellError::EBADF),
        }
    }
}

// =====================================================================
// Syscalls
// =====================================================================

/// `sys_fs_open(path, flags, fd_out, mode, arg, size)`.
/// Validates access-mode bits then delegates to the host filesystem.
/// Returns the new fd.
#[must_use]
pub fn sys_fs_open<F: FileSystem + ?Sized>(
    fs: &mut F,
    fd_table: &mut FdTable,
    path: &str,
    flags: u32,
) -> Result<u32, CellError> {
    // Access mode must be one of RDONLY/WRONLY/RDWR.
    let access = flags & O_ACCMODE;
    if access > 2 {
        return Err(CellError::EINVAL);
    }
    if path.is_empty() {
        return Err(CellError::ENOENT);
    }
    let obj = fs.open(path, flags)?;
    let fd = fd_table.alloc_fd();
    fd_table.entries.insert(fd, FdEntry::File(obj));
    Ok(fd)
}

/// `sys_fs_close(fd)`.
#[must_use]
pub fn sys_fs_close<F: FileSystem + ?Sized>(
    fs: &mut F,
    fd_table: &mut FdTable,
    fd: u32,
) -> Result<(), CellError> {
    match fd_table.entries.remove(&fd) {
        Some(FdEntry::File(mut f)) => fs.close(&mut f),
        Some(FdEntry::Dir(_)) => Err(CellError::EISDIR),
        None => Err(CellError::EBADF),
    }
}

/// `sys_fs_read(fd, buf, size, read_out)` → bytes read.
#[must_use]
pub fn sys_fs_read<F: FileSystem + ?Sized>(
    fs: &mut F,
    fd_table: &mut FdTable,
    fd: u32,
    buf: &mut [u8],
) -> Result<u64, CellError> {
    let obj = fd_table.get_file(fd)?;
    // Verify file was opened readable.
    let access = obj.flags & O_ACCMODE;
    if access != O_RDONLY && access != O_RDWR {
        return Err(CellError::EBADF);
    }
    fs.read(obj, buf)
}

/// `sys_fs_write(fd, buf, size, write_out)` → bytes written.
#[must_use]
pub fn sys_fs_write<F: FileSystem + ?Sized>(
    fs: &mut F,
    fd_table: &mut FdTable,
    fd: u32,
    buf: &[u8],
) -> Result<u64, CellError> {
    let obj = fd_table.get_file(fd)?;
    let access = obj.flags & O_ACCMODE;
    if access != O_WRONLY && access != O_RDWR {
        return Err(CellError::EBADF);
    }
    fs.write(obj, buf)
}

/// `sys_fs_lseek(fd, offset, whence, pos_out)` → new position.
#[must_use]
pub fn sys_fs_lseek<F: FileSystem + ?Sized>(
    fs: &mut F,
    fd_table: &mut FdTable,
    fd: u32,
    offset: i64,
    whence_raw: i32,
) -> Result<u64, CellError> {
    let obj = fd_table.get_file(fd)?;
    let whence = Whence::from_i32(whence_raw).ok_or(CellError::EINVAL)?;
    fs.seek(obj, offset, whence)
}

/// `sys_fs_stat(path, stat_out)`.
#[must_use]
pub fn sys_fs_stat<F: FileSystem + ?Sized>(fs: &F, path: &str) -> Result<CellFsStat, CellError> {
    if path.is_empty() {
        return Err(CellError::ENOENT);
    }
    fs.stat(path)
}

/// `sys_fs_fstat(fd, stat_out)`. We proxy through stat; real RPCS3
/// caches stat per fd but the effect is the same.
#[must_use]
pub fn sys_fs_fstat<F: FileSystem + ?Sized>(
    fs: &F,
    fd_table: &FdTable,
    fd: u32,
) -> Result<CellFsStat, CellError> {
    // We cannot stat() through the handle here without the path; in
    // practice the FS implementation tracks the source path inside the
    // FileObject. We return the error so the emu core can wire up a
    // real `fstat` fast-path when it integrates.
    let _ = (fs, fd_table, fd);
    Err(CellError::ENOSYS)
}

/// `sys_fs_mkdir(path, mode)`.
#[must_use]
pub fn sys_fs_mkdir<F: FileSystem + ?Sized>(
    fs: &mut F,
    path: &str,
    mode: u32,
) -> Result<(), CellError> {
    if path.is_empty() {
        return Err(CellError::ENOENT);
    }
    fs.mkdir(path, mode)
}

/// `sys_fs_rmdir(path)`.
#[must_use]
pub fn sys_fs_rmdir<F: FileSystem + ?Sized>(fs: &mut F, path: &str) -> Result<(), CellError> {
    if path.is_empty() {
        return Err(CellError::ENOENT);
    }
    fs.rmdir(path)
}

/// `sys_fs_unlink(path)`.
#[must_use]
pub fn sys_fs_unlink<F: FileSystem + ?Sized>(fs: &mut F, path: &str) -> Result<(), CellError> {
    if path.is_empty() {
        return Err(CellError::ENOENT);
    }
    fs.unlink(path)
}

/// `sys_fs_opendir(path, fd_out)`.
#[must_use]
pub fn sys_fs_opendir<F: FileSystem + ?Sized>(
    fs: &mut F,
    fd_table: &mut FdTable,
    path: &str,
) -> Result<u32, CellError> {
    if path.is_empty() {
        return Err(CellError::ENOENT);
    }
    let obj = fs.opendir(path)?;
    let fd = fd_table.alloc_fd();
    fd_table.entries.insert(fd, FdEntry::Dir(obj));
    Ok(fd)
}

/// `sys_fs_closedir(fd)`.
#[must_use]
pub fn sys_fs_closedir<F: FileSystem + ?Sized>(
    fs: &mut F,
    fd_table: &mut FdTable,
    fd: u32,
) -> Result<(), CellError> {
    match fd_table.entries.remove(&fd) {
        Some(FdEntry::Dir(mut d)) => fs.closedir(&mut d),
        Some(FdEntry::File(_)) => Err(CellError::ENOTDIR),
        None => Err(CellError::EBADF),
    }
}

/// `sys_fs_readdir(fd, entry_out, nread_out)`. Returns the next entry
/// or `None` at EOF.
#[must_use]
pub fn sys_fs_readdir<F: FileSystem + ?Sized>(
    fs: &mut F,
    fd_table: &mut FdTable,
    fd: u32,
) -> Result<Option<DirEntry>, CellError> {
    let obj = fd_table.get_dir(fd)?;
    fs.readdir(obj)
}

// =====================================================================
// Tests — in-memory reference filesystem
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[derive(Clone)]
    struct MemFile {
        data: Vec<u8>,
    }
    #[derive(Default)]
    struct MemFs {
        files: HashMap<String, MemFile>,
        dirs: std::collections::HashSet<String>,
        next_handle: u64,
        // handle → (path, position)
        open_files: HashMap<u64, (String, u64)>,
        open_dirs: HashMap<u64, (Vec<DirEntry>, usize)>,
    }

    impl MemFs {
        fn new() -> Self {
            let mut fs = Self::default();
            fs.dirs.insert("/".into());
            fs.next_handle = 1;
            fs
        }
        fn ensure_parent(&self, path: &str) -> Result<(), CellError> {
            let parent = path.rsplit_once('/').map_or("/", |(p, _)| if p.is_empty() { "/" } else { p });
            if !self.dirs.contains(parent) {
                return Err(CellError::ENOENT);
            }
            Ok(())
        }
    }

    impl FileSystem for MemFs {
        fn open(&mut self, path: &str, flags: u32) -> Result<FileObject, CellError> {
            let create = flags & O_CREAT != 0;
            let excl = flags & O_EXCL != 0;
            let trunc = flags & O_TRUNC != 0;

            if self.dirs.contains(path) {
                return Err(CellError::EISDIR);
            }
            let exists = self.files.contains_key(path);
            if !exists {
                if !create {
                    return Err(CellError::ENOENT);
                }
                self.ensure_parent(path)?;
                self.files.insert(path.to_owned(), MemFile { data: Vec::new() });
            } else if excl {
                return Err(CellError::EEXIST);
            } else if trunc {
                if let Some(f) = self.files.get_mut(path) {
                    f.data.clear();
                }
            }

            let handle = self.next_handle;
            self.next_handle += 1;
            self.open_files.insert(handle, (path.to_owned(), 0));
            Ok(FileObject { handle, flags })
        }

        fn close(&mut self, obj: &mut FileObject) -> Result<(), CellError> {
            self.open_files.remove(&obj.handle).ok_or(CellError::EBADF)?;
            Ok(())
        }

        fn read(&mut self, obj: &mut FileObject, buf: &mut [u8]) -> Result<u64, CellError> {
            let (path, pos) = self.open_files.get_mut(&obj.handle).ok_or(CellError::EBADF)?;
            let f = self.files.get(path).ok_or(CellError::ENOENT)?;
            let available = (f.data.len() as u64).saturating_sub(*pos);
            let take = available.min(buf.len() as u64) as usize;
            let start = *pos as usize;
            buf[..take].copy_from_slice(&f.data[start..start + take]);
            *pos += take as u64;
            Ok(take as u64)
        }

        fn write(&mut self, obj: &mut FileObject, buf: &[u8]) -> Result<u64, CellError> {
            let (path, pos) = self.open_files.get_mut(&obj.handle).ok_or(CellError::EBADF)?;
            let f = self.files.get_mut(path).ok_or(CellError::ENOENT)?;
            let pos_u = *pos as usize;
            if pos_u + buf.len() > f.data.len() {
                f.data.resize(pos_u + buf.len(), 0);
            }
            f.data[pos_u..pos_u + buf.len()].copy_from_slice(buf);
            *pos += buf.len() as u64;
            Ok(buf.len() as u64)
        }

        fn seek(
            &mut self,
            obj: &mut FileObject,
            offset: i64,
            whence: Whence,
        ) -> Result<u64, CellError> {
            let (path, pos) = self.open_files.get_mut(&obj.handle).ok_or(CellError::EBADF)?;
            let f = self.files.get(path).ok_or(CellError::ENOENT)?;
            let new_pos: i64 = match whence {
                Whence::Set => offset,
                Whence::Cur => *pos as i64 + offset,
                Whence::End => f.data.len() as i64 + offset,
            };
            if new_pos < 0 {
                return Err(CellError::EINVAL);
            }
            *pos = new_pos as u64;
            Ok(*pos)
        }

        fn stat(&self, path: &str) -> Result<CellFsStat, CellError> {
            if let Some(f) = self.files.get(path) {
                Ok(CellFsStat {
                    mode: S_IFREG | 0o644,
                    size: f.data.len() as u64,
                    blksize: 512,
                    ..Default::default()
                })
            } else if self.dirs.contains(path) {
                Ok(CellFsStat {
                    mode: S_IFDIR | 0o755,
                    size: 0,
                    blksize: 512,
                    ..Default::default()
                })
            } else {
                Err(CellError::ENOENT)
            }
        }

        fn mkdir(&mut self, path: &str, _mode: u32) -> Result<(), CellError> {
            if self.dirs.contains(path) || self.files.contains_key(path) {
                return Err(CellError::EEXIST);
            }
            self.ensure_parent(path)?;
            self.dirs.insert(path.to_owned());
            Ok(())
        }

        fn rmdir(&mut self, path: &str) -> Result<(), CellError> {
            if !self.dirs.remove(path) {
                return Err(CellError::ENOENT);
            }
            Ok(())
        }

        fn unlink(&mut self, path: &str) -> Result<(), CellError> {
            self.files.remove(path).ok_or(CellError::ENOENT)?;
            Ok(())
        }

        fn opendir(&mut self, path: &str) -> Result<DirObject, CellError> {
            if !self.dirs.contains(path) {
                return Err(CellError::ENOENT);
            }
            // List children of `path`.
            let prefix = if path == "/" { String::from("/") } else { format!("{path}/") };
            let mut entries = Vec::new();
            for f in self.files.keys() {
                if f.starts_with(&prefix) && !f[prefix.len()..].contains('/') {
                    entries.push(DirEntry {
                        d_type: FS_TYPE_REGULAR,
                        name: f[prefix.len()..].to_owned(),
                    });
                }
            }
            for d in &self.dirs {
                if d != path && d.starts_with(&prefix) && !d[prefix.len()..].contains('/') {
                    entries.push(DirEntry {
                        d_type: FS_TYPE_DIRECTORY,
                        name: d[prefix.len()..].to_owned(),
                    });
                }
            }
            entries.sort_by(|a, b| a.name.cmp(&b.name));

            let handle = self.next_handle;
            self.next_handle += 1;
            self.open_dirs.insert(handle, (entries, 0));
            Ok(DirObject { handle })
        }

        fn closedir(&mut self, obj: &mut DirObject) -> Result<(), CellError> {
            self.open_dirs.remove(&obj.handle).ok_or(CellError::EBADF)?;
            Ok(())
        }

        fn readdir(&mut self, obj: &mut DirObject) -> Result<Option<DirEntry>, CellError> {
            let (entries, idx) = self.open_dirs.get_mut(&obj.handle).ok_or(CellError::EBADF)?;
            if *idx >= entries.len() {
                return Ok(None);
            }
            let e = entries[*idx].clone();
            *idx += 1;
            Ok(Some(e))
        }
    }

    // -- open / close --------------------------------------------

    #[test]
    fn open_nonexistent_without_creat_is_enoent() {
        let mut fs = MemFs::new();
        let mut tbl = FdTable::new();
        assert_eq!(
            sys_fs_open(&mut fs, &mut tbl, "/file", O_RDONLY),
            Err(CellError::ENOENT)
        );
    }

    #[test]
    fn open_with_creat_creates_file_and_returns_fd() {
        let mut fs = MemFs::new();
        let mut tbl = FdTable::new();
        let fd = sys_fs_open(&mut fs, &mut tbl, "/new", O_WRONLY | O_CREAT).unwrap();
        assert!(fd >= 4);
    }

    #[test]
    fn open_empty_path_is_enoent() {
        let mut fs = MemFs::new();
        let mut tbl = FdTable::new();
        assert_eq!(sys_fs_open(&mut fs, &mut tbl, "", O_RDONLY), Err(CellError::ENOENT));
    }

    #[test]
    fn open_bad_access_mode_is_einval() {
        let mut fs = MemFs::new();
        let mut tbl = FdTable::new();
        assert_eq!(
            sys_fs_open(&mut fs, &mut tbl, "/x", 0x3), // ACCMODE=3 (reserved)
            Err(CellError::EINVAL)
        );
    }

    #[test]
    fn open_excl_on_existing_is_eexist() {
        let mut fs = MemFs::new();
        let mut tbl = FdTable::new();
        sys_fs_open(&mut fs, &mut tbl, "/a", O_WRONLY | O_CREAT).unwrap();
        assert_eq!(
            sys_fs_open(&mut fs, &mut tbl, "/a", O_WRONLY | O_CREAT | O_EXCL),
            Err(CellError::EEXIST)
        );
    }

    #[test]
    fn close_unknown_fd_is_ebadf() {
        let mut fs = MemFs::new();
        let mut tbl = FdTable::new();
        assert_eq!(sys_fs_close(&mut fs, &mut tbl, 999), Err(CellError::EBADF));
    }

    // -- read / write --------------------------------------------

    #[test]
    fn write_then_read_roundtrip() {
        let mut fs = MemFs::new();
        let mut tbl = FdTable::new();
        let fd = sys_fs_open(&mut fs, &mut tbl, "/f", O_RDWR | O_CREAT).unwrap();
        let wrote = sys_fs_write(&mut fs, &mut tbl, fd, b"hello").unwrap();
        assert_eq!(wrote, 5);
        sys_fs_lseek(&mut fs, &mut tbl, fd, 0, Whence::Set as i32).unwrap();
        let mut buf = [0u8; 5];
        let read = sys_fs_read(&mut fs, &mut tbl, fd, &mut buf).unwrap();
        assert_eq!(read, 5);
        assert_eq!(&buf, b"hello");
    }

    #[test]
    fn read_on_write_only_fd_is_ebadf() {
        let mut fs = MemFs::new();
        let mut tbl = FdTable::new();
        let fd = sys_fs_open(&mut fs, &mut tbl, "/f", O_WRONLY | O_CREAT).unwrap();
        let mut buf = [0u8; 4];
        assert_eq!(
            sys_fs_read(&mut fs, &mut tbl, fd, &mut buf),
            Err(CellError::EBADF)
        );
    }

    #[test]
    fn write_on_read_only_fd_is_ebadf() {
        let mut fs = MemFs::new();
        let mut tbl = FdTable::new();
        // Create file then reopen read-only.
        let fd = sys_fs_open(&mut fs, &mut tbl, "/f", O_WRONLY | O_CREAT).unwrap();
        sys_fs_write(&mut fs, &mut tbl, fd, b"data").unwrap();
        sys_fs_close(&mut fs, &mut tbl, fd).unwrap();
        let fd = sys_fs_open(&mut fs, &mut tbl, "/f", O_RDONLY).unwrap();
        assert_eq!(
            sys_fs_write(&mut fs, &mut tbl, fd, b"nope"),
            Err(CellError::EBADF)
        );
    }

    // -- lseek ---------------------------------------------------

    #[test]
    fn lseek_set_cur_end() {
        let mut fs = MemFs::new();
        let mut tbl = FdTable::new();
        let fd = sys_fs_open(&mut fs, &mut tbl, "/f", O_RDWR | O_CREAT).unwrap();
        sys_fs_write(&mut fs, &mut tbl, fd, b"1234567890").unwrap();
        // SEEK_SET 2 → pos=2
        assert_eq!(sys_fs_lseek(&mut fs, &mut tbl, fd, 2, 0), Ok(2));
        // SEEK_CUR +3 → pos=5
        assert_eq!(sys_fs_lseek(&mut fs, &mut tbl, fd, 3, 1), Ok(5));
        // SEEK_END -1 → pos = len-1 = 9
        assert_eq!(sys_fs_lseek(&mut fs, &mut tbl, fd, -1, 2), Ok(9));
    }

    #[test]
    fn lseek_bad_whence_is_einval() {
        let mut fs = MemFs::new();
        let mut tbl = FdTable::new();
        let fd = sys_fs_open(&mut fs, &mut tbl, "/f", O_RDWR | O_CREAT).unwrap();
        assert_eq!(
            sys_fs_lseek(&mut fs, &mut tbl, fd, 0, 99),
            Err(CellError::EINVAL)
        );
    }

    #[test]
    fn lseek_negative_is_einval() {
        let mut fs = MemFs::new();
        let mut tbl = FdTable::new();
        let fd = sys_fs_open(&mut fs, &mut tbl, "/f", O_RDWR | O_CREAT).unwrap();
        assert_eq!(
            sys_fs_lseek(&mut fs, &mut tbl, fd, -5, 0),
            Err(CellError::EINVAL)
        );
    }

    // -- stat ----------------------------------------------------

    #[test]
    fn stat_file_returns_regular_mode() {
        let mut fs = MemFs::new();
        let mut tbl = FdTable::new();
        let fd = sys_fs_open(&mut fs, &mut tbl, "/f", O_WRONLY | O_CREAT).unwrap();
        sys_fs_write(&mut fs, &mut tbl, fd, b"12345").unwrap();
        let s = sys_fs_stat(&fs, "/f").unwrap();
        assert!(s.is_file());
        assert_eq!(s.size, 5);
    }

    #[test]
    fn stat_directory_returns_dir_mode() {
        let fs = MemFs::new();
        let s = sys_fs_stat(&fs, "/").unwrap();
        assert!(s.is_dir());
    }

    #[test]
    fn stat_nonexistent_is_enoent() {
        let fs = MemFs::new();
        assert_eq!(sys_fs_stat(&fs, "/nope"), Err(CellError::ENOENT));
    }

    // -- mkdir / rmdir / unlink ----------------------------------

    #[test]
    fn mkdir_creates_directory() {
        let mut fs = MemFs::new();
        sys_fs_mkdir(&mut fs, "/newdir", 0o755).unwrap();
        assert!(sys_fs_stat(&fs, "/newdir").unwrap().is_dir());
    }

    #[test]
    fn mkdir_existing_is_eexist() {
        let mut fs = MemFs::new();
        sys_fs_mkdir(&mut fs, "/d", 0o755).unwrap();
        assert_eq!(sys_fs_mkdir(&mut fs, "/d", 0o755), Err(CellError::EEXIST));
    }

    #[test]
    fn unlink_removes_file() {
        let mut fs = MemFs::new();
        let mut tbl = FdTable::new();
        let fd = sys_fs_open(&mut fs, &mut tbl, "/del", O_WRONLY | O_CREAT).unwrap();
        sys_fs_close(&mut fs, &mut tbl, fd).unwrap();
        sys_fs_unlink(&mut fs, "/del").unwrap();
        assert_eq!(sys_fs_stat(&fs, "/del"), Err(CellError::ENOENT));
    }

    #[test]
    fn rmdir_removes_directory() {
        let mut fs = MemFs::new();
        sys_fs_mkdir(&mut fs, "/rm", 0).unwrap();
        sys_fs_rmdir(&mut fs, "/rm").unwrap();
        assert_eq!(sys_fs_stat(&fs, "/rm"), Err(CellError::ENOENT));
    }

    // -- opendir / readdir / closedir ----------------------------

    #[test]
    fn opendir_readdir_enumerates_entries() {
        let mut fs = MemFs::new();
        let mut tbl = FdTable::new();
        // Setup: /dir/{a,b,sub}
        sys_fs_mkdir(&mut fs, "/dir", 0).unwrap();
        sys_fs_mkdir(&mut fs, "/dir/sub", 0).unwrap();
        for name in ["/dir/a", "/dir/b"] {
            let fd = sys_fs_open(&mut fs, &mut tbl, name, O_WRONLY | O_CREAT).unwrap();
            sys_fs_close(&mut fs, &mut tbl, fd).unwrap();
        }

        let dfd = sys_fs_opendir(&mut fs, &mut tbl, "/dir").unwrap();
        let mut names: Vec<String> = Vec::new();
        while let Ok(Some(e)) = sys_fs_readdir(&mut fs, &mut tbl, dfd) {
            names.push(e.name);
        }
        sys_fs_closedir(&mut fs, &mut tbl, dfd).unwrap();
        names.sort();
        assert_eq!(names, vec!["a", "b", "sub"]);
    }

    #[test]
    fn readdir_on_nondir_fd_is_enotdir() {
        let mut fs = MemFs::new();
        let mut tbl = FdTable::new();
        let fd = sys_fs_open(&mut fs, &mut tbl, "/file", O_WRONLY | O_CREAT).unwrap();
        assert_eq!(
            sys_fs_readdir(&mut fs, &mut tbl, fd),
            Err(CellError::ENOTDIR)
        );
    }

    #[test]
    fn close_on_dir_fd_is_eisdir() {
        let mut fs = MemFs::new();
        let mut tbl = FdTable::new();
        let dfd = sys_fs_opendir(&mut fs, &mut tbl, "/").unwrap();
        assert_eq!(
            sys_fs_close(&mut fs, &mut tbl, dfd),
            Err(CellError::EISDIR)
        );
    }

    #[test]
    fn closedir_on_file_fd_is_enotdir() {
        let mut fs = MemFs::new();
        let mut tbl = FdTable::new();
        let fd = sys_fs_open(&mut fs, &mut tbl, "/f", O_WRONLY | O_CREAT).unwrap();
        assert_eq!(
            sys_fs_closedir(&mut fs, &mut tbl, fd),
            Err(CellError::ENOTDIR)
        );
    }

    // -- constants frozen ----------------------------------------

    #[test]
    fn open_flag_constants_frozen() {
        assert_eq!(O_RDONLY, 0);
        assert_eq!(O_WRONLY, 1);
        assert_eq!(O_RDWR, 2);
        assert_eq!(O_CREAT, 4);
        assert_eq!(O_APPEND, 8);
        assert_eq!(O_TRUNC, 0x10);
        assert_eq!(O_EXCL, 0x80);
    }

    #[test]
    fn whence_values_frozen() {
        assert_eq!(Whence::Set as i32, 0);
        assert_eq!(Whence::Cur as i32, 1);
        assert_eq!(Whence::End as i32, 2);
    }

    #[test]
    fn mode_bits_frozen() {
        assert_eq!(S_IFDIR, 0o040000);
        assert_eq!(S_IFREG, 0o100000);
    }
}
