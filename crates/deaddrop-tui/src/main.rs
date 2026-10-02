//! `deaddrop-tui --home <dir> [--ascii]`: a small terminal view of one node.
//!
//! Refreshes run on a worker thread so the spinner keeps moving; the worker
//! opens its own `Shell` per refresh, since the shell opens stores per
//! operation anyway.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use deaddrop_shell::Shell;
use deaddrop_tui::app::{Action, App, Key};
use deaddrop_tui::avatar::Glyphs;
use deaddrop_tui::snapshot::{self, Snapshot};
use deaddrop_tui::ui;
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind};

const FRAME: Duration = Duration::from_millis(80);

const USAGE: &str = "usage: deaddrop-tui --home <dir> [--ascii]
  --home <dir>  node home (or DEADDROP_HOME)
  --ascii       ASCII glyphs instead of flowers and braille spinner";

enum Update {
    Progress(&'static str),
    Done(Result<Snapshot, String>),
}

fn main() -> ExitCode {
    let (home, glyphs) = match parse_args(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(message) => {
            eprintln!("{message}\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    // Fail before touching the terminal if the home cannot be opened.
    if let Err(error) = Shell::open(&home) {
        eprintln!("deaddrop-tui: {}: {error}", home.display());
        return ExitCode::FAILURE;
    }

    // `ratatui::init` enters raw mode and the alternate screen and installs a
    // panic hook that restores the terminal; `restore` runs on every exit.
    let terminal = ratatui::init();
    let result = run(terminal, &home, glyphs);
    ratatui::restore();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("deaddrop-tui: {error}");
            ExitCode::FAILURE
        }
    }
}

fn parse_args(args: impl Iterator<Item = String>) -> Result<(PathBuf, Glyphs), String> {
    let mut home = None;
    let mut glyphs = Glyphs::Unicode;
    let mut args = args;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--home" => home = Some(PathBuf::from(args.next().ok_or("--home needs a value")?)),
            "--ascii" => glyphs = Glyphs::Ascii,
            "-h" | "--help" => return Err(String::new()),
            other => return Err(format!("unknown argument {other:?}")),
        }
    }
    let home = home
        .or_else(|| std::env::var_os("DEADDROP_HOME").map(PathBuf::from))
        .ok_or("missing --home (or DEADDROP_HOME)")?;
    Ok((home, glyphs))
}

fn refresh(home: PathBuf, updates: Sender<Update>) {
    std::thread::spawn(move || {
        let _ = updates.send(Update::Progress("syncing"));
        let result = Shell::open(&home)
            .and_then(|shell| snapshot::load(&shell))
            .map_err(|e| e.to_string());
        let _ = updates.send(Update::Done(result));
    });
}

fn start_refresh(app: &mut App, home: &Path, updates: &Sender<Update>) {
    if app.begin_refresh() {
        refresh(home.to_owned(), updates.clone());
    }
}

fn run(mut terminal: DefaultTerminal, home: &Path, glyphs: Glyphs) -> std::io::Result<()> {
    let (updates, inbox): (Sender<Update>, Receiver<Update>) = mpsc::channel();
    let mut app = App::new();
    let started = Instant::now();
    start_refresh(&mut app, home, &updates);

    loop {
        for update in inbox.try_iter() {
            match update {
                Update::Progress(label) => app.progress(label),
                Update::Done(result) => app.finish(result),
            }
        }
        let tick = (started.elapsed().as_millis() / FRAME.as_millis()) as usize;
        terminal.draw(|frame| ui::draw(frame, &app, glyphs, tick))?;

        if !event::poll(FRAME)? {
            continue;
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        let key = match key.code {
            KeyCode::Up => Key::Up,
            KeyCode::Down => Key::Down,
            KeyCode::Enter => Key::Enter,
            KeyCode::Esc => Key::Esc,
            KeyCode::Tab | KeyCode::BackTab | KeyCode::Left | KeyCode::Right => Key::Tab,
            KeyCode::Char(c) => Key::Char(c),
            _ => continue,
        };
        match app.key(key) {
            Action::Quit => return Ok(()),
            Action::Refresh => start_refresh(&mut app, home, &updates),
            Action::None => {}
        }
    }
}
