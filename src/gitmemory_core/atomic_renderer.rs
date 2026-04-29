use super::patch_core::{PatchOpKind, PatchPlan};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AtomicRenderOutput {
    pub relative_path: String,
    pub markdown: String,
    pub op_count: usize,
}

pub struct AtomicRenderer;

impl AtomicRenderer {
    pub fn render(session_id: &str, trace_id: &str, plan: &PatchPlan) -> AtomicRenderOutput {
        let filename = format!(
            "canonical/{}/{}.md",
            sanitize_segment(session_id),
            sanitize_segment(trace_id)
        );

        let mut body = String::new();
        body.push_str("---\n");
        body.push_str(&format!("session_id: {}\n", session_id));
        body.push_str(&format!("trace_id: {}\n", trace_id));
        body.push_str(&format!("namespace: {}\n", plan.namespace));
        body.push_str(&format!("ops: {}\n", plan.ops.len()));
        body.push_str("---\n\n");
        body.push_str("# Atomic Patch\n\n");

        for (idx, op) in plan.ops.iter().enumerate() {
            body.push_str(&format!(
                "{}. `{}` -> `{}` ({})\n",
                idx + 1,
                op_kind_label(&op.kind),
                op.target,
                op.reason
            ));
        }

        AtomicRenderOutput {
            relative_path: filename,
            markdown: body,
            op_count: plan.ops.len(),
        }
    }
}

fn op_kind_label(kind: &PatchOpKind) -> &'static str {
    match kind {
        PatchOpKind::Add => "add",
        PatchOpKind::Update => "update",
        PatchOpKind::Delete => "delete",
        PatchOpKind::None => "none",
    }
}

fn sanitize_segment(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}
