# Agent interface

Status: proof of concept. Each section says what is built and what is proposed. The full list is in [Status](#status).

A Rein program is a self-contained process for an agent. It works like an app for a person. The agent finds a program, starts it, and works with it. The program holds the state of the run: the answers, the current question, and the trace. The agent does not resend earlier answers.

## The model

Programs are for agents, as apps are for people. The model does not give full enforcement. The agent chooses a program by its description.

| Layer | Required | Role |
| --- | --- | --- |
| CLI | Yes | The base interface. Every agent can use it. |
| MCP | No | A client of the run process, for agents that call MCP tools. |
| Hook | No | Tells the agent that Rein and its programs exist. |

## Agent workflow

1. Find a program. Run `rein list`. Read one with `rein show <name>`. To install programs, see [Install a program](docs/install-programs.md).
2. Start a run. Run `rein start <program> [--input name=value]...`. It starts one run process. It prints the run id and the first step. (Proposed.)
3. Work the run. Answer each step by its run id. Use the CLI, IPC, or MCP. Each channel reaches the same process.
4. Finish. The run ends on a `done` step. To end it early, run `rein stop <run>`. (Proposed.)

### Channels to a run

A run lives in its own process. A command ends when it prints. Claude Code and Codex call tools and cannot keep a stdin open across calls. So the run cannot live in the caller. (Proposed.)

| Channel | Commands or tools |
| --- | --- |
| CLI | `rein answer <run> <value>`, `rein status <run>`, `rein stop <run>`, `rein ps`. `rein ps` lists the live runs of the agent. |
| IPC | A local socket for each run process. The CLI is a thin client of it. |
| MCP | `rein_start`, `rein_answer`, `rein_status`, `rein_stop`. `rein serve --mcp` is a thin client of the sockets. |

- The socket path is a proposal: `$XDG_RUNTIME_DIR/rein/runs/<run>.sock`. If `XDG_RUNTIME_DIR` is not set, use a folder in the user state directory.
- Each reply is a step, in the shape of [spec section 11](spec.md#11-stepping-agent-interface), with the `run` field. An answer uses the `{"key","value"}` line from `rein run --json`.
- `rein_start` starts a run process. The other MCP tools reach its socket. A run that the CLI started shows over MCP, and a run that MCP started shows in the CLI.

### Not channels

The agent does not run or answer a program through these:

- The hook. The hook only says that Rein and its programs exist.
- The file system. The agent does not read `.rein` or state files to run a program. It writes program files only to author them.
- A UI. The agent does not use a UI to run or answer a program.

### `rein step` and `rein run`

- `rein step` is stateless. The agent sends every answer on each call. Rein replays the flow from `main`. It is a test and debug tool, not the agent interface.
- `rein run` stays for people, tests, and embedding in other programs. Agents use the channels above.

## Programs are named apps

Programs are named apps. To install, list, and remove them, see [Install a program](docs/install-programs.md).

- `rein run <name>` starts the program from any directory.
- `rein start <name>` does the same for an agent. (Proposed.)

## Agents can author programs

An agent can write a program, then use it:

1. The agent writes a program folder with a `main.rein` and a manifest. The [install page](docs/install-programs.md) describes the manifest.
2. The agent runs `rein check <folder>`.
3. The agent installs the folder as described in [Install a program](docs/install-programs.md).
4. The agent starts it with `rein start <name>`. (Proposed.) Today it uses `rein run <name>` or `rein_start`.

This is why programs are visible files. When it authors, an agent reads and writes visible files with its normal tools. It does not need to know about a dot-folder. Authoring reads files. Running a program does not. `rein show` gives an agent the source of any installed program. The agent can read a program before it changes or copies it.

## Built today: the MCP host

`rein serve --mcp` is a long-running MCP server on stdio. It holds runs in its own memory. It uses rmcp, the official Rust MCP SDK. It is not yet a client of run processes.

| Tool | Arguments | Returns |
| --- | --- | --- |
| `rein_programs` | none | Each program: name, description, path, scope, and `first_step`. |
| `rein_show` | `name` | The program source. |
| `rein_start` | `name` | The first step, with `run`, the run id. |
| `rein_answer` | `run_id`, `value` | The next step. A wrong value returns an error step. The run stays alive and waits on the same key. |
| `rein_status` | `run_id` | The step the run waits on, with its trace and the answers so far. |
| `rein_stop` | `run_id` | `{"run": ..., "stopped": true}`. The host forgets the run. |
| `rein_list` | none | The live runs, each with its current step. |

- A step has the same shape as a `rein step` response, plus `run`.
- `rein_answer` answers the key the run waits on. The agent sends only the value.
- An unknown run id is a tool error.
- Runs end with `rein_stop` or when the host exits. Saving runs to disk is not built.

Register it in Claude Code:

```text
claude mcp add rein -- rein serve --mcp
```

### Worked example over MCP

This is real output from today's build. A small Python client spoke JSON-RPC to `rein serve --mcp` in `examples/`. `>>` is a message to the host. `<<` is the tool text. Long lines are cut with `...`.

```text
>> {"jsonrpc": "2.0", "method": "tools/list", "id": 2}
<< tools: ['rein_answer', 'rein_list', 'rein_programs', 'rein_show', 'rein_start', 'rein_status', 'rein_stop']
>> tools/call rein_start {"name": "support-ticket"}
<< {"answers":{},"description":"Is the customer on the VIP plan?","function":"main","key":"main.vip","kind":"question","name":"vip","options":null,"run":"run-1","trace":["main"],"type":"boolean","version":1}
>> tools/call rein_answer {"run_id": "run-1", "value": "maybe"}
<< {"answers":{"main.vip":"maybe"},"diagnostics":[],"key":"main.vip","kind":"error","message":"invalid value `maybe` for `vip`; expected true, false, yes or no","run":"run-1","trace":[],"version":1}
>> tools/call rein_answer {"run_id": "run-1", "value": "no"}
<< {"answers":{"main.vip":"no"},"description":"What category best fits the ticket?","function":"main","key":"main.category","kind":"question","name":"category","options":["billing","account","how_to"],"run":"run-1","trace":["main"],"type":"union","version":1}
>> tools/call rein_answer {"run_id": "run-1", "value": "how_to"}
<< {"answers":{"main.category":"how_to","main.vip":"no"},"description":"Did you find a help article that answers the question?","function":"answer/faq","key":"answer/faq.articleFound","kind":"question",...,"run":"run-1","trace":["main","answer/faq"],...}
>> tools/call rein_answer {"run_id": "run-1", "value": "yes"}
<< {...,"instruction":"Reply with the help article link and ask the customer if it solved the problem.","key":"answer/faq.solved","kind":"request",...,"run":"run-1",...}
>> tools/call rein_answer {"run_id": "run-1", "value": "yes"}
<< {"answers":{"answer/faq.articleFound":"yes","answer/faq.solved":"yes","main.category":"how_to","main.vip":"no"},"instruction":"Close the ticket as solved.","kind":"done","run":"run-1","trace":["main","answer/faq"],"version":1}
>> tools/call rein_stop {"run_id": "run-1"}
<< {"run":"run-1","stopped":true}
```

The bad answer `maybe` did not end the run. The next answer went to the same key. The test `crates/rein/tests/mcp.rs` runs the same walk.

## Built today: `rein run` over stdio

`rein run <name>` starts one program as a process. It is for people, tests, and embedding in other programs.

- It writes one step per line to stdout. The step shape is the same as `rein step`.
- It reads one answer per line on stdin.
- With `--json`, an answer line is `{"key": "main.vip", "value": "no"}`. `key` is optional. The default is the key the run waits on.
- Without `--json`, it prints text and a `> ` prompt. A person types the value.
- A bad answer prints an error step. The run then asks the same step again.
- The run ends on `done` with exit code 0. A program that cannot start ends with exit code 1.
- When stdin closes first, the exit code says what the run waits on. It is 2 for a question and 3 for a request.

### Worked example over stdio

Real output, from `examples/`:

```text
$ rein list
incident-triage	project	.../examples/rein/incident-triage	Decide how the on-call agent handles a new production alert.	first: Are customers seeing errors or downtime right now?
merge-pr	project	.../examples/rein/merge-pr	Decide the next step for a pull request task.	first: Which task does the agent do?
support-ticket	project	.../examples/rein/support-ticket	Decide how the support agent handles a customer ticket.	first: Is the customer on the VIP plan?

$ printf 'no\nhow_to\nyes\nyes\n' | rein run support-ticket
question: main.vip: boolean - Is the customer on the VIP plan?
> question: main.category: "billing" | "account" | "how_to" - What category best fits the ticket?
> question: answer/faq.articleFound: boolean - Did you find a help article that answers the question?
> request: Reply with the help article link and ask the customer if it solved the problem.
then answer: answer/faq.solved: boolean - Did the customer confirm the article solved the problem?
> Close the ticket as solved.
trace: main -> answer/faq
# exit 0
```

With `--json`, the same run prints one JSON step per line. The test `run_json_lines_walks_to_done_and_survives_a_bad_answer` in `crates/rein/tests/step.rs` checks it.

After you install `support-ticket` ([Install a program](docs/install-programs.md)), it starts by name from any directory:

```text
$ cd /some/other/dir
$ printf 'yes\n' | rein run support-ticket
question: main.vip: boolean - Is the customer on the VIP plan?
> Assign the ticket to the tier 2 queue. Tell the customer a person will reply within one business day. Do not promise a refund or a fix.
trace: main -> escalate
```

## Built today: `rein step` for replay

Stateless `rein step` does not change. It is for replay, debugging, and tests, not the agent interface. The same answers always give the same step. The old single-function `rein run FUNCTION --set ...` is now `rein eval`.

## Built today: the hook

`rein install claude-code` writes the agent hook. On the first prompt, the hook adds the list of programs to the agent context. The hook is optional. It adds no gate. Nothing stops the agent from skipping a program.

## Status

Checked against `rein --help`.

Built today:

- `rein list` and `rein show <name>`.
- `rein run`: one run over stdio, for people, tests, and embedding.
- `rein step`: stateless replay and debugging.
- `rein serve --mcp`: seven tools. Runs live in the host's own memory.
- `rein install claude-code` and the hook.

Proposed, not built:

- `rein start`, `rein answer`, `rein status`, `rein stop`, and `rein ps`.
- One run process for each run, with a local socket.
- `rein serve --mcp` as a client of the run processes.

Gaps in the built code:

- `rein_start` takes no inputs.
- The `rein step` help text calls it "the agent-facing interface".
- The hook context text tells the agent to fall back to `rein step`. It must change with this model.

## Not in scope

- Graph editors.
- Links between agents.
- Protocol layers above MCP.

## Open questions

- Run lifetime. How long may an idle run wait? Does it time out?
- Attach. May another agent attach to a run that one agent started?
- Sleep and restart. What happens to runs when the machine sleeps or restarts?
- MCP inputs. `rein_start` takes no inputs. Should it take `--input` values, as `rein start` does?

## Conflicts with spec section 11

- Section 11 says the runtime keeps nothing between calls. The evaluator still keeps nothing. The MCP host and `rein run` keep the answers in memory, above the evaluator.
- A step from `rein_start`, `rein_answer` and `rein_status` has a `run` field. A `rein step` response does not. MCP steps list their keys in alphabetical order.
- In `rein step`, a bad answer is an error with exit code 1. In a live run, a bad answer is an error step, and the run goes on.
- Sections 9 and 10 named the single-function command `rein run`. It is now `rein eval`. `rein run` starts a live run.
- Section 12 uses `rein install <tool>` for the hook config.
- The manifest `entry` can name a file other than `main.rein`. The checker still asks for a `main.rein` in each program.
