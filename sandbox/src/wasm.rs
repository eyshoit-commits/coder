use std::fs;
use std::path::{Path, PathBuf};

use wasmer::imports;
use wasmer::{Engine, Instance, Module, Store, StoreLimitsBuilder, Value};

use crate::errors::{Result, SandboxError};
use crate::path;

#[derive(Clone, Debug)]
pub struct WasmConfig {
    root: PathBuf,
    max_memory_bytes: u64,
    max_table_elements: u32,
    default_fuel: Option<u64>,
}

impl WasmConfig {
    pub fn new(
        root: impl AsRef<Path>,
        max_memory_bytes: u64,
        max_table_elements: u32,
        default_fuel: Option<u64>,
    ) -> Result<Self> {
        if max_memory_bytes == 0 {
            return Err(SandboxError::InvalidOperation(
                "wasm memory limit must be greater than zero".to_string(),
            ));
        }
        if max_table_elements == 0 {
            return Err(SandboxError::InvalidOperation(
                "wasm table element limit must be greater than zero".to_string(),
            ));
        }

        let root = path::ensure_absolute_base(root.as_ref())?;
        fs::create_dir_all(&root)?;

        Ok(Self {
            root,
            max_memory_bytes,
            max_table_elements,
            default_fuel,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn max_memory_bytes(&self) -> u64 {
        self.max_memory_bytes
    }

    pub fn max_table_elements(&self) -> u32 {
        self.max_table_elements
    }

    pub fn default_fuel(&self) -> Option<u64> {
        self.default_fuel
    }
}

#[derive(Clone, Debug)]
pub struct SandboxWasm {
    config: WasmConfig,
    engine: Engine,
}

impl SandboxWasm {
    pub fn new(config: WasmConfig) -> Self {
        let engine = Engine::default();
        Self { config, engine }
    }

    pub fn config(&self) -> &WasmConfig {
        &self.config
    }

    pub fn invoke(&self, invocation: WasmInvocation) -> Result<Vec<WasmValue>> {
        let WasmInvocation {
            module,
            function,
            params,
            fuel,
            memory_limit,
            table_elements_limit,
        } = invocation;

        let bytes = match module {
            WasmModuleSource::Path(path) => {
                let resolved = path::resolve(self.config.root(), &path)?;
                fs::read(resolved)?
            }
            WasmModuleSource::Bytes(bytes) => bytes,
        };
        self.invoke_from_bytes(
            bytes,
            function,
            params,
            fuel,
            memory_limit,
            table_elements_limit,
        )
    }

    fn invoke_from_bytes(
        &self,
        bytes: Vec<u8>,
        function: String,
        params: Vec<WasmValue>,
        fuel: Option<u64>,
        memory_limit: Option<u64>,
        table_elements_limit: Option<u32>,
    ) -> Result<Vec<WasmValue>> {
        let module = Module::new(&self.engine, &bytes).map_err(|err| {
            SandboxError::InvalidOperation(format!("failed to compile wasm module: {err}"))
        })?;

        let mut store = Store::new(&self.engine);
        let fuel_budget = fuel.or(self.config.default_fuel);
        if let Some(fuel) = fuel_budget {
            store.add_fuel(fuel).map_err(|err| {
                SandboxError::InvalidOperation(format!("failed to configure wasm fuel: {err}"))
            })?;
        }

        let memory_limit = memory_limit.unwrap_or(self.config.max_memory_bytes);
        if memory_limit == 0 {
            return Err(SandboxError::InvalidOperation(
                "wasm memory limit must be greater than zero".to_string(),
            ));
        }
        let table_limit = table_elements_limit.unwrap_or(self.config.max_table_elements);
        if table_limit == 0 {
            return Err(SandboxError::InvalidOperation(
                "wasm table element limit must be greater than zero".to_string(),
            ));
        }

        let mut store_limits = StoreLimitsBuilder::new()
            .with_memory_size_limit(memory_limit)
            .with_table_elements_limit(table_limit)
            .build();
        store.limiter(|_| -> &mut dyn wasmer::StoreLimiter { &mut store_limits });

        let instance = Instance::new(&mut store, &module, &imports! {}).map_err(|err| {
            SandboxError::InvalidOperation(format!("failed to instantiate wasm module: {err}"))
        })?;
        let function = instance.exports.get_function(&function).map_err(|err| {
            SandboxError::InvalidOperation(format!(
                "failed to locate exported function '{}': {err}",
                function
            ))
        })?;

        let params: Vec<Value> = params.iter().map(Value::from).collect();
        let result_values = function
            .call(&mut store, &params)
            .map_err(|err| SandboxError::WasmTrap(err.to_string()))?;

        result_values.into_iter().map(WasmValue::try_from).collect()
    }
}

#[derive(Clone, Debug)]
pub struct WasmInvocation {
    pub module: WasmModuleSource,
    pub function: String,
    pub params: Vec<WasmValue>,
    pub fuel: Option<u64>,
    pub memory_limit: Option<u64>,
    pub table_elements_limit: Option<u32>,
}

impl WasmInvocation {
    pub fn new(module: WasmModuleSource, function: impl Into<String>) -> Self {
        Self {
            module,
            function: function.into(),
            params: Vec::new(),
            fuel: None,
            memory_limit: None,
            table_elements_limit: None,
        }
    }

    pub fn with_params(mut self, params: Vec<WasmValue>) -> Self {
        self.params = params;
        self
    }

    pub fn with_fuel(mut self, fuel: u64) -> Self {
        self.fuel = Some(fuel);
        self
    }

    pub fn with_memory_limit(mut self, bytes: u64) -> Self {
        self.memory_limit = Some(bytes);
        self
    }

    pub fn with_table_elements_limit(mut self, elements: u32) -> Self {
        self.table_elements_limit = Some(elements);
        self
    }
}

#[derive(Clone, Debug)]
pub enum WasmModuleSource {
    Path(PathBuf),
    Bytes(Vec<u8>),
}

impl WasmModuleSource {
    pub fn from_path(path: impl Into<PathBuf>) -> Self {
        Self::Path(path.into())
    }

    pub fn from_bytes(bytes: impl Into<Vec<u8>>) -> Self {
        Self::Bytes(bytes.into())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum WasmValue {
    I32(i32),
    I64(i64),
    F32(f32),
    F64(f64),
}

impl From<&WasmValue> for Value {
    fn from(value: &WasmValue) -> Self {
        match value {
            WasmValue::I32(inner) => Value::I32(*inner),
            WasmValue::I64(inner) => Value::I64(*inner),
            WasmValue::F32(inner) => Value::F32(*inner),
            WasmValue::F64(inner) => Value::F64(*inner),
        }
    }
}

impl TryFrom<Value> for WasmValue {
    type Error = SandboxError;

    fn try_from(value: Value) -> Result<Self> {
        match value {
            Value::I32(inner) => Ok(WasmValue::I32(inner)),
            Value::I64(inner) => Ok(WasmValue::I64(inner)),
            Value::F32(inner) => Ok(WasmValue::F32(inner)),
            Value::F64(inner) => Ok(WasmValue::F64(inner)),
            other => Err(SandboxError::InvalidOperation(format!(
                "unsupported wasm return value: {other:?}"
            ))),
        }
    }
}
