//! Rendering. Reads `App` state and draws it; never mutates the game.

use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Cell, Paragraph, Row, Table, Widget, Wrap},
    Frame,
};

use crate::app::{
    App, BettingState, NoticeKind, RacingState, RenameState, ResultsState, Screen, FAST_FORWARD,
    MENU_ITEMS,
};
use crate::betting::{
    format_money, format_signed_money, gross_return, parse_wager, MIN_WAGER, STARTING_BALANCE,
};
use crate::commentary::{Tone, PHOTO_FINISH_TICKS};
use crate::horses::MAX_NAME_CHARS;
use crate::sim::{Effect, Placing, LANES};

pub const MIN_WIDTH: u16 = 70;
pub const MIN_HEIGHT: u16 = 22;

/// Display scale: one simulation tick is reported as this many "race seconds".
const SECONDS_PER_TICK: f64 = 0.3;

pub const LANE_COLORS: [Color; LANES] = [
    Color::LightRed,
    Color::LightGreen,
    Color::LightYellow,
    Color::LightBlue,
    Color::LightMagenta,
    Color::LightCyan,
];

const SPRITE_W: u16 = 7;
const HORSE_BODY: &str = " ,___/^";
const HORSE_LEGS: [&str; 2] = ["~/\\ /\\ ", "~|\\ |\\ "];

pub fn draw(f: &mut Frame, app: &App) {
    let area = f.area();
    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        draw_too_small(f, area);
        return;
    }
    match &app.screen {
        Screen::Title => draw_title(f, area, app),
        Screen::Menu => draw_menu(f, area, app),
        Screen::Betting(s) => draw_betting(f, area, app, s),
        Screen::Racing(s) => draw_racing(f, area, app, s),
        Screen::Results(s) => draw_results(f, area, app, s),
        Screen::Rename(s) => draw_rename(f, area, app, s),
        Screen::Stats => draw_stats(f, area, app),
        Screen::Broke => draw_broke(f, area, app),
    }
}

// ----- shared helpers ----------------------------------------------------------------------

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let w = width.min(area.width);
    let h = height.min(area.height);
    Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + (area.height - h) / 2,
        width: w,
        height: h,
    }
}

fn panel(title: &str) -> Block<'_> {
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(Color::DarkGray))
        .title(Span::styled(title, Style::new().bold().fg(Color::Yellow)))
}

fn footer(keys: &[(&str, &str)]) -> Paragraph<'static> {
    let mut spans = Vec::new();
    for (i, (key, what)) in keys.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw("   "));
        }
        spans.push(Span::styled(
            key.to_string(),
            Style::new().bold().fg(Color::Yellow),
        ));
        spans.push(Span::styled(
            format!(" {what}"),
            Style::new().fg(Color::Gray),
        ));
    }
    Paragraph::new(Line::from(spans)).centered()
}

fn lane_style(lane: usize) -> Style {
    Style::new().fg(LANE_COLORS[lane % LANES])
}

fn ordinal(n: usize) -> String {
    let suffix = match (n % 10, n % 100) {
        (1, 11) | (2, 12) | (3, 13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    };
    format!("{n}{suffix}")
}

fn place_label(p: &Placing) -> String {
    if p.tied_with > 1 {
        format!("={}", ordinal(p.place))
    } else {
        ordinal(p.place)
    }
}

fn race_time(ticks: f64) -> String {
    let secs = ticks * SECONDS_PER_TICK;
    format!("{}:{:05.2}", (secs / 60.0).floor() as u64, secs % 60.0)
}

fn bar(rating: f64, width: usize) -> String {
    let filled = ((rating.clamp(0.0, 1.0) * width as f64).round() as usize).clamp(1, width);
    format!("{}{}", "█".repeat(filled), "░".repeat(width - filled))
}

fn blink(app: &App, period: u64) -> bool {
    (app.anim / period).is_multiple_of(2)
}

fn notice_line(app: &App) -> Option<Line<'static>> {
    app.notice.as_ref().map(|n| {
        let style = match n.kind {
            NoticeKind::Info => Style::new().fg(Color::LightGreen),
            NoticeKind::Warning => Style::new().fg(Color::LightRed).bold(),
        };
        Line::styled(n.text.clone(), style)
    })
}

/// Paint a horse whose nose is at column `nose`, clipped to `[min_x, max_x)`.
/// Spaces in the sprite are transparent.
#[allow(clippy::too_many_arguments)]
fn paint_horse(
    buf: &mut Buffer,
    nose: i32,
    y: u16,
    phase: usize,
    style: Style,
    min_x: u16,
    max_x: u16,
) {
    let left = nose - i32::from(SPRITE_W) + 1;
    for (row, text) in [HORSE_BODY, HORSE_LEGS[phase % 2]].into_iter().enumerate() {
        for (i, ch) in text.chars().enumerate() {
            let x = left + i as i32;
            if ch == ' ' || x < i32::from(min_x) || x >= i32::from(max_x) {
                continue;
            }
            if let Some(cell) = buf.cell_mut((x as u16, y + row as u16)) {
                cell.set_char(ch).set_style(style);
            }
        }
    }
}

/// Shortens `text` to at most `width` characters, ending in "…" when cut.
fn ellipsize(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_string();
    }
    if width == 0 {
        return String::new();
    }
    let mut out: String = text.chars().take(width - 1).collect();
    out.truncate(out.trim_end_matches([' ', ',']).len());
    out.push('…');
    out
}

fn put(buf: &mut Buffer, x: u16, y: u16, ch: char, style: Style) {
    if let Some(cell) = buf.cell_mut((x, y)) {
        cell.set_char(ch).set_style(style);
    }
}

