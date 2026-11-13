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

function graphFor(key, text) {
  let g = graphs.get(key);
  if (!g) {
    g = { wrap: el("div", "graph-wrap"), box: el("div", "graph"), zoom: false, drawn: null, want: null, token: 0, scrollLeft: 0, scrollTop: 0, centered: false };
    const zoom = el("button", "zoom", "Zoom");
    zoom.type = "button";
    zoom.setAttribute("aria-pressed", "false");
    zoom.title = "Show the graph at natural size, scrolled inside its box";
    zoom.addEventListener("click", () => {
      g.zoom = !g.zoom;
      zoom.setAttribute("aria-pressed", String(g.zoom));
      g.centered = false;
      sizeSvg(g);
      centerCurrent(g);
    });
    g.box.addEventListener("scroll", () => {
      g.scrollLeft = g.box.scrollLeft;
      g.scrollTop = g.box.scrollTop;
    });
    g.wrap.append(zoom, g.box);
    graphs.set(key, g);
  }
  // `want` stops a poll from starting a second draw of text that is already on its way.
  if (g.drawn !== text && g.want !== text) {
    g.want = text;
    drawGraph(g, text);
  }
  return g.wrap;
}

// Fit to the box width (the whole graph), or natural size in zoom. The viewBox keeps the aspect ratio.
function sizeSvg(g) {
  const svg = g.box.querySelector("svg");
  g.box.classList.toggle("natural", g.zoom);
  if (!svg) return;
  const vb = svg.viewBox.baseVal;
  svg.removeAttribute("width");
  svg.removeAttribute("height");
  svg.style.maxWidth = "none";
  svg.style.height = "auto";
  svg.style.width = g.zoom && vb.width ? vb.width + "px" : "100%";
}

// Puts the current node in the middle of the box, so it shows without a scroll.
function centerCurrent(g) {
  const node = g.box.querySelector("g.node.current");
  if (!node || g.centered) return;
  g.centered = true;
  const r = node.getBoundingClientRect(), b = g.box.getBoundingClientRect();
  g.box.scrollLeft += r.left + r.width / 2 - (b.left + g.box.clientWidth / 2);
  g.box.scrollTop += r.top + r.height / 2 - (b.top + g.box.clientHeight / 2);
  g.scrollLeft = g.box.scrollLeft;
  g.scrollTop = g.box.scrollTop;
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
  if (newest && run.mermaid) box.append(graphFor(key, run.mermaid));
  if (run.step && run.step.kind === "question") {
    const d = details(key + "|question", "Question text", "text");
    d.append(el("p", null, run.step.description || "(no text)"));
    if (run.step.options) d.append(defs([["options", run.step.options.join(" | ")]]));
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
  const title = a.cwd ? a.cwd.split("/").filter(Boolean).pop() : "";
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

// The counters: agents alive in the whole list, and runs by status in the agents shown.
function summary(list, shown) {
  const runs = shown.flatMap((a) => a.runs);
  const count = (status) => runs.filter((r) => r.status === status).length;
  const alive = list.filter((a) => a.alive).length;
  return [chip("alive", alive + " alive"), chip("waiting", count("waiting") + " waiting"), chip("done", count("done") + " done"), chip("error", count("error") + " error")];
}

// Keeps the focus on the same summary after a poll rebuilds the page.
function render(list) {
  const shown = list.filter((a) => endedBox.checked || a.alive);
  const focused = document.activeElement && document.activeElement.dataset.focusKey;
  const cards = shown.map(agentView);
  if (!cards.length) cards.push(el("p", "empty", "No agents use rein right now."));
  agentsBox.replaceChildren(...cards);
  summaryBox.replaceChildren(...summary(list, shown));
  for (const g of graphs.values()) if (g.box.isConnected) { g.box.scrollLeft = g.scrollLeft; g.box.scrollTop = g.scrollTop; }
  const ids = new Set(list.map((a) => a.id));
  for (const key of [...openState.keys(), ...graphs.keys()]) if (!ids.has(key.split("|")[0])) { openState.delete(key); graphs.delete(key); }
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
