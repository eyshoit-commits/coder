use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::time::timeout;
use tracing::instrument;

use crate::errors::{Result, SandboxError};
use crate::path;

#[derive(Clone, Debug)]
pub struct RunConfig {
    root: PathBuf,
    allowed_programs: HashSet<String>,
    env_allowlist: HashSet<String>,
    fixed_env: HashMap<String, String>,
    default_timeout: Duration,
    max_timeout: Duration,
    max_output_bytes: usize,
}

impl RunConfig {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        root: impl AsRef<Path>,
        allowed_programs: impl IntoIterator<Item = String>,
        env_allowlist: impl IntoIterator<Item = String>,
        fixed_env: impl IntoIterator<Item = (String, String)>,
        default_timeout: Duration,
        max_timeout: Duration,
        max_output_bytes: usize,
    ) -> Result<Self> {
        if max_output_bytes == 0 {
            return Err(SandboxError::InvalidOperation(
                "max_output_bytes must be greater than zero".to_string(),
            ));
        }
        if max_timeout < default_timeout {
            return Err(SandboxError::InvalidOperation(
                "max_timeout must be greater than or equal to default_timeout".to_string(),
            ));
        }

        let root = path::ensure_absolute_base(root.as_ref())?;
        fs::create_dir_all(&root)?;

        let allowed_programs: HashSet<String> = allowed_programs
            .into_iter()
            .map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty())
            .collect();
        if allowed_programs.is_empty() {
            return Err(SandboxError::InvalidOperation(
                "no allowed programs configured for run sandbox".to_string(),
            ));
        }
        let env_allowlist: HashSet<String> = env_allowlist
            .into_iter()
            .map(|k| k.trim().to_string())
            .filter(|k| !k.is_empty())
            .collect();
        let fixed_env: HashMap<String, String> =
            fixed_env.into_iter().map(|(k, v)| (k, v)).collect();

        Ok(Self {
            root,
            allowed_programs,
            env_allowlist,
            fixed_env,
            default_timeout,
            max_timeout,
            max_output_bytes,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn allowed_programs(&self) -> impl Iterator<Item = &String> {
        self.allowed_programs.iter()
    }

    pub fn default_timeout(&self) -> Duration {
        self.default_timeout
    }

    pub fn max_timeout(&self) -> Duration {
        self.max_timeout
    }

    pub fn max_output_bytes(&self) -> usize {
        self.max_output_bytes
    }

    fn is_program_allowed(&self, program: &str) -> bool {
        self.allowed_programs.contains(program)
    }

    fn is_env_allowed(&self, key: &str) -> bool {
        self.env_allowlist.contains(key)
    }
}

#[derive(Clone, Debug)]
pub struct SandboxRun {
    config: RunConfig,
}

impl SandboxRun {
    pub fn new(config: RunConfig) -> Self {
        Self { config }
    }

    pub fn config(&self) -> &RunConfig {
        &self.config
    }

    #[instrument(skip(self, request), fields(program = %request.program))]
    pub async fn execute(&self, request: RunRequest) -> Result<RunOutput> {
        self.execute_inner(request).await
    }

    async fn execute_inner(&self, request: RunRequest) -> Result<RunOutput> {
        let RunRequest {
            program,
            args,
            stdin,
            env,
            working_dir,
            timeout,
        } = request;

        if !self.config.is_program_allowed(&program) {
            return Err(SandboxError::InvalidOperation(format!(
                "program '{}' is not permitted in sandbox",
                program
            )));
        }

        let working_dir = match &working_dir {
            Some(dir) => {
                let resolved = path::resolve(self.config.root(), dir)?;
                if !resolved.exists() {
                    return Err(SandboxError::InvalidOperation(format!(
                        "working directory '{}' does not exist",
                        dir
                    )));
                }
                if !resolved.is_dir() {
                    return Err(SandboxError::InvalidOperation(format!(
                        "working directory '{}' is not a directory",
                        dir
                    )));
                }
                resolved
            }
            None => self.config.root().to_path_buf(),
        };

        let timeout_duration = timeout.unwrap_or_else(|| self.config.default_timeout());
        if timeout_duration.is_zero() {
            return Err(SandboxError::InvalidOperation(
                "timeout must be greater than zero".to_string(),
            ));
        }
        if timeout_duration > self.config.max_timeout() {
            return Err(SandboxError::InvalidOperation(format!(
                "requested timeout {:?} exceeds maximum {:?}",
                timeout_duration,
                self.config.max_timeout()
            )));
        }

        let mut command = Command::new(&program);
        command.current_dir(working_dir);
        command.kill_on_drop(true);
        command.stdout(std::process::Stdio::piped());
        command.stderr(std::process::Stdio::piped());
        if stdin.is_some() {
            command.stdin(std::process::Stdio::piped());
        } else {
            command.stdin(std::process::Stdio::null());
        }
        command.env_clear();
        for (key, value) in &self.config.fixed_env {
            command.env(key, value);
        }
        for (key, value) in env {
            if !self.config.is_env_allowed(&key) {
                return Err(SandboxError::InvalidOperation(format!(
                    "environment variable '{}' is not permitted",
                    key
                )));
            }
            command.env(key, value);
        }
        for arg in args {
            command.arg(arg);
        }

        let mut child = command.spawn()?;

        if let Some(stdin) = stdin {
            if let Some(mut handle) = child.stdin.take() {
                handle.write_all(&stdin).await?;
            }
        }

        let start = Instant::now();
        let output = match timeout(timeout_duration, child.wait_with_output()).await {
            Ok(result) => result?,
            Err(_) => return Err(SandboxError::Timeout(timeout_duration)),
        };
        let duration = start.elapsed();

        if output.stdout.len() > self.config.max_output_bytes() {
            return Err(SandboxError::OutputTooLarge {
                stream: "stdout",
                limit: self.config.max_output_bytes(),
            });
        }
        if output.stderr.len() > self.config.max_output_bytes() {
            return Err(SandboxError::OutputTooLarge {
                stream: "stderr",
                limit: self.config.max_output_bytes(),
            });
        }

        let exit_code = match output.status.code() {
            Some(code) => code,
            None => return Err(SandboxError::TerminatedBySignal),
        };

        Ok(RunOutput {
            exit_code,
            stdout: output.stdout,
            stderr: output.stderr,
            duration,
        })
    }
}

#[derive(Debug)]
pub struct RunRequest {
    pub program: String,
    pub args: Vec<String>,
    pub stdin: Option<Vec<u8>>,
    pub env: Vec<(String, String)>,
    pub working_dir: Option<String>,
    pub timeout: Option<Duration>,
}

impl RunRequest {
    pub fn new(program: impl Into<String>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            stdin: None,
            env: Vec::new(),
            working_dir: None,
            timeout: None,
        }
    }

    pub fn with_args(mut self, args: Vec<String>) -> Self {
        self.args = args;
        self
    }

    pub fn with_stdin(mut self, stdin: Vec<u8>) -> Self {
        self.stdin = Some(stdin);
        self
    }

    pub fn with_env(mut self, env: Vec<(String, String)>) -> Self {
        self.env = env;
        self
    }

    pub fn with_working_dir(mut self, dir: impl Into<String>) -> Self {
        self.working_dir = Some(dir.into());
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }
}

#[derive(Debug)]
pub struct RunOutput {
    pub exit_code: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub duration: Duration,
}
