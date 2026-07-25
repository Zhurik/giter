use crate::drawer::app::{App, Deployed, NsRow, RefKind};
use crate::drawer::chrome::{glyph, PAD};
use crate::drawer::theme::Theme;
use crate::git::remote::Verdict;
use crate::text::{ellipsize, fit_head};
use ratatui::prelude::*;
use ratatui::widgets::{
    Cell, HighlightSpacing, Row, Scrollbar, ScrollbarOrientation, ScrollbarState, Table,
};

const GLYPH: u16 = 2;
const PODS: u16 = 5;
const DEPLOYED: u16 = 30;
const REFERENCE: u16 = 12;
const GROUP_WIDTH: usize = 15;
/// Room for `+12` once a namespace runs more commits than the column can list.
const HIDDEN_WIDTH: usize = 4;
const NARROW_REFERENCE: u16 = 11;
/// One column of the cursor, plus the single space `Table` keeps between columns.
const CURSOR: u16 = 1;
const GAP: u16 = 1;

/// The whole cluster in one table, worst namespaces first.
pub fn draw(frame: &mut Frame, area: Rect, app: &mut App, theme: &Theme) {
    let area = area.inner(Margin::new(PAD, 0));
    let title = deployed_title(app.deployed_kind());
    let rows = app.overview_rows();
    let selected = app.ns_position();
    let query = app.query().to_string();
    let spinner = app.spinner();

    let [table_area, _, scroll_area] = Layout::horizontal([
        Constraint::Fill(1),
        Constraint::Length(GAP),
        Constraint::Length(1),
    ])
    .areas(area);

    let (name_width, note_width) = flexible(table_area.width);
    let widths = columns(name_width);

    let header = Row::new(vec![
        Cell::from(""),
        Cell::from("NAMESPACE"),
        Cell::from(Text::from("PODS").right_aligned()),
        Cell::from(title),
        Cell::from("REFERENCE"),
        Cell::from("NOTE"),
    ])
    .style(Style::new().fg(theme.dim));

    let body: Vec<Row> = rows
        .iter()
        .map(|row| {
            Row::new(vec![
                Cell::from(Line::from(glyph(row.verdict, theme, spinner))),
                Cell::from(Line::from(name_spans(row, &query, theme, name_width))),
                Cell::from(
                    Text::from(match row.pods {
                        Some(count) => count.to_string(),
                        None => "—".to_string(),
                    })
                    .right_aligned(),
                )
                .style(Style::new().fg(theme.dim)),
                Cell::from(Line::from(deployed_spans(row, theme))),
                Cell::from(row.reference.clone())
                    .style(Style::new().fg(reference_color(row, theme))),
                Cell::from(ellipsize(&row.note, note_width))
                    .style(Style::new().fg(note_color(row.verdict, theme))),
            ])
        })
        .collect();

    let table = Table::new(body, widths)
        .header(header)
        .row_highlight_style(Style::new().bg(theme.selection))
        .highlight_symbol(Span::styled("▌", Style::new().fg(theme.cyan)))
        .highlight_spacing(HighlightSpacing::Always);

    let state = app.ns_state();
    state.select(selected);
    frame.render_stateful_widget(table, table_area, state);

    scrollbar(
        frame,
        scroll_area,
        theme,
        rows.len(),
        app.ns_state().offset(),
    );
}

/// Under a hundred columns the table gives up everything but the verdict, the name, the
/// deployed reference and the pod count.
pub fn draw_narrow(frame: &mut Frame, area: Rect, app: &mut App, theme: &Theme) {
    let area = area.inner(Margin::new(PAD, 0));
    let rows = app.overview_rows();
    let selected = app.ns_position();
    let query = app.query().to_string();
    let spinner = app.spinner();

    let widths = [
        Constraint::Length(GLYPH),
        Constraint::Fill(1),
        Constraint::Length(NARROW_REFERENCE),
        Constraint::Length(PODS),
    ];

    let body: Vec<Row> = rows
        .iter()
        .map(|row| {
            let deployed = row.deployed.first();

            Row::new(vec![
                Cell::from(Line::from(glyph(row.verdict, theme, spinner))),
                Cell::from(Line::from(name_spans(
                    row,
                    &query,
                    theme,
                    narrow_name_width(area.width),
                ))),
                Cell::from(
                    deployed
                        .map(|group| group.label.clone())
                        .unwrap_or_else(|| "—".to_string()),
                )
                .style(Style::new().fg(match deployed {
                    Some(group) => theme.verdict(group.verdict),
                    None => theme.dim,
                })),
                Cell::from(
                    Text::from(match row.pods {
                        Some(count) => count.to_string(),
                        None => "—".to_string(),
                    })
                    .right_aligned(),
                )
                .style(Style::new().fg(theme.dim)),
            ])
        })
        .collect();

    let table = Table::new(body, widths)
        .row_highlight_style(Style::new().bg(theme.selection))
        .highlight_symbol(Span::styled("▌", Style::new().fg(theme.cyan)))
        .highlight_spacing(HighlightSpacing::Always);

    let state = app.ns_state();
    state.select(selected);
    frame.render_stateful_widget(table, area, state);
}

