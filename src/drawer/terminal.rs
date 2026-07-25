use crate::drawer::app::{App, Focus, NamespaceEntry, KEY_HINTS};
use crate::errors::MsgError;
use crate::git::remote::{CommitStatus, RemoteState};
use crossterm::event::{self, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::backend::CrosstermBackend;
use ratatui::{prelude::*, widgets::*};
use std::time::Duration;

const POLL_INTERVAL: Duration = Duration::from_millis(100);
const PAGE: isize = 10;

pub fn process_frames(
    mut terminal: Terminal<CrosstermBackend<std::io::Stdout>>,
    app: &mut App,
) -> Result<(), MsgError> {
    loop {
        if terminal.draw(|frame| draw(frame, app)).is_err() {
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

        app.drain_lookups();

        if app.should_quit() {
            return Ok(());
        }
    }
}

fn handle_key(app: &mut App, key: KeyEvent) {
    if key.kind != KeyEventKind::Press {
        return;
    }

    match key.code {
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => app.quit(),
        KeyCode::Char('q') | KeyCode::Esc => app.quit(),
        KeyCode::Up | KeyCode::Char('k') => app.move_selection(-1),
        KeyCode::Down | KeyCode::Char('j') => app.move_selection(1),
        KeyCode::PageUp => app.move_selection(-PAGE),
        KeyCode::PageDown => app.move_selection(PAGE),
        KeyCode::Home => app.move_selection(isize::MIN),
        KeyCode::End => app.move_selection(isize::MAX),
        KeyCode::Right | KeyCode::Char('l') | KeyCode::Tab => app.focus_next(),
        KeyCode::Left | KeyCode::Char('h') | KeyCode::BackTab => app.focus_prev(),
        KeyCode::Enter => app.activate(),
        KeyCode::Char('m') => app.toggle_mode(),
        KeyCode::Char('r') => app.refresh_selected(),
        _ => (),
    }
}

fn draw(frame: &mut Frame, app: &mut App) {
    let root = Layout::vertical([Constraint::Min(3), Constraint::Length(1)]).split(frame.area());
    let panes = Layout::horizontal([
        Constraint::Percentage(30),
        Constraint::Percentage(35),
        Constraint::Percentage(35),
    ])
    .split(root[0]);

    let focus = app.focus();
    let ns_items = namespace_items(app);
    let pod_items = pod_items(app);
    let cont_items = container_items(app);
    let ns_title = format!("Namespaces ({})", ns_items.len());
    let pod_title = format!("Pods ({})", pod_items.len());
    let cont_title = containers_title(app);
    let footer = format!("{} · {}", app.footer(), KEY_HINTS);

    frame.render_stateful_widget(
        list(ns_items, ns_title, focus == Focus::Namespaces),
        panes[0],
        app.ns_state(),
    );
    frame.render_stateful_widget(
        list(pod_items, pod_title, focus == Focus::Pods),
        panes[1],
        app.pod_state(),
    );
    frame.render_stateful_widget(
        list(cont_items, cont_title, focus == Focus::Containers),
        panes[2],
        app.cont_state(),
    );

    frame.render_widget(
        Paragraph::new(Line::from(footer).style(Style::new().fg(Color::DarkGray))),
        root[1],
    );
}

fn list(items: Vec<ListItem<'static>>, title: String, focused: bool) -> List<'static> {
    let mut block = Block::new().borders(Borders::ALL).title(title);

    if focused {
        block = block
            .border_style(Style::new().fg(Color::Cyan))
            .title_style(Style::new().add_modifier(Modifier::BOLD));
    }

    List::new(items)
        .block(block)
        .highlight_style(Style::new().add_modifier(Modifier::REVERSED))
}

fn namespace_items(app: &App) -> Vec<ListItem<'static>> {
    app.namespaces()
        .iter()
        .map(|entry| item(namespace_label(entry), app.namespace_status(entry)))
        .collect()
}

fn namespace_label(entry: &NamespaceEntry) -> String {
    match &entry.error {
        Some(_) => format!("{} (unavailable)", entry.name),
        None => format!("{} ({})", entry.name, entry.pods.len()),
    }
}

fn pod_items(app: &App) -> Vec<ListItem<'static>> {
    let Some(entry) = app.selected_namespace() else {
        return vec![];
    };

    entry
        .pods
        .iter()
        .map(|pod| item(pod.name.clone(), app.pod_status(&entry.name, pod)))
        .collect()
}

fn container_items(app: &App) -> Vec<ListItem<'static>> {
    let Some(entry) = app.selected_namespace() else {
        return vec![];
    };
    let Some(pod) = app.selected_pod() else {
        return vec![];
    };

    pod.containers
        .iter()
        .map(|cont| {
            item(
                format!("{}  {}", cont.name, cont.tag().unwrap_or("no tag")),
                app.container_status(&entry.name, cont),
            )
        })
        .collect()
}

/// Shows what the pods are being compared against, so a gray pane is explainable.
fn containers_title(app: &App) -> String {
    let Some(entry) = app.selected_namespace() else {
        return "Containers".to_string();
    };

    match app.remote(&entry.name) {
        Some(RemoteState::Ready(target)) => format!("Containers · {}", target.label()),
        Some(RemoteState::Loading) => "Containers · resolving".to_string(),
        Some(RemoteState::Failed(reason)) => format!("Containers · {}", reason),
        None => "Containers".to_string(),
    }
}

fn item(label: String, status: CommitStatus) -> ListItem<'static> {
    ListItem::new(Line::from(label)).style(style_for(status))
}

fn style_for(status: CommitStatus) -> Style {
    match status {
        CommitStatus::Latest => Style::new().fg(Color::Green),
        CommitStatus::Outdated => Style::new().fg(Color::Red),
        CommitStatus::Unknown => Style::new().fg(Color::DarkGray),
    }
}
