use std::{collections::HashMap, env, process::Command, thread, time::Duration};

use anyhow::{Context, Result};
use autoloop_state_adapter::StateStore;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tracing::warn;

use crate::config::{ResearchBackend, ResearchConfig};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ResearchSourceKind {
    Official,
    Documentation,
    News,
    Community,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveryQuery {
    pub query: String,
    pub rationale: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResearchCandidate {
    pub url: String,
    pub source_kind: ResearchSourceKind,
    pub relevance: f32,
    pub trust_score: f32,
    pub selected_reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResearchArtifact {
    pub url: String,
    pub title: String,
    pub markdown: String,
    pub extracted_summary: String,
    pub discovered_at_ms: u64,
    pub source_kind: ResearchSourceKind,
    pub fetched_via: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResearchReport {
    pub anchor_id: String,
    pub topic: String,
    pub queries: Vec<DiscoveryQuery>,
    pub candidates: Vec<ResearchCandidate>,
    pub artifacts: Vec<ResearchArtifact>,
    pub knowledge_gaps: Vec<String>,
    pub autonomy_score: f32,
    pub backend_used: String,
    pub backend_warnings: Vec<String>,
    pub used_proxies: Vec<String>,
    pub exhausted_proxies: Vec<String>,
    pub open_circuit_proxies: Vec<String>,
    pub research_hops: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResearchHealthReport {
    pub backend: String,
    pub live_fetch_enabled: bool,
    pub curl_available: bool,
    pub node_available: bool,
    pub browser_render_configured: bool,
    pub firecrawl_configured: bool,
    pub proxy_pool_size: usize,
    pub browser_session_pool_size: usize,
    pub anti_bot_profile: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyFailureForensics {
    pub session_id: String,
    pub backend_used: String,
    pub total_warnings: usize,
    pub proxy_provider_name: String,
    pub proxy_pool_size: usize,
    pub anti_bot_profile: String,
    pub proxy_request_quota_per_proxy: usize,
    pub breaker_failure_threshold: usize,
    pub breaker_cooldown_ms: u64,
    pub used_proxies: Vec<String>,
    pub exhausted_proxies: Vec<String>,
    pub open_circuit_proxies: Vec<String>,
    pub warning_samples: Vec<String>,
    pub likely_proxy_pressure: bool,
}

#[derive(Debug, Clone)]
pub struct ResearchKernel {
    config: ResearchConfig,
}

#[derive(Debug)]
struct ResearchExecution {
    backend_used: String,
    candidates: Vec<ResearchCandidate>,
    artifacts: Vec<ResearchArtifact>,
    warnings: Vec<String>,
    used_proxies: Vec<String>,
    exhausted_proxies: Vec<String>,
    open_circuit_proxies: Vec<String>,
}

#[derive(Default)]
struct ProxyGovernanceState {
    usage_counts: HashMap<String, usize>,
    failure_counts: HashMap<String, usize>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FirecrawlSearchResponse {
    success: bool,
    data: FirecrawlSearchData,
    warning: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FirecrawlSearchData {
    web: Option<Vec<FirecrawlSearchResultOrDocument>>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum FirecrawlSearchResultOrDocument {
    WebResult(FirecrawlWebResult),
    Document(FirecrawlDocument),
}

#[derive(Debug, Deserialize)]
struct FirecrawlWebResult {
    url: String,
    title: Option<String>,
    description: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FirecrawlDocument {
    url: Option<String>,
    markdown: Option<String>,
    metadata: Option<FirecrawlMetadata>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FirecrawlMetadata {
    source_url: Option<String>,
    title: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GenericScrapeResponse {
    success: Option<bool>,
    url: Option<String>,
    title: Option<String>,
    markdown: Option<String>,
    content: Option<String>,
    data: Option<GenericScrapeData>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GenericScrapeData {
    url: Option<String>,
    title: Option<String>,
    markdown: Option<String>,
    content: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BrowserRenderResponse {
    success: Option<bool>,
    data: Option<BrowserRenderData>,
    warning: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BrowserRenderData {
    url: Option<String>,
    title: Option<String>,
    markdown: Option<String>,
    html: Option<String>,
    content: Option<String>,
}

impl ResearchKernel {
    pub fn from_config(config: &ResearchConfig) -> Self {
        Self {
            config: config.clone(),
        }
    }

    pub fn validate(&self) -> Result<()> {
        if self.config.enabled && self.config.max_queries_per_anchor == 0 {
            anyhow::bail!("research.max_queries_per_anchor must be greater than 0");
        }
        if self.config.enabled && self.config.max_candidates_per_query == 0 {
            anyhow::bail!("research.max_candidates_per_query must be greater than 0");
        }
        if self.config.enabled && self.config.request_timeout_secs == 0 {
            anyhow::bail!("research.request_timeout_secs must be greater than 0");
        }
        if self.config.enabled && self.config.retry_attempts == 0 {
            anyhow::bail!("research.retry_attempts must be greater than 0");
        }
        if matches!(self.config.backend, ResearchBackend::Firecrawl)
            && self.config.firecrawl_api_key_env.trim().is_empty()
        {
            anyhow::bail!("research.firecrawl_api_key_env must not be empty");
        }
        Ok(())
    }

    pub async fn run_anchor_research(
        &self,
        db: &StateStore,
        session_id: &str,
        anchor_text: &str,
    ) -> Result<ResearchReport> {
        if !self.config.enabled {
            return Ok(ResearchReport {
                anchor_id: format!("anchor:{session_id}"),
                topic: anchor_text.to_string(),
                queries: Vec::new(),
                candidates: Vec::new(),
                artifacts: Vec::new(),
                knowledge_gaps: vec!["research subsystem disabled".into()],
                autonomy_score: 0.0,
                backend_used: "disabled".into(),
                backend_warnings: Vec::new(),
                used_proxies: Vec::new(),
                exhausted_proxies: Vec::new(),
                open_circuit_proxies: Vec::new(),
                research_hops: 0,
            });
        }

        let anchor_id = format!("anchor:{session_id}");
        let queries = build_queries(anchor_text, &self.config);
        let execution = self.execute_research(anchor_text, &queries).await?;
        let mut candidates = execution.candidates;
        normalize_candidates(&mut candidates);
        let knowledge_gaps = infer_gaps(
            anchor_text,
            &candidates,
            &execution.artifacts,
            &execution.warnings,
        );
        let autonomy_score = compute_autonomy_score(
            &queries,
            &candidates,
            &execution.artifacts,
            &knowledge_gaps,
            &execution.backend_used,
        );

        let research_hops = queries.len();
        let report = ResearchReport {
            anchor_id: anchor_id.clone(),
            topic: anchor_text.to_string(),
            queries,
            candidates,
            artifacts: execution.artifacts,
            knowledge_gaps,
            autonomy_score,
            backend_used: execution.backend_used,
            backend_warnings: execution.warnings,
            used_proxies: execution.used_proxies,
            exhausted_proxies: execution.exhausted_proxies,
            open_circuit_proxies: execution.open_circuit_proxies,
            research_hops,
        };
        self.persist_report(db, session_id, &report).await?;
        Ok(report)
    }

    pub fn health_report(&self) -> ResearchHealthReport {
        ResearchHealthReport {
            backend: format!("{:?}", self.config.backend),
            live_fetch_enabled: self.config.live_fetch_enabled,
            curl_available: command_available(curl_binary(), "--version"),
            node_available: command_available(&self.config.playwright_node_binary, "--version"),
            browser_render_configured: self.config.browser_render_url.is_some(),
            firecrawl_configured: self.firecrawl_credentials().is_some(),
            proxy_pool_size: self.config.proxy_pool.len(),
            browser_session_pool_size: self.config.browser_session_pool.len(),
            anti_bot_profile: self.config.anti_bot_profile.clone(),
        }
    }

    pub async fn schedule_follow_up_research(
        &self,
        db: &StateStore,
        session_id: &str,
        actor_id: &str,
        report: &ResearchReport,
    ) -> Result<usize> {
        if !db
            .has_permission(
                actor_id,
                autoloop_state_adapter::PermissionAction::Dispatch,
            )
            .await?
        {
            return Ok(0);
        }

        let mut scheduled = 0usize;
        for gap in report.knowledge_gaps.iter().take(3) {
            db.create_schedule_event(
                session_id.to_string(),
                "research.follow_up".into(),
                "mcp::local-mcp::invoke".into(),
                serde_json::to_string(&json!({
                    "anchor_id": report.anchor_id,
                    "topic": report.topic,
                    "gap": gap,
                    "backend_used": report.backend_used,
                }))?,
                actor_id.to_string(),
            )
            .await?;
            scheduled += 1;
        }

        if report.backend_used != "synthetic" && report.knowledge_gaps.len() < 2 {
            db.create_schedule_event(
                session_id.to_string(),
                "research.refresh".into(),
                "mcp::local-mcp::invoke".into(),
                serde_json::to_string(&json!({
                    "anchor_id": report.anchor_id,
                    "topic": report.topic,
                    "reason": "freshness-check",
                }))?,
                actor_id.to_string(),
            )
            .await?;
            scheduled += 1;
        }

        Ok(scheduled)
    }

    async fn execute_research(
        &self,
        anchor_text: &str,
        queries: &[DiscoveryQuery],
    ) -> Result<ResearchExecution> {
        match self.config.backend {
            ResearchBackend::Synthetic => Ok(self.synthetic_execution(anchor_text, queries)),
            ResearchBackend::BrowserFetch => {
                self.browser_fetch_with_policy_fallback(anchor_text, queries)
                    .await
            }
            ResearchBackend::PlaywrightCli => {
                self.playwright_cli_execution(anchor_text, queries).await
            }
            ResearchBackend::Firecrawl => self.firecrawl_execution(anchor_text, queries).await,
            ResearchBackend::SelfHostedScraper => {
                self.self_hosted_execution(anchor_text, queries).await
            }
            ResearchBackend::WebFetch => self.web_fetch_execution(anchor_text, queries).await,
            ResearchBackend::Auto => self.auto_execution(anchor_text, queries).await,
        }
    }

    async fn browser_fetch_with_policy_fallback(
        &self,
        anchor_text: &str,
        queries: &[DiscoveryQuery],
    ) -> Result<ResearchExecution> {
        let unavailable_reason = self.browser_fetch_unavailable_reason();
        if let Some(reason) = unavailable_reason {
            let mut downgraded = self
                .fallback_execution(
                    anchor_text,
                    queries,
                    Some("browser_fetch"),
                    Some(reason.clone()),
                )
                .await?;
            downgraded.warnings.insert(
                0,
                format!(
                    "capability_negotiation: browser_fetch unavailable ({reason}); downgraded_to={}",
                    downgraded.backend_used
                ),
            );
            return Ok(downgraded);
        }

        match self.browser_fetch_execution(anchor_text, queries).await {
            Ok(execution) if !execution.artifacts.is_empty() => Ok(execution),
            Ok(_) => self
                .fallback_execution(
                    anchor_text,
                    queries,
                    Some("browser_fetch"),
                    Some("browser-fetch returned no artifacts".to_string()),
                )
                .await,
            Err(err) => self
                .fallback_execution(
                    anchor_text,
                    queries,
                    Some("browser_fetch"),
                    Some(err.to_string()),
                )
                .await,
        }
    }

    async fn fallback_execution(
        &self,
        anchor_text: &str,
        queries: &[DiscoveryQuery],
        failed_backend: Option<&str>,
        failure_reason: Option<String>,
    ) -> Result<ResearchExecution> {
        let mut warnings = Vec::<String>::new();
        if let Some(backend) = failed_backend {
            warnings.push(format!(
                "capability_negotiation: backend={backend} fallback_reason={}",
                failure_reason.as_deref().unwrap_or("unspecified")
            ));
        }

        if self.config.live_fetch_enabled {
            match self.playwright_cli_execution(anchor_text, queries).await {
                Ok(mut execution) if !execution.artifacts.is_empty() => {
                    execution.warnings.splice(0..0, warnings);
                    return Ok(execution);
                }
                Ok(_) => warn!("playwright-cli execution returned no artifacts during fallback"),
                Err(err) => warn!("playwright-cli execution failed during fallback: {err:#}"),
            }

            if self.firecrawl_credentials().is_some() {
                match self.firecrawl_execution(anchor_text, queries).await {
                    Ok(mut execution) if !execution.artifacts.is_empty() => {
                        execution.warnings.splice(0..0, warnings);
                        return Ok(execution);
                    }
                    Ok(_) => warn!("firecrawl execution returned no artifacts during fallback"),
                    Err(err) => warn!("firecrawl execution failed during fallback: {err:#}"),
                }
            }

            if self.config.self_hosted_scraper_url.is_some() {
                match self.self_hosted_execution(anchor_text, queries).await {
                    Ok(mut execution) if !execution.artifacts.is_empty() => {
                        execution.warnings.splice(0..0, warnings);
                        return Ok(execution);
                    }
                    Ok(_) => warn!("self-hosted scrape returned no artifacts during fallback"),
                    Err(err) => warn!("self-hosted scrape failed during fallback: {err:#}"),
                }
            }

            match self.web_fetch_execution(anchor_text, queries).await {
                Ok(mut execution) if !execution.artifacts.is_empty() => {
                    execution.warnings.splice(0..0, warnings);
                    return Ok(execution);
                }
                Ok(_) => warn!("web-fetch returned no artifacts during fallback"),
                Err(err) => warn!("web-fetch failed during fallback: {err:#}"),
            }
        }

        let mut synthetic = self.synthetic_execution(anchor_text, queries);
        synthetic.warnings.splice(0..0, warnings);
        Ok(synthetic)
    }

    fn browser_fetch_unavailable_reason(&self) -> Option<String> {
        if !self.config.live_fetch_enabled {
            return Some("live_fetch_disabled".into());
        }
        if self
            .config
            .browser_render_url
            .as_deref()
            .map(str::trim)
            .map(str::is_empty)
            .unwrap_or(true)
        {
            return Some("missing_browser_render_url".into());
        }
        if !command_available(curl_binary(), "--version") {
            return Some("curl_not_available".into());
        }
        None
    }

    async fn auto_execution(
        &self,
        anchor_text: &str,
        queries: &[DiscoveryQuery],
    ) -> Result<ResearchExecution> {
        if !self.config.live_fetch_enabled {
            return Ok(self.synthetic_execution(anchor_text, queries));
        }

        if self.config.browser_render_url.is_some() {
            match self.browser_fetch_execution(anchor_text, queries).await {
                Ok(execution) if !execution.artifacts.is_empty() => return Ok(execution),
                Ok(_) => warn!("browser-fetch execution returned no artifacts, falling back"),
                Err(err) => warn!("browser-fetch execution failed: {err:#}"),
            }
        }

        match self.playwright_cli_execution(anchor_text, queries).await {
            Ok(execution) if !execution.artifacts.is_empty() => return Ok(execution),
            Ok(_) => warn!("playwright-cli execution returned no artifacts, falling back"),
            Err(err) => warn!("playwright-cli execution failed: {err:#}"),
        }

        if self.firecrawl_credentials().is_some() {
            match self.firecrawl_execution(anchor_text, queries).await {
                Ok(execution) if !execution.artifacts.is_empty() => return Ok(execution),
                Ok(_) => warn!("firecrawl execution returned no artifacts, falling back"),
                Err(err) => warn!("firecrawl execution failed: {err:#}"),
            }
        }

        if self.config.self_hosted_scraper_url.is_some() {
            match self.self_hosted_execution(anchor_text, queries).await {
                Ok(execution) if !execution.artifacts.is_empty() => return Ok(execution),
                Ok(_) => warn!("self-hosted scrape returned no artifacts, falling back"),
                Err(err) => warn!("self-hosted scrape failed: {err:#}"),
            }
        }

        self.fallback_execution(anchor_text, queries, None, None).await
    }

    async fn browser_fetch_execution(
        &self,
        anchor_text: &str,
        queries: &[DiscoveryQuery],
    ) -> Result<ResearchExecution> {
        if !self.config.live_fetch_enabled {
            anyhow::bail!("live fetch disabled");
        }
        let endpoint = self
            .config
            .browser_render_url
            .clone()
            .context("missing browser render url")?;
        let mut candidates = Vec::new();
        for query in queries {
            candidates.extend(discover_candidates(query, anchor_text, &self.config));
        }
        normalize_candidates(&mut candidates);

        let mut artifacts = Vec::new();
        let mut warnings = Vec::new();
        let mut governance = ProxyGovernanceState::default();
        for (index, candidate) in candidates
            .iter()
            .take(self.config.max_candidates_per_query)
            .enumerate()
        {
            let session = self.browser_session(index);
            let Some(proxy) = self.reserve_proxy(&mut governance, candidate.url.as_str(), index)
            else {
                warnings.push(format!(
                    "proxy governance blocked browser render for {}",
                    candidate.url
                ));
                continue;
            };
            let body = json!({
                "url": candidate.url,
                "timeout_ms": self.timeout_ms(),
                "render": true,
                "wait_for_ms": self.render_wait_ms(),
                "user_agent": self.config.user_agent,
                "anti_bot_profile": self.config.anti_bot_profile,
                "browser_session": session,
                "proxy": proxy,
                "format": "markdown",
            });
            match self.json_post(&endpoint, &body, &[]) {
                Ok(value) => {
                    let response: BrowserRenderResponse = serde_json::from_value(value)
                        .context("failed to parse browser render response")?;
                    if let Some(warning) = response.warning {
                        warnings.push(warning);
                    }
                    let data = response
                        .data
                        .context("browser render response missing data")?;
                    let markdown = data
                        .markdown
                        .or_else(|| data.content.clone())
                        .or_else(|| data.html.as_ref().map(|html| html_to_markdownish(html)))
                        .unwrap_or_default();
                    if markdown.trim().is_empty() {
                        warnings.push(format!(
                            "browser renderer returned empty body for {}",
                            candidate.url
                        ));
                        continue;
                    }
                    artifacts.push(ResearchArtifact {
                        url: data.url.unwrap_or_else(|| candidate.url.clone()),
                        title: data.title.unwrap_or_else(|| summarize_url(&candidate.url)),
                        extracted_summary: summarize_text(&markdown, 280),
                        markdown,
                        discovered_at_ms: crate::orchestration::current_time_ms(),
                        source_kind: candidate.source_kind.clone(),
                        fetched_via: "browser_fetch".into(),
                    });
                    if response.success == Some(false) {
                        warnings.push(format!(
                            "browser renderer marked unsuccessful for {}",
                            candidate.url
                        ));
                    }
                }
                Err(err) => {
                    governance.record_failure(Some(&proxy));
                    warnings.push(format!(
                        "browser render failed for {}: {err:#}",
                        candidate.url
                    ));
                }
            }
        }

        if artifacts.is_empty() {
            anyhow::bail!("browser-fetch backend produced no artifacts");
        }
        Ok(ResearchExecution {
            backend_used: "browser_fetch".into(),
            candidates,
            artifacts,
            warnings,
            used_proxies: governance.used_proxies(),
            exhausted_proxies: governance
                .exhausted_proxies(self.config.proxy_request_quota_per_proxy),
            open_circuit_proxies: governance
                .open_circuit_proxies(self.config.proxy_breaker_failure_threshold),
        })
    }

    async fn playwright_cli_execution(
        &self,
        anchor_text: &str,
        queries: &[DiscoveryQuery],
    ) -> Result<ResearchExecution> {
        if !self.config.live_fetch_enabled {
            anyhow::bail!("live fetch disabled");
        }
        let mut candidates = Vec::new();
        for query in queries {
            candidates.extend(discover_candidates(query, anchor_text, &self.config));
        }
        normalize_candidates(&mut candidates);

        let mut artifacts = Vec::new();
        let mut warnings = Vec::new();
        let mut governance = ProxyGovernanceState::default();
        for (index, candidate) in candidates
            .iter()
            .take(self.config.max_candidates_per_query)
            .enumerate()
        {
            match self.playwright_cli_artifact(candidate, index, &mut governance) {
                Ok(artifact) => artifacts.push(artifact),
                Err(err) => warnings.push(format!(
                    "playwright cli failed for {}: {err:#}",
                    candidate.url
                )),
            }
        }

        if artifacts.is_empty() {
            anyhow::bail!("playwright-cli backend produced no artifacts");
        }
        Ok(ResearchExecution {
            backend_used: "playwright_cli".into(),
            candidates,
            artifacts,
            warnings,
            used_proxies: governance.used_proxies(),
            exhausted_proxies: governance
                .exhausted_proxies(self.config.proxy_request_quota_per_proxy),
            open_circuit_proxies: governance
                .open_circuit_proxies(self.config.proxy_breaker_failure_threshold),
        })
    }

    async fn firecrawl_execution(
        &self,
        anchor_text: &str,
        queries: &[DiscoveryQuery],
    ) -> Result<ResearchExecution> {
        if !self.config.live_fetch_enabled {
            anyhow::bail!("live fetch disabled");
        }
        let api_key = self
            .firecrawl_credentials()
            .context("missing firecrawl api key")?;
        let mut candidates = Vec::new();
        let mut artifacts = Vec::new();
        let mut warnings = Vec::new();
        let governance = ProxyGovernanceState::default();

        for query in queries {
            let body = json!({
                "query": query.query,
                "limit": self.config.max_candidates_per_query,
                "scrapeOptions": {
                    "formats": ["markdown"],
                    "onlyMainContent": true,
                    "timeout": self.timeout_ms(),
                    "waitFor": self.render_wait_ms(),
                    "mobile": false,
                }
            });
            let value = self.firecrawl_post("/search", &api_key, &body)?;
            let response: FirecrawlSearchResponse = serde_json::from_value(value)
                .context("failed to parse firecrawl search response")?;
            if !response.success {
                warnings.push(format!(
                    "firecrawl query `{}` returned unsuccessful response",
                    query.query
                ));
                continue;
            }
            if let Some(warning) = response.warning {
                warnings.push(warning);
            }
            for entry in response.data.web.unwrap_or_default() {
                match entry {
                    FirecrawlSearchResultOrDocument::WebResult(result) => {
                        candidates.push(ResearchCandidate {
                            url: result.url,
                            source_kind: classify_source_kind(
                                result
                                    .title
                                    .as_deref()
                                    .or(result.description.as_deref())
                                    .unwrap_or(anchor_text),
                            ),
                            relevance: 0.86,
                            trust_score: 0.74,
                            selected_reason: format!("firecrawl search hit for `{}`", query.query),
                        });
                    }
                    FirecrawlSearchResultOrDocument::Document(doc) => {
                        let url = doc
                            .metadata
                            .as_ref()
                            .and_then(|meta| meta.source_url.clone())
                            .or(doc.url.clone())
                            .unwrap_or_else(|| {
                                format!("firecrawl://{}", sanitize_topic(anchor_text))
                            });
                        let title = doc
                            .metadata
                            .as_ref()
                            .and_then(|meta| meta.title.clone())
                            .unwrap_or_else(|| summarize_url(&url));
                        let markdown = doc.markdown.unwrap_or_default();
                        let source_kind = classify_source_kind(&url);
                        candidates.push(ResearchCandidate {
                            url: url.clone(),
                            source_kind: source_kind.clone(),
                            relevance: 0.92,
                            trust_score: source_trust_score(&url, &source_kind),
                            selected_reason: format!(
                                "firecrawl scraped result for `{}`",
                                query.query
                            ),
                        });
                        if !markdown.trim().is_empty() {
                            artifacts.push(ResearchArtifact {
                                url,
                                title,
                                extracted_summary: summarize_text(&markdown, 280),
                                markdown,
                                discovered_at_ms: crate::orchestration::current_time_ms(),
                                source_kind,
                                fetched_via: "firecrawl".into(),
                            });
                        }
                    }
                }
            }
        }

        normalize_candidates(&mut candidates);
        if artifacts.is_empty() {
            for candidate in candidates.iter().take(self.config.max_candidates_per_query) {
                if let Ok(artifact) = self.firecrawl_scrape(candidate).await {
                    artifacts.push(artifact);
                }
            }
        }
        if artifacts.is_empty() {
            anyhow::bail!("firecrawl backend produced no artifacts");
        }
        Ok(ResearchExecution {
            backend_used: "firecrawl".into(),
            candidates,
            artifacts,
            warnings,
            used_proxies: governance.used_proxies(),
            exhausted_proxies: governance
                .exhausted_proxies(self.config.proxy_request_quota_per_proxy),
            open_circuit_proxies: governance
                .open_circuit_proxies(self.config.proxy_breaker_failure_threshold),
        })
    }

    async fn firecrawl_scrape(&self, candidate: &ResearchCandidate) -> Result<ResearchArtifact> {
        let api_key = self
            .firecrawl_credentials()
            .context("missing firecrawl api key")?;
        let body = json!({
            "url": candidate.url,
            "formats": ["markdown"],
            "onlyMainContent": true,
            "timeout": self.timeout_ms(),
            "waitFor": self.render_wait_ms(),
        });
        let response = self.firecrawl_post("/scrape", &api_key, &body)?;
        let data = response
            .get("data")
            .cloned()
            .context("firecrawl scrape response missing data")?;
        let markdown = data
            .get("markdown")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if markdown.trim().is_empty() {
            anyhow::bail!("firecrawl scrape returned empty markdown");
        }
        let title = data
            .get("metadata")
            .and_then(|metadata| metadata.get("title"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| summarize_url(&candidate.url));
        Ok(ResearchArtifact {
            url: candidate.url.clone(),
            title,
            extracted_summary: summarize_text(&markdown, 280),
            markdown,
            discovered_at_ms: crate::orchestration::current_time_ms(),
            source_kind: candidate.source_kind.clone(),
            fetched_via: "firecrawl".into(),
        })
    }

    async fn self_hosted_execution(
        &self,
        anchor_text: &str,
        queries: &[DiscoveryQuery],
    ) -> Result<ResearchExecution> {
        if !self.config.live_fetch_enabled {
            anyhow::bail!("live fetch disabled");
        }
        let endpoint = self
            .config
            .self_hosted_scraper_url
            .clone()
            .context("missing self-hosted scraper url")?;
        let mut candidates = Vec::new();
        for query in queries {
            candidates.extend(discover_candidates(query, anchor_text, &self.config));
        }
        normalize_candidates(&mut candidates);

        let mut artifacts = Vec::new();
        let mut warnings = Vec::new();
        let mut governance = ProxyGovernanceState::default();
        for candidate in candidates.iter().take(self.config.max_candidates_per_query) {
            let Some(proxy) = self.reserve_proxy(&mut governance, candidate.url.as_str(), 0) else {
                warnings.push(format!(
                    "proxy governance blocked self-hosted scrape for {}",
                    candidate.url
                ));
                continue;
            };
            let body = json!({
                "url": candidate.url,
                "timeout_ms": self.timeout_ms(),
                "user_agent": self.config.user_agent,
                "format": "markdown",
                "render": self.config.prefer_dynamic_render,
                "proxy": proxy,
            });
            match self.json_post(&normalize_scrape_endpoint(&endpoint), &body, &[]) {
                Ok(value) => {
                    let response: GenericScrapeResponse = serde_json::from_value(value)
                        .context("failed to parse self-hosted scraper response")?;
                    let markdown = response
                        .markdown
                        .or_else(|| response.content.clone())
                        .or_else(|| {
                            response
                                .data
                                .as_ref()
                                .and_then(|data| data.markdown.clone())
                        })
                        .or_else(|| response.data.as_ref().and_then(|data| data.content.clone()))
                        .unwrap_or_default();
                    if markdown.trim().is_empty() {
                        warnings.push(format!(
                            "self-hosted scraper returned empty body for {}",
                            candidate.url
                        ));
                        continue;
                    }
                    let url = response
                        .url
                        .or_else(|| response.data.as_ref().and_then(|data| data.url.clone()))
                        .unwrap_or_else(|| candidate.url.clone());
                    let title = response
                        .title
                        .or_else(|| response.data.as_ref().and_then(|data| data.title.clone()))
                        .unwrap_or_else(|| summarize_url(&url));
                    if response.success == Some(false) {
                        warnings.push(format!(
                            "self-hosted scraper marked unsuccessful for {}",
                            candidate.url
                        ));
                    }
                    artifacts.push(ResearchArtifact {
                        url,
                        title,
                        extracted_summary: summarize_text(&markdown, 280),
                        markdown,
                        discovered_at_ms: crate::orchestration::current_time_ms(),
                        source_kind: candidate.source_kind.clone(),
                        fetched_via: "self_hosted_scraper".into(),
                    });
                }
                Err(err) => {
                    governance.record_failure(Some(&proxy));
                    warnings.push(format!(
                        "self-hosted scrape failed for {}: {err:#}",
                        candidate.url
                    ));
                }
            }
        }

        if artifacts.is_empty() {
            anyhow::bail!("self-hosted scraper backend produced no artifacts");
        }
        Ok(ResearchExecution {
            backend_used: "self_hosted_scraper".into(),
            candidates,
            artifacts,
            warnings,
            used_proxies: governance.used_proxies(),
            exhausted_proxies: governance
                .exhausted_proxies(self.config.proxy_request_quota_per_proxy),
            open_circuit_proxies: governance
                .open_circuit_proxies(self.config.proxy_breaker_failure_threshold),
        })
    }

    async fn web_fetch_execution(
        &self,
        anchor_text: &str,
        queries: &[DiscoveryQuery],
    ) -> Result<ResearchExecution> {
        if !self.config.live_fetch_enabled {
            anyhow::bail!("live fetch disabled");
        }
        let mut candidates = Vec::new();
        for query in queries {
            candidates.extend(discover_candidates(query, anchor_text, &self.config));
        }
        normalize_candidates(&mut candidates);

        let mut artifacts = Vec::new();
        let mut warnings = Vec::new();
        let mut governance = ProxyGovernanceState::default();
        for candidate in candidates.iter().take(self.config.max_candidates_per_query) {
            match self.web_fetch_artifact(candidate, &mut governance) {
                Ok(artifact) => artifacts.push(artifact),
                Err(err) => {
                    warnings.push(format!("web fetch failed for {}: {err:#}", candidate.url))
                }
            }
        }

        if artifacts.is_empty() {
            anyhow::bail!("web-fetch backend produced no artifacts");
        }
        Ok(ResearchExecution {
            backend_used: "web_fetch".into(),
            candidates,
            artifacts,
            warnings,
            used_proxies: governance.used_proxies(),
            exhausted_proxies: governance
                .exhausted_proxies(self.config.proxy_request_quota_per_proxy),
            open_circuit_proxies: governance
                .open_circuit_proxies(self.config.proxy_breaker_failure_threshold),
        })
    }

    fn synthetic_execution(
        &self,
        anchor_text: &str,
        queries: &[DiscoveryQuery],
    ) -> ResearchExecution {
        let mut candidates = Vec::new();
        for query in queries {
            candidates.extend(discover_candidates(query, anchor_text, &self.config));
        }
        normalize_candidates(&mut candidates);
        let artifacts = candidates
            .iter()
            .take(self.config.max_queries_per_anchor * self.config.max_candidates_per_query)
            .map(|candidate| synthesize_artifact(candidate, anchor_text, "synthetic"))
            .collect::<Vec<_>>();
        ResearchExecution {
            backend_used: "synthetic".into(),
            candidates,
            artifacts,
            warnings: if self.config.live_fetch_enabled {
                vec!["live fetch adapters unavailable, fell back to synthetic research".into()]
            } else {
                Vec::new()
            },
            used_proxies: Vec::new(),
            exhausted_proxies: Vec::new(),
            open_circuit_proxies: Vec::new(),
        }
    }

    async fn persist_report(
        &self,
        db: &StateStore,
        session_id: &str,
        report: &ResearchReport,
    ) -> Result<()> {
        db.upsert_json_knowledge(
            format!("research:{session_id}:report"),
            report,
            "autonomous-research",
        )
        .await?;
        db.upsert_json_knowledge(
            format!("{}:research", report.anchor_id),
            report,
            "autonomous-research",
        )
        .await?;
        for (index, artifact) in report.artifacts.iter().enumerate() {
            db.upsert_json_knowledge(
                format!("research:{session_id}:artifact:{index}"),
                artifact,
                "autonomous-research",
            )
            .await?;
        }
        let proxy_forensics = ProxyFailureForensics {
            session_id: session_id.to_string(),
            backend_used: report.backend_used.clone(),
            total_warnings: report.backend_warnings.len(),
            proxy_provider_name: self.config.proxy_provider_name.clone(),
            proxy_pool_size: self.config.proxy_pool.len(),
            anti_bot_profile: self.config.anti_bot_profile.clone(),
            proxy_request_quota_per_proxy: self.config.proxy_request_quota_per_proxy,
            breaker_failure_threshold: self.config.proxy_breaker_failure_threshold,
            breaker_cooldown_ms: self.config.proxy_breaker_cooldown_ms,
            used_proxies: report.used_proxies.clone(),
            exhausted_proxies: report.exhausted_proxies.clone(),
            open_circuit_proxies: report.open_circuit_proxies.clone(),
            warning_samples: report.backend_warnings.iter().take(5).cloned().collect(),
            likely_proxy_pressure: report.backend_warnings.iter().any(|warning| {
                let lowered = warning.to_ascii_lowercase();
                lowered.contains("403")
                    || lowered.contains("429")
                    || lowered.contains("timeout")
                    || lowered.contains("captcha")
                    || lowered.contains("blocked")
            }),
        };
        db.upsert_json_knowledge(
            format!("research:{session_id}:proxy-forensics"),
            &proxy_forensics,
            "autonomous-research",
        )
        .await?;
        Ok(())
    }

    fn firecrawl_credentials(&self) -> Option<String> {
        env::var(&self.config.firecrawl_api_key_env)
            .ok()
            .filter(|value| !value.trim().is_empty())
    }

    fn firecrawl_post(&self, path: &str, api_key: &str, body: &Value) -> Result<Value> {
        let url = firecrawl_endpoint(&self.config.firecrawl_api_url, path);
        self.json_post(
            &url,
            body,
            &[
                ("Authorization", format!("Bearer {api_key}")),
                ("Content-Type", "application/json".into()),
            ],
        )
    }

    fn json_post(&self, url: &str, body: &Value, headers: &[(&str, String)]) -> Result<Value> {
        let mut last_error = None;
        for attempt in 1..=self.config.retry_attempts {
            let mut command = Command::new(curl_binary());
            command.args([
                "-sS",
                "-L",
                "--compressed",
                "-X",
                "POST",
                "--max-time",
                &self.config.request_timeout_secs.to_string(),
                "-H",
                "Accept: application/json",
                "-H",
                "Accept-Language: en-US,en;q=0.9,zh-CN;q=0.8",
                "-H",
                &format!("User-Agent: {}", self.config.user_agent),
                "-H",
                &format!("X-AntiBot-Profile: {}", self.config.anti_bot_profile),
            ]);
            for (name, value) in headers {
                command.arg("-H").arg(format!("{name}: {value}"));
            }
            if let Some(proxy) = self.proxy_for_body(body) {
                command.arg("-x").arg(proxy);
            }
            command.arg("--data").arg(body.to_string()).arg(url);
            match command
                .output()
                .with_context(|| format!("failed to invoke curl for {url}"))
            {
                Ok(output) if output.status.success() => {
                    return serde_json::from_slice(&output.stdout)
                        .with_context(|| format!("failed to decode JSON response from {url}"));
                }
                Ok(output) => {
                    last_error = Some(anyhow::anyhow!(
                        "curl POST {} failed: {}",
                        url,
                        String::from_utf8_lossy(&output.stderr)
                    ));
                }
                Err(err) => last_error = Some(err),
            }
            if attempt < self.config.retry_attempts {
                thread::sleep(Duration::from_millis(self.config.stability_backoff_ms));
            }
        }
        Err(last_error.unwrap_or_else(|| anyhow::anyhow!("curl POST {} failed", url)))
    }

    fn web_fetch_artifact(
        &self,
        candidate: &ResearchCandidate,
        governance: &mut ProxyGovernanceState,
    ) -> Result<ResearchArtifact> {
        let fetch_url = if self.config.prefer_dynamic_render {
            maybe_enable_dynamic_render(&candidate.url)
        } else {
            candidate.url.clone()
        };
        let Some(proxy) = self.reserve_proxy(governance, candidate.url.as_str(), 0) else {
            anyhow::bail!("proxy governance blocked request");
        };
        let mut last_error = None;
        for attempt in 1..=self.config.retry_attempts {
            let mut command = Command::new(curl_binary());
            command.args([
                "-sS",
                "-L",
                "--compressed",
                "--max-time",
                &self.config.request_timeout_secs.to_string(),
                "-A",
                &self.config.user_agent,
                "-H",
                "Accept-Language: en-US,en;q=0.9,zh-CN;q=0.8",
                "-H",
                "Cache-Control: no-cache",
                "-e",
                candidate.url.as_str(),
                &fetch_url,
            ]);
            command.arg("-x").arg(&proxy);
            match command
                .output()
                .with_context(|| format!("failed to invoke curl for {}", candidate.url))
            {
                Ok(output) if output.status.success() => {
                    let raw = String::from_utf8_lossy(&output.stdout).to_string();
                    if raw.trim().is_empty() {
                        last_error = Some(anyhow::anyhow!("empty response body"));
                    } else {
                        let title = extract_html_title(&raw)
                            .unwrap_or_else(|| summarize_url(&candidate.url));
                        let markdown = html_to_markdownish(&raw);
                        return Ok(ResearchArtifact {
                            url: candidate.url.clone(),
                            title,
                            extracted_summary: summarize_text(&markdown, 280),
                            markdown,
                            discovered_at_ms: crate::orchestration::current_time_ms(),
                            source_kind: candidate.source_kind.clone(),
                            fetched_via: if self.config.prefer_dynamic_render {
                                "web_fetch_dynamic".into()
                            } else {
                                "web_fetch".into()
                            },
                        });
                    }
                }
                Ok(output) => {
                    governance.record_failure(Some(&proxy));
                    last_error = Some(anyhow::anyhow!(
                        "curl GET {} failed: {}",
                        candidate.url,
                        String::from_utf8_lossy(&output.stderr)
                    ));
                }
                Err(err) => {
                    governance.record_failure(Some(&proxy));
                    last_error = Some(err)
                }
            }
            if attempt < self.config.retry_attempts {
                thread::sleep(Duration::from_millis(self.config.stability_backoff_ms));
            }
        }
        Err(last_error.unwrap_or_else(|| anyhow::anyhow!("web fetch failed")))
    }

    fn playwright_cli_artifact(
        &self,
        candidate: &ResearchCandidate,
        index: usize,
        governance: &mut ProxyGovernanceState,
    ) -> Result<ResearchArtifact> {
        let Some(proxy) = self.reserve_proxy(governance, candidate.url.as_str(), index) else {
            anyhow::bail!("proxy governance blocked playwright request");
        };
        let script = r#"
const [url, userAgent, timeoutMs, proxy, antiBot] = process.argv.slice(2);
(async () => {
  const { chromium } = await import('playwright');
  const launch = { headless: true, timeout: Number(timeoutMs) };
  if (proxy && proxy !== "none") launch.proxy = { server: proxy };
  const browser = await chromium.launch(launch);
  const context = await browser.newContext({ userAgent });
  const page = await context.newPage();
  await page.goto(url, { waitUntil: antiBot === "aggressive-stealth" ? "networkidle" : "domcontentloaded", timeout: Number(timeoutMs) });
  await page.waitForTimeout(Math.min(Number(timeoutMs), 3000));
  const title = await page.title();
  const html = await page.content();
  const content = await page.locator('body').innerText().catch(() => '');
  await browser.close();
  process.stdout.write(JSON.stringify({ title, html, content, url }));
})().catch((err) => {
  process.stderr.write(String(err && err.stack ? err.stack : err));
  process.exit(1);
});
"#;

        let mut command = Command::new(&self.config.playwright_node_binary);
        command
            .arg("-e")
            .arg(script)
            .arg(&candidate.url)
            .arg(&self.config.user_agent)
            .arg(self.playwright_timeout_ms().to_string())
            .arg(proxy.clone())
            .arg(&self.config.anti_bot_profile);

        let output = command
            .output()
            .with_context(|| format!("failed to invoke playwright cli for {}", candidate.url))?;
        if !output.status.success() {
            governance.record_failure(Some(&proxy));
            anyhow::bail!(
                "playwright cli {} failed: {}",
                candidate.url,
                String::from_utf8_lossy(&output.stderr)
            );
        }
        let value: Value = serde_json::from_slice(&output.stdout)
            .context("failed to parse playwright cli response")?;
        let raw = value
            .get("html")
            .and_then(Value::as_str)
            .or_else(|| value.get("content").and_then(Value::as_str))
            .unwrap_or_default()
            .to_string();
        if raw.trim().is_empty() {
            anyhow::bail!("playwright cli returned empty body");
        }
        let markdown = if raw.contains("<html") || raw.contains("<body") {
            html_to_markdownish(&raw)
        } else {
            raw.clone()
        };
        Ok(ResearchArtifact {
            url: candidate.url.clone(),
            title: value
                .get("title")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| summarize_url(&candidate.url)),
            extracted_summary: summarize_text(&markdown, 280),
            markdown,
            discovered_at_ms: crate::orchestration::current_time_ms(),
            source_kind: candidate.source_kind.clone(),
            fetched_via: "playwright_cli".into(),
        })
    }

    fn timeout_ms(&self) -> u64 {
        self.config.request_timeout_secs.saturating_mul(1000)
    }

    fn render_wait_ms(&self) -> u64 {
        if self.config.prefer_dynamic_render {
            self.timeout_ms().min(3_500)
        } else {
            0
        }
    }

    fn browser_session(&self, index: usize) -> String {
        if self.config.browser_session_pool.is_empty() {
            "default-browser-session".into()
        } else {
            self.config.browser_session_pool[index % self.config.browser_session_pool.len()].clone()
        }
    }

    fn reserve_proxy(
        &self,
        governance: &mut ProxyGovernanceState,
        seed: &str,
        index: usize,
    ) -> Option<String> {
        let proxy = self.select_proxy(index, seed)?;
        if governance.usage_counts.get(&proxy).copied().unwrap_or(0)
            >= self.config.proxy_request_quota_per_proxy
        {
            return None;
        }
        if governance.failure_counts.get(&proxy).copied().unwrap_or(0)
            >= self.config.proxy_breaker_failure_threshold
        {
            return None;
        }
        *governance.usage_counts.entry(proxy.clone()).or_insert(0) += 1;
        Some(proxy)
    }

    fn select_proxy(&self, index: usize, seed: &str) -> Option<String> {
        if self.config.proxy_pool.is_empty() {
            return None;
        }
        let offset = if self.config.rotate_proxy_per_request {
            seed.bytes()
                .fold(0usize, |acc, byte| acc.wrapping_add(byte as usize))
                + index
        } else {
            0
        };
        Some(self.config.proxy_pool[offset % self.config.proxy_pool.len()].clone())
    }

    fn proxy_for_body(&self, body: &Value) -> Option<String> {
        let seed = body
            .get("url")
            .and_then(Value::as_str)
            .or_else(|| body.get("query").and_then(Value::as_str))
            .unwrap_or("research");
        self.select_proxy(0, seed)
    }

    fn playwright_timeout_ms(&self) -> u64 {
        self.config
            .playwright_launch_timeout_secs
            .saturating_mul(1000)
            .max(self.timeout_ms())
    }
}

impl ProxyGovernanceState {
    fn record_failure(&mut self, proxy: Option<&str>) {
        if let Some(proxy) = proxy {
            *self.failure_counts.entry(proxy.to_string()).or_insert(0) += 1;
        }
    }

    fn used_proxies(&self) -> Vec<String> {
        let mut proxies = self.usage_counts.keys().cloned().collect::<Vec<_>>();
        proxies.sort();
        proxies
    }

    fn exhausted_proxies(&self, quota: usize) -> Vec<String> {
        let mut proxies = self
            .usage_counts
            .iter()
            .filter(|(_, used)| **used >= quota)
            .map(|(proxy, _)| proxy.clone())
            .collect::<Vec<_>>();
        proxies.sort();
        proxies
    }

    fn open_circuit_proxies(&self, threshold: usize) -> Vec<String> {
        let mut proxies = self
            .failure_counts
            .iter()
            .filter(|(_, failures)| **failures >= threshold)
            .map(|(proxy, _)| proxy.clone())
            .collect::<Vec<_>>();
        proxies.sort();
        proxies
    }
}

fn build_queries(anchor_text: &str, config: &ResearchConfig) -> Vec<DiscoveryQuery> {
    let normalized = anchor_text.trim();
    let mut queries = vec![DiscoveryQuery {
        query: normalized.to_string(),
        rationale: "primary anchor query".into(),
    }];
    if config.prefer_official_sources {
        queries.push(DiscoveryQuery {
            query: format!("{normalized} official source"),
            rationale: "prefer official or first-party source".into(),
        });
    }
    if looks_like_regional_policy_query(normalized) {
        queries.push(DiscoveryQuery {
            query: format!("{normalized} official notice implementation details"),
            rationale: "expand into regulator notices and regional implementation details".into(),
        });
    }
    if looks_like_docs_query(normalized) {
        queries.push(DiscoveryQuery {
            query: format!("{normalized} docs api examples changelog"),
            rationale: "pull documentation, examples, and update history".into(),
        });
    }
    queries.push(DiscoveryQuery {
        query: format!("{normalized} latest update"),
        rationale: "search for freshness and recent changes".into(),
    });
    queries.push(DiscoveryQuery {
        query: format!("{normalized} source gaps contradictions unresolved questions"),
        rationale: "second-hop query to fill evidence gaps and contradictions".into(),
    });
    queries.truncate(config.max_queries_per_anchor);
    queries
}

fn discover_candidates(
    query: &DiscoveryQuery,
    anchor_text: &str,
    config: &ResearchConfig,
) -> Vec<ResearchCandidate> {
    let lowered_anchor = anchor_text.to_ascii_lowercase();
    let is_policy = lowered_anchor.contains("policy")
        || anchor_text.contains("\u{653f}\u{7b56}")
        || lowered_anchor.contains("regulation")
        || lowered_anchor.contains("compliance");
    let is_docs = lowered_anchor.contains("api")
        || lowered_anchor.contains("sdk")
        || lowered_anchor.contains("rust")
        || lowered_anchor.contains("docs");
    let base = sanitize_topic(anchor_text);

    let mut candidates = if is_policy {
        vec![
            ResearchCandidate {
                url: format!("https://www.gov.cn/search/{}", base),
                source_kind: ResearchSourceKind::Official,
                relevance: 0.96,
                trust_score: 0.97,
                selected_reason: format!("{} -> national official source", query.query),
            },
            ResearchCandidate {
                url: format!("https://www.ndrc.gov.cn/search/{}", base),
                source_kind: ResearchSourceKind::Official,
                relevance: 0.91,
                trust_score: 0.94,
                selected_reason: "regulatory source candidate".into(),
            },
            ResearchCandidate {
                url: format!("https://www.miit.gov.cn/search/{}", base),
                source_kind: ResearchSourceKind::Official,
                relevance: 0.88,
                trust_score: 0.92,
                selected_reason: "industry authority candidate".into(),
            },
        ]
    } else if is_docs {
        vec![
            ResearchCandidate {
                url: format!("https://docs.rs/releases/search?query={}", base),
                source_kind: ResearchSourceKind::Documentation,
                relevance: 0.92,
                trust_score: 0.9,
                selected_reason: format!("{} -> documentation mapping", query.query),
            },
            ResearchCandidate {
                url: format!("https://github.com/search?q={}", base),
                source_kind: ResearchSourceKind::Documentation,
                relevance: 0.82,
                trust_score: 0.76,
                selected_reason: "code and issue discovery candidate".into(),
            },
            ResearchCandidate {
                url: format!("https://blog.rust-lang.org/search/{}", base),
                source_kind: ResearchSourceKind::News,
                relevance: 0.66,
                trust_score: 0.78,
                selected_reason: "supporting release/update context".into(),
            },
        ]
    } else {
        vec![
            ResearchCandidate {
                url: format!("https://www.wikipedia.org/wiki/{}", base),
                source_kind: ResearchSourceKind::Official,
                relevance: 0.78,
                trust_score: 0.7,
                selected_reason: format!("{} -> primary domain candidate", query.query),
            },
            ResearchCandidate {
                url: format!("https://news.google.com/search?q={}", base),
                source_kind: ResearchSourceKind::News,
                relevance: 0.64,
                trust_score: 0.64,
                selected_reason: "freshness candidate".into(),
            },
            ResearchCandidate {
                url: format!("https://community.example.com/{}", base),
                source_kind: ResearchSourceKind::Community,
                relevance: 0.42,
                trust_score: 0.38,
                selected_reason: "community corroboration candidate".into(),
            },
        ]
    };

    candidates.retain(|candidate| !candidate.url.contains("social"));
    candidates.truncate(config.max_candidates_per_query);
    candidates
}

fn normalize_candidates(candidates: &mut Vec<ResearchCandidate>) {
    candidates.sort_by(|left, right| {
        ((right.relevance * 100.0 + right.trust_score * 40.0) as i32)
            .cmp(&((left.relevance * 100.0 + left.trust_score * 40.0) as i32))
            .then_with(|| {
                right
                    .relevance
                    .partial_cmp(&left.relevance)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| left.url.cmp(&right.url))
    });
    candidates.dedup_by(|left, right| left.url == right.url);
}

fn source_trust_score(url: &str, source_kind: &ResearchSourceKind) -> f32 {
    let lowered = url.to_ascii_lowercase();
    let domain_bonus = if lowered.contains(".gov")
        || lowered.contains("docs.rs")
        || lowered.contains("rust-lang.org")
    {
        0.12
    } else if lowered.contains("github.com") {
        0.06
    } else if lowered.contains("community") || lowered.contains("forum") {
        -0.06
    } else {
        0.0
    };

    let base: f32 = match source_kind {
        ResearchSourceKind::Official => 0.82,
        ResearchSourceKind::Documentation => 0.78,
        ResearchSourceKind::News => 0.62,
        ResearchSourceKind::Community => 0.45,
    };
    (base + domain_bonus).clamp(0.0, 1.0)
}

fn synthesize_artifact(
    candidate: &ResearchCandidate,
    anchor_text: &str,
    fetched_via: &str,
) -> ResearchArtifact {
    let title = candidate
        .url
        .split('/')
        .filter(|part| !part.is_empty())
        .next_back()
        .unwrap_or("source")
        .replace('-', " ");
    let summary = match candidate.source_kind {
        ResearchSourceKind::Official => format!(
            "Official source coverage for `{anchor_text}` with emphasis on authoritative policy or first-party statements."
        ),
        ResearchSourceKind::Documentation => format!(
            "Documentation-oriented source for `{anchor_text}` with implementation and API details."
        ),
        ResearchSourceKind::News => format!(
            "Freshness-oriented source for `{anchor_text}` to detect updates, changes, and recent developments."
        ),
        ResearchSourceKind::Community => format!(
            "Community corroboration source for `{anchor_text}` to surface edge cases and practical signals."
        ),
    };
    ResearchArtifact {
        url: candidate.url.clone(),
        title,
        markdown: format!(
            "# {}\n\n{}\n\n- source_kind: {:?}\n- relevance: {:.2}\n- reason: {}",
            anchor_text,
            summary,
            candidate.source_kind,
            candidate.relevance,
            candidate.selected_reason
        ),
        extracted_summary: summary,
        discovered_at_ms: crate::orchestration::current_time_ms(),
        source_kind: candidate.source_kind.clone(),
        fetched_via: fetched_via.into(),
    }
}

fn infer_gaps(
    anchor_text: &str,
    candidates: &[ResearchCandidate],
    artifacts: &[ResearchArtifact],
    warnings: &[String],
) -> Vec<String> {
    let mut gaps = Vec::new();
    if !candidates
        .iter()
        .any(|candidate| candidate.source_kind == ResearchSourceKind::Official)
    {
        gaps.push("missing_official_source".into());
    }
    if !artifacts.iter().any(|artifact| {
        artifact
            .extracted_summary
            .to_ascii_lowercase()
            .contains("update")
            || artifact.url.to_ascii_lowercase().contains("news")
    }) {
        gaps.push("missing_freshness_signal".into());
    }
    if anchor_text.chars().count() < 12 {
        gaps.push("anchor_under_specified".into());
    }
    if warnings.iter().any(|warning| warning.contains("failed")) {
        gaps.push("partial_fetch_failure".into());
    }
    gaps
}

fn compute_autonomy_score(
    queries: &[DiscoveryQuery],
    candidates: &[ResearchCandidate],
    artifacts: &[ResearchArtifact],
    gaps: &[String],
    backend_used: &str,
) -> f32 {
    let query_score = (queries.len() as f32 / 3.0).clamp(0.0, 1.0) * 0.2;
    let candidate_score = (candidates.len() as f32 / 6.0).clamp(0.0, 1.0) * 0.25;
    let artifact_score = (artifacts.len() as f32 / 6.0).clamp(0.0, 1.0) * 0.25;
    let backend_bonus = match backend_used {
        "browser_fetch" => 0.24,
        "playwright_cli" => 0.26,
        "firecrawl" => 0.2,
        "self_hosted_scraper" => 0.16,
        "web_fetch" => 0.12,
        _ => 0.04,
    };
    let gap_penalty = (gaps.len() as f32 * 0.08).min(0.3);
    (query_score + candidate_score + artifact_score + backend_bonus - gap_penalty).clamp(0.0, 1.0)
}

fn sanitize_topic(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .take(6)
        .collect::<Vec<_>>()
        .join("-")
}

fn looks_like_regional_policy_query(text: &str) -> bool {
    let lowered = text.to_ascii_lowercase();
    lowered.contains("policy")
        || lowered.contains("regulation")
        || lowered.contains("compliance")
        || text.contains("\u{653f}\u{7b56}")
        || text.contains("\u{8865}\u{8d34}")
        || text.contains("\u{5408}\u{89c4}")
}

fn looks_like_docs_query(text: &str) -> bool {
    let lowered = text.to_ascii_lowercase();
    lowered.contains("api")
        || lowered.contains("sdk")
        || lowered.contains("rust")
        || lowered.contains("docs")
        || text.contains("\u{6587}\u{6863}")
}

fn maybe_enable_dynamic_render(url: &str) -> String {
    if url.contains('?') {
        format!("{url}&render=1")
    } else {
        format!("{url}?render=1")
    }
}

fn command_available(binary: &str, version_flag: &str) -> bool {
    Command::new(binary)
        .arg(version_flag)
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn summarize_text(value: &str, max_chars: usize) -> String {
    let compact = value.split_whitespace().collect::<Vec<_>>().join(" ");
    compact.chars().take(max_chars).collect()
}

fn summarize_url(value: &str) -> String {
    value
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .split('/')
        .next()
        .unwrap_or("source")
        .to_string()
}

fn classify_source_kind(signal: &str) -> ResearchSourceKind {
    let lowered = signal.to_ascii_lowercase();
    if lowered.contains(".gov")
        || lowered.contains("official")
        || lowered.contains("docs.rs")
        || lowered.contains("wikipedia")
    {
        if lowered.contains("docs") || lowered.contains("docs.rs") {
            ResearchSourceKind::Documentation
        } else {
            ResearchSourceKind::Official
        }
    } else if lowered.contains("blog") || lowered.contains("news") || lowered.contains("update") {
        ResearchSourceKind::News
    } else if lowered.contains("github") || lowered.contains("community") {
        ResearchSourceKind::Community
    } else {
        ResearchSourceKind::Documentation
    }
}

fn firecrawl_endpoint(base: &str, path: &str) -> String {
    let normalized_base = base.trim_end_matches('/');
    if normalized_base.ends_with("/v2") {
        format!("{normalized_base}{path}")
    } else {
        format!("{normalized_base}/v2{path}")
    }
}

fn normalize_scrape_endpoint(base: &str) -> String {
    let normalized = base.trim_end_matches('/');
    if normalized.ends_with("/scrape") {
        normalized.to_string()
    } else {
        format!("{normalized}/scrape")
    }
}

fn curl_binary() -> &'static str {
    if cfg!(windows) { "curl.exe" } else { "curl" }
}

fn extract_html_title(raw: &str) -> Option<String> {
    let lowered = raw.to_ascii_lowercase();
    let start = lowered.find("<title>")?;
    let end = lowered[start + 7..].find("</title>")?;
    Some(raw[start + 7..start + 7 + end].trim().to_string())
}

fn html_to_markdownish(raw: &str) -> String {
    let mut text = String::with_capacity(raw.len());
    let mut inside_tag = false;
    for ch in raw.chars() {
        match ch {
            '<' => {
                inside_tag = true;
                text.push(' ');
            }
            '>' => {
                inside_tag = false;
                text.push(' ');
            }
            _ if !inside_tag => text.push(ch),
            _ => {}
        }
    }
    let compact = text
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">");
    summarize_text(&compact, 4000)
}

#[cfg(test)]
mod tests {
    use super::*;
    use autoloop_state_adapter::{StateStoreBackend, StateStoreConfig};

    fn test_config() -> ResearchConfig {
        ResearchConfig {
            enabled: true,
            backend: ResearchBackend::Auto,
            live_fetch_enabled: false,
            max_queries_per_anchor: 3,
            max_candidates_per_query: 3,
            prefer_official_sources: true,
            prefer_dynamic_render: true,
            browser_render_url: None,
            browser_session_pool: vec!["test-browser".into()],
            proxy_provider_name: "test-pool".into(),
            proxy_pool: Vec::new(),
            anti_bot_profile: "balanced-stealth".into(),
            rotate_proxy_per_request: true,
            proxy_request_quota_per_proxy: 8,
            proxy_breaker_failure_threshold: 2,
            proxy_breaker_cooldown_ms: 60_000,
            playwright_node_binary: "node".into(),
            playwright_launch_timeout_secs: 25,
            firecrawl_api_url: "https://api.firecrawl.dev".into(),
            firecrawl_api_key_env: "FIRECRAWL_API_KEY".into(),
            self_hosted_scraper_url: None,
            request_timeout_secs: 20,
            retry_attempts: 2,
            stability_backoff_ms: 10,
            user_agent: "autoloop-test/0.1".into(),
        }
    }

    #[tokio::test]
    async fn research_kernel_persists_anchor_report_and_artifacts() {
        let db = StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 4,
        });
        let kernel = ResearchKernel::from_config(&test_config());

        let report = kernel
            .run_anchor_research(
                &db,
                "session-research",
                "2024 涓浗鏂拌兘婧愯ˉ璐存斂绛?official updates",
            )
            .await
            .expect("research");

        let stored = db
            .get_knowledge("research:session-research:report")
            .await
            .expect("report")
            .expect("report exists");

        assert!(!report.queries.is_empty());
        assert!(!report.artifacts.is_empty());
        assert_eq!(report.backend_used, "synthetic");
        assert!(stored.value.contains("autonomy_score"));
    }

    #[tokio::test]
    async fn browser_fetch_unavailable_negotiates_to_policy_fallback() {
        let db = StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 4,
        });
        let mut config = test_config();
        config.backend = ResearchBackend::BrowserFetch;
        config.live_fetch_enabled = false;
        config.browser_render_url = None;
        let kernel = ResearchKernel::from_config(&config);

        let report = kernel
            .run_anchor_research(&db, "session-research-browser-fallback", "Rust async sdk docs")
            .await
            .expect("browser fallback research");

        assert_eq!(report.backend_used, "synthetic");
        assert!(report.backend_warnings.iter().any(|warning| {
            warning.contains("capability_negotiation") && warning.contains("browser_fetch")
        }));
    }

    #[test]
    fn firecrawl_endpoint_normalizes_v2_paths() {
        assert_eq!(
            firecrawl_endpoint("https://api.firecrawl.dev", "/search"),
            "https://api.firecrawl.dev/v2/search"
        );
        assert_eq!(
            firecrawl_endpoint("https://self-hosted.firecrawl/v2/", "/scrape"),
            "https://self-hosted.firecrawl/v2/scrape"
        );
    }

    #[test]
    fn html_to_markdownish_extracts_page_text() {
        let markdown = html_to_markdownish(
            "<html><head><title>Demo</title></head><body><h1>Hello</h1><p>World</p></body></html>",
        );
        assert!(markdown.contains("Hello"));
        assert!(markdown.contains("World"));
    }

    #[test]
    fn build_queries_expands_policy_and_docs_plans() {
        let config = test_config();
        let policy = build_queries("2024 中国新能源补贴政策", &config);
        let docs = build_queries("Rust async sdk docs", &config);

        assert!(
            policy
                .iter()
                .any(|query| query.query.contains("implementation details"))
        );
        assert!(
            docs.iter()
                .any(|query| query.query.contains("examples changelog"))
        );
    }

    #[test]
    fn proxy_and_browser_session_rotation_are_stable() {
        let mut config = test_config();
        config.proxy_pool = vec!["http://proxy-a".into(), "http://proxy-b".into()];
        config.browser_session_pool = vec!["session-a".into(), "session-b".into()];
        let kernel = ResearchKernel::from_config(&config);

        assert!(kernel.select_proxy(0, "https://example.com").is_some());
        assert_ne!(kernel.browser_session(0), kernel.browser_session(1));
    }

    #[test]
    fn playwright_timeout_respects_launch_timeout_floor() {
        let mut config = test_config();
        config.playwright_launch_timeout_secs = 30;
        config.request_timeout_secs = 5;
        let kernel = ResearchKernel::from_config(&config);
        assert!(kernel.playwright_timeout_ms() >= 30_000);
    }

    #[test]
    fn health_report_reflects_configured_backends() {
        let mut config = test_config();
        config.browser_render_url = Some("http://browserless/render".into());
        config.proxy_pool = vec!["http://proxy-a".into()];
        let kernel = ResearchKernel::from_config(&config);
        let health = kernel.health_report();
        assert!(health.browser_render_configured);
        assert_eq!(health.proxy_pool_size, 1);
    }
}

