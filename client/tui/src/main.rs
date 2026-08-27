//! hy-tui: Config form + Save to ~/.hy/client.yaml (U1).
//! Start / Update hy / Run tab are visible no-ops.

mod config_gen;
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

#[tokio::main]
async fn main() -> Result<()> {
    let mut terminal = setup()?;
    let _guard = TerminalGuard;
    let mut app = App::new();

    loop {
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
            Action::None => {}
        }
    }
    Ok(())
}
