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
    /// `import` lines, in file order.
    pub imports: Vec<Import>,
    pub inputs: Vec<Input>,
    pub body: Vec<Stmt>,
    /// The `/** */` doc block right before `export default function`. `None` when the file has none.
    pub doc: Option<DocBlock>,
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

    pub fn import(&self, binding: &str) -> Option<&Import> {
        self.imports.iter().find(|i| i.binding.value == binding)
    }
}

/// `import <binding> from "<source>"`.
#[derive(Debug, Clone)]
pub struct Import {
    /// The local name the body uses to return the imported function.
    pub binding: Spanned<String>,
    /// The quoted path, as written. Its span includes the quotes.
    pub source: Spanned<String>,
    pub span: Span,
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
}

impl InputType {
    pub fn options(&self) -> Option<Vec<&str>> {
        match self {
            InputType::Bool => None,
            InputType::Choice(o) => Some(o.iter().map(|s| s.value.as_str()).collect()),
        }
    }
}

#[derive(Debug, Clone)]
pub enum Stmt {
    If {
        cond: Expr,
        then: Vec<Stmt>,
        /// `else if` is stored as an else block holding one `If`.
        otherwise: Option<Vec<Stmt>>,
        span: Span,
    },
    Switch {
        subject: Spanned<String>,
        cases: Vec<Case>,
        default: Option<Vec<Stmt>>,
        span: Span,
    },
    Return(Spanned<ReturnValue>),
}

impl Stmt {
    pub fn span(&self) -> Span {
        match self {
            Stmt::If { span, .. } | Stmt::Switch { span, .. } => *span,
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReturnValue {
    /// Final answer or instruction for the agent.
    Text(String),
    /// The local name of an imported function. The project resolves it to a function.
    Ref(String),
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
            Stmt::If { then, otherwise, .. } => {
                walk_stmts(then, f);
                if let Some(o) = otherwise {
                    walk_stmts(o, f);
                }
            }
            Stmt::Switch { cases, default, .. } => {
                for c in cases {
                    walk_stmts(&c.body, f);
                }
                if let Some(d) = default {
                    walk_stmts(d, f);
                }
            }
            Stmt::Return(_) => {}
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
