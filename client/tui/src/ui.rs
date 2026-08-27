//! Config form widgets and key handling (keyboard-only).

use crate::config_gen::{FormState, RouteMode};
use crate::fetch_route;
use crate::spawn;
use crate::sudo::{self, PASSWORD_PROMPT};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Tabs, Wrap};
use ratatui::Frame;
use zeroize::{Zeroize, Zeroizing};

pub const STATUS_HINT: &str =
    "U4: Start launches hy via sudo; Stop = SIGINT. Save / Update hy unchanged";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Config,
    Run,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Server,
    Auth,
    Sni,
    VerifyCert,
    TunName,
    TunIpv4,
    Ipv4Exclude,
    WriteRoute,
    Timeout,
    RouteOff,
    RouteLocal,
    RouteUrl,
    RouteLocalPath,
    RouteUrlValue,
    AdvancedToggle,
    BwUp,
    BwDown,
    ObfsType,
    ObfsPassword,
    HopPorts,
    HopInterval,
    QuicInitStream,
    QuicMaxStream,
    QuicInitConn,
    QuicMaxConn,
    TunIpv6,
    Lazy,
    FastOpen,
    Socks5Listen,
    HyPath,
    Save,
    Start,
    UpdateHy,
    Stop,
    Restart,
}

impl Focus {
    fn is_text(self) -> bool {
        matches!(
            self,
            Focus::Server
                | Focus::Auth
                | Focus::Sni
                | Focus::TunName
                | Focus::TunIpv4
                | Focus::Ipv4Exclude
                | Focus::Timeout
                | Focus::RouteLocalPath
                | Focus::RouteUrlValue
                | Focus::BwUp
                | Focus::BwDown
                | Focus::ObfsType
                | Focus::ObfsPassword
                | Focus::HopPorts
                | Focus::HopInterval
                | Focus::QuicInitStream
                | Focus::QuicMaxStream
                | Focus::QuicInitConn
                | Focus::QuicMaxConn
                | Focus::TunIpv6
                | Focus::Socks5Listen
                | Focus::HyPath
        )
    }
}

fn focus_order(form: &FormState) -> Vec<Focus> {
    let mut v = vec![
        Focus::Server,
        Focus::Auth,
        Focus::Sni,
        Focus::VerifyCert,
        Focus::TunName,
        Focus::TunIpv4,
        Focus::Ipv4Exclude,
        Focus::WriteRoute,
        Focus::Timeout,
        Focus::RouteOff,
        Focus::RouteLocal,
        Focus::RouteUrl,
    ];
    match form.route_mode {
        RouteMode::Off => {}
        RouteMode::Local => v.push(Focus::RouteLocalPath),
        RouteMode::Url => v.push(Focus::RouteUrlValue),
    }
    v.push(Focus::AdvancedToggle);
    if form.advanced_expanded {
        v.extend([
            Focus::BwUp,
            Focus::BwDown,
            Focus::ObfsType,
            Focus::ObfsPassword,
            Focus::HopPorts,
            Focus::HopInterval,
            Focus::QuicInitStream,
            Focus::QuicMaxStream,
            Focus::QuicInitConn,
            Focus::QuicMaxConn,
            Focus::TunIpv6,
            Focus::Lazy,
            Focus::FastOpen,
            Focus::Socks5Listen,
            Focus::HyPath,
        ]);
    }
    v.extend([Focus::Save, Focus::Start, Focus::UpdateHy]);
    v
}

fn run_focus_order() -> [Focus; 2] {
    [Focus::Stop, Focus::Restart]
}

fn next_focus(form: &FormState, cur: Focus) -> Focus {
    let order = focus_order(form);
    match order.iter().position(|f| *f == cur) {
        Some(i) => order[(i + 1) % order.len()],
        None => order[0],
    }
}

fn prev_focus(form: &FormState, cur: Focus) -> Focus {
    let order = focus_order(form);
    match order.iter().position(|f| *f == cur) {
        Some(0) => *order.last().unwrap(),
        Some(i) => order[i - 1],
        None => order[0],
    }
}

