//! Single-file checks: names, inputs, conditions, switches and returns.

use std::collections::BTreeSet;

use crate::ast::{Case, Expr, Function, Input, InputType, ReturnValue, Spanned, Stmt};
use crate::diagnostic::{Diagnostic, Span};
use crate::lexer::KEYWORDS;

/// Checks one function. `name` is its project path, used for the file-name rule.
/// `known_functions` tells whether `@name` is a function in the project.
pub fn check_function(
    f: &Function,
    name: &str,
    known_functions: &dyn Fn(&str) -> bool,
) -> Vec<Diagnostic> {
    let mut c = Checker { f, known: known_functions, out: Vec::new(), used: BTreeSet::new() };
    if !valid_name(name) {
        c.error(
            Span::default(),
            format!(
                "invalid function file name `{name}`; each `/` part must match [A-Za-z_][A-Za-z0-9_]* and must not be a keyword"
            ),
        );
    }
    c.description();
    c.inputs();
    c.block(&f.body);
    if !terminates_block(f, &f.body) {
        let span = f
            .body
            .last()
            .map(Stmt::span)
            .or(f.description.as_ref().map(|d| d.span))
            .unwrap_or_default();
        c.error(span, "not all paths return a value");
    }
    c.unused_inputs();
    c.out
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.split('/').all(|seg| {
            let mut chars = seg.chars();
            chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
                && !KEYWORDS.contains(&seg)
        })
}

fn terminates_block(f: &Function, stmts: &[Stmt]) -> bool {
    stmts.iter().any(|s| stmt_terminates(f, s))
}

fn stmt_terminates(f: &Function, s: &Stmt) -> bool {
    match s {
        Stmt::Return(_) => true,
        Stmt::If { then, otherwise, .. } => {
            otherwise.as_ref().is_some_and(|o| terminates_block(f, then) && terminates_block(f, o))
        }
        Stmt::Switch { subject, cases, default, .. } => {
            let default_ok = default.as_ref().is_some_and(|d| terminates_block(f, d));
            let covers = match f.input(&subject.value).map(|i| &i.ty) {
                Some(InputType::Choice(opts)) => {
                    opts.iter().all(|o| cases.iter().any(|c| c.value.value == o.value))
                }
                _ => false,
            };
            cases.iter().all(|c| terminates_block(f, &c.body)) && (default_ok || covers)
        }
    }
}

fn options_text(opts: &[Spanned<String>]) -> String {
    opts.iter().map(|o| format!("`{}`", o.value)).collect::<Vec<_>>().join(", ")
}

struct Checker<'a> {
    f: &'a Function,
    known: &'a dyn Fn(&str) -> bool,
    out: Vec<Diagnostic>,
    used: BTreeSet<String>,
}

impl<'a> Checker<'a> {
    fn error(&mut self, span: Span, msg: impl Into<String>) {
        self.out.push(Diagnostic::error(span, msg));
    }

    fn warning(&mut self, span: Span, msg: impl Into<String>) {
        self.out.push(Diagnostic::warning(span, msg));
    }

    fn unknown(&mut self, name: &Spanned<String>) {
        self.error(name.span, format!("unknown input `{}`", name.value));
    }

    fn description(&mut self) {
        let f = self.f;
        match &f.description {
            None => self.error(Span::default(), "missing `description`; it must be the first item"),
            Some(d) if d.value.trim().is_empty() => self.error(d.span, "empty description string"),
            Some(_) => {}
        }
    }

    fn inputs(&mut self) {
        let f = self.f;
        for (i, input) in f.inputs.iter().enumerate() {
            let name = &input.name.value;
            if f.inputs[..i].iter().any(|p| p.name.value == *name) {
                self.error(input.name.span, format!("duplicate input `{name}`"));
            }
            if let InputType::Choice(opts) = &input.ty {
                if opts.len() < 2 {
                    self.error(input.span, format!("choice input `{name}` needs at least 2 options"));
                }
                for (j, o) in opts.iter().enumerate() {
                    if o.value.trim().is_empty() {
                        self.error(o.span, format!("empty option in choice input `{name}`"));
                    } else if opts[..j].iter().any(|p| p.value == o.value) {
                        self.error(o.span, format!("duplicate option `{}` in choice input `{name}`", o.value));
                    }
                }
            }
        }
    }

    fn unused_inputs(&mut self) {
        let f = self.f;
        for input in &f.inputs {
            if !self.used.contains(&input.name.value) {
                self.warning(input.name.span, format!("unused input `{}`", input.name.value));
            }
        }
    }

    fn block(&mut self, stmts: &[Stmt]) {
        if let Some(i) = stmts.iter().position(|s| stmt_terminates(self.f, s)) {
            if let Some(next) = stmts.get(i + 1) {
                self.warning(next.span(), "unreachable statement");
            }
        }
        for s in stmts {
            self.stmt(s);
        }
    }

