mod app;
mod theme;

use std::io::{self};
use std::time::Duration;

use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    prelude::{CrosstermBackend, Terminal},
    style::{Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

use self::app::{App, InputMode, TuiMode};
use self::theme::{Theme, ORANGE};
use crate::config::AppConfig;

const LOGO: &str = r#"
 █▀▀█ █▀▀▄ ▀▀█▀▀ █▀▀█ █    █▀▀█ █▀▀█ █▀▀█
 █  █ █  █   █   █  █ █    █  █ █  █ █  █
 █  █ █▀▀    █   █  █ █    █  █ █  █ █▀▀█
 █▀▀█ █  █   █   █▀▀█ █▀▀▀ █▀▀█ █▀▀█ █
 █  █ █  █   █   █  █ █  █ █  █ █  █ █
 █  █ █▀▀    █   █  █ █▀▀▀ █  █ █  █ █
"#;

pub async fn run_tui(_config: AppConfig) -> anyhow::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let theme = theme::tui_theme();
    let mut app = App::new();
    app.push_message("assistant", "Welcome to OntoLoop. Press i to start typing, 1-4 to switch mode.".into());

    let result = run_event_loop(&mut terminal, &mut app, &theme);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;
    terminal.show_cursor()?;
    result
}

fn run_event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    theme: &Theme,
) -> anyhow::Result<()> {
    loop {
        terminal.draw(|frame| render(frame, app, theme))?;

        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Release {
                    continue;
                }
                match app.input_mode {
                    InputMode::Normal => match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                        KeyCode::Char('i') | KeyCode::Enter => {
                            app.input_mode = InputMode::Editing;
                        }
                        KeyCode::Tab => {
                            app.mode = app.mode.cycle();
                            app.status = format!("Mode: {}", app.mode.as_str());
                        }
                        KeyCode::Char('k') | KeyCode::Up => {
                            if app.scroll_offset < app.messages.len().saturating_sub(1) {
                                app.scroll_offset += 1;
                            }
                        }
                        KeyCode::Char('j') | KeyCode::Down => {
                            app.scroll_offset = app.scroll_offset.saturating_sub(1);
                        }
                        KeyCode::Char('1') => { app.mode = TuiMode::Plan; app.status = String::from("Mode: Plan"); }
                        KeyCode::Char('2') => { app.mode = TuiMode::Lite; app.status = String::from("Mode: Lite"); }
                        KeyCode::Char('3') => { app.mode = TuiMode::Full; app.status = String::from("Mode: Full"); }
                        KeyCode::Char('4') => { app.mode = TuiMode::Test; app.status = String::from("Mode: Test"); }
                        _ => {}
                    },
                    InputMode::Editing => match key.code {
                        KeyCode::Esc => {
                            app.input_mode = InputMode::Normal;
                        }
                        KeyCode::Enter => {
                            if !app.input.trim().is_empty() {
                                app.push_message("user", app.input.clone());
                                app.push_message("assistant", format!("[OntoLoop would process: '{}']", app.input));
                                app.input.clear();
                                app.cursor_pos = 0;
                                app.status = String::from("Ready");
                            }
                        }
                        KeyCode::Backspace => {
                            if app.cursor_pos > 0 {
                                app.cursor_pos -= 1;
                                let byte_pos = char_to_byte(&app.input, app.cursor_pos);
                                app.input.remove(byte_pos);
                            }
                        }
                        KeyCode::Left => { app.cursor_pos = app.cursor_pos.saturating_sub(1); }
                        KeyCode::Right => {
                            if app.cursor_pos < app.input.chars().count() { app.cursor_pos += 1; }
                        }
                        KeyCode::Home => app.cursor_pos = 0,
                        KeyCode::End => app.cursor_pos = app.input.chars().count(),
                        KeyCode::Char(c) => {
                            let byte_pos = char_to_byte(&app.input, app.cursor_pos);
                            app.input.insert(byte_pos, c);
                            app.cursor_pos += 1;
                        }
                        _ => {}
                    },
                }
            }
        }
    }
}

