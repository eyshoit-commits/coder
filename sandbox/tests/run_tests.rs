use std::time::Duration;

use sandbox::run::{RunConfig, RunRequest, SandboxRun};
use sandbox::SandboxError;
use tempfile::TempDir;

fn build_run_sandbox(root: &std::path::Path) -> SandboxRun {
    let config = RunConfig::new(
        root,
        vec!["/bin/sh".to_string()],
        vec!["PATH".to_string()],
        vec![("PATH".to_string(), "/usr/bin:/bin".to_string())],
        Duration::from_millis(500),
        Duration::from_secs(2),
        8 * 1024,
    )
    .expect("valid config");
    SandboxRun::new(config)
}

#[tokio::test]
async fn executes_allowed_program() {
    let temp = TempDir::new().unwrap();
    let sandbox = build_run_sandbox(temp.path());

    let request = RunRequest::new("/bin/sh")
        .with_args(vec!["-c".to_string(), "printf 'hello world'".to_string()]);
    let result = sandbox.execute(request).await.expect("command succeeds");
    assert_eq!(result.exit_code, 0);
    assert_eq!(result.stdout, b"hello world");
    assert!(result.stderr.is_empty());
}

#[tokio::test]
async fn enforces_timeout() {
    let temp = TempDir::new().unwrap();
    let sandbox = build_run_sandbox(temp.path());

    let request = RunRequest::new("/bin/sh")
        .with_args(vec!["-c".to_string(), "sleep 2".to_string()])
        .with_timeout(Duration::from_millis(200));

    let err = sandbox
        .execute(request)
        .await
        .expect_err("timeout expected");
    assert!(matches!(err, SandboxError::Timeout(_)));
}

#[tokio::test]
async fn rejects_forbidden_environment_variables() {
    let temp = TempDir::new().unwrap();
    let sandbox = build_run_sandbox(temp.path());

    let request =
        RunRequest::new("/bin/sh").with_env(vec![("SECRET".to_string(), "1".to_string())]);
    let err = sandbox
        .execute(request)
        .await
        .expect_err("env should be rejected");
    assert!(matches!(err, SandboxError::InvalidOperation(_)));
}