    fn stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::If { cond, then, otherwise, .. } => {
                self.expr(cond);
                self.block(then);
                if let Some(o) = otherwise {
                    self.block(o);
                }
            }
            Stmt::Switch { subject, cases, default, .. } => {
                self.switch(subject, cases, default.as_deref());
            }
            Stmt::Return(r) => {
                if let ReturnValue::Ref(name) = &r.value {
                    if !(self.known)(name) {
                        self.error(r.span, format!("unknown function reference `@{name}`"));
                    }
                }
            }
        }
    }

    fn expr(&mut self, e: &Expr) {
        match e {
            Expr::Var(v) => {
                self.used.insert(v.value.clone());
                match self.f.input(&v.value) {
                    None => self.unknown(v),
                    Some(Input { ty: InputType::Choice(_), .. }) => {
                        let n = &v.value;
                        self.error(
                            v.span,
                            format!("choice input `{n}` needs a comparison, like `{n} == \"...\"`"),
                        );
                    }
                    Some(_) => {}
                }
            }
            Expr::Compare { var, value, .. } => {
                self.used.insert(var.value.clone());
                let n = var.value.as_str();
                match self.f.input(n) {
                    None => self.unknown(var),
                    Some(Input { ty: InputType::Bool, .. }) => self.error(
                        var.span,
                        format!("bool input `{n}` cannot be compared; use `if {n}` or `if not {n}`"),
                    ),
                    Some(Input { ty: InputType::Choice(opts), .. }) => {
                        self.check_option(n, value, opts);
                    }
                }
            }
            Expr::Not(inner, _) => self.expr(inner),
            Expr::And(a, b) | Expr::Or(a, b) => {
                self.expr(a);
                self.expr(b);
            }
        }
    }

    fn check_option(&mut self, input: &str, value: &Spanned<String>, opts: &[Spanned<String>]) {
        if !opts.iter().any(|o| o.value == value.value) {
            let list = options_text(opts);
            self.error(
                value.span,
                format!("`{}` is not an option of `{input}`; options are {list}", value.value),
            );
        }
    }

    fn switch(&mut self, subject: &Spanned<String>, cases: &[Case], default: Option<&[Stmt]>) {
        self.used.insert(subject.value.clone());
        let n = subject.value.as_str();
        match self.f.input(n) {
            None => self.unknown(subject),
            Some(Input { ty: InputType::Bool, .. }) => self.error(
                subject.span,
                format!("bool input `{n}` cannot be used in a switch; use `if {n}` or `if not {n}`"),
            ),
            Some(Input { ty: InputType::Choice(opts), .. }) => {
                let mut seen: Vec<&str> = Vec::new();
                for c in cases {
                    self.check_option(n, &c.value, opts);
                    if seen.contains(&c.value.value.as_str()) {
                        self.error(c.value.span, format!("duplicate case `{}`", c.value.value));
                    }
                    seen.push(&c.value.value);
                }
            }
        }
        for c in cases {
            self.block(&c.body);
        }
        if let Some(d) = default {
            self.block(d);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;
    use crate::parser::tests::SAMPLE;
    use crate::diagnostic::Severity;

    fn check_with(src: &str, known: &dyn Fn(&str) -> bool) -> Vec<Diagnostic> {
        let (f, parse_diags) = parse(src);
        assert!(parse_diags.is_empty(), "{parse_diags:?}");
        check_function(&f, "test", known)
    }

    fn check_src(src: &str) -> Vec<Diagnostic> {
        check_with(src, &|_| true)
    }

    fn expect_diag(src: &str, severity: Severity, part: &str) {
        let diags = check_src(src);
        assert!(
            diags.iter().any(|d| d.severity == severity && d.message.contains(part)),
            "wanted {severity:?} containing `{part}`, got {diags:?}"
        );
    }

    #[test]
    fn clean_sample_has_no_diagnostics() {
        assert!(check_src(SAMPLE).is_empty(), "{:?}", check_src(SAMPLE));
    }

    #[test]
    fn missing_description() {
        expect_diag("return \"x\"", Severity::Error, "missing `description`");
    }

    #[test]
    fn empty_description() {
        expect_diag("description \"  \"\nreturn \"x\"", Severity::Error, "empty description string");
    }

    #[test]
    fn duplicate_input() {
        expect_diag(
            "description \"d\"\ninput a: bool \"a\"\ninput a: bool \"again\"\nif a { return \"1\" }\nreturn \"2\"",
            Severity::Error,
            "duplicate input `a`",
        );
    }

    #[test]
    fn choice_with_too_few_options() {
        expect_diag(
            "description \"d\"\ninput c: choice(\"only\") \"c\"\nif c == \"only\" { return \"1\" }\nreturn \"2\"",
            Severity::Error,
            "needs at least 2 options",
        );
    }

    #[test]
    fn duplicate_option() {
        expect_diag(
            "description \"d\"\ninput c: choice(\"a\", \"a\") \"c\"\nif c == \"a\" { return \"1\" }\nreturn \"2\"",
            Severity::Error,
            "duplicate option `a`",
        );
    }

    #[test]
    fn empty_option() {
        expect_diag(
            "description \"d\"\ninput c: choice(\"a\", \"\") \"c\"\nif c == \"a\" { return \"1\" }\nreturn \"2\"",
            Severity::Error,
            "empty option",
        );
    }

    #[test]
    fn unknown_input_in_condition() {
        expect_diag("description \"d\"\nif nope { return \"a\" }\nreturn \"b\"", Severity::Error, "unknown input `nope`");
    }

    #[test]
    fn unknown_input_in_switch_subject() {
        expect_diag("description \"d\"\nswitch nope { default { return \"a\" } }", Severity::Error, "unknown input `nope`");
    }

    #[test]
    fn bool_input_compared() {
        expect_diag(
            "description \"d\"\ninput a: bool \"a\"\nif a == \"x\" { return \"1\" }\nreturn \"2\"",
            Severity::Error,
            "bool input `a` cannot be compared; use `if a` or `if not a`",
        );
    }

    #[test]
    fn bool_input_as_switch_subject() {
        expect_diag(
            "description \"d\"\ninput a: bool \"a\"\nswitch a { default { return \"1\" } }",
            Severity::Error,
            "bool input `a` cannot be used in a switch",
        );
    }

    #[test]
    fn choice_input_used_bare() {
        expect_diag(
            "description \"d\"\ninput c: choice(\"x\", \"y\") \"c\"\nif c { return \"1\" }\nreturn \"2\"",
            Severity::Error,
            "choice input `c` needs a comparison, like `c == \"...\"`",
        );
    }

    #[test]
    fn compared_value_not_an_option() {
        expect_diag(
            "description \"d\"\ninput c: choice(\"x\", \"y\") \"c\"\nif c == \"z\" { return \"1\" }\nreturn \"2\"",
            Severity::Error,
            "`z` is not an option of `c`; options are `x`, `y`",
        );
    }

    #[test]
    fn case_value_not_an_option() {
        expect_diag(
            "description \"d\"\ninput c: choice(\"x\", \"y\") \"c\"\nswitch c { case \"z\" { return \"1\" } default { return \"2\" } }",
            Severity::Error,
            "`z` is not an option of `c`",
        );
    }

    #[test]
    fn duplicate_case_value() {
        expect_diag(
            "description \"d\"\ninput c: choice(\"x\", \"y\") \"c\"\nswitch c { case \"x\" { return \"1\" } case \"x\" { return \"2\" } default { return \"3\" } }",
            Severity::Error,
            "duplicate case `x`",
        );
    }

    #[test]
    fn not_all_paths_return() {
        expect_diag(
            "description \"d\"\ninput a: bool \"a\"\nif a { return \"1\" }",
            Severity::Error,
            "not all paths return",
        );
    }

    #[test]
    fn if_with_else_terminates_when_both_branches_return() {
        let diags = check_src(
            "description \"d\"\ninput a: bool \"a\"\nif a { return \"1\" } else { return \"2\" }",
        );
        assert!(diags.is_empty(), "{diags:?}");
    }

    #[test]
    fn if_with_non_returning_else_does_not_terminate() {
        expect_diag(
            "description \"d\"\ninput a: bool \"a\"\nif a { return \"1\" } else { if a { return \"2\" } }",
            Severity::Error,
            "not all paths return",
        );
    }

    #[test]
    fn switch_covering_all_options_terminates_without_default() {
        let diags = check_src(
            "description \"d\"\ninput c: choice(\"x\", \"y\") \"c\"\nswitch c { case \"x\" { return \"1\" } case \"y\" { return \"2\" } }",
        );
        assert!(diags.is_empty(), "{diags:?}");
    }

    #[test]
    fn switch_partial_coverage_without_default_does_not_terminate() {
        expect_diag(
            "description \"d\"\ninput c: choice(\"x\", \"y\") \"c\"\nswitch c { case \"x\" { return \"1\" } }",
            Severity::Error,
            "not all paths return",
        );
    }

    #[test]
    fn unknown_function_reference() {
        let diags = check_with("description \"d\"\nreturn @nope", &|_| false);
        assert!(
            diags.iter().any(|d| d.severity == Severity::Error && d.message.contains("unknown function reference `@nope`")),
            "{diags:?}"
        );
    }

    #[test]
    fn invalid_file_name() {
        let (f, _) = parse("description \"d\"\nreturn \"x\"");
        let diags = check_function(&f, "bad-name", &|_| true);
        assert!(diags.iter().any(|d| d.severity == Severity::Error && d.message.contains("invalid function file name")), "{diags:?}");
    }

    #[test]
    fn keyword_in_file_name() {
        let (f, _) = parse("description \"d\"\nreturn \"x\"");
        for name in ["if", "ci/return"] {
            let diags = check_function(&f, name, &|_| true);
            assert!(
                diags.iter().any(|d| d.severity == Severity::Error && d.message.contains("invalid function file name")),
                "{name}: {diags:?}"
            );
        }
    }

    #[test]
    fn unused_input() {
        expect_diag(
            "description \"d\"\ninput a: bool \"a\"\nreturn \"x\"",
            Severity::Warning,
            "unused input `a`",
        );
    }

    #[test]
    fn unreachable_statement() {
        expect_diag(
            "description \"d\"\nreturn \"a\"\nreturn \"b\"",
            Severity::Warning,
            "unreachable statement",
        );
    }
}
