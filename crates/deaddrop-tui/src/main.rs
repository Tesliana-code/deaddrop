//! `deaddrop-tui --home <dir> [--ascii]`: a small terminal view of one node.
//!
//! Syncs and sends run on one worker thread, in the order asked, so the UI
//! stays responsive and two jobs never touch the node home at once. The
//! worker opens its own `Shell` per job, since the shell opens stores per
//! operation anyway. A background sync runs every `app::AUTO_SYNC` while idle.
//!
//! `/task::wire` (deaddrop-task): the planner runs on its own thread, once
//! per task. Environment:
//!   DEADDROP_TASK_MAY_ASK        comma-separated peers tasks may ask (local
//!                                policy; empty or unset asks no one)
//!   DEADDROP_TASK_PLANNER        planner CLI (default `claude`)
//!   DEADDROP_TASK_PLANNER_MODEL  model passed to it (optional)

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use deaddrop_protocol::NodeId;
use deaddrop_shell::Shell;
use deaddrop_tui::app::{Action, App, Drawn, Outgoing, RoomOutgoing};
use deaddrop_tui::avatar::Glyphs;
use deaddrop_tui::clipboard::{Clipboard, WindowsClipboard};
use deaddrop_tui::input::{self, Input};
use deaddrop_tui::snapshot::{self, Snapshot};
use deaddrop_tui::{timing, ui};
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{
    self, EnableBracketedPaste, EnableMouseCapture, Event, KeyboardEnhancementFlags,
    PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use ratatui::crossterm::{execute, terminal};

const FRAME: Duration = Duration::from_millis(80);

const USAGE: &str = "usage: deaddrop-tui --home <dir> [--ascii] [--open <node-id>]
  --home <dir>        node home (or DEADDROP_HOME)
  --ascii             ASCII glyphs instead of flowers and braille spinner
  --open <node-id>    start in that contact's conversation (or #room)";

enum Job {
    Sync,
    /// The outgoing message, and when Enter was pressed (for timing).
    Send(Outgoing, u128),
    /// A room message: one ordinary signed delivery per member, and when
    /// Enter was pressed (for timing).
    SendRoom(RoomOutgoing, u128),
    /// A task's room message: delivered like any, never touching the draft.
    SendTask(RoomOutgoing),
}

enum Update {
    Progress(&'static str),
    Done(Result<Snapshot, String>),
    Sent(Result<String, String>),
    Pasted(Result<Option<String>, String>),
    /// Per member: the delivery's message id, or why it failed.
    RoomSent(RoomOutgoing, Vec<(String, Result<String, String>)>),
    TaskSent(RoomOutgoing, Vec<(String, Result<String, String>)>),
    Planned(String, Result<deaddrop_task::plan::Proposed, String>),
}

fn main() -> ExitCode {
    let (home, glyphs, open) = match parse_args(std::env::args().skip(1)) {
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
    let enhanced = enhance_keyboard();
    capture_mouse();
    let result = run(terminal, &home, glyphs, open);
    let _ = input::release_mouse(&mut std::io::stdout());
    if enhanced {
        let _ = execute!(std::io::stdout(), PopKeyboardEnhancementFlags);
    }
    ratatui::restore();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("deaddrop-tui: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Ask the terminal to tell Shift+Enter from Enter, where it can (the kitty
/// keyboard protocol). Most terminals, Windows Terminal included, do not
/// support it; there both arrive as Enter, and Ctrl+J is the newline key.
/// Returns whether the flags must be popped on exit.
/// Take mouse events, for the wheel. Windows Terminal still selects text
/// with Shift+drag while an app has the mouse. Released on every exit,
/// panics included.
fn capture_mouse() {
    if execute!(std::io::stdout(), EnableMouseCapture, EnableBracketedPaste).is_ok() {
        let hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let _ = input::release_mouse(&mut std::io::stdout());
            hook(info);
        }));
    }
}

fn enhance_keyboard() -> bool {
    if !terminal::supports_keyboard_enhancement().unwrap_or(false) {
        return false;
    }
    let pushed = execute!(
        std::io::stdout(),
        PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
    )
    .is_ok();
    if pushed {
        // Undo it on a panic too, before ratatui's hook restores the rest.
        let hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let _ = execute!(std::io::stdout(), PopKeyboardEnhancementFlags);
            hook(info);
        }));
    }
    pushed
}

type Args = (PathBuf, Glyphs, Option<String>);

fn parse_args(args: impl Iterator<Item = String>) -> Result<Args, String> {
    let mut home = None;
    let mut glyphs = Glyphs::Unicode;
    let mut open = None;
    let mut args = args;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--home" => home = Some(PathBuf::from(args.next().ok_or("--home needs a value")?)),
            "--ascii" => glyphs = Glyphs::Ascii,
            "--open" => open = Some(args.next().ok_or("--open needs a node id")?),
            "-h" | "--help" => return Err(String::new()),
            other => return Err(format!("unknown argument {other:?}")),
        }
    }
    let home = home
        .or_else(|| std::env::var_os("DEADDROP_HOME").map(PathBuf::from))
        .ok_or("missing --home (or DEADDROP_HOME)")?;
    Ok((home, glyphs, open))
}

