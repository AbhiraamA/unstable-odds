//! Race simulation: pure, deterministic, and independent of rendering, input and clocks.
//!
//! Positions are measured in track lengths. Every horse starts at `0.0` and the finish line
//! is at [`FINISH`]. One call to [`RaceSim::step`] advances the race by one tick.
//!
//! Each horse owns its own random stream (derived from the race seed and its lane), and horses
//! never interact, so a horse's run does not depend on the order horses are updated in.
//! Finishing order is decided by the exact crossing time inside the tick (see
//! [`crossing_time`] and [`compute_placings`]), never by iteration order.

use fastrand::Rng;

/// Number of horses in every race.
pub const LANES: usize = 6;
/// Distance of the finish line, in track lengths.
pub const FINISH: f64 = 1.0;
/// Distance a perfectly average horse covers per tick at full pace.
pub const BASE_STEP: f64 = 0.0055;
/// Safety cap; with the minimum-speed floor every horse finishes long before this.
pub const MAX_TICKS: u32 = 4_000;
/// Crossing times closer than this (in ticks) are a dead heat.
pub const TIE_EPSILON: f64 = 1e-9;

const RAMP_TICKS: f64 = 6.0;
const EVENT_GRACE_TICKS: u32 = 10;
const MOOD_PERSISTENCE: f64 = 0.92;
const MOOD_LIMIT: f64 = 0.35;
const BURST_BOOST: f64 = 1.40;
const STUMBLE_DRAG: f64 = 0.35;
const MIN_SPEED_FACTOR: f64 = 0.10;
/// How far past the line a finished horse jogs (purely visual; results are already fixed).
const GALLOP_OUT: f64 = 0.10;
const LEAD_MARGIN: f64 = 0.004;
const LEAD_CALLS_FROM_TICK: u32 = 20;
const FINAL_STRETCH_AT: f64 = 0.75;

const SALT_RUN: u64 = 0x5255_4E5F_5241_4345; // "RUN_RACE"

/// The three attributes that drive both the race and the displayed odds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Attributes {
    /// Base pace multiplier, around 1.0.
    pub speed: f64,
    /// 0..=1. Higher means smaller random swings and fewer bursts or stumbles.
    pub consistency: f64,
    /// 0..=1. High stamina finishes strongly; low stamina starts quick and fades.
    pub stamina: f64,
}

impl Attributes {
    pub const SPEED: (f64, f64) = (0.95, 1.05);
    pub const CONSISTENCY: (f64, f64) = (0.30, 0.95);
    pub const STAMINA: (f64, f64) = (0.25, 0.95);

    /// Roll a horse's form for one race.
    pub fn random(rng: &mut Rng) -> Self {
        Self {
            speed: lerp(Self::SPEED, rng.f64()),
            consistency: lerp(Self::CONSISTENCY, rng.f64()),
            stamina: lerp(Self::STAMINA, rng.f64()),
        }
    }

    /// Attributes normalised to 0..=1 within their possible ranges (for display).
    pub fn ratings(&self) -> [f64; 3] {
        [
            unlerp(Self::SPEED, self.speed),
            unlerp(Self::CONSISTENCY, self.consistency),
            unlerp(Self::STAMINA, self.stamina),
        ]
    }
}

fn lerp((lo, hi): (f64, f64), t: f64) -> f64 {
    lo + (hi - lo) * t
}

fn unlerp((lo, hi): (f64, f64), v: f64) -> f64 {
    ((v - lo) / (hi - lo)).clamp(0.0, 1.0)
}

/// A temporary condition affecting a horse.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    Burst,
    Stumble,
}

/// Things worth commenting on. Lane indices are 0-based.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RaceEvent {
    Burst(usize),
    Stumble(usize),
    LeadChange(usize),
    FinalStretch(usize),
    Finished(usize),
}

/// An event tagged with the snapshot index at which it becomes visible.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimedEvent {
    pub frame: usize,
    pub event: RaceEvent,
}

/// The state of the field after a tick, for playback.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Snapshot {
    pub positions: [f64; LANES],
    pub effects: [Option<Effect>; LANES],
}

/// A finishing position. Horses in a dead heat share the same `place`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placing {
    pub lane: usize,
    /// 1-based. Uses standard competition ranking, so a dead heat for first gives 1, 1, 3.
    pub place: usize,
    /// Crossing time in ticks (fractional), or `None` if the horse never finished.
    pub time: Option<f64>,
    /// How many horses share this place (1 means no dead heat).
    pub tied_with: usize,
}

