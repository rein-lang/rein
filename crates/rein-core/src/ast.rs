//! Syntax tree of one function file.

use crate::diagnostic::Span;

/// A `.struct.rein` file's project path ends with this, such as `ticket.struct` for `ticket.struct.rein`.
pub const STRUCT_SUFFIX: &str = ".struct";

/// True for the project path of a `.struct.rein` file. Such a file holds a struct, not a function.
pub fn is_struct(name: &str) -> bool {
    name.ends_with(STRUCT_SUFFIX)
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Spanned<T> {
    pub value: T,
    pub span: Span,
}

#[derive(Debug, Clone, Default)]
pub struct Function {
    /// Text of the function doc block. Filled from `doc`.
    pub description: Option<Spanned<String>>,
    /// The values the function reads by name: its scalar parameter, or the fields of its struct parameter.
    /// A struct parameter's fields are filled in by [`Function::bind_struct`]. Every later stage reads this list.
    pub inputs: Vec<Input>,
    /// The parameters as written. More than one is a checker error.
    pub params: Vec<Param>,
    /// Every `interface` in the file: inline input structs, or the one `export default interface` of a
    /// `.struct.rein` file.
    pub structs: Vec<Struct>,
    /// The struct that the struct parameter names, or why it does not resolve. `None` when the function has no
    /// struct parameter, or when a `@./` struct was not resolved, as in a file parsed outside a project.
    pub input_struct: Option<Result<Struct, String>>,
    /// The struct of each struct answer, by answer name, or why it does not resolve. Filled by
    /// [`Function::bind_answer_struct`]: an inline struct when the file parses, a `@./` struct when the project loads.
    pub answer_structs: std::collections::BTreeMap<String, Result<Struct, String>>,
    /// One input per field of each struct answer, named `answer.field`, such as `pick.pr`.
    pub answer_fields: Vec<Input>,
    /// The name after `export default function`. `None` when the file has no function.
    pub name: Option<Spanned<String>>,
    pub body: Vec<Stmt>,
    /// The `/** */` doc block right before `export default function`. `None` when the file has none.
    pub doc: Option<DocBlock>,
    /// `export default async function`.
    pub is_async: bool,
    /// The `async` keyword, when the function has it.
    pub async_span: Option<Span>,
    /// From `export` to the closing `}`. The function's lines are the lines this span covers. Empty when there is no function.
    pub span: Span,
    /// From `export` to the function name: the header the line-limit error points at.
    pub header: Span,
    /// The `import` lines, in file order. A use of a local name builds the same node as its `@` reference.
    pub imports: Vec<Import>,
}

/// One `import` line. Its local name stands for what the target declares: a function, a struct, or a program's
/// entry. A use of the local name builds the same node as the `@` reference of the path.
#[derive(Debug, Clone, PartialEq)]
pub struct Import {
    /// The name the target declares. A default import has its local name here.
    pub name: Spanned<String>,
    /// The name this file uses. The same as `name`, unless `as` gives another.
    pub local: Spanned<String>,
    /// The path as written: `./ci/fix`, `../x`, `./message.struct`, or a program name such as `pr-handling`.
    pub path: Spanned<String>,
    /// `import fix from "./ci/fix"`: the default form. It resolves like the named form.
    pub default: bool,
    /// `as` was written. Set even when it keeps the name.
    pub aliased: bool,
    /// From `import` to the path.
    pub span: Span,
    /// The file uses the local name. Set by the parser.
    pub used: bool,
}

impl Import {
    /// True when the path names a `.struct.rein` file.
    pub fn is_struct(&self) -> bool {
        self.path.value.ends_with(STRUCT_SUFFIX)
    }

    /// The path the `@` reference of this import would write: `./message` for `./message.struct`, else the path.
    pub fn reference(&self) -> &str {
        self.path.value.strip_suffix(STRUCT_SUFFIX).unwrap_or(&self.path.value)
    }
}

/// A parsed `/** */` doc block.
#[derive(Debug, Clone, Default)]
pub struct DocBlock {
    /// Span of the whole comment, from `/**` to `*/`.
    pub span: Span,
    /// Text before the first `@` line. Empty when there is none.
    pub description: Spanned<String>,
    /// Every `@` tag name, without the `@`, in file order. Each one gets a diagnostic from the checker.
    pub tags: Vec<Spanned<String>>,
}

impl Function {
    pub fn input(&self, name: &str) -> Option<&Input> {
        self.inputs.iter().find(|i| i.name.value == name)
    }

    /// The `const` answer named `name`, anywhere in the body.
    pub fn response(&self, name: &str) -> Option<&Input> {
        let mut found = None;
        walk_stmts(&self.body, &mut |s| {
            if let Stmt::Request { response, .. } = s {
                if found.is_none() && response.name.value == name {
                    found = Some(&**response);
                }
            }
        });
        found.or_else(|| self.answer_fields.iter().find(|i| i.name.value == name))
    }

    /// Every struct answer of the body, as `(answer name, struct it names)`.
    pub fn answer_struct_refs(&self) -> Vec<(String, StructRef)> {
        let mut out = Vec::new();
        walk_stmts(&self.body, &mut |s| {
            if let Stmt::Request { response, .. } = s {
                if let InputType::Struct(r) = &response.ty {
                    out.push((response.name.value.clone(), r.clone()));
                }
            }
        });
        out
    }

    /// The struct that the struct answer `name` names, or why it does not resolve.
    pub fn answer_struct(&self, name: &str) -> Option<&Result<Struct, String>> {
        self.answer_structs.get(name)
    }

    /// Records what the struct answer `name` names, and adds one input per field, named `name.field`.
    pub fn bind_answer_struct(&mut self, name: &str, found: Result<Struct, String>) {
        let at = self.response(name).map(|i| i.name.span).unwrap_or_default();
        if let Ok(s) = &found {
            for field in &s.fields {
                let name = Spanned { value: format!("{name}.{}", field.name.value), span: at };
                self.answer_fields.push(Input { name, span: at, ..field.clone() });
            }
        }
        self.answer_structs.insert(name.to_string(), found);
    }

    /// A template is never run by itself. See [`Function::template_literal`].
    pub fn is_template(&self) -> bool {
        self.template_literal().is_some()
    }

    /// The template literal of a template: every parameter is `string` or a format type, there is at least one, and
    /// the body is exactly one `return` of a template literal. Any other function is a normal function.
    pub fn template_literal(&self) -> Option<&TemplateLit> {
        if self.inputs.is_empty() || self.inputs.iter().any(|i| !i.ty.is_text()) {
            return None;
        }
        match self.body.as_slice() {
            [Stmt::Return(Spanned { value: ReturnValue::Template(lit), .. })] => Some(lit),
            _ => None,
        }
    }

    /// The struct parameter, when the function has one.
    pub fn struct_param(&self) -> Option<&StructParam> {
        self.params.iter().find_map(|p| match p {
            Param::Struct(s) => Some(s),
            Param::Scalar(_) => None,
        })
    }

    /// The `export default interface` of a `.struct.rein` file.
    pub fn exported_struct(&self) -> Option<&Struct> {
        self.structs.iter().find(|s| s.default)
    }

    /// Records what the struct parameter names, and adds one input per destructured field, in field order.
    /// Each input keeps the field's type and doc, at the span of its name in `{ }`.
    pub fn bind_struct(&mut self, found: Result<Struct, String>) {
        let Some(param) = self.struct_param().cloned() else { return };
        if let Ok(s) = &found {
            for field in &s.fields {
                if let Some(at) = param.fields.iter().find(|n| n.value == field.name.value) {
                    self.inputs.push(Input { name: at.clone(), span: at.span, ..field.clone() });
                }
            }
        }
        self.input_struct = Some(found);
    }

    /// The `string` and format parameters, in declaration order. Their values come from the caller.
    pub fn text_inputs(&self) -> impl Iterator<Item = &Input> {
        self.inputs.iter().filter(|i| i.ty.is_text())
    }
}

/// What a call or a return names: a std name, or a reference such as `@./ci/fix`.
#[derive(Debug, Clone, PartialEq)]
pub enum Callee {
    /// A std name, such as `delegate`. Std names need no import.
    Std(Spanned<String>),
    /// A reference, with the path as written, such as `./ci/fix` or `../escalate`.
    Ref(Spanned<String>),
}

impl Callee {
    /// The text the callee is known by in a [`Binding`] map: the std name, or the reference path.
    pub fn key(&self) -> &str {
        match self {
            Callee::Std(s) | Callee::Ref(s) => &s.value,
        }
    }

    pub fn span(&self) -> Span {
        match self {
            Callee::Std(s) | Callee::Ref(s) => s.span,
        }
    }
}

/// What a name or reference refers to.
#[derive(Debug, Clone)]
pub enum Binding<'a> {
    /// A function of the project, by project path. Returning it hands the agent to it.
    Function(String),
    /// A template, by project path (`None` for std). Calling it expands to text.
    Template { path: Option<String>, function: &'a Function },
    /// `request` from std. It is awaited, not returned or called.
    Request,
}

/// One parameter as written.
#[derive(Debug, Clone)]
pub enum Param {
    /// `/** question */ name: type`, where the type is `boolean`, `string` or a union.
    Scalar(Input),
    /// `{ a, b }: Name` or `{ a, b }: @./name`. The function reads the fields by name.
    Struct(StructParam),
}

impl Param {
    pub fn span(&self) -> Span {
        match self {
            Param::Scalar(i) => i.span,
            Param::Struct(s) => s.span,
        }
    }
}

/// A destructured struct parameter, such as `{ title, vip }: Ticket`.
#[derive(Debug, Clone)]
pub struct StructParam {
    /// The names inside `{ }`, in source order.
    pub fields: Vec<Spanned<String>>,
    pub ty: StructRef,
    pub span: Span,
}

/// The type of a struct parameter.
#[derive(Debug, Clone, PartialEq)]
pub enum StructRef {
    /// An `interface` in the same file, by name.
    Local(Spanned<String>),
    /// A `.struct.rein` file, by reference: `@./ticket` names `ticket.struct.rein`.
    Ref(Spanned<String>),
}

impl StructRef {
    pub fn name(&self) -> &Spanned<String> {
        match self {
            StructRef::Local(s) | StructRef::Ref(s) => s,
        }
    }
}

/// `interface Name { /** doc */ field: type; ... }`, with an optional `export` or `export default` before it.
#[derive(Debug, Clone)]
pub struct Struct {
    pub name: Spanned<String>,
    pub doc: Option<DocBlock>,
    /// Each field is like a scalar parameter: a doc comment, a name and a type.
    pub fields: Vec<Input>,
    /// The `export` keyword, when the struct has one.
    pub export: Option<Span>,
    /// `export default interface`.
    pub default: bool,
    pub span: Span,
}

impl Struct {
    pub fn field(&self, name: &str) -> Option<&Input> {
        self.fields.iter().find(|f| f.name.value == name)
    }
}

#[derive(Debug, Clone)]
pub struct Input {
    pub name: Spanned<String>,
    pub ty: InputType,
    /// The parameter's own doc comment, cleaned up. Empty, at the name span, when there is none.
    pub description: Spanned<String>,
    /// The parameter's doc block. `None` when the parameter has none.
    pub doc: Option<DocBlock>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum InputType {
    Bool,
    /// A union of string literals. `choice` is the name the CLI and JSON use.
    Choice(Vec<Spanned<String>>),
    /// `string`. Only a template takes it, and a field of a struct answer.
    Text,
    /// A string with a fixed shape, such as `IRI`. It goes where `string` goes; the run checks each value.
    Format(crate::formats::Format),
    /// A struct answer: an inline `interface` or a shared `@./name`. Only a `request` answer has it.
    Struct(StructRef),
}

impl InputType {
    pub fn options(&self) -> Option<Vec<&str>> {
        match self {
            InputType::Bool | InputType::Text | InputType::Format(_) | InputType::Struct(_) => None,
            InputType::Choice(o) => Some(o.iter().map(|s| s.value.as_str()).collect()),
        }
    }

    /// `string` or a format type: a value the caller passes as text.
    pub fn is_text(&self) -> bool {
        matches!(self, InputType::Text | InputType::Format(_))
    }

    /// The type as TypeScript writes it: `boolean`, `string`, or `"a" | "b"`.
    pub fn ts_type(&self) -> String {
        match self {
            InputType::Bool => "boolean".to_string(),
            InputType::Text => "string".to_string(),
            InputType::Format(f) => f.name().to_string(),
            InputType::Struct(r) => r.name().value.clone(),
            InputType::Choice(o) => o.iter().map(|s| format!("\"{}\"", s.value)).collect::<Vec<_>>().join(" | "),
        }
    }
}

#[derive(Debug, Clone)]
pub enum Stmt {
    /// A guard. The block must return on every path. Code after the `if` handles the other case.
    If {
        cond: Expr,
        then: Vec<Stmt>,
        span: Span,
    },
    /// Every member of the union has a case. The checker enforces it.
    Switch {
        subject: Spanned<String>,
        cases: Vec<Case>,
        span: Span,
    },
    /// `/** question */ const name: type = await request(arg);`, or `= await @program(args);`. The answer is a
    /// parameter for the rest of its block.
    Request {
        response: Box<Input>,
        awaited: Awaited,
        span: Span,
    },
    Return(Spanned<ReturnValue>),
}

impl Stmt {
    pub fn span(&self) -> Span {
        match self {
            Stmt::If { span, .. } | Stmt::Switch { span, .. } | Stmt::Request { span, .. } => *span,
            Stmt::Return(r) => r.span,
        }
    }
}

/// What an answer awaits.
#[derive(Debug, Clone)]
pub enum Awaited {
    /// `await request(arg)`. `callee` is the `request` name.
    Request { callee: Spanned<String>, arg: Value },
    /// `await @./fn`, `await @program(args)`: a call with return. The callee runs as a hand-off does. Its final
    /// text becomes a request whose answer is this `const`. Only `ReturnValue::Ref` or `ReturnValue::Call`.
    Call(ReturnValue),
}

/// One group of `case` labels that share a body, such as `case "a": case "b": return x;`.
#[derive(Debug, Clone)]
pub struct Case {
    pub values: Vec<Spanned<String>>,
    pub body: Vec<Stmt>,
}

/// What a function hands back to the agent.
#[derive(Debug, Clone, PartialEq)]
pub enum ReturnValue {
    /// Final answer or instruction for the agent. The span is the quoted string.
    Text(Spanned<String>),
    /// A reference to another function, such as `@./ci/fix`. The project resolves it.
    Ref(Spanned<String>),
    /// A template literal, such as `return `Pick ${c}`;`.
    Template(TemplateLit),
    /// A call of a template, such as `return delegate(model="a", task="b");` or `return @./tpl(a="a", b="b");`.
    Call(Call),
}

/// One part of a template literal.
#[derive(Debug, Clone, PartialEq)]
pub enum Piece {
    Text(String),
    /// `${name}`. The span covers the name only.
    Name(Spanned<String>),
}

/// A template literal, with its span.
#[derive(Debug, Clone, PartialEq)]
pub struct TemplateLit {
    pub pieces: Vec<Piece>,
    pub span: Span,
}

/// A value that becomes text: a string, a template literal, or a template call (only as an argument of `request`).
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Str(Spanned<String>),
    Template(TemplateLit),
    /// A bare name of a parameter or an answer of the caller, or `true`/`false`. Only a call argument can be one.
    Name(Spanned<String>),
    Call(Call),
}

impl Value {
    pub fn span(&self) -> Span {
        match self {
            Value::Str(s) | Value::Name(s) => s.span,
            Value::Template(lit) => lit.span,
            Value::Call(call) => call.span,
        }
    }
}

/// One argument of a call: `name=value`. The name is the parameter it binds to. The checker rejects an
/// argument with no name, so only `name=value` reaches the evaluator.
#[derive(Debug, Clone, PartialEq)]
pub struct Arg {
    /// `None` only for a positional argument, which the checker reports.
    pub name: Option<Spanned<String>>,
    pub value: Value,
    /// From the name to the end of the value. A positional argument starts at its value.
    pub span: Span,
}

/// `callee(args)`: a template call, or a hand-off to a function with parameters.
/// Each arg is `name=value`. For a `string` parameter the value is a string, a template literal, or a bare name of
/// a caller `string`. For a `boolean` or union parameter it is a bare name of a caller value of a fitting type, or a
/// literal: `true`/`false`, or a quoted member. The order of the args does not matter.
#[derive(Debug, Clone, PartialEq)]
pub struct Call {
    pub callee: Callee,
    pub args: Vec<Arg>,
    pub span: Span,
}

impl Call {
    /// This call, then every call nested in its arguments, at any depth.
    pub fn calls(&self) -> Vec<&Call> {
        let mut out = vec![self];
        for arg in &self.args {
            if let Value::Call(inner) = &arg.value {
                out.extend(inner.calls());
            }
        }
        out
    }
}

#[derive(Debug, Clone)]
pub enum Expr {
    /// A boolean parameter used bare.
    Var(Spanned<String>),
    /// `union_param === "member"` or `!==`.
    Compare { var: Spanned<String>, value: Spanned<String>, equal: bool },
    Not(Box<Expr>, Span),
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
}

impl Expr {
    pub fn span(&self) -> Span {
        match self {
            Expr::Var(v) => v.span,
            Expr::Compare { var, value, .. } => var.span.to(value.span),
            Expr::Not(e, s) => s.to(e.span()),
            Expr::And(a, b) | Expr::Or(a, b) => a.span().to(b.span()),
        }
    }
}

/// Calls `f` on every statement, depth first.
pub fn walk_stmts<'a>(stmts: &'a [Stmt], f: &mut impl FnMut(&'a Stmt)) {
    for s in stmts {
        f(s);
        match s {
            Stmt::If { then, .. } => walk_stmts(then, f),
            Stmt::Switch { cases, .. } => {
                for c in cases {
                    walk_stmts(&c.body, f);
                }
            }
            Stmt::Request { .. } | Stmt::Return(_) => {}
        }
    }
}

/// Calls `f` on every variable use (bare boolean or compared union) in an expression.
pub fn walk_vars<'a>(e: &'a Expr, f: &mut impl FnMut(&'a Spanned<String>)) {
    match e {
        Expr::Var(v) => f(v),
        Expr::Compare { var, .. } => f(var),
        Expr::Not(e, _) => walk_vars(e, f),
        Expr::And(a, b) | Expr::Or(a, b) => {
            walk_vars(a, f);
            walk_vars(b, f);
        }
    }
}
