//! `rein` command line: checks, runs and describes Rein projects, and serves the language server.

mod cli;
mod lsp;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "rein", version, about = "A small language for agent decision flows")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Check a project directory, or one .rein file, and print diagnostics.
    Check {
        /// Project directory or .rein file.
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Evaluate one step of a function.
    Run {
        /// Function name, as its path under the root without `.rein`. Default: `main`.
        #[arg(default_value = "main")]
        function: String,
        /// Project root. Default: the nearest directory with rein.toml.
        #[arg(long)]
        root: Option<PathBuf>,
        /// Input value as name=value. Repeat for each input.
        #[arg(long = "set", value_name = "NAME=VALUE")]
        set: Vec<String>,
        /// Print the result as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Print a function's schema as JSON.
    Describe {
        /// Function name. Default: `main`.
        #[arg(default_value = "main")]
        function: String,
        /// Project root. Default: the nearest directory with rein.toml.
        #[arg(long)]
        root: Option<PathBuf>,
    },
    /// Print the reference graph as a Mermaid flowchart.
    Graph {
        /// Project root. Default: the nearest directory with rein.toml.
        #[arg(long)]
        root: Option<PathBuf>,
    },
    /// Run the language server on stdio.
    Lsp,
    /// Print the command reference as Markdown.
    #[command(hide = true)]
    DocsCli,
}

fn main() -> ExitCode {
    let code = match Cli::parse().command {
        Command::Check { path } => cli::check(&path),
        Command::Run { function, root, set, json } => cli::run(&function, root, &set, json),
        Command::Describe { function, root } => cli::describe(&function, root),
        Command::Graph { root } => cli::graph(root),
        Command::Lsp => lsp::serve(),
        Command::DocsCli => {
            let options = clap_markdown::MarkdownOptions::new().show_footer(false);
            print!("{}", clap_markdown::help_markdown_custom::<Cli>(&options));
            0
        }
    };
    ExitCode::from(code)
}
