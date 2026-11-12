//! `rein graph`: prints the program graph as Mermaid, DOT, JSON or an HTML page, or renders it with Graphviz.
//! With `--entry`, `--input` or `--answer`, it steps the flow first, as `rein step` does, and marks the walk.

use std::fs;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use clap::{Args, ValueEnum};
use rein_core::graph::{self, Graph};
use rein_core::project::ENTRY;
use rein_core::step::{invalid_request, step, StepRequest, StepResponse};

use crate::cli;

/// The mermaid build the HTML page loads. Pinned, so a page draws the same later.
const MERMAID: &str = "https://cdn.jsdelivr.net/npm/mermaid@11.4.1/dist/mermaid.esm.min.mjs";

#[derive(Clone, Copy, ValueEnum)]
pub enum GraphFormat {
    Mermaid,
    Dot,
    Json,
    /// One page that draws the Mermaid diagram in a browser.
    Html,
}

/// Flags and operands of `rein graph`.
#[derive(Args)]
pub struct GraphArgs {
    /// Program to draw: a `.rein` file, or a name (see `rein list`). Default: the project around the current directory.
    #[arg(conflicts_with = "root")]
    program: Option<String>,
    /// Project root. Default: the nearest directory with main.rein.
    #[arg(long)]
    root: Option<PathBuf>,
    /// Draw only the references: one node per function, one edge per reference.
    #[arg(long)]
    refs: bool,
    /// Walk the flow from this function, as its path under the root without `.rein`. Default: the program's entry.
    #[arg(long, conflicts_with = "refs")]
    entry: Option<String>,
    /// Walk the flow with this answer, as key=value, such as `main.task=merge`. Repeat for each answer.
    #[arg(long = "answer", value_name = "KEY=VALUE", conflicts_with = "refs")]
    answer: Vec<String>,
    /// Walk the flow with this value of a `string` parameter of the entry, as name=value. Repeat for each input.
    #[arg(long = "input", value_name = "NAME=VALUE", conflicts_with = "refs")]
    input: Vec<String>,
    /// Output format. Default: `html` for a `.html` FILE or with --open, else `mermaid`.
    #[arg(long, value_enum)]
    format: Option<GraphFormat>,
    /// Write to FILE instead of stdout. A `.svg` or `.png` FILE is rendered from DOT by Graphviz `dot`.
    #[arg(long, short, value_name = "FILE")]
    output: Option<PathBuf>,
    /// Open the page in the default browser. Without -o, write it to a temporary file first.
    #[arg(long)]
    open: bool,
}

pub fn run(a: &GraphArgs) -> u8 {
    let (project, entry) = match cli::load_flow(a.root.clone(), a.program.clone()) {
        Ok(found) => found,
        Err(message) => return fail(&message),
    };
    let walking = a.entry.is_some() || !a.answer.is_empty() || !a.input.is_empty();
    let walk = if walking {
        let mut request = match cli::build_request(a.entry.clone(), &a.answer, &a.input, false) {
            Ok(request) => request,
            Err(message) => return stop(&invalid_request(message)),
        };
        // As `rein step`: the default entry is the program's.
        if request.entry == ENTRY {
            request.entry = entry.clone();
        }
        let response = step(&project, &request);
        if matches!(response, StepResponse::Error { .. }) {
            return stop(&response);
        }
        Some((request, response))
    } else {
        None
    };
    let g = match &walk {
        Some((request, response)) => graph::walk(&project, &request.entry, response),
        None if a.refs => graph::refs(&project, &entry),
        None => graph::flow(&project, &entry),
    };
    match write(a, &g, walk.as_ref().map(|(request, _)| request)) {
        Ok(()) => 0,
        Err(message) => fail(&message),
    }
}

/// Prints a step error as `rein step` does, and returns its exit code.
fn stop(response: &StepResponse) -> u8 {
    cli::print_step(response, false);
    cli::exit_code(response)
}

