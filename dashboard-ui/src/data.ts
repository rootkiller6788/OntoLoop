import type { DashboardSessionSnapshot } from "./types/snapshot";
import type { SessionReplay } from "./types/replay";

export const demoReplay: SessionReplay = {
  sessionId: "cli-focus",
  deliberation: {
    round_count: 4,
    final_execution_order: ["architecture", "knowledge", "capability-forge", "execution"],
    consensus_signals: ["graph_prefers_mcp", "verifier=pass"],
    rounds: [
      { speaker: "planner", stance: "proposal", summary: "Planner sequences graph before execution." },
      { speaker: "critic", stance: "challenge", summary: "Critic requests verifier and proxy caution." },
      { speaker: "planner", stance: "rebuttal", summary: "Planner narrows execution to verified capabilities." },
      { speaker: "judge", stance: "arbitration", summary: "Judge freezes a verifier-gated route." }
    ]
  },
  executionFeedback: [
    {
      tool: "mcp::local-mcp::policy-extract",
      outcome_score: 4,
      route_variant: "treatment",
      output: "completed successfully"
    },
    {
      tool: "mcp::local-mcp::browser-research",
      outcome_score: 2,
      route_variant: "control_fallback",
      output: "completed with bounded retry"
    }
  ],
  traces: [
    {
      span_name: "swarm.completed",
      level: "info",
      detail: "validation_ready=true verifier_score=0.91"
    },
    {
      span_name: "runtime.circuit",
      level: "warn",
      detail: "browser-research entered half-open recovery"
    }
  ],
  routeAnalytics: {
    total_reports: 6,
    treatment_share: 0.41,
    guarded_reports: 1,
    top_tools: ["mcp::local-mcp::policy-extract", "mcp::local-mcp::browser-research"]
  },
  failureForensics: {
    primary_failure_mode: "approval-gated",
    blocked_tools: [],
    approval_gated_tools: ["mcp::local-mcp::browser-research"]
  }
};

export const demoDashboardSnapshot: DashboardSessionSnapshot = {
  sessionId: "cli-focus",
  anchor: "2024涓浗涔樼敤杞︽柊鑳芥簮琛ヨ创鏀跨瓥",
  ceoSummary:
    "CEO routes requirement clarification into research, capability reuse, verifier-gated execution, and graph memory consolidation.",
  validationSummary:
    "Verifier pass with bounded follow-up refresh tasks and active catalog governance.",
  routeTreatmentShare: 0.41,
  readiness: true,
  capabilityCatalog: [
    {
      name: "mcp::local-mcp::policy-extract",
      status: "active",
      approval: "verified",
      health: 0.92,
      scope: "global",
      risk: "medium"
    },
    {
      name: "mcp::local-mcp::browser-research",
      status: "active",
      approval: "verified",
      health: 0.86,
      scope: "task_family",
      risk: "high"
    },
    {
      name: "mcp::local-mcp::diagram-export",
      status: "pending_verification",
      approval: "pending",
      health: 0.57,
      scope: "session",
      risk: "low"
    }
  ],
  proxyForensics: {
    provider: "managed-rotating-pool",
    poolSize: 4,
    used: ["proxy-a", "proxy-b", "proxy-c"],
    exhausted: ["proxy-b"],
    openCircuit: ["proxy-c"],
    likelyPressure: true,
    warningSamples: [
      "browser render failed with 429 from upstream source",
      "playwright cli timeout after dynamic page hydration"
    ]
  },
  researchHealth: {
    backend: "PlaywrightCli",
    liveFetchEnabled: true,
    curlAvailable: true,
    nodeAvailable: true,
    browserRenderConfigured: true,
    proxyPoolSize: 4,
    browserSessionPoolSize: 3,
    antiBotProfile: "aggressive-stealth"
  },
  graph: {
    entities: 148,
    relationships: 233,
    communities: 19,
    forgedCapabilityCount: 7,
    topEntities: ["鏂拌兘婧愯ˉ璐?, "涓婃捣甯傚彂鏀瑰", "policy-extract", "browser-research", "StateStore"]
  },
  verifier: {
    verdict: "pass",
    score: 0.91,
    summary:
      "Task-level judge, route correctness, and capability regression suite all passed for the current anchor.",
    failingTools: []
  },
  business: {
    revenueMicros: 54000,
    costMicros: 31000,
    profitMicros: 23000,
    marginRatio: 0.4259,
    slaSuccessRatio: 0.93,
    breachedOrders: 1,
    riskSummary: "SLA breaches detected on 1 tasks"
  },
  workOrders: [
    {
      workOrderId: "wo:cli-focus:task-1",
      taskId: "task-1",
      taskRole: "execution",
      status: "delivered",
      serviceTier: "standard"
    },
    {
      workOrderId: "wo:cli-focus:task-2",
      taskId: "task-2",
      taskRole: "security",
      status: "sla_breached",
      serviceTier: "premium"
    }
  ],
  revenueEvents: [
    {
      revenueEventId: "rev:cli-focus:task-1",
      taskId: "task-1",
      revenueMicros: 28000,
      costMicros: 16000,
      profitMicros: 12000,
      source: "mcp::local-mcp::policy-extract"
    },
    {
      revenueEventId: "rev:cli-focus:task-2",
      taskId: "task-2",
      revenueMicros: 26000,
      costMicros: 15000,
      profitMicros: 11000,
      source: "mcp::local-mcp::browser-research"
    }
  ],
  operationsNotes: [
    "Proxy pressure rising on the browser-research capability",
    "Global graph snapshot is being reinforced by cross-session merges",
    "One low-risk forged capability is awaiting verifier promotion"
  ],
  capabilityLifecycle: {
    total_lineages: 3,
    rollback_ready_capabilities: 2,
    deprecated_capabilities: 0,
    entries: [
      {
        tool_name: "mcp::local-mcp::browser-research",
        lineage_key: "capability:browser-research",
        active_version: 3,
        latest_version: 4,
        stable_version: 3,
        deprecated_versions: [],
        rolled_back_versions: [2],
        average_health: 0.79,
        status_summary: "active with rollback target"
      }
    ]
  },
  runtimeCircuits: {
    "metrics:circuit:tool:mcp::local-mcp::browser-research": {
      phase: "half_open",
      failure_count: 1,
      success_count: 3
    }
  },
  replay: demoReplay
};

