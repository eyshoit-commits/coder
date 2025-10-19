//! Process execution sandbox utilities.
//!
//! Provides a restrictive wrapper around spawning commands from the workspace
//! with sane defaults: an allow-listed set of binaries, trimmed environment,
//! bounded stdout/stderr collection, and a hard execution timeout.

use std::fmt;
use std::fs;
use std::io::{self, Read};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use thiserror::Error;

use crate::fs::{workspace_root, FsError};

/// Maximum amount of time a command is permitted to run.
const EXECUTION_TIMEOUT: Duration = Duration::from_secs(5);

/// Maximum number of bytes captured from stdout/stderr.
const MAX_OUTPUT_BYTES: usize = 512 * 1024; // 512 KiB

/// Minimal search path for spawned commands.
const DEFAULT_PATH: &str = "/usr/local/bin:/usr/bin:/bin";

/// Commands that may be executed by the sandbox.
const ALLOWED_COMMANDS: &[&str] = &[
    "sh",
    "/bin/sh",
    "bash",
    "/bin/bash",
    "python3",
    "/usr/bin/python3",
    "node",
    "/usr/bin/node",
    "deno",
    "/usr/bin/deno",
    "cargo",
    "/usr/bin/cargo",
    "npm",
    "/usr/bin/npm",
    "pnpm",
    "/usr/bin/pnpm",
    "yarn",
    "/usr/bin/yarn",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutputStream {
    Stdout,
    Stderr,
}

impl fmt::Display for OutputStream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OutputStream::Stdout => write!(f, "stdout"),
            OutputStream::Stderr => write!(f, "stderr"),
        }
    }
}

/// Errors that may arise while executing sandboxed commands.
#[derive(Debug, Error)]
pub enum RunError {
    #[error("command `{0}` is not permitted inside the sandbox")]
    CommandNotAllowed(String),

    #[error("execution timed out after {0:?}")]
    Timeout(Duration),

    #[error("output on {stream} exceeded limit of {limit} bytes")]
    OutputLimit { stream: OutputStream, limit: usize },

    #[error("child process did not expose {stream} stream")]
    MissingStream { stream: OutputStream },

    #[error("output reader thread panicked")]
    ReaderThread,

    #[error(transparent)]
    Workspace(#[from] FsError),

    #[error(transparent)]
    Io(#[from] io::Error),
}

/// Result of a sandboxed process execution.
#[derive(Debug)]
pub struct ExecutionResult {
    /// Exit status returned by the child process.
    pub status: ExitStatus,
    /// Captured stdout (truncated to [`MAX_OUTPUT_BYTES`]).
    pub stdout: String,
    /// Captured stderr (truncated to [`MAX_OUTPUT_BYTES`]).
    pub stderr: String,
}

/// Executes a command inside the sandbox and captures its output.
pub fn execute(command: &str, args: &[String]) -> Result<ExecutionResult, RunError> {
    if !is_command_allowed(command) {
        return Err(RunError::CommandNotAllowed(command.to_string()));
    }

    let workspace = workspace_root()?;
    let home_dir = workspace.join(".sandbox_home");
    fs::create_dir_all(&home_dir)?;

    let mut cmd = Command::new(command);
    cmd.args(args)
        .current_dir(&workspace)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    cmd.env_clear();
    cmd.env("PATH", DEFAULT_PATH);
    cmd.env("HOME", &home_dir);
    cmd.env(crate::fs::WORKSPACE_ROOT_ENV, &workspace);

    let mut child = cmd.spawn()?;

    let stdout = child.stdout.take().ok_or(RunError::MissingStream {
        stream: OutputStream::Stdout,
    })?;
    let stderr = child.stderr.take().ok_or(RunError::MissingStream {
        stream: OutputStream::Stderr,
    })?;

    let stdout_handle = spawn_reader(stdout, OutputStream::Stdout);
    let stderr_handle = spawn_reader(stderr, OutputStream::Stderr);

    let status = match wait_with_timeout(&mut child) {
        Ok(status) => status,
        Err(err) => {
            // ensure reader threads finish before returning the timeout.
            let _ = join_reader(stdout_handle);
            let _ = join_reader(stderr_handle);
            return Err(err);
        }
    };

    let stdout_bytes = join_reader(stdout_handle)?;
    let stderr_bytes = join_reader(stderr_handle)?;

    Ok(ExecutionResult {
        status,
        stdout: String::from_utf8_lossy(&stdout_bytes).to_string(),
        stderr: String::from_utf8_lossy(&stderr_bytes).to_string(),
    })
}

fn spawn_reader<R>(reader: R, stream: OutputStream) -> thread::JoinHandle<Result<Vec<u8>, RunError>>
where
    R: Read + Send + 'static,
{
    thread::spawn(move || read_stream(reader, stream))
}

fn read_stream<R: Read>(mut reader: R, stream: OutputStream) -> Result<Vec<u8>, RunError> {
    let mut buf = Vec::new();
    let mut chunk = [0_u8; 8192];

    loop {
        let read = reader.read(&mut chunk)?;
        if read == 0 {
            break;
        }

        if buf.len() + read > MAX_OUTPUT_BYTES {
            return Err(RunError::OutputLimit {
                stream,
                limit: MAX_OUTPUT_BYTES,
            });
        }

        buf.extend_from_slice(&chunk[..read]);
    }

    Ok(buf)
}

fn join_reader(handle: thread::JoinHandle<Result<Vec<u8>, RunError>>) -> Result<Vec<u8>, RunError> {
    match handle.join() {
        Ok(result) => result,
        Err(_) => Err(RunError::ReaderThread),
    }
}

fn wait_with_timeout(child: &mut Child) -> Result<ExitStatus, RunError> {
    let start = Instant::now();

    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }

        if start.elapsed() >= EXECUTION_TIMEOUT {
            child.kill().ok();
            let _ = child.wait();
            return Err(RunError::Timeout(EXECUTION_TIMEOUT));
        }

        thread::sleep(Duration::from_millis(25));
    }
}

fn is_command_allowed(command: &str) -> bool {
    ALLOWED_COMMANDS.iter().any(|allowed| *allowed == command)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::fs::WORKSPACE_ROOT_ENV;

    fn with_temp_workspace<F: FnOnce()>(test: F) {
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        std::env::set_var(WORKSPACE_ROOT_ENV, temp_dir.path());
        test();
        std::env::remove_var(WORKSPACE_ROOT_ENV);
    }

    #[test]
    fn executes_allowed_command() {
        with_temp_workspace(|| {
            let result = execute("sh", &["-c".to_string(), "printf cyberdev".to_string()])
                .expect("execution succeeds");

            assert!(result.status.success());
            assert_eq!(result.stdout, "cyberdev");
            assert_eq!(result.stderr, "");
        });
    }

    #[test]
    fn rejects_disallowed_command() {
        with_temp_workspace(|| {
            let err = execute("rm", &[]).expect_err("rm should be disallowed");
            assert!(matches!(err, RunError::CommandNotAllowed(cmd) if cmd == "rm"));
        });
    }

    #[test]
    fn terminates_on_timeout() {
        with_temp_workspace(|| {
            let err = execute("sh", &["-c".to_string(), "sleep 10".to_string()])
                .expect_err("execution should time out");
            assert!(matches!(err, RunError::Timeout(_)));
        });
    }
}
