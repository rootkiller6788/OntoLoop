use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use trustkernel::ir::{
    ConstraintSet as TrustedConstraintSet, ExecutionPlan as TrustedExecutionPlan,
    ExecutionStep as TrustedExecutionStep, FailurePolicy, HardeningPolicy, IdentityContext, Intent,
    ReproducibleClosure, RuntimeIsland, StepAction, SupplyChainEnvelope, TraceContext,
    TrustedExecutionRequest, VerificationSpec,
};
use trustkernel::ledger::EvidenceRecord;
use trustkernel::trust::{CryptoService, Sha256CryptoService};

use super::attestation_verifier::{AttestationVerifierRegistry, AttestationVerifyInput};
use crate::config::AttestationBackend;
use crate::contracts::types::{
    AttestationEvidence, AttestationPlatform, AttestationPolicy, QuoteBundle, TaskEnvelope,
    TrustExecutionPlan, VerifierVerdict,
};

#[derive(Debug, Clone)]
pub struct TrustedAdmissionContext {
    pub request: TrustedExecutionRequest,
    pub trust_level: String,
    pub attestation_required: bool,
    pub attestation_backend_hint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttestationReport {
    pub backend: String,
    pub verified: bool,
    pub reference: String,
    pub details: serde_json::Value,
    pub evidence: AttestationEvidence,
    pub verdict: VerifierVerdict,
}
#[derive(Debug, Clone)]
struct AttestationChallenge {
    nonce: String,
    expected_binding: String,
    issued_at_ms: u64,
    expires_at_ms: u64,
    tenant_scope: String,
    session_scope: String,
    one_time_key: String,
}

#[derive(Debug, Default)]
struct ReplayProtectionCache {
    used_challenges: HashMap<String, u64>,
}

static REPLAY_PROTECTION_CACHE: OnceLock<Mutex<ReplayProtectionCache>> = OnceLock::new();
static CHALLENGE_COUNTER: AtomicU64 = AtomicU64::new(1);
const POLICY_REJECT_PREFIX: &str = "policy_reject::";

fn policy_reject_error(stage: &str, message: &str) -> anyhow::Error {
    anyhow::anyhow!("{}{stage}::{message}", POLICY_REJECT_PREFIX)
}

#[cfg(test)]
pub(crate) fn parse_policy_reject_error(error: &anyhow::Error) -> Option<(String, String)> {
    let msg = error.to_string();
    let raw = msg.strip_prefix(POLICY_REJECT_PREFIX)?;
    let mut parts = raw.splitn(2, "::");
    let stage = parts.next()?.to_string();
    let reason = parts.next().unwrap_or("attestation rejected").to_string();
    Some((stage, reason))
}

fn replay_cache() -> &'static Mutex<ReplayProtectionCache> {
    REPLAY_PROTECTION_CACHE.get_or_init(|| Mutex::new(ReplayProtectionCache::default()))
}

fn reserve_one_time_challenge(challenge: &AttestationChallenge) -> Result<()> {
    let now = current_time_ms();
    let mut guard = replay_cache()
        .lock()
        .map_err(|_| anyhow::anyhow!("attestation replay cache lock poisoned"))?;
    guard
        .used_challenges
        .retain(|_, expires_at| *expires_at > now);

    if guard.used_challenges.contains_key(&challenge.one_time_key) {
        return Err(policy_reject_error(
            "attestation_replay",
            "challenge replay detected for tenant/session scope",
        ));
    }

    guard
        .used_challenges
        .insert(challenge.one_time_key.clone(), challenge.expires_at_ms);
    Ok(())
}

#[allow(dead_code)]
pub fn envelope_to_trusted_request(
    envelope: &TaskEnvelope,
    preferred_model: Option<&str>,
    attestation_required: bool,
) -> Result<TrustedAdmissionContext> {
    envelope_to_trusted_request_with_supply_chain(
        envelope,
        preferred_model,
        attestation_required,
        None,
    )
}

pub fn envelope_to_trusted_request_with_supply_chain(
    envelope: &TaskEnvelope,
    preferred_model: Option<&str>,
    attestation_required: bool,
    supply_chain_override: Option<SupplyChainEnvelope>,
) -> Result<TrustedAdmissionContext> {
    let payload = if let Some(text) = envelope.payload.as_str() {
        text.to_string()
    } else {
        serde_json::to_string(&envelope.payload)?
    };

    let trust_plan = envelope
        .trust_plan
        .clone()
        .unwrap_or_else(|| default_trust_plan(envelope, attestation_required));

    let mut local_constraints = HashMap::new();
    local_constraints.insert(
        "max_tokens".to_string(),
        envelope.constraints.max_tokens.to_string(),
    );
    local_constraints.insert("rollout_gate".to_string(), trust_plan.rollout_gate.clone());
    if let Some(model) = preferred_model {
        local_constraints.insert("model_candidates".to_string(), model.to_string());
    }

    let step = TrustedExecutionStep {
        step_id: envelope.task_id.to_string(),
        action: StepAction::CapabilityCall,
        capability_ref: Some(envelope.capability_id.to_string()),
        runtime_island: RuntimeIsland::Trusted,
        hardening: HardeningPolicy {
            hardening_profile: Some(envelope.constraints.sandbox_profile.clone()),
            syscall_policy_ref: Some("autoloop:syscall:minimal".to_string()),
            syscall_allowlist: vec!["read".to_string(), "write".to_string()],
            seccomp_profile_ref: Some("autoloop:seccomp:default".to_string()),
            enforce_namespace_isolation: true,
            enforce_cgroup_isolation: true,
            enforce_fs_isolation: true,
            enforce_network_isolation: false,
            max_runtime_ms: Some(envelope.constraints.timeout_ms),
            max_memory_mb: Some(envelope.constraints.max_memory_mb),
            max_cpu_units: Some(envelope.constraints.max_cpu_percent.max(1) as u32),
        },
        input: payload,
        dependencies: vec![],
        local_constraints,
        failure_policy: FailurePolicy {
            retry_max: envelope.constraints.max_retries,
            allow_degrade: true,
            rollback_on_fail: true,
        },
    };

    let supply_chain = supply_chain_override.unwrap_or_else(|| SupplyChainEnvelope {
        executor_digest: format!("sha256:{}", local_digest_text("autoloop-runtime-executor")),
        verifier_digest: format!("sha256:{}", local_digest_text("autoloop-verifier")),
        policy_bundle_digest: format!("sha256:{}", local_digest_text(&envelope.identity.policy_id)),
        capability_package_digest: format!(
            "sha256:{}",
            local_digest_text(&envelope.capability_id.to_string())
        ),
        provenance_digest: format!(
            "sha256:{}",
            local_digest_text(&format!(
                "{}:{}:{}",
                envelope.session_id, envelope.trace_id, envelope.task_id
            ))
        ),
        signer_identity_ref: format!(
            "autoloop://tenant/{}/principal/{}",
            envelope.identity.tenant_id, envelope.identity.principal_id
        ),
        signer_oidc_issuer: "https://autoloop.local/issuer".to_string(),
        rekor_url: "https://rekor.sigstore.dev".to_string(),
        rekor_log_ref: format!("rekor://trace/{}", envelope.trace_id),
        rekor_inclusion_proof: format!(
            "sha256:{}",
            local_digest_text(&format!(
                "{}:{}:{}",
                envelope.identity.tenant_id, envelope.identity.principal_id, envelope.trace_id
            ))
        ),
        provenance_type: "autoloop-trust-plan".to_string(),
    });

    let closure = resolve_reproducible_closure(envelope, &trust_plan, &supply_chain);
    let request = TrustedExecutionRequest {
        execution_id: format!("{}:{}", envelope.trace_id, envelope.task_id),
        identity: IdentityContext {
            principal: envelope.identity.principal_id.clone(),
            tenant_id: envelope.identity.tenant_id.clone(),
            workspace: envelope.session_id.to_string(),
            signature: envelope.identity.lease_token.clone(),
        },
        intent: Intent {
            operation: envelope.capability_id.to_string(),
            payload: envelope.payload.to_string(),
        },
        plan: TrustedExecutionPlan {
            plan_id: format!("plan:{}", envelope.trace_id),
            version: 1,
            strategy: "single_step".into(),
            steps: vec![step],
        },
        constraints: TrustedConstraintSet {
            max_runtime_ms: envelope.constraints.timeout_ms,
            max_cpu_units: envelope.constraints.max_cpu_percent.max(1) as u32,
            max_memory_mb: envelope.constraints.max_memory_mb,
            policy_refs: if trust_plan.policy_refs.is_empty() {
                vec![envelope.identity.policy_id.clone()]
            } else {
                trust_plan.policy_refs.clone()
            },
        },
        verification_spec: VerificationSpec {
            verify_environment: trust_plan.verify_environment,
            verify_identity: trust_plan.verify_identity,
            validate_output: true,
            trust_level: trust_plan.trust_level.clone(),
        },
        supply_chain,
        closure,
        trace_context: TraceContext {
            trace_id: envelope.trace_id.to_string(),
            parent_execution_id: None,
            submitted_at_ms: current_time_ms() as u128,
        },
    };

    Ok(TrustedAdmissionContext {
        request,
        trust_level: trust_plan.trust_level,
        attestation_required: trust_plan.attestation_required,
        attestation_backend_hint: trust_plan.attestation_backend,
    })
}

