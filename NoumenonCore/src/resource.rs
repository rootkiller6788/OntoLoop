use std::process::Command;
use std::sync::Mutex;

use anyhow::{Result, anyhow};

use crate::ir::{ConstraintSet, ExecutionStep, RuntimeIsland};

#[derive(Debug, Clone)]
pub struct ResourceRequirement {
    pub cpu_units: u32,
    pub memory_mb: u32,
    pub max_runtime_ms: u64,
}

impl ResourceRequirement {
    pub fn from_constraints(c: &ConstraintSet) -> Self {
        Self {
            cpu_units: c.max_cpu_units.max(1),
            memory_mb: c.max_memory_mb.max(64),
            max_runtime_ms: c.max_runtime_ms.max(1_000),
        }
    }
}

pub trait ResourceGovernor: Send + Sync {
    fn reserve(&self, req: &ResourceRequirement) -> Result<()>;
    fn release(&self, req: &ResourceRequirement);
}

#[derive(Debug)]
pub struct SimpleResourceGovernor {
    state: Mutex<ResourcePoolState>,
}

#[derive(Debug)]
struct ResourcePoolState {
    cpu_limit: u32,
    memory_limit_mb: u32,
    cpu_used: u32,
    memory_used_mb: u32,
}

impl SimpleResourceGovernor {
    pub fn new(cpu_limit: u32, memory_limit_mb: u32) -> Self {
        Self {
            state: Mutex::new(ResourcePoolState {
                cpu_limit,
                memory_limit_mb,
                cpu_used: 0,
                memory_used_mb: 0,
            }),
        }
    }
}

impl ResourceGovernor for SimpleResourceGovernor {
    fn reserve(&self, req: &ResourceRequirement) -> Result<()> {
        let mut s = self
            .state
            .lock()
            .map_err(|_| anyhow!("resource lock poisoned"))?;
        if s.cpu_used + req.cpu_units > s.cpu_limit {
            return Err(anyhow!("cpu quota exceeded"));
        }
        if s.memory_used_mb + req.memory_mb > s.memory_limit_mb {
            return Err(anyhow!("memory quota exceeded"));
        }
        s.cpu_used += req.cpu_units;
        s.memory_used_mb += req.memory_mb;
        Ok(())
    }

    fn release(&self, req: &ResourceRequirement) {
        if let Ok(mut s) = self.state.lock() {
            s.cpu_used = s.cpu_used.saturating_sub(req.cpu_units);
            s.memory_used_mb = s.memory_used_mb.saturating_sub(req.memory_mb);
        }
    }
}

pub struct ResourceReservation<'a> {
    governor: &'a dyn ResourceGovernor,
    req: ResourceRequirement,
}

impl<'a> ResourceReservation<'a> {
    pub fn new(governor: &'a dyn ResourceGovernor, req: ResourceRequirement) -> Result<Self> {
        governor.reserve(&req)?;
        Ok(Self { governor, req })
    }
}

impl Drop for ResourceReservation<'_> {
    fn drop(&mut self) {
        self.governor.release(&self.req);
    }
}

fn parse_runtime_island(label: &str) -> Option<RuntimeIsland> {
    match label.trim().to_ascii_lowercase().as_str() {
        "trusted" => Some(RuntimeIsland::Trusted),
        "wasm" => Some(RuntimeIsland::Wasm),
        "ffi" => Some(RuntimeIsland::Ffi),
        "plugin" => Some(RuntimeIsland::Plugin),
        _ => None,
    }
}

fn infer_runtime_island_from_capability(capability_ref: Option<&str>) -> Option<RuntimeIsland> {
    let cap = capability_ref?.to_ascii_lowercase();
    if cap.contains("wasm") {
        return Some(RuntimeIsland::Wasm);
    }
    if cap.contains("ffi") || cap.contains("cabi") {
        return Some(RuntimeIsland::Ffi);
    }
    if cap.contains("plugin") {
        return Some(RuntimeIsland::Plugin);
    }
    None
}

fn effective_runtime_island(step: &ExecutionStep) -> RuntimeIsland {
    if let Some(parsed) = step
        .local_constraints
        .get("runtime_island")
        .and_then(|v| parse_runtime_island(v))
    {
        return parsed;
    }
    if !matches!(step.runtime_island, RuntimeIsland::Trusted) {
        return step.runtime_island.clone();
    }
    infer_runtime_island_from_capability(step.capability_ref.as_deref())
        .unwrap_or(RuntimeIsland::Trusted)
}