/// The time, in ticks, at which a horse moving from `start` by `velocity` during tick `tick`
/// crosses the finish line, or `None` if it does not cross during this tick.
///
/// The fractional part is the share of the tick that elapsed before the nose hit the line,
/// assuming constant speed within the tick.
pub fn crossing_time(tick: u32, start: f64, velocity: f64) -> Option<f64> {
    if start < FINISH && velocity > 0.0 && start + velocity >= FINISH {
        Some(f64::from(tick) + (FINISH - start) / velocity)
    } else {
        None
    }
}

/// Rank horses by crossing time (earliest first). Horses that never finished rank after all
/// finishers, by distance covered. Crossing times within [`TIE_EPSILON`] of the first horse
/// in a group form a dead heat and share the place. The returned order within a dead heat is
/// by lane, for display only; it never changes anyone's place.
pub fn compute_placings(times: &[Option<f64>; LANES], positions: &[f64; LANES]) -> Vec<Placing> {
    let key = |lane: usize| (times[lane], positions[lane]);
    let mut order: Vec<usize> = (0..LANES).collect();
    order.sort_by(|&a, &b| compare_runs(key(a), key(b)).then(a.cmp(&b)));

    let mut placings = Vec::with_capacity(LANES);
    let mut start = 0;
    while start < order.len() {
        let mut end = start + 1;
        while end < order.len() && same_result(key(order[start]), key(order[end])) {
            end += 1;
        }
        for &lane in &order[start..end] {
            placings.push(Placing {
                lane,
                place: start + 1,
                time: times[lane],
                tied_with: end - start,
            });
        }
        start = end;
    }
    placings
}

fn compare_runs(a: (Option<f64>, f64), b: (Option<f64>, f64)) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    match (a.0, b.0) {
        (Some(ta), Some(tb)) => ta.total_cmp(&tb),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => b.1.total_cmp(&a.1),
    }
}

fn same_result(a: (Option<f64>, f64), b: (Option<f64>, f64)) -> bool {
    match (a.0, b.0) {
        (Some(ta), Some(tb)) => (ta - tb).abs() <= TIE_EPSILON,
        (None, None) => (a.1 - b.1).abs() <= TIE_EPSILON,
        _ => false,
    }
}