pub enum Action {
    None,
    Quit,
    Save,
    Start,
    UpdateHy,
    Stop,
    Restart,
    PasswordSubmit,
    PasswordCancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunPhase {
    Stopped,
    Starting,
    Running,
    Stopping,
    Error,
}

impl RunPhase {
    pub fn label(self) -> &'static str {
        match self {
            Self::Stopped => "STOPPED",
            Self::Starting => "STARTING",
            Self::Running => "RUNNING",
            Self::Stopping => "STOPPING",
            Self::Error => "ERROR",
        }
    }
}

pub struct PasswordPrompt {
    buf: Zeroizing<String>,
    pub failures: u8,
}

impl PasswordPrompt {
    pub fn new(failures: u8) -> Self {
        Self {
            buf: Zeroizing::new(String::new()),
            failures,
        }
    }

    pub fn len(&self) -> usize {
        self.buf.chars().count()
    }

    pub fn take_buf(mut self) -> Zeroizing<String> {
        let mut empty = Zeroizing::new(String::new());
        std::mem::swap(&mut self.buf, &mut empty);
        empty
    }
}

impl Drop for PasswordPrompt {
    fn drop(&mut self) {
        Zeroize::zeroize(&mut *self.buf);
    }
}

pub struct App {
    pub tab: Tab,
    pub form: FormState,
    pub focus: Focus,
    pub status: String,
    pub status_error: bool,
    pub status_warn: bool,
    pub cursor: usize,
    scroll: u16,
    pub run_phase: RunPhase,
    pub hy_pid: Option<u32>,
    pub password_prompt: Option<PasswordPrompt>,
}

impl App {
    pub fn new() -> Self {
        let mut app = Self {
            tab: Tab::Config,
            form: FormState::default(),
            focus: Focus::Server,
            status: STATUS_HINT.to_string(),
            status_error: false,
            status_warn: false,
            cursor: 0,
            scroll: 0,
            run_phase: RunPhase::Stopped,
            hy_pid: None,
            password_prompt: None,
        };
        app.sync_cursor();
        app
    }

    pub fn begin_password_prompt(&mut self, failures: u8) {
        self.password_prompt = Some(PasswordPrompt::new(failures));
        self.set_status(PASSWORD_PROMPT, false);
    }

    pub fn take_password_prompt(&mut self) -> Option<PasswordPrompt> {
        self.password_prompt.take()
    }

    pub fn set_status(&mut self, text: impl Into<String>, error: bool) {
        self.status = text.into();
        self.status_error = error;
        self.status_warn = false;
    }

    pub fn set_status_warn(&mut self, text: impl Into<String>) {
        self.status = text.into();
        self.status_error = false;
        self.status_warn = true;
    }

    fn focused_text_mut(&mut self) -> Option<&mut String> {
        Some(match self.focus {
            Focus::Server => &mut self.form.server,
            Focus::Auth => &mut self.form.auth,
            Focus::Sni => &mut self.form.tls_sni,
            Focus::TunName => &mut self.form.tun_name,
            Focus::TunIpv4 => &mut self.form.tun_ipv4,
            Focus::Ipv4Exclude => &mut self.form.ipv4_exclude,
            Focus::Timeout => &mut self.form.timeout,
            Focus::RouteLocalPath => &mut self.form.route_local_path,
            Focus::RouteUrlValue => &mut self.form.route_url,
            Focus::BwUp => &mut self.form.bandwidth_up,
            Focus::BwDown => &mut self.form.bandwidth_down,
            Focus::ObfsType => &mut self.form.obfs_type,
            Focus::ObfsPassword => &mut self.form.obfs_password,
            Focus::HopPorts => &mut self.form.hop_ports,
            Focus::HopInterval => &mut self.form.hop_interval,
            Focus::QuicInitStream => &mut self.form.quic_init_stream_window,
            Focus::QuicMaxStream => &mut self.form.quic_max_stream_window,
            Focus::QuicInitConn => &mut self.form.quic_init_conn_window,
            Focus::QuicMaxConn => &mut self.form.quic_max_conn_window,
            Focus::TunIpv6 => &mut self.form.tun_ipv6,
            Focus::Socks5Listen => &mut self.form.socks5_listen,
            Focus::HyPath => &mut self.form.hy_path,
            _ => return None,
        })
    }

