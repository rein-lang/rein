//! The program graph behind `rein graph`: the decision flow of every function, or only the references.
//! Node ids and order come from the function names and the source order, so the output is stable.

use std::collections::{BTreeMap, HashMap};

use serde::Serialize;

use crate::ast::{Awaited, Binding, Call, Callee, Expr, Function, InputType, Piece, ReturnValue, Stmt, Value};
use crate::eval;
use crate::project::{Project, SourceFile};
use crate::step::{inputs_of, owners, Answers, StepResponse, CALL};

/// Version of the JSON schema. Bump it when a field changes meaning or goes away.
pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Serialize)]
pub struct Graph {
    pub version: u32,
    /// `flow` (the decision flow) or `refs` (functions and references only).
    pub view: &'static str,
    pub entry: String,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    /// With a walk (`rein graph --answer ...`): the ids of the nodes taken, in walk order.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub path: Vec<String>,
    /// With a walk: the node where it stopped.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current: Option<Current>,
}

/// Where a walk stopped: the next question, a request, or the outcome.
#[derive(Debug, Serialize)]
pub struct Current {
    pub node: String,
    /// `question`, `request` or `done`, as the step response `kind`.
    pub kind: &'static str,
    /// The answer key a question or request asks for.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum NodeKind {
    /// One function file. Its label is the name, with its `string` parameters.
    Function,
    /// A parameter or a request answer that a branch tests. The label is the question, then `name: type`.
    Question,
    /// A test that is not one parameter, such as `a && b`, labelled with the expression. Or a parameter that every
    /// caller in this project passes, so a run of this project never asks it, labelled like a question.
    Condition,
    /// `await request(...)`: the agent does the step, then the flow goes on.
    Request,
    /// The flow ends with this text for the agent.
    Done,
    /// The flow ends by handing a task to a subagent (`delegate(...)`).
    Delegate,
    /// A function of another program (`@program/function`). The graph does not draw that program.
    Program,
    /// A reference that does not resolve. `rein check` reports it.
    Unresolved,
}

#[derive(Debug, Serialize)]
pub struct Node {
    /// `f<i>` for the i-th function in name order, `f<i>_<j>` for the j-th step in it.
    pub id: String,
    pub kind: NodeKind,
    /// The function the node is part of.
    pub function: String,
    /// Display text. Lines are split by `\n`.
    pub label: String,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub entry: bool,
    /// For a question: the parameter or answer name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// For a question: the type as written, such as `boolean` or `"a" | "b"`.
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub ty: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum EdgeKind {
    /// From one step to the next inside a function.
    Flow,
    /// A return of another function: the agent goes on there.
    Handoff,
    /// A template call: the text comes from that template.
    Template,
    /// `await @function(...)`: the function runs, and its result is the answer. The flow goes on from the `await`.
    Call,
}

#[derive(Debug, Serialize)]
pub struct Edge {
    pub from: String,
    pub to: String,
    pub kind: EdgeKind,
    /// The answer that takes this edge: `yes`, `no`, or union members.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// The named arguments of a hand-off, as written.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<Arg>,
    /// With a walk: the walk took this edge.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub taken: bool,
}

#[derive(Debug, Serialize)]
pub struct Arg {
    pub name: String,
    pub value: String,
}

/// The decision flow of every function in the project, `entry` flagged.
pub fn flow(project: &Project, entry: &str) -> Graph {
    let mut b = Builder::new(project, entry, "flow");
    b.flow();
    b.graph
}

/// The decision flow with the walk of a step `response` marked: `path`, `taken` edges and `current`.
/// The step engine walks the flow; each function on its trace is walked again here, against the same
/// answers and with the same evaluator, to find the nodes. An error response gets the plain flow.
pub fn walk<'a>(project: &'a Project, entry: &str, response: &'a StepResponse) -> Graph {
    let (answers, trace, kind, key, asker) = match response {
        StepResponse::Question { answers, trace, key, .. } => (answers, trace, "question", Some(key.clone()), None),
        StepResponse::Request { answers, trace, key, function, .. } => {
            (answers, trace, "request", Some(key.clone()), Some(function))
        }
        StepResponse::Done { answers, trace, .. } => (answers, trace, "done", None, None),
        StepResponse::Error { .. } => return flow(project, entry),
    };
    let mut b = Builder::new(project, entry, "flow");
    // The graph draws functions, not calls. A function that the walk visits in several calls is drawn with the
    // answers of its first call, in the order `rein step` walks: `main.a>fn` before `main.b>fn`.
    // The walk is stepped again to learn the call of each hop. When it does not give the response's trace, as for
    // a logged run of a program that changed since, each function on the trace is read through its plain keys.
    let plain = |o: &str| o.rsplit_once(CALL).map_or(o, |(_, f)| f).to_string();
    let stepped = owners(project, entry, answers);
    let walked = if stepped.iter().map(|(o, _)| plain(o)).eq(trace.iter().cloned()) {
        stepped
    } else {
        trace.iter().map(|f| (f.clone(), BTreeMap::new())).collect()
    };
    let mut first: HashMap<String, Visit> = HashMap::new();
    for (owner, passed) in walked {
        first.entry(plain(&owner)).or_insert((owner, passed));
    }
    b.walk = Some((answers, first));
    b.flow();
    // A request is asked at the last node its function's walk reaches. After an `await` callee ends, that is the
    // caller's `await` node, not the callee's outcome.
    let asked = asker.and_then(|f| b.paths.get(f.as_str())).and_then(|p| p.last().cloned());
    let mut path: Vec<String> = Vec::new();
    for id in trace.iter().filter_map(|f| b.paths.remove(f)).flatten() {
        if !path.contains(&id) {
            path.push(id);
        }
    }
    b.graph.current = asked.or_else(|| path.last().cloned()).map(|node| Current { node, kind, key });
    b.graph.path = path;
    b.graph
}

