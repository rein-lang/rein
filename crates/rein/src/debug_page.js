// The rein debug page: a React app. The server walks the flow (`POST /walk`); this page keeps the state, draws the
// controls and the step, and draws the graph with React Flow, laid out top-down by dagre.
import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import {
  ReactFlow, ReactFlowProvider, Background, Controls, MiniMap, Panel, Handle, Position,
  useNodesState, useEdgesState, useReactFlow,
} from "@xyflow/react";
import dagre from "@dagrejs/dagre";
import htm from "htm";

const html = htm.bind(React.createElement);

// The query string holds the whole state: `entry`, `in.<name>` per input, `a.<key>` per answer. A link that
// carries any of them wins over the starting values; otherwise the starting values are the state.
function readUrl(start) {
  const q = new URLSearchParams(location.search);
  if (![...q.keys()].some((k) => k === "entry" || k.startsWith("in.") || k.startsWith("a."))) {
    return { entry: start.entry, inputs: { ...start.inputs }, answers: { ...start.answers }, order: Object.keys(start.answers) };
  }
  const s = { entry: q.get("entry") || start.entry, inputs: {}, answers: {}, order: [] };
  for (const [k, v] of q) {
    if (v && k.startsWith("in.")) s.inputs[k.slice(3)] = v;
    else if (v && k.startsWith("a.")) { s.answers[k.slice(2)] = v; s.order.push(k.slice(2)); }
  }
  return s;
}

// Every change writes the whole state to the URL with replaceState, so the address is always a shareable link.
// It replaces rather than pushes: each keystroke would otherwise fill the Back history. Back still undoes answers.
function writeUrl(s) {
  const q = new URLSearchParams();
  if (s.entry) q.set("entry", s.entry);
  for (const [k, v] of Object.entries(s.inputs)) if (v) q.set("in." + k, v);
  for (const k of s.order) if (s.answers[k]) q.set("a." + k, s.answers[k]);
  history.replaceState(null, "", "?" + q);
}

// Auto follows the OS. Light and Dark set data-theme, and the choice is kept in localStorage if the browser allows.
function savedTheme() {
  try { const t = localStorage.getItem("rein-theme"); return ["light", "dark"].includes(t) ? t : "auto"; } catch (e) { return "auto"; }
}

// ---- Graph layout ----

const NODE_WIDTH = 240;
const CHARS_PER_LINE = 34;

// shortcut: the height comes from a character count, not a measure. Upgrade to measured sizes if labels overlap.
function nodeHeight(label) {
  const lines = label.split("\n").reduce((n, line) => n + Math.max(1, Math.ceil(line.length / CHARS_PER_LINE)), 0);
  return 30 + lines * 16;
}

function edgeText(e) {
  return [...(e.label ? [e.label] : []), ...(e.args || []).map((a) => `${a.name}=${a.value}`)].join(", ");
}

// The walk mark of a node: `current`, `taken`, `dimmed`, or "" without a walk. As `node_mark` in rein-core.
function mark(g, id) {
  if (!g.path || !g.path.length) return "";
  if (g.current && g.current.node === id) return "current";
  return g.path.includes(id) ? "taken" : "dimmed";
}