    fn sync_cursor(&mut self) {
        let len = self
            .focused_text_mut()
            .map(|s| s.chars().count())
            .unwrap_or(0);
        self.cursor = len;
    }

    fn insert_char(&mut self, c: char) {
        let cur = self.cursor;
        if let Some(s) = self.focused_text_mut() {
            let byte = s.char_indices().nth(cur).map(|(i, _)| i).unwrap_or(s.len());
            s.insert(byte, c);
            self.cursor = cur + 1;
        }
    }

    fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let cur = self.cursor;
        if let Some(s) = self.focused_text_mut() {
            if let Some((byte, ch)) = s.char_indices().nth(cur - 1) {
                let end = byte + ch.len_utf8();
                s.replace_range(byte..end, "");
                self.cursor = cur - 1;
            }
        }
    }

    fn delete(&mut self) {
        let cur = self.cursor;
        if let Some(s) = self.focused_text_mut() {
            if let Some((byte, ch)) = s.char_indices().nth(cur) {
                let end = byte + ch.len_utf8();
                s.replace_range(byte..end, "");
            }
        }
    }

    fn cursor_left(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
        }
    }

    fn cursor_right(&mut self) {
        let len = self
            .focused_text_mut()
            .map(|s| s.chars().count())
            .unwrap_or(0);
        if self.cursor < len {
            self.cursor += 1;
        }
    }
}

pub fn handle_key(app: &mut App, key: KeyEvent) -> Action {
    if app.password_prompt.is_some() {
        return handle_password_key(app, key);
    }
    if key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('c')) {
        return Action::Quit;
    }
    match key.code {
        KeyCode::Esc => return Action::Quit,
        KeyCode::Char('1') if !app.focus.is_text() || app.tab != Tab::Config => {
            app.tab = Tab::Config;
            return Action::None;
        }
        KeyCode::Char('2') if !app.focus.is_text() || app.tab != Tab::Config => {
            app.tab = Tab::Run;
            if !matches!(app.focus, Focus::Stop | Focus::Restart) {
                app.focus = Focus::Stop;
            }
            return Action::None;
        }
        KeyCode::Tab => {
            app.tab = match app.tab {
                Tab::Config => {
                    app.focus = Focus::Stop;
                    Tab::Run
                }
                Tab::Run => Tab::Config,
            };
            return Action::None;
        }
        _ => {}
    }

    if app.tab == Tab::Run {
        return handle_run_key(app, key);
    }

    if app.tab != Tab::Config {
        return Action::None;
    }

    match key.code {
        KeyCode::Down => {
            app.focus = next_focus(&app.form, app.focus);
            app.sync_cursor();
        }
        KeyCode::Up | KeyCode::BackTab => {
            app.focus = prev_focus(&app.form, app.focus);
            app.sync_cursor();
        }
        KeyCode::Left if app.focus.is_text() => app.cursor_left(),
        KeyCode::Right if app.focus.is_text() => app.cursor_right(),
        KeyCode::Home if app.focus.is_text() => app.cursor = 0,
        KeyCode::End if app.focus.is_text() => app.sync_cursor(),
        KeyCode::Backspace if app.focus.is_text() => app.backspace(),
        KeyCode::Delete if app.focus.is_text() => app.delete(),
        KeyCode::Char(' ') if !app.focus.is_text() => return activate(app),
        KeyCode::Enter => return activate(app),
        KeyCode::Char(c)
            if app.focus.is_text() && !key.modifiers.contains(KeyModifiers::CONTROL) =>
        {
            app.insert_char(c);
        }
        _ => {}
    }
    Action::None
}

