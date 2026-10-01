//! Money, odds, wager validation and payout math.
//!
//! All money is stored as whole cents (`u64`) and odds as hundredths (`u32`), so payouts are
//! exact integer arithmetic. The displayed odds are exactly the odds used for payment.

use std::fmt;

/// Fake money, in cents.
pub type Cents = u64;

/// What a new player (or a fresh start) gets: $1,000.00.
pub const STARTING_BALANCE: Cents = 100_000;
/// Smallest accepted wager: $1.00.
pub const MIN_WAGER: Cents = 100;
/// Pre-filled wager for a first bet: $100.00.
pub const DEFAULT_WAGER: Cents = 10_000;

/// Decimal odds stored in hundredths: `Odds::from_hundredths(340)` is 3.40.
///
/// A winning bet returns `stake × odds`, which *includes* the original stake.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Odds(u32);

impl Odds {
    pub const MIN: Odds = Odds(101);
    pub const MAX: Odds = Odds(10_000);

    /// Build odds, clamped to the range offered by the book (1.01 to 100.00).
    pub fn from_hundredths(hundredths: u32) -> Self {
        Odds(hundredths.clamp(Self::MIN.0, Self::MAX.0))
    }

    pub fn hundredths(self) -> u32 {
        self.0
    }
}

impl fmt::Display for Odds {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = format!("{}.{:02}", self.0 / 100, self.0 % 100);
        f.pad(&text)
    }
}

/// Full return of a winning bet, including the stake, rounded down to the cent.
///
/// Dead-heat rule: when `dead_heat_ways` horses share first place, the stake is divided
/// equally between them and the share on the backed horse is paid at full odds, so the
/// return is `stake × odds ÷ ways`. With no dead heat, pass 1.
pub fn gross_return(stake: Cents, odds: Odds, dead_heat_ways: u32) -> Cents {
    let ways = u128::from(dead_heat_ways.max(1));
    let total = u128::from(stake) * u128::from(odds.hundredths()) / (100 * ways);
    Cents::try_from(total).unwrap_or(Cents::MAX)
}

/// Why a wager was rejected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WagerError {
    Empty,
    NotANumber,
    Negative,
    TooManyDecimals,
    BelowMinimum,
    ExceedsBalance { balance: Cents },
}

impl fmt::Display for WagerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WagerError::Empty => write!(f, "Type an amount to bet."),
            WagerError::NotANumber => write!(f, "That's not a number. Try 25 or 12.50."),
            WagerError::Negative => write!(f, "Nice try. Wagers must be positive."),
            WagerError::TooManyDecimals => write!(f, "Cents only: at most two decimal places."),
            WagerError::BelowMinimum => {
                write!(f, "Minimum wager is {}.", format_money(MIN_WAGER))
            }
            WagerError::ExceedsBalance { balance } => {
                write!(f, "You only have {}.", format_money(*balance))
            }
        }
    }
}

/// Parse and validate a typed wager such as `25`, `12.50`, `$1,000` or `.75`.
pub fn parse_wager(input: &str, balance: Cents) -> Result<Cents, WagerError> {
    let trimmed = input.trim();
    let trimmed = trimmed.strip_prefix('$').unwrap_or(trimmed).trim_start();
    if trimmed.starts_with('-') {
        return Err(WagerError::Negative);
    }
    let cleaned: String = trimmed.chars().filter(|&c| c != ',' && c != '_').collect();
    if cleaned.is_empty() {
        return Err(WagerError::Empty);
    }

    let (whole, frac) = cleaned.split_once('.').unwrap_or((cleaned.as_str(), ""));
    let is_digits = |s: &str| s.chars().all(|c| c.is_ascii_digit());
    if (whole.is_empty() && frac.is_empty()) || !is_digits(whole) || !is_digits(frac) {
        return Err(WagerError::NotANumber);
    }
    if frac.len() > 2 {
        return Err(WagerError::TooManyDecimals);
    }

    let too_big = WagerError::ExceedsBalance { balance };
    let whole_cents = if whole.is_empty() {
        0
    } else {
        whole
            .parse::<u64>()
            .ok()
            .and_then(|w| w.checked_mul(100))
            .ok_or_else(|| too_big.clone())?
    };
    let frac_cents = match frac.len() {
        0 => 0,
        1 => frac.parse::<u64>().unwrap_or(0) * 10,
        _ => frac.parse::<u64>().unwrap_or(0),
    };
    let cents = whole_cents.checked_add(frac_cents).ok_or(too_big)?;

    if cents < MIN_WAGER {
        Err(WagerError::BelowMinimum)
    } else if cents > balance {
        Err(WagerError::ExceedsBalance { balance })
    } else {
        Ok(cents)
    }
}

/// `$1,234.56`
pub fn format_money(cents: Cents) -> String {
    format!("${}.{:02}", group_thousands(cents / 100), cents % 100)
}

/// `+$12.00` or `-$12.00`
pub fn format_signed_money(delta: i128) -> String {
    let sign = if delta < 0 { '-' } else { '+' };
    let magnitude = Cents::try_from(delta.unsigned_abs()).unwrap_or(Cents::MAX);
    format!("{sign}{}", format_money(magnitude))
}

