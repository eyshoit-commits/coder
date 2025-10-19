//! Filesystem sandbox module.
//!
//! Provides safe wrappers for working with project files inside a bounded
//! workspace directory. The helpers ensure that callers cannot escape the
//! configured root via `..` traversal or absolute paths and enforce a
//! conservative file size limit so runaway writes do not starve the host.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};

use thiserror::Error;

/// Environment variable that controls the sandbox root.
pub const WORKSPACE_ROOT_ENV: &str = "CYBERDEV_WORKSPACE_ROOT";

/// Default directory (relative to the current working directory) that will be
/// used when `CYBERDEV_WORKSPACE_ROOT` is not provided.
const DEFAULT_WORKSPACE_DIR: &str = "workspace";

/// Maximum number of bytes a single file operation may write.
const MAX_FILE_SIZE_BYTES: usize = 2 * 1024 * 1024; // 2 MiB

#[derive(Debug, Error, PartialEq, Eq)]
pub enum FsError {
    /// The caller attempted to provide an absolute path which is not allowed.
    #[error("absolute paths are not permitted inside the sandbox: {0}")]
    AbsolutePath(PathBuf),

    /// The caller attempted to escape the workspace root via `..` segments.
    #[error("path traversal outside the workspace root is not allowed")]
    TraversalAttempt,

    /// The workspace root could not be prepared as a directory.
    #[error("workspace root is not a directory: {0}")]
    WorkspaceRootInvalid(PathBuf),

    /// The requested file is larger than the configured limit.
    #[error("file exceeds maximum size of {limit} bytes (attempted {size} bytes)")]
    FileTooLarge { size: usize, limit: usize },

    /// Wrapper around [`std::io::Error`].
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// Writes content to a sandbox-managed file.
pub fn write_file<P, C>(path: P, contents: C) -> Result<(), FsError>
where
    P: AsRef<Path>,
    C: AsRef<[u8]>,
{
    let data = contents.as_ref();
    if data.len() > MAX_FILE_SIZE_BYTES {
        return Err(FsError::FileTooLarge {
            size: data.len(),
            limit: MAX_FILE_SIZE_BYTES,
        });
    }

    let target = resolve_workspace_path(path.as_ref())?;

    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&target)?;
    file.write_all(data)?;
    file.sync_all()?;

    Ok(())
}

/// Reads a file from the sandboxed workspace.
pub fn read_file<P: AsRef<Path>>(path: P) -> Result<String, FsError> {
    let target = resolve_workspace_path(path.as_ref())?;

    let metadata = fs::metadata(&target)?;
    if metadata.len() as usize > MAX_FILE_SIZE_BYTES {
        return Err(FsError::FileTooLarge {
            size: metadata.len() as usize,
            limit: MAX_FILE_SIZE_BYTES,
        });
    }

    Ok(std::fs::read_to_string(target)?)
}

pub(crate) fn resolve_workspace_path(path: &Path) -> Result<PathBuf, FsError> {
    let sanitized = sanitize_relative_path(path)?;
    let root = workspace_root()?;
    Ok(root.join(sanitized))
}

pub(crate) fn workspace_root() -> Result<PathBuf, FsError> {
    let env_root = std::env::var(WORKSPACE_ROOT_ENV).ok().map(PathBuf::from);

    let mut root = env_root.unwrap_or_else(|| PathBuf::from(DEFAULT_WORKSPACE_DIR));

    if root.is_relative() {
        root = std::env::current_dir()?.join(root);
    }

    if root.exists() {
        if !root.is_dir() {
            return Err(FsError::WorkspaceRootInvalid(root));
        }
    } else {
        fs::create_dir_all(&root)?;
    }

    Ok(root)
}

fn sanitize_relative_path(path: &Path) -> Result<PathBuf, FsError> {
    let mut sanitized = PathBuf::new();

    for component in path.components() {
        match component {
            Component::Prefix(_) | Component::RootDir => {
                return Err(FsError::AbsolutePath(path.to_path_buf()));
            }
            Component::CurDir => {}
            Component::ParentDir => {
                if !sanitized.pop() {
                    return Err(FsError::TraversalAttempt);
                }
            }
            Component::Normal(part) => sanitized.push(part),
        }
    }

    Ok(sanitized)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_temp_workspace<F: FnOnce()>(test: F) {
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        std::env::set_var(WORKSPACE_ROOT_ENV, temp_dir.path());
        test();
        std::env::remove_var(WORKSPACE_ROOT_ENV);
    }

    #[test]
    fn write_and_read_roundtrip() {
        with_temp_workspace(|| {
            write_file("nested/hello.txt", "cyberdev").expect("write succeeded");
            let contents = read_file("nested/hello.txt").expect("read succeeded");
            assert_eq!(contents, "cyberdev");
        });
    }

    #[test]
    fn rejects_absolute_paths() {
        with_temp_workspace(|| {
            let path = PathBuf::from("/etc/passwd");
            let err = write_file(&path, "nope").expect_err("should reject absolute path");
            assert!(matches!(err, FsError::AbsolutePath(p) if p == path));
        });
    }

    #[test]
    fn rejects_traversal() {
        with_temp_workspace(|| {
            let err = write_file("../escape.txt", "nope").expect_err("should reject traversal");
            assert!(matches!(err, FsError::TraversalAttempt));
        });
    }

    #[test]
    fn respects_size_limit() {
        with_temp_workspace(|| {
            let big = vec![0_u8; MAX_FILE_SIZE_BYTES + 1];
            let err = write_file("big.bin", &big).expect_err("should reject big file");
            assert!(matches!(err, FsError::FileTooLarge { .. }));
        });
    }

    #[test]
    fn workspace_root_must_be_directory() {
        let file = tempfile::NamedTempFile::new().expect("temp file");
        std::env::set_var(WORKSPACE_ROOT_ENV, file.path());
        let result = write_file("foo.txt", "bar");
        std::env::remove_var(WORKSPACE_ROOT_ENV);
        assert!(matches!(result, Err(FsError::WorkspaceRootInvalid(_))));
    }
}