fn draw_too_small(f: &mut Frame, area: Rect) {
    let lines = vec![
        Line::styled("Terminal too small", Style::new().bold().fg(Color::Yellow)),
        Line::raw(""),
        Line::raw(format!(
            "Unstable Odds needs at least {MIN_WIDTH}×{MIN_HEIGHT}."
        )),
        Line::raw(format!("Current size: {}×{}.", area.width, area.height)),
        Line::raw(""),
        Line::raw("Enlarge the window (or shrink the font)."),
        Line::styled("Ctrl+C quits at any time.", Style::new().fg(Color::Gray)),
    ];
    let h = (lines.len() as u16).min(area.height);
    let rect = Rect {
        y: area.y + (area.height - h) / 2,
        height: h,
        ..area
    };
    f.render_widget(
        Paragraph::new(lines).centered().wrap(Wrap { trim: true }),
        rect,
    );
}

// ----- title -------------------------------------------------------------------------------

fn glyph(c: char) -> [&'static str; 5] {
    match c {
        'U' => ["█   █", "█   █", "█   █", "█   █", " ███ "],
        'N' => ["█   █", "██  █", "█ █ █", "█  ██", "█   █"],
        'S' => [" ████", "█    ", " ███ ", "    █", "████ "],
        'T' => ["█████", "  █  ", "  █  ", "  █  ", "  █  "],
        'A' => [" ███ ", "█   █", "█████", "█   █", "█   █"],
        'B' => ["████ ", "█   █", "████ ", "█   █", "████ "],
        'L' => ["█    ", "█    ", "█    ", "█    ", "█████"],
        'E' => ["█████", "█    ", "████ ", "█    ", "█████"],
        'O' => [" ███ ", "█   █", "█   █", "█   █", " ███ "],
        'D' => ["████ ", "█   █", "█   █", "█   █", "████ "],
        _ => ["     "; 5],
    }
}

fn big_word(word: &str, colors: [Color; 5]) -> Vec<Line<'static>> {
    (0..5)
        .map(|row| {
            let text: Vec<&str> = word.chars().map(|c| glyph(c)[row]).collect();
            Line::styled(text.join(" "), Style::new().fg(colors[row]).bold())
        })
        .collect()
}

fn draw_title(f: &mut Frame, area: Rect, app: &App) {
    let rect = centered(area, area.width, 21);
    let [logo, _, tagline, _, horses, _, prompt] = Layout::vertical([
        Constraint::Length(11),
        Constraint::Length(1),
        Constraint::Length(2),
        Constraint::Length(1),
        Constraint::Length(4),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(rect);

    let mut lines = big_word(
        "UNSTABLE",
        [
            Color::LightYellow,
            Color::Yellow,
            Color::LightRed,
            Color::Red,
            Color::Red,
        ],
    );
    lines.push(Line::raw(""));
    lines.extend(big_word(
        "ODDS",
        [
            Color::LightGreen,
            Color::LightGreen,
            Color::Green,
            Color::Green,
            Color::Green,
        ],
    ));
    f.render_widget(Paragraph::new(lines).centered(), logo);

    f.render_widget(
        Paragraph::new(vec![
            Line::styled(
                "A RIDICULOUS HORSE-RACING SIMULATOR",
                Style::new().bold().fg(Color::White),
            ),
            Line::styled(
                "Name horses after your friends. Bet fake money. Regret real things.",
                Style::new().fg(Color::Gray).italic(),
            ),
        ])
        .centered(),
        tagline,
    );

    // Two horses jostling across the screen forever.
    let strip = centered(horses, horses.width.min(64), 4);
    let cycle = i32::from(strip.width) + 24;
    let base = (app.anim as i32 * 2 / 3 + 30) % cycle - 10;
    let jostle_a = ((app.anim / 6) % 4) as i32;
    let jostle_b = 3 - ((app.anim / 9) % 4) as i32;
    let buf = f.buffer_mut();
    for x in strip.x..strip.right() {
        let ground = if (u64::from(x) + app.anim / 2).is_multiple_of(3) {
            '·'
        } else {
            ' '
        };
        put(
            buf,
            x,
            strip.y + 1,
            ground,
            Style::new().fg(Color::DarkGray),
        );
        put(
            buf,
            x,
            strip.y + 3,
            ground,
            Style::new().fg(Color::DarkGray),
        );
    }
    let phase = (app.anim / 3) as usize;
    for (row, offset, lane) in [(0u16, jostle_a, 0usize), (2, jostle_b, 5)] {
        let nose = i32::from(strip.x) + base + offset;
        paint_horse(
            buf,
            nose,
            strip.y + row,
            phase + lane,
            lane_style(lane).bold(),
            strip.x,
            strip.right(),
        );
    }

    let prompt_style = if blink(app, 10) {
        Style::new().bold().fg(Color::Yellow)
    } else {
        Style::new().fg(Color::DarkGray)
    };
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Press ENTER to start", prompt_style),
            Span::styled("   ·   Q to quit", Style::new().fg(Color::Gray)),
        ]))
        .centered(),
        prompt,
    );
}

// ----- menu --------------------------------------------------------------------------------

