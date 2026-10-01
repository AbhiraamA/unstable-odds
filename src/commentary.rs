//! Turns race events into dramatic commentary.
//!
//! Commentary has its own random stream, derived from the race seed, so it is reproducible
//! and can never influence the race itself.

use fastrand::Rng;

use crate::sim::{derive_seed, RaceEvent, RaceRecord, LANES};

const SALT_COMMENTARY: u64 = 0x5441_4C4B; // "TALK"
const EVENT_COOLDOWN: usize = 12;
const LEAD_COOLDOWN: usize = 10;
const QUIET_GAP: usize = 45;
/// Winning margins below this many ticks are called a photo finish.
pub const PHOTO_FINISH_TICKS: f64 = 0.6;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Normal,
    Hype,
    Good,
    Bad,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommentaryLine {
    /// Playback frame at which the line appears.
    pub frame: usize,
    pub text: String,
    pub tone: Tone,
}

const OPENERS: &[&str] = &[
    "And they're off!",
    "The gates fly open and they're away!",
    "And they're off! Mostly in the right direction!",
];

const BURSTS: &[&str] = &[
    "{h} has found a second gear!",
    "{h} just remembered there are carrots at the finish!",
    "{h} is absolutely flying!",
    "{h} has unlocked turbo mode. Nobody knew that was an option.",
    "{h} smells victory. Or possibly snacks.",
    "Somebody told {h} about the prize money!",
    "{h} surges like an unexpected tax refund!",
];

const STUMBLES: &[&str] = &[
    "{h} has forgotten why they're here.",
    "{h} stops to reconsider some life choices.",
    "{h} is distracted by a very interesting cloud.",
    "{h} trips over absolutely nothing.",
    "{h} appears to be checking their phone.",
    "{h} has wandered over to greet a fan.",
    "{h} is lodging a complaint with the stewards. Mid-race.",
];

const LEADS: &[&str] = &[
    "{h} takes the lead!",
    "{h} storms to the front!",
    "It's {h} out in front now!",
    "{h} pokes a nose ahead!",
];

const STRETCH: &[&str] = &[
    "Into the final stretch, and it's {h} leading the charge!",
    "Here comes the home straight, {h} in front!",
];

const FILLERS: &[&str] = &[
    "{h} is running on vibes alone.",
    "{h} is pacing themselves. Possibly for next year.",
    "{h} has the look of a horse with a plan. A bad plan.",
    "{h} appears to be humming.",
    "The crowd is going mild!",
    "Somewhere, a bookmaker is sweating.",
];

const WINS: &[&str] = &[
    "{h} wins it! Absolute scenes!",
    "{h} crosses the line first! Somebody call their mother!",
    "IT'S {h}! WHAT A RACE!",
];

const REAR: &[&str] = &[
    "{h} brings up the rear, with dignity.",
    "{h} finishes last but technically finishes.",
];

fn say(rng: &mut Rng, pool: &[&str], name: &str) -> String {
    pool[rng.usize(0..pool.len())].replace("{h}", name)
}

