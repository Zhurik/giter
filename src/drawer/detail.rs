use crate::drawer::app::{App, ContainerRow, MiniRow, Pane, PodRow, RefKind};
use crate::drawer::chrome::{glyph, spread, PAD};
use crate::drawer::overview::{matched_spans, name_color, name_spans, scrollbar, split_name};
use crate::drawer::theme::Theme;
use crate::git::remote::Verdict;
use crate::k8s::pods::is_calm;
use crate::text::{ellipsize, fit_head, split_pod_name};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Cell, HighlightSpacing, Paragraph, Row, Table};

const MINI_WIDTH: u16 = 38;
const MINI_PODS: u16 = 3;
/// One column of the cursor, plus the single space `Table` keeps between columns.
const CURSOR: u16 = 1;
const GAP: u16 = 1;
/// What the mini list leaves for a name: its width less the border, the padding either
/// side, the scrollbar, the cursor, the glyph, the pod count and the gaps between them.
const MINI_NAME: u16 = MINI_WIDTH - 1 - 2 * PAD - SCROLLBAR - CURSOR - GLYPH - MINI_PODS - 2 * GAP;
/// The bar itself plus a column of air, so the pod counts do not run into it.
const SCROLLBAR: u16 = 1 + GAP;
const SUMMARY_HEIGHT: u16 = 4;
const GLYPH: u16 = 2;
const COMMIT: u16 = 11;
const STATUS: u16 = 18;
const RESTARTS: u16 = 4;
const AGE: u16 = 5;
const CONTAINERS: u16 = 5;
const CONTAINER_ROWS: usize = 4;
const CONTAINER_NAME: u16 = 30;
const CONTAINER_TAG: u16 = 22;

/// One namespace taken apart: its pods, and the containers of the selected pod.
pub fn draw(frame: &mut Frame, area: Rect, app: &mut App, theme: &Theme, wide: bool) {
    let work = match wide {
        true => {
            let [mini, work] =
                Layout::horizontal([Constraint::Length(MINI_WIDTH), Constraint::Fill(1)])
                    .areas(area);
            draw_namespaces(frame, mini, app, theme);
            work
        }
        false => area,
    };
    let work = work.inner(Margin::new(PAD, 0));

    let containers = app.container_rows();
    let height = 2 + containers.len().clamp(1, CONTAINER_ROWS) as u16;

    let [summary, pods, list] = Layout::vertical([
        Constraint::Length(SUMMARY_HEIGHT),
        Constraint::Fill(1),
        Constraint::Length(height),
    ])
    .areas(work);

    draw_summary(frame, summary, app, theme);
    draw_pods(frame, pods, app, theme, wide);
    draw_containers(frame, list, app, theme, containers);
}

/// The narrow list of namespaces, grouped by family so that the eye can jump between
/// products rather than scanning one long alphabet.
fn draw_namespaces(frame: &mut Frame, area: Rect, app: &mut App, theme: &Theme) {
    let focused = app.pane() == Pane::Namespaces;
    let rows = app.mini_rows();
    let selected = app.mini_selected();
    let query = app.query().to_string();
    let spinner = app.spinner();
    let position = format!(
        "{}/{}",
        app.ns_position().map(|i| i + 1).unwrap_or(0),
        app.matching_namespaces()
    );

    let block = Block::new()
        .borders(Borders::RIGHT)
        .border_style(Style::new().fg(match focused {
            true => theme.cyan,
            false => theme.line,
        }));
    let inner = block.inner(area).inner(Margin::new(PAD, 0));
    frame.render_widget(block, area);

    let [head, rest] = Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(inner);
    let [body, _, scroll] = Layout::horizontal([
        Constraint::Fill(1),
        Constraint::Length(GAP),
        Constraint::Length(1),
    ])
    .areas(rest);

    frame.render_widget(
        Paragraph::new(spread(
            vec![Span::styled(
                "NAMESPACES".to_string(),
                Style::new().fg(pane_title(focused, theme)),
            )],
            vec![Span::styled(position, Style::new().fg(theme.faint))],
            head.width,
        )),
        head,
    );

    let widths = [
        Constraint::Length(GLYPH),
        Constraint::Fill(1),
        Constraint::Length(MINI_PODS),
    ];

    let body_rows: Vec<Row> = rows
        .iter()
        .map(|row| match row {
            MiniRow::Family(family) => Row::new(vec![
                Cell::from(""),
                Cell::from(family_heading(family)).style(Style::new().fg(theme.faint)),
                Cell::from(""),
            ]),
            MiniRow::Namespace(ns) => Row::new(vec![
                Cell::from(Line::from(glyph(ns.verdict, theme, spinner))),
                Cell::from(Line::from(matched_spans(
                    &ellipsize(split_name(ns).1, MINI_NAME as usize),
                    &query,
                    theme,
                    name_color(ns, theme),
                ))),
                Cell::from(
                    Text::from(match ns.pods {
                        Some(count) => count.to_string(),
                        None => "—".to_string(),
                    })
                    .right_aligned(),
                )
                .style(Style::new().fg(theme.faint)),
            ]),
        })
        .collect();

    let table = Table::new(body_rows, widths)
        .row_highlight_style(Style::new().bg(selection(focused, theme)))
        .highlight_symbol(Span::styled("▌", Style::new().fg(theme.cyan)))
        .highlight_spacing(HighlightSpacing::Always);

    let state = app.ns_state();
    state.select(selected);
    frame.render_stateful_widget(table, body, state);

    scrollbar(frame, scroll, theme, rows.len(), app.ns_state().offset());
}

