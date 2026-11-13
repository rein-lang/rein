// Every value from the log is set with textContent. The log holds strings that agents wrote. The graph is Mermaid
// text from the server, drawn with securityLevel "strict", as the debug page draws it.
const MERMAID_URL = "__MERMAID__";
const agentsBox = document.getElementById("agents");
const endedBox = document.getElementById("ended");
const statusLine = document.getElementById("status");
const summaryBox = document.getElementById("summary");
const dark = matchMedia("(prefers-color-scheme: dark)");
const GLYPH = { waiting: "◐", done: "✓", stopped: "■", error: "✕", alive: "●", ended: "○" };
let last = [];
// Open state of each details, by key. Graph boxes persist across polls, keyed the same way.
const openState = new Map();
const graphs = new Map();
let mermaidPromise = null;
let renderSeq = 0;
let renders = 0;

function el(tag, cls, text) {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text !== undefined && text !== null) e.textContent = String(text);
  return e;
}
const chip = (kind, text) => el("span", "chip " + kind, GLYPH[kind] + " " + text);
const short = (v) => (v === null || v === undefined || v === "" ? "-" : typeof v === "string" ? v : JSON.stringify(v));

// A details with a closed default. Its open state survives each poll, and it can take keyboard focus again.
function details(key, summary, cls) {
  const d = el("details", cls);
  const s = el("summary", null, summary);
  s.dataset.focusKey = key;
  d.append(s);
  d.open = openState.get(key) === true;
  d.addEventListener("toggle", () => {
    if (d.open) openState.set(key, true);
    else openState.delete(key);
  });
  return d;
}

function defs(rows) {
  const list = el("dl");
  for (const [label, value] of rows) list.append(el("dt", null, label), el("dd", null, short(value)));
  return list;
}

function nowLabel(step) {
  if (!step) return "no step yet";
  if (step.kind === "question" || step.kind === "request") return step.key || step.kind;
  return "done";
}

function pills(run) {
  const box = el("div", "steps");
  box.setAttribute("aria-label", "Functions on the path, in order");
  const trace = Array.isArray(run.trace) ? run.trace : [];
  const current = run.status === "waiting" || run.status === "error" ? trace.length - 1 : -1;
  trace.forEach((name, i) => box.append(el("span", i === current ? "pill current" : "pill", name)));
  return box;
}

// The graph of a run: "Path" (the functions on its trace) or "Whole program". One box per run, kept across polls.
function graphFor(key, run) {
  let g = graphs.get(key);
  if (!g) {
    g = { wrap: el("div", "graph-wrap"), box: el("div", "graph"), mode: "path", drawn: null, want: null, token: 0, saved: null, texts: {} };
    const modes = el("div", "modes");
    modes.setAttribute("role", "group");
    modes.setAttribute("aria-label", "Graph");
    const buttons = { path: el("button", "mode", "Path"), full: el("button", "mode", "Whole program") };
    for (const [mode, button] of Object.entries(buttons)) {
      button.type = "button";
      button.setAttribute("aria-pressed", String(mode === g.mode));
      button.addEventListener("click", () => {
        g.mode = mode;
        for (const [m, b] of Object.entries(buttons)) b.setAttribute("aria-pressed", String(m === mode));
        if (!ensure(g)) {
          sizeSvg(g);
          centerCurrent(g);
        }
      });
    }
    modes.append(buttons.path, buttons.full);
    g.wrap.append(modes, g.box);
    graphs.set(key, g);
  }
  g.texts = { path: run.mermaid, full: run.mermaid_full };
  g.seen = renderSeq;
  ensure(g);
  return g.wrap;
}

// Draws the text of the chosen mode unless the box already holds it. Returns true when a draw starts.
function ensure(g) {
  const text = g.texts[g.mode];
  if (!text || g.drawn === text || g.want === text) return false;
  g.want = text;
  drawGraph(g, text);
  return true;
}

