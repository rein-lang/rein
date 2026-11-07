//! Single-file checks: names, doc blocks, parameters, conditions, switches, returns, references and templates.

use std::collections::BTreeSet;

use crate::ast::{Binding, Callee, Call, Case, DocBlock, Expr, Function, Input, InputType, Piece, ReturnValue, Spanned, Stmt, TemplateLit, Value};
use crate::diagnostic::{Diagnostic, Span};
use crate::lexer::{KEYWORDS, RESERVED};
use crate::stdlib;

/// What a reference or a std name is, as the checker needs it. A template also needs its parameter count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Function,
    Template { params: usize },
    Request,
}

impl From<&Binding<'_>> for Kind {
    fn from(b: &Binding<'_>) -> Self {
        match b {
            Binding::Function(_) => Kind::Function,
            Binding::Template { function, .. } => Kind::Template { params: function.inputs.len() },
            Binding::Request => Kind::Request,
        }
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
    if !valid_name(name) {
        c.error(
            Span::default(),
            format!(
                "invalid function file name `{name}`; each `/` part must match [A-Za-z_][A-Za-z0-9_]* and must not be a keyword"
            ),
        );
    }
    c.doc();
    c.params();
    if f.is_template() {
        c.template();
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

fn members_text(members: &[Spanned<String>]) -> String {
    members.iter().map(|o| format!("`{}`", o.value)).collect::<Vec<_>>().join(", ")
}

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

    /// A value of `string`, a template literal, or a template call.
    fn value(&mut self, v: &Value) {
        match v {
            Value::Str(_) => {}
            Value::Template(lit) => self.template_names(lit),
            Value::Call(call) => self.call(call),
        }
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

    /// A template file: one `return` of a template literal, whose names are its own `string` parameters.
    fn template(&mut self) {
        let f = self.f;
        for input in &f.inputs {
            if input.ty != InputType::Text {
                let name = &input.name.value;
                self.error(
                    input.name.span,
                    format!("a template takes only `string` parameters; `{name}` is `{}`", input.ty.ts_type()),
                );
            }
        }
        let lit = match f.body.as_slice() {
            [Stmt::Return(Spanned { value: ReturnValue::Template(lit), .. })] => lit,
            _ => {
                let span = f.body.first().map(Stmt::span).or(f.doc.as_ref().map(|d| d.span)).unwrap_or_default();
                self.error(span, "a template is one `return` of a template literal, like return `...${name}...`;");
                return;
            }
        };
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
                if matches!(response.ty, InputType::Text) {
                    self.error(response.name.span, "a request answer is `boolean` or a union of strings");
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
                ReturnValue::Call(call) => self.call(call),
            },
        }
    }

    /// A `return` of a reference. It must name a function, not a template.
    fn returned_ref(&mut self, reference: &Spanned<String>) {
        let path = reference.value.as_str();
        match (self.resolve)(path) {
            Err(msg) => self.error(reference.span, msg),
            Ok(Kind::Template { .. }) => self.error(
                reference.span,
                format!("`@{path}` is a template; call it with arguments, like `@{path}(...)`"),
            ),
            Ok(Kind::Function | Kind::Request) => {}
        }
    }

    /// A call of a template: a std name, or a reference. The arguments must match its parameters.
    fn call(&mut self, call: &Call) {
        match &call.callee {
            Callee::Ref(path) => match (self.resolve)(&path.value) {
                Err(msg) => self.error(path.span, msg),
                Ok(Kind::Function) => {
                    self.error(path.span, "only templates can be called; return the function to hand off");
                }
                Ok(kind) => self.arity(call, kind),
            },
            Callee::Std(name) => match stdlib::item(&name.value) {
                Err(msg) => self.error(name.span, msg),
                Ok(Binding::Request) => {
                    self.error(name.span, "`request` is not a template; use `await request(...)`");
                }
                Ok(binding) => self.arity(call, Kind::from(&binding)),
            },
        }
        for arg in &call.args {
            self.value(arg);
        }
    }

    fn arity(&mut self, call: &Call, kind: Kind) {
        if let Kind::Template { params } = kind {
            let got = call.args.len();
            if params != got {
                let name = match call.callee {
                    Callee::Ref(_) => format!("@{}", call.callee.key()),
                    Callee::Std(_) => call.callee.key().to_string(),
                };
                self.error(call.span, format!("`{name}` takes {params} arguments, got {got}"));
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
                    Some(InputType::Text) => {}
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
            Some(InputType::Text) => self.error(
                subject.span,
                format!("string parameter `{n}` cannot be used in a switch; only union parameters can"),
            ),
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
mod tests {
    use super::*;
    use crate::diagnostic::Severity;
    use crate::parser::parse;

    const TPL: &str = r#"/** Pick. */
export default function tpl(
  /** First. */ a: string,
  /** Second. */ b: string,
) {
  return `${a} ${b}`;
}"#;

    fn tpl() -> &'static Function {
        static F: std::sync::OnceLock<Function> = std::sync::OnceLock::new();
        F.get_or_init(|| parse(TPL).0)
    }

    /// Test resolver: `./tpl` is a template, `std` names come from std, paths with `missing` do not resolve,
    /// and every other path is a function.
    fn resolves(path: &str) -> Result<Kind, String> {
        if path == "./tpl" {
            return Ok(Kind::Template { params: tpl().inputs.len() });
        }
        if path.contains("missing") {
            return Err(format!("cannot find function `@{path}`"));
        }
        Ok(Kind::Function)
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
            "/** d */\nexport default function f(a: boolean) { if (a) { return \"1\" } return \"2\"  }",
            Severity::Error,
            "parameter `a` needs a doc comment: /** question */ a: ...",
        );
    }

    #[test]
    fn empty_param_doc_comment_counts_as_missing() {
        expect_diag(
            "/** d */\nexport default function f(\n  /**  */\n  a: boolean,\n) { if (a) { return \"1\" } return \"2\"  }",
            Severity::Error,
            "needs a doc comment",
        );
    }

    #[test]
    fn param_tag_anywhere_is_an_error() {
        expect_diag(
            "/** d\n * @param a A?\n */\nexport default function f(\n  /** A? */\n  a: boolean,\n) { if (a) { return \"1\" } return \"2\"  }",
            Severity::Error,
            "describe each parameter in its own doc comment, right before it",
        );
        expect_diag(
            "/** d */\nexport default function f(\n  /** a\n   * @param a A\n   */\n  a: boolean,\n) { if (a) { return \"1\" } return \"2\"  }",
            Severity::Error,
            "describe each parameter in its own doc comment, right before it",
        );
    }

    #[test]
    fn multi_line_param_doc_is_clean() {
        clean(
            "/** d */\nexport default function f(\n  /**\n   * Is the customer a VIP?\n   * Check the CRM.\n   */\n  vip: boolean,\n) { if (vip) { return \"1\" } return \"2\"  }",
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
            "/** d */\nexport default function f(\n  /** A */ a: boolean,\n  /** A */ a: boolean,\n) { if (a) { return \"1\" } return \"2\"  }",
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
            "/** d */\nexport default function f(\n  /** C */ c: \"only\",\n) { if (c === \"only\") { return \"1\" } return \"2\"  }",
            Severity::Error,
            "needs at least 2 members",
        );
    }

    #[test]
    fn duplicate_member() {
        expect_diag(
            "/** d */\nexport default function f(\n  /** C */ c: \"a\" | \"a\",\n) { if (c === \"a\") { return \"1\" } return \"2\"  }",
            Severity::Error,
            "duplicate member `a` in union parameter `c`",
        );
    }

    #[test]
    fn empty_member() {
        expect_diag(
            "/** d */\nexport default function f(\n  /** C */ c: \"a\" | \"\",\n) { if (c === \"a\") { return \"1\" } return \"2\"  }",
            Severity::Error,
            "empty member",
        );
    }

    #[test]
    fn unknown_parameter_in_condition() {
        expect_diag("/** d */\nexport default function f() { if (nope) { return \"a\" } return \"b\"  }", Severity::Error, "unknown parameter `nope`");
    }

    #[test]
    fn unknown_parameter_in_switch_subject() {
        expect_diag(
            "/** d */\nexport default function f() { switch (nope) { case \"a\": return \"a\" } }",
            Severity::Error,
            "unknown parameter `nope`",
        );
    }

    #[test]
    fn boolean_parameter_compared() {
        expect_diag(
            "/** d */\nexport default function f(\n  /** A */ a: boolean,\n) { if (a === \"x\") { return \"1\" } return \"2\"  }",
            Severity::Error,
            "boolean parameter `a` cannot be compared; use `if (a)` or `if (!a)`",
        );
    }

    #[test]
    fn boolean_parameter_as_switch_subject() {
        expect_diag(
            "/** d */\nexport default function f(\n  /** A */ a: boolean,\n) { switch (a) { case \"x\": return \"1\" } }",
            Severity::Error,
            "boolean parameter `a` cannot be used in a switch",
        );
    }

    #[test]
    fn union_parameter_used_bare() {
        expect_diag(
            "/** d */\nexport default function f(\n  /** C */ c: \"x\" | \"y\",\n) { if (c) { return \"1\" } return \"2\"  }",
            Severity::Error,
            "union parameter `c` needs a comparison, like `c === \"...\"`",
        );
    }

    #[test]
    fn compared_value_not_a_member() {
        expect_diag(
            "/** d */\nexport default function f(\n  /** C */ c: \"x\" | \"y\",\n) { if (c === \"z\") { return \"1\" } return \"2\"  }",
            Severity::Error,
            "`z` is not a member of `c`; members are `x`, `y`",
        );
    }

    #[test]
    fn case_value_not_a_member() {
        expect_diag(
            "/** d */\nexport default function f(\n  /** C */ c: \"x\" | \"y\",\n) { switch (c) { case \"z\": return \"1\"; case \"x\": return \"2\" } }",
            Severity::Error,
            "`z` is not a member of `c`",
        );
    }

    #[test]
    fn duplicate_case_value_across_groups() {
        expect_diag(
            "/** d */\nexport default function f(\n  /** C */ c: \"x\" | \"y\",\n) { switch (c) { case \"x\": return \"1\"; case \"x\": return \"2\"; case \"y\": return \"3\" } }",
            Severity::Error,
            "duplicate case `x`",
        );
    }

    #[test]
    fn case_with_statements_must_end_in_return() {
        expect_diag(
            "/** d */\nexport default function f(\n  /** C */ c: \"x\" | \"y\",\n) { switch (c) { case \"x\": if (c === \"x\") { return \"1\" } case \"y\": return \"2\" } }",
            Severity::Error,
            "case falls through; end it with return",
        );
    }

    #[test]
    fn grouped_labels_without_return_fall_through() {
        expect_diag(
            "/** d */\nexport default function f(\n  /** C */ c: \"x\" | \"y\" | \"z\",\n) { switch (c) { case \"x\": return \"1\"; case \"y\": case \"z\": } }",
            Severity::Error,
            "case falls through; end it with return",
        );
    }

    #[test]
    fn grouped_cases_share_one_return() {
        clean(
            "/** d */\nexport default function f(\n  /** C */ c: \"x\" | \"y\" | \"z\",\n) { switch (c) { case \"x\": case \"y\": return \"1\"; case \"z\": return \"2\" } }",
        );
    }

    #[test]
    fn not_all_paths_return() {
        expect_diag(
            "/** d */\nexport default function f(\n  /** A */ a: boolean,\n) { if (a) { return \"1\" } }",
            Severity::Error,
            "every path must return something",
        );
    }

    #[test]
    fn if_with_else_terminates_when_both_branches_return() {
        clean("/** d */\nexport default function f(\n  /** A */ a: boolean,\n) { if (a) { return \"1\" } return \"2\"  }");
    }

    #[test]
    fn if_block_that_falls_through_is_an_error() {
        expect_diag(
            "/** d */\nexport default function f(\n  /** A */ a: boolean,\n) { if (a) { if (a) { return \"2\" } } return \"1\" }",
            Severity::Error,
            "every `if` block must end in a return",
        );
    }

    #[test]
    fn guard_chain_is_clean() {
        clean("/** d */\nexport default function f(\n  /** A */ a: boolean,\n) { if (a) { return \"1\" } return \"2\" }");
    }

    #[test]
    fn switch_covering_all_members_terminates() {
        clean(
            "/** d */\nexport default function f(\n  /** C */ c: \"x\" | \"y\",\n) { switch (c) { case \"x\": return \"1\"; case \"y\": return \"2\" } }",
        );
    }

    #[test]
    fn switch_missing_a_member_is_an_error() {
        expect_diag(
            "/** d */\nexport default function f(\n  /** C */ c: \"x\" | \"y\",\n) { switch (c) { case \"x\": return \"1\" } }",
            Severity::Error,
            "switch on `c` needs a case for every member; missing: `y`",
        );
    }

    #[test]
    fn switch_missing_several_members_lists_them_all() {
        expect_diag(
            "/** d */\nexport default function f(\n  /** C */ c: \"x\" | \"y\" | \"z\",\n) { switch (c) { case \"x\": return \"1\" } }",
            Severity::Error,
            "missing: `y`, `z`",
        );
    }

    #[test]
    fn code_after_return_in_a_guard_chain_is_an_error() {
        expect_diag(
            "/** d */\nexport default function f(\n  /** A */ a: boolean,\n) { if (a) { return \"1\" } return \"2\"  return \"3\" }",
            Severity::Error,
            "unreachable statement",
        );
    }

    #[test]
    fn code_after_switch_is_an_error() {
        expect_diag(
            "/** d */\nexport default function f(\n  /** C */ c: \"x\" | \"y\",\n) { switch (c) { case \"x\": return \"1\"; case \"y\": return \"2\" } return \"3\" }",
            Severity::Error,
            "unreachable statement",
        );
    }

    #[test]
    fn reference_that_does_not_resolve_is_an_error_on_the_reference() {
        expect_diag(
            "/** d */\nexport default function f() { return @./missing; }",
            Severity::Error,
            "cannot find function `@./missing`",
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
            Severity::Error,
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

    const TPL_IMPORT: &str = "";
    const STD_IMPORT: &str = "";

    fn with(prefix: &str, body: &str) -> String {
        format!("{prefix}{body}")
    }

    #[test]
    fn template_call_with_literal_and_union_args_is_clean() {
        clean(&with(TPL_IMPORT, r#"/** d */
export default function f(
  /** C */ c: "x" | "y",
) {
  if (c === "x") {
    return @./tpl("one", `x ${c}`);
  }
  return @./tpl("two", `y ${c}`);
}"#));
    }

    #[test]
    fn template_literal_return_is_clean() {
        clean(r#"/** d */
export default function f(
  /** C */ c: "x" | "y",
) {
  return `Pick ${c}`;
}"#);
    }

    #[test]
    fn template_call_arity_is_checked() {
        expect_diag(
            &with(TPL_IMPORT, r#"/** d */
export default function f() { return @./tpl("only one") }"#),
            Severity::Error,
            "`@./tpl` takes 2 arguments, got 1",
        );
    }

    #[test]
    fn only_templates_can_be_called() {
        expect_diag(
            &with("", r#"/** d */
export default function f() { return @./other("a") }"#),
            Severity::Error,
            "only templates can be called; return the function to hand off",
        );
    }

    #[test]
    fn calling_an_unimported_name_is_an_error() {
        expect_diag(
            r#"/** d */
export default function f() { return nope("a") }"#,
            Severity::Error,
            "`nope` is not in std",
        );
    }

    #[test]
    fn template_returned_without_a_call_is_an_error() {
        expect_diag(
            &with(TPL_IMPORT, r#"/** d */
export default function f() { return @./tpl }"#),
            Severity::Error,
            "`@./tpl` is a template; call it with arguments, like `@./tpl(...)`",
        );
    }

    #[test]
    fn boolean_parameter_cannot_be_interpolated() {
        expect_diag(
            r#"/** d */
export default function f(
  /** A */ a: boolean,
) { if (a) { return `x ${a}` } return "y"  }"#,
            Severity::Error,
            "boolean parameter `a` cannot be interpolated",
        );
    }

    #[test]
    fn unknown_name_cannot_be_interpolated() {
        expect_diag(
            r#"/** d */
export default function f(
  /** C */ c: "x" | "y",
) { return `x ${zz} ${c}` }"#,
            Severity::Error,
            "unknown parameter `zz`",
        );
    }

    #[test]
    fn boolean_cannot_be_a_call_argument() {
        expect_diag(
            &with(TPL_IMPORT, r#"/** d */
export default function f(
  /** A */ a: boolean,
) { if (a) { return @./tpl("x", `${a}`) } return "y"  }"#),
            Severity::Error,
            "boolean parameter `a` cannot be interpolated",
        );
    }

    #[test]
    fn template_file_with_string_params_is_clean() {
        clean(r#"/** Say hi. */
export default function hi(
  /** Who? */ who: string,
) {
  return `Hi ${who}`;
}"#);
    }

    #[test]
    fn template_body_must_be_one_template_return() {
        expect_diag(
            r#"/** d */
export default function t(
  /** A */ a: string,
) { return "x" }"#,
            Severity::Error,
            "a template is one `return` of a template literal",
        );
    }

    #[test]
    fn template_cannot_have_an_if() {
        expect_diag(
            r#"/** d */
export default function t(
  /** A */ a: string,
) { if (a) { return `${a}` } return `b`  }"#,
            Severity::Error,
            "a template is one `return` of a template literal",
        );
    }

    #[test]
    fn template_takes_only_string_parameters() {
        expect_diag(
            r#"/** d */
export default function t(
  /** A */ a: string,
  /** B */ b: "x" | "y",
) { return `${a}` }"#,
            Severity::Error,
            "a template takes only `string` parameters; `b` is",
        );
    }

    #[test]
    fn template_must_use_every_parameter() {
        expect_diag(
            r#"/** d */
export default function t(
  /** A */ a: string,
  /** B */ b: string,
) { return `${a}` }"#,
            Severity::Error,
            "template parameter `b` is not used",
        );
    }

    #[test]
    fn std_template_call_is_clean() {
        clean(&with(STD_IMPORT, r#"/** d */
export default function f(
  /** C */ c: "x" | "y",
) {
  if (c === "x") {
    return delegate("use opus model", `task ${c}`);
  } return delegate("use haiku model", "code review"); 
}"#));
    }



    #[test]
    fn await_needs_an_async_function() {
        expect_diag(
            r#"/** d */
export default function f() {
  /** Ok? */
  const ok: boolean = await request("x");
  if (ok) { return "y" }
  return "n"
}"#,
            Severity::Error,
            "`await` needs `export default async function`",
        );
    }

    #[test]
    fn async_without_await_is_a_warning() {
        expect_diag(
            r#"/** d */
export default async function f() { return "x" }"#,
            Severity::Warning,
            "`async` function has no `await`; drop `async`",
        );
    }

    #[test]
    fn answer_used_as_a_guard_is_clean() {
        clean(r#"/** d */
export default async function f(
  /** C */ c: "x" | "y",
) {
  /** Ok? */
  const ok: boolean = await request(`pick ${c}`);
  if (!ok) { return @./tpl("a", "b") }
  return "y"
}"#);
    }

    #[test]
    fn answer_clashes_with_a_parameter() {
        expect_diag(
            r#"/** d */
export default async function f(
  /** C */ c: "x" | "y",
) {
  /** Ok? */
  const c: boolean = await request("x");
  if (c) { return "y" }
  return "n"
}"#,
            Severity::Error,
            "duplicate name `c`",
        );
    }

    #[test]
    fn answer_used_before_it_is_declared() {
        expect_diag(
            r#"/** d */
export default async function f() {
  /** First */
  const first: "x" | "y" = await request(`${second}`);
  /** Second */
  const second: "x" | "y" = await request("q");
  return "b"
}"#,
            Severity::Error,
            "`second` is used before it is declared",
        );
    }

    #[test]
    fn answer_cannot_be_a_string() {
        expect_diag(
            r#"/** d */
export default async function f() {
  /** Ok? */
  const s: string = await request("x");
  return "y"
}"#,
            Severity::Error,
            "a request answer is `boolean` or a union of strings",
        );
    }

    #[test]
    fn union_answer_needs_every_case() {
        expect_diag(
            r#"/** d */
export default async function f() {
  /** Pick */
  const c: "x" | "y" = await request("pick");
  switch (c) {
    case "x":
      return "1";
  }
}"#,
            Severity::Error,
            "switch on `c` needs a case for every member; missing: `y`",
        );
    }

    #[test]
    fn empty_function_must_return_something() {
        expect_diag("/** d */\nexport default function f() {\n}", Severity::Error, "function must return something");
    }

    #[test]
    fn body_that_can_end_without_return_says_every_path() {
        expect_diag(
            "/** d */\nexport default function f(\n  /** A */ a: boolean,\n) { if (a) { return \"1\" } }",
            Severity::Error,
            "every path must return something",
        );
    }
}