/// The heading spells out the prefix its rows leave out, and a rule fills the rest of the
/// column.
fn family_heading(family: &str) -> String {
    let rule = (MINI_NAME as usize).saturating_sub(family.chars().count() + 1);

    format!("{} {}", family, "─".repeat(rule))
}

/// What the namespace is, where its code lives, and what its pods are measured against.
fn draw_summary(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let Some(row) = app.selected_row() else {
        return;
    };
    let (reference, explanation) = app.reference_detail();

    let block = Block::new()
        .borders(Borders::BOTTOM)
        .border_style(Style::new().fg(theme.line));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut title = vec![glyph(row.verdict, theme, app.spinner())];
    title.extend(name_spans(&row, "", theme, (inner.width / 2) as usize));

    let room = |taken: usize| (inner.width as usize).saturating_sub(taken + 4);
    let hint = "r to re-resolve";

    let lines = vec![
        spread(
            title,
            vec![Span::styled(
                ellipsize(&row.note, room(row.name.chars().count() + GLYPH as usize)),
                Style::new().fg(theme.verdict(row.verdict)),
            )],
            inner.width,
        ),
        Line::from(vec![
            Span::styled("repo      ".to_string(), Style::new().fg(theme.dim)),
            Span::styled(
                app.repo_url(&row.name)
                    .unwrap_or("missing in repos.json")
                    .to_string(),
                Style::new().fg(theme.fg2),
            ),
        ]),
        spread(
            vec![
                Span::styled("reference ".to_string(), Style::new().fg(theme.dim)),
                Span::styled(reference.clone(), Style::new().fg(theme.fg)),
                Span::styled(
                    format!(
                        "  {}",
                        ellipsize(
                            &explanation,
                            room(10 + reference.chars().count() + hint.len())
                        )
                    ),
                    Style::new().fg(theme.dim),
                ),
            ],
            vec![Span::styled(hint.to_string(), Style::new().fg(theme.faint))],
            inner.width,
        ),
    ];

    frame.render_widget(Paragraph::new(lines), inner);
}

fn draw_pods(frame: &mut Frame, area: Rect, app: &mut App, theme: &Theme, wide: bool) {
    let focused = app.pane() == Pane::Pods;
    let rows = app.pod_rows();
    let selected = app.pod_position();
    let spinner = app.spinner();

    let reference_title = match app.selected_kind() {
        RefKind::Commits => "COMMIT",
        RefKind::Tags => "TAG",
        RefKind::Mixed => "DEPLOYED",
    };

    let name_width = match wide {
        true => area.width.saturating_sub(
            GLYPH + COMMIT + STATUS + RESTARTS + AGE + CONTAINERS + 6 * GAP + CURSOR + SCROLLBAR,
        ),
        false => area
            .width
            .saturating_sub(GLYPH + COMMIT + CONTAINERS + 3 * GAP + CURSOR + SCROLLBAR),
    } as usize;

    let (widths, header) = match wide {
        true => (
            vec![
                Constraint::Length(GLYPH),
                Constraint::Fill(1),
                Constraint::Length(COMMIT),
                Constraint::Length(STATUS),
                Constraint::Length(RESTARTS),
                Constraint::Length(AGE),
                Constraint::Length(CONTAINERS),
            ],
            vec![
                Cell::from(""),
                Cell::from("POD"),
                Cell::from(reference_title),
                Cell::from("STATUS"),
                Cell::from(Text::from("RST").right_aligned()),
                Cell::from(Text::from("AGE").right_aligned()),
                Cell::from(Text::from("CTR").right_aligned()),
            ],
        ),
        false => (
            vec![
                Constraint::Length(GLYPH),
                Constraint::Fill(1),
                Constraint::Length(COMMIT),
                Constraint::Length(CONTAINERS),
            ],
            vec![
                Cell::from(""),
                Cell::from("POD"),
                Cell::from(reference_title),
                Cell::from(Text::from("CTR").right_aligned()),
            ],
        ),
    };

    let body: Vec<Row> = rows
        .iter()
        .map(|pod| {
            let name = Cell::from(Line::from(pod_name_spans(pod, theme, name_width)));
            let reference = Cell::from(ellipsize(&pod.reference, COMMIT as usize - 1))
                .style(Style::new().fg(theme.verdict(pod.verdict)));
            let containers = Cell::from(Text::from(pod.readiness.clone()).right_aligned())
                .style(Style::new().fg(theme.dim));

            let cells = match wide {
                true => vec![
                    Cell::from(Line::from(glyph(pod.verdict, theme, spinner))),
                    name,
                    reference,
                    Cell::from(ellipsize(&pod.status, STATUS as usize - 1)).style(Style::new().fg(
                        match is_calm(&pod.status) {
                            true => theme.dim,
                            false => theme.bad,
                        },
                    )),
                    Cell::from(Text::from(pod.restarts.to_string()).right_aligned()).style(
                        Style::new().fg(match pod.restarts {
                            0 => theme.faint,
                            _ => theme.warn,
                        }),
                    ),
                    Cell::from(Text::from(pod.age.clone()).right_aligned())
                        .style(Style::new().fg(theme.dim)),
                    containers,
                ],
                false => vec![
                    Cell::from(Line::from(glyph(pod.verdict, theme, spinner))),
                    name,
                    reference,
                    containers,
                ],
            };

            Row::new(cells)
        })
        .collect();

    let table = Table::new(body, widths)
        .header(Row::new(header).style(Style::new().fg(pane_title(focused, theme))))
        .row_highlight_style(Style::new().bg(selection(focused, theme)))
        .highlight_symbol(Span::styled("▌", Style::new().fg(theme.cyan)))
        .highlight_spacing(HighlightSpacing::Always);

    let [table_area, _, scroll] = Layout::horizontal([
        Constraint::Fill(1),
        Constraint::Length(GAP),
        Constraint::Length(1),
    ])
    .areas(area);

    let state = app.pod_state();
    state.select(selected);
    frame.render_stateful_widget(table, table_area, state);

    scrollbar(frame, scroll, theme, rows.len(), app.pod_state().offset());
}

