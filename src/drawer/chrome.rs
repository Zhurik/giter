use crate::drawer::app::{App, Filter, Input, NoteKind, Screen};
use crate::drawer::theme::Theme;
use crate::git::remote::{Failure, Mode, Unresolved, Verdict};
use crate::text::ellipsize;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, Padding, Paragraph};

/// Below this the interface says so instead of drawing something unreadable.
pub const MIN_WIDTH: u16 = 60;
pub const MIN_HEIGHT: u16 = 15;
/// Below this the panes collapse into a single list.
pub const WIDE_WIDTH: u16 = 100;

const PROGRESS_WIDTH: usize = 20;
/// Columns of air kept between any text and the edge it sits against.
pub const PAD: u16 = 1;
const SEPARATOR: &str = "│";

/// `giter │ PROD kube-prod │ mode commit tag` and, on the right, the tally and how fresh
/// the numbers are.
pub fn header(app: &App, theme: &Theme, width: u16) -> Paragraph<'static> {
    let width = width.saturating_sub(2 * PAD);
    let mut left = vec![
        Span::styled("giter", Style::new().fg(theme.cyan).bold()),
        Span::styled(format!(" {} ", SEPARATOR), Style::new().fg(theme.faint)),
    ];

    left.extend(cluster_spans(app, theme));
    left.push(Span::styled(
        format!(" {} ", SEPARATOR),
        Style::new().fg(theme.faint),
    ));
    left.push(Span::styled("mode ", Style::new().fg(theme.dim)));
    left.extend(mode_spans(app.mode(), theme));

    let right = match app.screen() {
        Screen::Overview => {
            let mut spans = tally_spans(app, theme);
            spans.extend(freshness_spans(app, theme));
            spans
        }
        Screen::Detail => {
            let mut spans = vec![
                Span::styled("overview ", Style::new().fg(theme.dim)),
                Span::styled("esc", Style::new().fg(theme.fg2)),
            ];
            spans.extend(freshness_spans(app, theme));
            spans
        }
    };

    padded(spread(left, right, width), theme.env_bg)
}

/// The same information for a terminal too narrow to spell it out.
pub fn narrow_header(app: &App, theme: &Theme, width: u16) -> Paragraph<'static> {
    let width = width.saturating_sub(2 * PAD);
    let left = cluster_spans(app, theme);
    let right = vec![Span::styled(
        app.mode().label().to_string(),
        Style::new().fg(theme.cyan),
    )];

    padded(spread(left, right, width), theme.env_bg)
}

/// The bar under the header: the search prompt while typing, the start-up progress while
/// the cluster answers, and the filter and sort controls the rest of the time.
pub fn bar(app: &App, theme: &Theme, width: u16) -> Paragraph<'static> {
    let width = width.saturating_sub(2 * PAD);
    let line = match (app.input(), app.listing()) {
        (Input::Search, _) => search_bar(app, theme, width),
        (_, Some((done, total))) => startup_bar(app, theme, width, done, total),
        _ => filter_bar(app, theme, width),
    };

    padded(line, theme.bar)
}

/// Counts alone, for the narrow layout that has no room for controls.
pub fn narrow_bar(app: &App, theme: &Theme, width: u16) -> Paragraph<'static> {
    let width = width.saturating_sub(2 * PAD);
    let right = match app.listing() {
        Some((done, total)) => vec![Span::styled(
            format!("{}/{}", done, total),
            Style::new().fg(theme.fg2),
        )],
        None => vec![Span::styled(app.freshness(), Style::new().fg(theme.dim))],
    };

    padded(spread(tally_spans(app, theme), right, width), theme.bar)
}