// React Flow nodes and edges for a rein graph. A function with steps is a group box around them. `keys` maps a
// question node to its answer key. `entries` lists the functions a click can make the entry. Returns the center of each node too, to scroll to the current one.
function layout(g, keys, entries) {
  const d = new dagre.graphlib.Graph({ compound: true, multigraph: true });
  d.setGraph({ rankdir: "TB", nodesep: 40, ranksep: 56, marginx: 24, marginy: 24 });
  d.setDefaultEdgeLabel(() => ({}));
  const groups = new Map();
  for (const n of g.nodes) {
    const list = groups.get(n.function) || [];
    list.push(n);
    groups.set(n.function, list);
  }
  const parent = {};
  for (const [fn, list] of groups) {
    if (list.length < 2) continue;
    d.setNode("group:" + fn, {});
    for (const n of list) parent[n.id] = "group:" + fn;
  }
  for (const n of g.nodes) {
    d.setNode(n.id, { width: NODE_WIDTH, height: nodeHeight(n.label) });
    if (parent[n.id]) d.setParent(n.id, parent[n.id]);
  }
  g.edges.forEach((e, i) => {
    const text = edgeText(e);
    d.setEdge(e.from, e.to, text ? { width: Math.min(220, text.length * 6.5 + 12), height: 18, labelpos: "c" } : {}, "e" + i);
  });
  dagre.layout(d);

  const centers = {};
  const box = {};
  for (const n of g.nodes) {
    const p = d.node(n.id);
    centers[n.id] = { x: p.x, y: p.y };
    box[n.id] = { x: p.x - p.width / 2, y: p.y - p.height / 2, w: p.width, h: p.height };
  }
  // The group boxes are drawn around their nodes, with room for the title, rather than from dagre's cluster size.
  const nodes = [];
  const groupAt = {};
  for (const [fn, list] of groups) {
    if (list.length < 2) continue;
    const gid = "group:" + fn;
    const x0 = Math.min(...list.map((n) => box[n.id].x)) - 16, y0 = Math.min(...list.map((n) => box[n.id].y)) - 32;
    const x1 = Math.max(...list.map((n) => box[n.id].x + box[n.id].w)) + 16, y1 = Math.max(...list.map((n) => box[n.id].y + box[n.id].h)) + 16;
    groupAt[gid] = { x: x0, y: y0 };
    nodes.push({
      id: gid, type: "fn", position: { x: x0, y: y0 }, data: { title: fn + ".rein" },
      style: { width: x1 - x0, height: y1 - y0 }, selectable: false, zIndex: -1,
    });
  }
  for (const n of g.nodes) {
    const b = box[n.id], gid = parent[n.id];
    const at = gid ? { x: b.x - groupAt[gid].x, y: b.y - groupAt[gid].y } : { x: b.x, y: b.y };
    const key = keys[n.id];
    const pick = n.kind === "function" && !n.entry && entries.includes(n.function);
    const title = [`${n.kind} in ${n.function}.rein`, n.name ? `name: ${n.name}` : "", n.type ? `type: ${n.type}` : "", key ? "Click to set this answer." : "", pick ? "Click to start from this function." : ""].filter(Boolean).join("\n");
    nodes.push({
      id: n.id, type: "step", position: at, ...(gid ? { parentId: gid } : {}),
      data: { kind: n.kind, fn: n.function, label: n.label, entry: !!n.entry, mark: mark(g, n.id), key, pick, title },
      style: { width: b.w, height: b.h },
    });
  }
  const walked = !!(g.path && g.path.length);
  const edges = g.edges.map((e, i) => ({
    id: "e" + i, source: e.from, target: e.to, label: edgeText(e) || undefined,
    className: [e.kind, e.taken ? "taken" : walked ? "dimmed" : ""].join(" "),
    animated: !!e.taken, markerEnd: { type: "arrowclosed" },
    labelBgPadding: [4, 2], labelBgBorderRadius: 3,
  }));
  return { nodes, edges, centers };
}

// ---- Graph components ----

function StepNode({ data }) {
  const cls = ["flow-node", data.kind, data.entry ? "entry" : "", data.mark, data.key || data.pick ? "clickable" : ""].join(" ");
  return html`<div className=${cls} title=${data.title} data-kind=${data.kind}>
    <${Handle} type="target" position=${Position.Top} isConnectable=${false} />
    <div className="kind">${data.entry ? "entry" : data.kind}${data.mark === "current" ? " · current" : ""}</div>
    <div className="label">${data.label}</div>
    <${Handle} type="source" position=${Position.Bottom} isConnectable=${false} />
  </div>`;
}

function GroupNode({ data }) {
  return html`<div className="flow-group"><div className="title">${data.title}</div></div>`;
}

