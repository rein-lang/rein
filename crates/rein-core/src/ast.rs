//! Syntax tree of one function file.

use crate::diagnostic::Span;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Spanned<T> {
    pub value: T,
    pub span: Span,
}

#[derive(Debug, Clone, Default)]
pub struct Function {
    /// Text of the doc block before the first `@` tag. Filled from `doc`.
    pub description: Option<Spanned<String>>,
    pub inputs: Vec<Input>,
    pub body: Vec<Stmt>,
    /// The `/** */` doc block at the top of the file. `None` when the file has none.
    pub doc: Option<DocBlock>,
}

/// The parsed `/** */` doc block.
#[derive(Debug, Clone, Default)]
pub struct DocBlock {
    /// Span of the whole comment, from `/**` to `*/`.
    pub span: Span,
    /// Text before the first `@` line. Empty when there is none.
    pub description: Spanned<String>,
    /// `@param` tags, in file order.
    pub params: Vec<ParamTag>,
    /// Tags other than `@param`, without the `@`. Each one gets a warning.
    pub unknown_tags: Vec<Spanned<String>>,
}

/// One `@param <name> [-] <text>` tag.
#[derive(Debug, Clone)]
pub struct ParamTag {
    /// The input name. Empty when the tag has no name.
    pub name: Spanned<String>,
    /// The question after the name, joined over continuation lines. Empty when none.
    pub text: Spanned<String>,
}

impl Function {
    pub fn input(&self, name: &str) -> Option<&Input> {
        self.inputs.iter().find(|i| i.name.value == name)
    }
}

#[derive(Debug, Clone)]
pub struct Input {
    pub name: Spanned<String>,
    pub ty: InputType,
    /// The `@param` text for this input. Empty, at the name span, when there is no tag.
    pub description: Spanned<String>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum InputType {
    Bool,
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

#[derive(Debug, Clone)]
pub struct Case {
    pub value: Spanned<String>,
    pub body: Vec<Stmt>,
}

/// What a function hands back to the agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReturnValue {
    /// Final answer or instruction for the agent.
    Text(String),
    /// Continue with another function, by its project path (no `@`, no `.rein`).
    Ref(String),
}

#[derive(Debug, Clone)]
pub enum Expr {
    /// A bool input.
    Var(Spanned<String>),
    /// `choice_input == "option"` or `!=`.
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

/// Calls `f` on every variable use (bare bool or compared choice) in an expression.
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
