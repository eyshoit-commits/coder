//! WebAssembly sandbox placeholder.
//!
//! Will manage Wasmtime instances, capability injection, and timeout handling.

pub struct WasmSandbox;

impl WasmSandbox {
    pub fn execute_module(_bytes: &[u8], _entry: &str) -> anyhow::Result<()> {
        unimplemented!("WASM sandbox is not implemented yet");
    }
}