const nodeTypes = { step: StepNode, fn: GroupNode };

function Graph({ graph, keys, entries, theme, onPick, onEntry }) {
  const [nodes, setNodes, onNodesChange] = useNodesState([]);
  const [edges, setEdges, onEdgesChange] = useEdgesState([]);
  const flow = useReactFlow();
  const centers = useRef({});
  const current = graph && graph.current ? graph.current.node : null;

  const center = useCallback(() => {
    const c = current && centers.current[current];
    if (c) flow.setCenter(c.x, c.y, { zoom: 1, duration: 300 });
    else flow.fitView({ duration: 300 });
  }, [current, flow]);

  useEffect(() => {
    if (!graph) return;
    const out = layout(graph, keys, entries);
    centers.current = out.centers;
    setNodes(out.nodes);
    setEdges(out.edges);
    document.body.dataset.renders = String(Number(document.body.dataset.renders || 0) + 1);
  }, [graph, keys, entries]);

  // After the nodes are in, scroll to the node the walk stopped on.
  useEffect(() => {
    if (!nodes.length) return;
    const t = setTimeout(center, 50);
    return () => clearTimeout(t);
  }, [graph, nodes.length]);

  const colorMode = theme === "auto" ? "system" : theme;
  return html`<${ReactFlow}
      nodes=${nodes} edges=${edges} nodeTypes=${nodeTypes}
      onNodesChange=${onNodesChange} onEdgesChange=${onEdgesChange}
      onNodeClick=${(_, n) => (n.data.key ? onPick(n.data.key) : n.data.pick && onEntry(n.data.fn))}
      nodesConnectable=${false} colorMode=${colorMode} minZoom=${0.1} maxZoom=${4}>
    <${Background} gap=${20} />
    <${Controls} showInteractive=${false} />
    <${MiniMap} pannable zoomable nodeColor=${(n) => n.data.mark === "current" ? "#d97706" : n.data.mark === "taken" ? "#16a34a" : "#9ca3af"} />
    <${Panel} position="top-right" className="toolbar">
      <button type="button" id="zoom-current" onClick=${center}>${current ? "Go to current" : "Fit"}</button>
      <button type="button" id="zoom-fit" onClick=${() => flow.fitView({ duration: 300 })}>Fit all</button>
    <//>
  <//>`;
}

// ---- Controls and step ----

// One control per key: yes/no buttons, a list of the options, or a text box. A text box sets on Enter or blur.
function Control({ item, value, onChange, focused, needed }) {
  const id = "c-" + item.key;
  let field;
  if (item.type === "boolean") {
    field = html`<span>${["yes", "no"].map((v) => html`<button key=${v} type="button" aria-pressed=${String(value === v)}
      onClick=${() => onChange(value === v ? "" : v)}>${v}</button>`)}</span>`;
  } else if (item.type === "union") {
    field = html`<select id=${id} value=${value ?? ""} onChange=${(e) => onChange(e.target.value)}>
      <option value="">(unset)</option>
      ${item.options.map((o) => html`<option key=${o} value=${o}>${o}</option>`)}
    </select>`;
  } else {
    const placeholder = item.fields ? "{" + item.fields.map((f) => `"${f}": ...`).join(", ") + "}" : "(unset)";
    const commit = (e) => { const v = e.target.value.trim(); if (v !== (value ?? "")) onChange(v); };
    field = html`<input id=${id} key=${value ?? ""} type="text" placeholder=${placeholder} defaultValue=${value ?? ""}
      onBlur=${commit} onKeyDown=${(e) => e.key === "Enter" && commit(e)} />`;
  }
  const kind = item.type === "string" || item.type === "struct" ? ` (${item.type})` : "";
  const cls = ["control", value ? "set" : "", focused ? "focus" : "", needed ? "needed" : ""].join(" ");
  return html`<div className=${cls} data-key=${item.key}>
    <label htmlFor=${id}>${item.key}</label><small>${item.description + kind}</small>${field}
  </div>`;
}