fn activate(app: &mut App) -> Action {
    match app.focus {
        Focus::VerifyCert => {
            app.form.verify_cert = !app.form.verify_cert;
        }
        Focus::WriteRoute => {
            app.form.write_route = !app.form.write_route;
        }
        Focus::RouteOff => app.form.route_mode = RouteMode::Off,
        Focus::RouteLocal => app.form.route_mode = RouteMode::Local,
        Focus::RouteUrl => app.form.route_mode = RouteMode::Url,
        Focus::AdvancedToggle => {
            app.form.advanced_expanded = !app.form.advanced_expanded;
            if !app.form.advanced_expanded {
                app.focus = Focus::AdvancedToggle;
            }
        }
        Focus::Lazy => app.form.lazy = !app.form.lazy,
        Focus::FastOpen => app.form.fast_open = !app.form.fast_open,
        Focus::Save => return Action::Save,
        Focus::Start => return Action::Start,
        Focus::UpdateHy => return Action::UpdateHy,
        Focus::Stop => return Action::Stop,
        Focus::Restart => return Action::Restart,
        _ => {}
    }
    Action::None
}

fn handle_run_key(app: &mut App, key: KeyEvent) -> Action {
    let order = run_focus_order();
    if !order.contains(&app.focus) {
        app.focus = order[0];
    }
    match key.code {
        KeyCode::Left | KeyCode::Up | KeyCode::BackTab => {
            app.focus = order[0];
            Action::None
        }
        KeyCode::Right | KeyCode::Down => {
            app.focus = order[1];
            Action::None
        }
        KeyCode::Char(' ') | KeyCode::Enter => activate(app),
        _ => Action::None,
    }
}

fn handle_password_key(app: &mut App, key: KeyEvent) -> Action {
    if key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('c')) {
        let _ = app.take_password_prompt();
        return Action::PasswordCancel;
    }
    match key.code {
        KeyCode::Esc => {
            let _ = app.take_password_prompt();
            Action::PasswordCancel
        }
        KeyCode::Enter => Action::PasswordSubmit,
        KeyCode::Backspace => {
            if let Some(p) = app.password_prompt.as_mut() {
                p.buf.pop();
            }
            Action::None
        }
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            if let Some(p) = app.password_prompt.as_mut() {
                p.buf.push(c);
            }
            Action::None
        }
        _ => Action::None,
    }
}

pub fn note_save_ok(app: &mut App, path: &str) {
    app.set_status(format!("saved {path}"), false);
}

pub fn note_save_err(app: &mut App, err: &str) {
    app.set_status(format!("save failed: {err}"), true);
}

pub fn note_downloading(app: &mut App) {
    app.set_status("downloading…", false);
}

pub fn note_fetch_ok(app: &mut App, tag: &str) {
    let tag = tag.trim();
    let v = if tag.is_empty() {
        String::new()
    } else if tag.starts_with(['v', 'V']) {
        format!(" ({tag})")
    } else {
        format!(" (v{tag})")
    };
    app.set_status(format!("hy → ~/.hy/bin/hy{v}"), false);
}

pub fn note_fetch_err(app: &mut App, err: &str) {
    let one_line: String = err
        .chars()
        .map(|c| if c == '\n' { ' ' } else { c })
        .take(240)
        .collect();
    app.set_status(one_line, true);
}

pub fn note_starting(app: &mut App) {
    app.run_phase = RunPhase::Starting;
    app.set_status("preparing start…", false);
}

#[allow(dead_code)]
pub fn note_start_ok(app: &mut App, prepared: &spawn::PreparedStart) {
    let cmd = spawn::format_argv(&prepared.argv);
    if prepared.ruleset_warning {
        app.set_status_warn(format!("{}  {cmd}", fetch_route::RULESET_UNUSABLE));
    } else {
        app.set_status(cmd, false);
    }
}

