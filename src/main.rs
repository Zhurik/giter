use crossterm::{
    event::{self, KeyCode, KeyEventKind},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use giter::storage::json_storage::JsonStorage;
use giter::storage::common::Storage;
use ratatui::{
    prelude::*,
    widgets::Paragraph,
    widgets::Borders,
    widgets::Block,
};
use std::io::{stdout, Result};

fn main() -> Result<()> {
    stdout().execute(EnterAlternateScreen)?;
    enable_raw_mode()?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
    terminal.clear()?;

    loop {
        terminal.draw(|frame| {
            let layout = Layout::default()
                .direction(Direction::Vertical)
                .constraints(vec![
                    Constraint::Percentage(50),
                    Constraint::Percentage(50),
                ])
                .split(frame.size());
            //let area = frame.size();
            //frame.render_widget(
            //    Paragraph::new("Hello Ratatui! (press 'q' to quit)")
            //        .black()
            //        .on_dark_gray(),
            //    area,
            //);
            //frame.render_widget(
            //    Paragraph::new("Hello Ratatui! (press 'q' to quit)")
            //        .black()
            //        .on_dark_gray(),
            //    area,
            //);

            let storage = JsonStorage::new("./repos.json");
            let repos = storage.list_repos();
            for (index, repo) in repos.iter().enumerate() {
                frame.render_widget(
                    Paragraph::new(
                        format!(
                            "Here some {:?} called {:?}",
                            repo.url,
                            repo.name
                        )).block(
                            Block::new().borders(
                                Borders::ALL
                            )
                        ),
                    layout[index]
                );
            }
        })?;

        if event::poll(std::time::Duration::from_millis(16))? {
            if let event::Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press && key.code == KeyCode::Char('q') {
                    break;
                }
            }
        }
    }

    stdout().execute(LeaveAlternateScreen)?;
    disable_raw_mode()?;
    Ok(())
}
