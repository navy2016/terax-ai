use anyhow::{anyhow, Context, Result};
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
    Frame, Terminal,
};
use serde::{Deserialize, Serialize};
use std::{
    env,
    fs,
    io::{self, Stdout},
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};
use walkdir::WalkDir;

#[derive(Debug, Clone, Deserialize)]
struct AiConfig {
    base_url: String,
    api_key: String,
    model: String,
}

#[derive(Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    stream: bool,
}

#[derive(Serialize, Deserialize)]
struct ChatMessage {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatMessage,
}

fn load_ai_config() -> Result<AiConfig> {
    let home = env::var("HOME").context("HOME is not set")?;
    let path = PathBuf::from(home).join(".config/terax-tui/config.toml");
    let text = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    let cfg: AiConfig = toml::from_str(&text).with_context(|| format!("parse {}", path.display()))?;
    Ok(cfg)
}

fn call_ai(prompt: &str) -> Result<String> {
    let cfg = load_ai_config()?;
    let url = format!("{}/chat/completions", cfg.base_url.trim_end_matches('/'));
    let req = ChatRequest {
        model: cfg.model,
        stream: false,
        messages: vec![
            ChatMessage { role: "system".into(), content: "You are Terax TUI assistant. Be concise and practical.".into() },
            ChatMessage { role: "user".into(), content: prompt.to_string() },
        ],
    };
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()?;
    let res = client
        .post(url)
        .bearer_auth(cfg.api_key)
        .json(&req)
        .send()
        .context("AI request failed")?;
    let status = res.status();
    let body = res.text().context("read AI response")?;
    if !status.is_success() {
        return Err(anyhow!("AI HTTP {}: {}", status, body));
    }
    let parsed: ChatResponse = serde_json::from_str(&body).context("parse AI response")?;
    parsed.choices.into_iter().next()
        .map(|c| c.message.content)
        .ok_or_else(|| anyhow!("AI response has no choices"))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Focus {
    Files,
    Command,
    Log,
}

struct App {
    cwd: PathBuf,
    entries: Vec<PathBuf>,
    selected: usize,
    preview: String,
    command: String,
    logs: Vec<String>,
    focus: Focus,
    should_quit: bool,
}

impl App {
    fn new() -> Result<Self> {
        let cwd = env::current_dir().context("failed to read current directory")?;
        let mut app = Self {
            cwd,
            entries: Vec::new(),
            selected: 0,
            preview: String::new(),
            command: String::new(),
            logs: vec![
                "Terax TUI started.".into(),
                "Keys: q quit | Tab focus | ↑/↓ move | Enter open/run | : command | r refresh | ai <prompt>".into(),
            ],
            focus: Focus::Files,
            should_quit: false,
        };
        app.refresh()?;
        Ok(app)
    }

    fn refresh(&mut self) -> Result<()> {
        self.entries.clear();
        self.entries.push(self.cwd.join(".."));

        let mut dirs = Vec::new();
        let mut files = Vec::new();
        for entry in fs::read_dir(&self.cwd).with_context(|| format!("read_dir {}", self.cwd.display()))? {
            let entry = entry?;
            let path = entry.path();
            let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
            if name.starts_with('.') && name != ".github" {
                continue;
            }
            if path.is_dir() {
                dirs.push(path);
            } else {
                files.push(path);
            }
        }
        dirs.sort();
        files.sort();
        self.entries.extend(dirs);
        self.entries.extend(files);
        if self.selected >= self.entries.len() {
            self.selected = self.entries.len().saturating_sub(1);
        }
        self.update_preview();
        Ok(())
    }

    fn selected_path(&self) -> Option<&Path> {
        self.entries.get(self.selected).map(|p| p.as_path())
    }

    fn update_preview(&mut self) {
        let Some(path) = self.selected_path() else {
            self.preview.clear();
            return;
        };
        if path.is_dir() {
            let count = WalkDir::new(path).max_depth(2).into_iter().filter_map(Result::ok).count();
            self.preview = format!("Directory: {}\nEntries within depth 2: {}", path.display(), count);
            return;
        }
        match fs::metadata(path) {
            Ok(meta) if meta.len() > 256 * 1024 => {
                self.preview = format!("File: {}\nSize: {} bytes\nPreview skipped: file too large", path.display(), meta.len());
            }
            Ok(_) => match fs::read_to_string(path) {
                Ok(s) => {
                    self.preview = s.lines().take(200).collect::<Vec<_>>().join("\n");
                }
                Err(e) => {
                    self.preview = format!("File: {}\nBinary or unreadable: {e}", path.display());
                }
            },
            Err(e) => self.preview = format!("metadata error: {e}"),
        }
    }

    fn open_selected(&mut self) -> Result<()> {
        let Some(path) = self.selected_path().map(Path::to_path_buf) else { return Ok(()); };
        if path.is_dir() {
            self.cwd = fs::canonicalize(&path).unwrap_or(path);
            self.selected = 0;
            self.refresh()?;
            self.logs.push(format!("cwd -> {}", self.cwd.display()));
        } else {
            self.update_preview();
            self.logs.push(format!("preview -> {}", path.display()));
        }
        Ok(())
    }

    fn run_command(&mut self) {
        let cmd = self.command.trim().to_string();
        if cmd.is_empty() {
            return;
        }
        self.logs.push(format!("> {cmd}"));
        if cmd == "quit" || cmd == "exit" {
            self.should_quit = true;
            return;
        }
        if cmd == "pwd" {
            self.logs.push(self.cwd.display().to_string());
            self.command.clear();
            return;
        }
        if let Some(prompt) = cmd.strip_prefix("ai ") {
            self.logs.push("AI: thinking...".into());
            match call_ai(prompt.trim()) {
                Ok(answer) => {
                    self.logs.push("AI:".into());
                    for line in answer.lines().take(200) {
                        self.logs.push(line.to_string());
                    }
                }
                Err(e) => self.logs.push(format!("AI error: {e:#}")),
            }
            self.command.clear();
            return;
        }
        if let Some(rest) = cmd.strip_prefix("cd ") {
            let target = self.cwd.join(rest.trim());
            if target.is_dir() {
                self.cwd = fs::canonicalize(target).unwrap_or_else(|_| self.cwd.clone());
                self.selected = 0;
                if let Err(e) = self.refresh() {
                    self.logs.push(format!("refresh error: {e:#}"));
                }
            } else {
                self.logs.push("cd: not a directory".into());
            }
            self.command.clear();
            return;
        }
        let output = Command::new("sh")
            .arg("-lc")
            .arg(&cmd)
            .current_dir(&self.cwd)
            .output();
        match output {
            Ok(out) => {
                let stdout = String::from_utf8_lossy(&out.stdout);
                let stderr = String::from_utf8_lossy(&out.stderr);
                for line in stdout.lines().take(120) {
                    self.logs.push(line.to_string());
                }
                for line in stderr.lines().take(120) {
                    self.logs.push(format!("ERR: {line}"));
                }
                self.logs.push(format!("exit: {}", out.status));
            }
            Err(e) => self.logs.push(format!("command failed: {e}")),
        }
        self.command.clear();
    }

    fn on_key(&mut self, key: KeyEvent) -> Result<()> {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.should_quit = true;
            return Ok(());
        }
        match self.focus {
            Focus::Files => match key.code {
                KeyCode::Char('q') => self.should_quit = true,
                KeyCode::Tab => self.focus = Focus::Command,
                KeyCode::Char(':') => self.focus = Focus::Command,
                KeyCode::Char('r') => self.refresh()?,
                KeyCode::Up => {
                    self.selected = self.selected.saturating_sub(1);
                    self.update_preview();
                }
                KeyCode::Down => {
                    if self.selected + 1 < self.entries.len() {
                        self.selected += 1;
                    }
                    self.update_preview();
                }
                KeyCode::Enter => self.open_selected()?,
                _ => {}
            },
            Focus::Command => match key.code {
                KeyCode::Esc => self.focus = Focus::Files,
                KeyCode::Tab => self.focus = Focus::Log,
                KeyCode::Enter => self.run_command(),
                KeyCode::Backspace => {
                    self.command.pop();
                }
                KeyCode::Char(c) => self.command.push(c),
                _ => {}
            },
            Focus::Log => match key.code {
                KeyCode::Tab | KeyCode::Esc => self.focus = Focus::Files,
                KeyCode::Char('q') => self.should_quit = true,
                _ => {}
            },
        }
        Ok(())
    }
}