fn char_to_byte(s: &str, char_pos: usize) -> usize {
    s.chars().take(char_pos).map(|c| c.len_utf8()).sum()
}

fn render(frame: &mut ratatui::Frame, app: &App, theme: &Theme) {
    let area = frame.area();
    let main_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
            Constraint::Min(3),
            Constraint::Length(3),
            Constraint::Length(1),
        ])
        .split(area);

    render_header(frame, main_layout[0], app, theme);
    render_chat(frame, main_layout[1], app, theme);
    render_input(frame, main_layout[2], app, theme);
    render_status(frame, main_layout[3], app, theme);
}

fn render_header(frame: &mut ratatui::Frame, area: Rect, app: &App, theme: &Theme) {
    let lines: Vec<Line> = if app.messages.len() <= 1 {
        LOGO.lines().map(|l| {
            Line::from(Span::styled(l, Style::default().fg(ORANGE)))
        }).collect()
    } else {
        vec![Line::from(vec![
            Span::styled(" ■ OntoLoop ", Style::default().fg(theme.primary).bold()),
            Span::styled("| ", Style::default().fg(theme.text_dim)),
            Span::styled(app.mode.as_str(), Style::default().fg(theme.text)),
            Span::styled(" mode ", Style::default().fg(theme.text_dim)),
        ])]
    };

    let block = Block::default().borders(Borders::NONE).style(Style::default().bg(theme.bg));
    let paragraph = Paragraph::new(lines).block(block);
    frame.render_widget(paragraph, area);
}

fn render_chat(frame: &mut ratatui::Frame, area: Rect, app: &App, theme: &Theme) {
    let chat_height = (area.height as usize).saturating_sub(2);
    let visible = app.visible_messages(chat_height);

    let lines: Vec<Line> = visible.iter().map(|msg| {
        let (icon, color) = match msg.role.as_str() {
            "user" => ("▶", theme.primary),
            "assistant" => ("●", theme.info),
            "tool" => ("◆", theme.warning),
            _ => (" ", theme.text),
        };
        Line::from(vec![
            Span::styled(format!(" {} ", icon), Style::default().fg(color).bold()),
            Span::styled(msg.content.as_str(), Style::default().fg(theme.text)),
        ])
    }).collect();

    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(Style::default().fg(theme.border))
        .style(Style::default().bg(theme.bg));

    let paragraph = Paragraph::new(lines).block(block);
    frame.render_widget(paragraph, area);
}

fn render_input(frame: &mut ratatui::Frame, area: Rect, app: &App, theme: &Theme) {
    let border_color = if app.input_mode == InputMode::Editing {
        theme.border_active
    } else {
        theme.border
    };

    let hint = if app.input.is_empty() && app.input_mode == InputMode::Editing {
        " Type your message... (Esc: cancel, Enter: send)"
    } else if app.input.is_empty() {
        " i:type  |  1-4:mode  |  Tab:cycle  |  ↑↓:scroll  |  q:quit"
    } else {
        ""
    };

    let text = if app.input.is_empty() { hint.to_string() } else { app.input.clone() };
    let style = if app.input.is_empty() {
        Style::default().fg(theme.text_dim)
    } else {
        Style::default().fg(theme.text)
    };

    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(Style::default().fg(border_color))
        .style(Style::default().bg(theme.panel));

    frame.render_widget(Paragraph::new(text).style(style).block(block), area);
}

fn render_status(frame: &mut ratatui::Frame, area: Rect, app: &App, theme: &Theme) {
    let mode_span = Span::styled(
        format!(" {} ", app.mode.as_str()),
        Style::default().fg(theme.bg).bg(theme.primary).bold(),
    );

    let status_span = Span::styled(
        format!(" {} ", app.status),
        Style::default().fg(theme.text_muted),
    );

    let keys_span = Span::styled(
        " 1:Plan 2:Lite 3:Full 4:Test  Tab:cycle  i:type  q:quit ",
        Style::default().fg(theme.text_dim),
    );

    let line = Line::from(vec![mode_span, status_span, keys_span]);
    frame.render_widget(Paragraph::new(line).style(Style::default().bg(theme.panel)), area);
}
