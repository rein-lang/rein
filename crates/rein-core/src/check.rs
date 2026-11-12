//! Single-file checks: names, doc blocks, parameters, conditions, switches, returns, references and templates.

use std::collections::BTreeSet;

use crate::ast::{
    Binding, Callee, Call, Case, DocBlock, Expr, Function, Input, InputType, Param, Piece, ReturnValue, Spanned, Stmt, Struct,
    StructParam, StructRef, TemplateLit, Value, STRUCT_SUFFIX,
};
use crate::diagnostic::{line_col, Diagnostic, Span};
use crate::lexer::{KEYWORDS, RESERVED};
use crate::stdlib;

/// What a reference or a std name is, as the checker needs it. A template needs its parameter names.
/// A function needs the names of its `string` parameters, which a hand-off must pass as named arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    Function { strings: Vec<String> },
    Template { params: Vec<String> },
    Request,
}

/// `1 argument` or `2 arguments`.
fn count_args(n: usize) -> String {
    if n == 1 {
        "1 argument".to_string()
    } else {
        format!("{n} arguments")
    }
}

/// Checks one function. `name` is its project path, used for the file-name rule.
/// `resolve` says what a reference path (such as `./ci/fix`) names, or why it does not resolve.
pub fn check_function<'a>(f: &'a Function, name: &str, resolve: &'a dyn Fn(&str) -> Result<Kind, String>) -> Vec<Diagnostic> {
    let mut c = Checker {
        f,
        resolve,
        out: Vec::new(),
        used: BTreeSet::new(),
        answers: Vec::new(),
        pending: Vec::new(),
        names: BTreeSet::new(),
    };
    if let Some(stem) = name.strip_suffix(STRUCT_SUFFIX) {
        c.struct_file(stem);
        return c.out;
    }
    if !valid_name(name) {
        c.error(
            Span::default(),
            format!(
                "invalid function file name `{name}`; each `/` part must match [A-Za-z_][A-Za-z0-9_]* and must not be a keyword"
            ),
        );
    }
    c.doc(name);
    c.params();
    c.structs();
    if let Some(lit) = f.template_literal() {
        c.template(lit);
    } else if f.body.is_empty() {
        let span = f.doc.as_ref().map_or_else(Span::default, |d| d.span);
        c.error(span, "function must return something");
    } else {
        c.block(&f.body);
        if !terminates_block(f, &f.body) {
            let span = f
                .body
                .last()
                .map(Stmt::span)
                .or(f.description.as_ref().map(|d| d.span))
                .unwrap_or_default();
            c.error(span, "every path must return something; end the function with a return");
        }
        c.unused_params();
        if f.is_async && !has_request(&f.body) {
            if let Some(span) = f.async_span {
                c.warning(span, "`async` function has no `await`; drop `async`");
            }
        }
    }
    c.out
}

/// The line limit of a function when `rein.json` sets none.
pub const DEFAULT_MAX_FUNCTION_LINES: u64 = 40;

/// Checks the length of one function file. The function's lines run from its `export default` line to its
/// closing `}`, both included. Blank and comment lines count. The doc comment above it does not.
/// `text` is the file's source, and `max` the limit. Longer than `max` is an error on the function header.
pub fn check_length(f: &Function, name: &str, text: &str, max: u64) -> Vec<Diagnostic> {
    if f.name.is_none() {
        return Vec::new();
    }
    let (first, _) = line_col(text, f.span.start);
    let (last, _) = line_col(text, f.span.end);
    let lines = (last - first + 1) as u64;
    if lines <= max {
        return Vec::new();
    }
    vec![Diagnostic::error(
        f.header,
        format!(
            "function `{name}` has {lines} lines; the limit is {max}. Move part of it into its own function and call it with `@./name`"
        ),
    )]
}

fn has_request(stmts: &[Stmt]) -> bool {
    let mut found = false;
    crate::ast::walk_stmts(stmts, &mut |s| found |= matches!(s, Stmt::Request { .. }));
    found
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.split('/').all(|seg| {
            let mut chars = seg.chars();
            chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
                && !KEYWORDS.contains(&seg)
                && !RESERVED.contains(&seg)
        })
}

