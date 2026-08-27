//! hy-tui: Config form + Save (U1) + download hy to ~/.hy/bin/hy (U2).
//! Start / Run tab remain visible no-ops until later units.

mod config_gen;
mod detect;
mod fetch_hy;
mod ui;

use std::io::{self, stdout};
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use ui::{Action, App};

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

fn start_fetch() -> FetchJob {
    tokio::task::spawn_blocking(|| {
        let home = config_gen::home_dir().map_err(|e| e.to_string())?;
        fetch_hy::install_latest(&home).map_err(|e| e.to_string())
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

#[tokio::main]
async fn main() -> Result<()> {
    let mut terminal = setup()?;
    let _guard = TerminalGuard;
    let mut app = App::new();
    let mut fetch_job: Option<FetchJob> = None;

    if let Ok(home) = config_gen::home_dir() {
        if !fetch_hy::default_hy_bin(&home).is_file() {
            ui::note_downloading(&mut app);
            fetch_job = Some(start_fetch());
        }
    }

    loop {
        take_finished(&mut fetch_job, &mut app).await;
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
            Action::Quit => break,
            Action::Save => match config_gen::save_to_home(&app.form).await {
                Ok(path) => ui::note_save_ok(&mut app, &path.display().to_string()),
                Err(e) => ui::note_save_err(&mut app, &e.to_string()),
            },
            Action::UpdateHy => {
                if fetch_job.is_some() {
                    ui::note_downloading(&mut app);
                } else {
                    ui::note_downloading(&mut app);
                    fetch_job = Some(start_fetch());
                }
            }
            Action::None => {}
        }
    }
    Ok(())
}
