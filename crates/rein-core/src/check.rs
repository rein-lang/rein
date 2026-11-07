//! Single-file checks: names, doc blocks, imports, parameters, conditions, switches and returns.

use std::collections::BTreeSet;

use crate::ast::{Case, DocBlock, Expr, Function, Input, InputType, ReturnValue, Spanned, Stmt};
use crate::diagnostic::{Diagnostic, Span};
use crate::lexer::{KEYWORDS, RESERVED};

/// Resolves an import path (as written) to the project path of its function, or says why not.
pub type Resolver<'a> = &'a dyn Fn(&str) -> Result<String, String>;

/// Checks one function. `name` is its project path, used for the file-name rule.
/// `resolve` tells whether an import path names a function in the project.
pub fn check_function(f: &Function, name: &str, resolve: Resolver) -> Vec<Diagnostic> {
    let mut c = Checker { f, resolve, out: Vec::new(), used: BTreeSet::new(), returned: BTreeSet::new() };
    if !valid_name(name) {
        c.error(
            Span::default(),
            format!(
                "invalid function file name `{name}`; each `/` part must match [A-Za-z_][A-Za-z0-9_]* and must not be a keyword"
            ),
        );
    }
    c.doc();
    c.imports();
    c.params();
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
    c.unused_params();
    c.unused_imports();
    c.out
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
        Stmt::Return(_) => true,
        Stmt::If { then, otherwise, .. } => {
            otherwise.as_ref().is_some_and(|o| terminates_block(f, then) && terminates_block(f, o))
        }
        Stmt::Switch { subject, cases, default, .. } => {
            let default_ok = default.as_ref().is_some_and(|d| terminates_block(f, d));
            let covers = match f.input(&subject.value).map(|i| &i.ty) {
                Some(InputType::Choice(members)) => members.iter().all(|m| {
                    cases.iter().any(|c| c.values.iter().any(|v| v.value == m.value))
                }),
                _ => false,
            };
            cases.iter().all(|c| terminates_block(f, &c.body)) && (default_ok || covers)
        }
    }
}

fn members_text(members: &[Spanned<String>]) -> String {
    members.iter().map(|o| format!("`{}`", o.value)).collect::<Vec<_>>().join(", ")
}

struct Checker<'a> {
    f: &'a Function,
    resolve: Resolver<'a>,
    out: Vec<Diagnostic>,
    /// Parameters used in a condition or a switch subject.
    used: BTreeSet<String>,
    /// Import bindings used in a `return`.
    returned: BTreeSet<String>,
}

impl<'a> Checker<'a> {
    fn error(&mut self, span: Span, msg: impl Into<String>) {
        self.out.push(Diagnostic::error(span, msg));
    }

    fn warning(&mut self, span: Span, msg: impl Into<String>) {
        self.out.push(Diagnostic::warning(span, msg));
    }

    fn unknown(&mut self, name: &Spanned<String>) {
        self.error(name.span, format!("unknown parameter `{}`", name.value));
    }

