use crate::caller::browser::open_with_hash;
use crate::errors::MsgError;
use crate::k8s::pods::MyPod;
use crate::storage::json_storage::JsonStorage;
use crossterm::event::{self, KeyCode, KeyEventKind};
use ratatui::backend::CrosstermBackend;
use ratatui::{prelude::*, widgets::*};

pub fn process_frames(
    mut terminal: Terminal<CrosstermBackend<std::io::Stdout>>,
    pods: Vec<MyPod>,
    repos: JsonStorage,
) -> Result<(), MsgError> {
    let mut current_pod = 0;
    let mut current_container = 0;
    let mut pressed = false;

    loop {
        match terminal.draw(|frame| {
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
        }) {
            Ok(_) => (),
            Err(_) => return Err(MsgError::new("There was a problem drawing new frame")),
        }

        let polled = match event::poll(std::time::Duration::from_millis(16)) {
            Ok(x) => x,
            Err(_) => return Err(MsgError::new("There was an error when polling")),
        };

        if polled {
            if let Ok(event::Event::Key(key)) = event::read() {
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
                                Err(e) => return Err(e),
                            };

                            let url = match repos.get_repo_by_name(&pods[current_pod].namespace) {
                                Some(x) => &x.url,
                                None => return Err(MsgError::new("Missing repo in storage")),
                            };

                            match open_with_hash(&url, &hash) {
                                Ok(_) => (),
                                Err(_) => {
                                    return Err(MsgError::new(
                                        "There was a problem opening browser",
                                    ))
                                }
                            }

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
            } else {
                return Err(MsgError::new("There was an error when reading events"));
            }
        }
    }

    Ok(())
}
