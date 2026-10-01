//! Local JSON save file: balance, horse names and basic stats.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::betting::{Cents, MIN_WAGER, STARTING_BALANCE};
use crate::horses::{sanitize_names, DEFAULT_NAMES};
use crate::sim::LANES;

pub const SAVE_VERSION: u32 = 1;
/// Environment variable that overrides the save location.
pub const SAVE_ENV: &str = "UNSTABLE_ODDS_SAVE";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Stats {
    pub races_run: u64,
    pub bets_won: u64,
    pub bets_lost: u64,
    pub total_wagered_cents: Cents,
    pub total_returned_cents: Cents,
    pub biggest_return_cents: Cents,
    pub peak_balance_cents: Cents,
    pub current_streak: u64,
    pub best_streak: u64,
    pub fresh_starts: u64,
    /// Race wins per lane (a dead heat counts for every horse in it).
    pub horse_wins: Vec<u64>,
}

impl Default for Stats {
    fn default() -> Self {
        Self {
            races_run: 0,
            bets_won: 0,
            bets_lost: 0,
            total_wagered_cents: 0,
            total_returned_cents: 0,
            biggest_return_cents: 0,
            peak_balance_cents: STARTING_BALANCE,
            current_streak: 0,
            best_streak: 0,
            fresh_starts: 0,
            horse_wins: vec![0; LANES],
        }
    }
}

impl Stats {
    pub fn net_cents(&self) -> i128 {
        i128::from(self.total_returned_cents) - i128::from(self.total_wagered_cents)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SaveData {
    pub version: u32,
    pub balance_cents: Cents,
    pub horse_names: Vec<String>,
    pub stats: Stats,
}

impl Default for SaveData {
    fn default() -> Self {
        Self {
            version: SAVE_VERSION,
            balance_cents: STARTING_BALANCE,
            horse_names: DEFAULT_NAMES.iter().map(|s| s.to_string()).collect(),
            stats: Stats::default(),
        }
    }
}

impl SaveData {
    /// Repair anything odd in data read from disk.
    pub fn sanitize(&mut self) {
        self.version = SAVE_VERSION;
        sanitize_names(&mut self.horse_names);
        self.stats.horse_wins.resize(LANES, 0);
        self.stats.peak_balance_cents = self.stats.peak_balance_cents.max(self.balance_cents);
    }

    /// Too poor to place the minimum wager.
    pub fn is_broke(&self) -> bool {
        self.balance_cents < MIN_WAGER
    }

    /// Settle a race: take the stake, pay any return, and update stats.
    pub fn apply_race(
        &mut self,
        stake: Cents,
        returned: Cents,
        backed_winner: bool,
        winners: &[usize],
    ) {
        self.balance_cents = self
            .balance_cents
            .saturating_sub(stake)
            .saturating_add(returned);
        let s = &mut self.stats;
        s.races_run += 1;
        s.total_wagered_cents = s.total_wagered_cents.saturating_add(stake);
        s.total_returned_cents = s.total_returned_cents.saturating_add(returned);
        s.biggest_return_cents = s.biggest_return_cents.max(returned);
        s.peak_balance_cents = s.peak_balance_cents.max(self.balance_cents);
        if backed_winner {
            s.bets_won += 1;
            s.current_streak += 1;
            s.best_streak = s.best_streak.max(s.current_streak);
        } else {
            s.bets_lost += 1;
            s.current_streak = 0;
        }
        s.horse_wins.resize(LANES, 0);
        for &lane in winners {
            if let Some(w) = s.horse_wins.get_mut(lane) {
                *w += 1;
            }
        }
    }

    /// Reset the wallet to the starting balance. Names and stats are kept.
    pub fn fresh_start(&mut self) {
        self.balance_cents = STARTING_BALANCE;
        self.stats.fresh_starts += 1;
        self.stats.current_streak = 0;
    }
}

/// What happened when loading.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadOutcome {
    /// No save file yet: a brand-new player.
    New,
    Loaded,
    /// The file was unreadable; it was moved aside (if possible) and a new save started.
    Recovered {
        backup: Option<PathBuf>,
        reason: String,
    },
}

#[derive(Clone, Debug)]
pub struct SaveStore {
    path: PathBuf,
}

impl SaveStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> (SaveData, LoadOutcome) {
        let text = match fs::read_to_string(&self.path) {
            Ok(text) => text,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return (SaveData::default(), LoadOutcome::New)
            }
            Err(e) => return (SaveData::default(), self.recover(e.to_string())),
        };
        match serde_json::from_str::<SaveData>(&text) {
            Ok(mut data) => {
                data.sanitize();
                (data, LoadOutcome::Loaded)
            }
            Err(e) => (SaveData::default(), self.recover(e.to_string())),
        }
    }

    fn recover(&self, reason: String) -> LoadOutcome {
        let backup = self.path.with_extension("json.corrupt");
        let backup = fs::rename(&self.path, &backup).ok().map(|_| backup);
        LoadOutcome::Recovered { backup, reason }
    }

    /// Write atomically: a temp file next to the save, then rename over it.
    pub fn save(&self, data: &SaveData) -> io::Result<()> {
        if let Some(dir) = self.path.parent().filter(|d| !d.as_os_str().is_empty()) {
            fs::create_dir_all(dir)?;
        }
        let json = serde_json::to_string_pretty(data).map_err(io::Error::other)?;
        let tmp = self.path.with_extension("json.tmp");
        fs::write(&tmp, json)?;
        fs::rename(&tmp, &self.path)
    }
}