fn draw_menu(f: &mut Frame, area: Rect, app: &App) {
    let [banner, _, body, notice, foot] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(3),
        Constraint::Length(1),
    ])
    .areas(area);

    f.render_widget(
        Paragraph::new(vec![
            Line::styled(
                "≋≋≋   U N S T A B L E    O D D S   ≋≋≋",
                Style::new().bold().fg(Color::Yellow),
            ),
            Line::styled(
                "Fake money. Real horses. (The horses are also fake.)",
                Style::new().fg(Color::Gray).italic(),
            ),
        ])
        .centered(),
        Rect {
            y: banner.y + 1,
            height: 2,
            ..banner
        },
    );

    let boxed = centered(body, 40, 12);
    let block = panel(" Main Menu ");
    let inner = block.inner(boxed);
    f.render_widget(block, boxed);

    let wallet_style = if app.data.is_broke() {
        Style::new().bold().fg(Color::LightRed)
    } else {
        Style::new().bold().fg(Color::LightGreen)
    };
    let mut lines = vec![
        Line::from(vec![
            Span::styled("  Wallet: ", Style::new().fg(Color::Gray)),
            Span::styled(format_money(app.data.balance_cents), wallet_style),
            Span::styled("  (fake)", Style::new().fg(Color::DarkGray)),
        ]),
        Line::styled(
            format!("  Races run: {}", app.data.stats.races_run),
            Style::new().fg(Color::Gray),
        ),
        Line::raw(""),
    ];
    let hotkeys = ["R", "N", "S", "Q"];
    for (i, item) in MENU_ITEMS.iter().enumerate() {
        let selected = i == app.menu_index;
        let (marker, style) = if selected {
            (
                "  ▶ ",
                Style::new().bold().fg(Color::Black).bg(Color::Yellow),
            )
        } else {
            ("    ", Style::new().fg(Color::White))
        };
        let label = if i == 0 && app.data.is_broke() {
            format!(" {item} (you're broke!) ")
        } else {
            format!(" {item} ")
        };
        lines.push(Line::from(vec![
            Span::styled(marker, Style::new().fg(Color::Yellow)),
            Span::styled(label, style),
            Span::styled(
                format!("  [{}]", hotkeys[i]),
                Style::new().fg(Color::DarkGray),
            ),
        ]));
    }
    f.render_widget(Paragraph::new(lines), inner);

    if let Some(line) = notice_line(app) {
        f.render_widget(
            Paragraph::new(line).centered().wrap(Wrap { trim: true }),
            notice,
        );
    }
    f.render_widget(
        footer(&[
            ("↑↓", "select"),
            ("Enter", "choose"),
            ("R/N/S/Q", "shortcuts"),
        ]),
        foot,
    );
}

// ----- betting -----------------------------------------------------------------------------

fn draw_betting(f: &mut Frame, area: Rect, app: &App, s: &BettingState) {
    let [header, table, details, wager, foot] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Length(9),
        Constraint::Min(4),
        Constraint::Length(5),
        Constraint::Length(1),
    ])
    .areas(area);

    let card = &s.card;
    let head = panel(" Place your bet ").title(
        Line::styled(
            format!(" Race #{} · seed {} ", card.number, card.seed),
            Style::new().fg(Color::Gray),
        )
        .right_aligned(),
    );
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" Wallet ", Style::new().fg(Color::Gray)),
            Span::styled(
                format_money(app.data.balance_cents),
                Style::new().bold().fg(Color::LightGreen),
            ),
            Span::styled(
                "   ·   Six horses. One winner. Zero refunds.",
                Style::new().fg(Color::Gray),
            ),
        ]))
        .block(head),
        header,
    );

    let favourite = card.favourite();
    let name_w = card
        .entries
        .iter()
        .map(|e| e.name.chars().count())
        .max()
        .unwrap_or(0)
        .max(6)
        + 1;
    // Inner width minus the #, name and odds columns and the three column gaps.
    let profile_w = usize::from(table.width.saturating_sub(2)).saturating_sub(3 + name_w + 10 + 3);
    let rows: Vec<Row> = card
        .entries
        .iter()
        .enumerate()
        .map(|(lane, e)| {
            let selected = lane == s.selected;
            let marker = if selected { "▶" } else { " " };
            let fav = if lane == favourite { " fav" } else { "" };
            let row = Row::new(vec![
                Cell::from(format!("{marker}{}", lane + 1)),
                Cell::from(Span::styled(e.name.clone(), lane_style(lane).bold())),
                Cell::from(Line::from(vec![
                    Span::styled(
                        format!("{:>6}", e.odds),
                        Style::new().bold().fg(Color::White),
                    ),
                    Span::styled(fav, Style::new().fg(Color::Yellow)),
                ])),
                Cell::from(Span::styled(
                    ellipsize(&e.profile, profile_w),
                    Style::new().fg(Color::Gray),
                )),
            ]);
            if selected {
                row.style(
                    Style::new()
                        .bg(Color::DarkGray)
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                row
            }
        })
        .collect();
    let header_row = Row::new(vec!["  #", "Horse", "  Odds", "Form this race"])
        .style(Style::new().fg(Color::Yellow).bold());
    f.render_widget(
        Table::new(
            rows,
            [
                Constraint::Length(3),
                Constraint::Length(name_w as u16),
                Constraint::Length(10),
                Constraint::Fill(1),
            ],
        )
        .header(header_row)
        .block(panel(" The field ")),
        table,
    );

    // Selected horse details.
    let entry = &card.entries[s.selected];
    let block = panel(" Scouting report ").title(
        Line::styled(
            format!(" #{} {} ", s.selected + 1, entry.name),
            lane_style(s.selected).bold(),
        )
        .right_aligned(),
    );
    let inner = block.inner(details);
    f.render_widget(block, details);
    let bw = if inner.width >= 78 {
        10
    } else if inner.width >= 72 {
        8
    } else {
        6
    };
    let [spd, con, sta] = entry.attrs.ratings();
    let label = Style::new().fg(Color::Gray);
    let bars = lane_style(s.selected);
    let detail_lines = vec![
        Line::from(vec![
            Span::styled("Speed ", label),
            Span::styled(bar(spd, bw), bars),
            Span::styled("  Steadiness ", label),
            Span::styled(bar(con, bw), bars),
            Span::styled("  Stamina ", label),
            Span::styled(bar(sta, bw), bars),
            Span::styled(
                if entry.win_chance < 0.005 {
                    "  Sim win <1%".to_string()
                } else {
                    format!("  Sim win {:.0}%", entry.win_chance * 100.0)
                },
                Style::new().fg(Color::White),
            ),
        ]),
        Line::styled(format!("{}.", entry.profile), Style::new().fg(Color::White)),
        Line::styled(
            format!("\u{201c}{}\u{201d}", entry.quirk),
            Style::new().fg(Color::Gray).italic(),
        ),
    ];
    f.render_widget(Paragraph::new(detail_lines), inner);

    // Wager entry.
    let block = panel(" Your wager ");
    let inner = block.inner(wager);
    f.render_widget(block, wager);
    let cursor = if blink(app, 8) { "▏" } else { " " };
    let parsed = parse_wager(&s.input, app.data.balance_cents);
    let status = match (&s.error, &parsed) {
        (Some(err), _) => Line::styled(format!("✗ {err}"), Style::new().bold().fg(Color::LightRed)),
        (None, Ok(stake)) => {
            let back = gross_return(*stake, entry.odds, 1);
            Line::from(vec![
                Span::styled("If it wins: ", Style::new().fg(Color::Gray)),
                Span::styled(
                    format!("{} back", format_money(back)),
                    Style::new().bold().fg(Color::LightGreen),
                ),
                Span::styled(
                    format!(
                        "  ({} stake + {} profit)",
                        format_money(*stake),
                        format_money(back - stake)
                    ),
                    Style::new().fg(Color::Gray),
                ),
            ])
        }
        (None, Err(err)) => Line::styled(format!("· {err}"), Style::new().fg(Color::Yellow)),
    };
    f.render_widget(
        Paragraph::new(vec![
            Line::from(vec![
                Span::raw("Bet on "),
                Span::styled(
                    format!("#{} {}", s.selected + 1, entry.name),
                    lane_style(s.selected).bold(),
                ),
                Span::raw(format!(" @ {}:  ", entry.odds)),
                Span::styled(
                    format!("${}{cursor}", s.input),
                    Style::new().bold().fg(Color::White).bg(Color::DarkGray),
                ),
            ]),
            status,
            Line::styled(
                format!(
                    "Decimal odds: a win returns stake × odds, stake included. Min {}.",
                    format_money(MIN_WAGER)
                ),
                Style::new().fg(Color::DarkGray),
            ),
        ])
        .wrap(Wrap { trim: true }),
        inner,
    );

    f.render_widget(
        footer(&[
            ("↑↓", "horse"),
            ("0-9", "amount"),
            ("A", "all in"),
            ("H", "half"),
            ("Enter", "bet"),
            ("Esc", "menu"),
        ]),
        foot,
    );
}

