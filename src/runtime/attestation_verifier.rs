use std::sync::Arc;

use anyhow::{Result, bail};
use async_trait::async_trait;

use crate::config::AttestationBackend;
use crate::contracts::types::AttestationPolicy;

use super::trust_bridge::{AttestationReport, verify_attestation_legacy};
use trustkernel::ir::TrustedExecutionRequest;

#[derive(Debug, Clone)]
pub struct AttestationRequestBinding {
    pub tenant_id: String,
    pub policy_version: String,
    pub request_nonce_required: bool,
    pub evidence_ttl_ms: u64,
    pub execution_id: String,
}

#[derive(Debug, Clone)]
pub struct AttestationVerifyInput<'a> {
    pub required: bool,
    pub secret_env: &'a str,
    pub token_env: &'a str,
    pub quote_env: &'a str,
    pub cert_chain_env: &'a str,
    pub cert_subject_allowlist: &'a [String],
    pub remote_url: Option<&'a str>,
    pub policy: &'a AttestationPolicy,
    pub trusted: &'a TrustedExecutionRequest,
    pub binding: AttestationRequestBinding,
}

#[async_trait]
pub trait AttestationVerifier: Send + Sync {
    fn name(&self) -> &'static str;
    fn supports(&self, backend: &AttestationBackend, backend_hint: &str) -> bool;

    async fn verify(
        &self,
        backend: AttestationBackend,
        backend_hint: &str,
        input: &AttestationVerifyInput<'_>,
    ) -> Result<AttestationReport>;
}

#[derive(Default)]
pub struct AttestationVerifierRegistry {
    verifiers: Vec<Arc<dyn AttestationVerifier>>,
}

impl AttestationVerifierRegistry {
    pub fn with_defaults() -> Self {
        Self {
            verifiers: vec![
                Arc::new(MockVerifier),
                Arc::new(SgxDcapVerifier),
                Arc::new(SevSnpVerifier),
                Arc::new(BackendPassthroughVerifier {
                    backend: AttestationBackend::Env,
                    id: "env-verifier",
                }),
                Arc::new(BackendPassthroughVerifier {
                    backend: AttestationBackend::Remote,
                    id: "remote-verifier",
                }),
                Arc::new(BackendPassthroughVerifier {
                    backend: AttestationBackend::HardwareQuote,
                    id: "hardware-quote-verifier",
                }),
                Arc::new(BackendPassthroughVerifier {
                    backend: AttestationBackend::CertificateChain,
                    id: "certificate-chain-verifier",
                }),
            ],
        }
    }

    pub fn resolve(
        &self,
        backend: &AttestationBackend,
        backend_hint: &str,
    ) -> Arc<dyn AttestationVerifier> {
        self.verifiers
            .iter()
            .find(|verifier| verifier.supports(backend, backend_hint))
            .cloned()
            .unwrap_or_else(|| {
                Arc::new(BackendPassthroughVerifier {
                    backend: backend.clone(),
                    id: "fallback-backend-verifier",
                })
            })
    }
}

fn verify_report_binding(
    report: &AttestationReport,
    input: &AttestationVerifyInput<'_>,
) -> Result<()> {
    if report.verdict.policy_version != input.binding.policy_version {
        bail!(
            "attestation policy binding mismatch: expected={} actual={}",
            input.binding.policy_version,
            report.verdict.policy_version
        );
    }

    if report
        .verdict
        .evidence_id
        .as_deref()
        .is_some_and(|id: &str| !id.contains(&input.binding.execution_id))
    {
        bail!(
            "attestation evidence id binding mismatch: execution_id={} evidence_id={}",
            input.binding.execution_id,
            report.verdict.evidence_id.as_deref().unwrap_or_default()
        );
    }

    if input.policy.require_tenant_binding && !report.verdict.tenant_binding_passed {
        bail!("attestation tenant binding check failed");
    }

    if input.binding.request_nonce_required && !report.verdict.nonce_present {
        bail!("attestation nonce binding check failed");
    }

    if !report.verdict.freshness_passed {
        bail!("attestation freshness check failed");
    }

    if let Some(bundle) = &report.evidence.quote_bundle {
        if input.policy.require_tenant_binding {
            if bundle.tenant_binding.as_deref() != Some(input.binding.tenant_id.as_str()) {
                bail!(
                    "attestation quote tenant mismatch: expected={} actual={}",
                    input.binding.tenant_id,
                    bundle.tenant_binding.as_deref().unwrap_or_default()
                );
            }
        }

        let ttl_window = bundle.expires_at_ms.saturating_sub(bundle.issued_at_ms);
        if ttl_window > input.binding.evidence_ttl_ms {
            bail!(
                "attestation ttl binding mismatch: ttl_window={} policy_ttl={}",
                ttl_window,
                input.binding.evidence_ttl_ms
            );
        }
    }

    Ok(())
}
struct BackendPassthroughVerifier {
    backend: AttestationBackend,
    id: &'static str,
}