/// SplitMix64 finaliser: a cheap, well-mixed hash used to derive independent seeds.
pub fn splitmix64(x: u64) -> u64 {
    let mut z = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Derive an independent seed for a purpose (`salt`) and index from a race seed.
pub fn derive_seed(seed: u64, salt: u64, index: u64) -> u64 {
    splitmix64(splitmix64(seed ^ salt).wrapping_add(index))
}

struct StepOutcome {
    fired: Option<Effect>,
    crossed_at: Option<f64>,
}

#[derive(Clone, Debug)]
struct Runner {
    attrs: Attributes,
    rng: Rng,
    position: f64,
    mood: f64,
    effect: Option<Effect>,
    effect_left: u32,
    finish_time: Option<f64>,
}

impl Runner {
    fn new(attrs: Attributes, seed: u64) -> Self {
        Self {
            attrs,
            rng: Rng::with_seed(seed),
            position: 0.0,
            mood: 0.0,
            effect: None,
            effect_left: 0,
            finish_time: None,
        }
    }

    fn step(&mut self, tick: u32) -> StepOutcome {
        if self.finish_time.is_some() {
            // Jog out past the line. Cosmetic only: the result is already fixed.
            if self.position < FINISH + GALLOP_OUT {
                self.position += BASE_STEP * 0.5;
            }
            self.effect = None;
            return StepOutcome {
                fired: None,
                crossed_at: None,
            };
        }

        let a = self.attrs;
        let progress = self.position.clamp(0.0, FINISH);
        let wobble = 1.0 - a.consistency;
        let late = ((progress - 0.35) / 0.65).clamp(0.0, 1.0);

        // Correlated "mood": smooth surges and lulls rather than per-tick jitter.
        let amplitude = 0.03 + 0.10 * wobble;
        let shock = (self.rng.f64() * 2.0 - 1.0) * amplitude;
        self.mood = (self.mood * MOOD_PERSISTENCE + shock).clamp(-MOOD_LIMIT, MOOD_LIMIT);

        // Occasional short bursts and stumbles; erratic and tired horses get more of both.
        let mut fired = None;
        if self.effect.is_none() && tick >= EVENT_GRACE_TICKS && progress < 0.96 {
            let roll = self.rng.f64();
            let burst_chance = 0.0025 + 0.0045 * wobble;
            let stumble_chance = 0.0015 + 0.0040 * wobble + 0.0020 * (1.0 - a.stamina) * late;
            if roll < burst_chance {
                fired = Some(Effect::Burst);
                self.effect_left = 8 + self.rng.u32(0..8);
            } else if roll < burst_chance + stumble_chance {
                fired = Some(Effect::Stumble);
                self.effect_left = 5 + self.rng.u32(0..6);
            }
            if fired.is_some() {
                self.effect = fired;
            }
        }
        let effect_factor = match self.effect {
            Some(Effect::Burst) => BURST_BOOST,
            Some(Effect::Stumble) => STUMBLE_DRAG,
            None => 1.0,
        };

        // Stamina shapes the race: low stamina starts a touch quicker then fades late,
        // high stamina starts conservatively and closes strongly.
        let stamina_bias = a.stamina - 0.6;
        let pace = 1.0 - stamina_bias * 0.05 * (1.0 - late) + stamina_bias * 0.18 * late;
        let ramp = ((f64::from(tick) + 1.0) / RAMP_TICKS).min(1.0);

        let velocity = (BASE_STEP * a.speed * pace * (1.0 + self.mood) * effect_factor * ramp)
            .max(BASE_STEP * MIN_SPEED_FACTOR);

        let crossed_at = crossing_time(tick, self.position, velocity);
        self.position += velocity;
        if crossed_at.is_some() {
            self.finish_time = crossed_at;
        }

        if self.effect.is_some() {
            self.effect_left = self.effect_left.saturating_sub(1);
            if self.effect_left == 0 {
                self.effect = None;
            }
        }

        StepOutcome { fired, crossed_at }
    }
}

/// A race in progress.
#[derive(Clone, Debug)]
pub struct RaceSim {
    runners: Vec<Runner>,
    tick: u32,
    leader: Option<usize>,
    called_final_stretch: bool,
}

impl RaceSim {
    /// A race whose randomness is fully determined by `seed`.
    pub fn new(attrs: &[Attributes; LANES], seed: u64) -> Self {
        let seeds = std::array::from_fn(|lane| derive_seed(seed, SALT_RUN, lane as u64));
        Self::with_runner_seeds(attrs, seeds)
    }

    /// A race with an explicit random stream per horse.
    pub fn with_runner_seeds(attrs: &[Attributes; LANES], seeds: [u64; LANES]) -> Self {
        Self {
            runners: attrs
                .iter()
                .zip(seeds)
                .map(|(&a, s)| Runner::new(a, s))
                .collect(),
            tick: 0,
            leader: None,
            called_final_stretch: false,
        }
    }

    pub fn tick(&self) -> u32 {
        self.tick
    }

    pub fn positions(&self) -> [f64; LANES] {
        std::array::from_fn(|lane| self.runners[lane].position)
    }

    pub fn finish_times(&self) -> [Option<f64>; LANES] {
        std::array::from_fn(|lane| self.runners[lane].finish_time)
    }

    pub fn all_finished(&self) -> bool {
        self.runners.iter().all(|r| r.finish_time.is_some())
    }

    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            positions: self.positions(),
            effects: std::array::from_fn(|lane| self.runners[lane].effect),
        }
    }

    /// Advance one tick, appending anything noteworthy to `events`.
    pub fn step(&mut self, events: &mut Vec<RaceEvent>) {
        let tick = self.tick;
        let anyone_finished_before = self.runners.iter().any(|r| r.finish_time.is_some());

        let mut crossed: Vec<(f64, usize)> = Vec::new();
        for (lane, runner) in self.runners.iter_mut().enumerate() {
            let outcome = runner.step(tick);
            match outcome.fired {
                Some(Effect::Burst) => events.push(RaceEvent::Burst(lane)),
                Some(Effect::Stumble) => events.push(RaceEvent::Stumble(lane)),
                None => {}
            }
            if let Some(t) = outcome.crossed_at {
                crossed.push((t, lane));
            }
        }

        if !anyone_finished_before && crossed.is_empty() {
            self.update_leader(tick, events);
        }

        // Announce finishers in crossing-time order, not lane order.
        crossed.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        events.extend(crossed.iter().map(|&(_, lane)| RaceEvent::Finished(lane)));

        self.tick += 1;
    }

    fn update_leader(&mut self, tick: u32, events: &mut Vec<RaceEvent>) {
        let positions = self.positions();
        let front = (0..LANES).fold(0, |best, lane| {
            if positions[lane] > positions[best] {
                lane
            } else {
                best
            }
        });
        if tick >= LEAD_CALLS_FROM_TICK {
            let changed = match self.leader {
                None => true,
                Some(current) => {
                    front != current && positions[front] > positions[current] + LEAD_MARGIN
                }
            };
            if changed {
                self.leader = Some(front);
                events.push(RaceEvent::LeadChange(front));
            }
        }
        if !self.called_final_stretch && positions[front] >= FINAL_STRETCH_AT {
            self.called_final_stretch = true;
            events.push(RaceEvent::FinalStretch(front));
        }
    }
}

