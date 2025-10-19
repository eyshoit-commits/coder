use std::path::PathBuf;
use std::time::Duration;

use sandbox::micro::{
    MicroConfig, MicroExecuteRequest, MicroImage, MicroStartRequest, SandboxMicro,
};
use sandbox::SandboxError;
use tempfile::TempDir;

fn detect_binary(name: &str) -> Option<String> {
    std::env::var("PATH").ok().and_then(|path| {
        for segment in path.split(':').map(|entry| entry.trim()) {
            if segment.is_empty() {
                continue;
            }
            let candidate = PathBuf::from(segment).join(name);
            if candidate.is_file() {
                return Some(candidate.to_string_lossy().to_string());
            }
        }
        None
    })
}

fn build_micro_sandbox(root: &std::path::Path) -> SandboxMicro {
    let python_command = detect_binary("python3").unwrap_or_else(|| "python3".to_string());
    let image = MicroImage::new(
        "python",
        python_command,
        vec!["-u".to_string()],
        "py",
        vec![("PYTHONUNBUFFERED".to_string(), "1".to_string())],
    )
    .expect("valid python image");
    let config = MicroConfig::new(
        root,
        vec![image],
        Duration::from_millis(500),
        Duration::from_secs(2),
        64 * 1024,
        vec![
            (
                "PATH".to_string(),
                std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".to_string()),
            ),
            ("LANG".to_string(), "C".to_string()),
        ],
    )
    .expect("valid micro config");
    SandboxMicro::new(config)
}

#[tokio::test]
async fn executes_python_code() {
    let temp = TempDir::new().unwrap();
    let sandbox = build_micro_sandbox(temp.path());

    let instance = sandbox
        .start(MicroStartRequest {
            image: "python".to_string(),
            init_script: Some("import math".to_string()),
        })
        .await
        .expect("micro vm starts");

    let result = sandbox
        .execute(MicroExecuteRequest {
            vm_id: instance.id(),
            code: "print('micro sandbox')".to_string(),
            timeout: Some(Duration::from_millis(400)),
        })
        .await
        .expect("execution succeeds");

    assert_eq!(result.exit_code, 0);
    let stdout = String::from_utf8(result.stdout).expect("utf8 stdout");
    assert!(stdout.contains("micro sandbox"));
    assert!(result.stderr.is_empty());

    sandbox.stop(instance.id()).await.expect("micro vm stops");
}

#[tokio::test]
async fn rejects_unknown_image() {
    let temp = TempDir::new().unwrap();
    let sandbox = build_micro_sandbox(temp.path());

    let err = sandbox
        .start(MicroStartRequest {
            image: "unknown".to_string(),
            init_script: None,
        })
        .await
        .expect_err("image should be rejected");
    assert!(matches!(err, SandboxError::MicroImageNotConfigured(_)));
}
