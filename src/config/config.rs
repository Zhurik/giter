use crate::git::remote::Mode;
use clap::Parser;

/// Simple utility for quickly checking what commit in this pod
#[derive(Parser)]
#[command(author, version, about, long_about = None)]
pub struct Args {
    /// What a pod is expected to run: the tip of the default branch, or the latest tag
    #[arg(short, long, value_enum, default_value_t = Mode::Commit)]
    pub mode: Mode,

    /// Path to repos.json file
    #[arg(short, long, default_value_t = String::from("./repos.json"))]
    pub storage_path: String,

    /// Show a single namespace instead of every namespace listed in repos.json
    #[arg(short, long)]
    pub namespace: Option<String>,
}
