//! Unstable Odds — a ridiculous terminal horse-racing simulator.
//!
//! Layout:
//! - [`sim`]: pure, deterministic race simulation (no rendering, no I/O).
//! - [`card`]: builds a race card (horses, attributes, odds) from a seed.
//! - [`betting`]: money types, odds, payout maths and wager parsing.
//! - [`horses`]: default names, personality text and name validation.
//! - [`commentary`]: turns simulation events into dramatic commentary.
//! - [`save`]: JSON persistence of balance, names and stats.
//! - [`app`]: screen state machine and key handling (no terminal access).
//! - [`ui`]: Ratatui rendering of the app state.
//! - [`tui`]: terminal setup/teardown and the event loop.
//! - [`cli`]: command-line argument parsing.

pub mod app;
pub mod betting;
pub mod card;
pub mod cli;
pub mod commentary;
pub mod horses;
pub mod save;
pub mod sim;
pub mod tui;
pub mod ui;