// ----- racing ------------------------------------------------------------------------------

struct TrackView<'a> {
    race: &'a RacingState,
}

impl TrackView<'_> {
    const PLACE_W: u16 = 5;
    const RUNOUT: u16 = 5;
    /// Everything except the label and the racing span itself.
    const FIXED_W: u16 = Self::PLACE_W + Self::RUNOUT + SPRITE_W + 1;
    const MIN_SPAN: u16 = 24;

    /// Label column sized to the longest name, shrunk only if the track would get too short.
    fn label_width(&self, area_width: u16) -> u16 {
        let longest = self
            .race
            .card
            .entries
            .iter()
            .map(|e| e.name.chars().count())
            .max()
            .unwrap_or(0) as u16;
        let wanted = (longest + 4).max(15);
        let available = area_width.saturating_sub(Self::FIXED_W + Self::MIN_SPAN);
        wanted.min(available).max(10)
    }
}

impl Widget for TrackView<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let race = self.race;
        let snap = &race.record.frames[race.frame];
        let label_w = self.label_width(area.width);
        if area.width < label_w + Self::FIXED_W + 12 || area.height < 2 {
            return;
        }
        let track_x = area.x + label_w;
        let track_end = area.right() - Self::PLACE_W; // exclusive
        let finish_x = track_end - 1 - Self::RUNOUT;
        let start_nose = track_x + SPRITE_W - 1;
        let span = f64::from(finish_x - start_nose);
        let dim = Style::new().fg(Color::DarkGray);

        for lane in 0..LANES {
            let y = area.y + lane as u16 * 2;
            if y + 1 >= area.bottom() {
                break;
            }
            let entry = &race.card.entries[lane];
            let is_pick = lane == race.bet.lane;

            // Ground, distance markers, start gate and finish line.
            for x in track_x..track_end {
                let ch = if (x - track_x).is_multiple_of(2) {
                    '·'
                } else {
                    ' '
                };
                put(buf, x, y + 1, ch, dim);
            }
            for q in [0.25, 0.5, 0.75] {
                let mx = start_nose + (q * span).round() as u16;
                put(buf, mx, y, '┆', dim);
                put(buf, mx, y + 1, '┆', dim);
            }
            put(buf, start_nose + 1, y, '│', dim);
            put(buf, start_nose + 1, y + 1, '│', dim);
            for row in 0..2u16 {
                let light = (lane as u16 * 2 + row).is_multiple_of(2);
                let (ch, color) = if light {
                    ('█', Color::White)
                } else {
                    ('█', Color::DarkGray)
                };
                put(buf, finish_x, y + row, ch, Style::new().fg(color));
            }

            // Label.
            let mut label_style = lane_style(lane);
            if is_pick {
                label_style = label_style.bold();
            }
            let marker = if is_pick { "▶" } else { " " };
            buf.set_stringn(
                area.x,
                y,
                format!("{marker}{} {}", lane + 1, entry.name),
                usize::from(label_w - 1),
                label_style,
            );
            buf.set_stringn(
                area.x,
                y + 1,
                format!("   @{}", entry.odds),
                usize::from(label_w - 1),
                dim,
            );
            if is_pick {
                let tag = "◀ BET";
                let x = area.x + label_w.saturating_sub(6).max(10);
                buf.set_stringn(
                    x,
                    y + 1,
                    tag,
                    usize::from((area.x + label_w - 1).saturating_sub(x)),
                    Style::new().bold().fg(Color::Yellow),
                );
            }

            // The horse.
            let pos = snap.positions[lane];
            let nose = (f64::from(start_nose) + pos * span).round() as i32;
            let nose = nose.min(i32::from(track_end) - 1);
            let phase = (pos * 70.0) as usize;
            let mut style = lane_style(lane).bold();
            if is_pick {
                style = style.add_modifier(Modifier::UNDERLINED);
            }
            paint_horse(buf, nose, y, phase, style, track_x, track_end);

            match snap.effects[lane] {
                Some(Effect::Burst) => {
                    let tail = nose - i32::from(SPRITE_W);
                    for dx in 0..2 {
                        let x = tail - dx;
                        if x >= i32::from(track_x) {
                            put(buf, x as u16, y, '»', Style::new().fg(Color::Yellow).bold());
                        }
                    }
                }
                Some(Effect::Stumble) => {
                    let x = nose + 1;
                    if x + 1 < i32::from(track_end) {
                        put(
                            buf,
                            x as u16,
                            y,
                            '?',
                            Style::new().fg(Color::LightRed).bold(),
                        );
                        put(
                            buf,
                            x as u16 + 1,
                            y,
                            '!',
                            Style::new().fg(Color::LightRed).bold(),
                        );
                    }
                }
                None => {}
            }

            // Placing, once the horse has crossed in the playback.
            if let Some(ff) = race.record.finish_frame(lane) {
                if race.frame >= ff {
                    let p = race.record.placing(lane);
                    let style = if p.place == 1 {
                        Style::new().bold().fg(Color::Black).bg(Color::Yellow)
                    } else {
                        Style::new().bold().fg(Color::White)
                    };
                    buf.set_stringn(
                        track_end + 1,
                        y,
                        place_label(&p),
                        usize::from(Self::PLACE_W - 1),
                        style,
                    );
                }
            }
        }
    }
}