/// Where the save lives by default.
///
/// * `$UNSTABLE_ODDS_SAVE` if set
/// * Linux and other Unix: `$XDG_DATA_HOME/unstable-odds/save.json`
///   (default `~/.local/share/unstable-odds/save.json`)
/// * macOS: `~/Library/Application Support/unstable-odds/save.json`
/// * Windows: `%APPDATA%\unstable-odds\save.json`
/// * otherwise `./unstable-odds-save.json`
pub fn default_save_path() -> PathBuf {
    if let Some(p) = std::env::var_os(SAVE_ENV).filter(|p| !p.is_empty()) {
        return PathBuf::from(p);
    }
    data_dir()
        .map(|d| d.join("unstable-odds").join("save.json"))
        .unwrap_or_else(|| PathBuf::from("unstable-odds-save.json"))
}

fn env_dir(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
}

fn data_dir() -> Option<PathBuf> {
    if cfg!(windows) {
        env_dir("APPDATA")
    } else if cfg!(target_os = "macos") {
        env_dir("HOME").map(|h| h.join("Library").join("Application Support"))
    } else {
        env_dir("XDG_DATA_HOME").or_else(|| env_dir("HOME").map(|h| h.join(".local").join("share")))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// A unique scratch path under the system temp dir (no extra dependencies needed).
    pub(crate) fn temp_save_path(tag: &str) -> PathBuf {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir()
            .join(format!(
                "unstable-odds-test-{}-{tag}-{n}",
                std::process::id()
            ))
            .join("save.json")
    }

    #[test]
    fn missing_file_means_new_player_with_starting_money() {
        let store = SaveStore::new(temp_save_path("missing"));
        let (data, outcome) = store.load();
        assert_eq!(outcome, LoadOutcome::New);
        assert_eq!(data.balance_cents, STARTING_BALANCE);
        assert_eq!(data.horse_names.len(), LANES);
    }

    #[test]
    fn round_trips_through_json() {
        let store = SaveStore::new(temp_save_path("roundtrip"));
        let mut data = SaveData::default();
        data.horse_names[0] = "Dave".into();
        data.apply_race(5_000, 17_000, true, &[0]);
        store.save(&data).unwrap();
        let (loaded, outcome) = store.load();
        assert_eq!(outcome, LoadOutcome::Loaded);
        assert_eq!(loaded, data);
        assert_eq!(loaded.balance_cents, STARTING_BALANCE - 5_000 + 17_000);
        let _ = fs::remove_dir_all(store.path().parent().unwrap());
    }

    #[test]
    fn tolerates_missing_and_bad_fields() {
        let store = SaveStore::new(temp_save_path("partial"));
        fs::create_dir_all(store.path().parent().unwrap()).unwrap();
        fs::write(
            store.path(),
            r#"{"balance_cents": 4200, "horse_names": ["Dave", ""]}"#,
        )
        .unwrap();
        let (data, outcome) = store.load();
        assert_eq!(outcome, LoadOutcome::Loaded);
        assert_eq!(data.balance_cents, 4_200);
        assert_eq!(data.horse_names[0], "Dave");
        assert_eq!(data.horse_names[1], DEFAULT_NAMES[1]);
        assert_eq!(data.horse_names.len(), LANES);
        assert_eq!(data.stats.horse_wins.len(), LANES);
        let _ = fs::remove_dir_all(store.path().parent().unwrap());
    }

    #[test]
    fn corrupt_file_is_backed_up_not_lost() {
        let store = SaveStore::new(temp_save_path("corrupt"));
        fs::create_dir_all(store.path().parent().unwrap()).unwrap();
        fs::write(store.path(), "{ this is not json").unwrap();
        let (data, outcome) = store.load();
        assert_eq!(data, SaveData::default());
        match outcome {
            LoadOutcome::Recovered {
                backup: Some(b), ..
            } => {
                assert_eq!(fs::read_to_string(b).unwrap(), "{ this is not json");
            }
            other => panic!("unexpected {other:?}"),
        }
        let _ = fs::remove_dir_all(store.path().parent().unwrap());
    }

    #[test]
    fn stats_track_wins_losses_and_fresh_starts() {
        let mut data = SaveData::default();
        data.apply_race(10_000, 0, false, &[2]);
        data.apply_race(10_000, 30_000, true, &[1]);
        data.apply_race(10_000, 25_000, true, &[1, 4]);
        let s = &data.stats;
        assert_eq!((s.races_run, s.bets_won, s.bets_lost), (3, 2, 1));
        assert_eq!(s.best_streak, 2);
        assert_eq!(s.horse_wins, vec![0, 2, 1, 0, 1, 0]);
        assert_eq!(s.net_cents(), 25_000);
        assert_eq!(data.balance_cents, STARTING_BALANCE + 25_000);

        data.balance_cents = 50;
        assert!(data.is_broke());
        data.fresh_start();
        assert!(!data.is_broke());
        assert_eq!(data.balance_cents, STARTING_BALANCE);
        assert_eq!(data.stats.fresh_starts, 1);
        assert_eq!(data.stats.races_run, 3, "stats survive a fresh start");
    }
}
