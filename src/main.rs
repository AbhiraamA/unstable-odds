use std::process::ExitCode;

use unstable_odds::app::App;
use unstable_odds::betting::format_money;
use unstable_odds::cli::{self, Command, USAGE};
use unstable_odds::save::{default_save_path, SaveStore};
use unstable_odds::tui;

fn main() -> ExitCode {
    let options = match cli::parse_args(std::env::args().skip(1)) {
        Ok(Command::Play(options)) => options,
        Ok(Command::Help) => {
            print!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Ok(Command::Version) => {
            println!("unstable-odds {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Err(message) => {
            eprintln!("error: {message}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };

    let path = options.save_path.unwrap_or_else(default_save_path);
    let mut app = App::new(SaveStore::new(path), options.seed);

    match tui::run(&mut app) {
        Ok(()) => {
            println!(
                "Thanks for playing Unstable Odds. Wallet: {} (fake). Saved to {}",
                format_money(app.data.balance_cents),
                app.save_path().display()
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: terminal problem: {e}");
            ExitCode::FAILURE
        }
    }
}