fn draw_racing(f: &mut Frame, area: Rect, app: &App, s: &RacingState) {
    let [header, track, commentary, foot] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Length(LANES as u16 * 2 + 2),
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .areas(area);

    let entry = &s.card.entries[s.bet.lane];
    let status = if s.finished() {
        Span::styled(
            " FINISHED ",
            Style::new().bold().fg(Color::Black).bg(Color::Yellow),
        )
    } else if s.fast {
        Span::styled(
            format!(" ▶▶ x{FAST_FORWARD} "),
            Style::new().bold().fg(Color::Black).bg(Color::LightCyan),
        )
    } else if blink(app, 6) {
        Span::styled(" ● LIVE ", Style::new().bold().fg(Color::LightRed))
    } else {
        Span::styled(" ○ LIVE ", Style::new().fg(Color::Red))
    };
    let race_title = format!(" Race #{} ", s.card.number);
    let block = panel(&race_title).title(
        Line::styled(
            format!(
                " Wallet {} · seed {} ",
                format_money(s.outcome.balance_before - s.bet.stake),
                s.card.seed
            ),
            Style::new().fg(Color::Gray),
        )
        .right_aligned(),
    );
    f.render_widget(
        Paragraph::new(Line::from(vec![
            status,
            Span::raw("  Your bet "),
            Span::styled(
                format_money(s.bet.stake),
                Style::new().bold().fg(Color::White),
            ),
            Span::raw(" on "),
            Span::styled(entry.name.clone(), lane_style(s.bet.lane).bold()),
            Span::raw(format!(" @ {}", s.bet.odds)),
        ]))
        .block(block),
        header,
    );

    let block = panel(" Track ")
        .title(Line::styled(" FINISH ▸ ", Style::new().fg(Color::White).bold()).right_aligned());
    let inner = block.inner(track);
    f.render_widget(block, track);
    f.render_widget(TrackView { race: s }, inner);

    let block = panel(" Commentary ");
    let inner = block.inner(commentary);
    f.render_widget(block, commentary);
    let visible: Vec<_> = s.commentary.iter().filter(|l| l.frame <= s.frame).collect();
    let take = usize::from(inner.height);
    let shown = &visible[visible.len().saturating_sub(take)..];
    let lines: Vec<Line> = shown
        .iter()
        .enumerate()
        .map(|(i, l)| {
            let newest = i + 1 == shown.len();
            let style = match (newest, l.tone) {
                (false, _) => Style::new().fg(Color::Gray),
                (true, Tone::Hype) => Style::new().bold().fg(Color::Yellow),
                (true, Tone::Good) => Style::new().bold().fg(Color::LightGreen),
                (true, Tone::Bad) => Style::new().bold().fg(Color::LightRed),
                (true, Tone::Normal) => Style::new().bold().fg(Color::White),
            };
            Line::styled(format!("▸ {}", l.text), style)
        })
        .collect();
    f.render_widget(Paragraph::new(lines), inner);

    let keys: &[(&str, &str)] = if s.finished() {
        &[("Enter", "results"), ("Q", "quit")]
    } else {
        &[
            ("Space", "fast-forward"),
            ("Enter", "skip"),
            ("Q", "quit (bet already settled)"),
        ]
    };
    f.render_widget(footer(keys), foot);
}

// ----- results -----------------------------------------------------------------------------

