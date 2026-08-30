//! hy-tui: Config form + Save (U1) + download hy (U2) + rules URL / --route (U3)
//! + sudo Start / SIGINT Stop (U4) + Run logs / TUN ifstats (U5).

mod config_gen;
mod detect;
mod fetch_hy;
mod fetch_route;
mod ifstats;
mod logbuf;
mod spawn;
mod sudo;
mod ui;

use std::io::{self, stdout};
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{self, Event, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use ui::{Action, App};
use zeroize::Zeroizing;

struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(stdout(), LeaveAlternateScreen);
    }
}

fn setup() -> Result<Terminal<CrosstermBackend<io::Stdout>>> {
    enable_raw_mode()?;
    execute!(stdout(), EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout());
    Ok(Terminal::new(backend)?)
}

type FetchJob = tokio::task::JoinHandle<std::result::Result<fetch_hy::InstallResult, String>>;
type PrepareJob = tokio::task::JoinHandle<std::result::Result<sudo::LaunchPlan, String>>;
type SpawnJob = tokio::task::JoinHandle<std::result::Result<sudo::HyProcess, String>>;
type StopJob = tokio::task::JoinHandle<StopJobResult>;

enum StopJobResult {
    Exited,
    NeedsPassword(sudo::HyProcess),
    PasswordRejected(sudo::HyProcess),
    Failed {
        err: String,
        process: Option<sudo::HyProcess>,
    },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PasswordKind {
    Start,
    Stop,
}

fn start_fetch() -> FetchJob {
    tokio::task::spawn_blocking(|| {
        let home = config_gen::home_dir().map_err(|e| e.to_string())?;
        fetch_hy::install_latest(&home).map_err(|e| e.to_string())
    })
}

fn start_prepare(form: config_gen::FormState) -> PrepareJob {
    tokio::task::spawn_blocking(move || {
        let home = config_gen::home_dir().map_err(|e| e.to_string())?;
        sudo::prepare_launch(&home, &form).map_err(|e| e.to_string())
    })
}

fn start_spawn(plan: sudo::LaunchPlan, password: Option<Zeroizing<String>>) -> SpawnJob {
    tokio::task::spawn_blocking(move || {
        sudo::spawn_hy(&plan, password, sudo::SpawnOpts::default()).map_err(|e| e.to_string())
    })
}

async fn take_finished(job: &mut Option<FetchJob>, app: &mut App) {
    let finished = job.as_ref().is_some_and(|h| h.is_finished());
    if !finished {
        return;
    }
    let Some(handle) = job.take() else {
        return;
    };
    match handle.await {
        Ok(Ok(result)) => ui::note_fetch_ok(app, &result.tag),
        Ok(Err(e)) => ui::note_fetch_err(app, &e),
        Err(e) => ui::note_fetch_err(app, &format!("download task failed: {e}")),
    }
}

struct Runtime {
    session: sudo::StartSession,
    pending_plan: Option<sudo::LaunchPlan>,
    spawn_used_password: bool,
    restart_after_stop: bool,
    password_kind: PasswordKind,
    fetch_job: Option<FetchJob>,
    prepare_job: Option<PrepareJob>,
    spawn_job: Option<SpawnJob>,
    stop_job: Option<StopJob>,
}

impl Runtime {
    fn busy(&self) -> bool {
        self.prepare_job.is_some() || self.spawn_job.is_some() || self.stop_job.is_some()
    }

    async fn pump(&mut self, app: &mut App) {
        take_finished(&mut self.fetch_job, app).await;
        self.take_prepare(app).await;
        self.take_spawn(app).await;
        self.take_stop(app).await;
        self.poll_logs_and_child(app);
        self.sample_ifstats(app);
    }

    async fn take_prepare(&mut self, app: &mut App) {
        let finished = self.prepare_job.as_ref().is_some_and(|h| h.is_finished());
        if !finished {
            return;
        }
        let Some(handle) = self.prepare_job.take() else {
            return;
        };
        match handle.await {
            Ok(Ok(plan)) => self.on_plan(app, plan),
            Ok(Err(e)) => ui::note_start_err(app, &e),
            Err(e) => ui::note_start_err(app, &format!("start task failed: {e}")),
        }
    }

    fn on_plan(&mut self, app: &mut App, plan: sudo::LaunchPlan) {
        if plan.needs_password {
            if !sudo::has_tty() {
                ui::note_no_tty(app);
                self.pending_plan = None;
                return;
            }
            self.pending_plan = Some(plan);
            self.password_kind = PasswordKind::Start;
            ui::note_need_password(app, self.session.password_failures);
            return;
        }
        ui::note_starting(app);
        self.spawn_used_password = false;
        self.pending_plan = Some(plan.clone());
        self.spawn_job = Some(start_spawn(plan, None));
    }

