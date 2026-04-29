use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HookPhase {
    #[serde(alias = "pre_tool_use")]
    Before,
    #[serde(alias = "post_tool_use")]
    Step,
    #[serde(alias = "on_stream")]
    Stream,
    #[serde(alias = "on_result")]
    Return,
    #[serde(alias = "on_error")]
    Throws,
    Timeout,
    #[serde(alias = "on_kill")]
    Kill,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HookStage {
    PreToolUse,
    PostToolUse,
    OnResult,
    OnError,
}

impl HookStage {
    pub fn to_phase(&self) -> HookPhase {
        match self {
            HookStage::PreToolUse => HookPhase::Before,
            HookStage::PostToolUse => HookPhase::Step,
            HookStage::OnResult => HookPhase::Return,
            HookStage::OnError => HookPhase::Throws,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HookAction {
    Allow,
    Deny,
    Rewrite,
    Mutate,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HookChannel {
    Command,
    Http,
    Prompt,
    Agent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookRule {
    pub id: String,
    #[serde(default = "default_phase", alias = "stage")]
    pub phase: HookPhase,
    pub action: HookAction,
    #[serde(default)]
    pub channel: Option<HookChannel>,
    pub tool_contains: Option<String>,
    pub value: Option<String>,
    pub reason: String,
}

fn default_phase() -> HookPhase {
    HookPhase::Before
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookTrace {
    pub rule_id: String,
    pub phase: HookPhase,
    pub channel: HookChannel,
    pub action: HookAction,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookOutcome {
    pub allowed: bool,
    pub channel: HookChannel,
    pub tool_name: String,
    pub arguments: String,
    pub output: Option<String>,
    pub error: Option<String>,
    pub traces: Vec<HookTrace>,
}

#[derive(Debug, Clone, Default)]
pub struct HookRuntime {
    rules: Vec<HookRule>,
}

impl HookRuntime {
    pub fn with_rules(rules: Vec<HookRule>) -> Self {
        Self { rules }
    }

    pub fn set_rules(&mut self, rules: Vec<HookRule>) {
        self.rules = rules;
    }

    pub fn rules(&self) -> &[HookRule] {
        &self.rules
    }

    pub fn apply_before(&self, tool_name: &str, arguments: &str) -> HookOutcome {
        self.apply_before_with_channel(HookChannel::Command, tool_name, arguments)
    }

    pub fn apply_before_with_channel(
        &self,
        channel: HookChannel,
        tool_name: &str,
        arguments: &str,
    ) -> HookOutcome {
        self.apply_phase_only(
            HookPhase::Before,
            channel,
            tool_name,
            arguments,
            None,
            None,
        )
    }

    pub fn apply_step(&self, tool_name: &str, arguments: &str) -> HookOutcome {
        self.apply_step_with_channel(HookChannel::Command, tool_name, arguments)
    }

    pub fn apply_step_with_channel(
        &self,
        channel: HookChannel,
        tool_name: &str,
        arguments: &str,
    ) -> HookOutcome {
        self.apply_phase_only(HookPhase::Step, channel, tool_name, arguments, None, None)
    }

    pub fn apply_return(&self, tool_name: &str, arguments: &str, output: &str) -> HookOutcome {
        self.apply_return_with_channel(HookChannel::Command, tool_name, arguments, output)
    }

    pub fn apply_stream(&self, tool_name: &str, arguments: &str, output: &str) -> HookOutcome {
        self.apply_stream_with_channel(HookChannel::Command, tool_name, arguments, output)
    }

    pub fn apply_stream_with_channel(
        &self,
        channel: HookChannel,
        tool_name: &str,
        arguments: &str,
        output: &str,
    ) -> HookOutcome {
        self.apply_phase_only(
            HookPhase::Stream,
            channel,
            tool_name,
            arguments,
            Some(output.to_string()),
            None,
        )
    }

    pub fn apply_return_with_channel(
        &self,
        channel: HookChannel,
        tool_name: &str,
        arguments: &str,
        output: &str,
    ) -> HookOutcome {
        self.apply_phase_only(
            HookPhase::Return,
            channel,
            tool_name,
            arguments,
            Some(output.to_string()),
            None,
        )
    }

    pub fn apply_throws(&self, tool_name: &str, arguments: &str, error: &str) -> HookOutcome {
        self.apply_throws_with_channel(HookChannel::Command, tool_name, arguments, error)
    }

    pub fn apply_throws_with_channel(
        &self,
        channel: HookChannel,
        tool_name: &str,
        arguments: &str,
        error: &str,
    ) -> HookOutcome {
        self.apply_phase_only(
            HookPhase::Throws,
            channel,
            tool_name,
            arguments,
            None,
            Some(error.to_string()),
        )
    }

    pub fn apply_timeout(&self, tool_name: &str, arguments: &str, detail: &str) -> HookOutcome {
        self.apply_timeout_with_channel(HookChannel::Command, tool_name, arguments, detail)
    }

    pub fn apply_timeout_with_channel(
        &self,
        channel: HookChannel,
        tool_name: &str,
        arguments: &str,
        detail: &str,
    ) -> HookOutcome {
        self.apply_phase_only(
            HookPhase::Timeout,
            channel,
            tool_name,
            arguments,
            None,
            Some(detail.to_string()),
        )
    }

    pub fn apply_kill(&self, tool_name: &str, arguments: &str, detail: &str) -> HookOutcome {
        self.apply_kill_with_channel(HookChannel::Command, tool_name, arguments, detail)
    }

    pub fn apply_kill_with_channel(
        &self,
        channel: HookChannel,
        tool_name: &str,
        arguments: &str,
        detail: &str,
    ) -> HookOutcome {
        self.apply_phase_only(
            HookPhase::Kill,
            channel,
            tool_name,
            arguments,
            None,
            Some(detail.to_string()),
        )
    }

    pub fn apply_pre_tool_use(&self, tool_name: &str, arguments: &str) -> HookOutcome {
        self.apply_before(tool_name, arguments)
    }

    pub fn apply_pre_tool_use_with_channel(
        &self,
        channel: HookChannel,
        tool_name: &str,
        arguments: &str,
    ) -> HookOutcome {
        self.apply_before_with_channel(channel, tool_name, arguments)
    }

    pub fn apply_post_tool_use(&self, tool_name: &str, arguments: &str) -> HookOutcome {
        self.apply_step(tool_name, arguments)
    }

    pub fn apply_post_tool_use_with_channel(
        &self,
        channel: HookChannel,
        tool_name: &str,
        arguments: &str,
    ) -> HookOutcome {
        self.apply_step_with_channel(channel, tool_name, arguments)
    }

    pub fn apply_on_result(&self, tool_name: &str, arguments: &str, output: &str) -> HookOutcome {
        self.apply_return(tool_name, arguments, output)
    }

    pub fn apply_on_result_with_channel(
        &self,
        channel: HookChannel,
        tool_name: &str,
        arguments: &str,
        output: &str,
    ) -> HookOutcome {
        self.apply_return_with_channel(channel, tool_name, arguments, output)
    }

    pub fn apply_on_error(&self, tool_name: &str, arguments: &str, error: &str) -> HookOutcome {
        self.apply_throws(tool_name, arguments, error)
    }

    pub fn apply_on_error_with_channel(
        &self,
        channel: HookChannel,
        tool_name: &str,
        arguments: &str,
        error: &str,
    ) -> HookOutcome {
        self.apply_throws_with_channel(channel, tool_name, arguments, error)
    }

    fn apply_phase_only(
        &self,
        phase: HookPhase,
        channel: HookChannel,
        tool_name: &str,
        arguments: &str,
        output: Option<String>,
        error: Option<String>,
    ) -> HookOutcome {
        let mut args = arguments.to_string();
        let mut out = output;
        let mut err = error;
        let mut traces = Vec::new();
        let mut allowed = true;

        for rule in self.applicable(phase.clone(), &channel, tool_name) {
            traces.push(HookTrace {
                rule_id: rule.id.clone(),
                phase: phase.clone(),
                channel: channel.clone(),
                action: rule.action.clone(),
                reason: rule.reason.clone(),
            });
            match rule.action {
                HookAction::Allow => {}
                HookAction::Deny => {
                    allowed = false;
                    if err.is_none() {
                        err = Some(rule.reason.clone());
                    }
                    break;
                }
                HookAction::Rewrite => {
                    if let Some(value) = rule.value.as_deref() {
                        match phase {
                            HookPhase::Return | HookPhase::Stream => out = Some(value.to_string()),
                            HookPhase::Throws | HookPhase::Timeout | HookPhase::Kill => {
                                err = Some(value.to_string())
                            }
                            HookPhase::Before | HookPhase::Step => args = value.to_string(),
                        }
                    }
                }
                HookAction::Mutate => {
                    if let Some(value) = rule.value.as_deref() {
                        match phase {
                            HookPhase::Return | HookPhase::Stream => {
                                let mut updated = out.unwrap_or_default();
                                updated.push_str(value);
                                out = Some(updated);
                            }
                            HookPhase::Throws | HookPhase::Timeout | HookPhase::Kill => {
                                let mut updated = err.unwrap_or_default();
                                updated.push_str(value);
                                err = Some(updated);
                            }
                            HookPhase::Before | HookPhase::Step => args.push_str(value),
                        }
                    }
                }
            }
        }

        HookOutcome {
            allowed,
            channel,
            tool_name: tool_name.to_string(),
            arguments: args,
            output: out,
            error: err,
            traces,
        }
    }

    fn applicable(&self, phase: HookPhase, channel: &HookChannel, tool_name: &str) -> Vec<&HookRule> {
        self.rules
            .iter()
            .filter(|rule| rule.phase == phase)
            .filter(|rule| match rule.channel.as_ref() {
                Some(expected) => expected == channel,
                None => true,
            })
            .filter(|rule| match rule.tool_contains.as_deref() {
                Some(pattern) => tool_name.contains(pattern),
                None => true,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hook_runtime_denies_before_phase() {
        let runtime = HookRuntime::with_rules(vec![HookRule {
            id: "deny-shell".into(),
            phase: HookPhase::Before,
            action: HookAction::Deny,
            channel: None,
            tool_contains: Some("shell".into()),
            value: None,
            reason: "blocked by policy".into(),
        }]);

        let outcome = runtime.apply_before("mcp::shell", "{}");
        assert!(!outcome.allowed);
        assert_eq!(outcome.error.as_deref(), Some("blocked by policy"));
    }

    #[test]
    fn channel_specific_rule_does_not_hit_other_channel() {
        let runtime = HookRuntime::with_rules(vec![HookRule {
            id: "deny-http".into(),
            phase: HookPhase::Before,
            action: HookAction::Deny,
            channel: Some(HookChannel::Http),
            tool_contains: None,
            value: None,
            reason: "http disabled".into(),
        }]);

        let command = runtime.apply_before_with_channel(HookChannel::Command, "mcp::shell", "{}");
        assert!(command.allowed);

        let http = runtime.apply_before_with_channel(
            HookChannel::Http,
            "http::client::invoke",
            "{\"url\":\"https://example.com\"}",
        );
        assert!(!http.allowed);
        assert_eq!(http.error.as_deref(), Some("http disabled"));
    }

    #[test]
    fn legacy_stage_maps_to_phase() {
        assert_eq!(HookStage::PreToolUse.to_phase(), HookPhase::Before);
        assert_eq!(HookStage::PostToolUse.to_phase(), HookPhase::Step);
        assert_eq!(HookStage::OnResult.to_phase(), HookPhase::Return);
        assert_eq!(HookStage::OnError.to_phase(), HookPhase::Throws);
    }

    #[test]
    fn stream_and_kill_phases_support_mutation() {
        let runtime = HookRuntime::with_rules(vec![
            HookRule {
                id: "stream-mutate".into(),
                phase: HookPhase::Stream,
                action: HookAction::Mutate,
                channel: None,
                tool_contains: Some("mcp".into()),
                value: Some("::stream".into()),
                reason: "stream annotate".into(),
            },
            HookRule {
                id: "kill-mutate".into(),
                phase: HookPhase::Kill,
                action: HookAction::Mutate,
                channel: None,
                tool_contains: Some("mcp".into()),
                value: Some("::kill".into()),
                reason: "kill annotate".into(),
            },
        ]);
        let stream = runtime.apply_stream("mcp::shell", "{}", "chunk");
        assert_eq!(stream.output.as_deref(), Some("chunk::stream"));
        let kill = runtime.apply_kill("mcp::shell", "{}", "terminated");
        assert_eq!(kill.error.as_deref(), Some("terminated::kill"));
    }

    #[test]
    fn timeout_phase_rule_is_applied() {
        let runtime = HookRuntime::with_rules(vec![HookRule {
            id: "timeout-guard".into(),
            phase: HookPhase::Timeout,
            action: HookAction::Deny,
            channel: Some(HookChannel::Command),
            tool_contains: Some("shell".into()),
            value: None,
            reason: "timeout denied by guard".into(),
        }]);

        let outcome = runtime.apply_timeout("mcp::shell", "{\"cmd\":\"long\"}", "timeout 30s");
        assert!(!outcome.allowed);
        assert_eq!(outcome.error.as_deref(), Some("timeout 30s"));
        assert!(
            outcome
                .traces
                .iter()
                .any(|trace| trace.phase == HookPhase::Timeout),
            "expected timeout trace to be recorded"
        );
    }
}