fn draw_results(f: &mut Frame, area: Rect, app: &App, s: &ResultsState) {
    let [header, table, summary, foot] = Layout::vertical([
        Constraint::Length(4),
        Constraint::Length(9),
        Constraint::Min(5),
        Constraint::Length(1),
    ])
    .areas(area);

    let record = &s.record;
    let winners = record.winners();
    let name = |lane: usize| s.card.entries[lane].name.clone();
    let mut headline = vec![Span::styled("★ ", Style::new().fg(Color::Yellow))];
    if winners.len() > 1 {
        headline.push(Span::styled(
            "DEAD HEAT: ",
            Style::new().bold().fg(Color::Yellow),
        ));
        for (i, &w) in winners.iter().enumerate() {
            if i > 0 {
                headline.push(Span::raw(" & "));
            }
            headline.push(Span::styled(name(w), lane_style(w).bold()));
        }
    } else {
        headline.push(Span::styled(
            "WINNER: ",
            Style::new().bold().fg(Color::Yellow),
        ));
        headline.push(Span::styled(
            name(winners[0]),
            lane_style(winners[0]).bold(),
        ));
        headline.push(Span::raw(format!(
            " at {}",
            s.card.entries[winners[0]].odds
        )));
    }
    headline.push(Span::styled(" ★", Style::new().fg(Color::Yellow)));
    let margin = match (
        record.placings.first().and_then(|p| p.time),
        record.placings.get(1),
    ) {
        (Some(t1), Some(p2)) if winners.len() == 1 => p2.time.map(|t2| t2 - t1),
        _ => None,
    };
    let sub = match margin {
        Some(m) if m < PHOTO_FINISH_TICKS => {
            format!("Photo finish! Won by {:.2}s", m * SECONDS_PER_TICK)
        }
        Some(m) => format!("Won by {:.2}s", m * SECONDS_PER_TICK),
        None if winners.len() > 1 => {
            "Dead heat rule: stakes on the winners are split between them.".to_string()
        }
        None => String::new(),
    };
    f.render_widget(
        Paragraph::new(vec![
            Line::from(headline),
            Line::styled(sub, Style::new().fg(Color::Gray)),
        ])
        .centered()
        .block(panel(&format!(" Race #{} results ", s.card.number))),
        header,
    );

    let winner_time = record.placings.first().and_then(|p| p.time);
    let rows: Vec<Row> = record
        .placings
        .iter()
        .map(|p| {
            let pick = p.lane == s.bet.lane;
            let gap = match (p.time, winner_time) {
                (Some(t), Some(w)) if p.place > 1 => format!("+{:.2}s", (t - w) * SECONDS_PER_TICK),
                (Some(_), _) if p.tied_with > 1 => "dead heat".to_string(),
                (Some(_), _) => "—".to_string(),
                (None, _) => "DNF".to_string(),
            };
            let row = Row::new(vec![
                Cell::from(format!(
                    "{}{}",
                    if pick { "▶" } else { " " },
                    place_label(p)
                )),
                Cell::from(Span::styled(name(p.lane), lane_style(p.lane).bold())),
                Cell::from(format!("{:>6}", s.card.entries[p.lane].odds)),
                Cell::from(p.time.map(race_time).unwrap_or_else(|| "—".into())),
                Cell::from(gap),
            ]);
            if pick {
                row.style(Style::new().bg(Color::DarkGray).bold())
            } else {
                row
            }
        })
        .collect();
    f.render_widget(
        Table::new(
            rows,
            [
                Constraint::Length(6),
                Constraint::Length(MAX_NAME_CHARS as u16 + 1),
                Constraint::Length(7),
                Constraint::Length(9),
                Constraint::Fill(1),
            ],
        )
        .header(
            Row::new(vec![" Pos", "Horse", "  Odds", "Time", "Gap"])
                .style(Style::new().bold().fg(Color::Yellow)),
        )
        .block(panel(" Placings ")),
        table,
    );

    let o = &s.outcome;
    let bet = &s.bet;
    let pick_name = name(bet.lane);
    let mut lines = vec![Line::from(vec![
        Span::raw("You backed "),
        Span::styled(pick_name, lane_style(bet.lane).bold()),
        Span::raw(format!(
            " with {} at {}.",
            format_money(bet.stake),
            bet.odds
        )),
    ])];
    let profit = i128::from(o.returned) - i128::from(bet.stake);
    if o.won() {
        let how = if o.dead_heat_ways > 1 {
            format!(
                "Dead heat for 1st ({} ways), stake split:",
                o.dead_heat_ways
            )
        } else {
            "WINNER!".to_string()
        };
        lines.push(Line::from(vec![
            Span::styled(
                format!("{how} You collect {}", format_money(o.returned)),
                Style::new().bold().fg(Color::LightGreen),
            ),
            Span::styled(
                format!(
                    " ({} stake back, {} profit)",
                    format_money(bet.stake.min(o.returned)),
                    format_signed_money(profit)
                ),
                Style::new().fg(Color::Gray),
            ),
        ]));
    } else {
        lines.push(Line::styled(
            format!(
                "Finished {}. Stake lost: {}.",
                ordinal(o.place),
                format_signed_money(profit)
            ),
            Style::new().bold().fg(Color::LightRed),
        ));
    }
    lines.push(Line::from(vec![
        Span::styled("Wallet: ", Style::new().fg(Color::Gray)),
        Span::raw(format_money(o.balance_before)),
        Span::styled(" → ", Style::new().fg(Color::Gray)),
        Span::styled(
            format_money(app.data.balance_cents),
            Style::new().bold().fg(if profit >= 0 {
                Color::LightGreen
            } else {
                Color::LightRed
            }),
        ),
    ]));
    let broke = app.data.is_broke();
    if broke {
        lines.push(Line::styled(
            format!(
                "You're broke! Press F for a FRESH START: wallet reset to {} (names and stats kept).",
                format_money(STARTING_BALANCE)
            ),
            Style::new().bold().fg(Color::Black).bg(Color::LightRed),
        ));
    }
    if let Some(n) = notice_line(app) {
        lines.push(n);
    }
    f.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: true })
            .block(panel(" Your bet ")),
        summary,
    );

    let keys: &[(&str, &str)] = if broke {
        &[("F", "FRESH START"), ("M", "menu"), ("Q", "quit")]
    } else {
        &[
            ("R/Enter", "rematch (same pick & stake)"),
            ("M", "menu"),
            ("Q", "quit"),
        ]
    };
    f.render_widget(footer(keys), foot);
}

// ----- rename ------------------------------------------------------------------------------

