use clap::Parser;
use crossterm::{
    event::{self, KeyCode, KeyEventKind},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use giter::caller::browser::open_with_hash;
use giter::k8s::ns::get_current_namespace;
use giter::k8s::pods::get_pods_image_hashes;
use giter::storage::common::Storage;
use giter::config::config::Args;
use giter::storage::json_storage::JsonStorage;
use ratatui::{prelude::*, widgets::Paragraph};
use std::io::{stdout, Result};

fn main() -> Result<()> {
    let args = Args::parse();

    let storage = JsonStorage::new(args.storage_path.leak());
    let repos = storage.list_repos();

    let commit_hash = &get_pods_image_hashes()[0];
    let current_ns = get_current_namespace();

    stdout().execute(EnterAlternateScreen)?;
    enable_raw_mode()?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
    terminal.clear()?;

    loop {
        terminal.draw(|frame| {
            let constraint_percentage = (100 / repos.len()) as u16;
            let mut constraints: Vec<Constraint> = Vec::new();

            for _ in 0..repos.len() {
                constraints.push(Constraint::Percentage(constraint_percentage));
            }

            let area = frame.size();
            frame.render_widget(
                Paragraph::new(format!(
                    "Calling for this commit: {}\nIn namespace: {}\nPress 'y' to open browser\nPress 'q' to quit",
                    commit_hash,
                    current_ns,
                ))
                    .white(),
                    //.on_blue(),
                area,
            );
        })?;

        if event::poll(std::time::Duration::from_millis(16))? {
            if let event::Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    if key.code == KeyCode::Char('y') {
                        let repo = repos.iter().find(|x| x.name == current_ns).unwrap();
                        open_with_hash(&repo.url, commit_hash)?;
                    }
                    if key.code == KeyCode::Char('q') {}
                    break;
                }
            }
        }
    }

    stdout().execute(LeaveAlternateScreen)?;
    disable_raw_mode()?;
    Ok(())
}
