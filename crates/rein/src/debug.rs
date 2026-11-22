//! `rein debug`: a local web page to play a program. Change inputs and answers, step through the questions, and
//! watch the path the flow takes. The page asks this server; the server walks the flow with the step engine, as
//! `rein step` and `rein graph --answer` do.

use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;

use clap::Args;
use rein_core::ast::{walk_stmts, Input, Stmt};
use rein_core::graph::{self, NodeKind};
use rein_core::project::{Project, ENTRY};
use rein_core::step::{invalid_request, step, type_of, StepRequest};
use serde_json::{json, Value};

use crate::{cli, graph as graph_cmd, http};

/// Flags and operands of `rein debug`.
#[derive(Args)]
pub struct DebugArgs {
    /// Program to debug: a `.rein` file, or a name (see `rein list`). Default: the project around the current directory.
    #[arg(conflicts_with = "root")]
    program: Option<String>,
    /// Project root. Default: the nearest directory with main.rein.
    #[arg(long)]
    root: Option<PathBuf>,
    /// Function to start from, as its path under the root without `.rein`. Default: the program's entry.
    #[arg(long)]
    entry: Option<String>,
    /// First value of a `string` parameter of the entry, as name=value. Repeat for each input.
    #[arg(long = "input", value_name = "NAME=VALUE")]
    input: Vec<String>,
    /// First answer, as key=value, such as `main.task=merge`. Repeat for each answer.
    #[arg(long = "answer", value_name = "KEY=VALUE")]
    answer: Vec<String>,
    /// Port on 127.0.0.1. Default: a free port.
    #[arg(long, default_value_t = 0)]
    port: u16,
    /// Do not open the page in the browser.
    #[arg(long)]
    no_open: bool,
}

pub fn run(a: &DebugArgs) -> u8 {
    match serve(a) {
        Ok(()) => 0,
        Err(message) => {
            eprintln!("error: {message}");
            1
        }
    }
}

fn serve(a: &DebugArgs) -> Result<(), String> {
    let (project, entry) = cli::load_flow(a.root.clone(), a.program.clone())?;
    let mut first = cli::build_request(a.entry.clone(), &a.answer, &a.input, false)?;
    // As `rein step`: the default entry is the program's.
    if first.entry == ENTRY {
        first.entry = entry;
    }
    let listener =
        TcpListener::bind(("127.0.0.1", a.port)).map_err(|e| format!("cannot listen on 127.0.0.1:{}: {e}", a.port))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let url = start_url(&first, port);
    println!("rein debug: {url} (Ctrl-C stops it)");
    if !a.no_open {
        graph_cmd::open(&url);
    }
    let server = Server { project, first, port };
    for stream in listener.incoming().flatten() {
        if let Err(e) = server.answer(stream) {
            eprintln!("warning: {e}");
        }
    }
    Ok(())
}

/// The address of the page with the starting state in its query string, as the page keeps it: `entry`, `in.<name>`
/// for each input, `a.<key>` for each answer. An empty value is unset, so it is left out.
fn start_url(first: &StepRequest, port: u16) -> String {
    let mut query = form_urlencoded::Serializer::new(String::new());
    query.append_pair("entry", &first.entry);
    for (name, value) in first.inputs.iter().filter(|(_, v)| !v.is_empty()) {
        query.append_pair(&format!("in.{name}"), value);
    }
    for (key, value) in first.answers.iter().filter(|(_, v)| !v.is_empty()) {
        query.append_pair(&format!("a.{key}"), value);
    }
    format!("http://127.0.0.1:{port}/?{}", query.finish())
}

/// The loaded program, the values the page starts with, and the port the Host header must name.
struct Server {
    project: Project,
    first: StepRequest,
    port: u16,
}

impl Server {
    /// Reads one request, writes one response, closes. One at a time: the page sends one walk per change.
    fn answer(&self, stream: TcpStream) -> std::io::Result<()> {
        http::answer(stream, self.port, |method, path, body| self.respond(method, path, body))
    }

