use clap::Parser;

/// Simple utility for quickly checking what commit in this pod
#[derive(Parser)]
#[command(author, version, about, long_about = None)]
pub struct Args {
    /// Path to repos.json file
    #[arg(short, long, default_value_t = String::from("./repos.json"))]
    pub storage_path: String,

    /// Namespace where images would be checked
    #[arg(short, long)]
    pub namespace: Option<String>,
}