pub fn note_need_password(app: &mut App, failures: u8) {
    app.tab = Tab::Config;
    app.begin_password_prompt(failures);
}

pub fn note_password_cancel(app: &mut App) {
    app.run_phase = RunPhase::Stopped;
    app.hy_pid = None;
    app.set_status("已取消启动", false);
}

pub fn note_password_fail(app: &mut App, failures: u8, locked: bool) {
    app.tab = Tab::Config;
    if locked {
        app.run_phase = RunPhase::Stopped;
        app.hy_pid = None;
        app.set_status("sudo 密码错误三次，未启动 hy", true);
    } else {
        app.begin_password_prompt(failures);
        app.set_status(
            format!(
                "sudo 密码错误（{failures}/{}），请重试",
                sudo::MAX_PASSWORD_FAILS
            ),
            true,
        );
    }
}

pub fn note_spawned(app: &mut App, pid: u32, ruleset_warning: bool) {
    app.tab = Tab::Run;
    app.focus = Focus::Stop;
    app.run_phase = RunPhase::Running;
    app.hy_pid = Some(pid);
    let msg = format!("hy pid {pid}");
    if ruleset_warning {
        app.set_status_warn(format!("{}  {msg}", fetch_route::RULESET_UNUSABLE));
    } else {
        app.set_status(msg, false);
    }
}

pub fn note_stopping(app: &mut App) {
    app.run_phase = RunPhase::Stopping;
    app.set_status("stopping (SIGINT)…", false);
}

pub fn note_stopped(app: &mut App) {
    app.run_phase = RunPhase::Stopped;
    app.hy_pid = None;
    app.set_status("STOPPED", false);
}

pub fn note_hy_exited(app: &mut App) {
    app.run_phase = RunPhase::Error;
    app.hy_pid = None;
    app.set_status("hy exited", true);
}

pub fn note_no_tty(app: &mut App) {
    app.run_phase = RunPhase::Stopped;
    app.set_status(sudo::NO_TTY_MSG, true);
}

pub fn note_start_err(app: &mut App, err: &str) {
    app.run_phase = RunPhase::Stopped;
    app.hy_pid = None;
    let one_line: String = err
        .chars()
        .map(|c| if c == '\n' { ' ' } else { c })
        .take(240)
        .collect();
    app.set_status(format!("start failed: {one_line}"), true);
}

pub fn note_stop_err(app: &mut App, err: &str) {
    let one_line: String = err
        .chars()
        .map(|c| if c == '\n' { ' ' } else { c })
        .take(240)
        .collect();
    app.set_status(format!("stop failed: {one_line}"), true);
}

pub fn draw(frame: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(1),
            Constraint::Min(8),
            Constraint::Length(3),
        ])
        .split(frame.area());

    let titles = vec!["1 Config", "2 Run"];
    let selected = match app.tab {
        Tab::Config => 0,
        Tab::Run => 1,
    };
    let tabs = Tabs::new(titles)
        .block(Block::default().borders(Borders::ALL).title("hy-tui"))
        .highlight_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
        .select(selected);
    frame.render_widget(tabs, chunks[0]);

    let status_style = if app.status_error {
        Style::default().fg(Color::Red)
    } else if app.status_warn {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::Cyan)
    };
    let status = Paragraph::new(app.status.as_str()).style(status_style);
    frame.render_widget(status, chunks[1]);

    match app.tab {
        Tab::Config => draw_config(frame, app, chunks[2]),
        Tab::Run => draw_run(frame, app, chunks[2]),
    }

    draw_footer(frame, app, chunks[3]);
    if app.password_prompt.is_some() {
        draw_password_overlay(frame, app, frame.area());
    }
}

