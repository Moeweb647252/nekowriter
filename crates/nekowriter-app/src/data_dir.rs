//! The data directory: where the database and logs live, and the lock that keeps a second
//! process out of it.

use std::{
    fs::{self, File, TryLockError},
    io,
    path::{Path, PathBuf},
};

use thiserror::Error;

/// The name of the lock file inside the data directory.
const LOCK_FILE: &str = "nekowriter.lock";

#[derive(Debug, Error)]
pub enum DataDirError {
    #[error(
        "No application-data directory is known for this system. Start NekoWriter with --data-dir <path>."
    )]
    NoDefault,
    #[error(
        "The data directory {path} could not be prepared: {source}. Start NekoWriter with a different --data-dir."
    )]
    Prepare { path: PathBuf, source: io::Error },
    #[error(
        "Another NekoWriter process is using the data directory {path}. Close it, or start NekoWriter with a different --data-dir."
    )]
    InUse { path: PathBuf },
}

/// The default data directory: `NekoWriter` inside the operating system's local
/// application-data directory.
pub fn default_data_dir() -> Result<PathBuf, DataDirError> {
    directories::BaseDirs::new()
        .map(|dirs| dirs.data_local_dir().join("NekoWriter"))
        .ok_or(DataDirError::NoDefault)
}

/// Creates `path` if needed and returns its absolute form.
pub fn prepare(path: &Path) -> Result<PathBuf, DataDirError> {
    let failed = |source| DataDirError::Prepare {
        path: path.to_owned(),
        source,
    };
    fs::create_dir_all(path).map_err(failed)?;
    fs::canonicalize(path).map_err(failed)
}

/// An exclusive lock on a data directory, held until dropped.
#[derive(Debug)]
pub struct DataDirLock {
    _file: File,
}

impl DataDirLock {
    pub fn acquire(data_dir: &Path) -> Result<Self, DataDirError> {
        let path = data_dir.join(LOCK_FILE);
        let file = File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .map_err(|source| DataDirError::Prepare {
                path: data_dir.to_owned(),
                source,
            })?;
        match file.try_lock() {
            Ok(()) => Ok(Self { _file: file }),
            Err(TryLockError::WouldBlock) => Err(DataDirError::InUse {
                path: data_dir.to_owned(),
            }),
            Err(TryLockError::Error(source)) => Err(DataDirError::Prepare {
                path: data_dir.to_owned(),
                source,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_locked_data_directory_cannot_be_locked_again() {
        let dir = tempfile::tempdir().unwrap();
        let first = DataDirLock::acquire(dir.path()).unwrap();

        assert!(matches!(
            DataDirLock::acquire(dir.path()),
            Err(DataDirError::InUse { .. })
        ));

        drop(first);
        assert!(DataDirLock::acquire(dir.path()).is_ok());
    }
}
