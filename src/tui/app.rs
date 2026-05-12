use crate::providers::ChatMessage;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TuiMode {
    Plan,
    Lite,
    Full,
    Test,
}

impl TuiMode {
    pub fn as_str(&self) -> &str {
        match self {
            TuiMode::Plan => "Plan",
            TuiMode::Lite => "Lite",
            TuiMode::Full => "Full",
            TuiMode::Test => "Test",
        }
    }

    pub fn cycle(&self) -> TuiMode {
        match self {
            TuiMode::Plan => TuiMode::Lite,
            TuiMode::Lite => TuiMode::Full,
            TuiMode::Full => TuiMode::Test,
            TuiMode::Test => TuiMode::Plan,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum InputMode {
    Normal,
    Editing,
}

pub struct App {
    pub messages: Vec<ChatMessage>,
    pub input: String,
    pub input_mode: InputMode,
    pub cursor_pos: usize,
    pub mode: TuiMode,
    pub connected: bool,
    pub status: String,
    pub scroll_offset: usize,
}

impl App {
    pub fn new() -> Self {
        Self {
            messages: vec![],
            input: String::new(),
            input_mode: InputMode::Normal,
            cursor_pos: 0,
            mode: TuiMode::Lite,
            connected: true,
            status: String::from("Ready"),
            scroll_offset: 0,
        }
    }

    pub fn push_message(&mut self, role: &str, content: String) {
        self.messages.push(ChatMessage {
            role: role.to_string(),
            content,
            tool_call_id: None,
            tool_calls: None,
        });
    }

    pub fn visible_messages(&self, height: usize) -> &[ChatMessage] {
        let total = self.messages.len();
        if total <= height {
            return &self.messages;
        }
        let start = self.scroll_offset.min(total.saturating_sub(height));
        let end = (start + height).min(total);
        &self.messages[start..end]
    }
}
