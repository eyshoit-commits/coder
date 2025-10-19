use std::path::{Component, Path, PathBuf};

use crate::errors::{Result, SandboxError};

pub fn ensure_absolute_base(base_dir: &Path) -> Result<PathBuf> {
    if base_dir.is_relative() {
        return Err(SandboxError::InvalidOperation(
            "sandbox base directory must be absolute".to_string(),
        ));
    }
    Ok(base_dir.to_path_buf())
}

pub fn resolve(base_dir: &Path, relative: impl AsRef<Path>) -> Result<PathBuf> {
    let relative = relative.as_ref();
    if relative.components().count() == 0 {
        return Err(SandboxError::InvalidOperation(
            "path must not be empty".to_string(),
        ));
    }
    if relative.is_absolute() {
        return Err(SandboxError::OutsideRoot);
    }

    let mut clean = PathBuf::new();
    for component in relative.components() {
        match component {
            Component::CurDir => continue,
            Component::Normal(part) => clean.push(part),
            Component::ParentDir => return Err(SandboxError::PathTraversal),
            Component::RootDir | Component::Prefix(_) => return Err(SandboxError::OutsideRoot),
        }
    }

    let resolved = base_dir.join(clean);
    if !resolved.starts_with(base_dir) {
        return Err(SandboxError::OutsideRoot);
    }
    Ok(resolved)
}
