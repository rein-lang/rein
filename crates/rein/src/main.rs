//! `rein` command line: checks, runs and describes Rein projects, and serves the language server.

mod cli;
mod hook;
mod install;
mod lsp;
mod scopes;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};

#[derive(Clone, Copy, ValueEnum)]
enum Format {
    Text,
    Json,
}

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
        /// Project root. Default: the nearest directory with main.rein.
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
        /// Project root. Default: the nearest directory with main.rein.
        #[arg(long, conflicts_with = "flow")]
        root: Option<PathBuf>,
        /// Flow to step, by name, from the scoped `.rein/` folders (see `rein flows`).
        #[arg(long, conflicts_with = "root")]
        flow: Option<String>,
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
        /// Project root. Default: the nearest directory with main.rein.
        #[arg(long)]
        root: Option<PathBuf>,
    },
    /// Print the reference graph as a Mermaid flowchart.
    Graph {
        /// Project root. Default: the nearest directory with main.rein.
        #[arg(long)]
        root: Option<PathBuf>,
    },
    /// List the flows visible from here, by scope, with shadowed ones marked.
    Flows {
        /// Print the list as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Print the block an agent needs to use the visible flows.
    AgentContext {
        /// Output format.
        #[arg(long, value_enum, default_value = "text")]
        format: Format,
    },
    /// The agent hook entry point. Reads one event on stdin. Run by the agent, not by hand.
    Hook {
        /// The agent that sends the event, such as `claude-code`. Sets polyhook's caller.
        #[arg(long)]
        caller: Option<String>,
    },
    /// Write a tool's hook config so it runs `rein hook`. Only `claude-code` is supported.
    Install {
        /// The agent tool, such as `claude-code`.
        tool: String,
        /// Write to the project's settings (`.claude/settings.json`). Default.
        #[arg(long, conflicts_with = "user")]
        project: bool,
        /// Write to the user's settings (`~/.claude/settings.json`).
        #[arg(long)]
        user: bool,
        /// Print the resulting file and write nothing.
        #[arg(long)]
        dry_run: bool,
    },
    /// Run the language server on stdio.
    Lsp,
}

fn main() -> ExitCode {
    let code = match Cli::parse().command {
        Command::Check { path } => cli::check(&path),
        Command::Run { function, root, set, json } => cli::run(&function, root, &set, json),
        Command::Step { entry, root, flow, answer, stdin, json } => {
            cli::step(entry, root, flow, &answer, stdin, json)
        }
        Command::Describe { function, root } => cli::describe(&function, root),
        Command::Graph { root } => cli::graph(root),
        Command::Flows { json } => cli::flows(json),
        Command::AgentContext { format } => cli::agent_context(matches!(format, Format::Json)),
        Command::Hook { caller } => hook::run(caller.as_deref()),
        Command::Install { tool, project: _, user, dry_run } => {
            let target = if user { install::Target::User } else { install::Target::Project };
            install::run(&tool, target, dry_run)
        }
        Command::Lsp => lsp::serve(),
    };
    ExitCode::from(code)
}
