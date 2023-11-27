use clap::Parser;
use crossterm::{
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use giter::config::config::Args;
use giter::drawer::terminal::process_frames;
use giter::k8s::ns::get_current_namespace;
use giter::k8s::pods::MyPod;
use giter::storage::json_storage::JsonStorage;
use ratatui::prelude::*;
use std::io::{stdout, Error, ErrorKind, Result};

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    let repos: JsonStorage = match JsonStorage::new(args.storage_path.leak()) {
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

    stdout().execute(EnterAlternateScreen)?;
    enable_raw_mode()?;
    let mut terminal: Terminal<CrosstermBackend<std::io::Stdout>> =
        Terminal::new(CrosstermBackend::new(stdout()))?;
    terminal.clear()?;

    let error_msg = match process_frames(terminal, pods, repos) {
        Ok(_) => None,
        Err(e) => Some(e.details),
    };

    stdout().execute(LeaveAlternateScreen)?;
    disable_raw_mode()?;

    match error_msg {
        Some(x) => {
            eprintln!("{}", x);
            std::process::exit(1);
        }
        None => return Ok(()),
    }
}
