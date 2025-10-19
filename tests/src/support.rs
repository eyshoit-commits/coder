use std::env;

use anyhow::Result;
use tempfile::{tempdir, TempDir};

pub fn temp_workspace() -> Result<TempDir> {
    Ok(tempdir()?)
}

pub fn system_path() -> String {
    env::var("PATH").unwrap_or_else(|_| {
        "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin".to_string()
    })
}
