use std::time::Duration;

use anyhow::Result;
use integration_tests::support::{system_path, temp_workspace};
use sandbox::micro::{
    MicroConfig, MicroExecuteRequest, MicroImage, MicroStartRequest, SandboxMicro,
};
use sandbox::run::{RunConfig, RunRequest, SandboxRun};
use sandbox::wasm::{SandboxWasm, WasmConfig, WasmInvocation, WasmModuleSource, WasmValue};
use sandbox::{SandboxConfig, SandboxFs};

#[tokio::test]
async fn full_sandbox_pipeline() -> Result<()> {
    let workspace = temp_workspace()?;
    let fs_root = workspace.path().join("workspace");
    let fs = SandboxFs::new(SandboxConfig::new(&fs_root, 512 * 1024)?);

    fs.write("project/message.txt", b"CyberDevStudio")?;
    let echoed = fs.read("project/message.txt")?;
    assert_eq!(echoed, b"CyberDevStudio");

    let run_config = RunConfig::new(
        &fs_root,
        vec!["/bin/sh".to_string()],
        vec!["CUSTOM_GREETING".to_string()],
        vec![("PATH".to_string(), system_path())],
        Duration::from_secs(2),
        Duration::from_secs(6),
        64 * 1024,
    )?;
    let run = SandboxRun::new(run_config);
    let run_output = run
        .execute(
            RunRequest::new("/bin/sh")
                .with_args(vec![
                    "-c".to_string(),
                    "echo $CUSTOM_GREETING && cat project/message.txt".to_string(),
                ])
                .with_env(vec![("CUSTOM_GREETING".to_string(), "ready".to_string())]),
        )
        .await?;
    assert_eq!(run_output.exit_code, 0);
    assert!(run_output.stderr.is_empty());
    assert_eq!(
        String::from_utf8(run_output.stdout)?,
        "ready\nCyberDevStudio\n"
    );

    let wasm_config = WasmConfig::new(fs_root.join("wasm"), 128 * 1024, 64, Some(10_000))?;
    let wasm = SandboxWasm::new(wasm_config);
    let module = wat::parse_str(
        "(module (func (export \"add\") (param i32 i32) (result i32) local.get 0 local.get 1 i32.add))",
    )?;
    let results = wasm.invoke(
        WasmInvocation::new(WasmModuleSource::Bytes(module), "add")
            .with_params(vec![WasmValue::I32(20), WasmValue::I32(22)]),
    )?;
    assert_eq!(results, vec![WasmValue::I32(42)]);

    let python = MicroImage::new(
        "python",
        "python3",
        vec!["-u".to_string()],
        "py",
        vec![("PYTHONUNBUFFERED".to_string(), "1".to_string())],
    )?;
    let micro_config = MicroConfig::new(
        fs_root.join("micro"),
        vec![python],
        Duration::from_secs(2),
        Duration::from_secs(6),
        64 * 1024,
        vec![("PATH".to_string(), system_path())],
    )?;
    let micro = SandboxMicro::new(micro_config);
    let instance = micro
        .start(MicroStartRequest {
            image: "python".to_string(),
            init_script: Some("print('boot')".to_string()),
        })
        .await?;
    let execution = micro
        .execute(MicroExecuteRequest {
            vm_id: instance.id(),
            code: "print(6 * 7)".to_string(),
            timeout: Some(Duration::from_secs(2)),
        })
        .await?;
    assert_eq!(execution.exit_code, 0);
    assert!(execution.stderr.is_empty());
    assert_eq!(String::from_utf8(execution.stdout)?, "42\n");

    micro.stop(instance.id()).await?;

    Ok(())
}
