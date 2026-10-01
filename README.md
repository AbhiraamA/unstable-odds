# Unstable Odds

A ridiculous horse-racing simulator for your terminal. Name the horses after your
friends, bet fake money, and watch it all go wrong in glorious ASCII.

No real money, no accounts, no network. Just you, six unreliable horses and a
commentator who has seen too much.

## Launch

You need Rust 1.88 or newer (`rustup update` if in doubt).

```sh
cd unstable-odds
cargo run
```

Options (pass them after `--` when using cargo):

```sh
cargo run -- --seed 42          # reproducible races: race 1 uses seed 42, the next 43, ...
cargo run -- --save ./my.json   # use a specific save file
cargo run -- --help
```

The terminal must be at least **70×22**. If it's smaller, the game tells you and
waits for you to resize.

## How to play

1. On the menu, pick **Race**.
2. Look over the field. Each horse shows its odds, a one-line read on its form and
   a quirk. Move down the list to see its full scouting report.
3. Type a wager and press **Enter**.
4. Watch the race. Your horse is marked `◀ BET`.
5. Collect your winnings (or don't), then rematch or head back to the menu.

New players start with **$1,000** of fake money. The minimum bet is $1.00. If your
wallet drops below that, you're offered a clearly labelled **FRESH START** that
resets the wallet to $1,000. Horse names and career stats are kept.

## Controls

**Everywhere:** `Ctrl+C` quits immediately.

**Title:** any key continues, `Q` or `Esc` quits.

**Menu:** `↑`/`↓` (or `j`/`k`, `Tab`) to move, `Enter` to choose. Shortcuts: `R` Race,
`N` Rename Horses, `S` Stats, `Q` Quit.

**Betting:**

| Key | Action |
| --- | --- |
| `↑` `↓` / `j` `k` / `Tab` | Choose a horse |
| `0`–`9` `.` `,` `$` | Type the wager (`$1,250.50`, `12.5` and `.75` all work) |
| `Backspace` / `Delete` | Delete a character / clear the amount |
| `A` / `H` | All in / half your wallet |
| `Enter` | Place the bet and start the race |
| `Esc` | Back to the menu (the same race card is kept) |

**Racing:** `Space` or `F` toggles fast-forward, `Enter`/`Esc`/`S` skips to the
results, `Q` quits.

**Results:** `R` or `Enter` rematches (next race, same horse and stake ready to
confirm), `M`/`Esc` returns to the menu, `Q` quits. When broke, `F` takes the fresh start.

**Rename Horses:** `Enter` or `E` edits the selected name, `Enter` saves, `Esc`
cancels, `D` restores the default. Names can be up to 20 characters and must be
unique. Changes save instantly.

## Odds and payouts

Odds are **decimal**. A winning bet returns `stake × odds`, and that return
**includes your stake**. A $10 bet at 3.50 returns $35.00: your $10 back plus $25
profit. Payouts round down to the cent.

Each horse has a base speed, consistency and stamina. Before every race the game
simulates the field 2,000 times with those same attributes, turns each horse's win
rate into a price and adds a 10% house margin. The favourite really is the
favourite (in testing, favourites won about 40% of races, matching what the odds
predicted), and longshots do occasionally come in.

**Finishing order** is decided by the exact moment each horse crosses the line,
calculated within the final tick. The order horses are updated in never matters.

**Dead heats:** horses that cross at the same instant share the placing. If your
horse dead-heats for first with others, your stake is split between them. You are
paid at full odds on your share and lose the rest. Two horses tied for 1st means
half your stake wins.

**Settlement:** the race is fully determined by its seed, so your bet is settled
and saved the moment you place it, and the animation plays back the result.
Quitting mid-race can't undo a loss.

## Save file

Your wallet, horse names and stats are saved as JSON after every bet, rename and
fresh start:

| Platform | Location |
| --- | --- |
| Linux | `$XDG_DATA_HOME/unstable-odds/save.json`, else `~/.local/share/unstable-odds/save.json` |
| macOS | `~/Library/Application Support/unstable-odds/save.json` |
| Windows | `%APPDATA%\unstable-odds\save.json` |

The `--save PATH` flag or the `UNSTABLE_ODDS_SAVE` environment variable overrides
this. The Stats screen shows the path in use. Saves are written atomically. If the
file is ever unreadable, it's moved aside to `save.json.corrupt` instead of being
silently overwritten, and you start fresh.

## Development

```sh
cargo test                                  # payout maths, wager parsing, determinism, finish order, ...
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

Code layout (`src/`):

| File | Contents |
| --- | --- |
| `sim.rs` | The race simulation: pure, deterministic and independent of rendering |
| `card.rs` | Builds the field and prices the odds |
| `betting.rs` | Money, odds, payouts and wager parsing |
| `commentary.rs` | Turns race events into commentary |
| `horses.rs` | Names, form descriptions and quirks |
| `save.rs` | JSON persistence |
| `app.rs` | Screen state and key handling |
| `ui.rs` | Ratatui drawing |
| `tui.rs` | Terminal setup and the event loop |
| `cli.rs` / `main.rs` | Command-line flags and startup |

The terminal is restored on normal exit, on errors and on panics.