/// The shared prefix is dimmed, and when the column runs out it is the prefix that gives
/// up room — the tail is what tells one namespace from another. What the search matched is
/// underlined on top of that.
pub fn name_spans(row: &NsRow, query: &str, theme: &Theme, width: usize) -> Vec<Span<'static>> {
    let (prefix, tail) = split_name(row);
    let (prefix, tail) = fit_head(prefix, tail, width);

    let mut spans = vec![Span::styled(prefix, Style::new().fg(theme.faint))];
    spans.extend(matched_spans(&tail, query, theme, name_color(row, theme)));

    spans
}

/// Where a namespace name stops repeating its family and starts telling them apart.
pub fn split_name(row: &NsRow) -> (&str, &str) {
    let cut = row
        .name
        .char_indices()
        .nth(row.prefix)
        .map(|(index, _)| index)
        .unwrap_or(row.name.len());

    row.name.split_at(cut)
}

pub fn name_color(row: &NsRow, theme: &Theme) -> Color {
    match row.verdict {
        Verdict::InSync => theme.fg2,
        _ => theme.fg,
    }
}

/// Draws `text`, underlining the part the search matched.
pub fn matched_spans(text: &str, query: &str, theme: &Theme, color: Color) -> Vec<Span<'static>> {
    let Some(at) = hit(text, query) else {
        return vec![Span::styled(text.to_string(), Style::new().fg(color))];
    };

    vec![
        Span::styled(text[..at].to_string(), Style::new().fg(color)),
        Span::styled(
            text[at..at + query.len()].to_string(),
            Style::new().fg(theme.cyan).bold().underlined(),
        ),
        Span::styled(text[at + query.len()..].to_string(), Style::new().fg(color)),
    ]
}

/// Where the search text sits in the part of the name that is shown bright.
fn hit(tail: &str, query: &str) -> Option<usize> {
    if query.is_empty() || !tail.is_ascii() || !query.is_ascii() {
        return None;
    }

    tail.to_lowercase().find(&query.to_lowercase())
}

/// Every distinct reference the pods of a namespace run, with how many run it — a rollout
/// that stopped half way through shows up as two groups.
///
/// The groups keep the same width whatever the row holds, so that the second one starts in
/// the same place in every row and under the heading. Room for the `+N` of the ones left out
/// comes out of the last group rather than out of the layout.
fn deployed_spans(row: &NsRow, theme: &Theme) -> Vec<Span<'static>> {
    let hidden = row.hidden_deployed;
    let last = row.deployed.len().saturating_sub(1);

    let mut spans: Vec<Span> = row
        .deployed
        .iter()
        .enumerate()
        .map(|(index, group)| {
            let width = match hidden > 0 && index == last {
                true => GROUP_WIDTH - HIDDEN_WIDTH,
                false => GROUP_WIDTH,
            };

            Span::styled(
                format!("{:<width$}", label(group, width), width = width),
                Style::new().fg(theme.verdict(group.verdict)),
            )
        })
        .collect();

    if hidden > 0 {
        spans.push(Span::styled(
            format!("+{}", hidden),
            Style::new().fg(theme.dim),
        ));
    }

    spans
}

/// The heading names what the column actually holds. A cluster deployed from releases shows
/// tags whichever mode is on, so naming it after the mode would be a guess.
pub fn deployed_title(kind: RefKind) -> &'static str {
    match kind {
        RefKind::Commits => "DEPLOYED COMMIT",
        RefKind::Tags => "DEPLOYED TAG",
        RefKind::Mixed => "DEPLOYED",
    }
}

fn label(group: &Deployed, width: usize) -> String {
    let text = match group.count > 1 {
        true => format!("{} ×{}", group.label, group.count),
        false => group.label.clone(),
    };

    ellipsize(&text, width.saturating_sub(1))
}

fn reference_color(row: &NsRow, theme: &Theme) -> Color {
    match row.verdict {
        Verdict::Resolving => theme.dim,
        _ => theme.fg2,
    }
}

fn note_color(verdict: Verdict, theme: &Theme) -> Color {
    match verdict {
        Verdict::Behind => theme.bad,
        Verdict::Failed(_) => theme.warn,
        _ => theme.dim,
    }
}

