//! Syntax tree of one function file.

use crate::diagnostic::Span;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Spanned<T> {
    pub value: T,
    pub span: Span,
}

#[derive(Debug, Clone, Default)]
pub struct Function {
    /// Text of the function doc block. Filled from `doc`.
    pub description: Option<Spanned<String>>,
    pub inputs: Vec<Input>,
    pub body: Vec<Stmt>,
    /// The `/** */` doc block right before `export default function`. `None` when the file has none.
    pub doc: Option<DocBlock>,
    /// `export default async function`.
    pub is_async: bool,
    /// The `async` keyword, when the function has it.
    pub async_span: Option<Span>,
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
        found
    }

    /// A template is never run by itself. See [`Function::template_literal`].
    pub fn is_template(&self) -> bool {
        self.template_literal().is_some()
    }

    /// The template literal of a template: every parameter is `string`, there is at least one, and the body is
    /// exactly one `return` of a template literal. Any other function is a normal function.
    pub fn template_literal(&self) -> Option<&TemplateLit> {
        if self.inputs.is_empty() || self.inputs.iter().any(|i| i.ty != InputType::Text) {
            return None;
        }
        match self.body.as_slice() {
            [Stmt::Return(Spanned { value: ReturnValue::Template(lit), .. })] => Some(lit),
            _ => None,
        }
    }

    /// The `string` parameters, in declaration order. Their values come from the caller.
    pub fn text_inputs(&self) -> impl Iterator<Item = &Input> {
        self.inputs.iter().filter(|i| i.ty == InputType::Text)
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
    /// `string`. Only a template takes it.
    Text,
}

impl InputType {
    pub fn options(&self) -> Option<Vec<&str>> {
        match self {
            InputType::Bool | InputType::Text => None,
            InputType::Choice(o) => Some(o.iter().map(|s| s.value.as_str()).collect()),
        }
    }

    /// The type as TypeScript writes it: `boolean`, `string`, or `"a" | "b"`.
    pub fn ts_type(&self) -> String {
        match self {
            InputType::Bool => "boolean".to_string(),
            InputType::Text => "string".to_string(),
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
    /// `/** question */ const name: type = await request(arg);`. The answer is a parameter for the rest of its block.
    Request {
        response: Box<Input>,
        /// The `request` name.
        callee: Spanned<String>,
        arg: Value,
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

/// One group of `case` labels that share a body, such as `case "a": case "b": return x;`.
#[derive(Debug, Clone)]
pub struct Case {
    pub values: Vec<Spanned<String>>,
    pub body: Vec<Stmt>,
}

/// What a function hands back to the agent.
#[derive(Debug, Clone, PartialEq)]
pub enum ReturnValue {
    /// Final answer or instruction for the agent.
    Text(String),
    /// A reference to another function, such as `@./ci/fix`. The project resolves it.
    Ref(Spanned<String>),
    /// A template literal, such as `return `Pick ${c}`;`.
    Template(TemplateLit),
    /// A call of a template, such as `return delegate("a", "b");` or `return @./tpl("a", "b");`.
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
    /// A bare name of a `string` parameter of the caller. Only a call argument can be one.
    Name(Spanned<String>),
    Call(Call),
}

/// `callee(args)`: a template call, or a hand-off to a function that has `string` parameters.
/// The args are strings, template literals, or bare names of the caller's `string` parameters.
#[derive(Debug, Clone, PartialEq)]
pub struct Call {
    pub callee: Callee,
    pub args: Vec<Value>,
    pub span: Span,
}

impl Call {
    /// This call, then every call nested in its arguments, at any depth.
    pub fn calls(&self) -> Vec<&Call> {
        let mut out = vec![self];
        for arg in &self.args {
            if let Value::Call(inner) = arg {
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
