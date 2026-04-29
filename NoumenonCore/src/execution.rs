use std::collections::HashMap;

use anyhow::{Result, anyhow};

use crate::ir::{ExecutionStep, TrustedExecutionRequest};
use crate::routing::{LlmBackend, ModelRouter, parse_model_candidates};
use crate::syscall::{InMemorySyscallQueue, SyscallRequest, SyscallScheduler};

pub trait CapabilityAdapter: Send + Sync {
    fn capability_name(&self) -> &str;
    fn execute(&self, input: &str) -> Result<String>;
}

pub struct CapabilityRegistry {
    adapters: Vec<Box<dyn CapabilityAdapter>>,
    llm_backends: HashMap<String, Box<dyn LlmBackend>>,
    router: Option<ModelRouter>,
}

impl CapabilityRegistry {
    pub fn new() -> Self {
        Self {
            adapters: Vec::new(),
            llm_backends: HashMap::new(),
            router: None,
        }
    }

    pub fn register(&mut self, adapter: Box<dyn CapabilityAdapter>) {
        self.adapters.push(adapter);
    }

    pub fn register_llm_backend(&mut self, backend: Box<dyn LlmBackend>) {
        self.llm_backends
            .insert(backend.name().to_string(), backend);
    }

    pub fn set_router(&mut self, router: ModelRouter) {
        self.router = Some(router);
    }

    pub fn execute(&self, capability: &str, input: &str) -> Result<String> {
        let adapter = self
            .adapters
            .iter()
            .find(|a| a.capability_name() == capability)
            .ok_or_else(|| anyhow!("capability not found: {}", capability))?;
        adapter.execute(input)
    }

    fn execute_syscall(&self, req: &SyscallRequest, step: &ExecutionStep) -> Result<(String, u64)> {
        if req.capability == "llm.route" {
            return self.execute_llm_routed(step, &req.input).map(|o| (o, 2));
        }
        self.execute(&req.capability, &req.input).map(|o| (o, 1))
    }

    fn execute_llm_routed(&self, step: &ExecutionStep, input: &str) -> Result<String> {
        if self.llm_backends.is_empty() {
            return Err(anyhow!("no llm backends registered"));
        }
        let all_models: Vec<String> = self.llm_backends.keys().cloned().collect();
        let candidates = parse_model_candidates(&step.local_constraints, all_models);
        let router = self
            .router
            .as_ref()
            .ok_or_else(|| anyhow!("model router is not configured"))?;
        let selected = router.select(&candidates)?;
        let backend = self
            .llm_backends
            .get(&selected)
            .ok_or_else(|| anyhow!("selected model backend not found: {}", selected))?;
        backend.infer(input)
    }
}

#[derive(Debug, Clone)]
pub struct StepExecutionOutput {
    pub step_id: String,
    pub output: String,
    pub cost_units: u64,
}

pub struct KernelExecutor;

impl KernelExecutor {
    pub fn run_plan(
        registry: &CapabilityRegistry,
        request: &TrustedExecutionRequest,
    ) -> Result<Vec<StepExecutionOutput>> {
        let scheduler = SyscallScheduler::new(std::sync::Arc::new(InMemorySyscallQueue::default()));

        for step in &request.plan.steps {
            let capability = step
                .capability_ref
                .as_deref()
                .ok_or_else(|| anyhow!("step {} missing capability_ref", step.step_id))?
                .to_string();
            let priority = step
                .local_constraints
                .get("priority")
                .and_then(|v| v.parse::<u8>().ok())
                .unwrap_or(1);
            scheduler.submit(
                step.step_id.clone(),
                capability,
                step.input.clone(),
                priority,
            )?;
        }

        let responses = scheduler.drain(|req| {
            let step = request
                .plan
                .steps
                .iter()
                .find(|s| s.step_id == req.step_id)
                .ok_or_else(|| anyhow!("step not found for syscall: {}", req.step_id))?;
            registry.execute_syscall(req, step)
        })?;

        let mut outputs = Vec::with_capacity(responses.len());
        for r in responses {
            outputs.push(StepExecutionOutput {
                step_id: r.step_id,
                output: r.output,
                cost_units: r.cost_units,
            });
        }
        Ok(outputs)
    }

    pub fn run_batch(
        registry: &CapabilityRegistry,
        requests: &[TrustedExecutionRequest],
    ) -> Result<Vec<Vec<StepExecutionOutput>>> {
        let mut all = Vec::with_capacity(requests.len());
        for req in requests {
            all.push(Self::run_plan(registry, req)?);
        }
        Ok(all)
    }
}

#[derive(Debug, Clone)]
pub struct EchoCapability {
    name: String,
}

impl EchoCapability {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

impl CapabilityAdapter for EchoCapability {
    fn capability_name(&self) -> &str {
        &self.name
    }

    fn execute(&self, input: &str) -> Result<String> {
        Ok(format!("echo:{}", input))
    }
}