fn draw_run(frame: &mut Frame, app: &App, area: Rect) {
    let pid = app
        .hy_pid
        .map(|p| p.to_string())
        .unwrap_or_else(|| "—".into());
    let p = Paragraph::new(vec![
        Line::from(""),
        Line::from(format!("status: {}", app.run_phase.label())),
        Line::from(format!("pid: {pid}")),
        Line::from(""),
        Line::from("Stop = SIGINT to hy (then SIGTERM on timeout). Never SIGKILL."),
        Line::from("U5 will fill TUN rates and the log pane."),
        Line::from(""),
        Line::from("Tab / 1 / 2  switch tabs    Esc  quit"),
    ])
    .block(Block::default().borders(Borders::ALL).title("Run"));
    frame.render_widget(p, area);
}

fn draw_footer(frame: &mut Frame, app: &App, area: Rect) {
    let btn = |id: Focus, label: &str| -> Span<'static> {
        let focused = match app.tab {
            Tab::Config => {
                matches!(id, Focus::Save | Focus::Start | Focus::UpdateHy) && app.focus == id
            }
            Tab::Run => matches!(id, Focus::Stop | Focus::Restart) && app.focus == id,
        };
        let style = if focused {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };
        Span::styled(format!(" {label} "), style)
    };
    let line = match app.tab {
        Tab::Config => Line::from(vec![
            btn(Focus::Save, "Save"),
            Span::raw("  "),
            btn(Focus::Start, "Start"),
            Span::raw("  "),
            btn(Focus::UpdateHy, "Update hy"),
            Span::raw("    Enter=activate when focused   Space=toggle   ↑↓=move"),
        ]),
        Tab::Run => Line::from(vec![
            btn(Focus::Stop, "Stop"),
            Span::raw("  "),
            btn(Focus::Restart, "Restart"),
            Span::raw("    Enter=activate   Stop=SIGINT then SIGTERM"),
        ]),
    };
    let p = Paragraph::new(line).block(Block::default().borders(Borders::ALL).title("actions"));
    frame.render_widget(p, area);
}

fn draw_password_overlay(frame: &mut Frame, app: &App, area: Rect) {
    let Some(prompt) = app.password_prompt.as_ref() else {
        return;
    };
    let w = 52.min(area.width.saturating_sub(2)).max(24);
    let h = 7.min(area.height.saturating_sub(2)).max(5);
    let x = area.x + area.width.saturating_sub(w) / 2;
    let y = area.y + area.height.saturating_sub(h) / 2;
    let rect = Rect::new(x, y, w, h);
    let mask = "•".repeat(prompt.len());
    let attempt = prompt.failures + 1;
    let lines = vec![
        Line::from(PASSWORD_PROMPT),
        Line::from(format!("Password: {mask}│")),
        Line::from(format!(
            "Enter=OK  Esc=取消  ({attempt}/{})",
            sudo::MAX_PASSWORD_FAILS
        )),
    ];
    let p = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .title("sudo")
            .style(Style::default().fg(Color::Yellow)),
    );
    frame.render_widget(Clear, rect);
    frame.render_widget(p, rect);
}

fn draw_config(frame: &mut Frame, app: &App, area: Rect) {
    let lines = form_lines(app);
    let height = area.height.saturating_sub(2);
    let mut scroll = app.scroll;
    if let Some(idx) = focused_line_index(app) {
        let idx = idx as u16;
        if idx < scroll {
            scroll = idx;
        } else if idx >= scroll + height && height > 0 {
            scroll = idx.saturating_sub(height - 1);
        }
    }
    let p = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title("Config"))
        .scroll((scroll, 0))
        .wrap(Wrap { trim: false });
    frame.render_widget(Clear, area);
    frame.render_widget(p, area);
}

fn focused_line_index(app: &App) -> Option<usize> {
    // Keep the focused control near the top of the visible form.
    Some(match app.focus {
        Focus::Server => 1,
        Focus::Auth => 2,
        Focus::Sni => 3,
        Focus::VerifyCert => 4,
        Focus::TunName => 6,
        Focus::TunIpv4 => 7,
        Focus::Ipv4Exclude => 8,
        Focus::WriteRoute => 9,
        Focus::Timeout => 10,
        Focus::RouteOff | Focus::RouteLocal | Focus::RouteUrl => 12,
        Focus::RouteLocalPath | Focus::RouteUrlValue => 13,
        Focus::AdvancedToggle => 15,
        Focus::Save | Focus::Start | Focus::UpdateHy | Focus::Stop | Focus::Restart => 0,
        _ => 16,
    })
}

