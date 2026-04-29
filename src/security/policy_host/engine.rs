use std::{fs, path::Path, sync::Arc};

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use serde_json::Value;
use wasmtime::{Caller, Engine, Instance, Linker, Memory, Module, Store, TypedFunc};

use crate::contracts::policy_pdp::{
    DecisionReason, PolicyDecision, PolicyMode, PolicyVersion,
};

use super::traits::{PolicyHost, PolicyHostMetadata, UnifiedPolicyInput};

#[derive(Clone)]
pub struct WasmPolicyHost {
    engine: Arc<Engine>,
    module: Arc<Module>,
    metadata: PolicyHostMetadata,
}

impl WasmPolicyHost {
    pub fn from_wasm_file(
        policy_id: impl Into<String>,
        policy_version: PolicyVersion,
        mode: PolicyMode,
        wasm_entrypoint: impl Into<String>,
        wasm_path: impl AsRef<Path>,
    ) -> Result<Self> {
        let bytes = fs::read(&wasm_path).with_context(|| {
            format!(
                "failed to read policy wasm file: {}",
                wasm_path.as_ref().display()
            )
        })?;
        Self::from_wasm_bytes(policy_id, policy_version, mode, wasm_entrypoint, &bytes)
    }

    pub fn from_wasm_bytes(
        policy_id: impl Into<String>,
        policy_version: PolicyVersion,
        mode: PolicyMode,
        wasm_entrypoint: impl Into<String>,
        bytes: &[u8],
    ) -> Result<Self> {
        let engine = Engine::default();
        let module = Module::new(&engine, bytes).context("failed to parse policy wasm module")?;
        Ok(Self {
            engine: Arc::new(engine),
            module: Arc::new(module),
            metadata: PolicyHostMetadata {
                policy_id: policy_id.into(),
                policy_version,
                mode,
                wasm_entrypoint: wasm_entrypoint.into(),
            },
        })
    }

    fn evaluate_opa(&self, input: &UnifiedPolicyInput) -> Result<Value> {
        let mut linker = Linker::new(&self.engine);
        install_opa_imports(&mut linker)?;

        let mut store = Store::new(&self.engine, ());
        let instance = linker
            .instantiate(&mut store, &self.module)
            .context("failed to instantiate policy wasm")?;

        let result = if has_opa_abi(&mut store, &instance) {
            evaluate_with_opa_abi(&mut store, &instance, &input.to_eval_value())?
        } else {
            evaluate_with_basic_entrypoint(
                &mut store,
                &instance,
                &self.metadata.wasm_entrypoint,
                &input.to_eval_value(),
            )?
        };

        Ok(result)
    }
}

#[async_trait]
impl PolicyHost for WasmPolicyHost {
    fn metadata(&self) -> &PolicyHostMetadata {
        &self.metadata
    }

    async fn evaluate(&self, input: &UnifiedPolicyInput) -> Result<PolicyDecision> {
        let raw_result = self.evaluate_opa(input)?;
        Ok(decode_policy_decision(
            raw_result,
            self.metadata.mode.clone(),
            self.metadata.policy_version.clone(),
        ))
    }
}

