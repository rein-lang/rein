//! `rein serve --mcp`: the MCP host. A run is saved to a file, so an agent can start a program and answer it across
//! many tool calls, and the run survives a restart of this server.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};

use rmcp::handler::server::wrapper::Parameters;
use rmcp::{tool, tool_handler, tool_router, ServerHandler, ServiceExt};
use rein_core::step::StepResponse;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::programs;
use crate::run_log::Log;
use crate::saved_run;

#[derive(Clone)]
struct Host {
    log: Arc<Mutex<Log>>,
}

#[derive(Deserialize, JsonSchema)]
struct Name {
    /// The program name, as rein_programs lists it.
    name: String,
}

#[derive(Deserialize, JsonSchema)]
struct Start {
    /// The program name, as rein_programs lists it.
    name: String,
    /// Values of the program's string, format and schema inputs, by name. Give them once here. The run keeps them.
    #[serde(default)]
    inputs: BTreeMap<String, String>,
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

impl Host {
    fn new() -> Host {
        let mut log = Log::process("mcp");
        log.announce();
        Host { log: Arc::new(Mutex::new(log)) }
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

    #[tool(description = "Start a Rein program as a saved run. Pass its inputs once here: the run keeps them, so later calls send only answers. Returns `run` (the run id) and the first step. Answer each question or request with rein_answer until kind is done, then do the done instruction.")]
    fn rein_start(&self, Parameters(Start { name, inputs }): Parameters<Start>) -> Result<String, String> {
        let (run, entry, first) = saved_run::start(&name, inputs);
        self.log().start(run.as_deref(), Some(&name), &entry, &first);
        if matches!(first, StepResponse::Error { .. }) {
            return Err(saved_run::value(None, &first).to_string());
        }
        Ok(saved_run::value(run.as_deref(), &first).to_string())
    }

    #[tool(description = "Answer the key the run waits on. For a request, do the instruction first. Returns the next step. A wrong value returns an error step; the run stays alive and waits on the same key. The run keeps its inputs and earlier answers, so send only the new answer.")]
    fn rein_answer(&self, Parameters(Answer { run_id, value }): Parameters<Answer>) -> Result<String, String> {
        let (key, next) = saved_run::answer_asked(&run_id, value.clone());
        self.log().answer(&run_id, &key, &value, &next);
        Ok(saved_run::value(Some(&run_id), &next).to_string())
    }

    #[tool(description = "Return the step a run waits on, computed from its saved inputs and answers.")]
    fn rein_status(&self, Parameters(RunId { run_id }): Parameters<RunId>) -> Result<String, String> {
        let step = saved_run::status(&run_id);
        if matches!(step, StepResponse::Error { .. }) {
            return Err(saved_run::value(Some(&run_id), &step).to_string());
        }
        Ok(saved_run::value(Some(&run_id), &step).to_string())
    }

    #[tool(description = "Stop a run and delete its saved file.")]
    fn rein_stop(&self, Parameters(RunId { run_id }): Parameters<RunId>) -> Result<String, String> {
        saved_run::stop(&run_id)?;
        self.log().stop(&run_id);
        Ok(json!({"run": run_id, "stopped": true}).to_string())
    }

    #[tool(description = "List the saved runs, each with the step it waits on and its program.")]
    fn rein_list(&self) -> String {
        Value::Array(saved_run::list()).to_string()
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