fn draw_rename(f: &mut Frame, area: Rect, app: &App, s: &RenameState) {
    let [body, foot] = Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(area);
    let boxed = centered(body, 64, 16);
    let block = panel(" Rename Horses ");
    let inner = block.inner(boxed);
    f.render_widget(block, boxed);

    let mut lines = vec![
        Line::styled(
            "Name them after your friends. Changes save instantly.",
            Style::new().fg(Color::Gray),
        ),
        Line::raw(""),
    ];
    for lane in 0..LANES {
        let selected = lane == s.selected;
        let marker = if selected { " ▶ " } else { "   " };
        let mut spans = vec![
            Span::styled(marker, Style::new().fg(Color::Yellow)),
            Span::styled(format!("{}  ", lane + 1), Style::new().fg(Color::Gray)),
        ];
        match (&s.editing, selected) {
            (Some(buffer), true) => {
                let cursor = if blink(app, 8) { "▏" } else { " " };
                spans.push(Span::styled(
                    format!(" {buffer}{cursor} "),
                    Style::new().bold().fg(Color::White).bg(Color::DarkGray),
                ));
                spans.push(Span::styled(
                    format!("  {}/{}", buffer.trim().chars().count(), MAX_NAME_CHARS),
                    Style::new().fg(Color::DarkGray),
                ));
            }
            _ => {
                let mut style = lane_style(lane).bold();
                if selected {
                    style = style.bg(Color::DarkGray);
                }
                spans.push(Span::styled(
                    format!(" {} ", app.data.horse_names[lane]),
                    style,
                ));
            }
        }
        lines.push(Line::from(spans));
    }
    lines.push(Line::raw(""));
    if let Some(err) = &s.error {
        lines.push(Line::styled(
            format!("✗ {err}"),
            Style::new().bold().fg(Color::LightRed),
        ));
    } else if s.editing.is_some() {
        lines.push(Line::styled(
            format!("Up to {MAX_NAME_CHARS} characters. Names must be unique."),
            Style::new().fg(Color::Gray),
        ));
    }
    f.render_widget(Paragraph::new(lines), inner);

    let keys: &[(&str, &str)] = if s.editing.is_some() {
        &[("type", "name"), ("Enter", "save"), ("Esc", "cancel")]
    } else {
        &[
            ("↑↓", "select"),
            ("Enter", "edit"),
            ("D", "default name"),
            ("Esc", "menu"),
        ]
    };
    f.render_widget(footer(keys), foot);
}

// ----- stats -------------------------------------------------------------------------------

fn draw_stats(f: &mut Frame, area: Rect, app: &App) {
    let [body, path, foot] = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(area);
    let [left, right] =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(body);

    let st = &app.data.stats;
    let settled = st.bets_won + st.bets_lost;
    let rate = if settled > 0 {
        format!("{:.0}%", st.bets_won as f64 * 100.0 / settled as f64)
    } else {
        "—".into()
    };
    let net = st.net_cents();
    let kv = |k: &str, v: String, style: Style| {
        Line::from(vec![
            Span::styled(format!("{k:<16}"), Style::new().fg(Color::Gray)),
            Span::styled(v, style),
        ])
    };
    let plain = Style::new().fg(Color::White).bold();
    let plural = |n: u64, word: &str| format!("{n} {word}{}", if n == 1 { "" } else { "s" });
    let lines = vec![
        kv(
            "Wallet",
            format_money(app.data.balance_cents),
            Style::new().bold().fg(Color::LightGreen),
        ),
        kv("Peak wallet", format_money(st.peak_balance_cents), plain),
        kv("Races run", st.races_run.to_string(), plain),
        kv(
            "Bets won / lost",
            format!("{} / {}  ({rate})", st.bets_won, st.bets_lost),
            plain,
        ),
        kv("Total wagered", format_money(st.total_wagered_cents), plain),
        kv(
            "Total returned",
            format_money(st.total_returned_cents),
            plain,
        ),
        kv(
            "Net profit",
            format_signed_money(net),
            Style::new().bold().fg(if net >= 0 {
                Color::LightGreen
            } else {
                Color::LightRed
            }),
        ),
        kv(
            "Biggest return",
            format_money(st.biggest_return_cents),
            plain,
        ),
        kv(
            "Win streak",
            format!("{} (best {})", st.current_streak, st.best_streak),
            plain,
        ),
        kv("Fresh starts", plural(st.fresh_starts, "bailout"), plain),
    ];
    f.render_widget(
        Paragraph::new(lines).block(panel(" Your betting career ")),
        left,
    );

    let block = panel(" Hall of fame ");
    let inner = block.inner(right);
    f.render_widget(block, right);
    let most = st.horse_wins.iter().copied().max().unwrap_or(0).max(1);
    let longest = app
        .data
        .horse_names
        .iter()
        .map(|n| n.chars().count())
        .max()
        .unwrap_or(0);
    let name_w = longest.min(usize::from(inner.width / 2)).max(4);
    let digits = st
        .horse_wins
        .iter()
        .copied()
        .max()
        .unwrap_or(0)
        .to_string()
        .len();
    let bar_w = usize::from(inner.width)
        .saturating_sub(name_w + digits + 3)
        .max(1);
    let mut lines = vec![
        Line::styled("Race wins by horse", Style::new().fg(Color::Gray)),
        Line::raw(""),
    ];
    for lane in 0..LANES {
        let wins = st.horse_wins.get(lane).copied().unwrap_or(0);
        let len = (wins as usize * bar_w).div_ceil(most as usize);
        let name: String = app.data.horse_names[lane].chars().take(name_w).collect();
        lines.push(Line::from(vec![
            Span::styled(format!("{name:<name_w$} "), lane_style(lane).bold()),
            Span::styled("█".repeat(len), lane_style(lane)),
            Span::styled(format!(" {wins}"), Style::new().fg(Color::White)),
        ]));
    }
    f.render_widget(Paragraph::new(lines), inner);

    f.render_widget(
        Paragraph::new(Line::styled(
            format!("Save file: {}", app.save_path().display()),
            Style::new().fg(Color::DarkGray),
        ))
        .centered(),
        path,
    );
    f.render_widget(footer(&[("Esc", "back"), ("Q", "quit")]), foot);
}

// ----- broke -------------------------------------------------------------------------------