#[async_trait]
impl AttestationVerifier for BackendPassthroughVerifier {
    fn name(&self) -> &'static str {
        self.id
    }

    fn supports(&self, backend: &AttestationBackend, _backend_hint: &str) -> bool {
        std::mem::discriminant(backend) == std::mem::discriminant(&self.backend)
    }

    async fn verify(
        &self,
        _backend: AttestationBackend,
        _backend_hint: &str,
        input: &AttestationVerifyInput<'_>,
    ) -> Result<AttestationReport> {
        let report = verify_attestation_legacy(
            input.required,
            self.backend.clone(),
            input.secret_env,
            input.token_env,
            input.quote_env,
            input.cert_chain_env,
            input.cert_subject_allowlist,
            input.remote_url,
            input.policy,
            input.trusted,
        )
        .await?;
        verify_report_binding(&report, input)?;
        Ok(report)
    }
}

pub struct SgxDcapVerifier;

#[async_trait]
impl AttestationVerifier for SgxDcapVerifier {
    fn name(&self) -> &'static str {
        "sgx-dcap-verifier"
    }

    fn supports(&self, _backend: &AttestationBackend, backend_hint: &str) -> bool {
        matches!(
            backend_hint.to_ascii_lowercase().as_str(),
            "sgx" | "sgx_dcap" | "dcap"
        )
    }

    async fn verify(
        &self,
        _backend: AttestationBackend,
        _backend_hint: &str,
        input: &AttestationVerifyInput<'_>,
    ) -> Result<AttestationReport> {
        let mut report = verify_attestation_legacy(
            input.required,
            AttestationBackend::HardwareQuote,
            input.secret_env,
            input.token_env,
            input.quote_env,
            input.cert_chain_env,
            input.cert_subject_allowlist,
            input.remote_url,
            input.policy,
            input.trusted,
        )
        .await?;

        report.backend = "sgx_dcap".into();
        report.details["adapter"] = serde_json::Value::String(self.name().to_string());
        if let Some(bundle) = report.evidence.quote_bundle.as_mut() {
            bundle.platform = crate::contracts::types::AttestationPlatform::Sgx;
        }
        verify_report_binding(&report, input)?;
        Ok(report)
    }
}

pub struct SevSnpVerifier;

#[async_trait]
impl AttestationVerifier for SevSnpVerifier {
    fn name(&self) -> &'static str {
        "sev-snp-verifier"
    }

    fn supports(&self, _backend: &AttestationBackend, backend_hint: &str) -> bool {
        matches!(
            backend_hint.to_ascii_lowercase().as_str(),
            "sev" | "sev_snp" | "snp"
        )
    }

    async fn verify(
        &self,
        _backend: AttestationBackend,
        _backend_hint: &str,
        input: &AttestationVerifyInput<'_>,
    ) -> Result<AttestationReport> {
        let mut report = verify_attestation_legacy(
            input.required,
            AttestationBackend::HardwareQuote,
            input.secret_env,
            input.token_env,
            input.quote_env,
            input.cert_chain_env,
            input.cert_subject_allowlist,
            input.remote_url,
            input.policy,
            input.trusted,
        )
        .await?;

        report.backend = "sev_snp".into();
        report.details["adapter"] = serde_json::Value::String(self.name().to_string());
        if let Some(bundle) = report.evidence.quote_bundle.as_mut() {
            bundle.platform = crate::contracts::types::AttestationPlatform::SevSnp;
        }
        verify_report_binding(&report, input)?;
        Ok(report)
    }
}

pub struct MockVerifier;

