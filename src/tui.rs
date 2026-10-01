//! Terminal setup, teardown and the main event loop.
//!
//! The terminal is restored on every exit path: normal return, `?` errors
//! (via the [`TerminalGuard`] drop) and panics (via a panic hook that restores
//! the terminal before the default hook prints the message).

use std::io::{self, Stdout};
use std::panic;
use std::sync::Once;
use std::time::Instant;

use crossterm::cursor::{Hide, Show};
use crossterm::event::{self, Event, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use crate::app::{App, TICK_RATE};
use crate::ui;

type Term = Terminal<CrosstermBackend<Stdout>>;

/// Best-effort terminal restore. Safe to call more than once.
pub fn restore_terminal() {
    let _ = disable_raw_mode();
    let _ = execute!(io::stdout(), LeaveAlternateScreen, Show);
}

/// Owns the "terminal is in game mode" state and undoes it on drop.
pub struct TerminalGuard {
    terminal: Term,
}

impl TerminalGuard {
    pub fn new() -> io::Result<Self> {
        install_panic_hook();
        enable_raw_mode()?;
        // If anything after raw mode fails, put the terminal back before bailing.
        let setup = (|| -> io::Result<Term> {
            // Clear via crossterm rather than `Terminal::clear`, which queries the cursor
            // position and fails on terminals that never answer that query.
            execute!(
                io::stdout(),
                EnterAlternateScreen,
                Hide,
                Clear(ClearType::All)
            )?;
            Terminal::new(CrosstermBackend::new(io::stdout()))
        })();
        match setup {
            Ok(terminal) => Ok(Self { terminal }),
            Err(e) => {
                restore_terminal();
                Err(e)
            }
        }
    }

    pub fn terminal(&mut self) -> &mut Term {
        &mut self.terminal
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        restore_terminal();
    }
}

fn install_panic_hook() {
    static HOOK: Once = Once::new();
    HOOK.call_once(|| {
        let previous = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            restore_terminal();
            previous(info);
        }));
    });
}

/// Runs the game until the player quits. The terminal is restored before returning.
pub fn run(app: &mut App) -> io::Result<()> {
    let mut guard = TerminalGuard::new()?;
    event_loop(guard.terminal(), app)
    // `guard` drops here (or during unwinding) and restores the terminal.
}

fn event_loop(terminal: &mut Term, app: &mut App) -> io::Result<()> {
    let mut next_tick = Instant::now() + TICK_RATE;
    let mut dirty = true;

    while !app.should_quit {
        if dirty {
            terminal.draw(|f| ui::draw(f, app))?;
            dirty = false;
        }

        // Wait for input, but never past the next animation tick.
        let timeout = next_tick.saturating_duration_since(Instant::now());
        if event::poll(timeout)? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    app.on_key(key);
                    dirty = true;
                }
                Event::Resize(_, _) => {
                    terminal.autoresize()?;
                    dirty = true;
                }
                _ => {}
            }
        }

        let now = Instant::now();
        if now >= next_tick {
            app.on_tick();
            dirty = true;
            next_tick += TICK_RATE;
            // If we fell badly behind (suspended, slow terminal), don't try to
            // replay a burst of ticks; just resynchronise.
            if now > next_tick + TICK_RATE * 4 {
                next_tick = now + TICK_RATE;
            }
        }
    }
    Ok(())
}
