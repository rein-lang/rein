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
    /// Evaluate one function. Agents should use `rein step`; `run` is the single-function debug command.
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
    /// Step the whole flow from `main`. This is the agent-facing interface: answer one key per call.
    Step {
        /// Function to start from, as its path under the root without `.rein`. Default: `main`.
        #[arg(long)]
        entry: Option<String>,
        /// Project root. Default: the nearest directory with rein.toml.
        #[arg(long)]
        root: Option<PathBuf>,
        /// Answer as key=value, such as `merge_pr.ci=green`. Repeat for each answer.
        #[arg(long = "answer", value_name = "KEY=VALUE")]
        answer: Vec<String>,
        /// Read the whole request as JSON from stdin. Output is always JSON.
        #[arg(long)]
        stdin: bool,
        /// Print the response as JSON.
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
        Command::Step { entry, root, answer, stdin, json } => cli::step(entry, root, &answer, stdin, json),
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