/// The upper footer line: what just happened, what is being fetched, or the tally.
pub fn status_line(app: &App, theme: &Theme, width: u16, narrow: bool) -> Paragraph<'static> {
    let width = width.saturating_sub(2 * PAD);
    let line = match app.note() {
        Some(note) => {
            let (glyph, color) = match note.kind {
                NoteKind::Ok => ('✓', theme.ok),
                NoteKind::Warn => ('!', theme.warn),
                NoteKind::Error => ('✗', theme.bad),
            };
            let hint = match (note.retry, note.standing) {
                (true, true) => "r retry",
                (true, false) => "r retry · esc dismiss",
                (false, true) => "",
                (false, false) => "esc dismiss",
            };

            spread(
                vec![
                    Span::styled(format!("{} ", glyph), Style::new().fg(color)),
                    Span::styled(
                        ellipsize(&note.text, (width as usize).saturating_sub(hint.len() + 4)),
                        Style::new().fg(theme.fg2),
                    ),
                ],
                vec![Span::styled(hint.to_string(), Style::new().fg(theme.faint))],
                width,
            )
        }
        None => match app.screen() {
            Screen::Detail => match app.selection_summary() {
                Some(summary) => spread(
                    vec![Span::styled(
                        ellipsize(&summary, (width as usize).saturating_sub(4)),
                        Style::new().fg(theme.fg2),
                    )],
                    match narrow {
                        true => vec![],
                        false => vec![Span::styled(
                            "enter opens the deployed commit in GitLab".to_string(),
                            Style::new().fg(theme.faint),
                        )],
                    },
                    width,
                ),
                None => summary_line(app, theme, width),
            },
            // the narrow overview has no NOTE column, so the note of the selected row is
            // what this line is for
            Screen::Overview if narrow => match selected_note(app, theme) {
                Some(line) => line,
                None => summary_line(app, theme, width),
            },
            Screen::Overview => summary_line(app, theme, width),
        },
    };

    padded(line, theme.bar)
}

/// The lower footer line: only keys, and only the ones that do something here.
pub fn keys_line(app: &App, theme: &Theme, width: u16, narrow: bool) -> Paragraph<'static> {
    let width = width.saturating_sub(2 * PAD);
    let keys: Vec<(&str, &str)> = match (app.input(), app.screen(), narrow) {
        (Input::Search, _, _) => vec![("enter", "keep"), ("esc", "clear"), ("↑↓", "move")],
        (_, Screen::Overview, true) => vec![
            ("↑↓", "move"),
            ("enter", "in"),
            ("/", "find"),
            ("?", "keys"),
            ("q", "quit"),
        ],
        (_, Screen::Detail, true) => vec![
            ("↑↓", "pods"),
            ("tab", "containers"),
            ("enter", "commit"),
            ("esc", "back"),
            ("?", "keys"),
        ],
        (_, Screen::Overview, false) => vec![
            ("↑↓", "move"),
            ("enter", "open namespace"),
            ("/", "search"),
            ("f", "filter"),
            ("s", "sort"),
            ("m", "mode"),
            ("r", "refresh"),
            ("?", "help"),
            ("q", "quit"),
        ],
        (_, Screen::Detail, false) => vec![
            ("↑↓", "pods"),
            ("←→", "pane"),
            ("tab", "containers"),
            ("enter", "open commit"),
            ("m", "mode"),
            ("esc", "overview"),
            ("?", "help"),
        ],
    };

    let mut left: Vec<Span> = vec![];
    for (key, action) in keys {
        left.push(Span::styled(
            key.to_string(),
            Style::new().fg(theme.fg2).bold(),
        ));
        left.push(Span::styled(
            format!(" {}  ", action),
            Style::new().fg(theme.dim),
        ));
    }

    let right = vec![Span::styled(position(app), Style::new().fg(theme.dim))];

    padded(spread(left, right, width), theme.keys_bar)
}

