mod backup;

use crate::backup::*;
use crossterm::event::Event;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use std::io;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use thiserror::Error;
use tui::{
    backend::{Backend, CrosstermBackend},
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Span, Spans},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame, Terminal,
};

const BROGUE_SAVE_DIR: &str = "Library/Application Support/Brogue/Brogue CE";
const LOCAL_BACKUP_DIR: &str = ".brogue";

type Result<T> = std::result::Result<T, AppError>;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("no home dir")]
    NoHomeDir,
    #[error("missing save dir: {0}")]
    MissingDir(PathBuf),
    #[error("notify error")]
    NotifyError(#[from] notify::Error),
    #[error("IO error")]
    IoError(#[from] std::io::Error),
    #[error("unknown error")]
    Unknown,
}

// Basic logic:
// ====
// There is a save dir. New files appear (e.g. 'Saved #272472511 at depth 1 (easy).broguesave')
// Each save file should be moved out to a backup folder
// When it disappears from the save dir, but exists in the backup dir, copy it over
#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<()> {
    //setup_logger().expect("Could not set up logger");

    let user_home = dirs::home_dir().ok_or(AppError::NoHomeDir)?;
    let save_dir = user_home.join(BROGUE_SAVE_DIR);
    let backup_dir = user_home.join(LOCAL_BACKUP_DIR);

    if !backup_dir.exists() {
        std::fs::create_dir_all(&backup_dir)?;
    }

    // setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let app = App::new(save_dir, backup_dir);

    run_app(&mut terminal, app, Duration::from_millis(250))?;

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    Ok(())
}

fn run_app<B: Backend>(
    terminal: &mut Terminal<B>,
    mut app: App,
    tick_rate: Duration,
) -> Result<()> {
    let mut last_tick = Instant::now();

    loop {
        app.update_state()?;

        terminal.draw(|f| ui(f, &app))?;

        let timeout = tick_rate
            .checked_sub(last_tick.elapsed())
            .unwrap_or_else(|| Duration::from_secs(0));
        if crossterm::event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('q') => {
                        return Ok(());
                    }
                    KeyCode::Esc => {
                        app.delete_state = DeleteState::NotDeleting;
                    }
                    KeyCode::Char('d') => {
                        app.delete_state = DeleteState::AwaitingIndex;
                    }
                    KeyCode::Char(c) => {
                        if app.delete_state == DeleteState::AwaitingIndex && c.is_ascii_alphabetic()
                        {
                            let idx = ((c as u8) - b'a') as usize;
                            app.delete_state = DeleteState::Delete(idx);
                        }
                    }
                    _ => {}
                }
            }
        }

        if last_tick.elapsed() >= tick_rate {
            app.on_tick();
            last_tick = Instant::now();
        }
    }
}

fn letter(idx: usize) -> char {
    (b'a' + idx as u8) as char
}

fn ui<B: Backend>(f: &mut Frame<B>, app: &App) {
    let size = f.size();

    let block = Block::default().style(Style::default().bg(Color::White).fg(Color::Black));
    f.render_widget(block, size);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(2)
        .constraints([Constraint::Percentage(75), Constraint::Percentage(25)].as_ref())
        .split(size);

    let delete_state_description: String = match &app.delete_state {
        DeleteState::NotDeleting => "press 'd' to delete a save game".to_string(),
        DeleteState::AwaitingIndex => {
            "press a number to choose a game to delete, or ESC to cancel".to_string()
        }
        DeleteState::Delete(idx) => format!("deleting {}", idx),
    };

    let state_descrition = vec![
        Spans::from(delete_state_description),
        Spans::from("press 'q' to quit"),
    ];

    let file_spans: Vec<_> = app
        .state
        .saves
        .iter()
        .enumerate()
        .map(|(idx, s)| Spans::from(Span::raw(format!("{}) {}", letter(idx), s.to_string()))))
        .collect();

    let create_block = |title| {
        Block::default()
            .borders(Borders::ALL)
            .style(Style::default().bg(Color::White).fg(Color::Black))
            .title(Span::styled(
                title,
                Style::default().add_modifier(Modifier::BOLD),
            ))
    };

    let paragraph = Paragraph::new(file_spans.clone())
        .style(Style::default().bg(Color::White).fg(Color::Black))
        .block(create_block("Saves"))
        .alignment(Alignment::Left);
    f.render_widget(paragraph, chunks[0]);
    let paragraph = Paragraph::new(state_descrition.clone())
        .style(Style::default().bg(Color::White).fg(Color::Black))
        .block(create_block("Left, wrap"))
        .alignment(Alignment::Left)
        .wrap(Wrap { trim: true });
    f.render_widget(paragraph, chunks[1]);
}