pub async fn verify_attestation(
    required: bool,
    backend: AttestationBackend,
    backend_hint: &str,
    secret_env: &str,
    token_env: &str,
    quote_env: &str,
    cert_chain_env: &str,
    cert_subject_allowlist: &[String],
    remote_url: Option<&str>,
    policy: &AttestationPolicy,
    trusted: &TrustedExecutionRequest,
) -> Result<AttestationReport> {
    let registry = AttestationVerifierRegistry::with_defaults();
    let verifier = registry.resolve(&backend, backend_hint);
    let input = AttestationVerifyInput {
        required,
        secret_env,
        token_env,
        quote_env,
        cert_chain_env,
        cert_subject_allowlist,
        remote_url,
        policy,
        trusted,
        binding: super::attestation_verifier::AttestationRequestBinding {
            tenant_id: trusted.identity.tenant_id.clone(),
            policy_version: policy.version.clone(),
            request_nonce_required: policy.require_nonce,
            evidence_ttl_ms: policy.evidence_ttl_ms,
            execution_id: trusted.execution_id.clone(),
        },
    };
    verifier.verify(backend, backend_hint, &input).await
}

pub(crate) async fn verify_attestation_legacy(
    required: bool,
    backend: AttestationBackend,
    secret_env: &str,
    token_env: &str,
    quote_env: &str,
    cert_chain_env: &str,
    cert_subject_allowlist: &[String],
    remote_url: Option<&str>,
    policy: &AttestationPolicy,
    trusted: &TrustedExecutionRequest,
) -> Result<AttestationReport> {
    let backend_label = format!("{:?}", backend).to_ascii_lowercase();
    if !required {
        let evidence = AttestationEvidence {
            evidence_id: format!("evidence:{}:skipped", trusted.execution_id),
            backend: backend_label.clone(),
            quote_bundle: None,
            remote_report: None,
            digest: None,
            source_ref: Some("attestation-skipped".into()),
        };
        let verdict = VerifierVerdict {
            verified: true,
            reason: "attestation not required".into(),
            policy_version: policy.version.clone(),
            evidence_id: Some(evidence.evidence_id.clone()),
            verifier_name: "autoloop-attestation-gate".into(),
            min_tcb_passed: true,
            freshness_passed: true,
            tenant_binding_passed: true,
            nonce_present: true,
        };
        return Ok(AttestationReport {
            backend: backend_label,
            verified: true,
            reference: "attestation-skipped".into(),
            details: serde_json::json!({"required": false, "policy_version": policy.version}),
            evidence,
            verdict,
        });
    }

    let challenge = build_attestation_challenge(&backend, policy, trusted);
    if policy.require_nonce && challenge.nonce.trim().is_empty() {
        return Err(policy_reject_error(
            "attestation_nonce",
            "attestation policy requires nonce but challenge nonce is missing",
        ));
    }
    let now = current_time_ms();
    if now > challenge.expires_at_ms {
        return Err(policy_reject_error(
            "attestation_ttl",
            "attestation challenge expired before verification",
        ));
    }
    reserve_one_time_challenge(&challenge)?;

    match backend {
        AttestationBackend::Env => {
            let value = std::env::var(secret_env).unwrap_or_default();
            if value.trim().is_empty() {
                bail!("attestation required but env `{}` is not set", secret_env);
            }
            let evidence = AttestationEvidence {
                evidence_id: format!("evidence:{}:env", trusted.execution_id),
                backend: "env".into(),
                quote_bundle: None,
                remote_report: None,
                digest: Some(local_digest_text(&value)),
                source_ref: Some(format!("env:{}", secret_env)),
            };
            let verdict = VerifierVerdict {
                verified: true,
                reason: "env attestation secret present".into(),
                policy_version: policy.version.clone(),
                evidence_id: Some(evidence.evidence_id.clone()),
                verifier_name: "env-attestation-verifier".into(),
                min_tcb_passed: true,
                freshness_passed: true,
                tenant_binding_passed: true,
                nonce_present: !challenge.nonce.trim().is_empty(),
            };
            Ok(AttestationReport {
                backend: "env".into(),
                verified: true,
                reference: format!("env:{}", secret_env),
                details: serde_json::json!({
                    "execution_id": trusted.execution_id,
                    "tenant_id": trusted.identity.tenant_id,
                    "principal": trusted.identity.principal,
                    "policy_version": policy.version,
                }),
                evidence,
                verdict,
            })
        }
        AttestationBackend::Remote => {
            verify_remote_attestation(
                remote_url,
                token_env,
                &challenge,
                policy,
                serde_json::json!({
                    "mode": "remote",
                    "execution_id": trusted.execution_id,
                    "trace_id": trusted.trace_context.trace_id,
                    "tenant_id": trusted.identity.tenant_id,
                    "principal": trusted.identity.principal,
                    "workspace": trusted.identity.workspace,
                    "trust_level": trusted.verification_spec.trust_level,
                    "policy_refs": trusted.constraints.policy_refs,
                }),
            )
            .await
        }
        AttestationBackend::HardwareQuote => {
            let quote = std::env::var(quote_env).unwrap_or_default();
            if quote.trim().is_empty() {
                bail!("hardware quote attestation requires env `{}`", quote_env);
            }
            if quote.len() < 24 {
                bail!("hardware quote is too short to be valid");
            }
            if !quote.contains(&trusted.identity.tenant_id) {
                bail!("hardware quote does not match expected tenant context");
            }
            if policy.require_nonce && !quote.contains(&challenge.nonce) {
                bail!("hardware quote missing attestation nonce");
            }
            let now = current_time_ms();
            let actual_tcb = std::env::var("AUTOLOOP_ATTESTATION_TCB_VERSION")
                .unwrap_or_else(|_| "1.0.0".into());
            let min_tcb_passed = tcb_meets_minimum(&actual_tcb, &policy.min_tcb_version);
            if !min_tcb_passed {
                bail!("hardware quote tcb version below policy minimum");
            }
            let quote_bundle = QuoteBundle {
                platform: AttestationPlatform::Sgx,
                quote: quote.clone(),
                cert_chain: None,
                endorsements: Vec::new(),
                tcb_version: actual_tcb,
                issued_at_ms: now,
                expires_at_ms: now.saturating_add(policy.evidence_ttl_ms),
                tenant_binding: Some(trusted.identity.tenant_id.clone()),
                nonce: Some(challenge.nonce.clone()),
            };
            if policy.require_tenant_binding
                && quote_bundle.tenant_binding.as_deref()
                    != Some(trusted.identity.tenant_id.as_str())
            {
                bail!("hardware quote tenant binding mismatch");
            }
            if let Some(url) = remote_url {
                verify_remote_attestation(
                    Some(url),
                    token_env,
                    &challenge,
                    policy,
                    serde_json::json!({
                        "mode": "hardware_quote",
                        "quote": quote,
                        "execution_id": trusted.execution_id,
                        "trace_id": trusted.trace_context.trace_id,
                        "tenant_id": trusted.identity.tenant_id,
                        "principal": trusted.identity.principal,
                        "workspace": trusted.identity.workspace,
                        "quote_bundle": quote_bundle,
                    }),
                )
                .await
            } else {
                let evidence = AttestationEvidence {
                    evidence_id: format!("evidence:{}:hardware_quote", trusted.execution_id),
                    backend: "hardware_quote".into(),
                    quote_bundle: Some(quote_bundle.clone()),
                    remote_report: None,
                    digest: Some(local_digest_text(&quote_bundle.quote)),
                    source_ref: Some(format!("env:{}", quote_env)),
                };
                let verdict = build_verifier_verdict(
                    true,
                    "hardware quote verified locally",
                    policy,
                    Some(evidence.evidence_id.clone()),
                    "hardware-quote-local-verifier",
                    quote_bundle.issued_at_ms,
                    quote_bundle.expires_at_ms,
                    &quote_bundle.tcb_version,
                    quote_bundle.tenant_binding.as_deref(),
                    quote_bundle.nonce.as_deref(),
                    trusted,
                )?;
                Ok(AttestationReport {
                    backend: "hardware_quote".into(),
                    verified: true,
                    reference: format!("quote:{}", trusted.execution_id),
                    details: serde_json::json!({
                        "quote_env": quote_env,
                        "policy_version": policy.version,
                    }),
                    evidence,
                    verdict,
                })
            }
        }
        AttestationBackend::CertificateChain => {
            let chain = std::env::var(cert_chain_env).unwrap_or_default();
            if chain.trim().is_empty() {
                bail!("certificate attestation requires env `{}`", cert_chain_env);
            }
            let cert_count = chain.matches("-----END CERTIFICATE-----").count();
            if cert_count == 0 {
                bail!("certificate chain env does not contain a valid PEM chain");
            }
            if policy.strict {
                if cert_subject_allowlist.is_empty() {
                    bail!("strict attestation policy requires non-empty cert subject allowlist");
                }
                if remote_url.is_none() {
                    bail!("strict attestation policy requires attestation_remote_url");
                }
                let expected_proof =
                    std::env::var("AUTOLOOP_ATTESTATION_CERT_PROOF").unwrap_or_default();
                if expected_proof.trim().is_empty() {
                    bail!(
                        "strict attestation policy requires env `AUTOLOOP_ATTESTATION_CERT_PROOF`"
                    );
                }
                let actual_proof = derive_certificate_proof(&chain, trusted, &challenge.nonce);
                if expected_proof.trim() != actual_proof {
                    bail!("strict certificate attestation proof mismatch");
                }
            }
            if !cert_subject_allowlist.is_empty() {
                let chain_lower = chain.to_ascii_lowercase();
                let matched = cert_subject_allowlist
                    .iter()
                    .any(|subject| chain_lower.contains(&subject.to_ascii_lowercase()));
                if !matched {
                    bail!("certificate subject not present in allowlist");
                }
            }
            let now = current_time_ms();
            let actual_tcb = std::env::var("AUTOLOOP_ATTESTATION_TCB_VERSION")
                .unwrap_or_else(|_| "1.0.0".into());
            if !tcb_meets_minimum(&actual_tcb, &policy.min_tcb_version) {
                bail!("certificate attestation tcb version below policy minimum");
            }
            let quote_bundle = QuoteBundle {
                platform: AttestationPlatform::Generic,
                quote: format!("cert-chain:{}", local_digest_text(&chain)),
                cert_chain: Some(chain.clone()),
                endorsements: Vec::new(),
                tcb_version: actual_tcb,
                issued_at_ms: now,
                expires_at_ms: now.saturating_add(policy.evidence_ttl_ms),
                tenant_binding: Some(trusted.identity.tenant_id.clone()),
                nonce: Some(challenge.nonce.clone()),
            };
            if let Some(url) = remote_url {
                verify_remote_attestation(
                    Some(url),
                    token_env,
                    &challenge,
                    policy,
                    serde_json::json!({
                        "mode": "certificate_chain",
                        "certificate_chain": chain,
                        "execution_id": trusted.execution_id,
                        "trace_id": trusted.trace_context.trace_id,
                        "tenant_id": trusted.identity.tenant_id,
                        "principal": trusted.identity.principal,
                        "subject_allowlist": cert_subject_allowlist,
                        "quote_bundle": quote_bundle,
                    }),
                )
                .await
            } else {
                let evidence = AttestationEvidence {
                    evidence_id: format!("evidence:{}:certificate_chain", trusted.execution_id),
                    backend: "certificate_chain".into(),
                    quote_bundle: Some(quote_bundle.clone()),
                    remote_report: None,
                    digest: Some(local_digest_text(&chain)),
                    source_ref: Some(format!("env:{}", cert_chain_env)),
                };
                let verdict = build_verifier_verdict(
                    true,
                    "certificate chain verified locally",
                    policy,
                    Some(evidence.evidence_id.clone()),
                    "certificate-chain-local-verifier",
                    quote_bundle.issued_at_ms,
                    quote_bundle.expires_at_ms,
                    &quote_bundle.tcb_version,
                    quote_bundle.tenant_binding.as_deref(),
                    quote_bundle.nonce.as_deref(),
                    trusted,
                )?;
                Ok(AttestationReport {
                    backend: "certificate_chain".into(),
                    verified: true,
                    reference: format!("cert:{}", trusted.execution_id),
                    details: serde_json::json!({
                        "cert_chain_env": cert_chain_env,
                        "certificate_count": cert_count,
                        "subject_allowlist_applied": !cert_subject_allowlist.is_empty(),
                        "policy_version": policy.version,
                    }),
                    evidence,
                    verdict,
                })
            }
        }
    }
}