/// One node per function and one edge per distinct reference, as `rein graph --refs` prints.
pub fn refs(project: &Project, entry: &str) -> Graph {
    let mut b = Builder::new(project, entry, "refs");
    for (name, sf) in project.functions() {
        b.function_node(name, sf);
    }
    for (i, (from, target)) in project.program_refs().into_iter().enumerate() {
        let label = format!("@{target}");
        let id = format!("p{i}");
        let kind = NodeKind::Program;
        b.graph.nodes.push(Node { id: id.clone(), kind, function: label.clone(), label, entry: false, name: None, ty: None });
        let from = b.id_of(&from);
        b.graph.edges.push(Edge { from, to: id, kind: EdgeKind::Handoff, label: None, args: Vec::new(), taken: false });
    }
    for (from, to) in project.references() {
        let template = project.get(&to).is_some_and(|sf| sf.function.is_template());
        let kind = if template { EdgeKind::Template } else { EdgeKind::Handoff };
        let (from, to) = (b.id_of(&from), b.id_of(&to));
        b.graph.edges.push(Edge { from, to, kind, label: None, args: Vec::new(), taken: false });
    }
    b.graph
}

/// Where the next node hangs: the node before it, and the answer that leads on from it.
type From = (String, Option<String>);

/// The first call of a function in a walk: its owner, as `rein step` keys its answers, and the values passed to it.
type Visit = (String, BTreeMap<String, String>);

struct Builder<'a> {
    project: &'a Project,
    graph: Graph,
    function: &'a str,
    fid: usize,
    step: usize,
    /// The answers of a walk, when there is one, and the first call of each function it walks: its owner and the
    /// values passed to it.
    walk: Option<(&'a Answers, HashMap<String, Visit>)>,
    /// The answers of the function being drawn, for the walk.
    vars: HashMap<String, String>,
    /// The nodes the walk took, per function, in walk order.
    paths: HashMap<String, Vec<String>>,
}

impl<'a> Builder<'a> {
    fn new(project: &'a Project, entry: &str, view: &'static str) -> Self {
        let graph = Graph {
            version: SCHEMA_VERSION,
            view,
            entry: entry.to_string(),
            nodes: Vec::new(),
            edges: Vec::new(),
            path: Vec::new(),
            current: None,
        };
        let (walk, vars, paths) = (None, HashMap::new(), HashMap::new());
        Builder { project, graph, function: "", fid: 0, step: 0, walk, vars, paths }
    }

