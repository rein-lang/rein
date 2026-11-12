//! The program graph behind `rein graph`: the decision flow of every function, or only the references.
//! Node ids and order come from the function names and the source order, so the output is stable.

use serde::Serialize;

use crate::ast::{Binding, Call, Callee, Expr, Function, InputType, Piece, ReturnValue, Stmt, Value};
use crate::project::{Project, SourceFile};

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
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum NodeKind {
    /// One function file. Its label is the name, with its `string` parameters.
    Function,
    /// A parameter or a request answer that a branch tests. The label is the question, then `name: type`.
    Question,
    /// A test that is not one parameter, such as `a && b`. The label is the expression.
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
}

#[derive(Debug, Serialize)]
pub struct Arg {
    pub name: String,
    pub value: String,
}

/// The decision flow of every function in the project, `entry` flagged.
pub fn flow(project: &Project, entry: &str) -> Graph {
    let mut b = Builder::new(project, entry, "flow");
    for (i, (name, sf)) in project.functions().enumerate() {
        let id = b.function_node(name, sf);
        b.function = name;
        b.fid = i;
        b.step = 0;
        b.block(&sf.function, &sf.function.body, (id, None));
    }
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
        b.graph.edges.push(Edge { from, to: id, kind: EdgeKind::Handoff, label: None, args: Vec::new() });
    }
    for (from, to) in project.references() {
        let template = project.get(&to).is_some_and(|sf| sf.function.is_template());
        let kind = if template { EdgeKind::Template } else { EdgeKind::Handoff };
        let (from, to) = (b.id_of(&from), b.id_of(&to));
        b.graph.edges.push(Edge { from, to, kind, label: None, args: Vec::new() });
    }
    b.graph
}

/// Where the next node hangs: the node before it, and the answer that leads on from it.
type From = (String, Option<String>);

struct Builder<'a> {
    project: &'a Project,
    graph: Graph,
    function: &'a str,
    fid: usize,
    step: usize,
}

impl<'a> Builder<'a> {
    fn new(project: &'a Project, entry: &str, view: &'static str) -> Self {
        let graph = Graph { version: SCHEMA_VERSION, view, entry: entry.to_string(), nodes: Vec::new(), edges: Vec::new() };
        Builder { project, graph, function: "", fid: 0, step: 0 }
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
        let id = self.node(NodeKind::Question, label);
        if let Some(n) = self.graph.nodes.last_mut() {
            n.name = Some(name.to_string());
            n.ty = Some(ty);
        }
        id
    }

    fn edge(&mut self, from: From, to: &str, kind: EdgeKind, args: Vec<Arg>) {
        self.graph.edges.push(Edge { from: from.0, to: to.to_string(), kind, label: from.1, args });
    }

    /// A new step node after `from`.
    fn step_to(&mut self, from: From, kind: NodeKind, label: String) -> String {
        let node = self.node(kind, label);
        self.edge(from, &node, EdgeKind::Flow, Vec::new());
        node
    }

    /// The statements of one block, from `from`. An `if` splits the flow: its block on one answer, the rest
    /// of this block on the other.
    fn block(&mut self, f: &Function, stmts: &[Stmt], from: From) {
        let Some((first, rest)) = stmts.split_first() else { return };
        match first {
            Stmt::If { cond, then, .. } => {
                let (node, taken, other) = self.decision(f, cond);
                self.edge(from, &node, EdgeKind::Flow, Vec::new());
                self.block(f, then, (node.clone(), Some(taken)));
                self.block(f, rest, (node, Some(other)));
            }
            Stmt::Switch { subject, cases, .. } => {
                let node = self.question(f, &subject.value);
                self.edge(from, &node, EdgeKind::Flow, Vec::new());
                // ponytail: the checker makes every case return, so nothing after a switch is drawn.
                for case in cases {
                    let label = case.values.iter().map(|v| v.value.as_str()).collect::<Vec<_>>().join(" / ");
                    self.block(f, &case.body, (node.clone(), Some(label)));
                }
            }
            Stmt::Request { response, arg, .. } => {
                let mut label = format!("request: {}", value_text(arg));
                // A struct answer shows its name and struct, like `pick: Pick`.
                if matches!(response.ty, InputType::Struct(_)) {
                    label.push_str(&format!("\n{}: {}", response.name.value, response.ty.ts_type()));
                }
                let node = self.step_to(from, NodeKind::Request, label);
                if let Value::Call(call) = arg {
                    self.template_edges(&node, call);
                }
                self.block(f, rest, (node, None));
            }
            Stmt::Return(r) => self.ret(&r.value, from),
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

    fn ret(&mut self, value: &ReturnValue, from: From) {
        match value {
            ReturnValue::Text(text) => {
                self.step_to(from, NodeKind::Done, text.clone());
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
                    self.template_edges(&node, call);
                }
                _ => {
                    self.step_to(from, NodeKind::Unresolved, call_text(call));
                }
            },
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
            .replace('`', "#96;")
            .replace('\n', "<br/>")
    };
    let shape = |n: &Node| {
        let (id, l) = (&n.id, esc(&n.label));
        match n.kind {
            NodeKind::Function if n.entry => format!("{id}([\"{l}\"])"),
            NodeKind::Function => format!("{id}[[\"{l}\"]]"),
            NodeKind::Question | NodeKind::Condition => format!("{id}{{\"{l}\"}}"),
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
        let arrow = if e.kind == EdgeKind::Template { "-.->" } else { "-->" };
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
    out.join("\n") + "\n"
}

/// The graph as Graphviz DOT. A function with steps is a cluster.
pub fn dot(g: &Graph) -> String {
    let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n");
    let attrs = |n: &Node| {
        let shape = match n.kind {
            NodeKind::Function if n.entry => "shape=box, style=\"rounded,bold\"",
            NodeKind::Function => "shape=box, style=bold",
            NodeKind::Question | NodeKind::Condition => "shape=diamond",
            NodeKind::Request => "shape=parallelogram",
            NodeKind::Done => "shape=box, style=rounded",
            NodeKind::Delegate => "shape=cds",
            NodeKind::Program => "shape=box, style=dashed",
            NodeKind::Unresolved => "shape=octagon, color=red",
        };
        format!("{} [label=\"{}\", {shape}];", n.id, esc(&n.label))
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
        let a = if a.is_empty() { String::new() } else { format!(" [{}]", a.join(", ")) };
        out.push(format!("    {} -> {}{a};", e.from, e.to));
    }
    out.push("}".to_string());
    out.join("\n") + "\n"
}

#[cfg(test)]
#[path = "graph_tests.rs"]
mod tests;
