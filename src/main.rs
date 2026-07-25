use clap::Parser;
use crossterm::{
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use giter::config::config::Args;
use giter::drawer::app::App;
use giter::drawer::terminal::process_frames;
use giter::drawer::theme::Theme;
use giter::k8s::{client, context};
use giter::storage::common::Storage;
use giter::storage::json_storage::JsonStorage;
use kube::Client;
use ratatui::prelude::*;
use std::io::{stdout, Error, Result};

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    let repos: JsonStorage = match JsonStorage::new(&args.storage_path) {
        Ok(x) => x,
        Err(e) => fail(&e.to_string()),
    };

    let names: Vec<String> = match args.namespace {
        Some(ns) => vec![ns],
        None => repos.list_repos().iter().map(|r| r.name.clone()).collect(),
    };

    if names.is_empty() {
        fail(&format!("No repos found in {}", args.storage_path));
    }

    let kube_client: Client = match client::try_default().await {
        Ok(x) => x,
        Err(e) => fail(&e.details),
    };

    let cluster = context::current();
    let theme = Theme::new(cluster.env, args.colorblind);

    // pods and git references are fetched in the background, so the table is on screen
    // before the cluster has answered anything
    let mut app = App::new(names, repos, args.mode, cluster, kube_client);

    guard_against_panics();

    let outcome = run(&mut app, &theme);
    restore();

    match outcome {
        Err(e) => fail(&e.to_string()),
        Ok(_) => Ok(()),
    }
}

/// Puts the terminal into its drawing state and runs the loop. Restoring it is left to the
/// caller, so that a failure part way through here cannot strand the user in raw mode.
fn run(app: &mut App, theme: &Theme) -> Result<()> {
    stdout().execute(EnterAlternateScreen)?;
    enable_raw_mode()?;

    let mut terminal: Terminal<CrosstermBackend<std::io::Stdout>> =
        Terminal::new(CrosstermBackend::new(stdout()))?;
    terminal.clear()?;

    // the draw loop blocks its thread; this hands the runtime's other work to another one
    tokio::task::block_in_place(|| process_frames(terminal, app, theme))
        .map_err(|e| Error::other(e.details))
}

/// A panic unwinds past the restore below, which would leave the caller's terminal in raw
/// mode with no way back short of `reset`.
fn guard_against_panics() {
    let report = std::panic::take_hook();

    std::panic::set_hook(Box::new(move |info| {
        restore();
        report(info);
    }));
}

fn restore() {
    let _ = stdout().execute(LeaveAlternateScreen);
    let _ = disable_raw_mode();
}

fn fail(message: &str) -> ! {
    eprintln!("{}", message);
    std::process::exit(1);
}