// Buttons or a text box that answer the current question or request.
function AnswerBox({ item, onAnswer }) {
  const [text, setText] = useState("");
  if (item.type === "boolean") return html`<div>${["yes", "no"].map((v) => html`<button key=${v} type="button" onClick=${() => onAnswer(item.key, v)}>${v}</button>`)}</div>`;
  if (item.type === "union") return html`<div>${(item.options || []).map((o) => html`<button key=${o} type="button" onClick=${() => onAnswer(item.key, o)}>${o}</button>`)}</div>`;
  return html`<form onSubmit=${(e) => { e.preventDefault(); onAnswer(item.key, text.trim()); setText(""); }}>
    <input type="text" aria-label=${item.key} placeholder=${item.key} value=${text} onChange=${(e) => setText(e.target.value)} />
    <button>Answer</button>
  </form>`;
}

// The current step: the question, the request, the final instruction, or the error.
function Step({ step, schema, placeholders, onAnswer }) {
  if (!step) return html`<div id="step" aria-live="polite"></div>`;
  const known = schema.answers.find((a) => a.key === step.key);
  let body;
  if (step.kind === "question") {
    body = html`<pre>${step.description}</pre><${AnswerBox} key=${step.key} item=${known || { key: step.key, type: step.type, options: step.options }} onAnswer=${onAnswer} />`;
  } else if (step.kind === "request") {
    const r = step.response;
    body = html`<pre>${step.instruction}</pre><div>Then answer: ${r.description}</div>
      <${AnswerBox} key=${step.key} item=${known || { key: step.key, type: r.type, options: r.options }} onAnswer=${onAnswer} />`;
  } else if (step.kind === "done") {
    body = html`<pre>${step.instruction}</pre>`;
  } else {
    body = html`<pre>${step.message}</pre>${(step.diagnostics || []).map((d, i) => html`<div key=${i}>${d.file}:${d.line}:${d.column}: ${d.message}</div>`)}`;
  }
  return html`<div id="step" className=${step.kind} aria-live="polite">
    <div className="kind">${step.kind + (step.key ? " " + step.key : "")}</div>
    ${body}
    ${step.trace && step.trace.length ? html`<div className="trace">trace: ${step.trace.join(" -> ")}</div>` : null}
    ${placeholders.length ? html`<div className="trace">input needed: ${placeholders.join(", ")} (shown as ${placeholders.map((p) => "<" + p + ">").join(", ")})</div>` : null}
  </div>`;
}

// ---- App ----

