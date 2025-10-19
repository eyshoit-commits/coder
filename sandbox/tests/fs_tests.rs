use sandbox::{SandboxConfig, SandboxFs};
use tempfile::TempDir;

#[test]
fn write_and_read_roundtrip() {
    let temp = TempDir::new().unwrap();
    let config = SandboxConfig::new(temp.path(), 512 * 1024).unwrap();
    let fs = SandboxFs::new(config);

    fs.write("example.txt", b"hello world").unwrap();
    let bytes = fs.read("example.txt").unwrap();
    assert_eq!(bytes, b"hello world");
}

#[test]
fn prevent_path_traversal() {
    let temp = TempDir::new().unwrap();
    let config = SandboxConfig::new(temp.path(), 512 * 1024).unwrap();
    let fs = SandboxFs::new(config);

    let err = fs.write("../evil.txt", b"bad").unwrap_err();
    assert!(format!("{}", err).contains("path traversal"));
}

#[test]
fn enforce_file_size_limit() {
    let temp = TempDir::new().unwrap();
    let config = SandboxConfig::new(temp.path(), 4).unwrap();
    let fs = SandboxFs::new(config);

    let err = fs.write("large.txt", b"12345").unwrap_err();
    assert!(format!("{}", err).contains("file too large"));
}