fn build_verifier_verdict(
    verified: bool,
    reason: &str,
    policy: &AttestationPolicy,
    evidence_id: Option<String>,
    verifier_name: &str,
    issued_at_ms: u64,
    expires_at_ms: u64,
    actual_tcb_version: &str,
    tenant_binding: Option<&str>,
    nonce: Option<&str>,
    trusted: &TrustedExecutionRequest,
) -> Result<VerifierVerdict> {
    let now = current_time_ms();
    let freshness_passed = now >= issued_at_ms
        && now <= expires_at_ms
        && now.saturating_sub(issued_at_ms) <= policy.evidence_ttl_ms;
    let min_tcb_passed = tcb_meets_minimum(actual_tcb_version, &policy.min_tcb_version);
    let tenant_binding_passed = if policy.require_tenant_binding {
        tenant_binding == Some(trusted.identity.tenant_id.as_str())
    } else {
        true
    };
    let nonce_present = if policy.require_nonce {
        nonce.map(|value| !value.trim().is_empty()).unwrap_or(false)
    } else {
        true
    };

    if verified && !(freshness_passed && min_tcb_passed && tenant_binding_passed && nonce_present) {
        bail!("attestation verdict inconsistent with policy checks");
    }

    Ok(VerifierVerdict {
        verified,
        reason: reason.to_string(),
        policy_version: policy.version.clone(),
        evidence_id,
        verifier_name: verifier_name.to_string(),
        min_tcb_passed,
        freshness_passed,
        tenant_binding_passed,
        nonce_present,
    })
}