fn install_opa_imports(linker: &mut Linker<()>) -> Result<()> {
    linker.func_wrap("env", "opa_abort", |_caller: Caller<'_, ()>, _addr: i32| {})?;
    linker.func_wrap("env", "opa_println", |_caller: Caller<'_, ()>, _addr: i32| {})?;
    linker.func_wrap("env", "opa_builtin0", |_caller: Caller<'_, ()>, _builtin_id: i32| -> i32 {
        0
    })?;
    linker.func_wrap(
        "env",
        "opa_builtin1",
        |_caller: Caller<'_, ()>, _builtin_id: i32, _a1: i32| -> i32 { 0 },
    )?;
    linker.func_wrap(
        "env",
        "opa_builtin2",
        |_caller: Caller<'_, ()>, _builtin_id: i32, _a1: i32, _a2: i32| -> i32 { 0 },
    )?;
    linker.func_wrap(
        "env",
        "opa_builtin3",
        |_caller: Caller<'_, ()>, _builtin_id: i32, _a1: i32, _a2: i32, _a3: i32| -> i32 { 0 },
    )?;
    linker.func_wrap(
        "env",
        "opa_builtin4",
        |_caller: Caller<'_, ()>,
         _builtin_id: i32,
         _a1: i32,
         _a2: i32,
         _a3: i32,
         _a4: i32|
         -> i32 { 0 },
    )?;
    Ok(())
}

fn has_opa_abi(store: &mut Store<()>, instance: &Instance) -> bool {
    [
        "opa_malloc",
        "opa_json_parse",
        "opa_eval_ctx_new",
        "opa_eval_ctx_set_input",
        "opa_eval",
        "opa_eval_ctx_get_result",
        "opa_json_dump",
    ]
    .iter()
    .all(|name| instance.get_func(&mut *store, name).is_some())
}

fn evaluate_with_basic_entrypoint(
    store: &mut Store<()>,
    instance: &Instance,
    entrypoint: &str,
    input: &Value,
) -> Result<Value> {
    let func: TypedFunc<(i32, i32), i32> = instance
        .get_typed_func(&mut *store, entrypoint)
        .with_context(|| format!("missing basic policy entrypoint '{entrypoint}'"))?;
    let memory = memory_export(store, instance)?;

    let input_bytes = serde_json::to_vec(input)?;
    let input_len = input_bytes.len() as i32;
    let malloc: TypedFunc<i32, i32> = instance
        .get_typed_func(&mut *store, "opa_malloc")
        .context("basic entrypoint requires exported opa_malloc")?;
    let in_ptr = malloc.call(&mut *store, input_len)?;
    write_memory(store, &memory, in_ptr as usize, &input_bytes)?;

    let out_ptr = func.call(&mut *store, (in_ptr, input_len))?;
    if out_ptr <= 0 {
        bail!("basic entrypoint returned empty result pointer");
    }

    let out_json = read_cstring(store, &memory, out_ptr as usize)?;
    serde_json::from_str(&out_json).context("failed to decode basic entrypoint result json")
}

fn evaluate_with_opa_abi(store: &mut Store<()>, instance: &Instance, input: &Value) -> Result<Value> {
    let memory = memory_export(store, instance)?;

    let opa_malloc: TypedFunc<i32, i32> = instance.get_typed_func(&mut *store, "opa_malloc")?;
    let opa_json_parse: TypedFunc<(i32, i32), i32> =
        instance.get_typed_func(&mut *store, "opa_json_parse")?;
    let opa_eval_ctx_new: TypedFunc<(), i32> =
        instance.get_typed_func(&mut *store, "opa_eval_ctx_new")?;
    let opa_eval_ctx_set_input: TypedFunc<(i32, i32), ()> =
        instance.get_typed_func(&mut *store, "opa_eval_ctx_set_input")?;
    let opa_eval: TypedFunc<i32, ()> = instance.get_typed_func(&mut *store, "opa_eval")?;
    let opa_eval_ctx_get_result: TypedFunc<i32, i32> =
        instance.get_typed_func(&mut *store, "opa_eval_ctx_get_result")?;
    let opa_json_dump: TypedFunc<i32, i32> = instance.get_typed_func(&mut *store, "opa_json_dump")?;

    let input_bytes = serde_json::to_vec(input)?;
    let input_len = input_bytes.len() as i32;
    let input_ptr = opa_malloc.call(&mut *store, input_len)?;
    write_memory(store, &memory, input_ptr as usize, &input_bytes)?;

    let input_value_addr = opa_json_parse.call(&mut *store, (input_ptr, input_len))?;
    if input_value_addr == 0 {
        bail!("opa_json_parse returned null for policy input");
    }

    let eval_ctx = opa_eval_ctx_new.call(&mut *store, ())?;
    opa_eval_ctx_set_input.call(&mut *store, (eval_ctx, input_value_addr))?;
    opa_eval.call(&mut *store, eval_ctx)?;

    let result_value_addr = opa_eval_ctx_get_result.call(&mut *store, eval_ctx)?;
    if result_value_addr == 0 {
        bail!("opa_eval_ctx_get_result returned null");
    }

    let result_json_addr = opa_json_dump.call(&mut *store, result_value_addr)?;
    if result_json_addr == 0 {
        bail!("opa_json_dump returned null");
    }

    let json = read_cstring(store, &memory, result_json_addr as usize)?;
    serde_json::from_str(&json).context("failed to parse OPA evaluation JSON")
}

fn memory_export(store: &mut Store<()>, instance: &Instance) -> Result<Memory> {
    instance
        .get_memory(&mut *store, "memory")
        .context("policy wasm is missing exported memory")
}

fn write_memory(store: &mut Store<()>, memory: &Memory, ptr: usize, bytes: &[u8]) -> Result<()> {
    let data = memory.data_mut(&mut *store);
    let end = ptr.saturating_add(bytes.len());
    if end > data.len() {
        bail!("wasm write out of bounds while writing policy input");
    }
    data[ptr..end].copy_from_slice(bytes);
    Ok(())
}

fn read_cstring(store: &mut Store<()>, memory: &Memory, ptr: usize) -> Result<String> {
    let data = memory.data(&mut *store);
    if ptr >= data.len() {
        bail!("wasm read out of bounds at pointer {ptr}");
    }
    let mut end = ptr;
    while end < data.len() && data[end] != 0 {
        end += 1;
    }
    let raw = &data[ptr..end];
    String::from_utf8(raw.to_vec()).context("policy result was not valid utf-8")
}

fn decode_policy_decision(value: Value, mode: PolicyMode, version: PolicyVersion) -> PolicyDecision {
    let allowed = extract_allow(&value);
    let reasons = extract_reasons(&value);

    PolicyDecision {
        allowed,
        mode,
        version,
        reasons,
        mask_rules: Vec::new(),
        drop_rules: Vec::new(),
    }
}

fn extract_allow(value: &Value) -> bool {
    match value {
        Value::Bool(flag) => *flag,
        Value::Object(map) => map
            .get("allow")
            .and_then(Value::as_bool)
            .or_else(|| map.get("result").and_then(Value::as_bool))
            .unwrap_or(false),
        Value::Array(items) => items
            .first()
            .and_then(|first| first.get("result"))
            .and_then(Value::as_bool)
            .or_else(|| {
                items
                    .first()
                    .and_then(|first| first.get("expressions"))
                    .and_then(Value::as_array)
                    .and_then(|exprs| exprs.first())
                    .and_then(|expr| expr.get("value"))
                    .and_then(Value::as_bool)
            })
            .unwrap_or(false),
        _ => false,
    }
}

fn extract_reasons(value: &Value) -> Vec<DecisionReason> {
    let mut reasons = Vec::new();

    if let Some(array) = value
        .get("reasons")
        .and_then(Value::as_array)
        .or_else(|| value.get("deny_reasons").and_then(Value::as_array))
    {
        for item in array {
            if let Some(text) = item.as_str() {
                reasons.push(DecisionReason {
                    code: "policy.reason".into(),
                    message: text.to_string(),
                    rule_id: None,
                });
            }
        }
    }

    if reasons.is_empty() {
        reasons.push(DecisionReason {
            code: if extract_allow(value) {
                "policy.allow".into()
            } else {
                "policy.deny".into()
            },
            message: "evaluated by wasm policy host".into(),
            rule_id: None,
        });
    }

    reasons
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decision_decoder_accepts_opa_boolean_array_shape() {
        let raw = serde_json::json!([
            {
                "result": true
            }
        ]);
        let decision = decode_policy_decision(
            raw,
            PolicyMode::Shadow,
            PolicyVersion {
                id: "v1".into(),
                revision: 1,
            },
        );
        assert!(decision.allowed);
    }
}
