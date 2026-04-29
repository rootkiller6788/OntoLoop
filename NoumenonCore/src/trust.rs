use anyhow::{Result, anyhow};
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

use crate::ir::{IdentityContext, TrustedExecutionRequest};

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustLevel {
    Soft,
    Hard,
}

pub trait IdentityAuthority: Send + Sync {
    fn authenticate(&self, identity: &IdentityContext) -> Result<()>;
}

pub trait TenantAuthority: Send + Sync {
    fn authorize(&self, identity: &IdentityContext, capability: &str) -> Result<()>;
}

pub trait AttestationVerifier: Send + Sync {
    fn verify(&self) -> Result<()>;
}

pub trait SupplyChainVerifier: Send + Sync {
    fn verify_admission(&self, request: &TrustedExecutionRequest) -> Result<()>;
}

pub trait CryptoService: Send + Sync {
    fn digest_hex(&self, data: &str) -> String;
}

#[derive(Debug, Clone)]
pub struct HmacIdentityAuthority {
    secret: String,
}

impl HmacIdentityAuthority {
    pub fn new(secret: impl Into<String>) -> Self {
        Self {
            secret: secret.into(),
        }
    }

    pub fn sign(&self, principal: &str, tenant_id: &str, workspace: &str) -> String {
        let mut mac = HmacSha256::new_from_slice(self.secret.as_bytes()).expect("valid key");
        mac.update(principal.as_bytes());
        mac.update(b":");
        mac.update(tenant_id.as_bytes());
        mac.update(b":");
        mac.update(workspace.as_bytes());
        hex::encode(mac.finalize().into_bytes())
    }
}

impl IdentityAuthority for HmacIdentityAuthority {
    fn authenticate(&self, identity: &IdentityContext) -> Result<()> {
        let expected = self.sign(
            &identity.principal,
            &identity.tenant_id,
            &identity.workspace,
        );
        if expected == identity.signature {
            Ok(())
        } else {
            Err(anyhow!("identity signature mismatch"))
        }
    }
}

#[derive(Debug, Clone)]
pub struct StaticTenantAuthority;

impl TenantAuthority for StaticTenantAuthority {
    fn authorize(&self, identity: &IdentityContext, capability: &str) -> Result<()> {
        if identity.tenant_id.is_empty() || capability.is_empty() {
            return Err(anyhow!("invalid tenant authorization input"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct StaticAttestationVerifier {
    pub trusted: bool,
}

impl AttestationVerifier for StaticAttestationVerifier {
    fn verify(&self) -> Result<()> {
        if self.trusted {
            Ok(())
        } else {
            Err(anyhow!("attestation verification failed"))
        }
    }
}

#[derive(Debug, Clone)]
pub struct StaticSupplyChainVerifier;

impl SupplyChainVerifier for StaticSupplyChainVerifier {
    fn verify_admission(&self, request: &TrustedExecutionRequest) -> Result<()> {
        let sc = &request.supply_chain;
        let closure = &request.closure;

        if sc.executor_digest.is_empty() {
            return Err(anyhow!("missing executor digest"));
        }
        if sc.verifier_digest.is_empty() {
            return Err(anyhow!("missing verifier digest"));
        }
        if sc.policy_bundle_digest.is_empty() {
            return Err(anyhow!("missing policy bundle digest"));
        }
        if request.constraints.policy_refs.is_empty() {
            return Err(anyhow!("missing pinned policy refs"));
        }
        if !request
            .constraints
            .policy_refs
            .iter()
            .any(|p| p == &sc.policy_bundle_digest)
        {
            return Err(anyhow!("policy bundle digest is not pinned in constraints"));
        }
        if sc.capability_package_digest.is_empty() {
            return Err(anyhow!("missing capability package digest"));
        }
        if sc.provenance_digest.is_empty() {
            return Err(anyhow!("missing provenance digest"));
        }
        if sc.signer_oidc_issuer.is_empty() {
            return Err(anyhow!("missing signer OIDC issuer"));
        }
        if sc.rekor_log_ref.is_empty() || sc.rekor_inclusion_proof.is_empty() {
            return Err(anyhow!("missing Rekor inclusion evidence"));
        }
        if sc.rekor_url.is_empty() {
            return Err(anyhow!("missing Rekor URL"));
        }
        if sc.provenance_type.is_empty() {
            return Err(anyhow!("missing provenance type"));
        }
        if sc.signer_identity_ref.is_empty() {
            return Err(anyhow!("missing signer identity reference"));
        }

        if closure.flake_lock_digest.is_empty() {
            return Err(anyhow!("missing flake.lock digest"));
        }
        if closure.derivation_digest.is_empty() {
            return Err(anyhow!("missing derivation digest"));
        }
        if closure.runtime_closure_hash.is_empty() {
            return Err(anyhow!("missing runtime closure hash"));
        }
        if closure.store_paths.is_empty() {
            return Err(anyhow!("missing store paths"));
        }
        if closure.config_digest.is_empty() {
            return Err(anyhow!("missing config digest"));
        }
        if closure.generation_id.is_empty() {
            return Err(anyhow!("missing generation id"));
        }

        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct Sha256CryptoService;

impl CryptoService for Sha256CryptoService {
    fn digest_hex(&self, data: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(data.as_bytes());
        hex::encode(hasher.finalize())
    }
}