fn tcb_meets_minimum(actual: &str, minimum: &str) -> bool {
    let parse = |value: &str| {
        value
            .split('.')
            .map(|part| part.trim().parse::<u64>().unwrap_or(0))
            .collect::<Vec<_>>()
    };
    let mut a = parse(actual);
    let mut m = parse(minimum);
    let max_len = a.len().max(m.len());
    a.resize(max_len, 0);
    m.resize(max_len, 0);
    a >= m
}

fn local_digest_text(value: &str) -> String {
    let crypto = Sha256CryptoService;
    crypto.digest_hex(value)
}
fn ensure_sha256(value: impl Into<String>) -> String {
    let value = value.into();
    if value.starts_with("sha256:") {
        value
    } else {
        format!("sha256:{}", value)
    }
}

fn first_existing_path(candidates: &[PathBuf]) -> Option<PathBuf> {
    candidates.iter().find(|path| path.exists()).cloned()
}

fn digest_file(path: &Path) -> Option<String> {
    std::fs::read(path)
        .ok()
        .map(|bytes| ensure_sha256(local_digest_text(&String::from_utf8_lossy(&bytes))))
}

fn resolve_workspace_root() -> PathBuf {
    if let Ok(root) = std::env::var("AUTOLOOP_WORKSPACE_ROOT") {
        let trimmed = root.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn closure_store_paths_from_system(workspace_root: &Path) -> Vec<String> {
    let mut paths = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        paths.push(exe.to_string_lossy().to_string());
    }
    let cargo_toml = workspace_root.join("Cargo.toml");
    if cargo_toml.exists() {
        paths.push(cargo_toml.to_string_lossy().to_string());
    }
    let cargo_lock = workspace_root.join("Cargo.lock");
    if cargo_lock.exists() {
        paths.push(cargo_lock.to_string_lossy().to_string());
    }
    if let Ok(extra) = std::env::var("AUTOLOOP_CLOSURE_STORE_PATHS") {
        for item in extra
            .split(';')
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            paths.push(item.to_string());
        }
    }
    paths.sort();
    paths.dedup();
    paths
}

fn derive_runtime_closure_hash(store_paths: &[String]) -> String {
    let mut fragments = Vec::new();
    for path in store_paths {
        let pb = PathBuf::from(path);
        let meta = std::fs::metadata(&pb).ok();
        let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
        let modified = meta
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        fragments.push(format!("{}:{}:{}", path, size, modified));
    }
    ensure_sha256(local_digest_text(&fragments.join("|")))
}