// Fit to the box width, so the whole graph shows. When the fit would make labels smaller than MIN_LABEL_PX, the
// graph keeps natural size and the box scrolls. The viewBox keeps the aspect ratio.
const LABEL_PX = 16;
const MIN_LABEL_PX = 11;
function sizeSvg(g) {
  const svg = g.box.querySelector("svg");
  if (!svg) return;
  const vb = svg.viewBox.baseVal;
  const room = g.box.clientWidth - 18; // padding and border of the box
  const fit = vb.width ? Math.min(1, room / vb.width) : 1;
  const natural = fit * LABEL_PX < MIN_LABEL_PX || fit >= 1;
  g.box.classList.toggle("natural", natural);
  svg.removeAttribute("width");
  svg.removeAttribute("height");
  svg.style.maxWidth = "none";
  svg.style.height = "auto";
  svg.style.width = natural && vb.width ? vb.width + "px" : "100%";
}

// Puts the current node in the middle of the box. Called after each draw of the graph, never on a plain poll.
function centerCurrent(g) {
  const node = g.box.querySelector("g.node.current");
  if (!node) return;
  const r = node.getBoundingClientRect(), b = g.box.getBoundingClientRect();
  g.box.scrollLeft += r.left + r.width / 2 - (b.left + g.box.clientWidth / 2);
  g.box.scrollTop += r.top + r.height / 2 - (b.top + g.box.clientHeight / 2);
}

function loadMermaid() {
  if (!mermaidPromise) {
    mermaidPromise = import(MERMAID_URL).then((m) => {
      m.default.initialize({ startOnLoad: false, securityLevel: "strict", theme: dark.matches ? "dark" : "default" });
      return m.default;
    });
  }
  return mermaidPromise;
}

// Draws the graph only when its text changed. A poll with the same text keeps the drawn SVG and its scroll position.
async function drawGraph(g, text) {
  const token = ++g.token;
  try {
    const mermaid = await loadMermaid();
    const { svg } = await mermaid.render("watch" + renders++, text);
    if (token !== g.token) return;
    g.box.innerHTML = svg;
    g.drawn = text;
    sizeSvg(g);
    centerCurrent(g);
  } catch (e) {
    if (token === g.token) g.box.textContent = "The graph cannot be drawn here (the Mermaid script did not load, or the program changed).";
  }
}

function runView(agent, run, newest) {
  const key = agent.id + "|" + run.run;
  const box = el("section", "run");
  const head = el("div", "run-head");
  head.append(el("h3", null, run.program || "(no program)"), chip(run.status, run.status));
  // A done run shows its outcome, the label of its final node, in place of a now line.
  if (run.status === "done") {
    const text = String(run.step.instruction || "");
    head.append(el("span", "outcome", text.length > 60 ? text.slice(0, 59) + "…" : text));
    box.append(head, pills(run));
  } else {
    const now = el("p", "now");
    now.append(el("span", "now-label", "now"), el("strong", null, nowLabel(run.step)));
    box.append(head, now, pills(run));
  }
  if (newest && run.mermaid) box.append(graphFor(key, run));
  else if (newest && run.program_changed) box.append(el("div", "graph graph-note", "Program changed since this run"));
  if (run.step && run.step.kind === "question") {
    const d = details(key + "|question", "Question text", "text");
    d.append(el("p", null, run.step.description || "(no text)"));
    if (Array.isArray(run.step.options)) d.append(defs([["options", run.step.options.map(short).join(" | ")]]));
    box.append(d);
  } else if (run.step && run.step.kind) {
    const d = details(key + "|instruction", "Instruction", "text");
    d.append(el("p", null, run.step.instruction || "(no text)"));
    box.append(d);
  }
  const answers = run.functions.flatMap((f) => Object.entries(f.answers).map(([k, v]) => [f.name + "." + k, v]));
  if (answers.length) {
    const d = details(key + "|answers", "Answers (" + answers.length + ")");
    d.append(defs(answers));
    box.append(d);
  }
  const inputs = Object.entries(run.inputs || {});
  if (inputs.length) {
    const d = details(key + "|inputs", "Inputs (" + inputs.length + ")");
    d.append(defs(inputs));
    box.append(d);
  }
  if (run.error) {
    const d = details(key + "|error", "Last error");
    d.append(el("p", "error", run.error));
    box.append(d);
  }
  const d = details(key + "|log", "Log lines (" + run.events.length + ")");
  d.append(el("pre", null, JSON.stringify(run.events, null, 2)));
  box.append(d);
  return box;
}