/// Full key map and the legend of glyphs, over the top of everything else.
pub fn help(frame: &mut Frame, area: Rect, theme: &Theme) {
    let keys: [(&str, &str); 13] = [
        ("↑↓ k j", "move"),
        ("pgup pgdn", "jump ten rows"),
        ("home end", "first / last"),
        ("←→ tab", "switch pane"),
        ("enter", "open namespace · open commit in GitLab"),
        ("esc", "back · dismiss · quit"),
        ("/", "search namespaces"),
        ("f", "filter: all / problems / unknown"),
        ("s", "sort: status / name / pods"),
        ("m", "commit ⇄ tag mode"),
        ("r", "re-resolve the selected namespace"),
        ("R", "re-list pods from Kubernetes"),
        ("q", "quit"),
    ];

    let legend: [(Verdict, &str); 12] = [
        (Verdict::InSync, "deployed commit equals the reference"),
        (
            Verdict::Behind,
            "deployed commit differs from the reference",
        ),
        (
            Verdict::Failed(Failure::Kube),
            "cannot list pods — no access or no such namespace",
        ),
        (
            Verdict::Failed(Failure::Git),
            "ls-remote failed — the reason is in the row",
        ),
        (Verdict::Resolving, "the reference is being fetched"),
        (
            Verdict::Unknown(Unresolved::Sidecar),
            "foreign image — nothing to compare",
        ),
        (
            Verdict::Unknown(Unresolved::Digest),
            "pinned by digest, the tag carries no commit",
        ),
        (
            Verdict::Unknown(Unresolved::NoConfig),
            "namespace has no entry in repos.json",
        ),
        (
            Verdict::Unknown(Unresolved::NoTags),
            "tag mode, but the repository has no tags",
        ),
        (
            Verdict::Unknown(Unresolved::WrongMode),
            "a release image, and commit mode cannot resolve one",
        ),
        (
            Verdict::Unknown(Unresolved::Finished),
            "a pod that has run its course — a CronJob leaves those behind",
        ),
        (
            Verdict::Unknown(Unresolved::Empty),
            "no pods, so nothing to compare",
        ),
    ];

    let width = area.width.min(74);
    let height = area.height.min((keys.len() + legend.len() + 6) as u16);
    let popup = Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    };

    let mut lines: Vec<Line> = vec![Line::from(vec![
        Span::styled("Keys", Style::new().fg(theme.cyan).bold()),
        Span::styled("   esc / ? close", Style::new().fg(theme.faint)),
    ])];

    for (key, action) in keys {
        lines.push(Line::from(vec![
            Span::styled(format!("{:<11}", key), Style::new().fg(theme.cyan)),
            Span::styled(action.to_string(), Style::new().fg(theme.dim)),
        ]));
    }

    lines.push(Line::default());
    lines.push(Line::from(Span::styled(
        "Legend",
        Style::new().fg(theme.cyan).bold(),
    )));

    for (verdict, meaning) in legend {
        lines.push(Line::from(vec![
            Span::styled(
                format!("{}  ", verdict.glyph()),
                Style::new().fg(theme.verdict(verdict)),
            ),
            Span::styled(format!("{:<9}", verdict.code()), Style::new().fg(theme.fg2)),
            Span::styled(meaning.to_string(), Style::new().fg(theme.dim)),
        ]));
    }

    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::new()
                    .borders(Borders::ALL)
                    .border_style(Style::new().fg(theme.cyan))
                    .padding(Padding::horizontal(1)),
            )
            .style(Style::new().bg(theme.bar)),
        popup,
    );
}

pub fn too_small(frame: &mut Frame, area: Rect, theme: &Theme) {
    let text = format!(
        "terminal too small: {}×{}, need {}×{}",
        area.width, area.height, MIN_WIDTH, MIN_HEIGHT
    );

    frame.render_widget(
        Paragraph::new(ellipsize(&text, area.width as usize))
            .style(Style::new().fg(theme.warn).bg(theme.bg)),
        area,
    );
}

/// A one-line bar: the background reaches the edges of the screen, the text does not.
fn padded(line: Line<'static>, background: Color) -> Paragraph<'static> {
    Paragraph::new(line)
        .block(Block::new().padding(Padding::horizontal(PAD)))
        .style(Style::new().bg(background))
}

/// Left-aligned and right-aligned spans on one line, whatever the width.
pub fn spread(left: Vec<Span<'static>>, right: Vec<Span<'static>>, width: u16) -> Line<'static> {
    let used: usize = left.iter().chain(right.iter()).map(Span::width).sum();
    let gap = (width as usize).saturating_sub(used).max(1);

    let mut spans = left;
    spans.push(Span::raw(" ".repeat(gap)));
    spans.extend(right);

    Line::from(spans)
}

pub fn glyph(verdict: Verdict, theme: &Theme, spinner: char) -> Span<'static> {
    let symbol = match verdict {
        Verdict::Resolving => spinner,
        other => other.glyph(),
    };

    let style = Style::new().fg(theme.verdict(verdict));

    Span::styled(
        format!("{} ", symbol),
        match verdict {
            Verdict::Behind => style.bold(),
            _ => style,
        },
    )
}

/// How fresh the numbers are — or, before anything has come back, that the tool is starting.
fn freshness_spans(app: &App, theme: &Theme) -> Vec<Span<'static>> {
    let freshness = app.freshness();
    let text = match freshness.is_empty() {
        true => "starting up".to_string(),
        false => freshness,
    };

    vec![
        Span::styled(format!(" {} ", SEPARATOR), Style::new().fg(theme.faint)),
        Span::styled(text, Style::new().fg(theme.dim)),
    ]
}