fn ui(frame: &mut Frame, app: &mut App) {
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(8), Constraint::Length(3), Constraint::Length(7)])
        .split(frame.area());

    let main = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(34), Constraint::Percentage(66)])
        .split(root[0]);

    render_files(frame, app, main[0]);
    render_preview(frame, app, main[1]);
    render_command(frame, app, root[1]);
    render_log(frame, app, root[2]);
}

fn focus_style(active: bool) -> Style {
    if active { Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD) } else { Style::default() }
}

fn render_files(frame: &mut Frame, app: &mut App, area: Rect) {
    let items: Vec<ListItem> = app.entries.iter().map(|p| {
        let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("..");
        let icon = if p.is_dir() { "▸" } else { " " };
        ListItem::new(Line::from(vec![Span::raw(icon), Span::raw(" "), Span::raw(name.to_string())]))
    }).collect();
    let mut state = ListState::default();
    state.select(Some(app.selected));
    let list = List::new(items)
        .block(Block::default().title(format!(" Files: {} ", app.cwd.display())).borders(Borders::ALL).border_style(focus_style(app.focus == Focus::Files)))
        .highlight_style(Style::default().bg(Color::DarkGray).fg(Color::White).add_modifier(Modifier::BOLD));
    frame.render_stateful_widget(list, area, &mut state);
}

fn render_preview(frame: &mut Frame, app: &App, area: Rect) {
    let p = Paragraph::new(app.preview.clone())
        .block(Block::default().title(" Preview ").borders(Borders::ALL))
        .wrap(Wrap { trim: false });
    frame.render_widget(p, area);
}

fn render_command(frame: &mut Frame, app: &App, area: Rect) {
    let p = Paragraph::new(format!(":{}", app.command))
        .block(Block::default().title(" Command ").borders(Borders::ALL).border_style(focus_style(app.focus == Focus::Command)));
    frame.render_widget(p, area);
}

fn render_log(frame: &mut Frame, app: &App, area: Rect) {
    let height = area.height.saturating_sub(2) as usize;
    let start = app.logs.len().saturating_sub(height);
    let text = app.logs[start..].join("\n");
    let p = Paragraph::new(text)
        .block(Block::default().title(" Log ").borders(Borders::ALL).border_style(focus_style(app.focus == Focus::Log)))
        .wrap(Wrap { trim: false });
    frame.render_widget(p, area);
}

struct TerminalGuard;
impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
    }
}

fn run() -> Result<()> {
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen)?;
    let _guard = TerminalGuard;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal: Terminal<CrosstermBackend<Stdout>> = Terminal::new(backend)?;
    let mut app = App::new()?;

    while !app.should_quit {
        terminal.draw(|f| ui(f, &mut app))?;
        if event::poll(Duration::from_millis(200))? {
            if let Event::Key(key) = event::read()? {
                app.on_key(key)?;
            }
        }
    }
    Ok(())
}

fn main() {
    if let Err(e) = run() {
        eprintln!("terax-tui error: {e:#}");
        std::process::exit(1);
    }
}