/// A complete race, recorded for playback.
#[derive(Clone, Debug, PartialEq)]
pub struct RaceRecord {
    /// `frames[0]` is the start; `frames[n]` is the field after tick `n - 1`.
    pub frames: Vec<Snapshot>,
    pub events: Vec<TimedEvent>,
    pub finish_times: [Option<f64>; LANES],
    pub placings: Vec<Placing>,
}

impl RaceRecord {
    pub fn placing(&self, lane: usize) -> Placing {
        *self
            .placings
            .iter()
            .find(|p| p.lane == lane)
            .expect("every lane has a placing")
    }

    /// All lanes that share first place (more than one only on a dead heat).
    pub fn winners(&self) -> Vec<usize> {
        self.placings
            .iter()
            .filter(|p| p.place == 1)
            .map(|p| p.lane)
            .collect()
    }

    /// The first frame in which this horse is shown past the finish line.
    pub fn finish_frame(&self, lane: usize) -> Option<usize> {
        self.finish_times[lane].map(|t| (t.floor() as usize + 1).min(self.frames.len() - 1))
    }
}

/// Run a whole race and record every frame and event.
pub fn run_race(attrs: &[Attributes; LANES], seed: u64) -> RaceRecord {
    let mut sim = RaceSim::new(attrs, seed);
    let mut frames = vec![sim.snapshot()];
    let mut events = Vec::new();
    let mut tick_events = Vec::new();
    while !sim.all_finished() && sim.tick() < MAX_TICKS {
        tick_events.clear();
        sim.step(&mut tick_events);
        let frame = frames.len();
        events.extend(tick_events.iter().map(|&event| TimedEvent { frame, event }));
        frames.push(sim.snapshot());
    }
    let finish_times = sim.finish_times();
    let placings = compute_placings(&finish_times, &sim.positions());
    RaceRecord {
        frames,
        events,
        finish_times,
        placings,
    }
}

