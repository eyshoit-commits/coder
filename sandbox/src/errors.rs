use std::io;
use std::time::Duration;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SandboxError {
    #[error("path traversal detected")]
    PathTraversal,
    #[error("operation outside sandbox root")]
    OutsideRoot,
    #[error("file too large: {0} bytes exceeds limit")]
    FileTooLarge(u64),
    #[error("process execution timed out after {0:?}")]
    Timeout(Duration),
    #[error("process produced {stream} output exceeding limit of {limit} bytes")]
    OutputTooLarge { stream: &'static str, limit: usize },
    #[error("process terminated by signal")]
    TerminatedBySignal,
    #[error("io error: {0}")]
    Io(#[from] io::Error),
    #[error("invalid operation: {0}")]
    InvalidOperation(String),
    #[error("wasm trap: {0}")]
    WasmTrap(String),
    #[error("micro image '{0}' is not configured")]
    MicroImageNotConfigured(String),
    #[error("micro vm '{0}' not found")]
    MicroVmNotFound(String),
    #[error("agent '{0}' is not registered")]
    AgentUnavailable(String),
    #[error("agent task '{0}' not found")]
    AgentTaskNotFound(String),
    #[error("agent context size {provided} bytes exceeds limit {limit}")]
    ContextTooLarge { provided: usize, limit: usize },
    #[error("agent execution failed: {0}")]
    AgentFailed(String),
    #[error("network request failed: {0}")]
    Network(String),
    #[error("agent operation cancelled")]
    Cancelled,
}

pub type Result<T> = std::result::Result<T, SandboxError>;