function agentView(a) {
  const title = typeof a.cwd === "string" ? a.cwd.split("/").filter(Boolean).pop() : "";
  const card = el("article", a.alive ? "agent" : "agent ended");
  const head = el("div", "agent-head");
  head.append(
    el("span", "dot " + (a.alive ? "alive" : "ended")),
    el("span", a.alive ? "alive" : "ended", a.alive ? "alive" : "ended"),
    el("h2", null, title || "(no folder)"),
    el("span", "chip transport", a.transport || "unknown"),
  );
  card.append(head);
  if (a.errors && a.errors.length) {
    const d = details(a.id + "|errors", "Start errors (" + a.errors.length + ")", "text");
    for (const e of a.errors) d.append(el("p", "error", (e.program ? e.program + ": " : "") + e.message));
    card.append(d);
  }
  const who = details(a.id + "|details", "Agent details", "text");
  who.append(defs([["cwd", a.cwd], ["session", a.session], ["pid", a.pid]]));
  card.append(who);
  if (!a.runs.length) card.append(el("p", "meta", "No runs."));
  const [newest, ...earlier] = a.runs;
  if (newest) card.append(runView(a, newest, true));
  if (earlier.length) {
    const d = details(a.id + "|earlier", earlier.length + (earlier.length === 1 ? " earlier run" : " earlier runs"), "earlier");
    for (const r of earlier) d.append(runView(a, r, false));
    card.append(d);
  }
  return card;
}

// The counters: agents alive in the whole list, and the newest run of each agent shown. Older runs are folded.
function summary(list, shown) {
  const runs = shown.map((a) => a.runs[0]).filter(Boolean);
  const count = (status) => runs.filter((r) => r.status === status).length;
  const alive = list.filter((a) => a.alive).length;
  return [chip("alive", alive + " alive"), chip("waiting", count("waiting") + " waiting"), chip("done", count("done") + " done"), chip("error", count("error") + " error")];
}

// Keeps the focus on the same summary after a poll rebuilds the page.
// One card per agent. A card that cannot be built shows a short line, and the other cards still render.
function safeAgentView(a) {
  try {
    return agentView(a);
  } catch (e) {
    return el("article", "agent", "Cannot show this agent.");
  }
}

function render(list) {
  const shown = list.filter((a) => endedBox.checked || a.alive);
  const focused = document.activeElement && document.activeElement.dataset.focusKey;
  // Moving a box into a new card resets its scroll. The position is read before the cards are built, and put back
  // after, so a poll keeps the reader's place.
  for (const g of graphs.values()) g.saved = g.box.isConnected ? [g.box.scrollLeft, g.box.scrollTop] : null;
  const seq = ++renderSeq;
  const cards = shown.map(safeAgentView);
  if (!cards.length) cards.push(el("p", "empty", "No agents use rein right now."));
  agentsBox.replaceChildren(...cards);
  summaryBox.replaceChildren(...summary(list, shown));
  for (const g of graphs.values()) if (g.saved && g.box.isConnected) [g.box.scrollLeft, g.box.scrollTop] = g.saved;
  // Graphs of runs that are no longer the newest run of a shown agent are dropped.
  const ids = new Set(list.map((a) => a.id));
  for (const [key, g] of graphs) if (g.seen !== seq) graphs.delete(key);
  for (const key of openState.keys()) if (!ids.has(key.split("|")[0])) openState.delete(key);
  if (focused) agentsBox.querySelector('[data-focus-key="' + CSS.escape(focused) + '"]')?.focus({ preventScroll: true });
}

async function poll() {
  try {
    const res = await fetch("/api/agents", { cache: "no-store" });
    if (!res.ok) throw new Error("HTTP " + res.status);
    last = await res.json();
    render(last);
    statusLine.textContent = "updated " + new Date().toLocaleTimeString();
  } catch (e) {
    statusLine.textContent = "cannot read agents: " + e.message;
  }
}

// A theme change redraws every graph in the new colors.
dark.addEventListener("change", async () => {
  const m = await loadMermaid();
  m.initialize({ startOnLoad: false, securityLevel: "strict", theme: dark.matches ? "dark" : "default" });
  for (const g of graphs.values()) g.drawn = g.want = null;
  render(last);
});
endedBox.addEventListener("change", () => render(last));
poll();
setInterval(poll, 2000);
