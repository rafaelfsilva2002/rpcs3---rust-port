//! In-memory VFS backend for `EmuCore`.
//!
//! A deterministic, RAM-backed [`rpcs3_lv2_fs::FileSystem`]: guest paths map
//! directly to byte content. Tests pre-seed files via `EmuCore::vfs_add_file`
//! BEFORE `run_self` (mirroring the `sysutil_queue` pre-seed pattern), so a
//! homebrew that opens + reads a known file yields a deterministic result — the
//! behavior-freeze oracle for the lv2 fs syscalls (sys_fs_open/read/close/...).
//!
//! Lifted near-verbatim from the proven `#[cfg(test)] MemFs` reference impl in
//! `rpcs3-lv2-fs` (which is not compiled into that crate's rlib), keyed directly
//! by the raw guest path string (no mount-point / host-disk resolution — that is
//! a future on-disk backend behind the same trait).

use std::collections::{HashMap, HashSet};

use rpcs3_emu_types::CellError;
use rpcs3_lv2_fs::{
    CellFsStat, DirEntry, DirObject, FileObject, FileSystem, Whence, FS_TYPE_DIRECTORY,
    FS_TYPE_REGULAR, O_CREAT, O_EXCL, O_TRUNC, S_IFDIR, S_IFREG,
};

/// In-memory filesystem: `path -> bytes`, with a per-open read/write cursor.
#[derive(Debug, Default)]
pub struct MemVfs {
    files: HashMap<String, Vec<u8>>,
    dirs: HashSet<String>,
    next_handle: u64,
    /// open handle -> (path, cursor position)
    open_files: HashMap<u64, (String, u64)>,
    open_dirs: HashMap<u64, (Vec<DirEntry>, usize)>,
}

impl MemVfs {
    #[must_use]
    pub fn new() -> Self {
        let mut fs = Self::default();
        fs.dirs.insert("/".into());
        fs.next_handle = 1;
        fs
    }

    /// Seed a file at `path` with `data`, auto-creating ancestor directories so
    /// `stat`/`opendir` of the parents succeed. Used by tests before `run_self`;
    /// the key MUST be byte-identical to the path the guest passes to open.
    pub fn add_file(&mut self, path: &str, data: Vec<u8>) {
        let comps: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        if comps.len() > 1 {
            let mut acc = String::new();
            for comp in &comps[..comps.len() - 1] {
                acc.push('/');
                acc.push_str(comp);
                self.dirs.insert(acc.clone());
            }
        }
        self.files.insert(path.to_owned(), data);
    }

    /// Seed an (empty) directory at `path`, auto-creating ancestor dirs. Lets a
    /// test set up the parent directory an `O_CREAT` open will create into.
    pub fn add_dir(&mut self, path: &str) {
        let mut acc = String::new();
        for comp in path.split('/').filter(|s| !s.is_empty()) {
            acc.push('/');
            acc.push_str(comp);
            self.dirs.insert(acc.clone());
        }
    }

    fn ensure_parent(&self, path: &str) -> Result<(), CellError> {
        let parent = path
            .rsplit_once('/')
            .map_or("/", |(p, _)| if p.is_empty() { "/" } else { p });
        if !self.dirs.contains(parent) {
            return Err(CellError::ENOENT);
        }
        Ok(())
    }

    /// Stat an open file by its `FileObject` handle — the `fstat` path. The lv2
    /// `sys_fs_fstat` is generic and has no path; resolve the handle to its
    /// source path here (kept in the open-file table) and stat that.
    pub fn stat_handle(&self, handle: u64) -> Result<CellFsStat, CellError> {
        let path = self
            .open_files
            .get(&handle)
            .map(|(p, _)| p.clone())
            .ok_or(CellError::EBADF)?;
        self.stat(&path)
    }
}

impl FileSystem for MemVfs {
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
            self.files.insert(path.to_owned(), Vec::new());
        } else if excl {
            return Err(CellError::EEXIST);
        } else if trunc {
            if let Some(d) = self.files.get_mut(path) {
                d.clear();
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
        let data = self.files.get(path).ok_or(CellError::ENOENT)?;
        let available = (data.len() as u64).saturating_sub(*pos);
        let take = available.min(buf.len() as u64) as usize;
        let start = *pos as usize;
        buf[..take].copy_from_slice(&data[start..start + take]);
        *pos += take as u64;
        Ok(take as u64)
    }

    fn write(&mut self, obj: &mut FileObject, buf: &[u8]) -> Result<u64, CellError> {
        let (path, pos) = self.open_files.get_mut(&obj.handle).ok_or(CellError::EBADF)?;
        let data = self.files.get_mut(path).ok_or(CellError::ENOENT)?;
        let pos_u = *pos as usize;
        if pos_u + buf.len() > data.len() {
            data.resize(pos_u + buf.len(), 0);
        }
        data[pos_u..pos_u + buf.len()].copy_from_slice(buf);
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
        let data = self.files.get(path).ok_or(CellError::ENOENT)?;
        let new_pos: i64 = match whence {
            Whence::Set => offset,
            Whence::Cur => *pos as i64 + offset,
            Whence::End => data.len() as i64 + offset,
        };
        if new_pos < 0 {
            return Err(CellError::EINVAL);
        }
        *pos = new_pos as u64;
        Ok(*pos)
    }

    fn stat(&self, path: &str) -> Result<CellFsStat, CellError> {
        if let Some(data) = self.files.get(path) {
            Ok(CellFsStat {
                mode: S_IFREG | 0o644,
                size: data.len() as u64,
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
        let prefix = if path == "/" {
            String::from("/")
        } else {
            format!("{path}/")
        };
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
