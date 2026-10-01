//! Horse names, personalities and name validation.

use crate::sim::{Attributes, LANES};

pub const DEFAULT_NAMES: [&str; LANES] = [
    "Tax Evasion",
    "Sir Trips-a-Lot",
    "Unpaid Intern",
    "Mane Character",
    "Neigh Sayer",
    "Mid-Life Crisis",
];

pub const MAX_NAME_CHARS: usize = 20;

/// Flavour text. Purely cosmetic: it never affects the race.
pub const QUIRKS: &[&str] = &[
    "Believes the finish line is a social construct.",
    "Was promised a sandwich.",
    "Has a podcast nobody asked for.",
    "Ran a 5K once and won't stop mentioning it.",
    "Allergic to hay, and to effort.",
    "Still upset about last Tuesday.",
    "Thinks the jockey is a backpack.",
    "Hooves freshly waxed for aerodynamics.",
    "Trained exclusively on motivational posters.",
    "Only runs well when someone is filming.",
    "Owes money to at least three other horses.",
    "Recently discovered espresso.",
    "Kicks like a mule. Is not a mule.",
    "Insists on being called a 'speed pony'.",
    "Has never once read the race rules.",
    "Doing this ironically.",
];

/// A short, honest summary of a horse's attributes for this race.
pub fn profile(attrs: &Attributes) -> String {
    let [speed, _, _] = attrs.ratings();
    let pace = match speed {
        r if r >= 0.75 => "Blazing",
        r if r >= 0.5 => "Quick",
        r if r >= 0.25 => "Middling",
        _ => "Plodding",
    };
    let steadiness = match attrs.consistency {
        c if c >= 0.80 => "rock steady",
        c if c >= 0.62 => "fairly steady",
        c if c >= 0.45 => "streaky",
        _ => "wildly erratic",
    };
    let finish = match attrs.stamina {
        s if s >= 0.78 => "strong closer",
        s if s >= 0.55 => "keeps going",
        s if s >= 0.40 => "tires late",
        _ => "fades badly",
    };
    format!("{pace}, {steadiness}, {finish}")
}

/// Validate a new name for `lane`. Returns the cleaned name or a message for the player.
pub fn validate_name(candidate: &str, lane: usize, names: &[String]) -> Result<String, String> {
    let cleaned: String = candidate
        .chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if cleaned.is_empty() {
        return Err("A horse needs a name. Even a bad one.".into());
    }
    if cleaned.chars().count() > MAX_NAME_CHARS {
        return Err(format!("Keep it to {MAX_NAME_CHARS} characters or fewer."));
    }
    let taken = names
        .iter()
        .enumerate()
        .any(|(i, n)| i != lane && n.to_lowercase() == cleaned.to_lowercase());
    if taken {
        return Err("Another horse already has that name.".into());
    }
    Ok(cleaned)
}

/// Fix up names loaded from disk: exactly six, non-empty, short and unique.
pub fn sanitize_names(names: &mut Vec<String>) {
    names.resize(LANES, String::new());
    for lane in 0..LANES {
        let current = names[lane].clone();
        names[lane] = match validate_name(&current, lane, &names[..lane]) {
            Ok(clean) => clean,
            Err(_) => {
                // Fall back to the default, then to a numbered variant if that is taken too.
                let fallback = DEFAULT_NAMES[lane].to_string();
                if validate_name(&fallback, lane, &names[..lane]).is_ok() {
                    fallback
                } else {
                    format!("Horse #{}", lane + 1)
                }
            }
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names() -> Vec<String> {
        DEFAULT_NAMES.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn defaults_are_valid_and_unique() {
        let mut n = names();
        sanitize_names(&mut n);
        assert_eq!(n, names());
        assert!(QUIRKS.len() >= LANES);
    }

    #[test]
    fn validates_names() {
        let n = names();
        assert_eq!(
            validate_name("  Dave   the  Horse ", 0, &n),
            Ok("Dave the Horse".into())
        );
        assert!(validate_name("   ", 0, &n).is_err());
        assert!(validate_name(&"x".repeat(MAX_NAME_CHARS + 1), 0, &n).is_err());
        assert!(
            validate_name("unpaid intern", 0, &n).is_err(),
            "duplicate, any case"
        );
        assert!(
            validate_name("Tax Evasion", 0, &n).is_ok(),
            "keeping your own name is fine"
        );
    }

    #[test]
    fn sanitizes_broken_saves() {
        let mut n = vec!["".to_string(), "Bob".into(), "bob".into()];
        sanitize_names(&mut n);
        assert_eq!(n.len(), LANES);
        assert_eq!(n[0], DEFAULT_NAMES[0]);
        assert_eq!(n[1], "Bob");
        assert_eq!(n[2], DEFAULT_NAMES[2]);
    }

    #[test]
    fn profiles_reflect_attributes() {
        let a = Attributes {
            speed: 1.05,
            consistency: 0.9,
            stamina: 0.9,
        };
        assert_eq!(profile(&a), "Blazing, rock steady, strong closer");
        let b = Attributes {
            speed: 0.95,
            consistency: 0.3,
            stamina: 0.3,
        };
        assert_eq!(profile(&b), "Plodding, wildly erratic, fades badly");
    }
}
