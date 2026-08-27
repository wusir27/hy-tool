//! Config form widgets and key handling (keyboard-only).

use crate::config_gen::{FormState, RouteMode};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Tabs, Wrap};
use ratatui::Frame;

pub const STATUS_U1: &str = "U1: Save only";

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
    Save,
    Start,
    UpdateHy,
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
        ]);
    }
    v.extend([Focus::Save, Focus::Start, Focus::UpdateHy]);
    v
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
}

pub struct App {
    pub tab: Tab,
    pub form: FormState,
    pub focus: Focus,
    pub status: String,
    pub cursor: usize,
    scroll: u16,
}

impl App {
    pub fn new() -> Self {
        let mut app = Self {
            tab: Tab::Config,
            form: FormState::default(),
            focus: Focus::Server,
            status: STATUS_U1.to_string(),
            cursor: 0,
            scroll: 0,
        };
        app.sync_cursor();
        app
    }

    fn set_status_extra(&mut self, extra: &str) {
        if extra.is_empty() {
            self.status = STATUS_U1.to_string();
        } else {
            self.status = format!("{STATUS_U1} | {extra}");
        }
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
            return Action::None;
        }
        KeyCode::Tab => {
            app.tab = match app.tab {
                Tab::Config => Tab::Run,
                Tab::Run => Tab::Config,
            };
            return Action::None;
        }
        _ => {}
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
        Focus::Start | Focus::UpdateHy => {
            app.set_status_extra("Start / Update hy are no-ops in U1");
        }
        _ => {}
    }
    Action::None
}

pub fn note_save_ok(app: &mut App, path: &str) {
    app.set_status_extra(&format!("saved {path}"));
}

pub fn note_save_err(app: &mut App, err: &str) {
    app.set_status_extra(&format!("save failed: {err}"));
}

pub fn draw(frame: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(3),
            Constraint::Length(1),
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

    match app.tab {
        Tab::Config => draw_config(frame, app, chunks[1]),
        Tab::Run => draw_run(frame, chunks[1]),
    }

    draw_footer(frame, app, chunks[2]);
    let status = Paragraph::new(app.status.as_str()).style(Style::default().fg(Color::Cyan));
    frame.render_widget(status, chunks[3]);
}

fn draw_run(frame: &mut Frame, area: Rect) {
    let p = Paragraph::new(vec![
        Line::from(""),
        Line::from("Run tab (U1 stub)"),
        Line::from(""),
        Line::from("Start / Stop / logs / ifstats are not implemented."),
        Line::from("Save from Config still writes ~/.hy/client.yaml."),
        Line::from(""),
        Line::from("Tab / 1 / 2  switch tabs    Esc  quit"),
    ])
    .block(Block::default().borders(Borders::ALL).title("Run"));
    frame.render_widget(p, area);
}

fn draw_footer(frame: &mut Frame, app: &App, area: Rect) {
    let btn = |id: Focus, label: &str| -> Span<'static> {
        let focused = app.tab == Tab::Config && app.focus == id;
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
    let line = Line::from(vec![
        btn(Focus::Save, "Save"),
        Span::raw("  "),
        btn(Focus::Start, "Start"),
        Span::raw("  "),
        btn(Focus::UpdateHy, "Update hy"),
        Span::raw("    Enter=Save when focused   Space=toggle   ↑↓=move"),
    ]);
    let p = Paragraph::new(line).block(Block::default().borders(Borders::ALL).title("actions"));
    frame.render_widget(p, area);
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
        Focus::Save | Focus::Start | Focus::UpdateHy => 0,
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
            "  (U1: selection is in-memory only; Save does not write route.file)",
        )),
        RouteMode::Local => {
            lines.push(text_row(
                app,
                Focus::RouteLocalPath,
                "local .conf",
                &f.route_local_path,
            ));
        }
        RouteMode::Url => {
            lines.push(text_row(
                app,
                Focus::RouteUrlValue,
                "HTTPS URL",
                &f.route_url,
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
        lines.push(dim("  empty advanced keys are omitted from yaml"));
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