/// The one worker. Jobs run strictly one after another.
fn worker(home: PathBuf, updates: Sender<Update>) -> Sender<Job> {
    let (jobs, queue) = mpsc::channel::<Job>();
    std::thread::spawn(move || {
        for job in queue {
            let update = match job {
                Job::Sync => {
                    let _ = updates.send(Update::Progress("syncing"));
                    Update::Done(
                        Shell::open(&home)
                            .and_then(|shell| snapshot::load(&shell))
                            .map_err(|e| e.to_string()),
                    )
                }
                Job::Send(outgoing, pressed) => {
                    let result = Shell::open(&home)
                        .map_err(|e| e.to_string())
                        .and_then(|shell| snapshot::send(&shell, &outgoing));
                    if let Ok(id) = &result {
                        timing::mark_at("T0", id, pressed);
                        timing::mark("T1", id);
                    }
                    Update::Sent(result)
                }
                Job::SendTask(outgoing) => {
                    let results = fan_out(&home, &outgoing);
                    Update::TaskSent(outgoing, results)
                }
                Job::SendRoom(outgoing, pressed) => {
                    let results = match Shell::open(&home) {
                        Ok(shell) => outgoing
                            .to
                            .iter()
                            .map(|member| {
                                let sent = NodeId::parse(member.as_str())
                                    .map_err(|e| e.to_string())
                                    .and_then(|to| {
                                        shell
                                            .send(&to, &outgoing.body, None, &[])
                                            .map(|id| id.to_string())
                                            .map_err(|e| e.to_string())
                                    });
                                (member.clone(), sent)
                            })
                            .collect(),
                        Err(e) => outgoing
                            .to
                            .iter()
                            .map(|m| (m.clone(), Err(e.to_string())))
                            .collect(),
                    };
                    timing::mark_at("T0", &outgoing.id, pressed);
                    timing::mark("T1", &outgoing.id);
                    Update::RoomSent(outgoing, results)
                }
            };
            if updates.send(update).is_err() {
                return;
            }
        }
    });
    jobs
}

/// One signed delivery per member.
fn fan_out(home: &Path, outgoing: &RoomOutgoing) -> Vec<(String, Result<String, String>)> {
    match Shell::open(home) {
        Ok(shell) => outgoing
            .to
            .iter()
            .map(|member| {
                let sent = NodeId::parse(member.as_str())
                    .map_err(|e| e.to_string())
                    .and_then(|to| {
                        shell
                            .send(&to, &outgoing.body, None, &[])
                            .map(|id| id.to_string())
                            .map_err(|e| e.to_string())
                    });
                (member.clone(), sent)
            })
            .collect(),
        Err(e) => outgoing
            .to
            .iter()
            .map(|m| (m.clone(), Err(e.to_string())))
            .collect(),
    }
}

/// The task planner, from the environment.
fn planner() -> deaddrop_task::planner::ClaudePlanner {
    let var = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
    deaddrop_task::planner::ClaudePlanner {
        program: var("DEADDROP_TASK_PLANNER")
            .unwrap_or_else(|| "claude".into())
            .into(),
        model: var("DEADDROP_TASK_PLANNER_MODEL"),
        timeout: Duration::from_secs(90),
        workdir: std::env::temp_dir().join(format!("deaddrop-planner-{}", std::process::id())),
    }
}

fn send_task_messages(app: &mut App, jobs: &Sender<Job>) {
    for outgoing in app.take_task_sends() {
        let _ = jobs.send(Job::SendTask(outgoing));
    }
}

fn start_refresh(app: &mut App, jobs: &Sender<Job>) {
    if app.begin_refresh() {
        let _ = jobs.send(Job::Sync);
    }
}

