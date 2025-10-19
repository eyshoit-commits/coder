use std::fs;

use sandbox::wasm::{SandboxWasm, WasmConfig, WasmInvocation, WasmModuleSource, WasmValue};

#[test]
fn executes_simple_wasm_function() {
    let temp = tempfile::tempdir().expect("create temp dir");
    let root = temp.path().canonicalize().expect("canonical root");

    let wasm_bytes = wat::parse_str(
        r#"
        (module
            (func $add (param $lhs i32) (param $rhs i32) (result i32)
                local.get $lhs
                local.get $rhs
                i32.add)
            (export "add" (func $add))
        )
        "#,
    )
    .expect("compile wat");

    let module_path = root.join("add.wasm");
    fs::write(&module_path, &wasm_bytes).expect("write wasm module");

    let config = WasmConfig::new(root.clone(), 64 * 1024, 1024, None).expect("config");
    let sandbox = SandboxWasm::new(config);

    let invocation = WasmInvocation::new(WasmModuleSource::from_path("add.wasm"), "add")
        .with_params(vec![WasmValue::I32(5), WasmValue::I32(7)]);

    let outputs = sandbox.invoke(invocation).expect("invoke wasm");
    assert_eq!(outputs, vec![WasmValue::I32(12)]);
}
