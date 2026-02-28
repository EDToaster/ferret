mod args;
mod color;

use args::{Cli, Command};
use clap::Parser;

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    match cli.command {
        Command::Search {
            query,
            language: _,
            path: _,
            limit: _,
            format: _,
        } => {
            println!("TODO: implement search (query: {query:?})");
        }
        Command::Files {
            query,
            language: _,
            limit: _,
        } => {
            println!("TODO: implement files (query: {query:?})");
        }
        Command::Symbols {
            query,
            kind: _,
            language: _,
            limit: _,
        } => {
            println!("TODO: implement symbols (query: {query:?})");
        }
        Command::Preview {
            file,
            line: _,
            context: _,
        } => {
            println!("TODO: implement preview (file: {})", file.display());
        }
        Command::Status => {
            println!("TODO: implement status");
        }
        Command::Reindex { full } => {
            println!(
                "TODO: implement reindex (mode: {})",
                if full { "full" } else { "incremental" }
            );
        }
    }
}