    /// Every function's flow. With a walk, the functions on its trace are walked too.
    fn flow(&mut self) {
        let project = self.project;
        for (i, (name, sf)) in project.functions().enumerate() {
            let id = self.function_node(name, sf);
            self.function = name;
            self.fid = i;
            self.step = 0;
            let walked = self.walk.as_ref().and_then(|(a, first)| first.get(name.as_str()).map(|o| (*a, o.clone())));
            if let Some((answers, (owner, passed))) = &walked {
                let mut inputs = inputs_of(owner, answers);
                inputs.extend(passed.iter().map(|(k, v)| (k.clone(), v.clone())));
                self.vars = eval::with_struct_fields(&sf.function, &inputs);
                self.paths.entry(name.clone()).or_default().push(id.clone());
            }
            self.block(&sf.function, &sf.function.body, (id, None), walked.is_some());
        }
    }

    /// With `on`, the walk took the last edge: mark it and add its target to the path.
    fn mark(&mut self, on: bool) {
        let Some(e) = self.graph.edges.last_mut().filter(|_| on) else { return };
        e.taken = true;
        let to = e.to.clone();
        self.paths.entry(self.function.to_string()).or_default().push(to);
    }

    // ponytail: linear scan per lookup; index the names if a project ever has thousands of files.
    fn id_of(&self, name: &str) -> String {
        format!("f{}", self.project.functions().position(|(n, _)| n == name).unwrap_or_default())
    }

    /// The node a hand-off goes to: the function's node, or a new node for a function of another program.
    fn target_node(&mut self, target: &str) -> String {
        if target.starts_with('@') {
            return self.node(NodeKind::Program, target.to_string());
        }
        self.id_of(target)
    }

    fn function_node(&mut self, name: &str, sf: &SourceFile) -> String {
        let id = self.id_of(name);
        let strings: Vec<&str> = sf.function.text_inputs().map(|i| i.name.value.as_str()).collect();
        let label = if strings.is_empty() { name.to_string() } else { format!("{name}({})", strings.join(", ")) };
        let entry = name == self.graph.entry;
        let function = name.to_string();
        self.graph.nodes.push(Node { id: id.clone(), kind: NodeKind::Function, function, label, entry, name: None, ty: None });
        id
    }

    fn node(&mut self, kind: NodeKind, label: String) -> String {
        let id = format!("f{}_{}", self.fid, self.step);
        self.step += 1;
        let function = self.function.to_string();
        self.graph.nodes.push(Node { id: id.clone(), kind, function, label, entry: false, name: None, ty: None });
        id
    }

    fn question(&mut self, f: &Function, name: &str) -> String {
        let input = f.input(name).or_else(|| f.response(name));
        let ty = input.map_or_else(String::new, |i| i.ty.ts_type());
        let text = input.map_or("", |i| i.description.value.as_str());
        let label = if text.is_empty() { format!("{name}: {ty}") } else { format!("{text}\n{name}: {ty}") };
        // A parameter that every caller passes is a test on a value, not a question the run asks.
        let kind = if f.input(name).is_some() && self.passed(name) { NodeKind::Condition } else { NodeKind::Question };
        let id = self.node(kind, label);
        if let Some(n) = self.graph.nodes.last_mut() {
            n.name = Some(name.to_string());
            n.ty = Some(ty);
        }
        id
    }

    /// True when the function being drawn is not the entry, has a caller, and every hand-off or `await` to it
    /// in this project passes `param`. It sees only this project's call sites. A call from another program
    /// (`@program/function`) is not seen here: when that call omits `param`, its run still asks it.
    // ponytail: scans every function per question; index the call sites if graphs get slow.
    fn passed(&self, param: &str) -> bool {
        let (function, project) = (self.function, self.project);
        if function == self.graph.entry {
            return false;
        }
        let (mut sites, mut all) = (0, true);
        for (from, sf) in project.functions() {
            crate::ast::walk_stmts(&sf.function.body, &mut |s| {
                let target = match s {
                    Stmt::Return(r) => &r.value,
                    Stmt::Request { awaited: Awaited::Call(t), .. } => t,
                    _ => return,
                };
                let (key, args) = match target {
                    ReturnValue::Ref(r) => (r.value.as_str(), None),
                    ReturnValue::Call(c) => (c.callee.key(), Some(&c.args)),
                    ReturnValue::Text(_) | ReturnValue::Template(_) => return,
                };
                if matches!(project.binding_for(from, key), Ok(Binding::Function(path)) if path == function) {
                    sites += 1;
                    all &= args.is_some_and(|a| a.iter().any(|a| a.name.as_ref().is_some_and(|n| n.value == param)));
                }
            });
        }
        sites > 0 && all
    }

