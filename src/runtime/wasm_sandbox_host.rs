use std::{fs, path::Path, sync::Arc};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use wasmtime::{Caller, Config, Engine, Instance, Linker, Memory, Module, Store, TypedFunc};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WasmSandboxPlan {
    pub module_path: String,
    #[serde(default = "default_entrypoint")]
    pub entrypoint: String,
    #[serde(default)]
    pub payload: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WasmSandboxExecutionResult {
    pub module_path: String,
    pub entrypoint: String,
    pub output: Value,
    pub host_logs: Vec<String>,
    pub host_events: Vec<Value>,
}

#[derive(Debug, Clone)]
pub struct WasmSandboxLimits {
    pub fuel: u64,
    pub max_host_logs: usize,
    pub max_host_events: usize,
}

impl Default for WasmSandboxLimits {
    fn default() -> Self {
        Self {
            fuel: 5_000_000,
            max_host_logs: 64,
            max_host_events: 64,
        }
    }
}

#[derive(Clone)]
pub struct WasmSandboxHost {
    engine: Arc<Engine>,
    limits: WasmSandboxLimits,
}

#[derive(Debug, Clone)]
struct HostState {
    logs: Vec<String>,
    events: Vec<Value>,
    max_logs: usize,
    max_events: usize,
}

impl WasmSandboxHost {
    pub fn new(limits: WasmSandboxLimits) -> Result<Self> {
        let mut config = Config::new();
        config.consume_fuel(true);
        let engine = Engine::new(&config).context("failed to initialize wasmtime engine")?;
        Ok(Self {
            engine: Arc::new(engine),
            limits,
        })
    }

    pub fn execute_plan(&self, plan: &WasmSandboxPlan) -> Result<WasmSandboxExecutionResult> {
        let bytes = fs::read(&plan.module_path).with_context(|| {
            format!(
                "failed to read wasm module at '{}'",
                Path::new(&plan.module_path).display()
            )
        })?;
        self.execute_bytes(plan, &bytes)
    }

    pub fn execute_bytes(
        &self,
        plan: &WasmSandboxPlan,
        module_bytes: &[u8],
    ) -> Result<WasmSandboxExecutionResult> {
        let module = Module::new(&self.engine, module_bytes)
            .context("failed to compile wasm module for sandbox host")?;
        let mut linker = Linker::new(&self.engine);
        install_restricted_host_api(&mut linker)?;

        let mut store = Store::new(
            &self.engine,
            HostState {
                logs: Vec::new(),
                events: Vec::new(),
                max_logs: self.limits.max_host_logs,
                max_events: self.limits.max_host_events,
            },
        );
        store
            .set_fuel(self.limits.fuel)
            .context("failed to set fuel for wasm sandbox")?;

        let instance = linker
            .instantiate(&mut store, &module)
            .context("failed to instantiate wasm sandbox module")?;
        let memory = memory_export(&mut store, &instance)?;
        let allocator = allocator_export(&mut store, &instance)?;
        let entry = entrypoint_export(&mut store, &instance, &plan.entrypoint)?;

        let payload_bytes = serde_json::to_vec(&plan.payload)?;
        let payload_len = i32::try_from(payload_bytes.len()).context("payload is too large")?;
        let payload_ptr = allocator.call(&mut store, payload_len)?;
        if payload_ptr <= 0 {
            bail!("wasm allocator returned an invalid pointer");
        }
        write_memory(&mut store, &memory, payload_ptr as usize, &payload_bytes)?;

        let output_ptr = entry.call(&mut store, (payload_ptr, payload_len))?;
        if output_ptr <= 0 {
            bail!("wasm entrypoint returned empty output pointer");
        }

        let output_json = read_cstring(&mut store, &memory, output_ptr as usize)?;
        let output = serde_json::from_str::<Value>(&output_json)
            .context("failed to parse wasm entrypoint output as json")?;

        let state = store.data();
        Ok(WasmSandboxExecutionResult {
            module_path: plan.module_path.clone(),
            entrypoint: plan.entrypoint.clone(),
            output,
            host_logs: state.logs.clone(),
            host_events: state.events.clone(),
        })
    }
}

fn install_restricted_host_api(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        "autoloop",
        "log_utf8",
        |mut caller: Caller<'_, HostState>, ptr: i32, len: i32| -> i32 {
            let Some(message) = read_guest_utf8(&mut caller, ptr, len) else {
                return 0;
            };
            let state = caller.data_mut();
            if state.logs.len() < state.max_logs {
                state.logs.push(message);
            }
            1
        },
    )?;

    linker.func_wrap(
        "autoloop",
        "emit_event_json",
        |mut caller: Caller<'_, HostState>, ptr: i32, len: i32| -> i32 {
            let Some(raw) = read_guest_utf8(&mut caller, ptr, len) else {
                return 0;
            };
            let Ok(value) = serde_json::from_str::<Value>(&raw) else {
                return 0;
            };
            let state = caller.data_mut();
            if state.events.len() < state.max_events {
                state.events.push(value);
            }
            1
        },
    )?;

    Ok(())
}

fn read_guest_utf8(caller: &mut Caller<'_, HostState>, ptr: i32, len: i32) -> Option<String> {
    if ptr < 0 || len < 0 {
        return None;
    }
    let memory = caller.get_export("memory")?.into_memory()?;
    let data = memory.data(caller);
    let start = usize::try_from(ptr).ok()?;
    let span = usize::try_from(len).ok()?;
    let end = start.checked_add(span)?;
    if end > data.len() {
        return None;
    }
    std::str::from_utf8(&data[start..end])
        .ok()
        .map(|value| value.to_string())
}

fn memory_export(store: &mut Store<HostState>, instance: &Instance) -> Result<Memory> {
    instance
        .get_memory(&mut *store, "memory")
        .context("sandbox wasm is missing exported memory")
}

fn allocator_export(
    store: &mut Store<HostState>,
    instance: &Instance,
) -> Result<TypedFunc<i32, i32>> {
    for name in ["autoloop_alloc", "malloc", "opa_malloc"] {
        if let Ok(func) = instance.get_typed_func::<i32, i32>(&mut *store, name) {
            return Ok(func);
        }
    }
    bail!("sandbox wasm must export allocator function: autoloop_alloc|malloc|opa_malloc")
}

fn entrypoint_export(
    store: &mut Store<HostState>,
    instance: &Instance,
    entrypoint: &str,
) -> Result<TypedFunc<(i32, i32), i32>> {
    instance
        .get_typed_func::<(i32, i32), i32>(&mut *store, entrypoint)
        .with_context(|| format!("sandbox wasm is missing entrypoint '{entrypoint}'"))
}

fn write_memory(
    store: &mut Store<HostState>,
    memory: &Memory,
    ptr: usize,
    bytes: &[u8],
) -> Result<()> {
    let data = memory.data_mut(&mut *store);
    let end = ptr.saturating_add(bytes.len());
    if end > data.len() {
        bail!("sandbox wasm write out of bounds");
    }
    data[ptr..end].copy_from_slice(bytes);
    Ok(())
}

fn read_cstring(store: &mut Store<HostState>, memory: &Memory, ptr: usize) -> Result<String> {
    let data = memory.data(&mut *store);
    if ptr >= data.len() {
        bail!("sandbox wasm read out of bounds");
    }
    let mut end = ptr;
    while end < data.len() && data[end] != 0 {
        end += 1;
    }
    let raw = &data[ptr..end];
    String::from_utf8(raw.to_vec()).context("sandbox wasm output was not valid utf-8")
}

fn default_entrypoint() -> String {
    "autoloop_run".to_string()
}