fn terminates_block(f: &Function, stmts: &[Stmt]) -> bool {
    stmts.iter().any(|s| stmt_terminates(f, s))
}

fn stmt_terminates(f: &Function, s: &Stmt) -> bool {
    match s {
        Stmt::Request { .. } | Stmt::If { .. } => false,
        Stmt::Return(_) => true,
        Stmt::Switch { subject, cases, .. } => {
            let covers = missing_members(f, &subject.value, cases).is_empty();
            cases.iter().all(|c| terminates_block(f, &c.body)) && covers
        }
    }
}

/// Members of the subject union that no case names. Empty when the subject is not a union.
fn missing_members<'a>(f: &'a Function, subject: &str, cases: &[Case]) -> Vec<&'a Spanned<String>> {
    match f.input(subject).or_else(|| f.response(subject)).map(|i| &i.ty) {
        Some(InputType::Choice(members)) => members
            .iter()
            .filter(|m| !cases.iter().any(|c| c.values.iter().any(|v| v.value == m.value)))
            .collect(),
        _ => Vec::new(),
    }
}

/// A `string` input cannot decide a branch. A condition and a switch subject both give this error.
pub fn string_in_condition(name: &str) -> String {
    format!("`{name}` is a string input; a condition cannot read it")
}

fn members_text(members: &[Spanned<String>]) -> String {
    members.iter().map(|o| format!("`{}`", o.value)).collect::<Vec<_>>().join(", ")
}

/// `` `a` ``, `` `a` and `b` ``, or `` `a`, `b` and `c` ``.
fn and_list(names: &[String]) -> String {
    let quoted: Vec<String> = names.iter().map(|n| format!("`{n}`")).collect();
    match quoted.split_last() {
        Some((last, rest)) if !rest.is_empty() => format!("{} and {last}", rest.join(", ")),
        _ => quoted.concat(),
    }
}

/// The names a parameter brings: its own name, or the names in `{ }`.
fn written_names(p: &Param) -> Vec<String> {
    match p {
        Param::Scalar(i) => vec![i.name.value.clone()],
        Param::Struct(s) => s.fields.iter().map(|n| n.value.clone()).collect(),
    }
}

const STRUCT_FILE_ONLY: &str = "a `.struct.rein` file holds one `export default interface` and nothing else";

struct Checker<'a> {
    f: &'a Function,
    resolve: &'a dyn Fn(&str) -> Result<Kind, String>,
    out: Vec<Diagnostic>,
    /// Parameters used in a condition, a switch subject, or a template.
    used: BTreeSet<String>,
    /// `const` answers visible here: declared earlier in an enclosing block.
    answers: Vec<Input>,
    /// Answer names declared later in an enclosing block. Using one of them early is an error.
    pending: Vec<String>,
    /// Every answer name seen in the function, for the duplicate-name rule.
    names: BTreeSet<String>,
}

impl<'a> Checker<'a> {
    fn error(&mut self, span: Span, msg: impl Into<String>) {
        self.out.push(Diagnostic::error(span, msg));
    }

    fn warning(&mut self, span: Span, msg: impl Into<String>) {
        self.out.push(Diagnostic::warning(span, msg));
    }

    fn unknown(&mut self, name: &Spanned<String>) {
        if self.pending.contains(&name.value) {
            self.error(name.span, format!("`{}` is used before it is declared", name.value));
        } else {
            self.error(name.span, format!("unknown parameter `{}`", name.value));
        }
    }

    /// What a name refers to here: an answer declared earlier in an enclosing block, or a parameter.
    fn lookup(&self, name: &str) -> Option<InputType> {
        self.answers
            .iter()
            .rev()
            .find(|i| i.name.value == name)
            .map(|i| i.ty.clone())
            .or_else(|| self.f.input(name).map(|i| i.ty.clone()))
    }

    /// A value of `string`, a template literal, a bare name of a `string` parameter, or a call.
    /// A call here is not a hand-off: only a `return` hands the agent to a function.
    fn value(&mut self, v: &Value) {
        match v {
            Value::Str(_) => {}
            Value::Template(lit) => self.template_names(lit),
            Value::Name(name) => self.string_arg(name),
            Value::Call(call) => self.call(call, false),
        }
    }