fn non_empty(value: Option<&str>) -> Option<String> {
    let v = value?.trim();
    if v.is_empty() {
        None
    } else {
        Some(v.to_string())
    }
}

fn resolve_hardening_profile(step: &ExecutionStep) -> Option<String> {
    if let Some(v) = non_empty(step.hardening.hardening_profile.as_deref()) {
        return Some(v);
    }
    non_empty(
        step.local_constraints
            .get("hardening_profile")
            .map(|v| v.as_str()),
    )
}

fn resolve_syscall_constraints(step: &ExecutionStep) -> bool {
    if non_empty(step.hardening.syscall_policy_ref.as_deref()).is_some() {
        return true;
    }
    if !step.hardening.syscall_allowlist.is_empty() {
        return true;
    }
    if non_empty(
        step.local_constraints
            .get("syscall_policy_ref")
            .map(|v| v.as_str()),
    )
    .is_some()
    {
        return true;
    }
    non_empty(
        step.local_constraints
            .get("syscall_allowlist")
            .map(|v| v.as_str()),
    )
    .is_some()
}

fn resolve_seccomp_profile(step: &ExecutionStep) -> bool {
    if non_empty(step.hardening.seccomp_profile_ref.as_deref()).is_some() {
        return true;
    }
    non_empty(
        step.local_constraints
            .get("seccomp_profile_ref")
            .map(|v| v.as_str()),
    )
    .is_some()
}

