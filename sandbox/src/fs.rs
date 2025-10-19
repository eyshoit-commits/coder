use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::Serialize;
use tracing::instrument;

use crate::errors::{Result, SandboxError};
use crate::path;

#[derive(Clone, Debug)]
pub struct SandboxConfig {
    pub base_dir: PathBuf,
    pub max_file_size: u64,
}

impl SandboxConfig {
    pub fn new(base_dir: impl AsRef<Path>, max_file_size: u64) -> Result<Self> {
        let base = path::ensure_absolute_base(base_dir.as_ref())?;
        fs::create_dir_all(&base)?;
        Ok(Self {
            base_dir: base,
            max_file_size,
        })
    }
}

#[derive(Clone, Debug)]
pub struct SandboxFs {
    config: SandboxConfig,
}

impl SandboxFs {
    pub fn new(config: SandboxConfig) -> Self {
        Self { config }
    }

    pub fn base_dir(&self) -> &Path {
        &self.config.base_dir
    }

    fn resolve_path(&self, relative: impl AsRef<Path>) -> Result<PathBuf> {
        path::resolve(&self.config.base_dir, relative)
    }

    #[instrument(skip(self), fields(path = %relative.as_ref().display()))]
    pub fn read(&self, relative: impl AsRef<Path>) -> Result<Vec<u8>> {
        let path = self.resolve_path(relative)?;
        let metadata = fs::metadata(&path)?;
        if metadata.len() > self.config.max_file_size {
            return Err(SandboxError::FileTooLarge(metadata.len()));
        }
        let mut file = fs::File::open(path)?;
        let mut buffer = Vec::with_capacity(metadata.len() as usize);
        file.read_to_end(&mut buffer)?;
        Ok(buffer)
    }

    #[instrument(skip(self, bytes), fields(path = %relative.as_ref().display(), size = bytes.as_ref().len()))]
    pub fn write(&self, relative: impl AsRef<Path>, bytes: impl AsRef<[u8]>) -> Result<()> {
        let path = self.resolve_path(relative)?;
        let data = bytes.as_ref();
        let size = data.len() as u64;
        if size > self.config.max_file_size {
            return Err(SandboxError::FileTooLarge(size));
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, data)?;
        Ok(())
    }

    #[instrument(skip(self))]
    pub fn delete(&self, relative: impl AsRef<Path>) -> Result<()> {
        let path = self.resolve_path(relative)?;
        if path.is_dir() {
            fs::remove_dir_all(path)?;
        } else if path.exists() {
            fs::remove_file(path)?;
        }
        Ok(())
    }

    #[instrument(skip(self))]
    pub fn mkdir(&self, relative: impl AsRef<Path>) -> Result<()> {
        let path = self.resolve_path(relative)?;
        fs::create_dir_all(path)?;
        Ok(())
    }

    #[instrument(skip(self))]
    pub fn copy(&self, source: impl AsRef<Path>, target: impl AsRef<Path>) -> Result<()> {
        let from = self.resolve_path(source)?;
        let to = self.resolve_path(target)?;
        if from.is_dir() {
            return Err(SandboxError::InvalidOperation(
                "copying directories is not supported".to_string(),
            ));
        }
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(from, to)?;
        Ok(())
    }

    #[instrument(skip(self))]
    pub fn move_path(&self, source: impl AsRef<Path>, target: impl AsRef<Path>) -> Result<()> {
        let from = self.resolve_path(source)?;
        let to = self.resolve_path(target)?;
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::rename(from, to)?;
        Ok(())
    }

    #[instrument(skip(self))]
    pub fn list(&self, relative: impl AsRef<Path>) -> Result<Vec<FileEntry>> {
        let path = self.resolve_path(relative)?;
        let mut entries = Vec::new();
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let metadata = entry.metadata()?;
            entries.push(FileEntry {
                name: entry.file_name().into_string().map_err(|_| {
                    SandboxError::InvalidOperation("invalid utf8 filename".to_string())
                })?,
                is_dir: metadata.is_dir(),
                size: metadata.len(),
            });
        }
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(entries)
    }
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct FileEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
}