    /// A bare name as a call argument: it must be a `string` parameter of this function.
    fn string_arg(&mut self, name: &Spanned<String>) {
        self.used.insert(name.value.clone());
        if self.lookup(&name.value) != Some(InputType::Text) {
            self.error(name.span, format!("`{}` is not a string input", name.value));
        }
    }

    /// Rules for the function's doc block: its presence, the description, and its tags.
    /// Every function must say what it decides; there is no flag to skip this.
    fn doc(&mut self, name: &str) {
        let f = self.f;
        let Some(doc) = &f.doc else {
            self.error(
                Span::default(),
                format!("missing doc comment on `{name}`; put /** description */ right before `export default function`"),
            );
            return;
        };
        if f.description.as_ref().is_none_or(|d| d.value.trim().is_empty()) {
            self.error(doc.span, format!("empty description on `{name}`; write the function description in the doc comment"));
        }
        self.tags(doc);
    }

    /// `@param` is not allowed anywhere. Other tags are warnings.
    fn tags(&mut self, doc: &DocBlock) {
        for tag in &doc.tags {
            if tag.value == "param" {
                self.error(tag.span, "describe each parameter in its own doc comment, right before it");
            } else {
                self.warning(tag.span, format!("unknown tag `@{}`; Rein supports no tags yet", tag.value));
            }
        }
    }

    /// A function takes one parameter at most. A scalar parameter has a doc comment and a valid type. A struct
    /// parameter names a struct and destructures every field of it.
    fn params(&mut self) {
        let f = self.f;
        if f.params.len() > 1 {
            let names: Vec<String> = f.params.iter().flat_map(written_names).collect();
            let span = f.params[1].span().to(f.params[f.params.len() - 1].span());
            self.error(span, format!("a function takes one parameter; put {} in a struct", and_list(&names)));
        }
        let scalars: Vec<&Input> = f
            .params
            .iter()
            .filter_map(|p| match p {
                Param::Scalar(i) => Some(i),
                Param::Struct(_) => None,
            })
            .collect();
        self.declarations(&scalars, "parameter");
        if let Some(p) = f.struct_param() {
            match &f.input_struct {
                Some(Err(msg)) => self.error(p.ty.name().span, msg.clone()),
                Some(Ok(s)) => self.destructure(p, s),
                None => {}
            }
        }
    }

    /// Doc comments, unique names and union members, for parameters or for the fields of a struct.
    /// `noun` is `parameter` or `field`.
    fn declarations(&mut self, inputs: &[&Input], noun: &str) {
        for (i, input) in inputs.iter().enumerate() {
            let name = &input.name.value;
            if inputs[..i].iter().any(|p| p.name.value == *name) {
                self.error(input.name.span, format!("duplicate {noun} `{name}`"));
            }
            if let Some(doc) = &input.doc {
                self.tags(doc);
            }
            if input.description.value.trim().is_empty() {
                self.error(
                    input.name.span,
                    format!("{noun} `{name}` needs a doc comment: /** question */ {name}: ..."),
                );
            }
            if let InputType::Choice(members) = &input.ty {
                if members.len() < 2 {
                    self.error(input.span, format!("union {noun} `{name}` needs at least 2 members"));
                }
                for (j, m) in members.iter().enumerate() {
                    if m.value.trim().is_empty() {
                        self.error(m.span, format!("empty member in union {noun} `{name}`"));
                    } else if members[..j].iter().any(|p| p.value == m.value) {
                        self.error(m.span, format!("duplicate member `{}` in union {noun} `{name}`", m.value));
                    }
                }
            }
        }
    }

    /// `{ a, b }: S` names only fields of `S`, each once, and every field of `S`.
    fn destructure(&mut self, p: &StructParam, s: &Struct) {
        let fields: Vec<&str> = s.fields.iter().map(|f| f.name.value.as_str()).collect();
        for (i, n) in p.fields.iter().enumerate() {
            if !fields.contains(&n.value.as_str()) {
                self.error(n.span, format!("`{}` is not a field of `{}`; fields are {}", n.value, s.name.value, fields.join(", ")));
            } else if p.fields[..i].iter().any(|o| o.value == n.value) {
                self.error(n.span, format!("`{}` is destructured twice", n.value));
            }
        }
        for field in &fields {
            if !p.fields.iter().any(|n| n.value == *field) {
                self.error(p.span, format!("destructure every field of `{}`; missing: `{field}`", s.name.value));
            }
        }
    }

