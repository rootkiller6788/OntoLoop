use std::process::Command;
use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use sha2::{Digest, Sha256};

use crate::ir::TrustedExecutionRequest;
use crate::trust::SupplyChainVerifier;

const DEFAULT_REKOR_URL: &str = "https://rekor.sigstore.dev";
const DEFAULT_PROVENANCE_TYPE: &str = "slsaprovenance";

#[derive(Debug, Clone)]
pub struct SigstoreAdmissionConfig {
    pub cosign_path: String,
    pub require_tlog_inclusion: bool,
}

impl Default for SigstoreAdmissionConfig {
    fn default() -> Self {
        Self {
            cosign_path: "cosign".to_string(),
            require_tlog_inclusion: true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CommandOutput {
    pub status: i32,
    pub stdout: String,
    pub stderr: String,
}

pub trait CommandRunner: Send + Sync {
    fn run(&self, program: &str, args: &[String]) -> Result<CommandOutput>;
}

#[derive(Debug, Clone, Default)]
pub struct ProcessCommandRunner;

impl CommandRunner for ProcessCommandRunner {
    fn run(&self, program: &str, args: &[String]) -> Result<CommandOutput> {
        let output = Command::new(program)
            .args(args)
            .output()
            .with_context(|| format!("failed to spawn command: {} {}", program, args.join(" ")))?;

        Ok(CommandOutput {
            status: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        })
    }
}

pub struct SigstoreAdmissionVerifier {
    config: SigstoreAdmissionConfig,
    runner: Arc<dyn CommandRunner>,
}

impl SigstoreAdmissionVerifier {
    pub fn new(config: SigstoreAdmissionConfig) -> Self {
        Self::with_runner(config, Arc::new(ProcessCommandRunner))
    }

    pub fn with_runner(config: SigstoreAdmissionConfig, runner: Arc<dyn CommandRunner>) -> Self {
        Self { config, runner }
    }

    fn verify_policy_pin(&self, request: &TrustedExecutionRequest) -> Result<()> {
        if request.constraints.policy_refs.is_empty() {
            return Err(anyhow!("missing pinned policy refs"));
        }
        if !request
            .constraints
            .policy_refs
            .iter()
            .any(|p| p == &request.supply_chain.policy_bundle_digest)
        {
            return Err(anyhow!("policy bundle digest is not pinned in constraints"));
        }
        Ok(())
    }

    fn verify_rekor_evidence_fields(&self, request: &TrustedExecutionRequest) -> Result<()> {
        if request.supply_chain.rekor_log_ref.is_empty() {
            return Err(anyhow!("missing Rekor log reference"));
        }
        if request.supply_chain.rekor_inclusion_proof.is_empty() {
            return Err(anyhow!("missing Rekor inclusion proof"));
        }
        Ok(())
    }

    fn verify_artifact_signature(
        &self,
        request: &TrustedExecutionRequest,
        artifact_ref: &str,
        label: &str,
    ) -> Result<()> {
        if artifact_ref.is_empty() {
            return Err(anyhow!("missing artifact reference for {}", label));
        }

        let mut args = vec![
            "verify".to_string(),
            artifact_ref.to_string(),
            "--output".to_string(),
            "json".to_string(),
            "--rekor-url".to_string(),
            self.rekor_url(request),
            "--certificate-identity".to_string(),
            request.supply_chain.signer_identity_ref.clone(),
            "--certificate-oidc-issuer".to_string(),
            request.supply_chain.signer_oidc_issuer.clone(),
        ];

        if self.config.require_tlog_inclusion {
            args.push("--insecure-ignore-tlog=false".to_string());
        }

        let stdout = self
            .run_cosign(&args)
            .with_context(|| format!("signature verification failed for {}", label))?;
        self.verify_signature_output_binding(artifact_ref, &stdout)
            .with_context(|| format!("signature output missing expected digest for {}", label))?;
        Ok(())
    }

    fn verify_provenance(&self, request: &TrustedExecutionRequest) -> Result<()> {
        if request.supply_chain.provenance_digest.is_empty() {
            return Err(anyhow!("missing provenance digest"));
        }

        let provenance_type = if request.supply_chain.provenance_type.is_empty() {
            DEFAULT_PROVENANCE_TYPE.to_string()
        } else {
            request.supply_chain.provenance_type.clone()
        };

        let mut args = vec![
            "verify-attestation".to_string(),
            request.supply_chain.executor_digest.clone(),
            "--type".to_string(),
            provenance_type,
            "--output".to_string(),
            "json".to_string(),
            "--rekor-url".to_string(),
            self.rekor_url(request),
            "--certificate-identity".to_string(),
            request.supply_chain.signer_identity_ref.clone(),
            "--certificate-oidc-issuer".to_string(),
            request.supply_chain.signer_oidc_issuer.clone(),
        ];

        if self.config.require_tlog_inclusion {
            args.push("--insecure-ignore-tlog=false".to_string());
        }

        let stdout = self
            .run_cosign(&args)
            .context("provenance attestation verification failed")?;

        let expected = request.supply_chain.provenance_digest.trim();
        let output_hash = Self::sha256_hex(stdout.as_bytes());
        let output_hash_prefixed = format!("sha256:{}", output_hash);

        if !stdout.contains(expected) && expected != output_hash && expected != output_hash_prefixed
        {
            return Err(anyhow!(
                "provenance digest mismatch: expected={} output_sha256={}",
                expected,
                output_hash
            ));
        }
        if !stdout.contains(request.supply_chain.rekor_log_ref.trim())
            && !stdout.contains(request.supply_chain.rekor_inclusion_proof.trim())
        {
            return Err(anyhow!(
                "provenance output missing Rekor binding: ref={} proof={}",
                request.supply_chain.rekor_log_ref,
                request.supply_chain.rekor_inclusion_proof
            ));
        }

        Ok(())
    }

    fn verify_signature_output_binding(&self, artifact_ref: &str, stdout: &str) -> Result<()> {
        let expected_token =
            Self::digest_token(artifact_ref).unwrap_or_else(|| artifact_ref.trim());
        if stdout.trim().is_empty() {
            return Err(anyhow!("empty signature verification output"));
        }
        if !stdout.contains(expected_token) {
            return Err(anyhow!(
                "verification output does not contain artifact digest token: {}",
                expected_token
            ));
        }
        Ok(())
    }

    fn run_cosign(&self, args: &[String]) -> Result<String> {
        let result = self
            .runner
            .run(&self.config.cosign_path, args)
            .with_context(|| format!("failed to execute cosign: {}", args.join(" ")))?;

        if result.status == 0 {
            Ok(result.stdout)
        } else {
            Err(anyhow!(
                "cosign exited with status {}: {}",
                result.status,
                result.stderr
            ))
        }
    }

    fn rekor_url(&self, request: &TrustedExecutionRequest) -> String {
        if request.supply_chain.rekor_url.is_empty() {
            DEFAULT_REKOR_URL.to_string()
        } else {
            request.supply_chain.rekor_url.clone()
        }
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        hex::encode(hasher.finalize())
    }

    fn digest_token(artifact_ref: &str) -> Option<&str> {
        let trimmed = artifact_ref.trim();
        if trimmed.is_empty() {
            return None;
        }
        if let Some(idx) = trimmed.rfind('@') {
            return Some(trimmed[idx + 1..].trim());
        }
        if let Some(idx) = trimmed.find("sha256:") {
            return Some(trimmed[idx..].trim());
        }
        Some(trimmed)
    }
}

impl SupplyChainVerifier for SigstoreAdmissionVerifier {
    fn verify_admission(&self, request: &TrustedExecutionRequest) -> Result<()> {
        self.verify_policy_pin(request)?;
        self.verify_rekor_evidence_fields(request)?;
        self.verify_artifact_signature(request, &request.supply_chain.executor_digest, "executor")?;
        self.verify_artifact_signature(request, &request.supply_chain.verifier_digest, "verifier")?;
        self.verify_artifact_signature(
            request,
            &request.supply_chain.capability_package_digest,
            "capability package",
        )?;
        self.verify_artifact_signature(
            request,
            &request.supply_chain.policy_bundle_digest,
            "policy bundle",
        )?;
        self.verify_provenance(request)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, VecDeque};
    use std::sync::{Arc, Mutex};

    use anyhow::{Result, anyhow};

    use super::{CommandOutput, CommandRunner, SigstoreAdmissionConfig, SigstoreAdmissionVerifier};
    use crate::ir::{
        ConstraintSet, ExecutionPlan, ExecutionStep, FailurePolicy, HardeningPolicy,
        IdentityContext, Intent, ReproducibleClosure, RuntimeIsland, StepAction,
        SupplyChainEnvelope, TraceContext, TrustedExecutionRequest, VerificationSpec,
    };
    use crate::trust::SupplyChainVerifier;

    #[derive(Default)]
    struct FakeRunner {
        outputs: Mutex<VecDeque<CommandOutput>>,
        calls: Mutex<Vec<Vec<String>>>,
    }

    impl FakeRunner {
        fn with_outputs(outputs: Vec<CommandOutput>) -> Self {
            Self {
                outputs: Mutex::new(outputs.into()),
                calls: Mutex::new(vec![]),
            }
        }

        fn call_count(&self) -> usize {
            self.calls.lock().map(|c| c.len()).unwrap_or(0)
        }

        fn calls(&self) -> Vec<Vec<String>> {
            self.calls.lock().map(|c| c.clone()).unwrap_or_default()
        }
    }

    impl CommandRunner for FakeRunner {
        fn run(&self, _program: &str, args: &[String]) -> Result<CommandOutput> {
            self.calls
                .lock()
                .map_err(|_| anyhow!("calls lock poisoned"))?
                .push(args.to_vec());

            self.outputs
                .lock()
                .map_err(|_| anyhow!("outputs lock poisoned"))?
                .pop_front()
                .ok_or_else(|| anyhow!("no fake output configured"))
        }
    }

    fn success_output(stdout: &str) -> CommandOutput {
        CommandOutput {
            status: 0,
            stdout: stdout.to_string(),
            stderr: String::new(),
        }
    }

    fn error_output(stderr: &str) -> CommandOutput {
        CommandOutput {
            status: 1,
            stdout: String::new(),
            stderr: stderr.to_string(),
        }
    }

    fn make_request(provenance_digest: &str) -> TrustedExecutionRequest {
        let step = ExecutionStep {
            step_id: "step-1".to_string(),
            action: StepAction::CapabilityCall,
            capability_ref: Some("llm.echo".to_string()),
            runtime_island: RuntimeIsland::Trusted,
            hardening: HardeningPolicy::default(),
            input: "hello".to_string(),
            dependencies: vec![],
            local_constraints: HashMap::new(),
            failure_policy: FailurePolicy::default(),
        };

        TrustedExecutionRequest {
            execution_id: "exec-1".to_string(),
            identity: IdentityContext {
                principal: "agent-1".to_string(),
                tenant_id: "tenant-1".to_string(),
                workspace: "ws-a".to_string(),
                signature: "sig".to_string(),
            },
            intent: Intent {
                operation: "execute".to_string(),
                payload: "task".to_string(),
            },
            plan: ExecutionPlan {
                plan_id: "plan-1".to_string(),
                version: 1,
                strategy: "sequential".to_string(),
                steps: vec![step],
            },
            constraints: ConstraintSet {
                max_runtime_ms: 30_000,
                max_cpu_units: 2,
                max_memory_mb: 256,
                policy_refs: vec!["ghcr.io/acme/policy@sha256:policy-v1".to_string()],
            },
            verification_spec: VerificationSpec {
                verify_environment: true,
                verify_identity: true,
                validate_output: true,
                trust_level: "hard".to_string(),
            },
            supply_chain: SupplyChainEnvelope {
                executor_digest: "ghcr.io/acme/executor@sha256:executor-v1".to_string(),
                verifier_digest: "ghcr.io/acme/verifier@sha256:verifier-v1".to_string(),
                policy_bundle_digest: "ghcr.io/acme/policy@sha256:policy-v1".to_string(),
                capability_package_digest: "ghcr.io/acme/capability@sha256:cap-v1".to_string(),
                provenance_digest: provenance_digest.to_string(),
                signer_identity_ref: "ci@acme.dev".to_string(),
                signer_oidc_issuer: "https://token.actions.githubusercontent.com".to_string(),
                rekor_url: "https://rekor.sigstore.dev".to_string(),
                rekor_log_ref: "rekor://entry/1".to_string(),
                rekor_inclusion_proof: "proof-1".to_string(),
                provenance_type: "slsaprovenance".to_string(),
            },
            closure: ReproducibleClosure {
                flake_ref: "github:acme/trustkernel".to_string(),
                flake_lock_digest: "sha256:flake-lock-v1".to_string(),
                derivation_digest: "sha256:drv-v1".to_string(),
                store_paths: vec!["/nix/store/aaa".to_string()],
                runtime_closure_hash: "sha256:closure-v1".to_string(),
                generation_id: "gen-1".to_string(),
                profile_id: "profile-prod".to_string(),
                config_digest: "sha256:cfg-v1".to_string(),
            },
            trace_context: TraceContext {
                trace_id: "trace-1".to_string(),
                parent_execution_id: None,
                submitted_at_ms: 1,
            },
        }
    }

    #[test]
    fn verifies_signatures_rekor_and_provenance() {
        let provenance_payload = r#"[{\"predicateType\":\"https://slsa.dev/provenance/v1\"}]"#;
        let req = make_request("sha256:attestation-v1");
        let exec_token = "sha256:executor-v1";
        let verifier_token = "sha256:verifier-v1";
        let cap_token = "sha256:cap-v1";
        let policy_token = "sha256:policy-v1";
        let runner = Arc::new(FakeRunner::with_outputs(vec![
            success_output(&format!(
                "[{{\"critical\":{{\"image\":{{\"docker-manifest-digest\":\"{}\"}}}}}}]",
                exec_token
            )),
            success_output(&format!(
                "[{{\"critical\":{{\"image\":{{\"docker-manifest-digest\":\"{}\"}}}}}}]",
                verifier_token
            )),
            success_output(&format!(
                "[{{\"critical\":{{\"image\":{{\"docker-manifest-digest\":\"{}\"}}}}}}]",
                cap_token
            )),
            success_output(&format!(
                "[{{\"critical\":{{\"image\":{{\"docker-manifest-digest\":\"{}\"}}}}}}]",
                policy_token
            )),
            success_output(&format!(
                "{} {} {} {}",
                provenance_payload,
                "sha256:attestation-v1",
                req.supply_chain.rekor_log_ref,
                req.supply_chain.rekor_inclusion_proof
            )),
        ]));
        let verifier = SigstoreAdmissionVerifier::with_runner(
            SigstoreAdmissionConfig::default(),
            runner.clone(),
        );

        verifier
            .verify_admission(&req)
            .expect("admission must pass");

        assert_eq!(runner.call_count(), 5);
        let calls = runner.calls();
        assert!(
            calls
                .iter()
                .any(|args| args.contains(&"verify-attestation".to_string()))
        );
        assert!(
            calls
                .iter()
                .filter(|args| args.contains(&"verify".to_string()))
                .all(|args| args.contains(&"--rekor-url".to_string()))
        );
    }

    #[test]
    fn fails_fast_when_policy_not_pinned() {
        let mut req = make_request("sha256:attestation-v1");
        req.constraints.policy_refs.clear();
        let runner = Arc::new(FakeRunner::with_outputs(vec![success_output("unused")]));
        let verifier = SigstoreAdmissionVerifier::with_runner(
            SigstoreAdmissionConfig::default(),
            runner.clone(),
        );

        let err = verifier
            .verify_admission(&req)
            .expect_err("must fail when policy is not pinned");

        assert!(err.to_string().contains("policy"));
        assert_eq!(runner.call_count(), 0);
    }

    #[test]
    fn fails_when_signature_verification_fails() {
        let req = make_request("sha256:attestation-v1");
        let runner = Arc::new(FakeRunner::with_outputs(vec![error_output(
            "bad signature",
        )]));
        let verifier = SigstoreAdmissionVerifier::with_runner(
            SigstoreAdmissionConfig::default(),
            runner.clone(),
        );

        let err = verifier
            .verify_admission(&req)
            .expect_err("must fail on signature verification");

        assert!(err.to_string().contains("signature verification failed"));
        assert_eq!(runner.call_count(), 1);
    }

    #[test]
    fn fails_when_provenance_digest_mismatches() {
        let req = make_request("sha256:attestation-v1");
        let runner = Arc::new(FakeRunner::with_outputs(vec![
            success_output("sha256:executor-v1"),
            success_output("sha256:verifier-v1"),
            success_output("sha256:cap-v1"),
            success_output("sha256:policy-v1"),
            success_output("[{\"predicateType\":\"https://slsa.dev/provenance/v1\"}]"),
        ]));
        let verifier =
            SigstoreAdmissionVerifier::with_runner(SigstoreAdmissionConfig::default(), runner);

        let err = verifier
            .verify_admission(&req)
            .expect_err("must fail on provenance mismatch");
        assert!(err.to_string().contains("provenance digest mismatch"));
    }
}