fn resolve_reproducible_closure(
    envelope: &TaskEnvelope,
    trust_plan: &TrustExecutionPlan,
    supply_chain: &SupplyChainEnvelope,
) -> ReproducibleClosure {
    let workspace_root = resolve_workspace_root();
    let flake_ref = std::env::var("AUTOLOOP_CLOSURE_FLAKE_REF")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| format!("git+file://{}", workspace_root.to_string_lossy()));

    let flake_lock_candidates = [
        std::env::var("AUTOLOOP_FLAKE_LOCK_PATH")
            .ok()
            .map(PathBuf::from)
            .unwrap_or_else(|| workspace_root.join("flake.lock")),
        workspace_root.join("Cargo.lock"),
    ];
    let flake_lock_digest = first_existing_path(&flake_lock_candidates)
        .as_deref()
        .and_then(digest_file)
        .unwrap_or_else(|| ensure_sha256(local_digest_text("autoloop-fallback-flake-lock")));

    let derivation_candidates = [
        std::env::var("AUTOLOOP_DERIVATION_PATH")
            .ok()
            .map(PathBuf::from)
            .unwrap_or_else(|| workspace_root.join("noumenon.derivation")),
        workspace_root.join("Cargo.toml"),
    ];
    let derivation_digest = first_existing_path(&derivation_candidates)
        .as_deref()
        .and_then(digest_file)
        .unwrap_or_else(|| ensure_sha256(local_digest_text("autoloop-fallback-derivation")));

    let store_paths = closure_store_paths_from_system(&workspace_root);
    let runtime_closure_hash = derive_runtime_closure_hash(&store_paths);

    let generation_id = std::env::var("AUTOLOOP_CLOSURE_GENERATION")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| format!("gen-{}", envelope.session_id));
    let profile_id = std::env::var("AUTOLOOP_CLOSURE_PROFILE")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "autoloop-default".to_string());

    ReproducibleClosure {
        flake_ref,
        flake_lock_digest,
        derivation_digest,
        store_paths,
        runtime_closure_hash,
        generation_id,
        profile_id,
        config_digest: ensure_sha256(local_digest_text(&format!(
            "{}:{}:{}:{}",
            trust_plan.trust_level,
            trust_plan.rollout_gate,
            envelope.identity.policy_id,
            supply_chain.policy_bundle_digest
        ))),
    }
}
fn build_attestation_challenge(
    backend: &AttestationBackend,
    policy: &AttestationPolicy,
    trusted: &TrustedExecutionRequest,
) -> AttestationChallenge {
    let crypto = Sha256CryptoService;
    let mode = format!("{:?}", backend).to_ascii_lowercase();
    let issued_at_ms = current_time_ms();
    let expires_at_ms = issued_at_ms.saturating_add(policy.evidence_ttl_ms);
    let nonce = std::env::var("AUTOLOOP_ATTESTATION_NONCE")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| {
            let seq = CHALLENGE_COUNTER.fetch_add(1, Ordering::Relaxed);
            crypto.digest_hex(&format!(
                "{}:{}:{}:{}:{}:{}",
                trusted.execution_id,
                trusted.trace_context.trace_id,
                trusted.identity.tenant_id,
                trusted.identity.workspace,
                issued_at_ms,
                seq
            ))
        });
    let expected_binding = crypto.digest_hex(&format!(
        "{}:{}:{}:{}:{}:{}:{}",
        mode,
        trusted.execution_id,
        trusted.trace_context.trace_id,
        trusted.identity.tenant_id,
        trusted.identity.workspace,
        trusted.identity.principal,
        nonce
    ));
    let one_time_key = crypto.digest_hex(&format!(
        "{}:{}:{}:{}",
        trusted.identity.tenant_id, trusted.identity.workspace, trusted.execution_id, nonce
    ));
    AttestationChallenge {
        nonce,
        expected_binding,
        issued_at_ms,
        expires_at_ms,
        tenant_scope: trusted.identity.tenant_id.clone(),
        session_scope: trusted.identity.workspace.clone(),
        one_time_key,
    }
}
async fn verify_remote_attestation(
    remote_url: Option<&str>,
    token_env: &str,
    challenge: &AttestationChallenge,
    policy: &AttestationPolicy,
    mut payload: serde_json::Value,
) -> Result<AttestationReport> {
    let url = remote_url
        .ok_or_else(|| anyhow::anyhow!("attestation backend requires attestation_remote_url"))?;
    let token = std::env::var(token_env).unwrap_or_default();
    if token.trim().is_empty() {
        bail!("remote attestation token env `{}` is not set", token_env);
    }

    if let Some(map) = payload.as_object_mut() {
        map.insert(
            "challenge_nonce".to_string(),
            serde_json::Value::String(challenge.nonce.clone()),
        );
        map.insert(
            "challenge_binding".to_string(),
            serde_json::Value::String(challenge.expected_binding.clone()),
        );
        map.insert(
            "challenge_issued_at_ms".to_string(),
            serde_json::Value::Number(serde_json::Number::from(challenge.issued_at_ms)),
        );
        map.insert(
            "challenge_expires_at_ms".to_string(),
            serde_json::Value::Number(serde_json::Number::from(challenge.expires_at_ms)),
        );
        map.insert(
            "challenge_tenant_scope".to_string(),
            serde_json::Value::String(challenge.tenant_scope.clone()),
        );
        map.insert(
            "challenge_session_scope".to_string(),
            serde_json::Value::String(challenge.session_scope.clone()),
        );
        map.insert(
            "challenge_one_time_key".to_string(),
            serde_json::Value::String(challenge.one_time_key.clone()),
        );
        map.insert(
            "attestation_policy_version".to_string(),
            serde_json::Value::String(policy.version.clone()),
        );
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()?;
    let response = client
        .post(url)
        .bearer_auth(token)
        .json(&payload)
        .send()
        .await?;
    let status = response.status();
    let body: serde_json::Value = response
        .json()
        .await
        .unwrap_or_else(|_| serde_json::json!({}));
    if !status.is_success() {
        bail!("remote attestation failed with status {}", status);
    }

    let verified = body
        .get("verified")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if !verified {
        let reason = body
            .get("reason")
            .and_then(|v| v.as_str())
            .unwrap_or("remote verifier rejected attestation");
        bail!("{}", reason);
    }

    let remote_nonce = body
        .get("challenge_nonce")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    if remote_nonce != challenge.nonce {
        bail!("remote attestation challenge nonce mismatch");
    }
    let remote_binding = body
        .get("challenge_binding")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    if remote_binding != challenge.expected_binding {
        return Err(policy_reject_error(
            "attestation_binding",
            "remote attestation challenge binding mismatch",
        ));
    }

    let remote_challenge_exp = body
        .get("challenge_expires_at_ms")
        .and_then(|value| value.as_u64())
        .unwrap_or(challenge.expires_at_ms);
    if remote_challenge_exp != challenge.expires_at_ms {
        return Err(policy_reject_error(
            "attestation_ttl",
            "remote attestation challenge ttl mismatch",
        ));
    }
    if current_time_ms() > remote_challenge_exp {
        return Err(policy_reject_error(
            "attestation_ttl",
            "remote attestation challenge expired",
        ));
    }

    let verifier_nonce = body
        .get("verifier_nonce")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    if verifier_nonce.trim().is_empty() {
        return Err(policy_reject_error(
            "attestation_challenge",
            "remote verifier nonce missing",
        ));
    }
    let response_binding = body
        .get("challenge_response_binding")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    let expected_response_binding = derive_response_binding(
        &challenge.expected_binding,
        verifier_nonce,
        payload
            .get("execution_id")
            .and_then(|value| value.as_str())
            .unwrap_or("remote"),
        payload
            .get("tenant_id")
            .and_then(|value| value.as_str())
            .unwrap_or("unknown"),
    );
    if response_binding != expected_response_binding {
        return Err(policy_reject_error(
            "attestation_challenge",
            "remote challenge-response binding mismatch",
        ));
    }

    let now = current_time_ms();
    let mode = payload
        .get("mode")
        .and_then(|v| v.as_str())
        .unwrap_or("remote");
    let tenant_id = payload
        .get("tenant_id")
        .and_then(|value| value.as_str())
        .unwrap_or("unknown");
    let actual_tcb = body
        .get("tcb_version")
        .and_then(|value| value.as_str())
        .unwrap_or("1.0.0");
    let issued_at_ms = body
        .get("issued_at_ms")
        .and_then(|value| value.as_u64())
        .unwrap_or(now);
    let expires_at_ms = body
        .get("expires_at_ms")
        .and_then(|value| value.as_u64())
        .unwrap_or(now.saturating_add(policy.evidence_ttl_ms));

    let platform = match body
        .get("platform")
        .and_then(|value| value.as_str())
        .unwrap_or(mode)
        .to_ascii_lowercase()
        .as_str()
    {
        "sgx" => AttestationPlatform::Sgx,
        "sev" | "sev-snp" | "snp" => AttestationPlatform::SevSnp,
        "tpm" => AttestationPlatform::Tpm,
        _ => AttestationPlatform::Generic,
    };
    let quote_bundle = QuoteBundle {
        platform,
        quote: body
            .get("quote")
            .and_then(|value| value.as_str())
            .unwrap_or("remote_quote")
            .to_string(),
        cert_chain: body
            .get("certificate_chain")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        endorsements: body
            .get("endorsements")
            .and_then(|value| value.as_array())
            .map(|values| {
                values
                    .iter()
                    .filter_map(|value| value.as_str().map(str::to_string))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default(),
        tcb_version: actual_tcb.to_string(),
        issued_at_ms,
        expires_at_ms,
        tenant_binding: Some(tenant_id.to_string()),
        nonce: Some(remote_nonce.to_string()),
    };

    let evidence = AttestationEvidence {
        evidence_id: body
            .get("evidence_id")
            .and_then(|value| value.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| {
                format!(
                    "evidence:remote:{}",
                    local_digest_text(&challenge.expected_binding)
                )
            }),
        backend: mode.to_string(),
        quote_bundle: Some(quote_bundle.clone()),
        remote_report: Some(body.clone()),
        digest: Some(local_digest_text(&body.to_string())),
        source_ref: Some(url.to_string()),
    };

    let trusted_stub = TrustedExecutionRequest {
        execution_id: payload
            .get("execution_id")
            .and_then(|value| value.as_str())
            .unwrap_or("remote")
            .to_string(),
        identity: IdentityContext {
            principal: payload
                .get("principal")
                .and_then(|value| value.as_str())
                .unwrap_or("remote")
                .to_string(),
            tenant_id: tenant_id.to_string(),
            workspace: payload
                .get("workspace")
                .and_then(|value| value.as_str())
                .unwrap_or("remote")
                .to_string(),
            signature: "remote-attested".to_string(),
        },
        intent: Intent {
            operation: "remote_attestation".to_string(),
            payload: serde_json::to_string(&payload).unwrap_or_default(),
        },
        plan: TrustedExecutionPlan {
            plan_id: "remote-attestation-plan".to_string(),
            version: 1,
            strategy: "verify".to_string(),
            steps: Vec::new(),
        },
        constraints: TrustedConstraintSet {
            max_runtime_ms: 0,
            max_cpu_units: 1,
            max_memory_mb: 1,
            policy_refs: Vec::new(),
        },
        verification_spec: VerificationSpec {
            verify_environment: true,
            verify_identity: true,
            validate_output: true,
            trust_level: "strict".to_string(),
        },
        supply_chain: SupplyChainEnvelope {
            executor_digest: format!(
                "sha256:{}",
                local_digest_text("remote-attestation-executor")
            ),
            verifier_digest: format!(
                "sha256:{}",
                local_digest_text("remote-attestation-verifier")
            ),
            policy_bundle_digest: format!("sha256:{}", local_digest_text(&policy.version)),
            capability_package_digest: format!(
                "sha256:{}",
                local_digest_text("remote-attestation-capability")
            ),
            provenance_digest: format!("sha256:{}", local_digest_text(&body.to_string())),
            signer_identity_ref: "remote://attestation-verifier".to_string(),
            signer_oidc_issuer: "remote-attestation".to_string(),
            rekor_url: "local://rekor".to_string(),
            rekor_log_ref: format!("rekor://attestation/{}", tenant_id),
            rekor_inclusion_proof: "local-proof".to_string(),
            provenance_type: "remote-attestation-report".to_string(),
        },
        closure: {
            let workspace_root = resolve_workspace_root();
            let store_paths = closure_store_paths_from_system(&workspace_root);
            ReproducibleClosure {
                flake_ref: std::env::var("AUTOLOOP_CLOSURE_FLAKE_REF")
                    .ok()
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or_else(|| format!("git+file://{}", workspace_root.to_string_lossy())),
                flake_lock_digest: digest_file(&workspace_root.join("Cargo.lock")).unwrap_or_else(
                    || ensure_sha256(local_digest_text("remote-attestation-flake")),
                ),
                derivation_digest: digest_file(&workspace_root.join("Cargo.toml")).unwrap_or_else(
                    || ensure_sha256(local_digest_text("remote-attestation-derivation")),
                ),
                store_paths: store_paths.clone(),
                runtime_closure_hash: derive_runtime_closure_hash(&store_paths),
                generation_id: std::env::var("AUTOLOOP_CLOSURE_GENERATION")
                    .ok()
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or_else(|| "gen-remote-attestation".to_string()),
                profile_id: std::env::var("AUTOLOOP_CLOSURE_PROFILE")
                    .ok()
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or_else(|| "autoloop-remote-attestation".to_string()),
                config_digest: ensure_sha256(local_digest_text("remote-attestation-config")),
            }
        },
        trace_context: TraceContext {
            trace_id: payload
                .get("trace_id")
                .and_then(|value| value.as_str())
                .unwrap_or("remote")
                .to_string(),
            parent_execution_id: None,
            submitted_at_ms: now as u128,
        },
    };

    let verdict = build_verifier_verdict(
        true,
        "remote verifier accepted attestation",
        policy,
        Some(evidence.evidence_id.clone()),
        "remote-attestation-verifier",
        quote_bundle.issued_at_ms,
        quote_bundle.expires_at_ms,
        &quote_bundle.tcb_version,
        quote_bundle.tenant_binding.as_deref(),
        quote_bundle.nonce.as_deref(),
        &trusted_stub,
    )?;

    let reference = body
        .get("attestation_id")
        .and_then(|v| v.as_str())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("{}:remote", mode));

    Ok(AttestationReport {
        backend: mode.to_string(),
        verified,
        reference,
        details: body,
        evidence,
        verdict,
    })
}
fn derive_response_binding(
    expected_binding: &str,
    verifier_nonce: &str,
    execution_id: &str,
    tenant_id: &str,
) -> String {
    let crypto = Sha256CryptoService;
    crypto.digest_hex(&format!(
        "{}:{}:{}:{}",
        expected_binding, verifier_nonce, execution_id, tenant_id
    ))
}

fn derive_certificate_proof(chain: &str, trusted: &TrustedExecutionRequest, nonce: &str) -> String {
    let crypto = Sha256CryptoService;
    crypto.digest_hex(&format!(
        "{}:{}:{}:{}:{}",
        chain, trusted.execution_id, trusted.identity.tenant_id, trusted.identity.principal, nonce
    ))
}

pub fn build_hash_chain_records(
    trusted: &TrustedExecutionRequest,
    output_content: &str,
) -> Vec<EvidenceRecord> {
    let crypto = Sha256CryptoService;
    let mut prev_hash = "genesis".to_string();
    let stream_id = format!("trust:{}", trusted.execution_id);
    let now = current_time_ms() as u128;

    trusted
        .plan
        .steps
        .iter()
        .enumerate()
        .map(|(idx, step)| {
            let input_digest = crypto.digest_hex(&step.input);
            let output_digest = crypto.digest_hex(output_content);
            let record_hash = crypto.digest_hex(&format!(
                "{}:{}:{}:{}:{}",
                trusted.execution_id, step.step_id, input_digest, output_digest, prev_hash
            ));
            let record = EvidenceRecord {
                record_id: format!("{}:{}:{}", trusted.execution_id, step.step_id, idx),
                stream_id: stream_id.clone(),
                execution_id: trusted.execution_id.clone(),
                step_id: step.step_id.clone(),
                event_type: "STEP_EXECUTED".to_string(),
                payload_digest: output_digest,
                prev_hash: prev_hash.clone(),
                record_hash: record_hash.clone(),
                timestamp_ms: now + idx as u128,
            };
            prev_hash = record_hash;
            record
        })
        .collect()
}

pub fn build_hash_chain_evidence(
    trusted: &TrustedExecutionRequest,
    output_content: &str,
) -> Vec<serde_json::Value> {
    build_hash_chain_records(trusted, output_content)
        .into_iter()
        .map(|record| {
            serde_json::json!({
                "step_id": record.step_id,
                "event_type": record.event_type,
                "payload_digest": record.payload_digest,
                "prev_hash": record.prev_hash,
                "record_hash": record.record_hash,
                "record_id": record.record_id,
                "stream_id": record.stream_id,
            })
        })
        .collect()
}

fn default_trust_plan(envelope: &TaskEnvelope, attestation_required: bool) -> TrustExecutionPlan {
    TrustExecutionPlan {
        trust_level: if attestation_required {
            "hard".into()
        } else {
            "soft".into()
        },
        verify_identity: true,
        verify_environment: attestation_required,
        rollout_gate: "runtime".into(),
        attestation_backend: "env".into(),
        attestation_required,
        attestation_policy_version: Some("v1".into()),
        policy_refs: vec![envelope.identity.policy_id.clone()],
        budget_scope: format!(
            "{}:{}:{}",
            envelope.identity.tenant_id,
            envelope.identity.principal_id,
            envelope.identity.policy_id
        ),
    }
}

#[cfg(test)]
fn default_attestation_policy() -> AttestationPolicy {
    AttestationPolicy {
        version: "v1".into(),
        strict: true,
        min_tcb_version: "1.0.0".into(),
        evidence_ttl_ms: 300_000,
        require_tenant_binding: true,
        require_nonce: true,
    }
}
fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::ids::{CapabilityId, SessionId, TaskId, TraceId};
    use crate::contracts::types::{ConstraintSet, ExecutionIdentity, TaskEnvelope};
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::thread;
    use std::time::Duration;

    static TEST_ENVELOPE_COUNTER: AtomicU64 = AtomicU64::new(1);

    fn envelope() -> TaskEnvelope {
        let nonce = TEST_ENVELOPE_COUNTER.fetch_add(1, Ordering::SeqCst);
        TaskEnvelope {
            session_id: SessionId::from(format!("session-tk-{nonce}")),
            trace_id: TraceId::from(format!("trace-tk-{nonce}")),
            task_id: TaskId::from(format!("task-tk-{nonce}")),
            capability_id: CapabilityId::from("provider:openai-compatible"),
            identity: ExecutionIdentity {
                tenant_id: "tenant-a".into(),
                principal_id: "principal-a".into(),
                policy_id: "policy-a".into(),
                lease_token: "lease-a".into(),
            },
            payload: serde_json::json!({ "prompt": "hello" }),
            constraints: ConstraintSet {
                max_cpu_percent: 50,
                max_memory_mb: 512,
                timeout_ms: 30_000,
                max_retries: 2,
                max_tokens: 1024,
                io_allow_paths: vec!["./workspace".into()],
                io_deny_paths: vec!["./.git".into()],
                sandbox_profile: "default".into(),
                requires_human_approval: false,
            },
            trust_plan: None,
        }
    }

    #[test]
    fn closure_identity_uses_real_source_artifacts() {
        let cwd = std::env::current_dir().expect("cwd");
        unsafe {
            std::env::set_var("AUTOLOOP_WORKSPACE_ROOT", cwd.to_string_lossy().to_string());
        }
        let ctx = envelope_to_trusted_request(&envelope(), Some("model-a"), true)
            .expect("mapping should work");
        let closure = &ctx.request.closure;

        assert!(closure.flake_ref.contains("git+file://"));
        assert!(closure.flake_lock_digest.starts_with("sha256:"));
        assert!(closure.derivation_digest.starts_with("sha256:"));
        assert!(closure.runtime_closure_hash.starts_with("sha256:"));
        assert!(!closure.store_paths.is_empty());
        assert!(
            closure.store_paths.iter().any(|path| {
                path.ends_with("Cargo.toml")
                    || path.ends_with("Cargo.lock")
                    || path.ends_with(".exe")
            }),
            "expected closure store paths to include real workspace/runtime artifacts"
        );
        assert_ne!(
            closure.store_paths,
            vec!["/nix/store/autoloop-runtime".to_string()]
        );
        unsafe {
            std::env::remove_var("AUTOLOOP_WORKSPACE_ROOT");
        }
    }
    #[test]
    fn maps_task_envelope_to_trusted_execution_request() {
        let ctx = envelope_to_trusted_request(&envelope(), Some("model-a"), true)
            .expect("mapping should work");
        assert_eq!(ctx.request.identity.tenant_id, "tenant-a");
        assert_eq!(ctx.request.plan.steps.len(), 1);
        assert_eq!(ctx.request.verification_spec.trust_level, "hard");
        assert!(ctx.attestation_required);
    }

    #[tokio::test]
    async fn attestation_gate_requires_env_when_enabled() {
        let key = "AUTOLOOP_TEST_ATTEST_ENV";
        unsafe { std::env::remove_var(key) };
        let ctx = envelope_to_trusted_request(&envelope(), None, true).expect("ctx");
        let err = verify_attestation(
            true,
            AttestationBackend::Env,
            "env",
            key,
            "AUTOLOOP_TEST_TOKEN",
            "AUTOLOOP_TEST_QUOTE",
            "AUTOLOOP_TEST_CERT_CHAIN",
            &[],
            None,
            &default_attestation_policy(),
            &ctx.request,
        )
        .await
        .expect_err("must fail when missing");
        assert!(!err.to_string().is_empty());
        unsafe { std::env::set_var(key, "ok") };
        let ctx_second = envelope_to_trusted_request(&envelope(), None, true).expect("ctx-second");
        let ok = verify_attestation(
            true,
            AttestationBackend::Env,
            "env",
            key,
            "AUTOLOOP_TEST_TOKEN",
            "AUTOLOOP_TEST_QUOTE",
            "AUTOLOOP_TEST_CERT_CHAIN",
            &[],
            None,
            &default_attestation_policy(),
            &ctx_second.request,
        )
        .await
        .expect("must pass when present");
        assert!(ok.verified);
    }

    #[tokio::test]
    async fn certificate_attestation_strict_mode_requires_remote_backend() {
        unsafe {
            std::env::set_var("AUTOLOOP_ATTESTATION_CERT_STRICT", "true");
            std::env::set_var(
                "AUTOLOOP_TEST_CERT_CHAIN",
                "-----BEGIN CERTIFICATE-----\nsubject=autoloop\n-----END CERTIFICATE-----",
            );
            std::env::set_var("AUTOLOOP_ATTESTATION_CERT_PROOF", "mismatch-proof");
        }
        let ctx = envelope_to_trusted_request(&envelope(), None, true).expect("ctx");
        let err = verify_attestation(
            true,
            AttestationBackend::CertificateChain,
            "certificate_chain",
            "AUTOLOOP_TEST_ATTEST_ENV",
            "AUTOLOOP_TEST_TOKEN",
            "AUTOLOOP_TEST_QUOTE",
            "AUTOLOOP_TEST_CERT_CHAIN",
            &["autoloop".to_string()],
            None,
            &default_attestation_policy(),
            &ctx.request,
        )
        .await
        .expect_err("strict cert mode should require remote backend");
        assert!(!err.to_string().is_empty());
        unsafe {
            std::env::remove_var("AUTOLOOP_ATTESTATION_CERT_STRICT");
            std::env::remove_var("AUTOLOOP_TEST_CERT_CHAIN");
            std::env::remove_var("AUTOLOOP_ATTESTATION_CERT_PROOF");
        }
    }
    fn spawn_remote_attestation_stub_with_delay(delay_ms: u64) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind remote attestation stub");
        let addr = listener.local_addr().expect("local addr");
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept connection");
            let mut buffer = vec![0u8; 16384];
            let bytes_read = stream.read(&mut buffer).expect("read request");
            let request = String::from_utf8_lossy(&buffer[..bytes_read]);
            let body = request.split("\r\n\r\n").nth(1).unwrap_or("{}");
            let payload: serde_json::Value =
                serde_json::from_str(body).unwrap_or_else(|_| serde_json::json!({}));
            let challenge_nonce = payload
                .get("challenge_nonce")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let challenge_binding = payload
                .get("challenge_binding")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let challenge_expires_at_ms = payload
                .get("challenge_expires_at_ms")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let execution_id = payload
                .get("execution_id")
                .and_then(|v| v.as_str())
                .unwrap_or("remote");
            let tenant_id = payload
                .get("tenant_id")
                .and_then(|v| v.as_str())
                .unwrap_or("tenant-a");
            let verifier_nonce = "verifier-nonce-ptee7";
            let challenge_response_binding = derive_response_binding(
                &challenge_binding,
                verifier_nonce,
                execution_id,
                tenant_id,
            );

            thread::sleep(Duration::from_millis(delay_ms));

            let response_body = serde_json::json!({
                "verified": true,
                "challenge_nonce": challenge_nonce,
                "challenge_binding": challenge_binding,
                "challenge_expires_at_ms": challenge_expires_at_ms,
                "verifier_nonce": verifier_nonce,
                "challenge_response_binding": challenge_response_binding,
                "tcb_version": "1.0.0",
                "issued_at_ms": challenge_expires_at_ms.saturating_sub(1),
                "expires_at_ms": challenge_expires_at_ms,
                "platform": "sgx",
                "quote": "remote-quote-ok"
            })
            .to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response_body.as_bytes().len(),
                response_body
            );
            stream
                .write_all(response.as_bytes())
                .expect("write response");
            let _ = stream.flush();
        });
        format!("http://{}", addr)
    }

    #[tokio::test]
    async fn attestation_ttl_expired_is_rejected() {
        unsafe {
            std::env::set_var("AUTOLOOP_TEST_TOKEN", "token-ptee7");
        }
        let mut policy = default_attestation_policy();
        policy.evidence_ttl_ms = 1;

        let remote_url = spawn_remote_attestation_stub_with_delay(30);
        let ctx = envelope_to_trusted_request(&envelope(), None, true).expect("ctx");
        let err = verify_attestation(
            true,
            AttestationBackend::Remote,
            "remote",
            "AUTOLOOP_TEST_ATTEST_ENV",
            "AUTOLOOP_TEST_TOKEN",
            "AUTOLOOP_TEST_QUOTE",
            "AUTOLOOP_TEST_CERT_CHAIN",
            &[],
            Some(&remote_url),
            &policy,
            &ctx.request,
        )
        .await
        .expect_err("expired attestation challenge must be rejected");

        let anyhow_error = anyhow::anyhow!(err.to_string());
        let (stage, reason) =
            parse_policy_reject_error(&anyhow_error).expect("policy_reject error expected for ttl");
        assert_eq!(stage, "attestation_ttl");
        assert!(reason.contains("expired") || reason.contains("ttl"));

        unsafe {
            std::env::remove_var("AUTOLOOP_TEST_TOKEN");
        }
    }
    #[tokio::test]
    async fn replay_challenge_is_blocked_per_tenant_session_scope() {
        let ctx = envelope_to_trusted_request(&envelope(), None, true).expect("ctx");
        let policy = default_attestation_policy();
        let challenge =
            build_attestation_challenge(&AttestationBackend::Env, &policy, &ctx.request);

        reserve_one_time_challenge(&challenge).expect("first challenge reservation should pass");
        let second = reserve_one_time_challenge(&challenge)
            .expect_err("replayed attestation challenge must be blocked");

        let anyhow_error = anyhow::anyhow!(second.to_string());
        let (stage, reason) =
            parse_policy_reject_error(&anyhow_error).expect("policy_reject error expected");
        assert_eq!(stage, "attestation_replay");
        assert!(reason.contains("replay"));
    }

    #[tokio::test]
    async fn forged_hardware_quote_with_wrong_tenant_binding_is_rejected() {
        unsafe {
            std::env::set_var("AUTOLOOP_ATTESTATION_NONCE", "nonce-fixed-ptee7");
            std::env::set_var(
                "AUTOLOOP_TEST_QUOTE",
                "quote:tenant=tenant-other:nonce=nonce-fixed-ptee7:tcb=1.2.0:abcdefghijklmnopqrstuvwxyz",
            );
        }
        let ctx = envelope_to_trusted_request(&envelope(), None, true).expect("ctx");
        let err = verify_attestation(
            true,
            AttestationBackend::HardwareQuote,
            "hardware_quote",
            "AUTOLOOP_TEST_ATTEST_ENV",
            "AUTOLOOP_TEST_TOKEN",
            "AUTOLOOP_TEST_QUOTE",
            "AUTOLOOP_TEST_CERT_CHAIN",
            &[],
            None,
            &default_attestation_policy(),
            &ctx.request,
        )
        .await
        .expect_err("forged quote tenant binding must be rejected");
        assert!(
            err.to_string().contains("tenant context")
                || err.to_string().contains("tenant binding mismatch"),
            "unexpected error: {}",
            err
        );
        unsafe {
            std::env::remove_var("AUTOLOOP_ATTESTATION_NONCE");
            std::env::remove_var("AUTOLOOP_TEST_QUOTE");
        }
    }
    #[test]
    fn hash_chain_links_previous_record_hash() {
        let ctx = envelope_to_trusted_request(&envelope(), None, false).expect("ctx");
        let chain = build_hash_chain_records(&ctx.request, "output");
        assert_eq!(chain.len(), 1);
        assert_eq!(chain[0].prev_hash, "genesis");
        assert!(!chain[0].record_hash.is_empty());
    }
}
