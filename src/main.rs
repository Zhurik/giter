use clap::Parser;
use crossterm::{
    event::{self, KeyCode, KeyEventKind},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use giter::caller::browser::open_with_hash;
use giter::config::config::Args;
use giter::k8s::ns::get_current_namespace;
use giter::k8s::pods::MyPod;
use giter::storage::json_storage::JsonStorage;
use ratatui::{prelude::*, widgets::*};
use std::io::{stdout, Error, ErrorKind, Result};

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    let repos = match JsonStorage::new(args.storage_path.leak()) {
        Ok(x) => x,
        Err(e) => return Err(Error::new(ErrorKind::InvalidData, e.to_string())),
    };

    let current_ns: String = match args.namespace {
        Some(x) => x,
        None => match get_current_namespace() {
            Ok(x) => x,
            Err(e) => return Err(Error::other(e.details)),
        },
    };

    let pods = match MyPod::get_pods_by_ns(current_ns).await {
        Ok(x) => x,
        Err(e) => return Err(Error::other(e.details)),
    };

    let mut current_pod = 0;
    let mut current_container = 0;
    let mut pressed = false;

    stdout().execute(EnterAlternateScreen)?;
    enable_raw_mode()?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
    terminal.clear()?;

    loop {
        terminal.draw(|frame| {
            let layout = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(frame.size());

            let mut pods_text: Vec<Line> = vec![];
            let mut conts_text: Vec<Line> = vec![];

            for (i, pod) in pods.iter().enumerate() {
                let msg = format!("{}", pod.name);

                let line = if i == current_pod && !pressed {
                    Line::from(msg.on_white().black())
                } else if i == current_pod && pressed {
                    Line::from(msg.on_dark_gray().white())
                } else {
                    Line::from(msg)
                };
                pods_text.push(line);

                if i == current_pod {
                    for (j, cont) in pod.containers.iter().enumerate() {
                        let cont_msg = format!("{}", cont.image);

                        let cont_line = if j == current_container && pressed {
                            Line::from(cont_msg.on_white().black())
                        } else {
                            Line::from(cont_msg)
                        };
                        conts_text.push(cont_line)
                    }
                }
            }
            frame.render_widget(
                Paragraph::new(pods_text).block(Block::new().borders(Borders::ALL).title("Pods")),
                layout[0],
            );

            frame.render_widget(
                Paragraph::new(conts_text)
                    .block(Block::new().borders(Borders::ALL).title("Containers")),
                layout[1],
            );
        })?;

        if event::poll(std::time::Duration::from_millis(16))? {
            if let event::Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    if key.code == KeyCode::Char('y') {}
                    if key.code == KeyCode::Char('q') {
                        break;
                    }
                    if key.code == KeyCode::Up {
                        if !pressed {
                            if current_pod > 0 {
                                current_pod -= 1;
                            }
                        } else {
                            if current_container > 0 {
                                current_container -= 1;
                            }
                        }
                    }
                    if key.code == KeyCode::Down {
                        if !pressed {
                            if current_pod + 1 < pods.len() {
                                current_pod += 1;
                            }
                        } else {
                            if current_container + 1 < pods[current_pod].containers.len() {
                                current_container += 1;
                            }
                        }
                    }

                    if key.code == KeyCode::Enter {
                        if !pressed {
                            pressed = true;
                        } else {
                            let hash = match pods[current_pod].containers[current_container]
                                .commit_hash()
                            {
                                Ok(x) => x,
                                Err(e) => return Err(Error::other(e.details)),
                            };

                            let url = match repos.get_repo_by_name(&pods[current_pod].namespace) {
                                Some(x) => &x.url,
                                None => return Err(Error::other("Can't find ns to in storage")),
                            };

                            open_with_hash(&url, &hash)?;
                            break;
                        }
                    }

                    if key.code == KeyCode::Right {
                        if !pressed {
                            pressed = true;
                        }
                    }

                    if key.code == KeyCode::Left {
                        if pressed {
                            pressed = false;
                        }
                    }
                }
            }
        }
    }

    stdout().execute(LeaveAlternateScreen)?;
    disable_raw_mode()?;
    Ok(())
}
