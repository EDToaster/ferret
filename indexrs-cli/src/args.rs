use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

/// Local code search index — fast grep, file, and symbol search for your repositories.
#[derive(Debug, Parser)]
#[command(name = "indexrs", version, about = "Local code search index")]
pub struct Cli {
    /// Color output mode
    #[arg(long, value_enum, default_value_t = ColorMode::Auto, global = true)]
    pub color: ColorMode,

    /// Repository root path (default: current directory)
    #[arg(short = 'r', long, value_name = "PATH", global = true)]
    pub repo: Option<PathBuf>,

    /// Increase verbosity (can repeat: -vv for debug)
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    pub verbose: u8,

    #[command(subcommand)]
    pub command: Command,
}

/// Color output mode
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ColorMode {
    /// Automatic: color when stdout is a TTY
    Auto,
    /// Always emit color codes
    Always,
    /// Never emit color codes
    Never,
}

/// Output format for search results
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum OutputFormat {
    /// Grep-compatible format (file:line:col:content)
    Grep,
    /// JSON output
    Json,
    /// Human-readable pretty output
    Pretty,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Search code in indexed files
    Search {
        /// Search query string
        query: String,

        /// Filter by programming language
        #[arg(short = 'l', long, value_name = "LANG")]
        language: Option<String>,

        /// Filter by path pattern
        #[arg(short, long, value_name = "PATTERN")]
        path: Option<String>,

        /// Maximum number of results
        #[arg(short = 'n', long, default_value_t = 50)]
        limit: usize,

        /// Output format
        #[arg(long, value_enum, default_value_t = OutputFormat::Grep)]
        format: OutputFormat,
    },

    /// Search file names and paths
    Files {
        /// Optional query to filter file names
        query: Option<String>,

        /// Filter by programming language
        #[arg(short = 'l', long, value_name = "LANG")]
        language: Option<String>,

        /// Maximum number of results
        #[arg(short = 'n', long)]
        limit: Option<usize>,
    },

    /// Search symbols (functions, types, constants)
    Symbols {
        /// Optional query to filter symbols
        query: Option<String>,

        /// Filter by symbol kind (fn, struct, trait, enum, etc.)
        #[arg(short = 'k', long, value_name = "KIND")]
        kind: Option<String>,

        /// Filter by programming language
        #[arg(short = 'l', long, value_name = "LANG")]
        language: Option<String>,

        /// Maximum number of results
        #[arg(short = 'n', long)]
        limit: Option<usize>,
    },

    /// Preview file contents with syntax highlighting
    Preview {
        /// File to preview
        file: PathBuf,

        /// Jump to line number
        #[arg(long)]
        line: Option<usize>,

        /// Lines of context around matches
        #[arg(short = 'C', long)]
        context: Option<usize>,
    },

    /// Show index status (file count, last update, etc.)
    Status,

    /// Trigger reindex of the repository
    Reindex {
        /// Perform a full reindex (default: incremental)
        #[arg(long)]
        full: bool,
    },
}