    /// Rules for the function's doc block: its presence, the description, and its tags.
    fn doc(&mut self) {
        let f = self.f;
        let Some(doc) = &f.doc else {
            self.error(Span::default(), "missing doc comment; put /** description */ right before `export default function`");
            return;
        };
        if f.description.as_ref().is_none_or(|d| d.value.trim().is_empty()) {
            self.error(doc.span, "empty description; write the function description in the doc comment");
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

    /// Import paths resolve, and each binding is unique.
    fn imports(&mut self) {
        let f = self.f;
        for (i, imp) in f.imports.iter().enumerate() {
            let name = &imp.binding.value;
            let clash_import = f.imports[..i].iter().any(|p| p.binding.value == *name);
            let clash_param = f.inputs.iter().any(|p| p.name.value == *name);
            if clash_import || clash_param {
                self.error(imp.binding.span, format!("duplicate name `{name}`"));
            }
            if let Err(msg) = (self.resolve)(&imp.source.value) {
                self.error(imp.source.span, msg);
            }
        }
    }

    /// Each parameter has a doc comment, and names are unique.
    fn params(&mut self) {
        let f = self.f;
        for (i, input) in f.inputs.iter().enumerate() {
            let name = &input.name.value;
            if f.inputs[..i].iter().any(|p| p.name.value == *name) {
                self.error(input.name.span, format!("duplicate parameter `{name}`"));
            }
            if let Some(doc) = &input.doc {
                self.tags(doc);
            }
            if input.description.value.trim().is_empty() {
                self.error(
                    input.name.span,
                    format!("parameter `{name}` needs a doc comment: /** question */ {name}: ..."),
                );
            }
            if let InputType::Choice(members) = &input.ty {
                if members.len() < 2 {
                    self.error(input.span, format!("union parameter `{name}` needs at least 2 members"));
                }
                for (j, m) in members.iter().enumerate() {
                    if m.value.trim().is_empty() {
                        self.error(m.span, format!("empty member in union parameter `{name}`"));
                    } else if members[..j].iter().any(|p| p.value == m.value) {
                        self.error(m.span, format!("duplicate member `{}` in union parameter `{name}`", m.value));
                    }
                }
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

    fn unused_imports(&mut self) {
        let f = self.f;
        for imp in &f.imports {
            if !self.returned.contains(&imp.binding.value) {
                self.warning(imp.binding.span, format!("unused import `{}`", imp.binding.value));
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
            Stmt::Switch { subject, cases, default, span } => {
                self.switch(subject, cases, default.as_deref(), *span);
            }
            Stmt::Return(r) => {
                if let ReturnValue::Ref(name) = &r.value {
                    self.returned.insert(name.clone());
                    if self.f.import(name).is_none() {
                        self.error(r.span, format!("`{name}` is not an imported function"));
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
                            format!("union parameter `{n}` needs a comparison, like `{n} === \"...\"`"),
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
                        format!("boolean parameter `{n}` cannot be compared; use `if ({n})` or `if (!{n})`"),
                    ),
                    Some(Input { ty: InputType::Choice(members), .. }) => {
                        self.check_member(n, value, members);
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

    fn check_member(&mut self, param: &str, value: &Spanned<String>, members: &[Spanned<String>]) {
        if !members.iter().any(|m| m.value == value.value) {
            let list = members_text(members);
            self.error(
                value.span,
                format!("`{}` is not a member of `{param}`; members are {list}", value.value),
            );
        }
    }

    fn switch(&mut self, subject: &Spanned<String>, cases: &[Case], default: Option<&[Stmt]>, span: Span) {
        self.used.insert(subject.value.clone());
        let n = subject.value.as_str();
        match self.f.input(n) {
            None => self.unknown(subject),
            Some(Input { ty: InputType::Bool, .. }) => self.error(
                subject.span,
                format!("boolean parameter `{n}` cannot be used in a switch; use `if ({n})` or `if (!{n})`"),
            ),
            Some(Input { ty: InputType::Choice(members), .. }) => {
                let mut seen: Vec<&str> = Vec::new();
                for c in cases {
                    for v in &c.values {
                        self.check_member(n, v, members);
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
                let at = c.values.first().map_or(span, |v| v.span);
                self.error(at, "case falls through; end it with return");
            }
        }
        if let Some(d) = default {
            self.block(d);
            if !terminates_block(self.f, d) {
                self.error(span, "default falls through; end it with return");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostic::Severity;
    use crate::parser::parse;

    /// Test resolver: every import path resolves, except those containing `missing`.
    fn resolves(src: &str) -> Result<String, String> {
        if src.contains("missing") {
            Err(format!("cannot find module '{src}'"))
        } else {
            Ok("x".into())
        }
    }

    fn check_src(src: &str) -> Vec<Diagnostic> {
        let (f, parse_diags) = parse(src);
        assert!(parse_diags.is_empty(), "{parse_diags:?}");
        check_function(&f, "test", &resolves)
    }

    fn expect_diag(src: &str, severity: Severity, part: &str) {
        let diags = check_src(src);
        assert!(
            diags.iter().any(|d| d.severity == severity && d.message.contains(part)),
            "wanted {severity:?} containing `{part}`, got {diags:?}"
        );
    }

    fn clean(src: &str) {
        let diags = check_src(src);
        assert!(diags.is_empty(), "{diags:?}");
    }

    const IMPORT: &str = "import a from \"./a\";\n";

    #[test]
    fn clean_sample_has_no_diagnostics() {
        let sample = crate::parser::tests::SAMPLE;
        assert!(check_src(sample).is_empty(), "{:?}", check_src(sample));
    }

    #[test]
    fn missing_doc_comment() {
        expect_diag(
            "export default function f() { return \"x\" }",
            Severity::Error,
            "missing doc comment; put /** description */ right before `export default function`",
        );
    }

    #[test]
    fn empty_description() {
        expect_diag("/**/\nexport default function f() { return \"x\" }", Severity::Error, "empty description");
        expect_diag("/**   */\nexport default function f() { return \"x\" }", Severity::Error, "empty description");
    }

    #[test]
    fn param_without_doc_comment() {
        expect_diag(
            "/** d */\nexport default function f(a: boolean) { if (a) { return \"1\" } return \"2\" }",
            Severity::Error,
            "parameter `a` needs a doc comment: /** question */ a: ...",
        );
    }

    #[test]
    fn empty_param_doc_comment_counts_as_missing() {
        expect_diag(
            "/** d */\nexport default function f(\n  /**  */\n  a: boolean,\n) { if (a) { return \"1\" } return \"2\" }",
            Severity::Error,
            "needs a doc comment",
        );
    }

    #[test]
    fn param_tag_anywhere_is_an_error() {
        expect_diag(
            "/** d\n * @param a A?\n */\nexport default function f(\n  /** A? */\n  a: boolean,\n) { if (a) { return \"1\" } return \"2\" }",
            Severity::Error,
            "describe each parameter in its own doc comment, right before it",
        );
        expect_diag(
            "/** d */\nexport default function f(\n  /** a\n   * @param a A\n   */\n  a: boolean,\n) { if (a) { return \"1\" } return \"2\" }",
            Severity::Error,
            "describe each parameter in its own doc comment, right before it",
        );
    }

    #[test]
    fn multi_line_param_doc_is_clean() {
        clean(
            "/** d */\nexport default function f(\n  /**\n   * Is the customer a VIP?\n   * Check the CRM.\n   */\n  vip: boolean,\n) { if (vip) { return \"1\" } return \"2\" }",
        );
    }

    #[test]
    fn unknown_tag_is_a_warning() {
        expect_diag(
            "/** d\n * @returns Something.\n */\nexport default function f() { return \"x\" }",
            Severity::Warning,
            "unknown tag `@returns`",
        );
    }

    #[test]
    fn duplicate_parameter() {
        expect_diag(
            "/** d */\nexport default function f(\n  /** A */ a: boolean,\n  /** A */ a: boolean,\n) { if (a) { return \"1\" } return \"2\" }",
            Severity::Error,
            "duplicate parameter `a`",
        );
    }

    #[test]
    fn param_without_name_is_a_syntax_error() {
        let (_, diags) = parse("/** d */\nexport default function f(: boolean) { return \"x\" }");
        assert_eq!(diags.len(), 1, "{diags:?}");
    }

    #[test]
    fn union_with_too_few_members() {
        expect_diag(
            "/** d */\nexport default function f(\n  /** C */ c: \"only\",\n) { if (c === \"only\") { return \"1\" } return \"2\" }",
            Severity::Error,
            "needs at least 2 members",
        );
    }

    #[test]
    fn duplicate_member() {
        expect_diag(
            "/** d */\nexport default function f(\n  /** C */ c: \"a\" | \"a\",\n) { if (c === \"a\") { return \"1\" } return \"2\" }",
            Severity::Error,
            "duplicate member `a` in union parameter `c`",
        );
    }

    #[test]
    fn empty_member() {
        expect_diag(
            "/** d */\nexport default function f(\n  /** C */ c: \"a\" | \"\",\n) { if (c === \"a\") { return \"1\" } return \"2\" }",
            Severity::Error,
            "empty member",
        );
    }

    #[test]
    fn unknown_parameter_in_condition() {
        expect_diag("/** d */\nexport default function f() { if (nope) { return \"a\" } return \"b\" }", Severity::Error, "unknown parameter `nope`");
    }

    #[test]
    fn unknown_parameter_in_switch_subject() {
        expect_diag(
            "/** d */\nexport default function f() { switch (nope) { default: return \"a\" } }",
            Severity::Error,
            "unknown parameter `nope`",
        );
    }

    #[test]
    fn boolean_parameter_compared() {
        expect_diag(
            "/** d */\nexport default function f(\n  /** A */ a: boolean,\n) { if (a === \"x\") { return \"1\" } return \"2\" }",
            Severity::Error,
            "boolean parameter `a` cannot be compared; use `if (a)` or `if (!a)`",
        );
    }

    #[test]
    fn boolean_parameter_as_switch_subject() {
        expect_diag(
            "/** d */\nexport default function f(\n  /** A */ a: boolean,\n) { switch (a) { default: return \"1\" } }",
            Severity::Error,
            "boolean parameter `a` cannot be used in a switch",
        );
    }

    #[test]
    fn union_parameter_used_bare() {
        expect_diag(
            "/** d */\nexport default function f(\n  /** C */ c: \"x\" | \"y\",\n) { if (c) { return \"1\" } return \"2\" }",
            Severity::Error,
            "union parameter `c` needs a comparison, like `c === \"...\"`",
        );
    }

    #[test]
    fn compared_value_not_a_member() {
        expect_diag(
            "/** d */\nexport default function f(\n  /** C */ c: \"x\" | \"y\",\n) { if (c === \"z\") { return \"1\" } return \"2\" }",
            Severity::Error,
            "`z` is not a member of `c`; members are `x`, `y`",
        );
    }

    #[test]
    fn case_value_not_a_member() {
        expect_diag(
            "/** d */\nexport default function f(\n  /** C */ c: \"x\" | \"y\",\n) { switch (c) { case \"z\": return \"1\"; default: return \"2\" } }",
            Severity::Error,
            "`z` is not a member of `c`",
        );
    }

    #[test]
    fn duplicate_case_value_across_groups() {
        expect_diag(
            "/** d */\nexport default function f(\n  /** C */ c: \"x\" | \"y\",\n) { switch (c) { case \"x\": return \"1\"; case \"x\": return \"2\"; default: return \"3\" } }",
            Severity::Error,
            "duplicate case `x`",
        );
    }

    #[test]
    fn case_with_statements_must_end_in_return() {
        expect_diag(
            "/** d */\nexport default function f(\n  /** C */ c: \"x\" | \"y\",\n) { switch (c) { case \"x\": if (c === \"x\") { return \"1\" } default: return \"2\" } }",
            Severity::Error,
            "case falls through; end it with return",
        );
    }

    #[test]
    fn grouped_labels_without_return_fall_through() {
        expect_diag(
            "/** d */\nexport default function f(\n  /** C */ c: \"x\" | \"y\" | \"z\",\n) { switch (c) { case \"x\": case \"y\": } return \"2\" }",
            Severity::Error,
            "case falls through; end it with return",
        );
    }

    #[test]
    fn default_with_no_return_falls_through() {
        expect_diag(
            "/** d */\nexport default function f(\n  /** C */ c: \"x\" | \"y\",\n) { switch (c) { case \"x\": return \"1\"; default: { } } return \"2\" }",
            Severity::Error,
            "default falls through; end it with return",
        );
    }

    #[test]
    fn grouped_cases_share_one_return() {
        clean(
            "/** d */\nexport default function f(\n  /** C */ c: \"x\" | \"y\" | \"z\",\n) { switch (c) { case \"x\": case \"y\": return \"1\"; default: return \"2\" } }",
        );
    }

    #[test]
    fn not_all_paths_return() {
        expect_diag(
            "/** d */\nexport default function f(\n  /** A */ a: boolean,\n) { if (a) { return \"1\" } }",
            Severity::Error,
            "not all paths return",
        );
    }

    #[test]
    fn if_with_else_terminates_when_both_branches_return() {
        clean("/** d */\nexport default function f(\n  /** A */ a: boolean,\n) { if (a) { return \"1\" } else { return \"2\" } }");
    }

    #[test]
    fn if_with_non_returning_else_does_not_terminate() {
        expect_diag(
            "/** d */\nexport default function f(\n  /** A */ a: boolean,\n) { if (a) { return \"1\" } else { if (a) { return \"2\" } } }",
            Severity::Error,
            "not all paths return",
        );
    }

    #[test]
    fn switch_covering_all_members_terminates_without_default() {
        clean(
            "/** d */\nexport default function f(\n  /** C */ c: \"x\" | \"y\",\n) { switch (c) { case \"x\": return \"1\"; case \"y\": return \"2\" } }",
        );
    }

    #[test]
    fn switch_partial_coverage_without_default_does_not_terminate() {
        expect_diag(
            "/** d */\nexport default function f(\n  /** C */ c: \"x\" | \"y\",\n) { switch (c) { case \"x\": return \"1\" } }",
            Severity::Error,
            "not all paths return",
        );
    }

    #[test]
    fn import_that_does_not_resolve_is_an_error_on_the_path() {
        let src = "import a from \"./missing\";\n/** d */\nexport default function f() { return a; }";
        let (f, _) = parse(src);
        let diags = check_function(&f, "test", &resolves);
        let d = diags.iter().find(|d| d.message.contains("cannot find module './missing'")).expect("import error");
        assert_eq!(&src[d.span.start..d.span.end], "\"./missing\"");
    }

    #[test]
    fn returning_an_unimported_name_is_an_error() {
        expect_diag("/** d */\nexport default function f() { return nope; }", Severity::Error, "`nope` is not an imported function");
    }

    #[test]
    fn returning_a_parameter_is_an_error() {
        expect_diag(
            "/** d */\nexport default function f(\n  /** A */ a: boolean,\n) { if (a) { return a; } return \"x\" }",
            Severity::Error,
            "`a` is not an imported function",
        );
    }

    #[test]
    fn unused_import_is_a_warning() {
        expect_diag(
            &format!("{IMPORT}/** d */\nexport default function f() {{ return \"x\" }}"),
            Severity::Warning,
            "unused import `a`",
        );
    }

    #[test]
    fn import_binding_clashes() {
        expect_diag(
            &format!("{IMPORT}import a from \"./b\";\n/** d */\nexport default function f() {{ return a }}"),
            Severity::Error,
            "duplicate name `a`",
        );
        expect_diag(
            &format!("{IMPORT}/** d */\nexport default function f(\n  /** A */ a: boolean,\n) {{ if (a) {{ return a }} return \"x\" }}"),
            Severity::Error,
            "duplicate name `a`",
        );
    }

    #[test]
    fn unused_parameter() {
        expect_diag(
            "/** d */\nexport default function f(\n  /** A */ a: boolean,\n) { return \"x\" }",
            Severity::Warning,
            "unused parameter `a`",
        );
    }

    #[test]
    fn unreachable_statement() {
        expect_diag(
            "/** d */\nexport default function f() { return \"a\"; return \"b\" }",
            Severity::Warning,
            "unreachable statement",
        );
    }

    #[test]
    fn invalid_file_name() {
        let (f, _) = parse("/** d */\nexport default function f() { return \"x\" }");
        let diags = check_function(&f, "bad-name", &resolves);
        assert!(diags.iter().any(|d| d.severity == Severity::Error && d.message.contains("invalid function file name")), "{diags:?}");
    }

    #[test]
    fn keyword_or_reserved_word_in_file_name() {
        let (f, _) = parse("/** d */\nexport default function f() { return \"x\" }");
        for name in ["if", "ci/return", "for", "ci/const"] {
            let diags = check_function(&f, name, &resolves);
            assert!(
                diags.iter().any(|d| d.severity == Severity::Error && d.message.contains("invalid function file name")),
                "{name}: {diags:?}"
            );
        }
    }
}