/// Build the full commentary track for a recorded race.
pub fn build(record: &RaceRecord, names: &[String], seed: u64) -> Vec<CommentaryLine> {
    let mut rng = Rng::with_seed(derive_seed(seed, SALT_COMMENTARY, 0));
    let name = |lane: usize| names.get(lane).map(String::as_str).unwrap_or("A horse");
    let last_frame = record.frames.len().saturating_sub(1);

    let mut lines = vec![CommentaryLine {
        frame: 0,
        text: say(&mut rng, OPENERS, ""),
        tone: Tone::Hype,
    }];

    let first_finish = record
        .events
        .iter()
        .find(|e| matches!(e.event, RaceEvent::Finished(_)))
        .map_or(last_frame, |e| e.frame);

    let mut last_said = 0;
    for ev in record.events.iter().filter(|e| e.frame < first_finish) {
        let (pool, lane, tone, cooldown) = match ev.event {
            RaceEvent::Burst(l) => (BURSTS, l, Tone::Good, EVENT_COOLDOWN),
            RaceEvent::Stumble(l) => (STUMBLES, l, Tone::Bad, EVENT_COOLDOWN),
            RaceEvent::LeadChange(l) => (LEADS, l, Tone::Normal, LEAD_COOLDOWN),
            RaceEvent::FinalStretch(l) => (STRETCH, l, Tone::Hype, 0),
            RaceEvent::Finished(_) => continue,
        };
        if ev.frame < last_said + cooldown {
            continue;
        }
        lines.push(CommentaryLine {
            frame: ev.frame,
            text: say(&mut rng, pool, name(lane)),
            tone,
        });
        last_said = ev.frame;
    }

    // Fill long quiet stretches with some colour.
    let mut fillers = Vec::new();
    let mut boundaries: Vec<usize> = lines.iter().map(|l| l.frame).collect();
    boundaries.push(first_finish);
    for pair in boundaries.windows(2) {
        let (mut cursor, next) = (pair[0], pair[1]);
        while next.saturating_sub(cursor) >= QUIET_GAP {
            cursor += QUIET_GAP / 2 + rng.usize(0..QUIET_GAP / 2);
            if cursor + 5 >= next {
                break;
            }
            let lane = rng.usize(0..LANES);
            fillers.push(CommentaryLine {
                frame: cursor,
                text: say(&mut rng, FILLERS, name(lane)),
                tone: Tone::Normal,
            });
        }
    }
    lines.extend(fillers);

    // The finish.
    let winners = record.winners();
    let win_frame = record.finish_frame(winners[0]).unwrap_or(last_frame);
    if winners.len() > 1 {
        let listed: Vec<&str> = winners.iter().map(|&l| name(l)).collect();
        let (last, rest) = listed.split_last().expect("at least two winners");
        lines.push(CommentaryLine {
            frame: win_frame,
            text: format!(
                "DEAD HEAT! {} and {last} cannot be separated!",
                rest.join(", ")
            ),
            tone: Tone::Hype,
        });
    } else {
        let margin = match (record.placings.first(), record.placings.get(1)) {
            (Some(a), Some(b)) => b.time.zip(a.time).map(|(tb, ta)| tb - ta),
            _ => None,
        };
        let mut reveal = win_frame;
        if let (Some(m), Some(second)) = (margin, record.placings.get(1)) {
            if m < PHOTO_FINISH_TICKS {
                lines.push(CommentaryLine {
                    frame: win_frame,
                    text: format!(
                        "PHOTO FINISH between {} and {}! The stewards are squinting...",
                        name(winners[0]),
                        name(second.lane)
                    ),
                    tone: Tone::Hype,
                });
                reveal = (win_frame + 8).min(last_frame);
            }
        }
        lines.push(CommentaryLine {
            frame: reveal,
            text: say(&mut rng, WINS, name(winners[0])),
            tone: Tone::Hype,
        });
    }
    if let Some(last) = record.placings.last() {
        if let Some(frame) = record.finish_frame(last.lane) {
            if last.place > 1 && frame > win_frame + 3 {
                lines.push(CommentaryLine {
                    frame,
                    text: say(&mut rng, REAR, name(last.lane)),
                    tone: Tone::Normal,
                });
            }
        }
    }

    for line in &mut lines {
        line.frame = line.frame.min(last_frame);
    }
    lines.sort_by_key(|l| l.frame);
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::{run_race, Attributes};

    fn setup(seed: u64) -> (RaceRecord, Vec<String>) {
        let mut rng = Rng::with_seed(seed);
        let attrs: [Attributes; LANES] = std::array::from_fn(|_| Attributes::random(&mut rng));
        let names = (1..=LANES).map(|i| format!("Horse{i}")).collect();
        (run_race(&attrs, seed), names)
    }

    #[test]
    fn commentary_is_deterministic_and_ordered() {
        for seed in 0..20 {
            let (record, names) = setup(seed);
            let a = build(&record, &names, seed);
            assert_eq!(a, build(&record, &names, seed));
            assert!(a.windows(2).all(|w| w[0].frame <= w[1].frame));
            assert!(a.iter().all(|l| l.frame < record.frames.len()));
            assert!(!a.iter().any(|l| l.text.contains("{h}")));
        }
    }

    #[test]
    fn commentary_names_the_winner() {
        for seed in 0..20 {
            let (record, names) = setup(seed);
            let lines = build(&record, &names, seed);
            let winner = &names[record.winners()[0]];
            assert!(
                lines
                    .iter()
                    .any(|l| l.tone == Tone::Hype && l.text.contains(winner.as_str())),
                "seed {seed}: no winner call"
            );
        }
    }
}