fn fail(message: &str) -> u8 {
    eprintln!("error: {message}");
    1
}

fn write(a: &GraphArgs, g: &Graph, request: Option<&StepRequest>) -> Result<(), String> {
    let ext = a.output.as_deref().and_then(Path::extension).and_then(|e| e.to_str());
    if let (Some(path), Some(ext @ ("svg" | "png"))) = (a.output.as_deref(), ext) {
        return render(&graph::dot(g), ext, path);
    }
    let html_default = ext == Some("html") || a.open;
    let format = a.format.unwrap_or(if html_default { GraphFormat::Html } else { GraphFormat::Mermaid });
    let text = match format {
        GraphFormat::Mermaid => graph::mermaid(g),
        GraphFormat::Dot => graph::dot(g),
        GraphFormat::Json => serde_json::to_string_pretty(g).unwrap_or_default() + "\n",
        GraphFormat::Html => html(g, request),
    };
    let path = match (&a.output, a.open) {
        (Some(path), _) => path.clone(),
        (None, true) => std::env::temp_dir().join(format!("rein-graph-{}.html", std::process::id())),
        (None, false) => {
            print!("{text}");
            return Ok(());
        }
    };
    fs::write(&path, text).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    if a.open {
        eprintln!("opening {}", path.display());
        open(&path);
    }
    Ok(())
}

/// One self-contained page: the Mermaid diagram, drawn in the browser by the pinned mermaid. With a walk, a
/// legend and the entry, inputs and answers used.
fn html(g: &Graph, request: Option<&StepRequest>) -> String {
    let esc = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    let walk = request.map_or_else(String::new, |r| {
        let flags: String = r
            .inputs
            .iter()
            .map(|(k, v)| format!("--input {k}={v}"))
            .chain(r.answers.iter().map(|(k, v)| format!("--answer {k}={v}")))
            .map(|f| format!("<li><code>{}</code></li>", esc(&f)))
            .collect();
        let stop = g.current.as_ref().map_or_else(String::new, |c| match &c.key {
            Some(key) => format!("{} <code>{}</code>", c.kind, esc(key)),
            None => c.kind.to_string(),
        });
        format!(
            "<p class=\"legend\"><span><i class=\"taken\"></i>taken</span><span><i class=\"current\"></i>current: {stop}</span><span><i class=\"dimmed\"></i>not taken</span></p>\n<p>Entry <code>{}</code></p>\n<ul>{flags}</ul>\n",
            esc(&r.entry)
        )
    });
    include_str!("graph_page.html")
        .replace("__TITLE__", &esc(&format!("rein graph {}", g.entry)))
        .replace("__WALK__", &walk)
        .replace("__MERMAID__", MERMAID)
        .replace("__DIAGRAM__", &esc(&graph::mermaid(g)))
}

/// Opens `path` in the default browser: `open` on macOS, `xdg-open` on Linux, nothing elsewhere.
fn open(path: &Path) {
    let tool = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(target_os = "linux") {
        "xdg-open"
    } else {
        return;
    };
    // ponytail: a browser that does not start is not an error; the path is on stderr.
    let _ = Command::new(tool).arg(path).status();
}

/// Renders DOT source to an image with Graphviz `dot`.
fn render(source: &str, ext: &str, path: &Path) -> Result<(), String> {
    let mut child = Command::new("dot")
        .arg(format!("-T{ext}"))
        .arg("-o")
        .arg(path)
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|e| match e.kind() {
            ErrorKind::NotFound => format!(
                "writing {} needs Graphviz: `dot` is not on PATH. Install Graphviz, or write a .mmd, .dot or .json file",
                path.display()
            ),
            _ => format!("cannot run `dot`: {e}"),
        })?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(source.as_bytes()).map_err(|e| format!("cannot write to `dot`: {e}"))?;
    }
    let status = child.wait().map_err(|e| format!("cannot run `dot`: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("`dot` failed ({status}) while writing {}", path.display()))
    }
}