fn form_lines(app: &App) -> Vec<Line<'static>> {
    let f = &app.form;
    let mut lines = Vec::new();
    lines.push(section("连接"));
    lines.push(text_row(app, Focus::Server, "server", &f.server));
    lines.push(text_row(app, Focus::Auth, "auth", &f.auth));
    lines.push(text_row(app, Focus::Sni, "tls.sni", &f.tls_sni));
    lines.push(check_row(
        app,
        Focus::VerifyCert,
        "校验证书",
        f.verify_cert,
        "off → tls.insecure: true",
    ));

    lines.push(section("TUN"));
    lines.push(text_row(app, Focus::TunName, "name", &f.tun_name));
    lines.push(text_row(app, Focus::TunIpv4, "address.ipv4", &f.tun_ipv4));
    lines.push(text_row(
        app,
        Focus::Ipv4Exclude,
        "ipv4Exclude",
        &f.ipv4_exclude,
    ));
    lines.push(check_row(
        app,
        Focus::WriteRoute,
        "write route:",
        f.write_route,
        "off → device only",
    ));
    lines.push(text_row(app, Focus::Timeout, "timeout", &f.timeout));

    lines.push(section("路由"));
    lines.push(radio_row(app, f.route_mode));
    match f.route_mode {
        RouteMode::Off => lines.push(dim(
            "  off: Start omits --route; Save still does not write route.file",
        )),
        RouteMode::Local => {
            lines.push(text_row(
                app,
                Focus::RouteLocalPath,
                "local .conf",
                &f.route_local_path,
            ));
            lines.push(dim("  Start passes --route <abs path> (file must exist)"));
        }
        RouteMode::Url => {
            lines.push(text_row(
                app,
                Focus::RouteUrlValue,
                "HTTPS URL",
                &f.route_url,
            ));
            lines.push(dim(
                "  Start downloads to ~/.hy/route.conf and passes --route",
            ));
        }
    }

    lines.push(section("高级"));
    let adv_label = if f.advanced_expanded {
        "折叠高级"
    } else {
        "展开高级"
    };
    lines.push(button_row(app, Focus::AdvancedToggle, adv_label));
    if f.advanced_expanded {
        lines.push(text_row(app, Focus::BwUp, "bandwidth.up", &f.bandwidth_up));
        lines.push(text_row(
            app,
            Focus::BwDown,
            "bandwidth.down",
            &f.bandwidth_down,
        ));
        lines.push(text_row(app, Focus::ObfsType, "obfs.type", &f.obfs_type));
        lines.push(text_row(
            app,
            Focus::ObfsPassword,
            "obfs.password",
            &f.obfs_password,
        ));
        lines.push(text_row(app, Focus::HopPorts, "hop ports", &f.hop_ports));
        lines.push(text_row(
            app,
            Focus::HopInterval,
            "hopInterval",
            &f.hop_interval,
        ));
        lines.push(text_row(
            app,
            Focus::QuicInitStream,
            "quic.initStreamReceiveWindow",
            &f.quic_init_stream_window,
        ));
        lines.push(text_row(
            app,
            Focus::QuicMaxStream,
            "quic.maxStreamReceiveWindow",
            &f.quic_max_stream_window,
        ));
        lines.push(text_row(
            app,
            Focus::QuicInitConn,
            "quic.initConnReceiveWindow",
            &f.quic_init_conn_window,
        ));
        lines.push(text_row(
            app,
            Focus::QuicMaxConn,
            "quic.maxConnReceiveWindow",
            &f.quic_max_conn_window,
        ));
        lines.push(text_row(app, Focus::TunIpv6, "address.ipv6", &f.tun_ipv6));
        lines.push(check_row(app, Focus::Lazy, "lazy", f.lazy, ""));
        lines.push(check_row(app, Focus::FastOpen, "fastOpen", f.fast_open, ""));
        lines.push(text_row(
            app,
            Focus::Socks5Listen,
            "socks5.listen",
            &f.socks5_listen,
        ));
        lines.push(text_row(app, Focus::HyPath, "hy path", &f.hy_path));
        lines.push(dim(
            "  empty advanced keys are omitted from yaml; Update hy always writes ~/.hy/bin/hy",
        ));
    }
    lines
}