fn resolve_bool_constraint(step: &ExecutionStep, key: &str) -> Option<bool> {
    let raw = step.local_constraints.get(key)?.trim().to_ascii_lowercase();
    match raw.as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

fn resolve_namespace_isolation(step: &ExecutionStep) -> bool {
    step.hardening.enforce_namespace_isolation
        || resolve_bool_constraint(step, "enforce_namespace_isolation").unwrap_or(false)
}

fn resolve_cgroup_isolation(step: &ExecutionStep) -> bool {
    step.hardening.enforce_cgroup_isolation
        || resolve_bool_constraint(step, "enforce_cgroup_isolation").unwrap_or(false)
}

fn resolve_fs_isolation(step: &ExecutionStep) -> bool {
    step.hardening.enforce_fs_isolation
        || resolve_bool_constraint(step, "enforce_fs_isolation").unwrap_or(false)
}

fn resolve_network_isolation(step: &ExecutionStep) -> bool {
    step.hardening.enforce_network_isolation
        || resolve_bool_constraint(step, "enforce_network_isolation").unwrap_or(false)
}

fn parse_u64_constraint(step: &ExecutionStep, key: &str) -> Option<u64> {
    step.local_constraints.get(key)?.parse::<u64>().ok()
}

fn parse_u32_constraint(step: &ExecutionStep, key: &str) -> Option<u32> {
    step.local_constraints.get(key)?.parse::<u32>().ok()
}

fn resolve_step_runtime_ms(step: &ExecutionStep) -> Option<u64> {
    step.hardening
        .max_runtime_ms
        .or_else(|| parse_u64_constraint(step, "step_max_runtime_ms"))
}

fn resolve_step_memory_mb(step: &ExecutionStep) -> Option<u32> {
    step.hardening
        .max_memory_mb
        .or_else(|| parse_u32_constraint(step, "step_max_memory_mb"))
}

fn resolve_step_cpu_units(step: &ExecutionStep) -> Option<u32> {
    step.hardening
        .max_cpu_units
        .or_else(|| parse_u32_constraint(step, "step_max_cpu_units"))
}


fn resolve_isolation_backend(step: &ExecutionStep) -> Option<String> {
    non_empty(
        step.local_constraints
            .get("isolation_backend")
            .map(|v| v.as_str()),
    )
}

fn command_available(program: &str) -> bool {
    Command::new(program)
        .arg("--version")
        .output()
        .map(|output| output.status.success() || !output.stdout.is_empty() || !output.stderr.is_empty())
        .unwrap_or(false)
}

fn linux_kernel_isolation_primitives_available() -> bool {
    std::path::Path::new("/sys/fs/cgroup").exists() && std::path::Path::new("/proc/self/ns").exists()
}

fn ensure_isolation_executor_available(step: &ExecutionStep, backend: &str) -> Result<()> {
    let capability = step
        .capability_ref
        .as_deref()
        .unwrap_or("<missing-capability>");

    let backend_lc = backend.trim().to_ascii_lowercase();
    let island = effective_runtime_island(step);

    let linux_backend_ok = |cmd: &str| -> bool {
        cfg!(target_os = "linux") && linux_kernel_isolation_primitives_available() && command_available(cmd)
    };

    let supported = match backend_lc.as_str() {
        "auto" => {
            if cfg!(target_os = "windows") {
                true
            } else {
                linux_backend_ok("nsjail") || linux_backend_ok("bwrap") || linux_backend_ok("firejail") || linux_backend_ok("unshare")
            }
        }
        "nsjail" => linux_backend_ok("nsjail"),
        "bwrap" | "bubblewrap" => linux_backend_ok("bwrap"),
        "firejail" => linux_backend_ok("firejail"),
        "unshare" => linux_backend_ok("unshare"),
        "wasmtime" => command_available("wasmtime"),
        "job_object" | "windows-job-object" => cfg!(target_os = "windows"),
        custom => command_available(custom),
    };

    if !supported {
        return Err(anyhow!(
            "isolation backend '{}' is unavailable for high-risk island capability: {}",
            backend,
            capability
        ));
    }

    match island {
        RuntimeIsland::Wasm => {
            if backend_lc == "wasmtime"
                || backend_lc == "auto"
                || backend_lc == "nsjail"
                || backend_lc == "bwrap"
                || backend_lc == "bubblewrap"
                || backend_lc == "firejail"
                || backend_lc == "unshare"
                || backend_lc == "unshare"
                || backend_lc == "job_object"
                || backend_lc == "windows-job-object"
            {
                Ok(())
            } else {
                Err(anyhow!(
                    "isolation backend '{}' is not allowed for wasm island capability: {}",
                    backend,
                    capability
                ))
            }
        }
        RuntimeIsland::Ffi | RuntimeIsland::Plugin => {
            if backend_lc == "auto"
                || backend_lc == "nsjail"
                || backend_lc == "bwrap"
                || backend_lc == "bubblewrap"
                || backend_lc == "firejail"
                || backend_lc == "unshare"
                || backend_lc == "unshare"
                || backend_lc == "job_object"
                || backend_lc == "windows-job-object"
            {
                Ok(())
            } else {
                Err(anyhow!(
                    "isolation backend '{}' is not allowed for {} island capability: {}",
                    backend,
                    match island {
                        RuntimeIsland::Ffi => "ffi",
                        RuntimeIsland::Plugin => "plugin",
                        _ => "unknown",
                    },
                    capability
                ))
            }
        }
        RuntimeIsland::Trusted => Ok(()),
    }
}
pub fn enforce_runtime_island_hardening(
    step: &ExecutionStep,
    constraints: &ConstraintSet,
) -> Result<()> {
    let island = effective_runtime_island(step);
    if !island.requires_hardening() {
        return Ok(());
    }

    let capability = step
        .capability_ref
        .as_deref()
        .unwrap_or("<missing-capability>");
    let profile = resolve_hardening_profile(step).ok_or_else(|| {
        anyhow!(
            "missing hardening_profile for high-risk island capability: {}",
            capability
        )
    })?;
    if profile.eq_ignore_ascii_case("none") {
        return Err(anyhow!(
            "hardening_profile=none is forbidden for high-risk island capability: {}",
            capability
        ));
    }
    if !resolve_seccomp_profile(step) {
        return Err(anyhow!(
            "missing seccomp profile for high-risk island capability: {}",
            capability
        ));
    }
    if !resolve_syscall_constraints(step) {
        return Err(anyhow!(
            "missing syscall constraint (syscall_policy_ref or syscall_allowlist) for high-risk island capability: {}",
            capability
        ));
    }
    if !resolve_namespace_isolation(step) {
        return Err(anyhow!(
            "missing namespace isolation for high-risk island capability: {}",
            capability
        ));
    }
    if !resolve_cgroup_isolation(step) {
        return Err(anyhow!(
            "missing cgroup isolation for high-risk island capability: {}",
            capability
        ));
    }
    if !resolve_fs_isolation(step) {
        return Err(anyhow!(
            "missing fs isolation for high-risk island capability: {}",
            capability
        ));
    }
    if !resolve_network_isolation(step) {
        return Err(anyhow!(
            "missing network isolation for high-risk island capability: {}",
            capability
        ));
    }

    let isolation_backend = resolve_isolation_backend(step).ok_or_else(|| {
        anyhow!(
            "missing isolation_backend for high-risk island capability: {}",
            capability
        )
    })?;
    ensure_isolation_executor_available(step, &isolation_backend)?;

    let step_runtime_ms = resolve_step_runtime_ms(step).ok_or_else(|| {
        anyhow!(
            "missing step_max_runtime_ms for high-risk island capability: {}",
            capability
        )
    })?;
    let step_memory_mb = resolve_step_memory_mb(step).ok_or_else(|| {
        anyhow!(
            "missing step_max_memory_mb for high-risk island capability: {}",
            capability
        )
    })?;
    let step_cpu_units = resolve_step_cpu_units(step).ok_or_else(|| {
        anyhow!(
            "missing step_max_cpu_units for high-risk island capability: {}",
            capability
        )
    })?;

    if step_runtime_ms == 0 || step_memory_mb == 0 || step_cpu_units == 0 {
        return Err(anyhow!(
            "step resource constraints must be > 0 for high-risk island capability: {}",
            capability
        ));
    }

    let max_runtime_ms = constraints.max_runtime_ms.max(1);
    let max_memory_mb = constraints.max_memory_mb.max(1);
    let max_cpu_units = constraints.max_cpu_units.max(1);

    if step_runtime_ms > max_runtime_ms {
        return Err(anyhow!(
            "step_max_runtime_ms={} exceeds request max_runtime_ms={} for capability: {}",
            step_runtime_ms,
            max_runtime_ms,
            capability
        ));
    }
    if step_memory_mb > max_memory_mb {
        return Err(anyhow!(
            "step_max_memory_mb={} exceeds request max_memory_mb={} for capability: {}",
            step_memory_mb,
            max_memory_mb,
            capability
        ));
    }
    if step_cpu_units > max_cpu_units {
        return Err(anyhow!(
            "step_max_cpu_units={} exceeds request max_cpu_units={} for capability: {}",
            step_cpu_units,
            max_cpu_units,
            capability
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::enforce_runtime_island_hardening;
    use crate::ir::{
        ConstraintSet, ExecutionStep, FailurePolicy, HardeningPolicy, RuntimeIsland, StepAction,
    };

    fn step(cap: &str) -> ExecutionStep {
        ExecutionStep {
            step_id: "s1".to_string(),
            action: StepAction::CapabilityCall,
            capability_ref: Some(cap.to_string()),
            runtime_island: RuntimeIsland::Trusted,
            hardening: HardeningPolicy::default(),
            input: "hello".to_string(),
            dependencies: vec![],
            local_constraints: HashMap::new(),
            failure_policy: FailurePolicy::default(),
        }
    }

    fn request_constraints() -> ConstraintSet {
        ConstraintSet {
            max_runtime_ms: 30_000,
            max_cpu_units: 4,
            max_memory_mb: 256,
            policy_refs: vec!["sha256:policy-v1".to_string()],
        }
    }

    #[test]
    fn trusted_path_does_not_require_hardening() {
        let s = step("llm.echo");
        let c = request_constraints();
        assert!(enforce_runtime_island_hardening(&s, &c).is_ok());
    }

    #[test]
    fn high_risk_requires_hardening_profile() {
        let mut s = step("wasm.exec");
        s.runtime_island = RuntimeIsland::Wasm;
        s.hardening.max_runtime_ms = Some(1_000);
        s.hardening.max_memory_mb = Some(128);
        s.hardening.max_cpu_units = Some(1);
        s.hardening.syscall_policy_ref = Some("syscall:min".to_string());
        let c = request_constraints();
        let err = enforce_runtime_island_hardening(&s, &c).expect_err("must require profile");
        assert!(err.to_string().contains("hardening_profile"));
    }

    #[test]
    fn high_risk_requires_syscall_constraints() {
        let mut s = step("ffi.call");
        s.runtime_island = RuntimeIsland::Ffi;
        s.hardening.hardening_profile = Some("hardened-v1".to_string());
        s.hardening.seccomp_profile_ref = Some("seccomp:v1".to_string());
        s.hardening.enforce_namespace_isolation = true;
        s.hardening.enforce_cgroup_isolation = true;
        s.hardening.enforce_fs_isolation = true;
        s.hardening.enforce_network_isolation = true;
        s.hardening.max_runtime_ms = Some(1_000);
        s.hardening.max_memory_mb = Some(128);
        s.hardening.max_cpu_units = Some(1);
        let c = request_constraints();
        let err =
            enforce_runtime_island_hardening(&s, &c).expect_err("must require syscall policy");
        assert!(err.to_string().contains("syscall"));
    }

    #[test]
    fn high_risk_requires_isolation_constraints() {
        let mut s = step("wasm.exec");
        s.runtime_island = RuntimeIsland::Wasm;
        s.hardening.hardening_profile = Some("hardened-v1".to_string());
        s.hardening.seccomp_profile_ref = Some("seccomp:v1".to_string());
        s.hardening.syscall_policy_ref = Some("syscall:v1".to_string());
        s.hardening.max_runtime_ms = Some(1_000);
        s.hardening.max_memory_mb = Some(128);
        s.hardening.max_cpu_units = Some(1);
        let c = request_constraints();
        let err = enforce_runtime_island_hardening(&s, &c)
            .expect_err("must require explicit isolation toggles");
        assert!(err.to_string().contains("namespace isolation"));
    }

    #[test]
    fn high_risk_step_resources_must_be_bounded_by_request() {
        let mut s = step("plugin.exec");
        s.runtime_island = RuntimeIsland::Plugin;
        s.hardening.hardening_profile = Some("hardened-v1".to_string());
        s.hardening.seccomp_profile_ref = Some("seccomp:v1".to_string());
        s.hardening.syscall_policy_ref = Some("syscall:plugin".to_string());
        s.hardening.enforce_namespace_isolation = true;
        s.hardening.enforce_cgroup_isolation = true;
        s.hardening.enforce_fs_isolation = true;
        s.hardening.enforce_network_isolation = true;
        s.hardening.max_runtime_ms = Some(50_000);
        s.hardening.max_memory_mb = Some(128);
        s.hardening.max_cpu_units = Some(1);
        s.local_constraints
            .insert("isolation_backend".to_string(), "auto".to_string());
        let c = request_constraints();
        let err =
            enforce_runtime_island_hardening(&s, &c).expect_err("must enforce resource bounds");
        assert!(err.to_string().contains("step_max_runtime_ms"));
    }

    #[test]
    fn high_risk_requires_isolation_backend() {
        let mut s = step("wasm.exec");
        s.runtime_island = RuntimeIsland::Wasm;
        s.hardening.hardening_profile = Some("hardened-v1".to_string());
        s.hardening.seccomp_profile_ref = Some("seccomp:v1".to_string());
        s.hardening.syscall_allowlist = vec!["read".to_string(), "write".to_string()];
        s.hardening.enforce_namespace_isolation = true;
        s.hardening.enforce_cgroup_isolation = true;
        s.hardening.enforce_fs_isolation = true;
        s.hardening.enforce_network_isolation = true;
        s.hardening.max_runtime_ms = Some(10_000);
        s.hardening.max_memory_mb = Some(128);
        s.hardening.max_cpu_units = Some(2);
        let c = request_constraints();
        let err = enforce_runtime_island_hardening(&s, &c).expect_err("must require isolation backend");
        assert!(err.to_string().contains("missing isolation_backend"));
    }
    #[test]
    fn high_risk_with_hardening_passes() {
        let mut s = step("wasm.exec");
        s.runtime_island = RuntimeIsland::Wasm;
        s.hardening.hardening_profile = Some("hardened-v1".to_string());
        s.hardening.seccomp_profile_ref = Some("seccomp:v1".to_string());
        s.hardening.syscall_allowlist = vec!["read".to_string(), "write".to_string()];
        s.hardening.enforce_namespace_isolation = true;
        s.hardening.enforce_cgroup_isolation = true;
        s.hardening.enforce_fs_isolation = true;
        s.hardening.enforce_network_isolation = true;
        s.hardening.max_runtime_ms = Some(10_000);
        s.hardening.max_memory_mb = Some(128);
        s.hardening.max_cpu_units = Some(2);
        s.local_constraints
            .insert("isolation_backend".to_string(), "auto".to_string());
        let c = request_constraints();
        assert!(enforce_runtime_island_hardening(&s, &c).is_ok());
    }
}
