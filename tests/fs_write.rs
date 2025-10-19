use anyhow::Result;
use integration_tests::support::temp_workspace;
use sandbox::errors::SandboxError;
use sandbox::{SandboxConfig, SandboxFs};

#[test]
fn writes_and_lists_files() -> Result<()> {
    let workspace = temp_workspace()?;
    let fs = SandboxFs::new(SandboxConfig::new(
        workspace.path().join("project"),
        512 * 1024,
    )?);

    fs.write("src/main.rs", b"fn main() {}\n")?;
    fs.write("README.md", b"CyberDevStudio")?;

    let contents = fs.read("src/main.rs")?;
    assert_eq!(contents, b"fn main() {}\n");

    let mut entries = fs.list(".")?;
    entries.retain(|entry| entry.name == "README.md" || entry.name == "src");
    assert_eq!(entries.len(), 2);

    Ok(())
}

#[test]
fn rejects_oversized_payloads() -> Result<()> {
    let workspace = temp_workspace()?;
    let fs = SandboxFs::new(SandboxConfig::new(workspace.path(), 16)?);

    let result = fs.write("big.bin", vec![0_u8; 32]);
    match result {
        Err(SandboxError::FileTooLarge(size)) => assert!(size > 16),
        other => panic!("expected size error, got {:?}", other),
    }

    Ok(())
}