fn section(title: &str) -> Line<'static> {
    Line::from(Span::styled(
        format!("[{title}]"),
        Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD),
    ))
}

fn dim(s: &str) -> Line<'static> {
    Line::from(Span::styled(
        s.to_string(),
        Style::default().fg(Color::DarkGray),
    ))
}

fn mark(app: &App, id: Focus) -> &'static str {
    if app.focus == id {
        ">"
    } else {
        " "
    }
}

fn field_style(app: &App, id: Focus) -> Style {
    if app.focus == id {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    }
}

fn text_row(app: &App, id: Focus, label: &str, value: &str) -> Line<'static> {
    let mut shown = value.to_string();
    if app.focus == id {
        let mut chars: Vec<char> = shown.chars().collect();
        let i = app.cursor.min(chars.len());
        chars.insert(i, '│');
        shown = chars.into_iter().collect();
    }
    Line::from(vec![
        Span::styled(format!("{} {label}: ", mark(app, id)), field_style(app, id)),
        Span::styled(shown, field_style(app, id)),
    ])
}

fn check_row(app: &App, id: Focus, label: &str, on: bool, hint: &str) -> Line<'static> {
    let boxc = if on { "[x]" } else { "[ ]" };
    let mut spans = vec![Span::styled(
        format!("{} {boxc} {label}", mark(app, id)),
        field_style(app, id),
    )];
    if !hint.is_empty() {
        spans.push(Span::styled(
            format!("  {hint}"),
            Style::default().fg(Color::DarkGray),
        ));
    }
    Line::from(spans)
}

fn button_row(app: &App, id: Focus, label: &str) -> Line<'static> {
    Line::from(Span::styled(
        format!("{} [{label}]", mark(app, id)),
        field_style(app, id),
    ))
}

fn radio_row(app: &App, mode: RouteMode) -> Line<'static> {
    let item = |id: Focus, this: RouteMode, label: &str| -> Span<'static> {
        let mark_c = if app.focus == id { ">" } else { " " };
        let radio = if mode == this { "(•)" } else { "( )" };
        Span::styled(format!("{mark_c}{radio} {label}  "), field_style(app, id))
    };
    Line::from(vec![
        item(Focus::RouteOff, RouteMode::Off, "off"),
        item(Focus::RouteLocal, RouteMode::Local, "local"),
        item(Focus::RouteUrl, RouteMode::Url, "url"),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn password_esc_cancels_and_does_not_quit() {
        let mut app = App::new();
        app.begin_password_prompt(0);
        let action = handle_key(&mut app, key(KeyCode::Esc));
        assert!(matches!(action, Action::PasswordCancel));
        assert!(app.password_prompt.is_none());
    }

    #[test]
    fn password_enter_submits() {
        let mut app = App::new();
        app.begin_password_prompt(1);
        let action = handle_key(&mut app, key(KeyCode::Enter));
        assert!(matches!(action, Action::PasswordSubmit));
        assert!(app.password_prompt.is_some());
    }

    #[test]
    fn run_enter_on_stop_is_stop_action() {
        let mut app = App::new();
        app.tab = Tab::Run;
        app.focus = Focus::Stop;
        let action = handle_key(&mut app, key(KeyCode::Enter));
        assert!(matches!(action, Action::Stop));
    }
}
