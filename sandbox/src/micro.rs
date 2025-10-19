use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::time::timeout;
use uuid::Uuid;

use crate::errors::{Result, SandboxError};
use crate::path;

#[derive(Clone, Debug)]
pub struct MicroImage {
    name: String,
    command: String,
    args: Vec<String>,
    extension: String,
    env: HashMap<String, String>,
}

impl MicroImage {
    pub fn new(
        name: impl Into<String>,
        command: impl Into<String>,
        args: impl IntoIterator<Item = String>,
        extension: impl Into<String>,
        env: impl IntoIterator<Item = (String, String)>,
    ) -> Result<Self> {
        let name = name.into().trim().to_string();
        if name.is_empty() {
            return Err(SandboxError::InvalidOperation(
                "micro image name must not be empty".to_string(),
            ));
        }
        let command = command.into().trim().to_string();
        if command.is_empty() {
            return Err(SandboxError::InvalidOperation(format!(
                "micro image '{name}' command must not be empty"
            )));
        }
        let extension = extension.into().trim().trim_start_matches('.').to_string();
        if extension.is_empty() {
            return Err(SandboxError::InvalidOperation(format!(
                "micro image '{name}' extension must not be empty"
            )));
        }
        let args = args
            .into_iter()
            .map(|arg| arg.trim().to_string())
            .filter(|arg| !arg.is_empty())
            .collect::<Vec<_>>();
        let env = env
            .into_iter()
            .map(|(key, value)| (key.trim().to_string(), value))
            .filter(|(key, _)| !key.is_empty())
            .collect::<HashMap<_, _>>();

        Ok(Self {
            name,
            command,
            args,
            extension,
            env,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn command(&self) -> &str {
        &self.command
    }

    pub fn args(&self) -> impl Iterator<Item = &String> {
        self.args.iter()
    }

    pub fn extension(&self) -> &str {
        &self.extension
    }

    pub fn env(&self) -> impl Iterator<Item = (&String, &String)> {
        self.env.iter()
    }
}

#[derive(Clone, Debug)]
pub struct MicroConfig {
    root: PathBuf,
    images: HashMap<String, MicroImage>,
    default_timeout: Duration,
    max_timeout: Duration,
    max_output_bytes: usize,
    base_env: HashMap<String, String>,
}

impl MicroConfig {
    pub fn new(
        root: impl AsRef<Path>,
        images: impl IntoIterator<Item = MicroImage>,
        default_timeout: Duration,
        max_timeout: Duration,
        max_output_bytes: usize,
        base_env: impl IntoIterator<Item = (String, String)>,
    ) -> Result<Self> {
        if max_output_bytes == 0 {
            return Err(SandboxError::InvalidOperation(
                "micro sandbox max_output_bytes must be greater than zero".to_string(),
            ));
        }
        if max_timeout < default_timeout {
            return Err(SandboxError::InvalidOperation(
                "micro sandbox max_timeout must be greater than or equal to default_timeout"
                    .to_string(),
            ));
        }
        let root = path::ensure_absolute_base(root.as_ref())?;
        std::fs::create_dir_all(&root)?;

        let mut images_map = HashMap::new();
        for image in images {
            let name = image.name().to_string();
            if images_map.insert(name.clone(), image).is_some() {
                return Err(SandboxError::InvalidOperation(
                    "duplicate micro image names are not permitted".to_string(),
                ));
            }
        }
        if images_map.is_empty() {
            return Err(SandboxError::InvalidOperation(
                "no micro images configured".to_string(),
            ));
        }

        let base_env = base_env
            .into_iter()
            .map(|(k, v)| (k.trim().to_string(), v))
            .filter(|(k, _)| !k.is_empty())
            .collect::<HashMap<_, _>>();

        Ok(Self {
            root,
            images: images_map,
            default_timeout,
            max_timeout,
            max_output_bytes,
            base_env,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
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

    pub fn images(&self) -> impl Iterator<Item = &MicroImage> {
        self.images.values()
    }

    pub fn image(&self, name: &str) -> Option<&MicroImage> {
        self.images.get(name)
    }

    pub fn base_env(&self) -> &HashMap<String, String> {
        &self.base_env
    }
}

#[derive(Debug)]
pub struct SandboxMicro {
    config: MicroConfig,
    instances: Mutex<HashMap<Uuid, MicroVm>>,
}

impl SandboxMicro {
    pub fn new(config: MicroConfig) -> Self {
        Self {
            config,
            instances: Mutex::new(HashMap::new()),
        }
    }

    pub fn config(&self) -> &MicroConfig {
        &self.config
    }

    pub async fn start(&self, request: MicroStartRequest) -> Result<MicroInstance> {
        let image = self
            .config
            .image(&request.image)
            .cloned()
            .ok_or_else(|| SandboxError::MicroImageNotConfigured(request.image.clone()))?;

        let vm_id = Uuid::new_v4();
        let workdir = self.config.root().join(vm_id.to_string());
        fs::create_dir_all(&workdir).await?;

        if let Some(script) = request.init_script {
            if !script.trim().is_empty() {
                if let Err(err) = run_code(
                    &image,
                    &self.config,
                    &workdir,
                    &script,
                    self.config.default_timeout(),
                )
                .await
                {
                    let _ = fs::remove_dir_all(&workdir).await;
                    return Err(err);
                }
            }
        }

        let instance = MicroInstance {
            id: vm_id,
            image: image.name().to_string(),
            workdir: workdir.clone(),
        };
        let mut guard = self.instances.lock();
        guard.insert(
            vm_id,
            MicroVm {
                id: vm_id,
                image: instance.image.clone(),
                workdir,
            },
        );
        Ok(instance)
    }

    pub async fn execute(&self, request: MicroExecuteRequest) -> Result<MicroOutput> {
        let (image, workdir) = {
            let guard = self.instances.lock();
            let vm = guard
                .get(&request.vm_id)
                .ok_or_else(|| SandboxError::MicroVmNotFound(request.vm_id.to_string()))?;
            let image = self
                .config
                .image(&vm.image)
                .cloned()
                .ok_or_else(|| SandboxError::MicroImageNotConfigured(vm.image.clone()))?;
            (image, vm.workdir.clone())
        };

        let timeout = request
            .timeout
            .unwrap_or_else(|| self.config.default_timeout());
        if timeout.is_zero() {
            return Err(SandboxError::InvalidOperation(
                "micro execution timeout must be greater than zero".to_string(),
            ));
        }
        if timeout > self.config.max_timeout() {
            return Err(SandboxError::InvalidOperation(format!(
                "requested timeout {:?} exceeds maximum {:?}",
                timeout,
                self.config.max_timeout()
            )));
        }

        run_code(&image, &self.config, &workdir, &request.code, timeout).await
    }

    pub async fn stop(&self, vm_id: Uuid) -> Result<()> {
        let workdir = {
            let mut guard = self.instances.lock();
            let vm = guard
                .remove(&vm_id)
                .ok_or_else(|| SandboxError::MicroVmNotFound(vm_id.to_string()))?;
            vm.workdir
        };

        match fs::remove_dir_all(&workdir).await {
            Ok(_) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(SandboxError::Io(err)),
        }
    }
}

#[derive(Debug)]
pub struct MicroStartRequest {
    pub image: String,
    pub init_script: Option<String>,
}

#[derive(Debug)]
pub struct MicroExecuteRequest {
    pub vm_id: Uuid,
    pub code: String,
    pub timeout: Option<Duration>,
}

#[derive(Debug)]
pub struct MicroInstance {
    id: Uuid,
    image: String,
    workdir: PathBuf,
}

impl MicroInstance {
    pub fn id(&self) -> Uuid {
        self.id
    }

    pub fn image(&self) -> &str {
        &self.image
    }

    pub fn workdir(&self) -> &Path {
        &self.workdir
    }
}

#[derive(Debug)]
pub struct MicroOutput {
    pub exit_code: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub duration: Duration,
}

#[derive(Debug)]
struct MicroVm {
    id: Uuid,
    image: String,
    workdir: PathBuf,
}

async fn run_code(
    image: &MicroImage,
    config: &MicroConfig,
    workdir: &Path,
    source: &str,
    timeout: Duration,
) -> Result<MicroOutput> {
    let mut contents = source.to_string();
    if !contents.ends_with('\n') {
        contents.push('\n');
    }
    let script_name = format!("script_{}.{}", Uuid::new_v4(), image.extension());
    let script_path = workdir.join(script_name);

    {
        let mut file = fs::File::create(&script_path).await?;
        file.write_all(contents.as_bytes()).await?;
        file.sync_all().await?;
    }

    let mut command = Command::new(image.command());
    command.kill_on_drop(true);
    command.current_dir(workdir);
    command.stdin(std::process::Stdio::null());
    command.stdout(std::process::Stdio::piped());
    command.stderr(std::process::Stdio::piped());
    command.env_clear();
    for (key, value) in config.base_env() {
        command.env(key, value);
    }
    command.env("HOME", workdir);
    command.env("MICRO_SANDBOX_IMAGE", image.name());
    for (key, value) in image.env() {
        command.env(key, value);
    }
    for arg in image.args() {
        command.arg(arg);
    }
    command.arg(&script_path);

    let start = Instant::now();
    let output = match timeout(timeout, command.spawn()?.wait_with_output()).await {
        Ok(result) => result?,
        Err(_) => {
            let _ = fs::remove_file(&script_path).await;
            return Err(SandboxError::Timeout(timeout));
        }
    };
    let duration = start.elapsed();

    let _ = fs::remove_file(&script_path).await;

    if output.stdout.len() > config.max_output_bytes() {
        return Err(SandboxError::OutputTooLarge {
            stream: "stdout",
            limit: config.max_output_bytes(),
        });
    }
    if output.stderr.len() > config.max_output_bytes() {
        return Err(SandboxError::OutputTooLarge {
            stream: "stderr",
            limit: config.max_output_bytes(),
        });
    }

    let exit_code = output
        .status
        .code()
        .ok_or(SandboxError::TerminatedBySignal)?;

    Ok(MicroOutput {
        exit_code,
        stdout: output.stdout,
        stderr: output.stderr,
        duration,
    })
}
