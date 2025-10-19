use std::time::Duration;

use anyhow::Result;
use integration_tests::support::{system_path, temp_workspace};
use sandbox::errors::SandboxError;
use sandbox::run::{RunConfig, RunRequest, SandboxRun};

#[tokio::test]
async fn executes_whitelisted_program() -> Result<()> {
    let workspace = temp_workspace()?;
    let config = RunConfig::new(
        workspace.path(),
        vec!["/bin/sh".to_string()],
        Vec::<String>::new(),
        vec![("PATH".to_string(), system_path())],
        Duration::from_secs(2),
        Duration::from_secs(5),
        32 * 1024,
    )?;
    let sandbox = SandboxRun::new(config);

    let output = sandbox
        .execute(
            RunRequest::new("/bin/sh")
                .with_args(vec!["-c".to_string(), "printf '%s' ready".to_string()]),
        )
        .await?;

    assert_eq!(output.exit_code, 0);
    assert_eq!(String::from_utf8(output.stdout)?, "ready");
    assert!(output.stderr.is_empty());

    Ok(())
}

#[tokio::test]
async fn rejects_disallowed_programs() -> Result<()> {
    let workspace = temp_workspace()?;
    let config = RunConfig::new(
        workspace.path(),
        vec!["/bin/sh".to_string()],
        Vec::<String>::new(),
        vec![("PATH".to_string(), system_path())],
        Duration::from_secs(2),
        Duration::from_secs(5),
        32 * 1024,
    )?;
    let sandbox = SandboxRun::new(config);

    let error = sandbox
        .execute(RunRequest::new("/usr/bin/env").with_args(vec!["python3".to_string()]))
        .await
        .expect_err("expected invalid operation");

    match error {
        SandboxError::InvalidOperation(message) => {
            assert!(
                message.contains("not permitted"),
                "unexpected message: {}",
                message
            );
        }
        other => panic!("unexpected error variant: {:?}", other),
    }

    Ok(())
}