    fn edge(&mut self, from: From, to: &str, kind: EdgeKind, args: Vec<Arg>) {
        self.graph.edges.push(Edge { from: from.0, to: to.to_string(), kind, label: from.1, args, taken: false });
    }

    /// A new step node after `from`.
    fn step_to(&mut self, from: From, kind: NodeKind, label: String) -> String {
        let node = self.node(kind, label);
        self.edge(from, &node, EdgeKind::Flow, Vec::new());
        node
    }

    /// The statements of one block, from `from`. An `if` splits the flow: its block on one answer, the rest
    /// of this block on the other. `on`: the walk reaches this block. A missing answer stops it there.
    fn block(&mut self, f: &Function, stmts: &[Stmt], from: From, on: bool) {
        let Some((first, rest)) = stmts.split_first() else { return };
        match first {
            Stmt::If { cond, then, .. } => {
                let (node, taken, other) = self.decision(f, cond);
                self.edge(from, &node, EdgeKind::Flow, Vec::new());
                self.mark(on);
                let holds = if on { eval::holds(f, cond, &self.vars) } else { None };
                self.block(f, then, (node.clone(), Some(taken)), holds == Some(true));
                self.block(f, rest, (node, Some(other)), holds == Some(false));
            }
            Stmt::Switch { subject, cases, .. } => {
                let node = self.question(f, &subject.value);
                self.edge(from, &node, EdgeKind::Flow, Vec::new());
                self.mark(on);
                let value = if on { self.vars.get(&subject.value).cloned() } else { None };
                // ponytail: the checker makes every case return, so nothing after a switch is drawn.
                for case in cases {
                    let label = case.values.iter().map(|v| v.value.as_str()).collect::<Vec<_>>().join(" / ");
                    let hit = value.as_ref().is_some_and(|v| case.values.iter().any(|c| c.value == *v));
                    self.block(f, &case.body, (node.clone(), Some(label)), hit);
                }
            }
            Stmt::Request { response, awaited, .. } => {
                let mut label = match awaited {
                    Awaited::Request { arg, .. } => format!("request: {}", value_text(arg)),
                    Awaited::Call(target) => format!("await {}", return_text(target)),
                };
                // A struct answer, or the answer of a call, shows its name and type, like `pick: Pick`.
                if matches!(response.ty, InputType::Struct(_)) || matches!(awaited, Awaited::Call(_)) {
                    label.push_str(&format!("\n{}: {}", response.name.value, response.ty.ts_type()));
                }
                let node = self.step_to(from, NodeKind::Request, label);
                self.mark(on);
                match awaited {
                    Awaited::Request { arg: Value::Call(call), .. } => self.template_edges(&node, call),
                    Awaited::Call(target) => self.call_edge(&node, target),
                    Awaited::Request { .. } => {}
                }
                let answered = on && self.vars.contains_key(&response.name.value);
                self.block(f, rest, (node, None), answered);
            }
            Stmt::Return(r) => self.ret(&r.value, from, on),
        }
    }

    /// The node for a test, with the label of the edge taken when it holds and of the edge taken when it does not.
    fn decision(&mut self, f: &Function, cond: &Expr) -> (String, String, String) {
        match cond {
            Expr::Var(v) => (self.question(f, &v.value), "yes".into(), "no".into()),
            Expr::Not(inner, _) if matches!(**inner, Expr::Var(_)) => {
                let Expr::Var(v) = &**inner else { unreachable!() };
                (self.question(f, &v.value), "no".into(), "yes".into())
            }
            Expr::Compare { var, value, equal } => {
                let input = f.input(&var.value).or_else(|| f.response(&var.value));
                let options = input.and_then(|i| i.ty.options()).unwrap_or_default();
                let others = options.into_iter().filter(|o| *o != value.value).collect::<Vec<_>>().join(" / ");
                let node = self.question(f, &var.value);
                if *equal {
                    (node, value.value.clone(), others)
                } else {
                    (node, others, value.value.clone())
                }
            }
            _ => (self.node(NodeKind::Condition, expr_text(cond)), "yes".into(), "no".into()),
        }
    }