fn cluster_spans(app: &App, theme: &Theme) -> Vec<Span<'static>> {
    let mut spans = vec![];

    if let Some(env) = app.cluster.env {
        spans.push(Span::styled(
            format!(" {} ", env.label()),
            Style::new().fg(theme.env_fg).bg(theme.env).bold(),
        ));
        spans.push(Span::raw(" "));
    }

    spans.push(Span::styled(
        app.cluster
            .context
            .clone()
            .unwrap_or_else(|| "no kubeconfig context".to_string()),
        Style::new().fg(theme.env),
    ));

    spans
}

fn mode_spans(mode: Mode, theme: &Theme) -> Vec<Span<'static>> {
    [Mode::Commit, Mode::Tag]
        .into_iter()
        .map(|candidate| {
            let label = format!(" {} ", candidate.label());

            match candidate == mode {
                true => Span::styled(label, Style::new().fg(theme.bg).bg(theme.cyan).bold()),
                false => Span::styled(label, Style::new().fg(theme.faint)),
            }
        })
        .collect()
}

fn tally_spans(app: &App, theme: &Theme) -> Vec<Span<'static>> {
    let counts = app.counts();

    vec![
        Span::styled(format!("● {}  ", counts.in_sync), Style::new().fg(theme.ok)),
        Span::styled(
            format!("▼ {}  ", counts.behind),
            Style::new().fg(theme.bad).bold(),
        ),
        Span::styled(
            format!("✗ {}  ", counts.failed),
            Style::new().fg(theme.warn),
        ),
        Span::styled(format!("○ {}", counts.unknown), Style::new().fg(theme.dim)),
    ]
}

/// What the overview would have said in its NOTE column about the row under the cursor.
fn selected_note(app: &App, theme: &Theme) -> Option<Line<'static>> {
    let row = app.selected_row()?;
    if row.note.is_empty() {
        return None;
    }

    Some(Line::from(vec![
        glyph(row.verdict, theme, app.spinner()),
        Span::styled(row.note, Style::new().fg(theme.fg2)),
    ]))
}

fn summary_line(app: &App, theme: &Theme, width: u16) -> Line<'static> {
    let counts = app.counts();
    let resolving = app.resolving();

    if resolving > 0 {
        return spread(
            vec![
                Span::styled(format!("{} ", app.spinner()), Style::new().fg(theme.cyan)),
                Span::styled(
                    "resolving references ".to_string(),
                    Style::new().fg(theme.fg2),
                ),
            ]
            .into_iter()
            .chain(progress_spans(
                app.total_namespaces() - resolving,
                app.total_namespaces(),
                theme,
            ))
            .chain([Span::styled(
                format!(
                    " {} of {}",
                    app.total_namespaces() - resolving,
                    app.total_namespaces()
                ),
                Style::new().fg(theme.fg2),
            )])
            .collect(),
            vec![Span::styled(
                "background · the interface stays responsive".to_string(),
                Style::new().fg(theme.dim),
            )],
            width,
        );
    }

    spread(
        vec![
            Span::styled(
                format!("● {} in sync", counts.in_sync),
                Style::new().fg(theme.ok),
            ),
            Span::styled(" · ".to_string(), Style::new().fg(theme.faint)),
            Span::styled(
                format!("▼ {} behind", counts.behind),
                Style::new().fg(theme.bad),
            ),
            Span::styled(" · ".to_string(), Style::new().fg(theme.faint)),
            Span::styled(
                format!("✗ {} failed", counts.failed),
                Style::new().fg(theme.warn),
            ),
            Span::styled(" · ".to_string(), Style::new().fg(theme.faint)),
            Span::styled(
                format!("○ {} unknown", counts.unknown),
                Style::new().fg(theme.dim),
            ),
        ],
        vec![Span::styled(
            format!("idle · {}", app.freshness()),
            Style::new().fg(theme.dim),
        )],
        width,
    )
}

fn search_bar(app: &App, theme: &Theme, width: u16) -> Line<'static> {
    spread(
        vec![
            Span::styled("/ ".to_string(), Style::new().fg(theme.cyan).bold()),
            Span::styled(app.query().to_string(), Style::new().fg(theme.fg)),
            Span::styled("▏".to_string(), Style::new().fg(theme.cyan)),
        ],
        vec![Span::styled(
            format!(
                "{} of {}",
                app.matching_namespaces(),
                app.total_namespaces()
            ),
            Style::new().fg(theme.dim),
        )],
        width,
    )
}

