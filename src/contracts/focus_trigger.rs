#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FocusItem {
    pub id: String,
    pub title: String,
    pub status: String,
    pub owner: String,
    pub acceptance_hint: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct FocusBoard {
    pub session_id: String,
    pub goal: String,
    pub items: Vec<FocusItem>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerKind {
    Cron,
    Once,
    Interval,
    Poll,
    OnMessage,
    Webhook,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TriggerSpec {
    pub trigger_id: String,
    pub kind: TriggerKind,
    pub config: serde_json::Value,
    pub reason: String,
    pub focus_ref: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TriggerRef {
    pub trigger_id: String,
    pub session_id: String,
    pub status: String,
}