fn draw_broke(f: &mut Frame, area: Rect, app: &App) {
    let boxed = centered(area, 62, 13);
    let block = Block::bordered()
        .border_type(BorderType::Double)
        .border_style(Style::new().fg(Color::LightRed))
        .title(Span::styled(
            " BANKRUPT ",
            Style::new().bold().fg(Color::LightRed),
        ));
    let lines = vec![
        Line::raw(""),
        Line::styled("B R O K E", Style::new().bold().fg(Color::LightRed)),
        Line::raw(""),
        Line::raw(format!(
            "Your wallet holds {}. The minimum bet is {}.",
            format_money(app.data.balance_cents),
            format_money(MIN_WAGER)
        )),
        Line::styled(
            "The horses thank you for your generous donations.",
            Style::new().fg(Color::Gray).italic(),
        ),
        Line::raw(""),
        Line::from(vec![
            Span::styled(
                " F ",
                Style::new().bold().fg(Color::Black).bg(Color::Yellow),
            ),
            Span::styled(
                format!(
                    "  FRESH START: reset wallet to {}",
                    format_money(STARTING_BALANCE)
                ),
                Style::new().bold().fg(Color::Yellow),
            ),
        ]),
        Line::styled(
            "(horse names and stats are kept)",
            Style::new().fg(Color::Gray),
        ),
        Line::raw(""),
        Line::styled("Esc  back to menu", Style::new().fg(Color::Gray)),
    ];
    f.render_widget(Paragraph::new(lines).centered().block(block), boxed);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{App, RenameState, Screen};
    use crate::save::SaveStore;
    use ratatui::{backend::TestBackend, Terminal};

    fn render(app: &App, w: u16, h: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        terminal.draw(|f| draw(f, app)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let mut out = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    fn app() -> App {
        let path = crate::save::tests::temp_save_path("ui");
        App::new(SaveStore::new(path), Some(2024))
    }

    #[test]
    fn every_screen_renders_at_minimum_and_large_sizes() {
        let mut app = app();
        let mut screens = vec![Screen::Title, Screen::Menu, Screen::Stats, Screen::Broke];
        screens.push(Screen::Rename(RenameState::default()));
        screens.push(Screen::Rename(RenameState {
            selected: 2,
            editing: Some("Dave".into()),
            error: Some("nope".into()),
        }));
        for screen in screens {
            app.screen = screen;
            for (w, h) in [(MIN_WIDTH, MIN_HEIGHT), (80, 24), (200, 60)] {
                render(&app, w, h);
            }
        }
        // Betting, every frame of a race, and results.
        app.open_betting();
        for (w, h) in [(MIN_WIDTH, MIN_HEIGHT), (120, 40)] {
            let text = render(&app, w, h);
            assert!(
                text.contains("Decimal odds"),
                "odds explanation visible at {w}x{h}"
            );
        }
        let Screen::Betting(bet) = &app.screen else {
            panic!()
        };
        let card = bet.card.clone();
        app.start_race(card, 0, 100);
        let frames = match &app.screen {
            Screen::Racing(r) => r.record.frames.len(),
            _ => panic!(),
        };
        for i in 0..frames {
            if let Screen::Racing(r) = &mut app.screen {
                r.frame = i;
            }
            render(&app, MIN_WIDTH, MIN_HEIGHT);
            render(&app, 160, 30);
        }
        for _ in 0..100 {
            app.on_tick();
        }
        assert!(matches!(app.screen, Screen::Results(_)));
        render(&app, MIN_WIDTH, MIN_HEIGHT);
        let _ = std::fs::remove_dir_all(app.save_path().parent().unwrap());
    }

    #[test]
    fn ellipsize_cuts_cleanly() {
        assert_eq!(ellipsize("Quick, rock steady", 40), "Quick, rock steady");
        assert_eq!(ellipsize("Quick, rock steady", 8), "Quick…");
        assert_eq!(ellipsize("Quick, rock steady", 1), "…");
        assert_eq!(ellipsize("Quick", 0), "");
    }

    #[test]
    fn small_terminal_shows_help_message() {
        let app = app();
        for (w, h) in [(MIN_WIDTH - 1, 40), (100, MIN_HEIGHT - 1), (20, 5), (1, 1)] {
            let text = render(&app, w, h);
            if w >= 20 {
                assert!(text.contains("too small"), "{w}x{h}:\n{text}");
            }
        }
    }

    #[test]
    fn finish_line_is_visible_on_track() {
        let mut app = app();
        app.open_betting();
        let Screen::Betting(bet) = &app.screen else {
            panic!()
        };
        let card = bet.card.clone();
        app.start_race(card, 2, 100);
        let text = render(&app, 80, 24);
        assert!(text.contains("FINISH"));
        assert!(text.contains("◀ BET"));
        let _ = std::fs::remove_dir_all(app.save_path().parent().unwrap());
    }

    #[test]
    #[ignore = "prints screens for manual inspection"]
    fn print_screens() {
        let mut app = app();
        println!("{}", render(&app, 80, 24));
        app.screen = Screen::Menu;
        println!("{}", render(&app, 80, 24));
        app.open_betting();
        println!("{}", render(&app, 80, 24));
        let Screen::Betting(bet) = &app.screen else {
            panic!()
        };
        let card = bet.card.clone();
        app.start_race(card, 1, 5_000);
        for _ in 0..90 {
            app.on_tick();
        }
        println!("{}", render(&app, 80, 24));
        for _ in 0..200 {
            app.on_tick();
            if let Screen::Racing(r) = &app.screen {
                if r.finished() {
                    break;
                }
            }
        }
        println!("{}", render(&app, 80, 24));
        for _ in 0..100 {
            app.on_tick();
        }
        println!("{}", render(&app, 80, 24));
        app.screen = Screen::Stats;
        println!("{}", render(&app, 80, 24));
        app.screen = Screen::Broke;
        println!("{}", render(&app, 70, 22));
        let _ = std::fs::remove_dir_all(app.save_path().parent().unwrap());
    }
}