    /// The response to one request: status, content type, body.
    fn respond(&self, method: &str, path: &str, body: &str) -> http::Reply {
        // The page address carries the state in its query string, which the routes do not read.
        let path = path.split_once('?').map_or(path, |(route, _)| route);
        match (method, path) {
            ("GET", "/") => ("200 OK", "text/html", page(&self.first)),
            ("GET", "/schema") => ("200 OK", "application/json", schema(&self.project, &self.first.entry).to_string()),
            ("POST", "/walk") => ("200 OK", "application/json", walk(&self.project, &self.first.entry, body).to_string()),
            _ => ("404 Not Found", "text/plain", "not found\n".to_string()),
        }
    }
}

/// The page, with the first inputs and answers. `<` is escaped so the JSON cannot close the script.
fn page(first: &StepRequest) -> String {
    let initial = serde_json::to_string(first).unwrap_or_default().replace('<', "\\u003c");
    include_str!("debug_page.html").replace("__MERMAID__", graph_cmd::MERMAID).replace("__INITIAL__", &initial)
}

/// The controls of the page: the functions an entry can be, the `string` inputs of each, and every key the flow
/// can ask for, with its type. `entry` is the one the page starts with.
fn schema(project: &Project, entry: &str) -> Value {
    let (mut functions, mut inputs) = (Vec::new(), serde_json::Map::new());
    let mut answers: Vec<Value> = Vec::new();
    for (name, sf) in project.functions() {
        let f = &sf.function;
        if f.is_template() {
            continue;
        }
        functions.push(name.clone());
        let texts: Vec<Value> =
            f.text_inputs().map(|i| json!({"name": i.name.value, "description": i.description.value})).collect();
        inputs.insert(name.clone(), json!(texts));
        let mut asked: Vec<&Input> = f.inputs.iter().filter(|i| !i.ty.is_text()).collect();
        walk_stmts(&f.body, &mut |s| {
            if let Stmt::Request { response, .. } = s {
                asked.push(response);
            }
        });
        for i in asked {
            let key = format!("{name}.{}", i.name.value);
            if answers.iter().any(|a| a["key"] == key) {
                continue;
            }
            let (ty, options) = type_of(i);
            let fields = f
                .answer_struct(&i.name.value)
                .and_then(|s| s.as_ref().ok())
                .map(|s| s.fields.iter().map(|x| x.name.value.clone()).collect::<Vec<_>>());
            answers.push(json!({"key": key, "type": ty, "options": options, "fields": fields, "description": i.description.value}));
        }
    }
    json!({"entry": entry, "functions": functions, "inputs": inputs, "answers": answers})
}

/// The answer to `POST /walk`: the real step response, the graph with the walk marked as Mermaid text, the answer
/// key of each question node (so a click on one can focus its control), and the `string` inputs left unset.
/// An unset input is walked as the placeholder `<name>`, so the page always has a step to show. `entry` is the
/// default entry.
fn walk(project: &Project, entry: &str, body: &str) -> Value {
    let mut placeholders = Vec::new();
    let (entry, response) = match serde_json::from_str::<StepRequest>(body) {
        Ok(mut request) => {
            // As `rein step`: the default entry is the program's.
            if request.entry == ENTRY {
                request.entry = entry.to_string();
            }
            if let Some(sf) = project.get(&request.entry) {
                for i in sf.function.text_inputs() {
                    let name = &i.name.value;
                    if request.inputs.get(name).is_none_or(|v| v.is_empty()) {
                        request.inputs.insert(name.clone(), format!("<{name}>"));
                        placeholders.push(name.clone());
                    }
                }
            }
            (request.entry.clone(), step(project, &request))
        }
        Err(e) => (entry.to_string(), invalid_request(format!("invalid request JSON: {e}"))),
    };
    let g = graph::walk(project, &entry, &response);
    let keys: serde_json::Map<String, Value> = g
        .nodes
        .iter()
        .filter(|n| n.kind == NodeKind::Question)
        .filter_map(|n| Some((n.id.clone(), json!(format!("{}.{}", n.function, n.name.as_ref()?)))))
        .collect();
    let mermaid = graph::mermaid(&g);
    json!({"step": response, "mermaid": mermaid, "path": g.path, "current": g.current, "keys": keys, "placeholders": placeholders})
}

#[cfg(test)]
#[path = "debug_tests.rs"]
mod tests;