#[async_trait]
impl AttestationVerifier for MockVerifier {
    fn name(&self) -> &'static str {
        "mock-verifier"
    }

    fn supports(&self, _backend: &AttestationBackend, backend_hint: &str) -> bool {
        matches!(
            backend_hint.to_ascii_lowercase().as_str(),
            "mock" | "test" | "dev_mock"
        )
    }

    async fn verify(
        &self,
        backend: AttestationBackend,
        _backend_hint: &str,
        input: &AttestationVerifyInput<'_>,
    ) -> Result<AttestationReport> {
        let pass = !matches!(
            std::env::var("AUTOLOOP_ATTESTATION_MOCK_RESULT")
                .unwrap_or_else(|_| "pass".to_string())
                .to_ascii_lowercase()
                .as_str(),
            "fail" | "reject" | "false"
        );
        if !pass {
            bail!("mock attestation verifier rejected evidence");
        }

        let mut report = verify_attestation_legacy(
            input.required,
            backend,
            input.secret_env,
            input.token_env,
            input.quote_env,
            input.cert_chain_env,
            input.cert_subject_allowlist,
            input.remote_url,
            input.policy,
            input.trusted,
        )
        .await?;
        report.details["adapter"] = serde_json::Value::String(self.name().to_string());
        verify_report_binding(&report, input)?;
        Ok(report)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use trustkernel::ir::{
        ConstraintSet, ExecutionPlan, IdentityContext, Intent, ReproducibleClosure,
        SupplyChainEnvelope, TraceContext, TrustedExecutionRequest, VerificationSpec,
    };

    fn trusted_request() -> TrustedExecutionRequest {
        TrustedExecutionRequest {
            execution_id: "exec:test".into(),
            identity: IdentityContext {
                principal: "principal-a".into(),
                tenant_id: "tenant-a".into(),
                workspace: "workspace-a".into(),
                signature: "sig-a".into(),
            },
            intent: Intent {
                operation: "op".into(),
                payload: "{}".into(),
            },
            plan: ExecutionPlan {
                plan_id: "plan-a".into(),
                version: 1,
                strategy: "single".into(),
                steps: Vec::new(),
            },
            constraints: ConstraintSet {
                max_runtime_ms: 1000,
                max_cpu_units: 1,
                max_memory_mb: 64,
                policy_refs: vec!["policy-a".into()],
            },
            verification_spec: VerificationSpec {
                verify_environment: true,
                verify_identity: true,
                validate_output: true,
                trust_level: "hard".into(),
            },
            supply_chain: SupplyChainEnvelope {
                executor_digest: "sha256:executor-a".into(),
                verifier_digest: "sha256:verifier-a".into(),
                policy_bundle_digest: "sha256:policy-a".into(),
                capability_package_digest: "sha256:capability-a".into(),
                provenance_digest: "sha256:provenance-a".into(),
                signer_identity_ref: "sigstore://issuer/subject".into(),
                signer_oidc_issuer: "https://issuer.example".into(),
                rekor_url: "https://rekor.sigstore.dev".into(),
                rekor_log_ref: "rekor://entry/a".into(),
                rekor_inclusion_proof: "proof-a".into(),
                provenance_type: "slsaprovenance".into(),
            },
            closure: ReproducibleClosure {
                flake_ref: "github:autoloop/noumenoncore".into(),
                flake_lock_digest: "sha256:flake-lock-a".into(),
                derivation_digest: "sha256:drv-a".into(),
                store_paths: vec!["/nix/store/autoloop-runtime".into()],
                runtime_closure_hash: "sha256:closure-a".into(),
                generation_id: "gen-a".into(),
                profile_id: "profile-a".into(),
                config_digest: "sha256:config-a".into(),
            },
            trace_context: TraceContext {
                trace_id: "trace-a".into(),
                parent_execution_id: None,
                submitted_at_ms: 0,
            },
        }
    }

    fn policy() -> AttestationPolicy {
        AttestationPolicy {
            version: "v1".into(),
            strict: true,
            min_tcb_version: "1.0.0".into(),
            evidence_ttl_ms: 300_000,
            require_tenant_binding: true,
            require_nonce: true,
        }
    }

    #[test]
    fn registry_resolves_sgx_adapter_from_hint() {
        let registry = AttestationVerifierRegistry::with_defaults();
        let verifier = registry.resolve(&AttestationBackend::Env, "sgx_dcap");
        assert_eq!(verifier.name(), "sgx-dcap-verifier");
    }

    #[test]
    fn binding_guard_rejects_policy_version_mismatch() {
        let policy = policy();
        let report = AttestationReport {
            backend: "env".into(),
            verified: true,
            reference: "env:AUTOLOOP_TEST_ATTEST_ENV".into(),
            details: serde_json::json!({}),
            evidence: crate::contracts::types::AttestationEvidence {
                evidence_id: "evidence:exec:test:env".into(),
                backend: "env".into(),
                quote_bundle: None,
                remote_report: None,
                digest: Some("sha256:test".into()),
                source_ref: Some("env:AUTOLOOP_TEST_ATTEST_ENV".into()),
            },
            verdict: crate::contracts::types::VerifierVerdict {
                verified: true,
                reason: "ok".into(),
                policy_version: policy.version.clone(),
                evidence_id: Some("evidence:exec:test:env".into()),
                verifier_name: "mock".into(),
                min_tcb_passed: true,
                freshness_passed: true,
                tenant_binding_passed: true,
                nonce_present: true,
            },
        };
        let request = trusted_request();
        let input = AttestationVerifyInput {
            required: true,
            secret_env: "AUTOLOOP_TEST_ATTEST_ENV",
            token_env: "AUTOLOOP_TEST_TOKEN",
            quote_env: "AUTOLOOP_TEST_QUOTE",
            cert_chain_env: "AUTOLOOP_TEST_CERT_CHAIN",
            cert_subject_allowlist: &[],
            remote_url: None,
            policy: &policy,
            trusted: &request,
            binding: AttestationRequestBinding {
                tenant_id: request.identity.tenant_id.clone(),
                policy_version: "v2".into(),
                request_nonce_required: true,
                evidence_ttl_ms: policy.evidence_ttl_ms,
                execution_id: request.execution_id.clone(),
            },
        };

        let err = verify_report_binding(&report, &input).expect_err("policy mismatch must fail");
        assert!(err.to_string().contains("policy binding mismatch"));
    }

    #[test]
    fn binding_guard_rejects_missing_nonce_when_required() {
        let policy = policy();
        let report = AttestationReport {
            backend: "remote".into(),
            verified: true,
            reference: "remote:attestation".into(),
            details: serde_json::json!({}),
            evidence: crate::contracts::types::AttestationEvidence {
                evidence_id: "evidence:exec:test:remote".into(),
                backend: "remote".into(),
                quote_bundle: None,
                remote_report: Some(serde_json::json!({"verified": true})),
                digest: Some("sha256:test".into()),
                source_ref: Some("remote:https://attestation".into()),
            },
            verdict: crate::contracts::types::VerifierVerdict {
                verified: true,
                reason: "ok".into(),
                policy_version: policy.version.clone(),
                evidence_id: Some("evidence:exec:test:remote".into()),
                verifier_name: "remote".into(),
                min_tcb_passed: true,
                freshness_passed: true,
                tenant_binding_passed: true,
                nonce_present: false,
            },
        };
        let request = trusted_request();
        let input = AttestationVerifyInput {
            required: true,
            secret_env: "AUTOLOOP_TEST_ATTEST_ENV",
            token_env: "AUTOLOOP_TEST_TOKEN",
            quote_env: "AUTOLOOP_TEST_QUOTE",
            cert_chain_env: "AUTOLOOP_TEST_CERT_CHAIN",
            cert_subject_allowlist: &[],
            remote_url: None,
            policy: &policy,
            trusted: &request,
            binding: AttestationRequestBinding {
                tenant_id: request.identity.tenant_id.clone(),
                policy_version: policy.version.clone(),
                request_nonce_required: true,
                evidence_ttl_ms: policy.evidence_ttl_ms,
                execution_id: request.execution_id.clone(),
            },
        };

        let err = verify_report_binding(&report, &input).expect_err("nonce missing must fail");
        assert!(err.to_string().contains("nonce binding check failed"));
    }
    #[tokio::test]
    async fn mock_verifier_can_block_execution() {
        unsafe {
            std::env::set_var("AUTOLOOP_ATTESTATION_MOCK_RESULT", "fail");
            std::env::set_var("AUTOLOOP_TEST_ATTEST_ENV", "ok");
        }
        let request = trusted_request();
        let policy = policy();
        let input = AttestationVerifyInput {
            required: true,
            secret_env: "AUTOLOOP_TEST_ATTEST_ENV",
            token_env: "AUTOLOOP_TEST_TOKEN",
            quote_env: "AUTOLOOP_TEST_QUOTE",
            cert_chain_env: "AUTOLOOP_TEST_CERT_CHAIN",
            cert_subject_allowlist: &[],
            remote_url: None,
            policy: &policy,
            trusted: &request,
            binding: AttestationRequestBinding {
                tenant_id: request.identity.tenant_id.clone(),
                policy_version: policy.version.clone(),
                request_nonce_required: policy.require_nonce,
                evidence_ttl_ms: policy.evidence_ttl_ms,
                execution_id: request.execution_id.clone(),
            },
        };
        let verifier = MockVerifier;
        let err = verifier
            .verify(AttestationBackend::Env, "mock", &input)
            .await
            .expect_err("mock verifier should reject when forced to fail");
        assert!(
            err.to_string()
                .contains("mock attestation verifier rejected")
        );
        unsafe {
            std::env::remove_var("AUTOLOOP_ATTESTATION_MOCK_RESULT");
            std::env::remove_var("AUTOLOOP_TEST_ATTEST_ENV");
        }
    }
}