    fn ret(&mut self, value: &ReturnValue, from: From, on: bool) {
        match value {
            ReturnValue::Text(text) => {
                self.step_to(from, NodeKind::Done, text.value.clone());
            }
            ReturnValue::Template(lit) => {
                self.step_to(from, NodeKind::Done, template_text(&lit.pieces));
            }
            ReturnValue::Ref(r) => match self.project.binding_for(self.function, &r.value) {
                Ok(Binding::Function(target)) => {
                    let to = self.target_node(&target);
                    self.edge(from, &to, EdgeKind::Handoff, Vec::new());
                }
                _ => {
                    self.step_to(from, NodeKind::Unresolved, format!("@{}", r.value));
                }
            },
            ReturnValue::Call(call) => match self.project.binding_for(self.function, call.callee.key()) {
                Ok(Binding::Function(target)) => {
                    let to = self.target_node(&target);
                    let args = call.args.iter().map(|a| Arg { name: arg_name(a), value: value_text(&a.value) }).collect();
                    self.edge(from, &to, EdgeKind::Handoff, args);
                }
                Ok(Binding::Template { .. }) => {
                    let delegate = matches!(&call.callee, Callee::Std(s) if s.value == "delegate");
                    let kind = if delegate { NodeKind::Delegate } else { NodeKind::Done };
                    let node = self.step_to(from, kind, call_text(call));
                    self.mark(on);
                    self.template_edges(&node, call);
                    return;
                }
                _ => {
                    self.step_to(from, NodeKind::Unresolved, call_text(call));
                }
            },
        }
        self.mark(on);
    }

    /// A `call` edge from the node of an `await` to the function it runs, with the named arguments. A target
    /// that does not resolve gets no edge; `rein check` reports it.
    fn call_edge(&mut self, node: &str, target: &ReturnValue) {
        let (key, args) = match target {
            ReturnValue::Ref(r) => (r.value.as_str(), Vec::new()),
            ReturnValue::Call(call) => {
                let args = call.args.iter().map(|a| Arg { name: arg_name(a), value: value_text(&a.value) }).collect();
                (call.callee.key(), args)
            }
            ReturnValue::Text(_) | ReturnValue::Template(_) => return,
        };
        if let Ok(Binding::Function(path)) = self.project.binding_for(self.function, key) {
            let to = self.target_node(&path);
            self.edge((node.to_string(), None), &to, EdgeKind::Call, args);
        }
    }

    /// A dashed edge from `node` to each project template that `call` uses, at any depth.
    fn template_edges(&mut self, node: &str, call: &Call) {
        for c in call.calls() {
            let Callee::Ref(_) = c.callee else { continue };
            if let Ok(Binding::Template { path: Some(path), .. }) = self.project.binding_for(self.function, c.callee.key()) {
                if path.starts_with('@') {
                    continue;
                }
                let to = self.id_of(&path);
                self.edge((node.to_string(), None), &to, EdgeKind::Template, Vec::new());
            }
        }
    }
}

fn arg_name(a: &crate::ast::Arg) -> String {
    a.name.as_ref().map_or_else(String::new, |n| n.value.clone())
}

fn template_text(pieces: &[Piece]) -> String {
    pieces
        .iter()
        .map(|p| match p {
            Piece::Text(t) => t.clone(),
            Piece::Name(n) => format!("${{{}}}", n.value),
        })
        .collect()
}

/// A value as written: `"text"`, `` `text ${name}` ``, a bare name, or a call.
fn value_text(v: &Value) -> String {
    match v {
        Value::Str(s) => format!("\"{}\"", s.value),
        Value::Template(lit) => format!("`{}`", template_text(&lit.pieces)),
        Value::Name(n) => n.value.clone(),
        Value::Call(c) => call_text(c),
    }
}

/// The target of an `await` as written: `@./x`, or `@program` without its arguments (the edge carries them).
fn return_text(v: &ReturnValue) -> String {
    match v {
        ReturnValue::Ref(r) => format!("@{}", r.value),
        ReturnValue::Call(c) => format!("@{}", c.callee.key()),
        ReturnValue::Text(t) => t.value.clone(),
        ReturnValue::Template(lit) => template_text(&lit.pieces),
    }
}