fn startup_bar(app: &App, theme: &Theme, width: u16, done: usize, total: usize) -> Line<'static> {
    let mut left = vec![
        Span::styled(format!("{} ", app.spinner()), Style::new().fg(theme.cyan)),
        Span::styled("listing pods ".to_string(), Style::new().fg(theme.fg2)),
    ];
    left.extend(progress_spans(done, total, theme));
    left.push(Span::styled(
        format!(" {}/{}", done, total),
        Style::new().fg(theme.fg2),
    ));

    spread(
        left,
        vec![Span::styled(
            format!("resolving git references · {} left", app.resolving()),
            Style::new().fg(theme.dim),
        )],
        width,
    )
}

fn filter_bar(app: &App, theme: &Theme, width: u16) -> Line<'static> {
    let counts = app.counts();
    let chips = [
        (Filter::All, app.total_namespaces()),
        (Filter::Problems, counts.behind + counts.failed),
        (Filter::Unknown, counts.unknown),
    ];

    let mut left = vec![Span::styled(
        "filter ".to_string(),
        Style::new().fg(theme.dim),
    )];

    for (filter, count) in chips {
        let label = format!(" {} {} ", filter.label(), count);

        left.push(match filter == app.filter() {
            true => Span::styled(label, Style::new().fg(theme.fg).bg(theme.selection)),
            false => Span::styled(label, Style::new().fg(theme.dim)),
        });
    }

    left.push(Span::styled(
        " f ".to_string(),
        Style::new().fg(theme.faint),
    ));
    left.push(Span::styled(
        format!("{}  ", SEPARATOR),
        Style::new().fg(theme.faint),
    ));
    left.push(Span::styled(
        "sort ".to_string(),
        Style::new().fg(theme.dim),
    ));
    left.push(Span::styled(
        format!("{} ▾", app.sort().label()),
        Style::new().fg(theme.fg2),
    ));

    spread(
        left,
        vec![Span::styled(
            format!(
                "{} namespaces · {} pods · {} containers",
                app.total_namespaces(),
                app.total_pods(),
                app.total_containers()
            ),
            Style::new().fg(theme.dim),
        )],
        width,
    )
}

fn progress_spans(done: usize, total: usize, theme: &Theme) -> Vec<Span<'static>> {
    let filled = match total {
        0 => PROGRESS_WIDTH,
        total => done * PROGRESS_WIDTH / total,
    };

    vec![
        Span::styled("█".repeat(filled), Style::new().fg(theme.ok)),
        Span::styled(
            "█".repeat(PROGRESS_WIDTH - filled),
            Style::new().fg(theme.line),
        ),
    ]
}

fn position(app: &App) -> String {
    let (label, index, total) = match app.screen() {
        Screen::Overview => ("", app.ns_position(), app.matching_namespaces()),
        Screen::Detail => ("pod ", app.pod_position(), app.pod_rows().len()),
    };

    match (index, total) {
        (_, 0) => "0/0".to_string(),
        (Some(index), total) => format!("{}{}/{}", label, index + 1, total),
        (None, total) => format!("{}-/{}", label, total),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use speculoos::prelude::*;

    fn text(content: &str) -> Vec<Span<'static>> {
        vec![Span::raw(content.to_string())]
    }

    #[test]
    fn a_line_of_two_halves_fills_the_width_it_is_given() {
        let width = 40;
        let line = spread(text("● 13 in sync"), text("idle"), width);

        assert_that!(line.width()).is_equal_to(width as usize);
    }

    #[test]
    fn halves_that_do_not_fit_are_still_kept_apart() {
        let line = spread(text("cannot list pods: forbidden"), text("r retry"), 20);

        asserting!("running the two together would read as one sentence")
            .that(&line.spans.iter().any(|span| span.content.trim().is_empty()))
            .is_true();
    }

    #[test]
    fn progress_of_nothing_does_not_divide_by_zero() {
        let bar = progress_spans(0, 0, &Theme::new(None, false));

        assert_that!(bar.iter().map(Span::width).sum::<usize>()).is_equal_to(PROGRESS_WIDTH);
    }

    #[test]
    fn progress_bar_keeps_its_width_as_it_fills() {
        let bar = progress_spans(7, 19, &Theme::new(None, false));

        assert_that!(bar.iter().map(Span::width).sum::<usize>()).is_equal_to(PROGRESS_WIDTH);
    }
}