    async fn take_spawn(&mut self, app: &mut App) {
        let finished = self.spawn_job.as_ref().is_some_and(|h| h.is_finished());
        if !finished {
            return;
        }
        let Some(handle) = self.spawn_job.take() else {
            return;
        };
        let used_password = self.spawn_used_password;
        match handle.await {
            Ok(Ok(proc)) => {
                let pid = proc.hy_pid;
                let warn = self
                    .pending_plan
                    .as_ref()
                    .map(|p| p.ruleset_warning)
                    .unwrap_or(false);
                self.pending_plan = None;
                self.session.record_child(proc);
                ui::note_spawned(app, pid, warn);
            }
            Ok(Err(e)) => self.on_spawn_err(app, used_password, &e),
            Err(e) => self.on_spawn_err(app, used_password, &format!("spawn task failed: {e}")),
        }
    }

    fn on_spawn_err(&mut self, app: &mut App, used_password: bool, err: &str) {
        if used_password {
            self.session.note_password_failure();
            let locked = self.session.password_locked_out();
            if locked {
                self.pending_plan = None;
            }
            ui::note_password_fail(app, self.session.password_failures, locked);
            if locked {
                // Stay on Config; do not leave a hy process. Keep the 3-fail status.
                let _ = err;
            }
        } else {
            self.pending_plan = None;
            ui::note_start_err(app, err);
        }
    }

    async fn take_stop(&mut self, app: &mut App) {
        let finished = self.stop_job.as_ref().is_some_and(|h| h.is_finished());
        if !finished {
            return;
        }
        let Some(handle) = self.stop_job.take() else {
            return;
        };
        match handle.await {
            Ok(StopJobResult::Exited) => {
                ui::append_logs(app, self.session.poll_logs());
                ui::note_stopped(app);
                let restart = self.restart_after_stop;
                self.restart_after_stop = false;
                if restart {
                    begin_start(self, app).await;
                }
            }
            Ok(StopJobResult::NeedsPassword(proc)) => {
                self.session.process = Some(proc);
                ui::append_logs(app, self.session.poll_logs());
                ui::restore_run_if_alive(app);
                if !sudo::has_tty() {
                    ui::note_stop_no_tty(app);
                    self.restart_after_stop = false;
                    return;
                }
                self.session.password_failures = 0;
                self.password_kind = PasswordKind::Stop;
                ui::note_need_password(app, 0);
                ui::restore_run_if_alive(app);
            }
            Ok(StopJobResult::PasswordRejected(proc)) => {
                self.session.process = Some(proc);
                ui::append_logs(app, self.session.poll_logs());
                self.session.note_stop_password_failure();
                let locked = self.session.password_locked_out();
                if locked {
                    self.restart_after_stop = false;
                }
                self.password_kind = PasswordKind::Stop;
                ui::note_stop_password_fail(app, self.session.password_failures, locked);
            }
            Ok(StopJobResult::Failed { err, process }) => {
                if let Some(proc) = process {
                    self.session.process = Some(proc);
                    ui::restore_run_if_alive(app);
                }
                ui::append_logs(app, self.session.poll_logs());
                self.restart_after_stop = false;
                ui::note_stop_err(app, &err);
            }
            Err(e) => {
                self.restart_after_stop = false;
                ui::note_stop_err(app, &format!("stop task failed: {e}"));
            }
        }
    }

    fn poll_logs_and_child(&mut self, app: &mut App) {
        ui::append_logs(app, self.session.poll_logs());
        if self.stop_job.is_some() || self.spawn_job.is_some() {
            return;
        }
        let dead = self.session.process.as_mut().is_some_and(|p| !p.is_alive());
        if dead {
            ui::append_logs(app, self.session.poll_logs());
            self.session.process = None;
            ui::note_hy_exited(app);
        }
    }

    fn sample_ifstats(&mut self, app: &mut App) {
        let watching = self.session.has_child()
            && matches!(
                app.run_phase,
                ui::RunPhase::Starting | ui::RunPhase::Running | ui::RunPhase::Stopping
            );
        if !watching {
            return;
        }
        let now = Instant::now();
        if !app.ifstats.due(now, ifstats::SAMPLE_INTERVAL) {
            return;
        }
        let name = app.form.tun_name.trim();
        app.ifstats.sample(now, ifstats::read_iface(name));
    }

