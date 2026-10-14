use std::{
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

/// The application storage owner must atomically commit complete public snapshots.
/// A commit error must leave the previously committed snapshot readable.
pub trait RadioJournal: Send {
    fn load(&mut self) -> io::Result<Option<Vec<u8>>>;
    fn commit(&mut self, bytes: &[u8]) -> io::Result<()>;
}
/// Atomic same-directory replacement; only public correspondence is stored here.
/// The host chooses its application-owned path and permissions.
pub struct FileJournal {
    path: PathBuf,
    lock: Option<std::fs::File>,
}
impl FileJournal {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            lock: None,
        }
    }
}
impl RadioJournal for FileJournal {
    fn load(&mut self) -> io::Result<Option<Vec<u8>>> {
        let parent = self
            .path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        std::fs::create_dir_all(parent)?;
        let mut lock_path = self.path.as_os_str().to_os_string();
        lock_path.push(".lock");
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock_path)?;
        lock.try_lock()
            .map_err(|error| io::Error::other(error.to_string()))?;
        self.lock = Some(lock);
        let file = match std::fs::File::open(&self.path) {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        let mut bytes = Vec::new();
        file.take(16 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
        if bytes.len() > 16 * 1024 * 1024 {
            return Err(io::Error::other("radio journal exceeds read bound"));
        }
        Ok(Some(bytes))
    }
    fn commit(&mut self, bytes: &[u8]) -> io::Result<()> {
        if bytes.len() > 16 * 1024 * 1024 {
            return Err(io::Error::other("radio journal exceeds write bound"));
        }
        let parent = self
            .path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        std::fs::create_dir_all(parent)?;
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        file.write_all(bytes)?;
        file.as_file().sync_all()?;
        file.persist(&self.path).map_err(|e| e.error)?;
        // The rename has committed. A subsequent directory-sync failure cannot
        // be reported as an unchanged snapshot; expose it as best effort here.
        #[cfg(unix)]
        {
            if let Ok(directory) = std::fs::File::open(parent) {
                let _ = directory.sync_all();
            }
        }
        Ok(())
    }
}
