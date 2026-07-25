use clap::Parser;
use crossterm::{
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use futures::future::join_all;
use giter::config::config::Args;
use giter::drawer::app::{App, NamespaceEntry};
use giter::drawer::terminal::process_frames;
use giter::k8s::client;
use giter::k8s::pods::MyPod;
use giter::storage::common::Storage;
use giter::storage::json_storage::JsonStorage;
use kube::Client;
use ratatui::prelude::*;
use std::io::{stdout, Result};

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

    let namespaces = collect_namespaces(&kube_client, names).await;
    let mut app = App::new(namespaces, repos, args.mode);

    stdout().execute(EnterAlternateScreen)?;
    enable_raw_mode()?;
    let mut terminal: Terminal<CrosstermBackend<std::io::Stdout>> =
        Terminal::new(CrosstermBackend::new(stdout()))?;
    terminal.clear()?;

    let error_msg = match process_frames(terminal, &mut app) {
        Ok(_) => None,
        Err(e) => Some(e.details),
    };

    stdout().execute(LeaveAlternateScreen)?;
    disable_raw_mode()?;

    match error_msg {
        Some(x) => fail(&x),
        None => Ok(()),
    }
}

/// Lists pods in every namespace at once; a namespace that cannot be read stays in
/// the list carrying its error instead of disappearing.
async fn collect_namespaces(client: &Client, names: Vec<String>) -> Vec<NamespaceEntry> {
    let fetches = names
        .iter()
        .map(|name| MyPod::get_pods_by_ns(client, name))
        .collect::<Vec<_>>();

    let results = join_all(fetches).await;

    names
        .into_iter()
        .zip(results)
        .map(|(name, result)| match result {
            Ok(pods) => NamespaceEntry {
                name,
                pods,
                error: None,
            },
            Err(e) => NamespaceEntry {
                name,
                pods: vec![],
                error: Some(e.details),
            },
        })
        .collect()
}

fn fail(message: &str) -> ! {
    eprintln!("{}", message);
    std::process::exit(1);
}