function App({ schema }) {
  const [state, setState] = useState(() => readUrl(initial));
  const [out, setOut] = useState(null);
  const [theme, setTheme] = useState(savedTheme);
  const [focus, setFocus] = useState(null);
  const [copied, setCopied] = useState(false);
  const ticket = useRef(0);

  // Each change of the state writes the URL and walks the flow again. A late answer to an older walk is dropped.
  useEffect(() => {
    writeUrl(state);
    const mine = ++ticket.current;
    const body = JSON.stringify({ entry: state.entry, inputs: state.inputs, answers: state.answers });
    fetch("/walk", { method: "POST", headers: { "Content-Type": "application/json" }, body })
      .then((r) => r.json())
      .then((o) => { if (mine === ticket.current) setOut(o); })
      .catch((e) => { if (mine === ticket.current) setOut({ step: { kind: "error", message: "the rein debug server did not answer: " + e }, placeholders: [] }); });
  }, [state]);

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    try { localStorage.setItem("rein-theme", theme); } catch (e) {}
  }, [theme]);

  // A click on a question node focuses its control.
  useEffect(() => {
    if (!focus) return;
    const c = document.querySelector(`.control[data-key="${CSS.escape(focus)}"]`);
    if (!c) return;
    c.scrollIntoView({ block: "center" });
    c.querySelector("select, input, button")?.focus();
  }, [focus]);

  // An empty value unsets the key.
  const setAnswer = useCallback((key, value) => setState((s) => {
    const answers = { ...s.answers };
    const order = s.order.filter((k) => k !== key);
    if (!value) delete answers[key];
    else { answers[key] = value; order.push(key); }
    return { ...s, answers, order };
  }), []);
  const setInput = (name, value) => setState((s) => {
    const inputs = { ...s.inputs };
    if (!value) delete inputs[name]; else inputs[name] = value;
    return { ...s, inputs };
  });
  const back = () => { const key = state.order.at(-1); if (key) setAnswer(key, ""); };
  const reset = () => setState((s) => ({ ...s, inputs: { ...initial.inputs }, answers: {}, order: [] }));
  const pickEntry = (entry) => setState((s) => ({ ...s, entry, inputs: {} }));

  // Copy the address of this page, which holds the state. The fallback selects the link in a field for a manual copy.
  const copy = async () => {
    const link = location.href;
    try {
      await navigator.clipboard.writeText(link);
    } catch {
      const box = document.createElement("textarea");
      box.value = link;
      document.body.append(box);
      box.select();
      if (!document.execCommand("copy")) window.prompt("Copy this link:", link);
      box.remove();
    }
    setCopied(true);
    setTimeout(() => setCopied(false), 1500);
  };

  const placeholders = (out && out.placeholders) || [];
  const inputs = schema.inputs[state.entry] || [];
  const keys = useMemo(() => (out && out.keys) || {}, [out]);
  return html`
    <header>
      <h1>rein debug</h1>
      <label>Entry <select id="entry" value=${state.entry} onChange=${(e) => pickEntry(e.target.value)}>
        ${schema.functions.map((f) => html`<option key=${f} value=${f}>${f}</option>`)}
      </select></label>
      <label>Theme <select id="theme" value=${theme} onChange=${(e) => setTheme(e.target.value)}>
        <option value="auto">Auto</option><option value="light">Light</option><option value="dark">Dark</option>
      </select></label>
      <button id="back" type="button" onClick=${back} disabled=${!state.order.length}>Back</button>
      <button id="reset" type="button" onClick=${reset}>Reset</button>
      <button id="copy" type="button" onClick=${copy}>${copied ? "Copied" : "Copy link"}</button>
    </header>
    <main>
      <aside id="controls" aria-label="Inputs and answers">
        ${inputs.length ? html`<h2>Inputs</h2>` : null}
        ${inputs.map((i) => {
          const unset = placeholders.includes(i.name);
          const note = unset ? " (unset: the process shows <" + i.name + ">)" : "";
          return html`<${Control} key=${"in." + i.name} item=${{ key: i.name, type: "string", description: i.description + note }}
            value=${state.inputs[i.name]} onChange=${(v) => setInput(i.name, v)} needed=${unset} focused=${focus === i.name} />`;
        })}
        <h2>Answers</h2>
        ${schema.answers.map((a) => html`<${Control} key=${a.key} item=${a} value=${state.answers[a.key]}
          onChange=${(v) => setAnswer(a.key, v)} focused=${focus === a.key} />`)}
      </aside>
      <section>
        <${Step} step=${out && out.step} schema=${schema} placeholders=${placeholders} onAnswer=${setAnswer} />
        <p className="legend"><span><i className="taken"></i>taken</span><span><i className="current"></i>current</span><span><i className="dimmed"></i>not taken</span><span className="trace">Drag to pan, scroll to zoom, click a question to set it, or a function to start from it.</span></p>
        <div id="diagram">
          <${ReactFlowProvider}>
            <${Graph} graph=${out && out.graph} keys=${keys} entries=${schema.functions} theme=${theme} onPick=${setFocus} onEntry=${pickEntry} />
          <//>
        </div>
      </section>
    </main>`;
}

const schema = await (await fetch("/schema")).json();
createRoot(document.getElementById("root")).render(html`<${App} schema=${schema} />`);
