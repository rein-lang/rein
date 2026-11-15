//! `rein serve --mcp`: the MCP host. A long-running process that holds live runs in memory, so an
//! agent can start a program and answer it across many tool calls.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use rmcp::handler::server::wrapper::Parameters;
use rmcp::{tool, tool_handler, tool_router, ServerHandler, ServiceExt};
use rein_core::step::StepResponse;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::cli::load_flow;
use crate::live_run::Run;
use crate::programs;
use crate::run_log::Log;

// ponytail: runs live until rein_stop or until the server exits; add disk snapshots to resume after a crash.
#[derive(Clone)]
struct Host {
    runs: Arc<Mutex<BTreeMap<String, Run>>>,
    next: Arc<AtomicUsize>,
    log: Arc<Mutex<Log>>,
}

#[derive(Deserialize, JsonSchema)]
struct Name {
    /// The program name, as rein_programs lists it.
    name: String,
}

#[derive(Deserialize, JsonSchema)]
struct RunId {
    /// The run id that rein_start returned.
    run_id: String,
}

#[derive(Deserialize, JsonSchema)]
struct Answer {
    /// The run id that rein_start returned.
    run_id: String,
    /// The answer to the key the run waits on: yes, no, or one of the options.
    value: String,
}

/// The step as JSON, with the run id added as `run`.
fn step_value(run_id: Option<&str>, step: &StepResponse) -> Value {
    let mut v = serde_json::to_value(step).unwrap_or_default();
    if let (Some(id), Some(map)) = (run_id, v.as_object_mut()) {
        map.insert("run".into(), id.into());
    }
    v
}

fn unknown(run_id: &str) -> String {
    format!("unknown run `{run_id}`; rein_list shows the live runs")
}

impl Host {
    fn new() -> Host {
        let mut log = Log::process("mcp");
        log.announce();
        Host {
            runs: Arc::default(),
            next: Arc::default(),
            log: Arc::new(Mutex::new(log)),
        }
    }

    fn runs(&self) -> MutexGuard<'_, BTreeMap<String, Run>> {
        self.runs.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn log(&self) -> MutexGuard<'_, Log> {
        self.log.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[tool_router]
impl Host {
    #[tool(description = "List the Rein programs: name, description, and first_step, the step a new run starts on. Pick a program by what it does.")]
    fn rein_programs(&self) -> String {
        Value::Array(programs::catalog()).to_string()
    }

    #[tool(description = "Return the source of a Rein program: its manifest and every .rein file.")]
    fn rein_show(&self, Parameters(Name { name }): Parameters<Name>) -> Result<String, String> {
        programs::source(&name)
    }

    #[tool(description = "Start a Rein program as a live run. Returns `run` (the run id) and the first step. Answer each question or request with rein_answer until kind is done, then do the done instruction.")]
    fn rein_start(&self, Parameters(Name { name }): Parameters<Name>) -> Result<String, String> {
        let (project, entry) = load_flow(None, Some(name.clone()))?;
        let run = Run::start(project, entry.clone());
        if matches!(run.last(), StepResponse::Error { .. }) {
            // A failed start has no run id, so its log line has `run` null.
            self.log().start(None, Some(&name), &entry, run.last());
            return Err(step_value(None, run.last()).to_string());
        }
        let id = format!("run-{}", self.next.fetch_add(1, Ordering::Relaxed) + 1);
        let out = step_value(Some(&id), run.last()).to_string();
        self.log().start(Some(&id), Some(&name), &entry, run.last());
        self.runs().insert(id, run);
        Ok(out)
    }

    #[tool(description = "Answer the key the run waits on. For a request, do the instruction first. Returns the next step. A wrong value returns an error step; the run stays alive and waits on the same key.")]
    fn rein_answer(&self, Parameters(Answer { run_id, value }): Parameters<Answer>) -> Result<String, String> {
        let mut runs = self.runs();
        let run = runs.get_mut(&run_id).ok_or_else(|| unknown(&run_id))?;
        let key = run.asked().unwrap_or_default().to_string();
        let next = run.answer(None, value.clone());
        self.log().answer(&run_id, &key, &value, &next);
        Ok(step_value(Some(&run_id), &next).to_string())
    }

    #[tool(description = "Return the step a run waits on, with its trace and the answers so far.")]
    fn rein_status(&self, Parameters(RunId { run_id }): Parameters<RunId>) -> Result<String, String> {
        let runs = self.runs();
        let run = runs.get(&run_id).ok_or_else(|| unknown(&run_id))?;
        Ok(step_value(Some(&run_id), run.last()).to_string())
    }

    #[tool(description = "Stop a run and forget it.")]
    fn rein_stop(&self, Parameters(RunId { run_id }): Parameters<RunId>) -> Result<String, String> {
        self.runs().remove(&run_id).ok_or_else(|| unknown(&run_id))?;
        self.log().stop(&run_id);
        Ok(json!({"run": run_id, "stopped": true}).to_string())
    }

    #[tool(description = "List the live runs, each with the step it waits on.")]
    fn rein_list(&self) -> String {
        let runs = self.runs();
        Value::Array(runs.iter().map(|(id, run)| step_value(Some(id), run.last())).collect()).to_string()
    }
}

#[tool_handler(
    name = "rein",
    version = "0.1.0",
    instructions = "Rein programs are step-by-step processes. Find one with rein_programs, start it with rein_start, and answer each step with rein_answer until kind is done. Follow the program instead of guessing."
)]
impl ServerHandler for Host {}

/// Serves MCP on stdio until the client closes it.
pub fn serve() -> u8 {
    let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("error: cannot start the runtime: {e}");
            return 1;
        }
    };
    let result: Result<(), Box<dyn std::error::Error>> = runtime.block_on(async {
        Host::new().serve(rmcp::transport::stdio()).await?.waiting().await?;
        Ok(())
    });
    match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}
