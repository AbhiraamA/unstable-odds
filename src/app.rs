//! Application state and input handling. No terminal I/O and no drawing happens here.

use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::betting::{
    format_money, format_plain, gross_return, parse_wager, Cents, Odds, DEFAULT_WAGER,
};
use crate::card::RaceCard;
use crate::commentary::{self, CommentaryLine};
use crate::horses::{validate_name, DEFAULT_NAMES};
use crate::save::{LoadOutcome, SaveData, SaveStore};
use crate::sim::{run_race, RaceRecord, LANES};

/// Animation and simulation tick.
pub const TICK_RATE: Duration = Duration::from_millis(50);
/// Frames advanced per tick while fast-forwarding.
pub const FAST_FORWARD: usize = 4;
/// Pause on the finished race before switching to results (in ticks).
pub const FINISH_HOLD_TICKS: u32 = 36;
const MAX_WAGER_INPUT: usize = 14;

pub const MENU_ITEMS: [&str; 4] = ["Race", "Rename Horses", "Stats", "Quit"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoticeKind {
    Info,
    Warning,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    pub text: String,
    pub kind: NoticeKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bet {
    pub lane: usize,
    pub stake: Cents,
    pub odds: Odds,
}

/// How a bet was settled. Recorded when the gates open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BetOutcome {
    pub place: usize,
    pub dead_heat_ways: usize,
    pub returned: Cents,
    pub balance_before: Cents,
    pub balance_after: Cents,
}

impl BetOutcome {
    pub fn won(&self) -> bool {
        self.place == 1
    }
}

#[derive(Clone, Debug)]
pub struct BettingState {
    pub card: RaceCard,
    pub selected: usize,
    pub input: String,
    pub error: Option<String>,
}

#[derive(Clone, Debug)]
pub struct RacingState {
    pub card: RaceCard,
    pub record: RaceRecord,
    pub commentary: Vec<CommentaryLine>,
    pub bet: Bet,
    pub outcome: BetOutcome,
    pub frame: usize,
    pub fast: bool,
    pub hold: u32,
}

impl RacingState {
    pub fn finished(&self) -> bool {
        self.frame + 1 >= self.record.frames.len()
    }
}

#[derive(Clone, Debug)]
pub struct ResultsState {
    pub card: RaceCard,
    pub record: RaceRecord,
    pub bet: Bet,
    pub outcome: BetOutcome,
}

#[derive(Clone, Debug, Default)]
pub struct RenameState {
    pub selected: usize,
    pub editing: Option<String>,
    pub error: Option<String>,
}

#[derive(Clone, Debug)]
pub enum Screen {
    Title,
    Menu,
    Betting(BettingState),
    Racing(Box<RacingState>),
    Results(Box<ResultsState>),
    Rename(RenameState),
    Stats,
    Broke,
}

/// Produces race seeds. With `--seed N` the first race uses exactly `N`, then `N+1`, ...
#[derive(Clone, Debug)]
pub struct SeedSequence {
    next: u64,
}

impl SeedSequence {
    pub fn starting_at(seed: u64) -> Self {
        Self { next: seed }
    }

    /// Random starting point; kept to 32 bits so displayed seeds are easy to retype.
    pub fn random() -> Self {
        Self {
            next: fastrand::u64(..u64::from(u32::MAX)),
        }
    }

    pub fn next_seed(&mut self) -> u64 {
        let seed = self.next;
        self.next = self.next.wrapping_add(1);
        seed
    }
}

pub struct App {
    pub data: SaveData,
    store: SaveStore,
    seeds: SeedSequence,
    pub screen: Screen,
    pub menu_index: usize,
    /// Free-running animation counter (ticks).
    pub anim: u64,
    pub notice: Option<Notice>,
    pub should_quit: bool,
    last_bet: Option<(usize, Cents)>,
    /// A card the player looked at but backed out of; reused so browsing the
    /// menu doesn't reroll the field (and seeded runs stay reproducible).
    pending_card: Option<RaceCard>,
}

impl App {
    pub fn new(store: SaveStore, seed: Option<u64>) -> Self {
        let (data, outcome) = store.load();
        let notice = match outcome {
            LoadOutcome::New => Some(Notice {
                text: format!(
                    "Welcome! Here's {} in fake money. Spend it unwisely.",
                    format_money(data.balance_cents)
                ),
                kind: NoticeKind::Info,
            }),
            LoadOutcome::Loaded => Some(Notice {
                text: format!("Welcome back. Wallet: {}", format_money(data.balance_cents)),
                kind: NoticeKind::Info,
            }),
            LoadOutcome::Recovered { backup, reason } => Some(Notice {
                text: match backup {
                    Some(b) => format!(
                        "Save file was unreadable ({reason}). Moved it to {} and started fresh.",
                        b.display()
                    ),
                    None => format!("Save file was unreadable ({reason}). Started fresh."),
                },
                kind: NoticeKind::Warning,
            }),
        };
        Self {
            data,
            store,
            seeds: seed.map_or_else(SeedSequence::random, SeedSequence::starting_at),
            screen: Screen::Title,
            menu_index: 0,
            anim: 0,
            notice,
            should_quit: false,
            last_bet: None,
            pending_card: None,
        }
    }

    pub fn save_path(&self) -> &std::path::Path {
        self.store.path()
    }

    fn persist(&mut self) {
        if let Err(e) = self.store.save(&self.data) {
            self.notice = Some(Notice {
                text: format!("Couldn't save to {}: {e}", self.store.path().display()),
                kind: NoticeKind::Warning,
            });
        }
    }

    // ----- ticking -------------------------------------------------------------------------

    pub fn on_tick(&mut self) {
        self.anim = self.anim.wrapping_add(1);
        let mut done = false;
        if let Screen::Racing(race) = &mut self.screen {
            let last = race.record.frames.len() - 1;
            if race.frame < last {
                let step = if race.fast { FAST_FORWARD } else { 1 };
                race.frame = (race.frame + step).min(last);
            } else {
                race.hold += 1;
                done = race.hold >= FINISH_HOLD_TICKS;
            }
        }
        if done {
            self.show_results();
        }
    }

    // ----- input ---------------------------------------------------------------------------

    pub fn on_key(&mut self, key: KeyEvent) {
        if key.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(key.code, KeyCode::Char('c') | KeyCode::Char('C'))
        {
            self.should_quit = true;
            return;
        }
        match &self.screen {
            Screen::Title => self.key_title(key),
            Screen::Menu => self.key_menu(key),
            Screen::Betting(_) => self.key_betting(key),
            Screen::Racing(_) => self.key_racing(key),
            Screen::Results(_) => self.key_results(key),
            Screen::Rename(_) => self.key_rename(key),
            Screen::Stats => self.key_stats(key),
            Screen::Broke => self.key_broke(key),
        }
    }

    fn go_menu(&mut self) {
        self.screen = Screen::Menu;
    }

    fn key_title(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => self.should_quit = true,
            _ => self.go_menu(),
        }
    }

    fn key_menu(&mut self, key: KeyEvent) {
        let choice = match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.menu_index = (self.menu_index + MENU_ITEMS.len() - 1) % MENU_ITEMS.len();
                None
            }
            KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => {
                self.menu_index = (self.menu_index + 1) % MENU_ITEMS.len();
                None
            }
            KeyCode::Enter | KeyCode::Char(' ') => Some(self.menu_index),
            KeyCode::Char('r') | KeyCode::Char('R') => Some(0),
            KeyCode::Char('n') | KeyCode::Char('N') => Some(1),
            KeyCode::Char('s') | KeyCode::Char('S') => Some(2),
            KeyCode::Char('q') | KeyCode::Char('Q') => Some(3),
            _ => None,
        };
        if let Some(index) = choice {
            self.menu_index = index;
            self.notice = None;
            match index {
                0 => self.open_betting(),
                1 => self.screen = Screen::Rename(RenameState::default()),
                2 => self.screen = Screen::Stats,
                _ => self.should_quit = true,
            }
        }
    }

    /// Start a new race card, pre-filling the previous bet if there was one.
    pub fn open_betting(&mut self) {
        if self.data.is_broke() {
            self.screen = Screen::Broke;
            return;
        }
        let card = match self.pending_card.take() {
            Some(mut card) => {
                // Names may have been edited in the meantime; the field itself is unchanged.
                for (entry, name) in card.entries.iter_mut().zip(&self.data.horse_names) {
                    entry.name.clone_from(name);
                }
                card
            }
            None => {
                let seed = self.seeds.next_seed();
                RaceCard::generate(self.data.stats.races_run + 1, seed, &self.data.horse_names)
            }
        };
        let balance = self.data.balance_cents;
        let (selected, stake) = match self.last_bet {
            Some((lane, stake)) => (lane, stake.min(balance)),
            None => (card.favourite(), DEFAULT_WAGER.min(balance)),
        };
        self.screen = Screen::Betting(BettingState {
            card,
            selected,
            input: format_plain(stake),
            error: None,
        });
    }

    fn key_betting(&mut self, key: KeyEvent) {
        let balance = self.data.balance_cents;
        let Screen::Betting(bet) = &mut self.screen else {
            return;
        };
        match key.code {
            KeyCode::Esc => {
                if let Screen::Betting(state) = std::mem::replace(&mut self.screen, Screen::Menu) {
                    self.pending_card = Some(state.card);
                }
                self.go_menu();
            }
            KeyCode::Up | KeyCode::Char('k') | KeyCode::BackTab => {
                bet.selected = (bet.selected + LANES - 1) % LANES;
            }
            KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => {
                bet.selected = (bet.selected + 1) % LANES;
            }
            KeyCode::Char(c) if c.is_ascii_digit() || matches!(c, '.' | ',' | '$') => {
                if bet.input.len() < MAX_WAGER_INPUT {
                    bet.input.push(c);
                }
                bet.error = None;
            }
            KeyCode::Backspace => {
                bet.input.pop();
                bet.error = None;
            }
            KeyCode::Delete => {
                bet.input.clear();
                bet.error = None;
            }
            KeyCode::Char('a') | KeyCode::Char('A') => {
                bet.input = format_plain(balance);
                bet.error = None;
            }
            KeyCode::Char('h') | KeyCode::Char('H') => {
                bet.input = format_plain((balance / 2).max(crate::betting::MIN_WAGER).min(balance));
                bet.error = None;
            }
            KeyCode::Enter => match parse_wager(&bet.input, balance) {
                Ok(stake) => {
                    let card = bet.card.clone();
                    let lane = bet.selected;
                    self.start_race(card, lane, stake);
                }
                Err(e) => bet.error = Some(e.to_string()),
            },
            _ => {}
        }
    }

    /// Lock in the bet. The race is simulated and settled immediately (it is fully
    /// determined by its seed); the animation then plays it back. Quitting mid-race
    /// therefore can't dodge a loss, and the save is never left with a dangling bet.
    pub fn start_race(&mut self, card: RaceCard, lane: usize, stake: Cents) {
        let odds = card.entries[lane].odds;
        let record = run_race(&card.attributes(), card.seed);
        let commentary = commentary::build(&record, &card.names(), card.seed);
        let placing = record.placing(lane);
        let won = placing.place == 1;
        let returned = if won {
            gross_return(stake, odds, placing.tied_with as u32)
        } else {
            0
        };
        let balance_before = self.data.balance_cents;
        self.data
            .apply_race(stake, returned, won, &record.winners());
        self.persist();
        self.last_bet = Some((lane, stake));

        let outcome = BetOutcome {
            place: placing.place,
            dead_heat_ways: placing.tied_with,
            returned,
            balance_before,
            balance_after: self.data.balance_cents,
        };
        self.screen = Screen::Racing(Box::new(RacingState {
            card,
            record,
            commentary,
            bet: Bet { lane, stake, odds },
            outcome,
            frame: 0,
            fast: false,
            hold: 0,
        }));
    }

    fn key_racing(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char(' ') | KeyCode::Char('f') | KeyCode::Char('F') => {
                if let Screen::Racing(race) = &mut self.screen {
                    race.fast = !race.fast;
                }
            }
            KeyCode::Enter | KeyCode::Esc | KeyCode::Char('s') | KeyCode::Char('S') => {
                self.show_results()
            }
            KeyCode::Char('q') | KeyCode::Char('Q') => self.should_quit = true,
            _ => {}
        }
    }

    fn show_results(&mut self) {
        if let Screen::Racing(race) = std::mem::replace(&mut self.screen, Screen::Menu) {
            let race = *race;
            self.screen = Screen::Results(Box::new(ResultsState {
                card: race.card,
                record: race.record,
                bet: race.bet,
                outcome: race.outcome,
            }));
        }
    }

    fn key_results(&mut self, key: KeyEvent) {
        let broke = self.data.is_broke();
        match key.code {
            KeyCode::Char('r') | KeyCode::Char('R') | KeyCode::Enter if !broke => {
                self.open_betting()
            }
            KeyCode::Char('f') | KeyCode::Char('F') if broke => self.do_fresh_start(),
            KeyCode::Char('m') | KeyCode::Char('M') | KeyCode::Esc => self.go_menu(),
            KeyCode::Char('q') | KeyCode::Char('Q') => self.should_quit = true,
            _ => {}
        }
    }

    fn key_broke(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('f') | KeyCode::Char('F') => self.do_fresh_start(),
            KeyCode::Esc | KeyCode::Char('m') | KeyCode::Char('M') => self.go_menu(),
            KeyCode::Char('q') | KeyCode::Char('Q') => self.should_quit = true,
            _ => {}
        }
    }

    fn do_fresh_start(&mut self) {
        self.data.fresh_start();
        self.last_bet = None;
        self.persist();
        if self.notice.is_none() {
            self.notice = Some(Notice {
                text: format!(
                    "Fresh start! Your wallet is back to {}. The horses remember nothing.",
                    format_money(self.data.balance_cents)
                ),
                kind: NoticeKind::Info,
            });
        }
        self.menu_index = 0;
        self.go_menu();
    }

    fn key_stats(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('q') | KeyCode::Char('Q') => self.should_quit = true,
            KeyCode::Esc | KeyCode::Enter | KeyCode::Backspace | KeyCode::Char('m') => {
                self.go_menu()
            }
            _ => {}
        }
    }

    fn key_rename(&mut self, key: KeyEvent) {
        let Screen::Rename(state) = &mut self.screen else {
            return;
        };
        if let Some(buffer) = &mut state.editing {
            match key.code {
                KeyCode::Esc => {
                    state.editing = None;
                    state.error = None;
                }
                KeyCode::Enter => {
                    match validate_name(buffer, state.selected, &self.data.horse_names) {
                        Ok(name) => {
                            self.data.horse_names[state.selected] = name;
                            state.editing = None;
                            state.error = None;
                            self.persist();
                        }
                        Err(msg) => state.error = Some(msg),
                    }
                }
                KeyCode::Backspace => {
                    buffer.pop();
                    state.error = None;
                }
                KeyCode::Char(c) if !c.is_control() => {
                    // Allow a little slack; validation enforces the real limit on save.
                    if buffer.chars().count() < crate::horses::MAX_NAME_CHARS + 5 {
                        buffer.push(c);
                    }
                    state.error = None;
                }
                _ => {}
            }
            return;
        }
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                state.selected = (state.selected + LANES - 1) % LANES
            }
            KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => {
                state.selected = (state.selected + 1) % LANES
            }
            KeyCode::Enter | KeyCode::Char('e') => {
                state.editing = Some(self.data.horse_names[state.selected].clone());
                state.error = None;
            }
            KeyCode::Char('d') | KeyCode::Char('D') => {
                let lane = state.selected;
                match validate_name(DEFAULT_NAMES[lane], lane, &self.data.horse_names) {
                    Ok(name) => {
                        self.data.horse_names[lane] = name;
                        state.error = None;
                        self.persist();
                    }
                    Err(msg) => state.error = Some(msg),
                }
            }
            KeyCode::Esc | KeyCode::Char('m') => self.go_menu(),
            KeyCode::Char('q') | KeyCode::Char('Q') => self.should_quit = true,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::betting::STARTING_BALANCE;
    use crate::save::tests::temp_save_path;
    use crossterm::event::{KeyEventKind, KeyEventState};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    fn type_text(app: &mut App, text: &str) {
        for c in text.chars() {
            app.on_key(key(KeyCode::Char(c)));
        }
    }

    fn new_app(tag: &str, seed: u64) -> App {
        App::new(SaveStore::new(temp_save_path(tag)), Some(seed))
    }

    fn cleanup(app: &App) {
        let _ = std::fs::remove_dir_all(app.save_path().parent().unwrap());
    }

    #[test]
    fn full_round_settles_and_saves() {
        let mut app = new_app("round", 1234);
        assert_eq!(app.data.balance_cents, STARTING_BALANCE);
        app.on_key(key(KeyCode::Enter)); // title -> menu
        app.on_key(key(KeyCode::Enter)); // Race
        let Screen::Betting(bet) = &app.screen else {
            panic!("expected betting")
        };
        assert_eq!(bet.card.seed, 1234, "--seed applies to the first race");
        app.on_key(key(KeyCode::Delete));
        type_text(&mut app, "250");
        app.on_key(key(KeyCode::Enter));

        let Screen::Racing(race) = &app.screen else {
            panic!("expected racing")
        };
        let (bet, outcome) = (race.bet, race.outcome);
        assert_eq!(bet.stake, 25_000);
        let expected = if outcome.won() {
            STARTING_BALANCE - 25_000
                + gross_return(25_000, bet.odds, outcome.dead_heat_ways as u32)
        } else {
            STARTING_BALANCE - 25_000
        };
        assert_eq!(app.data.balance_cents, expected);

        // Already saved before the animation finishes.
        let (on_disk, _) = SaveStore::new(app.save_path().to_path_buf()).load();
        assert_eq!(on_disk.balance_cents, expected);
        assert_eq!(on_disk.stats.races_run, 1);

        // Play the whole race through and land on results.
        for _ in 0..2_000 {
            app.on_tick();
            if matches!(app.screen, Screen::Results(_)) {
                break;
            }
        }
        assert!(matches!(app.screen, Screen::Results(_)));

        // Rematch keeps the horse and stake.
        app.on_key(key(KeyCode::Char('r')));
        let Screen::Betting(next) = &app.screen else {
            panic!("expected betting")
        };
        assert_eq!(next.selected, bet.lane);
        assert_eq!(next.input, "250");
        assert_eq!(next.card.seed, 1235);
        cleanup(&app);
    }

    #[test]
    fn invalid_wager_shows_error_and_does_not_race() {
        let mut app = new_app("invalid", 5);
        app.open_betting();
        app.on_key(key(KeyCode::Delete));
        type_text(&mut app, "5000");
        app.on_key(key(KeyCode::Enter));
        let Screen::Betting(bet) = &app.screen else {
            panic!("expected betting")
        };
        assert_eq!(bet.error.as_deref(), Some("You only have $1,000.00."));
        assert_eq!(app.data.balance_cents, STARTING_BALANCE);
        cleanup(&app);
    }

    #[test]
    fn same_seed_same_result() {
        let run = |tag: &str| {
            let mut app = new_app(tag, 777);
            app.open_betting();
            app.on_key(key(KeyCode::Enter));
            let Screen::Racing(race) = &app.screen else {
                panic!()
            };
            let result = (
                race.record.placings.clone(),
                race.commentary.clone(),
                app.data.balance_cents,
            );
            cleanup(&app);
            result
        };
        assert_eq!(run("det-a"), run("det-b"));
    }

    #[test]
    fn backing_out_of_betting_keeps_the_same_field() {
        let mut app = new_app("browse", 4242);
        app.open_betting();
        let Screen::Betting(first) = &app.screen else {
            panic!()
        };
        let seed = first.card.seed;
        app.on_key(key(KeyCode::Esc));
        assert!(matches!(app.screen, Screen::Menu));
        app.data.horse_names[0] = "Dave".into();
        app.open_betting();
        let Screen::Betting(again) = &app.screen else {
            panic!()
        };
        assert_eq!(again.card.seed, seed, "same race card after backing out");
        assert_eq!(again.card.entries[0].name, "Dave", "renames still apply");
        // Once a race is run, the next card is new.
        app.on_key(key(KeyCode::Enter));
        app.on_key(key(KeyCode::Enter));
        app.open_betting();
        let Screen::Betting(next) = &app.screen else {
            panic!()
        };
        assert_eq!(next.card.seed, seed + 1);
        cleanup(&app);
    }

    #[test]
    fn going_broke_offers_fresh_start() {
        let mut app = new_app("broke", 9);
        app.data.balance_cents = 100;
        app.open_betting();
        // Back whoever is NOT the winner with the last dollar.
        let Screen::Betting(bet) = &app.screen else {
            panic!()
        };
        let card = bet.card.clone();
        let winner = run_race(&card.attributes(), card.seed).winners()[0];
        let loser = (winner + 1) % LANES;
        app.start_race(card, loser, 100);
        assert!(app.data.is_broke());
        app.on_key(key(KeyCode::Enter)); // skip to results
        assert!(matches!(app.screen, Screen::Results(_)));
        app.on_key(key(KeyCode::Char('r'))); // rematch is unavailable when broke
        assert!(matches!(app.screen, Screen::Results(_)));
        app.on_key(key(KeyCode::Char('f')));
        assert!(matches!(app.screen, Screen::Menu));
        assert_eq!(app.data.balance_cents, STARTING_BALANCE);
        assert_eq!(app.data.stats.fresh_starts, 1);

        // Choosing Race while broke goes to the fresh-start screen instead.
        app.data.balance_cents = 0;
        app.on_key(key(KeyCode::Char('r')));
        assert!(matches!(app.screen, Screen::Broke));
        cleanup(&app);
    }

    #[test]
    fn renaming_validates_and_persists() {
        let mut app = new_app("rename", 1);
        app.screen = Screen::Rename(RenameState::default());
        app.on_key(key(KeyCode::Enter));
        for _ in 0..20 {
            app.on_key(key(KeyCode::Backspace));
        }
        type_text(&mut app, "neigh sayer");
        app.on_key(key(KeyCode::Enter));
        let Screen::Rename(state) = &app.screen else {
            panic!()
        };
        assert!(state.error.is_some(), "duplicate name rejected");
        for _ in 0..20 {
            app.on_key(key(KeyCode::Backspace));
        }
        type_text(&mut app, "Dave");
        app.on_key(key(KeyCode::Enter));
        assert_eq!(app.data.horse_names[0], "Dave");
        let (on_disk, _) = SaveStore::new(app.save_path().to_path_buf()).load();
        assert_eq!(on_disk.horse_names[0], "Dave");
        app.on_key(key(KeyCode::Char('d')));
        assert_eq!(app.data.horse_names[0], DEFAULT_NAMES[0]);
        cleanup(&app);
    }
}
