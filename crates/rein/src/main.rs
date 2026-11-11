//! `rein` command line: checks, runs and describes Rein projects, and serves the language server.

mod cli;
mod graph;
mod hook;
mod install;
mod live_run;
mod lsp;
mod mcp;
mod programs;
mod scopes;

use std::path::{Path, PathBuf};
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
    /// Run a flow as a live process. Writes one step per line, reads one answer per line on stdin.
    Run {
        /// Program to run: a `.rein` file, or a name (see `rein list`).
        #[arg(conflicts_with = "root")]
        flow: Option<String>,
        /// Project root. Default: the nearest directory with main.rein.
        #[arg(long)]
        root: Option<PathBuf>,
        /// Function to start from, as its path under the root without `.rein`. Default: `main`.
        #[arg(long)]
        entry: Option<String>,
        /// Speak JSON Lines: one step object out, one `{"key","value"}` answer in, per line.
        #[arg(long)]
        json: bool,
    },
    /// Evaluate one function, for debugging. Agents use `rein run` or `rein step`.
    Eval {
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
        /// Program to step: a `.rein` file (its folder is the project root), or a name (see `rein list`).
        #[arg(conflicts_with_all = ["flow", "root", "entry"])]
        program: Option<String>,
        /// Function to start from, as its path under the root without `.rein`. Default: `main`.
        #[arg(long)]
        entry: Option<String>,
        /// Project root. Default: the nearest directory with main.rein.
        #[arg(long, conflicts_with = "flow")]
        root: Option<PathBuf>,
        /// Program to step, by name (see `rein list`). Same as the positional program.
        #[arg(long, conflicts_with = "root")]
        flow: Option<String>,
        /// Answer as key=value, such as `merge_pr.ci=green`. Repeat for each answer.
        #[arg(long = "answer", value_name = "KEY=VALUE")]
        answer: Vec<String>,
        /// Value of a `string` parameter of the entry function, as name=value. Repeat for each input.
        #[arg(long = "input", value_name = "NAME=VALUE")]
        input: Vec<String>,
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
    /// Print the program graph: functions, questions, branches, outcomes, and hand-offs.
    Graph {
        /// Program to draw: a `.rein` file, or a name (see `rein list`). Default: the project around the current directory.
        #[arg(conflicts_with = "root")]
        program: Option<String>,
        /// Project root. Default: the nearest directory with main.rein.
        #[arg(long)]
        root: Option<PathBuf>,
        /// Draw only the references: one node per function, one edge per reference.
        #[arg(long)]
        refs: bool,
        /// Output format.
        #[arg(long, value_enum, default_value = "mermaid")]
        format: graph::GraphFormat,
        /// Write to FILE instead of stdout. A `.svg` or `.png` FILE is rendered from DOT by Graphviz `dot`.
        #[arg(long, short, value_name = "FILE")]
        output: Option<PathBuf>,
    },
    /// List the programs visible from here: name, scope, path, description, and first question.
    #[command(alias = "flows")]
    List {
        /// Print the list as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Print a program's source: its manifest and every .rein file.
    Show {
        /// Program name (see `rein list`).
        name: String,
    },
    /// Serve the agent interface. Holds live runs in memory.
    Serve {
        /// Speak MCP on stdio. The only transport today.
        #[arg(long)]
        mcp: bool,
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
    /// Register a program folder in the user programs folder, or write a tool's hook config (`claude-code`).
    Install {
        /// A program folder to register, or the agent tool, such as `claude-code`.
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
    /// Print the command reference as Markdown.
    #[command(hide = true)]
    DocsCli,
}

fn main() -> ExitCode {
    let code = match Cli::parse().command {
        Command::Check { path } => cli::check(&path),
        Command::Run { flow, root, entry, json } => live_run::run(flow, root, entry, json),
        Command::Eval { function, root, set, json } => cli::eval(&function, root, &set, json),
        Command::Step { program, entry, root, flow, answer, input, stdin, json } => {
            cli::step(entry, root, flow.or(program), &answer, &input, stdin, json)
        }
        Command::Describe { function, root } => cli::describe(&function, root),
        Command::Graph { program, root, refs, format, output } => graph::run(program, root, refs, format, output.as_deref()),
        Command::List { json } => programs::list(json),
        Command::Show { name } => programs::show(&name),
        Command::Serve { mcp: true } => mcp::serve(),
        Command::Serve { mcp: false } => {
            eprintln!("error: only `rein serve --mcp` is supported");
            1
        }
        Command::AgentContext { format } => cli::agent_context(matches!(format, Format::Json)),
        Command::Hook { caller } => hook::run(caller.as_deref()),
        Command::Install { tool, project: _, user, dry_run } => {
            if Path::new(&tool).is_dir() {
                programs::install(Path::new(&tool))
            } else {
                let target = if user { install::Target::User } else { install::Target::Project };
                install::run(&tool, target, dry_run)
            }
        }
        Command::Lsp => lsp::serve(),
        Command::DocsCli => {
            let options = clap_markdown::MarkdownOptions::new().show_footer(false);
            print!("{}", clap_markdown::help_markdown_custom::<Cli>(&options));
            0
        }
    };
    ExitCode::from(code)
}
