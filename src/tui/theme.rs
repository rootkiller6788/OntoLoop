use ratatui::style::Color;

pub const ORANGE: Color = Color::Rgb(245, 144, 62);
pub const ORANGE_DIM: Color = Color::Rgb(180, 100, 40);
pub const GRAY_DARK: Color = Color::Rgb(20, 20, 20);
pub const GRAY_PANEL: Color = Color::Rgb(30, 30, 30);
pub const GRAY_BORDER: Color = Color::Rgb(60, 60, 60);
pub const GRAY_TEXT: Color = Color::Rgb(160, 160, 160);
pub const GRAY_MUTED: Color = Color::Rgb(100, 100, 100);
pub const WHITE: Color = Color::Rgb(238, 238, 238);

pub fn tui_theme() -> Theme {
    Theme {
        bg: GRAY_DARK,
        panel: GRAY_PANEL,
        border: GRAY_BORDER,
        border_active: ORANGE,
        primary: ORANGE,
        primary_dim: ORANGE_DIM,
        text: WHITE,
        text_muted: GRAY_TEXT,
        text_dim: GRAY_MUTED,
        success: Color::Rgb(127, 216, 143),
        error: Color::Rgb(224, 108, 117),
        warning: Color::Rgb(229, 192, 123),
        info: Color::Rgb(86, 182, 194),
    }
}

#[derive(Clone)]
pub struct Theme {
    pub bg: Color,
    pub panel: Color,
    pub border: Color,
    pub border_active: Color,
    pub primary: Color,
    pub primary_dim: Color,
    pub text: Color,
    pub text_muted: Color,
    pub text_dim: Color,
    pub success: Color,
    pub error: Color,
    pub warning: Color,
    pub info: Color,
}