    /// The fields of one `interface`.
    fn fields(&mut self, s: &Struct) {
        let fields: Vec<&Input> = s.fields.iter().collect();
        self.declarations(&fields, "field");
    }

    /// The `interface`s of a function file. Each one is an inline input struct: it is not exported, and the
    /// function's struct parameter names it.
    fn structs(&mut self) {
        let f = self.f;
        let used = f.struct_param().and_then(|p| match &p.ty {
            StructRef::Local(n) => Some(n.value.as_str()),
            StructRef::Ref(_) => None,
        });
        for s in &f.structs {
            if s.export.is_some() {
                self.error(
                    s.name.span,
                    format!("input struct `{}` cannot be exported; to share it, move it to its own `.struct.rein` file", s.name.value),
                );
            }
            if used != Some(s.name.value.as_str()) {
                self.warning(
                    s.name.span,
                    format!("unused struct `{}`; an inline struct is the input of the function in its file", s.name.value),
                );
            }
            self.fields(s);
        }
    }

    /// A `.struct.rein` file holds one `export default interface`, with a doc comment, and nothing else.
    fn struct_file(&mut self, stem: &str) {
        let f = self.f;
        if !valid_name(stem) {
            self.error(
                Span::default(),
                format!(
                    "invalid struct file name `{stem}.struct.rein`; each `/` part must match [A-Za-z_][A-Za-z0-9_]* and must not be a keyword"
                ),
            );
        }
        if let Some(name) = &f.name {
            self.error(name.span, format!("{STRUCT_FILE_ONLY}; move the function to its own `.rein` file"));
        }
        let mut exported = false;
        for s in &f.structs {
            if !s.default || exported {
                self.error(s.name.span, STRUCT_FILE_ONLY);
            } else {
                exported = true;
                let name = &s.name.value;
                match &s.doc {
                    Some(doc) if !doc.description.value.trim().is_empty() => self.tags(doc),
                    Some(doc) => {
                        self.error(doc.span, format!("empty description on struct `{name}`; write what it holds in the doc comment"))
                    }
                    None => self.error(
                        s.name.span,
                        format!("missing doc comment on struct `{name}`; put /** description */ right before `export default interface`"),
                    ),
                }
            }
            self.fields(s);
        }
    }

    /// A template: every parameter is `string` and the body is `lit`, so each parameter must be used in it.
    fn template(&mut self, lit: &TemplateLit) {
        let f = self.f;
        let mut used = BTreeSet::new();
        for piece in &lit.pieces {
            if let Piece::Name(n) = piece {
                used.insert(n.value.clone());
                if f.input(&n.value).is_none() {
                    self.unknown(n);
                }
            }
        }
        for input in &f.inputs {
            if !used.contains(&input.name.value) {
                let name = &input.name.value;
                self.error(input.name.span, format!("template parameter `{name}` is not used; add ${{{name}}} to the template"));
            }
        }
    }

    fn unused_params(&mut self) {
        let f = self.f;
        for input in &f.inputs {
            if !self.used.contains(&input.name.value) {
                self.warning(input.name.span, format!("unused parameter `{}`", input.name.value));
            }
        }
    }

    /// A block has no code after a terminating statement, such as a `return` or an exhaustive `switch`.
    fn block(&mut self, stmts: &[Stmt]) {
        let (answers_mark, pending_mark) = (self.answers.len(), self.pending.len());
        self.pending.extend(stmts.iter().filter_map(|s| match s {
            Stmt::Request { response, .. } => Some(response.name.value.clone()),
            _ => None,
        }));
        if let Some(i) = stmts.iter().position(|s| stmt_terminates(self.f, s)) {
            if let Some(next) = stmts.get(i + 1) {
                self.error(next.span(), "unreachable statement");
            }
        }
        for s in stmts {
            self.stmt(s);
        }
        self.answers.truncate(answers_mark);
        self.pending.truncate(pending_mark);
    }

