//! The race card: this race's field, each horse's form, and odds derived from simulation.
//!
//! Odds come from running the *same* simulation used for the real race a few thousand
//! times with independent seeds, counting wins, and applying a small house margin.

use fastrand::Rng;

use crate::betting::Odds;
use crate::horses::{profile, QUIRKS};
use crate::sim::{derive_seed, run_quick, Attributes, LANES};

/// Number of simulated races used to price each card.
pub const ODDS_SIMULATIONS: u32 = 2_000;
/// The book is priced to about 110%: roughly a 9% edge for the (fake) house.
pub const HOUSE_MARGIN: f64 = 0.10;

const SALT_CARD: u64 = 0x4341_5244; // "CARD"
const SALT_ODDS: u64 = 0x4F44_4453; // "ODDS"

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub name: String,
    pub attrs: Attributes,
    /// Share of simulated races this horse won (before the house margin).
    pub win_chance: f64,
    pub odds: Odds,
    pub profile: String,
    pub quirk: &'static str,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RaceCard {
    /// Lifetime race number, for display.
    pub number: u64,
    /// Seed that determines form, odds and the race itself.
    pub seed: u64,
    pub entries: Vec<Entry>,
}

impl RaceCard {
    pub fn generate(number: u64, seed: u64, names: &[String]) -> Self {
        let mut rng = Rng::with_seed(derive_seed(seed, SALT_CARD, 0));
        let attrs: [Attributes; LANES] = std::array::from_fn(|_| Attributes::random(&mut rng));
        let mut quirks = QUIRKS.to_vec();
        rng.shuffle(&mut quirks);

        let chances = estimate_win_chances(&attrs, seed, ODDS_SIMULATIONS);
        let entries = (0..LANES)
            .map(|lane| Entry {
                name: names
                    .get(lane)
                    .cloned()
                    .unwrap_or_else(|| format!("Horse #{}", lane + 1)),
                attrs: attrs[lane],
                win_chance: chances[lane],
                odds: odds_for_chance(chances[lane]),
                profile: profile(&attrs[lane]),
                quirk: quirks[lane],
            })
            .collect();
        Self {
            number,
            seed,
            entries,
        }
    }

    pub fn attributes(&self) -> [Attributes; LANES] {
        std::array::from_fn(|lane| self.entries[lane].attrs)
    }

    pub fn names(&self) -> Vec<String> {
        self.entries.iter().map(|e| e.name.clone()).collect()
    }

    /// Lane of the horse with the shortest odds (lowest lane on equal odds).
    pub fn favourite(&self) -> usize {
        (0..LANES)
            .min_by_key(|&lane| (self.entries[lane].odds, lane))
            .unwrap_or(0)
    }
}

/// Monte Carlo estimate of each horse's chance to win. Dead heats split the win.
pub fn estimate_win_chances(attrs: &[Attributes; LANES], seed: u64, sims: u32) -> [f64; LANES] {
    let mut wins = [0.0_f64; LANES];
    for k in 0..sims {
        let placings = run_quick(attrs, derive_seed(seed, SALT_ODDS, u64::from(k)));
        let winners: Vec<usize> = placings
            .iter()
            .filter(|p| p.place == 1)
            .map(|p| p.lane)
            .collect();
        let share = 1.0 / winners.len() as f64;
        for lane in winners {
            wins[lane] += share;
        }
    }
    // Light smoothing so a horse that never won in simulation still gets finite odds.
    let total = f64::from(sims) + 0.5 * LANES as f64;
    wins.map(|w| (w + 0.5) / total)
}

/// Decimal odds for a win probability, including the house margin, rounded down
/// to the cent and clamped to the range the book offers.
pub fn odds_for_chance(chance: f64) -> Odds {
    let fair = 1.0 / (chance.max(1e-6) * (1.0 + HOUSE_MARGIN));
    let hundredths = (fair * 100.0).floor().clamp(0.0, f64::from(u32::MAX)) as u32;
    Odds::from_hundredths(hundredths)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::horses::DEFAULT_NAMES;

    fn names() -> Vec<String> {
        DEFAULT_NAMES.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn cards_are_reproducible_from_seed() {
        let a = RaceCard::generate(1, 99, &names());
        let b = RaceCard::generate(1, 99, &names());
        assert_eq!(a, b);
        let c = RaceCard::generate(1, 100, &names());
        assert_ne!(a.attributes(), c.attributes());
    }

    #[test]
    fn chances_sum_to_one_and_book_has_margin() {
        for seed in 0..5 {
            let card = RaceCard::generate(1, seed, &names());
            let total: f64 = card.entries.iter().map(|e| e.win_chance).sum();
            assert!((total - 1.0).abs() < 1e-9, "chances sum to {total}");
            let book: f64 = card
                .entries
                .iter()
                .map(|e| 100.0 / f64::from(e.odds.hundredths()))
                .sum();
            assert!(book > 1.0 && book < 1.25, "overround {book}");
        }
    }

    #[test]
    fn odds_follow_the_attributes() {
        let avg = Attributes {
            speed: 1.0,
            consistency: 0.6,
            stamina: 0.6,
        };
        let star = Attributes {
            speed: 1.05,
            consistency: 0.9,
            stamina: 0.9,
        };
        let attrs = [avg, avg, star, avg, avg, avg];
        let chances = estimate_win_chances(&attrs, 3, 1_000);
        let best = (0..LANES)
            .max_by(|&a, &b| chances[a].total_cmp(&chances[b]))
            .unwrap();
        assert_eq!(best, 2);
        assert!(odds_for_chance(chances[2]) < odds_for_chance(chances[0]));
    }

    #[test]
    fn odds_from_chance() {
        assert_eq!(odds_for_chance(0.5).to_string(), "1.81");
        assert_eq!(odds_for_chance(0.1).to_string(), "9.09");
        assert_eq!(odds_for_chance(0.0), Odds::MAX);
        assert_eq!(odds_for_chance(1.0), Odds::MIN);
    }
}