fn run(
    mut terminal: DefaultTerminal,
    home: &Path,
    glyphs: Glyphs,
    open: Option<String>,
) -> std::io::Result<()> {
    let (updates, inbox): (Sender<Update>, Receiver<Update>) = mpsc::channel();
    let pasted = updates.clone();
    let jobs = worker(home.to_owned(), updates);
    let mut app = App::new();
    app.task_policy = std::env::var("DEADDROP_TASK_MAY_ASK")
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(str::to_owned)
        .collect();
    let planned = pasted.clone();
    match deaddrop_room::load_rooms(home) {
        Ok(rooms) => app.set_rooms(rooms),
        Err(error) => app.status = format!("rooms: {error}"),
    }
    // Every task journal, replayed: running tasks carry on after the first
    // sync, from exactly what was journaled.
    app.load_tasks(home);
    if let Some(peer) = open {
        app.open_on(peer);
    }
    let started = Instant::now();
    // Timing only: sent ids awaiting a reply, and replies not yet drawn.
    let mut awaiting: std::collections::HashSet<String> = Default::default();
    let mut arrived: Vec<String> = Vec::new();
    start_refresh(&mut app, &jobs);

    loop {
        for update in inbox.try_iter() {
            match update {
                Update::Progress(label) => app.progress(label),
                Update::Pasted(read) => app.pasted(read),
                Update::RoomSent(outgoing, results) => {
                    let any = results.iter().any(|(_, r)| r.is_ok());
                    app.finish_room_send(&outgoing, results);
                    if any && app.begin_refresh() {
                        app.quiet = true;
                        let _ = jobs.send(Job::Sync);
                    }
                }
                Update::TaskSent(outgoing, results) => {
                    app.finish_task_send(&outgoing, &results);
                    if results.iter().any(|(_, r)| r.is_ok()) && app.begin_refresh() {
                        app.quiet = true;
                        let _ = jobs.send(Job::Sync);
                    }
                }
                Update::Planned(id, proposed) => {
                    app.planned(&id, proposed);
                    send_task_messages(&mut app, &jobs);
                }
                Update::Done(result) => {
                    app.finish(result);
                    app.synced_at(Instant::now());
                    app.advance_tasks();
                    send_task_messages(&mut app, &jobs);
                    // Timing: a reply to something sent here has arrived.
                    if let Some(snapshot) = app.snapshot.as_ref() {
                        // Room reports answer a room request by its id.
                        for m in &snapshot.inbox {
                            if let Some(Ok(report)) = deaddrop_room::RoomMessage::decode(&m.body)
                                && let Some(asked) = report.reply_to.as_ref()
                                && awaiting.remove(asked)
                            {
                                timing::mark_by("T6", asked, &m.from);
                                arrived.push(asked.clone());
                            }
                        }
                        for m in &snapshot.inbox {
                            if let Some(asked) = m.correlation.as_ref()
                                && awaiting.remove(asked)
                            {
                                timing::mark("T6", asked);
                                arrived.push(asked.clone());
                            }
                        }
                    }
                }
                Update::Sent(result) => {
                    if let (true, Ok(id)) = (timing::enabled(), &result) {
                        awaiting.insert(id.clone());
                    }
                    let sent = result.is_ok();
                    app.finish_send(result);
                    // Show the new message; any ACK arrives on a later sync.
                    // Quietly, so "sent to …" stays on the status line.
                    if sent && app.begin_refresh() {
                        app.quiet = true;
                        let _ = jobs.send(Job::Sync);
                    }
                }
            }
        }
        if app.begin_auto_sync(Instant::now()) {
            let _ = jobs.send(Job::Sync);
        }
        let tick = (started.elapsed().as_millis() / FRAME.as_millis()) as usize;
        let mut drawn = Drawn::default();
        terminal.draw(|frame| drawn = ui::draw(frame, &app, glyphs, tick))?;
        for asked in arrived.drain(..) {
            timing::mark("T7", &asked);
        }
        // Scrolling works in the rows just drawn.
        app.observe(drawn.conversation.clone());
        app.observe_composer(drawn.composer.clone());

        if !event::poll(FRAME)? {
            continue;
        }
        let key = match event::read()? {
            Event::Key(key) => match input::translate(key) {
                Some(Input::Key(key)) => key,
                Some(Input::Quit) => return Ok(()),
                None => continue,
            },
            // Bracketed paste: pasted text arrives whole, never as keystrokes,
            // so a newline in it can never press Enter.
            Event::Paste(text) => {
                app.paste(&text);
                continue;
            }
            Event::Mouse(mouse) => match input::translate_mouse(mouse, &drawn) {
                Some(key) => key,
                None => continue,
            },
            _ => continue,
        };
        match app.key(key) {
            Action::Quit => return Ok(()),
            Action::Refresh => start_refresh(&mut app, &jobs),
            // Off the UI thread: the bridge takes a few hundred ms.
            Action::ReadClipboard => {
                let updates = pasted.clone();
                std::thread::spawn(move || {
                    let read = WindowsClipboard::default().read_text();
                    let _ = updates.send(Update::Pasted(read));
                });
            }
            Action::Copy(text) => {
                use std::io::Write;
                let mut out = std::io::stdout();
                let _ = write!(out, "{}", input::osc52(&text));
                let _ = out.flush();
            }
            Action::SendRoom(outgoing) => {
                if timing::enabled() {
                    awaiting.insert(outgoing.id.clone());
                }
                let _ = jobs.send(Job::SendRoom(outgoing, timing::now_us()));
            }
            Action::Send(outgoing) => {
                let _ = jobs.send(Job::Send(outgoing, timing::now_us()));
            }
            // Off the UI thread: one bounded model call.
            Action::PlanTask(request) => {
                let updates = planned.clone();
                std::thread::spawn(move || {
                    use deaddrop_task::planner::Planner;
                    let proposed = planner().propose(&request.payload, request.memory.as_deref());
                    let _ = updates.send(Update::Planned(request.id, proposed));
                });
            }
            Action::None => {}
        }
    }
}
