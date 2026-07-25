use crate::drawer::app::{App, Input, Pane, Screen};
use crate::drawer::chrome::{self, MIN_HEIGHT, MIN_WIDTH, WIDE_WIDTH};
use crate::drawer::theme::Theme;
use crate::drawer::{detail, overview};
use crate::errors::MsgError;
use crossterm::event::{self, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::backend::CrosstermBackend;
use ratatui::prelude::*;
use ratatui::widgets::Block;
use std::time::Duration;

/// Slow enough to keep the process idle, fast enough for the spinner to turn and for
/// answers from the background to show up as they land.
const POLL_INTERVAL: Duration = Duration::from_millis(100);
const PAGE: isize = 10;

pub fn process_frames(
    mut terminal: Terminal<CrosstermBackend<std::io::Stdout>>,
    app: &mut App,
    theme: &Theme,
) -> Result<(), MsgError> {
    loop {
        if terminal.draw(|frame| draw(frame, app, theme)).is_err() {
            return Err(MsgError::new("There was a problem drawing new frame"));
        }

        let polled = match event::poll(POLL_INTERVAL) {
            Ok(x) => x,
            Err(_) => return Err(MsgError::new("There was an error when polling")),
        };

        if polled {
            match event::read() {
                Ok(event::Event::Key(key)) => handle_key(app, key),
                Ok(_) => (),
                Err(_) => return Err(MsgError::new("There was an error when reading events")),
            }
        }

        app.tick();

        if app.should_quit() {
            return Ok(());
        }
    }
}

fn draw(frame: &mut Frame, app: &mut App, theme: &Theme) {
    let area = frame.area();
    frame.render_widget(
        Block::new().style(Style::new().bg(theme.bg).fg(theme.fg)),
        area,
    );

    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        chrome::too_small(frame, area, theme);
        return;
    }

    let narrow = area.width < WIDE_WIDTH;
    app.set_narrow(narrow);

    let [head, rest, status, keys] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(area);

    frame.render_widget(
        match narrow {
            true => chrome::narrow_header(app, theme, head.width),
            false => chrome::header(app, theme, head.width),
        },
        head,
    );

    // the controls bar belongs to the overview; the detail screen only borrows it to show
    // the search prompt, and the narrow layout shows counts in its place
    let searching = app.input() == Input::Search;
    let body = match narrow || app.screen() == Screen::Overview || searching {
        true => {
            let [bar, body] =
                Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(rest);

            frame.render_widget(
                match (searching, narrow) {
                    (true, _) => chrome::bar(app, theme, bar.width),
                    (false, true) => chrome::narrow_bar(app, theme, bar.width),
                    (false, false) => chrome::bar(app, theme, bar.width),
                },
                bar,
            );

            body
        }
        false => rest,
    };

    match (app.screen(), narrow) {
        (Screen::Overview, false) => overview::draw(frame, body, app, theme),
        (Screen::Overview, true) => overview::draw_narrow(frame, body, app, theme),
        (Screen::Detail, wide) => detail::draw(frame, body, app, theme, !wide),
    }

    frame.render_widget(
        chrome::status_line(app, theme, status.width, narrow),
        status,
    );
    frame.render_widget(chrome::keys_line(app, theme, keys.width, narrow), keys);

    if app.input() == Input::Help {
        chrome::help(frame, area, theme);
    }
}

fn handle_key(app: &mut App, key: KeyEvent) {
    if key.kind != KeyEventKind::Press {
        return;
    }

    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
        app.quit();
        return;
    }

    match app.input() {
        Input::Help => handle_help(app, key),
        Input::Search => handle_search(app, key),
        Input::Normal => handle_normal(app, key),
    }
}

fn handle_help(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char('?') | KeyCode::Char('q') | KeyCode::Esc => app.toggle_help(),
        _ => (),
    }
}

fn handle_search(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Enter => app.end_search(true),
        KeyCode::Esc => app.end_search(false),
        KeyCode::Backspace => app.search_pop(),
        KeyCode::Up => app.move_selection(-1),
        KeyCode::Down => app.move_selection(1),
        KeyCode::Char(symbol) => app.search_push(symbol),
        _ => (),
    }
}

fn handle_normal(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char('q') => app.quit(),
        KeyCode::Esc => app.back(),
        KeyCode::Up | KeyCode::Char('k') => app.move_selection(-1),
        KeyCode::Down | KeyCode::Char('j') => app.move_selection(1),
        KeyCode::PageUp => app.move_selection(-PAGE),
        KeyCode::PageDown => app.move_selection(PAGE),
        KeyCode::Home => app.move_selection(isize::MIN),
        KeyCode::End => app.move_selection(isize::MAX),
        KeyCode::Right | KeyCode::Char('l') => match app.screen() {
            Screen::Overview => app.open_namespace(),
            Screen::Detail => app.focus_next(),
        },
        KeyCode::Left | KeyCode::Char('h') => match (app.screen(), app.pane()) {
            // the namespace list is not on screen in the narrow layout, so there is nothing
            // to the left of the pods but the way back
            (Screen::Detail, Pane::Pods) if app.narrow() => app.back(),
            (Screen::Detail, Pane::Namespaces) => app.back(),
            (Screen::Detail, _) => app.focus_prev(),
            (Screen::Overview, _) => (),
        },
        KeyCode::Tab => match app.screen() {
            Screen::Detail => app.focus_containers(),
            Screen::Overview => app.open_namespace(),
        },
        KeyCode::BackTab => app.focus_prev(),
        KeyCode::Enter => app.activate(),
        KeyCode::Char('m') => app.toggle_mode(),
        KeyCode::Char('r') => app.refresh_selected(),
        KeyCode::Char('R') => app.relist_pods(),
        KeyCode::Char('f') => app.cycle_filter(),
        KeyCode::Char('s') => app.cycle_sort(),
        KeyCode::Char('/') => app.start_search(),
        KeyCode::Char('?') => app.toggle_help(),
        _ => (),
    }
}