fn call_text(c: &Call) -> String {
    let callee = match &c.callee {
        Callee::Std(s) => s.value.clone(),
        Callee::Ref(r) => format!("@{}", r.value),
    };
    let args: Vec<String> = c.args.iter().map(|a| format!("{}={}", arg_name(a), value_text(&a.value))).collect();
    format!("{callee}({})", args.join(", "))
}

fn expr_text(e: &Expr) -> String {
    let wrap = |e: &Expr| match e {
        Expr::And(..) | Expr::Or(..) => format!("({})", expr_text(e)),
        _ => expr_text(e),
    };
    match e {
        Expr::Var(v) => v.value.clone(),
        Expr::Compare { var, value, equal } => format!("{} {} \"{}\"", var.value, if *equal { "===" } else { "!==" }, value.value),
        Expr::Not(inner, _) => format!("!{}", wrap(inner)),
        Expr::And(a, b) => format!("{} && {}", wrap(a), wrap(b)),
        Expr::Or(a, b) => format!("{} || {}", wrap(a), wrap(b)),
    }
}

/// The edge text: the answer, then each argument on its own line.
fn edge_lines(e: &Edge) -> Vec<String> {
    e.label.iter().cloned().chain(e.args.iter().map(|a| format!("{}={}", a.name, a.value))).collect()
}

/// The walk class of a node: `current`, `taken` or `dimmed`. `None` without a walk.
fn node_mark(g: &Graph, id: &str) -> Option<&'static str> {
    if g.path.is_empty() {
        None
    } else if g.current.as_ref().is_some_and(|c| c.node == id) {
        Some("current")
    } else if g.path.iter().any(|p| p == id) {
        Some("taken")
    } else {
        Some("dimmed")
    }
}

/// Nodes in runs of one function each, in graph order.
fn groups(g: &Graph) -> Vec<&[Node]> {
    g.nodes.chunk_by(|a, b| a.function == b.function).collect()
}

/// The graph as a Mermaid flowchart. A function with steps is a subgraph.
pub fn mermaid(g: &Graph) -> String {
    let esc = |s: &str| {
        s.replace('#', "#35;")
            .replace('"', "#quot;")
            .replace('<', "#lt;")
            .replace('>', "#gt;")
            // ponytail: Mermaid reads a backtick as a markdown code span, which fails inside a label with a line break.
            // The grave accent looks like it and renders as text. Upgrade if Mermaid fixes its markdown.
            .replace('`', "\u{2cb}")
            .replace('\n', "<br/>")
    };
    let shape = |n: &Node| {
        let (id, l) = (&n.id, esc(&n.label));
        match n.kind {
            NodeKind::Function if n.entry => format!("{id}([\"{l}\"])"),
            NodeKind::Function => format!("{id}[[\"{l}\"]]"),
            NodeKind::Question => format!("{id}{{\"{l}\"}}"),
            NodeKind::Condition => format!("{id}{{{{\"{l}\"}}}}"),
            NodeKind::Request => format!("{id}[/\"{l}\"/]"),
            NodeKind::Done => format!("{id}(\"{l}\")"),
            NodeKind::Delegate => format!("{id}>\"{l}\"]"),
            NodeKind::Program => format!("{id}[[\"{l}\"]]:::program"),
            NodeKind::Unresolved => format!("{id}[\"{l}\"]:::unresolved"),
        }
    };
    let mut out = vec!["flowchart TD".to_string()];
    for group in groups(g) {
        if let [one] = group {
            out.push(format!("    {}", shape(one)));
            continue;
        }
        out.push(format!("    subgraph s{} [\"{}.rein\"]", &group[0].id[1..], esc(&group[0].function)));
        out.extend(group.iter().map(|n| format!("        {}", shape(n))));
        out.push("    end".to_string());
    }
    for e in &g.edges {
        let arrow = match e.kind {
            EdgeKind::Template => "-.->",
            EdgeKind::Call => "==>",
            EdgeKind::Flow | EdgeKind::Handoff => "-->",
        };
        let lines = edge_lines(e);
        if lines.is_empty() {
            out.push(format!("    {} {arrow} {}", e.from, e.to));
        } else {
            out.push(format!("    {} {arrow}|\"{}\"| {}", e.from, esc(&lines.join("\n")), e.to));
        }
    }
    if g.nodes.iter().any(|n| n.kind == NodeKind::Program) {
        out.push("    classDef program stroke-dasharray:4 2".to_string());
    }
    if g.nodes.iter().any(|n| n.kind == NodeKind::Unresolved) {
        out.push("    classDef unresolved stroke:#c00,color:#c00".to_string());
    }
    if !g.path.is_empty() {
        out.push("    classDef taken fill:#dcfce7,stroke:#16a34a,stroke-width:2px".to_string());
        out.push("    classDef current fill:#fde68a,stroke:#d97706,stroke-width:4px".to_string());
        out.push("    classDef dimmed opacity:0.4".to_string());
        for mark in ["taken", "current", "dimmed"] {
            let ids: Vec<&str> = g.nodes.iter().map(|n| n.id.as_str()).filter(|id| node_mark(g, id) == Some(mark)).collect();
            if !ids.is_empty() {
                out.push(format!("    class {} {mark}", ids.join(",")));
            }
        }
        let (taken, dimmed): (Vec<usize>, Vec<usize>) = (0..g.edges.len()).partition(|&i| g.edges[i].taken);
        for (ids, style) in [(taken, "stroke:#16a34a,stroke-width:3px"), (dimmed, "opacity:0.3")] {
            if !ids.is_empty() {
                let ids: Vec<String> = ids.iter().map(usize::to_string).collect();
                out.push(format!("    linkStyle {} {style}", ids.join(",")));
            }
        }
    }
    out.join("\n") + "\n"
}

