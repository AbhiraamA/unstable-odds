//! Command-line parsing. Deliberately tiny: no argument-parsing dependency.

use std::path::PathBuf;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Options {
    /// Seed for the first race; later races use seed+1, seed+2, ...
    pub seed: Option<u64>,
    /// Override for the save-file location.
    pub save_path: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Play(Options),
    Help,
    Version,
}

pub const USAGE: &str = "\
Unstable Odds — a ridiculous terminal horse-racing simulator (fake money only).

USAGE:
    unstable-odds [OPTIONS]

OPTIONS:
    -s, --seed <N>      Make races reproducible. Race 1 uses seed N, race 2 uses N+1, ...
                        Accepts decimal or 0x-prefixed hex.
        --save <PATH>   Use a specific save file instead of the default location.
                        (The UNSTABLE_ODDS_SAVE environment variable works too.)
    -h, --help          Show this help.
    -V, --version       Show the version.

With cargo:  cargo run -- --seed 42
";

pub fn parse_seed(text: &str) -> Result<u64, String> {
    let t = text.trim().replace('_', "");
    let parsed = if let Some(hex) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        u64::from_str_radix(hex, 16)
    } else {
        t.parse::<u64>()
    };
    parsed.map_err(|_| format!("invalid seed '{text}': expected a whole number like 42 or 0xBEEF"))
}

pub fn parse_args<I, S>(args: I) -> Result<Command, String>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let mut opts = Options::default();
    let mut iter = args.into_iter().map(Into::into);
    while let Some(arg) = iter.next() {
        let (flag, inline) = match arg.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f.to_string(), Some(v.to_string())),
            _ => (arg.clone(), None),
        };
        let mut value = |name: &str| -> Result<String, String> {
            match inline.clone() {
                Some(v) => Ok(v),
                None => iter.next().ok_or_else(|| format!("{name} needs a value")),
            }
        };
        match flag.as_str() {
            "-h" | "--help" => return Ok(Command::Help),
            "-V" | "--version" => return Ok(Command::Version),
            "-s" | "--seed" => {
                let v = value("--seed")?;
                opts.seed = Some(parse_seed(&v)?);
            }
            "--save" => {
                let v = value("--save")?;
                if v.trim().is_empty() {
                    return Err("--save needs a non-empty path".into());
                }
                opts.save_path = Some(PathBuf::from(v));
            }
            other => return Err(format!("unrecognised argument '{other}' (try --help)")),
        }
    }
    Ok(Command::Play(opts))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn play(args: &[&str]) -> Options {
        match parse_args(args.iter().copied()) {
            Ok(Command::Play(o)) => o,
            other => panic!("expected Play, got {other:?}"),
        }
    }

    #[test]
    fn no_args_plays_with_defaults() {
        assert_eq!(play(&[]), Options::default());
    }

    #[test]
    fn seed_forms() {
        assert_eq!(play(&["--seed", "42"]).seed, Some(42));
        assert_eq!(play(&["--seed=7"]).seed, Some(7));
        assert_eq!(play(&["-s", "0xff"]).seed, Some(255));
        assert_eq!(play(&["--seed", "1_000"]).seed, Some(1000));
    }

    #[test]
    fn save_path() {
        assert_eq!(
            play(&["--save", "/tmp/x.json"]).save_path,
            Some(PathBuf::from("/tmp/x.json"))
        );
        assert_eq!(
            play(&["--save=a.json"]).save_path,
            Some(PathBuf::from("a.json"))
        );
    }

    #[test]
    fn errors() {
        assert!(parse_args(["--seed"]).is_err());
        assert!(parse_args(["--seed", "banana"]).is_err());
        assert!(parse_args(["--seed", "-1"]).is_err());
        assert!(parse_args(["--wat"]).is_err());
        assert!(parse_args(["--save", ""]).is_err());
    }

    #[test]
    fn help_and_version() {
        assert_eq!(parse_args(["-h"]), Ok(Command::Help));
        assert_eq!(parse_args(["--version"]), Ok(Command::Version));
    }
}
