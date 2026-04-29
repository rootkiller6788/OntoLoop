use std::{
    collections::{BTreeMap, HashSet},
    sync::Arc,
};

use anyhow::{Result, bail};
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use tokio::sync::RwLock;

use super::{BridgeRuntimeStatus, TransportBridgeRuntime, parse_transport_kind};

#[derive(Debug, Clone)]
enum JwtSigner {
    Hs256 {
        secret: String,
    },
    Rs256 {
        private_pem: String,
        public_pem: String,
    },
}

#[derive(Debug, Clone)]
pub(crate) struct JwtPolicy {
    issuer: String,
    audience: String,
    max_ttl_ms: u64,
    signer: JwtSigner,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct JwtSessionClaims {
    pub iss: String,
    pub aud: String,
    pub sub: String,
    pub jti: String,
    pub exp: usize,
    pub iat: usize,
    pub nbf: usize,
    pub session_id: String,
    pub tenant_id: String,
    pub policy_version: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RemoteSessionStatus {
    pub session_id: String,
    pub token_issued: bool,
    pub bridge_running: bool,
    pub subject: Option<String>,
    pub tenant_id: Option<String>,
    pub expires_at_ms: Option<u64>,
    pub algorithm: String,
}

#[derive(Debug, Clone)]
struct RemoteSessionRecord {
    claims: JwtSessionClaims,
    token: String,
}

#[derive(Clone)]
pub struct RemoteSessionRunner {
    bridge: TransportBridgeRuntime,
    policy: JwtPolicy,
    sessions: Arc<RwLock<BTreeMap<String, RemoteSessionRecord>>>,
    consumed_jti: Arc<RwLock<HashSet<String>>>,
}

impl RemoteSessionRunner {
    pub(crate) fn new(bridge: TransportBridgeRuntime, policy: JwtPolicy) -> Self {
        Self {
            bridge,
            policy,
            sessions: Arc::new(RwLock::new(BTreeMap::new())),
            consumed_jti: Arc::new(RwLock::new(HashSet::new())),
        }
    }

    pub fn from_env(bridge: TransportBridgeRuntime) -> Self {
        let algorithm = std::env::var("AUTOLOOP_BRIDGE_JWT_ALG")
            .unwrap_or_else(|_| "HS256".to_string())
            .to_ascii_uppercase();
        let issuer = std::env::var("AUTOLOOP_BRIDGE_JWT_ISSUER")
            .unwrap_or_else(|_| "autoloop-bridge".to_string());
        let audience = std::env::var("AUTOLOOP_BRIDGE_JWT_AUDIENCE")
            .unwrap_or_else(|_| "autoloop-remote".to_string());
        let max_ttl_ms = std::env::var("AUTOLOOP_BRIDGE_JWT_MAX_TTL_MS")
            .ok()
            .and_then(|raw| raw.parse::<u64>().ok())
            .unwrap_or(3_600_000);

        let signer = if algorithm == "RS256" {
            let private_pem =
                std::env::var("AUTOLOOP_BRIDGE_JWT_PRIVATE_KEY_PEM").unwrap_or_default();
            let public_pem =
                std::env::var("AUTOLOOP_BRIDGE_JWT_PUBLIC_KEY_PEM").unwrap_or_default();
            if private_pem.is_empty() || public_pem.is_empty() {
                JwtSigner::Hs256 {
                    secret: std::env::var("AUTOLOOP_BRIDGE_JWT_SECRET")
                        .unwrap_or_else(|_| "autoloop-dev-bridge-secret".to_string()),
                }
            } else {
                JwtSigner::Rs256 {
                    private_pem,
                    public_pem,
                }
            }
        } else {
            JwtSigner::Hs256 {
                secret: std::env::var("AUTOLOOP_BRIDGE_JWT_SECRET")
                    .unwrap_or_else(|_| "autoloop-dev-bridge-secret".to_string()),
            }
        };

        Self::new(
            bridge,
            JwtPolicy {
                issuer,
                audience,
                max_ttl_ms,
                signer,
            },
        )
    }

    pub async fn issue_token(
        &self,
        session_id: &str,
        subject: &str,
        tenant_id: &str,
        ttl_ms: u64,
    ) -> Result<String> {
        let now_ms = current_time_ms();
        let bounded_ttl = ttl_ms.min(self.policy.max_ttl_ms).max(30_000);
        let iat = (now_ms / 1000) as usize;
        let exp = ((now_ms + bounded_ttl) / 1000) as usize;
        let claims = JwtSessionClaims {
            iss: self.policy.issuer.clone(),
            aud: self.policy.audience.clone(),
            sub: subject.to_string(),
            jti: format!("jti:{session_id}:{now_ms}"),
            exp,
            iat,
            nbf: iat,
            session_id: session_id.to_string(),
            tenant_id: tenant_id.to_string(),
            policy_version: "v1".to_string(),
        };
        let token = self.encode_claims(&claims)?;
        self.sessions.write().await.insert(
            session_id.to_string(),
            RemoteSessionRecord {
                claims: claims.clone(),
                token: token.clone(),
            },
        );
        self.bridge
            .record_remote_audit(
                session_id,
                "jwt_issued",
                &serde_json::json!({
                    "subject": subject,
                    "tenant_id": tenant_id,
                    "jti": claims.jti,
                    "algorithm": self.algorithm_name(),
                    "exp": claims.exp,
                    "iss": claims.iss,
                    "aud": claims.aud,
                }),
            )
            .await?;
        Ok(token)
    }

    pub async fn remote_start(
        &self,
        session_id: &str,
        transport_kind: &str,
        jwt_token: &str,
        ttl_ms: u64,
    ) -> Result<BridgeRuntimeStatus> {
        let claims = self.validate_token(session_id, jwt_token).await?;
        let kind = parse_transport_kind(transport_kind);
        let now_ms = current_time_ms();
        let bridge_ttl = ttl_ms
            .min(
                (claims.exp as u64)
                    .saturating_mul(1000)
                    .saturating_sub(now_ms),
            )
            .max(10_000);
        let status = self
            .bridge
            .start(session_id, kind, &claims.sub, &claims.tenant_id, bridge_ttl)
            .await?;
        self.bridge
            .record_remote_audit(
                session_id,
                "remote_start_admitted",
                &serde_json::json!({
                    "jti": claims.jti,
                    "subject": claims.sub,
                    "tenant_id": claims.tenant_id,
                    "algorithm": self.algorithm_name(),
                    "policy_version": claims.policy_version,
                }),
            )
            .await?;
        Ok(status)
    }

    pub async fn remote_status(&self, session_id: &str) -> Result<RemoteSessionStatus> {
        let bridge_status = self.bridge.status(session_id).await?;
        let record = self.sessions.read().await.get(session_id).cloned();
        Ok(RemoteSessionStatus {
            session_id: session_id.to_string(),
            token_issued: record.is_some(),
            bridge_running: bridge_status.running,
            subject: record.as_ref().map(|item| item.claims.sub.clone()),
            tenant_id: record.as_ref().map(|item| item.claims.tenant_id.clone()),
            expires_at_ms: record
                .as_ref()
                .map(|item| (item.claims.exp as u64).saturating_mul(1000)),
            algorithm: self.algorithm_name().to_string(),
        })
    }

    pub async fn remote_stop(&self, session_id: &str) -> Result<BridgeRuntimeStatus> {
        self.bridge
            .record_remote_audit(
                session_id,
                "remote_stop",
                &serde_json::json!({
                    "session_id": session_id,
                }),
            )
            .await?;
        self.bridge.stop(session_id).await
    }

    pub async fn validate_token(
        &self,
        session_id: &str,
        jwt_token: &str,
    ) -> Result<JwtSessionClaims> {
        let claims = self.decode_claims(jwt_token)?;
        if claims.session_id != session_id {
            self.audit_reject(
                session_id,
                "session_binding_mismatch",
                &serde_json::json!({
                    "token_session": claims.session_id,
                    "requested_session": session_id,
                }),
            )
            .await;
            bail!(
                "jwt session mismatch: token_session={} requested_session={}",
                claims.session_id,
                session_id
            );
        }
        if claims.exp <= (current_time_ms() / 1000) as usize {
            self.audit_reject(
                session_id,
                "token_expired",
                &serde_json::json!({
                    "exp": claims.exp,
                }),
            )
            .await;
            bail!("jwt session token expired");
        }
        if claims.iss != self.policy.issuer || claims.aud != self.policy.audience {
            self.audit_reject(
                session_id,
                "policy_binding_mismatch",
                &serde_json::json!({
                    "iss": claims.iss,
                    "aud": claims.aud,
                }),
            )
            .await;
            bail!("jwt claims policy mismatch");
        }
        if let Some(record) = self.sessions.read().await.get(session_id) {
            if record.token != jwt_token {
                self.audit_reject(
                    session_id,
                    "token_rotated",
                    &serde_json::json!({
                        "session_id": session_id,
                    }),
                )
                .await;
                bail!("jwt token revoked or rotated");
            }
        }
        let mut consumed = self.consumed_jti.write().await;
        if consumed.contains(&claims.jti) {
            self.audit_reject(
                session_id,
                "token_replay",
                &serde_json::json!({
                    "jti": claims.jti,
                }),
            )
            .await;
            bail!("jwt jti replay detected");
        }
        consumed.insert(claims.jti.clone());
        Ok(claims)
    }

    fn encode_claims(&self, claims: &JwtSessionClaims) -> Result<String> {
        match &self.policy.signer {
            JwtSigner::Hs256 { secret } => Ok(encode(
                &Header::new(Algorithm::HS256),
                claims,
                &EncodingKey::from_secret(secret.as_bytes()),
            )?),
            JwtSigner::Rs256 { private_pem, .. } => Ok(encode(
                &Header::new(Algorithm::RS256),
                claims,
                &EncodingKey::from_rsa_pem(private_pem.as_bytes())?,
            )?),
        }
    }

    fn decode_claims(&self, token: &str) -> Result<JwtSessionClaims> {
        let mut validation = Validation::new(match &self.policy.signer {
            JwtSigner::Hs256 { .. } => Algorithm::HS256,
            JwtSigner::Rs256 { .. } => Algorithm::RS256,
        });
        validation.set_audience(&[self.policy.audience.clone()]);
        validation.set_issuer(&[self.policy.issuer.clone()]);
        let data = match &self.policy.signer {
            JwtSigner::Hs256 { secret } => decode::<JwtSessionClaims>(
                token,
                &DecodingKey::from_secret(secret.as_bytes()),
                &validation,
            )?,
            JwtSigner::Rs256 { public_pem, .. } => decode::<JwtSessionClaims>(
                token,
                &DecodingKey::from_rsa_pem(public_pem.as_bytes())?,
                &validation,
            )?,
        };
        Ok(data.claims)
    }

    fn algorithm_name(&self) -> &'static str {
        match &self.policy.signer {
            JwtSigner::Hs256 { .. } => "HS256",
            JwtSigner::Rs256 { .. } => "RS256",
        }
    }

    async fn audit_reject(&self, session_id: &str, reason: &str, payload: &serde_json::Value) {
        let _ = self
            .bridge
            .record_remote_audit(
                session_id,
                "remote_start_rejected",
                &serde_json::json!({
                    "reason": reason,
                    "payload": payload,
                    "algorithm": self.algorithm_name(),
                }),
            )
            .await;
    }
}

fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}
