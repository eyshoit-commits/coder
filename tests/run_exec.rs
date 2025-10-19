//! Integration smoke tests for the sandbox process execution helper.

use cyberdev_sandbox::{execute, RunError, WORKSPACE_ROOT_ENV};

#[test]
fn run_exec_smoke() {
    let temp_dir = tempfile::tempdir().expect("create temp workspace");
    std::env::set_var(WORKSPACE_ROOT_ENV, temp_dir.path());

    let result = execute(
        "sh",
        &["-c".to_string(), "printf sandbox-ok".to_string()],
    )
    .expect("sandbox execute succeeds");

    std::env::remove_var(WORKSPACE_ROOT_ENV);

    assert!(result.status.success());
    assert_eq!(result.stdout, "sandbox-ok");
    assert_eq!(result.stderr, "");
}

#[test]
fn run_exec_disallows_unknown_binary() {
    let temp_dir = tempfile::tempdir().expect("create temp workspace");
    std::env::set_var(WORKSPACE_ROOT_ENV, temp_dir.path());

    let err = execute("rm", &[]).expect_err("rm should be blocked");

    std::env::remove_var(WORKSPACE_ROOT_ENV);

    assert!(matches!(err, RunError::CommandNotAllowed(cmd) if cmd == "rm"));
}