/// A wager as you would type it: `100` or `12.50` (no symbol or separators).
pub fn format_plain(cents: Cents) -> String {
    if cents.is_multiple_of(100) {
        (cents / 100).to_string()
    } else {
        format!("{}.{:02}", cents / 100, cents % 100)
    }
}

fn group_thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const BAL: Cents = 100_000;

    #[test]
    fn winning_return_includes_stake() {
        // $100 at 3.40 returns $340: the $100 stake plus $240 profit.
        assert_eq!(gross_return(10_000, Odds::from_hundredths(340), 1), 34_000);
        assert_eq!(gross_return(100, Odds::from_hundredths(101), 1), 101);
    }

    #[test]
    fn payout_rounds_down_to_the_cent() {
        // $1.01 at 3.33 = $3.3633 -> $3.36
        assert_eq!(gross_return(101, Odds::from_hundredths(333), 1), 336);
        // $0.99 * 1.99 = 1.9701 -> $1.97
        assert_eq!(gross_return(99, Odds::from_hundredths(199), 1), 197);
    }

    #[test]
    fn dead_heat_splits_the_stake() {
        let odds = Odds::from_hundredths(400);
        assert_eq!(gross_return(10_000, odds, 2), 20_000);
        assert_eq!(gross_return(10_000, odds, 3), 13_333);
        assert_eq!(
            gross_return(10_000, odds, 0),
            40_000,
            "0 ways is treated as 1"
        );
    }

    #[test]
    fn payout_never_overflows() {
        assert_eq!(gross_return(Cents::MAX, Odds::MAX, 1), Cents::MAX);
        assert_eq!(
            gross_return(Cents::MAX / 100, Odds::from_hundredths(100 * 100), 1),
            Cents::MAX / 100 * 100
        );
    }

    #[test]
    fn odds_are_clamped_and_displayed_with_two_decimals() {
        assert_eq!(Odds::from_hundredths(0), Odds::MIN);
        assert_eq!(Odds::from_hundredths(1_000_000), Odds::MAX);
        assert_eq!(Odds::from_hundredths(340).to_string(), "3.40");
        assert_eq!(Odds::from_hundredths(1205).to_string(), "12.05");
        assert_eq!(format!("{:>6}", Odds::from_hundredths(250)), "  2.50");
    }

    #[test]
    fn accepts_reasonable_wagers() {
        assert_eq!(parse_wager("25", BAL), Ok(2_500));
        assert_eq!(parse_wager("  25  ", BAL), Ok(2_500));
        assert_eq!(parse_wager("12.5", BAL), Ok(1_250));
        assert_eq!(parse_wager("12.50", BAL), Ok(1_250));
        assert_eq!(parse_wager("$1,000", BAL), Ok(100_000));
        assert_eq!(parse_wager("$ 7.", BAL), Ok(700));
        assert_eq!(parse_wager("1", BAL), Ok(100));
        assert_eq!(parse_wager("1000.00", BAL), Ok(BAL), "all in is fine");
    }

    #[test]
    fn rejects_invalid_wagers() {
        assert_eq!(parse_wager("", BAL), Err(WagerError::Empty));
        assert_eq!(parse_wager("   ", BAL), Err(WagerError::Empty));
        assert_eq!(parse_wager("$", BAL), Err(WagerError::Empty));
        assert_eq!(parse_wager("abc", BAL), Err(WagerError::NotANumber));
        assert_eq!(parse_wager(".", BAL), Err(WagerError::NotANumber));
        assert_eq!(parse_wager("1.2.3", BAL), Err(WagerError::NotANumber));
        assert_eq!(parse_wager("1e5", BAL), Err(WagerError::NotANumber));
        assert_eq!(parse_wager("NaN", BAL), Err(WagerError::NotANumber));
        assert_eq!(parse_wager("-5", BAL), Err(WagerError::Negative));
        assert_eq!(parse_wager("5.555", BAL), Err(WagerError::TooManyDecimals));
        assert_eq!(parse_wager("0", BAL), Err(WagerError::BelowMinimum));
        assert_eq!(parse_wager("0.99", BAL), Err(WagerError::BelowMinimum));
        assert_eq!(parse_wager(".5", BAL), Err(WagerError::BelowMinimum));
        assert_eq!(
            parse_wager("1000.01", BAL),
            Err(WagerError::ExceedsBalance { balance: BAL })
        );
        assert_eq!(
            parse_wager("99999999999999999999999", BAL),
            Err(WagerError::ExceedsBalance { balance: BAL })
        );
    }

    #[test]
    fn broke_players_cannot_bet() {
        assert_eq!(
            parse_wager("1", 50),
            Err(WagerError::ExceedsBalance { balance: 50 })
        );
    }

    #[test]
    fn formats_money() {
        assert_eq!(format_money(0), "$0.00");
        assert_eq!(format_money(5), "$0.05");
        assert_eq!(format_money(123_456_789), "$1,234,567.89");
        assert_eq!(format_money(100_000), "$1,000.00");
        assert_eq!(format_signed_money(-2_500), "-$25.00");
        assert_eq!(format_signed_money(2_500), "+$25.00");
        assert_eq!(format_plain(10_000), "100");
        assert_eq!(format_plain(1_250), "12.50");
        assert_eq!(format_plain(1_205), "12.05");
    }
}
