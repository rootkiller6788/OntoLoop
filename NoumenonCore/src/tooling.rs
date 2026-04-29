use anyhow::{Result, anyhow};

#[derive(Debug, Clone)]
pub struct ToolRequest {
    pub tool_name: String,
    pub input: String,
    pub workspace: String,
}

pub trait ToolAdapter: Send + Sync {
    fn name(&self) -> &str;
    fn invoke(&self, request: &ToolRequest) -> Result<String>;
}

#[derive(Debug, Clone)]
pub struct VirtualEnvToolAdapter {
    tool_name: String,
    env_name: String,
}

impl VirtualEnvToolAdapter {
    pub fn new(tool_name: impl Into<String>, env_name: impl Into<String>) -> Self {
        Self {
            tool_name: tool_name.into(),
            env_name: env_name.into(),
        }
    }
}

impl ToolAdapter for VirtualEnvToolAdapter {
    fn name(&self) -> &str {
        &self.tool_name
    }

    fn invoke(&self, request: &ToolRequest) -> Result<String> {
        if request.workspace.is_empty() {
            return Err(anyhow!("workspace missing"));
        }
        Ok(format!(
            "venv={} tool={} workspace={} input={}",
            self.env_name, request.tool_name, request.workspace, request.input
        ))
    }
}

pub struct ToolOrchestrator {
    adapters: Vec<Box<dyn ToolAdapter>>,
}

impl ToolOrchestrator {
    pub fn new() -> Self {
        Self {
            adapters: Vec::new(),
        }
    }

    pub fn register(&mut self, adapter: Box<dyn ToolAdapter>) {
        self.adapters.push(adapter);
    }

    pub fn call(&self, request: &ToolRequest) -> Result<String> {
        let adapter = self
            .adapters
            .iter()
            .find(|a| a.name() == request.tool_name)
            .ok_or_else(|| anyhow!("tool adapter not found: {}", request.tool_name))?;
        adapter.invoke(request)
    }
}