/// Run a race without recording frames (used by the pre-race odds simulation).
pub fn run_quick(attrs: &[Attributes; LANES], seed: u64) -> Vec<Placing> {
    let mut sim = RaceSim::new(attrs, seed);
    let mut scratch = Vec::new();
    while !sim.all_finished() && sim.tick() < MAX_TICKS {
        scratch.clear();
        sim.step(&mut scratch);
    }
    compute_placings(&sim.finish_times(), &sim.positions())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field() -> [Attributes; LANES] {
        let mut rng = Rng::with_seed(7);
        std::array::from_fn(|_| Attributes::random(&mut rng))
    }

    #[test]
    fn same_seed_gives_identical_race() {
        let attrs = field();
        let a = run_race(&attrs, 42);
        let b = run_race(&attrs, 42);
        assert_eq!(a, b);
    }

    #[test]
    fn different_seeds_give_different_races() {
        let attrs = field();
        let a = run_race(&attrs, 1);
        let b = run_race(&attrs, 2);
        assert_ne!(a.finish_times, b.finish_times);
    }

    #[test]
    fn quick_and_recorded_runs_agree() {
        let attrs = field();
        for seed in 0..20 {
            assert_eq!(run_race(&attrs, seed).placings, run_quick(&attrs, seed));
        }
    }

    #[test]
    fn every_horse_finishes_even_with_extreme_attributes() {
        let slow = Attributes {
            speed: Attributes::SPEED.0,
            consistency: Attributes::CONSISTENCY.0,
            stamina: Attributes::STAMINA.0,
        };
        let fast = Attributes {
            speed: Attributes::SPEED.1,
            consistency: Attributes::CONSISTENCY.1,
            stamina: Attributes::STAMINA.1,
        };
        let attrs = [slow, fast, slow, fast, slow, fast];
        for seed in 0..25 {
            let record = run_race(&attrs, seed);
            assert!(record.finish_times.iter().all(Option::is_some));
            assert!(
                record.frames.len() < 600,
                "race took {} ticks",
                record.frames.len()
            );
        }
    }

    #[test]
    fn crossing_time_is_fractional_within_tick() {
        assert_eq!(crossing_time(10, 0.99, 0.02), Some(10.5));
        assert_eq!(crossing_time(10, 0.98, 0.02), Some(11.0));
        assert_eq!(crossing_time(10, 0.90, 0.02), None);
        assert_eq!(crossing_time(10, 1.00, 0.02), None, "already past the line");
    }

    #[test]
    fn finish_order_uses_crossing_time_not_lane_order() {
        // All three cross during tick 100. Lane 0 is furthest ahead at the start of the tick,
        // but lane 2 is moving fastest and hits the line first.
        let starts = [(0.996, 0.006), (0.995, 0.010), (0.990, 0.040)];
        let mut times = [None; LANES];
        let mut positions = [0.5; LANES];
        for (lane, &(start, v)) in starts.iter().enumerate() {
            times[lane] = crossing_time(100, start, v);
            positions[lane] = start + v;
        }
        let placings = compute_placings(&times, &positions);
        let order: Vec<usize> = placings.iter().take(3).map(|p| p.lane).collect();
        assert_eq!(order, vec![2, 1, 0]);
        assert_eq!(placings[0].place, 1);
        assert_eq!(placings[0].tied_with, 1);
    }

    #[test]
    fn placings_do_not_depend_on_lane_numbering() {
        let times = [
            Some(180.4),
            Some(179.9),
            None,
            Some(181.0),
            Some(179.95),
            None,
        ];
        let positions = [1.0, 1.0, 0.97, 1.0, 1.0, 0.98];
        let base = compute_placings(&times, &positions);
        // Reverse the lanes and check every horse keeps its place.
        let mut rev_times = times;
        let mut rev_pos = positions;
        rev_times.reverse();
        rev_pos.reverse();
        let reversed = compute_placings(&rev_times, &rev_pos);
        for p in &base {
            let mirrored = reversed
                .iter()
                .find(|q| q.lane == LANES - 1 - p.lane)
                .unwrap();
            assert_eq!(p.place, mirrored.place);
        }
        let order: Vec<usize> = base.iter().map(|p| p.lane).collect();
        assert_eq!(
            order,
            vec![1, 4, 0, 3, 5, 2],
            "unfinished horses rank by distance"
        );
    }

    #[test]
    fn dead_heat_shares_place_with_competition_ranking() {
        let times = [
            Some(150.25),
            Some(150.25),
            Some(151.0),
            Some(150.0),
            Some(152.0),
            Some(153.0),
        ];
        let placings = compute_placings(&times, &[1.0; LANES]);
        let places: Vec<(usize, usize, usize)> = placings
            .iter()
            .map(|p| (p.lane, p.place, p.tied_with))
            .collect();
        assert_eq!(
            places,
            vec![
                (3, 1, 1),
                (0, 2, 2),
                (1, 2, 2),
                (2, 4, 1),
                (4, 5, 1),
                (5, 6, 1)
            ]
        );
    }

    #[test]
    fn dead_heat_for_first_lists_all_winners() {
        let times = [
            Some(99.5),
            Some(100.0),
            Some(99.5),
            Some(101.0),
            Some(102.0),
            Some(103.0),
        ];
        let placings = compute_placings(&times, &[1.0; LANES]);
        assert_eq!(placings[0].place, 1);
        assert_eq!(placings[1].place, 1);
        assert_eq!(placings[0].tied_with, 2);
        assert_eq!(placings[2].place, 3);
    }

    #[test]
    fn horses_are_independent_of_update_order() {
        // Permute both the attributes and each horse's random stream: every horse must
        // produce exactly the same run, so the winner is the same horse.
        let attrs = field();
        let seeds: [u64; LANES] = std::array::from_fn(|i| 1000 + i as u64 * 7);
        let perm = [5, 3, 1, 0, 4, 2];
        let p_attrs: [Attributes; LANES] = std::array::from_fn(|i| attrs[perm[i]]);
        let p_seeds: [u64; LANES] = std::array::from_fn(|i| seeds[perm[i]]);

        let run = |a: &[Attributes; LANES], s: [u64; LANES]| {
            let mut sim = RaceSim::with_runner_seeds(a, s);
            let mut ev = Vec::new();
            while !sim.all_finished() {
                sim.step(&mut ev);
            }
            sim.finish_times()
        };
        let base = run(&attrs, seeds);
        let permuted = run(&p_attrs, p_seeds);
        for i in 0..LANES {
            assert_eq!(permuted[i], base[perm[i]]);
        }
    }

    #[test]
    fn finish_events_follow_crossing_order() {
        let attrs = field();
        for seed in 0..30 {
            let record = run_race(&attrs, seed);
            let finished: Vec<usize> = record
                .events
                .iter()
                .filter_map(|e| match e.event {
                    RaceEvent::Finished(lane) => Some(lane),
                    _ => None,
                })
                .collect();
            let by_time: Vec<usize> = record.placings.iter().map(|p| p.lane).collect();
            assert_eq!(finished, by_time, "seed {seed}");
        }
    }
}