    fn stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::If { cond, then, .. } => {
                self.expr(cond);
                self.block(then);
                if !terminates_block(self.f, then) {
                    self.error(s.span(), "every `if` block must end in a return");
                }
            }
            Stmt::Switch { subject, cases, .. } => {
                self.switch(subject, cases);
            }
            Stmt::Request { response, arg, .. } => {
                let name = response.name.value.as_str();
                if !self.f.is_async {
                    self.error(s.span(), "`await` needs `export default async function`");
                }
                if response.description.value.trim().is_empty() {
                    self.error(
                        response.name.span,
                        format!("const `{name}` needs a doc comment: /** question */ const {name}: ..."),
                    );
                }
                if self.f.input(name).is_some() || self.names.contains(name) {
                    self.error(response.name.span, format!("duplicate name `{name}`"));
                }
                self.names.insert(name.to_string());
                self.value(arg);
                self.answers.push((**response).clone());
            }
            Stmt::Return(r) => match &r.value {
                ReturnValue::Text(_) => {}
                ReturnValue::Ref(reference) => self.returned_ref(reference),
                ReturnValue::Template(lit) => self.template_names(lit),
                ReturnValue::Call(call) => self.call(call, true),
            },
        }
    }

    /// A `return` of a reference. It must name a function, not a template. A function with `string`
    /// parameters needs them as arguments, so a bare reference to it is an error.
    fn returned_ref(&mut self, reference: &Spanned<String>) {
        let path = reference.value.as_str();
        match (self.resolve)(path) {
            Err(msg) => self.error(reference.span, msg),
            Ok(Kind::Template { .. }) => self.error(
                reference.span,
                format!("`@{path}` is a template; call it with arguments, like `@{path}(...)`"),
            ),
            Ok(Kind::Function { strings }) if !strings.is_empty() => self.error(
                reference.span,
                format!("`@{path}` takes {}, got 0", count_args(strings.len())),
            ),
            Ok(Kind::Function { .. } | Kind::Request) => {}
        }
    }

    /// A call. A call of a template expands to text. A `return` of a call to a function is a hand-off that
    /// passes the `string` arguments. Elsewhere, only a template can be called.
    fn call(&mut self, call: &Call, hand_off: bool) {
        match &call.callee {
            Callee::Ref(path) => match (self.resolve)(&path.value) {
                Err(msg) => self.error(path.span, msg),
                Ok(Kind::Function { .. }) if !hand_off => {
                    self.error(path.span, "A hand-off cannot be an argument of a call.");
                }
                Ok(Kind::Function { strings }) if strings.is_empty() => {
                    self.error(call.span, format!("`@{}` takes no arguments; return it without a call", path.value));
                }
                Ok(Kind::Function { strings }) => self.arguments(call, &strings, &format!("@{}", path.value)),
                Ok(Kind::Template { params }) => self.arguments(call, &params, &format!("@{}", path.value)),
                Ok(Kind::Request) => {}
            },
            Callee::Std(name) => match stdlib::item(&name.value) {
                Err(msg) => self.error(name.span, msg),
                Ok(Binding::Request) => {
                    self.error(name.span, "`request` is not a template; use `await request(...)`");
                }
                Ok(Binding::Template { function, .. }) => {
                    let params: Vec<String> = function.inputs.iter().map(|i| i.name.value.clone()).collect();
                    self.arguments(call, &params, &name.value);
                }
                // Std names are templates or `request`, never functions.
                Ok(Binding::Function(_)) => {}
            },
        }
        for arg in &call.args {
            self.value(&arg.value);
        }
    }

    /// The arguments of a call bind by name. Each one is `name=value`, its name is a parameter, and no name
    /// repeats. Every parameter is given, by name or by its place in a positional argument.
    fn arguments(&mut self, call: &Call, params: &[String], target: &str) {
        let list = params.join(", ");
        let mut given: BTreeSet<&str> = BTreeSet::new();
        for (i, arg) in call.args.iter().enumerate() {
            let Some(name) = &arg.name else {
                match params.get(i) {
                    Some(p) => self.error(arg.span, format!("missing argument `{p}` in call to `{target}`; write `{p}=value`")),
                    None => self.error(arg.span, format!("unexpected argument in call to `{target}`; parameters are {list}")),
                }
                continue;
            };
            if !params.contains(&name.value) {
                self.error(name.span, format!("`{}` is not a parameter of `{target}`; parameters are {list}", name.value));
            } else if !given.insert(name.value.as_str()) {
                self.error(name.span, format!("`{}` is given twice in call to `{target}`; give each name once", name.value));
            }
        }
        for (i, p) in params.iter().enumerate() {
            let positional = call.args.get(i).is_some_and(|a| a.name.is_none());
            if !given.contains(p.as_str()) && !positional {
                self.error(call.span, format!("missing argument `{p}` in call to `{target}`; write `{p}=value`"));
            }
        }
    }

    /// Every `${name}` in a template literal must be a union parameter, not a boolean.
    fn template_names(&mut self, lit: &TemplateLit) {
        for piece in &lit.pieces {
            if let Piece::Name(n) = piece {
                self.used.insert(n.value.clone());
                match self.lookup(&n.value) {
                    None => self.unknown(n),
                    Some(InputType::Bool) => self.error(
                        n.span,
                        format!("boolean parameter `{}` cannot be interpolated; only union parameters can go in a template", n.value),
                    ),
                    Some(_) => {}
                }
            }
        }
    }

    fn expr(&mut self, e: &Expr) {
        match e {
            Expr::Var(v) => {
                self.used.insert(v.value.clone());
                match self.lookup(&v.value) {
                    None => self.unknown(v),
                    Some(InputType::Choice(_)) => {
                        let n = &v.value;
                        self.error(
                            v.span,
                            format!("union parameter `{n}` needs a comparison, like `{n} === \"...\"`"),
                        );
                    }
                    Some(InputType::Text) => self.error(v.span, string_in_condition(&v.value)),
                    Some(_) => {}
                }
            }
            Expr::Compare { var, value, .. } => {
                self.used.insert(var.value.clone());
                let n = var.value.as_str();
                match self.lookup(n) {
                    None => self.unknown(var),
                    Some(InputType::Bool) => self.error(
                        var.span,
                        format!("boolean parameter `{n}` cannot be compared; use `if ({n})` or `if (!{n})`"),
                    ),
                    Some(InputType::Choice(members)) => self.check_member(n, value, &members),
                    Some(InputType::Text) => self.error(var.span, string_in_condition(n)),
                }
            }
            Expr::Not(inner, _) => self.expr(inner),
            Expr::And(a, b) | Expr::Or(a, b) => {
                self.expr(a);
                self.expr(b);
            }
        }
    }

    fn check_member(&mut self, param: &str, value: &Spanned<String>, members: &[Spanned<String>]) {
        if !members.iter().any(|m| m.value == value.value) {
            let list = members_text(members);
            self.error(
                value.span,
                format!("`{}` is not a member of `{param}`; members are {list}", value.value),
            );
        }
    }

    fn switch(&mut self, subject: &Spanned<String>, cases: &[Case]) {
        self.used.insert(subject.value.clone());
        let n = subject.value.as_str();
        match self.lookup(n) {
            None => self.unknown(subject),
            Some(InputType::Bool) => self.error(
                subject.span,
                format!("boolean parameter `{n}` cannot be used in a switch; use `if ({n})` or `if (!{n})`"),
            ),
            Some(InputType::Text) => self.error(subject.span, string_in_condition(n)),
            Some(InputType::Choice(members)) => {
                let mut seen: Vec<&str> = Vec::new();
                for c in cases {
                    for v in &c.values {
                        self.check_member(n, v, &members);
                        if seen.contains(&v.value.as_str()) {
                            self.error(v.span, format!("duplicate case `{}`", v.value));
                        }
                        seen.push(&v.value);
                    }
                }
            }
        }
        for c in cases {
            self.block(&c.body);
            if !terminates_block(self.f, &c.body) {
                let at = c.values.first().map_or(subject.span, |v| v.span);
                self.error(at, "case falls through; end it with return");
            }
        }
        let missing = missing_members(self.f, n, cases);
        if !missing.is_empty() && self.lookup(n).is_some() {
            let list = members_text(&missing.into_iter().cloned().collect::<Vec<_>>());
            self.error(
                subject.span,
                format!("switch on `{n}` needs a case for every member; missing: {list}"),
            );
        }
    }
}

#[cfg(test)]
#[path = "check_tests.rs"]
mod tests;