    fn request_stop(
        &mut self,
        app: &mut App,
        then_start: bool,
        password: Option<Zeroizing<String>>,
    ) {
        if self.stop_job.is_some() {
            ui::note_stopping(app);
            self.restart_after_stop = then_start || self.restart_after_stop;
            return;
        }
        if !self.session.has_child() {
            ui::note_stopped(app);
            return;
        }
        ui::note_stopping(app);
        self.restart_after_stop = then_start;
        let mut session_proc = self.session.process.take();
        self.stop_job = Some(tokio::task::spawn_blocking(move || {
            let Some(mut proc) = session_proc.take() else {
                return StopJobResult::Exited;
            };
            match proc.stop(sudo::STOP_WAIT, password) {
                Ok(sudo::StopResult::Exited) => StopJobResult::Exited,
                Ok(sudo::StopResult::NeedsPassword) => StopJobResult::NeedsPassword(proc),
                Ok(sudo::StopResult::PasswordRejected) => StopJobResult::PasswordRejected(proc),
                Err(e) => {
                    let alive = proc.is_alive();
                    StopJobResult::Failed {
                        err: e.to_string(),
                        process: if alive { Some(proc) } else { None },
                    }
                }
            }
        }));
    }
}

async fn begin_start(rt: &mut Runtime, app: &mut App) {
    if rt.session.has_child() {
        app.set_status("already running; Stop first", true);
        return;
    }
    if rt.busy() || app.password_prompt.is_some() {
        ui::note_starting(app);
        return;
    }
    match config_gen::save_to_home(&app.form).await {
        Ok(_) => {}
        Err(e) => {
            ui::note_save_err(app, &e.to_string());
            return;
        }
    }
    rt.session.begin_start();
    rt.pending_plan = None;
    rt.spawn_used_password = false;
    rt.password_kind = PasswordKind::Start;
    ui::note_starting(app);
    rt.prepare_job = Some(start_prepare(app.form.clone()));
}

fn submit_password(rt: &mut Runtime, app: &mut App) {
    let Some(prompt) = app.take_password_prompt() else {
        return;
    };
    match rt.password_kind {
        PasswordKind::Start => submit_start_password(rt, app, prompt),
        PasswordKind::Stop => submit_stop_password(rt, app, prompt),
    }
}

fn submit_start_password(rt: &mut Runtime, app: &mut App, prompt: ui::PasswordPrompt) {
    let Some(plan) = rt.pending_plan.clone() else {
        ui::note_start_err(app, "no pending start");
        return;
    };
    if rt.session.password_locked_out() {
        ui::note_password_fail(app, rt.session.password_failures, true);
        rt.pending_plan = None;
        return;
    }
    ui::note_starting(app);
    rt.spawn_used_password = true;
    rt.spawn_job = Some(start_spawn(plan, Some(prompt.take_buf())));
}

fn submit_stop_password(rt: &mut Runtime, app: &mut App, prompt: ui::PasswordPrompt) {
    if rt.session.password_locked_out() {
        ui::note_stop_password_fail(app, rt.session.password_failures, true);
        return;
    }
    if !rt.session.has_child() {
        ui::note_stop_err(app, "no running hy");
        return;
    }
    let restart = rt.restart_after_stop;
    rt.request_stop(app, restart, Some(prompt.take_buf()));
}

#[tokio::main]
async fn main() -> Result<()> {
    let mut terminal = setup()?;
    let _guard = TerminalGuard;
    let mut app = App::new();
    let mut rt = Runtime {
        session: sudo::StartSession::new(),
        pending_plan: None,
        spawn_used_password: false,
        restart_after_stop: false,
        password_kind: PasswordKind::Start,
        fetch_job: None,
        prepare_job: None,
        spawn_job: None,
        stop_job: None,
    };

    if let Ok(home) = config_gen::home_dir() {
        if !fetch_hy::default_hy_bin(&home).is_file() {
            ui::note_downloading(&mut app);
            rt.fetch_job = Some(start_fetch());
        }
    }

    loop {
        rt.pump(&mut app).await;
        terminal.draw(|frame| ui::draw(frame, &app))?;
        if !event::poll(Duration::from_millis(200))? {
            continue;
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        match ui::handle_key(&mut app, key) {
            Action::Quit => {
                if rt.session.quit() {
                    ui::note_quit_hy_maybe_running(&mut app);
                }
                break;
            }
            Action::Save => match config_gen::save_to_home(&app.form).await {
                Ok(path) => ui::note_save_ok(&mut app, &path.display().to_string()),
                Err(e) => ui::note_save_err(&mut app, &e.to_string()),
            },
            Action::UpdateHy => {
                if rt.fetch_job.is_some() {
                    ui::note_downloading(&mut app);
                } else {
                    ui::note_downloading(&mut app);
                    rt.fetch_job = Some(start_fetch());
                }
            }
            Action::Start => begin_start(&mut rt, &mut app).await,
            Action::Stop => rt.request_stop(&mut app, false, None),
            Action::Restart => {
                if rt.session.has_child() || rt.stop_job.is_some() {
                    rt.request_stop(&mut app, true, None);
                } else {
                    begin_start(&mut rt, &mut app).await;
                }
            }
            Action::PasswordSubmit => submit_password(&mut rt, &mut app),
            Action::PasswordCancel => match rt.password_kind {
                PasswordKind::Start => {
                    rt.pending_plan = None;
                    rt.session.cancel();
                    ui::note_password_cancel(&mut app);
                }
                PasswordKind::Stop => {
                    ui::note_stop_password_cancel(&mut app);
                    rt.restart_after_stop = false;
                }
            },
            Action::ClearLog => {
                app.log.clear();
            }
            Action::None => {}
        }
    }
    Ok(())
}
