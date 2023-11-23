use crossterm::{
    event::{self, KeyCode, KeyEventKind},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use giter::caller::browser::open_with_hash;
use giter::k8s::ns::get_current_namespace;
use giter::k8s::pods::get_pods_image_hashes;
use giter::storage::common::Storage;
use giter::storage::json_storage::JsonStorage;
use ratatui::{prelude::*, widgets::Paragraph};
use std::io::{stdout, Result};

fn main() -> Result<()> {
    let storage = JsonStorage::new("./repos.json");
    let repos = storage.list_repos();

    let commit_hash = &get_pods_image_hashes()[0];
    let current_ns = get_current_namespace();

    println!("test");

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
                    "Calling for this commit: {}\nIn namespace: {}\nPress 'y' to open browser\nPress 'q' to open quit",
                    commit_hash,
                    current_ns,
                ))
                    .white(),
                    //.on_blue(),
                area,
            );

            // let layout = Layout::default()
            //     .direction(Direction::Vertical)
            //     .constraints(constraints)
            //     .split(frame.size());

            // for (index, repo) in repos.iter().enumerate() {
            //     frame.render_widget(
            //         Paragraph::new(
            //             format!(
            //                 "Here some {:?} called {:?}",
            //                 repo.url,
            //                 repo.name
            //             )).block(
            //                 Block::new().borders(
            //                     Borders::ALL
            //                 )
            //             ),
            //         layout[index]
            //     );
            // }
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