/// The note shares whatever the fixed columns leave with the namespace name, so it has to
/// cut itself off at the same place the table would.
/// The six columns of the overview table. The namespace is a fixed length rather than a
/// share, so that what the cells cut themselves to is what the layout hands them.
fn columns(name_width: usize) -> [Constraint; 6] {
    [
        Constraint::Length(GLYPH),
        Constraint::Length(name_width as u16),
        Constraint::Length(PODS),
        Constraint::Length(DEPLOYED),
        Constraint::Length(REFERENCE),
        Constraint::Fill(1),
    ]
}

/// How the room the fixed columns leave is split between the namespace and the note: three
/// fifths to the namespace, as the design has it, the rest to the note.
///
/// The namespace column is then laid out as a fixed length rather than a share, so that the
/// width the cells cut themselves to is the width they actually get — rounding inside the
/// layout solver is not something to guess at, and being one column out means the last
/// character of a name disappears without an ellipsis to say so.
fn flexible(total: u16) -> (usize, usize) {
    let fixed = GLYPH + PODS + DEPLOYED + REFERENCE + 5 * GAP + CURSOR;
    let room = total.saturating_sub(fixed) as usize;
    let name = room * 3 / 5;

    (name, room - name)
}

fn narrow_name_width(total: u16) -> usize {
    total.saturating_sub(GLYPH + NARROW_REFERENCE + PODS + 3 * GAP + CURSOR) as usize
}

pub fn scrollbar(frame: &mut Frame, area: Rect, theme: &Theme, total: usize, offset: usize) {
    if total <= area.height as usize {
        return;
    }

    let mut state = ScrollbarState::new(total).position(offset);

    frame.render_stateful_widget(
        Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None)
            .track_symbol(Some(" "))
            .thumb_symbol("▐")
            .track_style(Style::new().fg(theme.line))
            .thumb_style(Style::new().fg(theme.line)),
        area,
        &mut state,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use speculoos::prelude::*;

    /// What `Table` hands each column at a given table width: it takes the cursor column off
    /// the top and lays the rest out itself.
    fn laid_out(total: u16) -> Vec<u16> {
        let (name, _) = flexible(total);
        let area = Rect::new(0, 0, total.saturating_sub(CURSOR), 1);

        Layout::horizontal(columns(name))
            .spacing(GAP)
            .split(area)
            .iter()
            .map(|column| column.width)
            .collect()
    }

    fn row(deployed: Vec<Deployed>, hidden: usize) -> NsRow {
        NsRow {
            name: "orchard-gateway".to_string(),
            prefix: 0,
            verdict: Verdict::Behind,
            pods: Some(9),
            deployed,
            hidden_deployed: hidden,
            reference: "4f1c0f2c".to_string(),
            note: "3 of 9 pods behind".to_string(),
        }
    }

    fn group(label: &str, count: usize) -> Deployed {
        Deployed {
            label: label.to_string(),
            count,
            verdict: Verdict::Behind,
        }
    }

    fn width_of(row: &NsRow) -> usize {
        deployed_spans(row, &Theme::new(None, false))
            .iter()
            .map(Span::width)
            .sum()
    }

    #[test]
    fn name_column_is_exactly_as_wide_as_the_names_are_cut_to() {
        let disagreements: Vec<u16> = (60..=200)
            .filter(|total| laid_out(*total)[1] as usize != flexible(*total).0)
            .collect();

        asserting!("a column one narrower than the text was cut to drops a character silently")
            .that(&disagreements)
            .is_empty();
    }

    #[test]
    fn two_commits_fill_the_column() {
        let listed = row(vec![group("4f1c0f2c", 17), group("91bd7e40", 4)], 0);

        assert_that!(width_of(&listed)).is_less_than_or_equal_to(DEPLOYED as usize);
    }

    #[test]
    fn commits_give_up_room_to_the_count_of_those_left_out() {
        let listed = row(vec![group("4f1c0f2c", 5), group("91bd7e40", 3)], 3);

        asserting!("the column would otherwise cut off the only sign that there are more")
            .that(&width_of(&listed))
            .is_less_than_or_equal_to(DEPLOYED as usize);
    }

    #[test]
    fn commits_left_out_are_counted_at_the_end() {
        let listed = row(vec![group("4f1c0f2c", 5), group("91bd7e40", 3)], 3);
        let spans = deployed_spans(&listed, &Theme::new(None, false));

        assert_that!(spans.last().map(|span| span.content.to_string()))
            .is_equal_to(Some("+3".to_string()));
    }
}