/// Containers of the selected pod, each one saying in words why it is judged the way it is.
fn draw_containers(
    frame: &mut Frame,
    area: Rect,
    app: &mut App,
    theme: &Theme,
    rows: Vec<ContainerRow>,
) {
    let focused = app.pane() == Pane::Containers;
    let selected = app.container_position();
    let spinner = app.spinner();
    let title = match (app.selected_entry(), app.selected_pod()) {
        (Some(entry), Some(pod)) => {
            let parts = split_pod_name(&pod.name, &entry.name);
            format!("CONTAINERS OF …{}{}", parts.replicaset, parts.suffix)
        }
        _ => "CONTAINERS".to_string(),
    };

    let block = Block::new()
        .borders(Borders::TOP)
        .border_style(Style::new().fg(theme.line))
        .title(Span::styled(
            title,
            Style::new().fg(pane_title(focused, theme)),
        ));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let widths = [
        Constraint::Length(GLYPH),
        Constraint::Length(CONTAINER_NAME),
        Constraint::Length(CONTAINER_TAG),
        Constraint::Fill(1),
    ];

    let body: Vec<Row> = rows
        .iter()
        .map(|container| {
            Row::new(vec![
                Cell::from(Line::from(glyph(container.verdict, theme, spinner))),
                Cell::from(ellipsize(&container.name, CONTAINER_NAME as usize - 1)).style(
                    Style::new().fg(match container.verdict {
                        Verdict::InSync => theme.fg2,
                        _ => theme.fg,
                    }),
                ),
                Cell::from(ellipsize(&container.tag, CONTAINER_TAG as usize - 1))
                    .style(Style::new().fg(theme.verdict(container.verdict))),
                Cell::from(container.why.clone()).style(Style::new().fg(theme.dim)),
            ])
        })
        .collect();

    let table = Table::new(body, widths)
        .row_highlight_style(Style::new().bg(selection(focused, theme)))
        .highlight_symbol(Span::styled("▌", Style::new().fg(theme.cyan)))
        .highlight_spacing(HighlightSpacing::Always);

    let state = app.cont_state();
    state.select(selected);
    frame.render_stateful_widget(table, inner, state);
}

/// Three levels of brightness: the workload prefix, the ReplicaSet hash that tells an old
/// rollout from a new one, and the suffix. The prefix is the first to be cut.
fn pod_name_spans(pod: &PodRow, theme: &Theme, width: usize) -> Vec<Span<'static>> {
    let telling = format!("{}{}", pod.replicaset, pod.suffix);
    let (prefix, kept) = fit_head(&pod.prefix, &telling, width);

    if kept != telling {
        return vec![Span::styled(kept, Style::new().fg(theme.fg))];
    }

    vec![
        Span::styled(prefix, Style::new().fg(theme.faint)),
        Span::styled(pod.replicaset.clone(), Style::new().fg(theme.dim)),
        Span::styled(pod.suffix.clone(), Style::new().fg(theme.fg)),
    ]
}

fn pane_title(focused: bool, theme: &Theme) -> Color {
    match focused {
        true => theme.cyan,
        false => theme.dim,
    }
}

fn selection(focused: bool, theme: &Theme) -> Color {
    match focused {
        true => theme.selection,
        false => theme.bar,
    }
}