/// The graph as Graphviz DOT. A function with steps is a cluster.
pub fn dot(g: &Graph) -> String {
    let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n");
    let attrs = |n: &Node| {
        let shape = match n.kind {
            NodeKind::Function if n.entry => "shape=box, style=\"rounded,bold\"",
            NodeKind::Function => "shape=box, style=bold",
            NodeKind::Question => "shape=diamond",
            NodeKind::Condition => "shape=hexagon",
            NodeKind::Request => "shape=parallelogram",
            NodeKind::Done => "shape=box, style=rounded",
            NodeKind::Delegate => "shape=cds",
            NodeKind::Program => "shape=box, style=dashed",
            NodeKind::Unresolved => "shape=octagon, color=red",
        };
        let walk = match node_mark(g, &n.id) {
            Some("current") => ", color=\"#d97706\", penwidth=4",
            Some("taken") => ", color=\"#16a34a\", penwidth=2",
            Some(_) => ", color=gray70, fontcolor=gray60",
            None => "",
        };
        format!("{} [label=\"{}\", {shape}{walk}];", n.id, esc(&n.label))
    };
    let mut out = vec!["digraph rein {".to_string(), "    node [fontname=\"Helvetica\"];".to_string()];
    for group in groups(g) {
        if let [one] = group {
            out.push(format!("    {}", attrs(one)));
            continue;
        }
        out.push(format!("    subgraph cluster_{} {{", &group[0].id[1..]));
        out.push(format!("        label=\"{}.rein\";", esc(&group[0].function)));
        out.extend(group.iter().map(|n| format!("        {}", attrs(n))));
        out.push("    }".to_string());
    }
    for e in &g.edges {
        let mut a = Vec::new();
        let lines = edge_lines(e);
        if !lines.is_empty() {
            a.push(format!("label=\"{}\"", esc(&lines.join("\n"))));
        }
        if e.kind == EdgeKind::Template {
            a.push("style=dashed".to_string());
        }
        if e.kind == EdgeKind::Call {
            a.push("style=bold".to_string());
        }
        if !g.path.is_empty() {
            let walk = if e.taken { "color=\"#16a34a\", penwidth=2" } else { "color=gray80, fontcolor=gray70" };
            a.push(walk.to_string());
        }
        let a = if a.is_empty() { String::new() } else { format!(" [{}]", a.join(", ")) };
        out.push(format!("    {} -> {}{a};", e.from, e.to));
    }
    out.push("}".to_string());
    out.join("\n") + "\n"
}

#[cfg(test)]
#[path = "graph_tests.rs"]
mod tests;
