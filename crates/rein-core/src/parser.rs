//! Recursive descent parser: tokens to a `Function` syntax tree.

use crate::ast::{Call, Callee, Case, DocBlock, Expr, Function, Input, InputType, ReturnValue, Spanned, Stmt, TemplateLit, Value};
use crate::diagnostic::{Diagnostic, Span};
use crate::doc::parse_doc;
use crate::lexer::{lex, Tok};

/// Parses one function file. Stops at the first error and returns the
/// tree built so far, plus that one error.
pub fn parse(src: &str) -> (Function, Vec<Diagnostic>) {
    let mut f = Function::default();
    let toks = match lex(src) {
        Ok(toks) => toks,
        Err(d) => return (f, vec![d]),
    };
    let mut p = Parser { toks, pos: 0 };
    let diags = match p.file(&mut f) {
        Ok(()) => vec![],
        Err(d) => vec![d],
    };
    (f, diags)
}

struct Parser {
    toks: Vec<(Tok, Span)>,
    pos: usize,
}

const DOC_PLACES: &str = "the doc comment must go right before `export default function` or a parameter";
const NO_VARIABLES: &str = "Rein has no variables, except `/** question */ const x: type = await request(...)`";
const NO_ELSE: &str = "Rein has no `else`: end the `if` block with a return, and handle the other case after it";

fn no_default(span: Span) -> Diagnostic {
    Diagnostic::error(span, "Rein has no `default`; add a `case` for every member")
}

impl Parser {
    fn peek(&self) -> &Tok {
        &self.toks[self.pos].0
    }

    fn span(&self) -> Span {
        self.toks[self.pos].1
    }

    /// Span of the last token consumed.
    fn last_span(&self) -> Span {
        self.toks[self.pos.saturating_sub(1)].1
    }

    /// Consumes the current token. The final `Eof` is never consumed.
    fn bump(&mut self) -> Span {
        let span = self.span();
        if self.pos + 1 < self.toks.len() {
            self.pos += 1;
        }
        span
    }

    fn unexpected(&self, expected: &str) -> Diagnostic {
        if matches!(self.peek(), Tok::Doc(_)) {
            return Diagnostic::error(self.span(), DOC_PLACES);
        }
        Diagnostic::error(
            self.span(),
            format!("expected {expected}, found {}", self.peek().describe()),
        )
    }

    fn expect(&mut self, want: Tok, expected: &str) -> Result<Span, Diagnostic> {
        if *self.peek() == want {
            Ok(self.bump())
        } else {
            Err(self.unexpected(expected))
        }
    }

    fn string(&mut self, expected: &str) -> Result<Spanned<String>, Diagnostic> {
        if let Tok::Str(value) = self.peek().clone() {
            let span = self.bump();
            Ok(Spanned { value, span })
        } else {
            Err(self.unexpected(expected))
        }
    }

    fn ident(&mut self, expected: &str) -> Result<Spanned<String>, Diagnostic> {
        if let Tok::Ident(value) = self.peek().clone() {
            let span = self.bump();
            Ok(Spanned { value, span })
        } else {
            Err(self.unexpected(expected))
        }
    }

    /// Reads a doc block token, if one is next. The text starts after `/**`.
    fn doc_block(&mut self) -> Option<DocBlock> {
        let Tok::Doc(inner) = self.peek().clone() else { return None };
        let span = self.bump();
        let mut doc = parse_doc(&inner, span.start + 3);
        doc.span = span;
        Some(doc)
    }

    /// The file's one function, with its doc block.
    fn file(&mut self, f: &mut Function) -> Result<(), Diagnostic> {
        if *self.peek() == Tok::Import {
            return Err(Diagnostic::error(
                self.span(),
                "Rein has no imports; reference functions with `@./path` and use std functions directly",
            ));
        }
        if let Some(doc) = self.doc_block() {
            f.description = Some(doc.description.clone());
            f.doc = Some(doc);
        }
        self.expect(Tok::Export, "`export default function`")?;
        self.expect(Tok::Default, "`default`")?;
        if matches!(self.peek(), Tok::Ident(w) if w == "async") {
            f.async_span = Some(self.bump());
            f.is_async = true;
        }
        self.expect(Tok::Function, "`function`")?;
        self.ident("function name")?;
        self.expect(Tok::LParen, "`(`")?;
        while *self.peek() != Tok::RParen {
            let doc = self.doc_block();
            let input = self.param(doc)?;
            f.inputs.push(input);
            if *self.peek() != Tok::Comma {
                break;
            }
            self.bump();
        }
        self.expect(Tok::RParen, "`)`")?;
        f.body = self.block()?;
        if *self.peek() != Tok::Eof {
            if matches!(self.peek(), Tok::Doc(_)) {
                return Err(Diagnostic::error(self.span(), DOC_PLACES));
            }
            return Err(Diagnostic::error(
                self.span(),
                format!(
                    "expected end of file, found {}; a file holds one `export default function`",
                    self.peek().describe()
                ),
            ));
        }
        Ok(())
    }

    /// `IDENT : type`. `doc` is the parameter's own doc block, when it has one.
    fn param(&mut self, doc: Option<DocBlock>) -> Result<Input, Diagnostic> {
        let name = self.ident("parameter name")?;
        self.expect(Tok::Colon, "`:`")?;
        let ty = self.param_type()?;
        let span = name.span.to(self.last_span());
        // A missing doc block gives an empty description; the checker reports it.
        let description = doc.as_ref().map_or_else(
            || Spanned { value: String::new(), span: name.span },
            |d| d.description.clone(),
        );
        Ok(Input { span, name, ty, description, doc })
    }

    /// `boolean`, or a union of strings with an optional leading `|`.
    fn param_type(&mut self) -> Result<InputType, Diagnostic> {
        if *self.peek() == Tok::Boolean {
            self.bump();
            return Ok(InputType::Bool);
        }
        if matches!(self.peek(), Tok::Ident(w) if w == "string") {
            self.bump();
            return Ok(InputType::Text);
        }
        if *self.peek() == Tok::Pipe {
            self.bump();
        }
        let mut members = vec![self.string("`boolean` or a string type, like \"green\"")?];
        while *self.peek() == Tok::Pipe {
            self.bump();
            members.push(self.string("a string type, like \"green\"")?);
        }
        Ok(InputType::Choice(members))
    }

    fn stmt(&mut self) -> Result<Stmt, Diagnostic> {
        match self.peek() {
            Tok::If => self.if_stmt(),
            Tok::Switch => self.switch_stmt(),
            Tok::Return => self.return_stmt(),
            Tok::Default => Err(no_default(self.span())),
            Tok::Else => Err(Diagnostic::error(self.span(), NO_ELSE)),
            Tok::Doc(_) => {
                let doc = self.doc_block().ok_or_else(|| self.unexpected("statement"))?;
                if !matches!(self.peek(), Tok::Reserved(w) if w == "const") {
                    return Err(Diagnostic::error(doc.span, DOC_PLACES));
                }
                self.request_stmt(Some(doc))
            }
            Tok::Reserved(word) if word == "const" => self.request_stmt(None),
            Tok::Reserved(word) => {
                let message = match word.as_str() {
                    "for" | "while" | "do" => "Rein has no loops yet",
                    _ => "Rein has no variables",
                };
                Err(Diagnostic::error(self.span(), message))
            }
            _ => Err(self.unexpected("statement")),
        }
    }

    fn if_stmt(&mut self) -> Result<Stmt, Diagnostic> {
        let start = self.bump();
        self.expect(Tok::LParen, "`(`")?;
        let cond = self.expr()?;
        self.expect(Tok::RParen, "`)`")?;
        let then = self.block()?;
        if *self.peek() == Tok::Else {
            return Err(Diagnostic::error(self.span(), NO_ELSE));
        }
        Ok(Stmt::If { cond, then, span: start.to(self.last_span()) })
    }

    fn switch_stmt(&mut self) -> Result<Stmt, Diagnostic> {
        let start = self.bump();
        self.expect(Tok::LParen, "`(`")?;
        let subject = self.ident("parameter name")?;
        self.expect(Tok::RParen, "`)`")?;
        self.expect(Tok::LBrace, "`{`")?;
        let mut cases = Vec::new();
        // Labels with no body yet. They join the next body, so `case "a": case "b": return x;` is one group.
        let mut pending: Vec<Spanned<String>> = Vec::new();
        loop {
            match self.peek() {
                Tok::Case => {
                    self.bump();
                    pending.push(self.string("case value in quotes")?);
                    self.expect(Tok::Colon, "`:`")?;
                }
                Tok::Default => return Err(no_default(self.span())),
                Tok::RBrace => break,
                _ => {
                    if pending.is_empty() {
                        return Err(self.unexpected("`case`"));
                    }
                    let body = self.clause_body()?;
                    cases.push(Case { values: std::mem::take(&mut pending), body });
                }
            }
        }
        if !pending.is_empty() {
            // Labels at the end with no body. The checker reports them as a fallthrough.
            cases.push(Case { values: pending, body: Vec::new() });
        }
        self.expect(Tok::RBrace, "`}`")?;
        Ok(Stmt::Switch { subject, cases, span: start.to(self.last_span()) })
    }

    /// A clause body: a block, or statements up to the next label or `}`.
    fn clause_body(&mut self) -> Result<Vec<Stmt>, Diagnostic> {
        if *self.peek() == Tok::LBrace {
            return self.block();
        }
        let mut stmts = Vec::new();
        while !matches!(self.peek(), Tok::Case | Tok::RBrace) {
            stmts.push(self.stmt()?);
        }
        Ok(stmts)
    }

    /// `return "text";` or `return importedName;`. The semicolon is optional.
    fn return_stmt(&mut self) -> Result<Stmt, Diagnostic> {
        let start = self.bump();
        if *self.peek() == Tok::Semi || *self.peek() == Tok::RBrace {
            return Err(Diagnostic::error(start, "`return` needs a value: a string, a template, or `@./function`"));
        }
        let (value, span) = match self.peek().clone() {
            Tok::Str(s) => (ReturnValue::Text(s), self.bump()),
            Tok::Template(_) => {
                let lit = self.template_lit()?;
                let span = lit.span;
                (ReturnValue::Template(lit), span)
            }
            Tok::Ref(path) => {
                if *self.next_tok() == Tok::LParen {
                    let call = self.call()?;
                    let span = call.span;
                    (ReturnValue::Call(call), span)
                } else {
                    let span = self.bump();
                    (ReturnValue::Ref(Spanned { value: path, span }), span)
                }
            }
            Tok::Ident(_) if *self.next_tok() == Tok::LParen => {
                let call = self.call()?;
                let span = call.span;
                (ReturnValue::Call(call), span)
            }
            _ => return Err(self.unexpected("a string, a template literal, or a reference like `@./path`")),
        };
        if *self.peek() == Tok::Semi {
            self.bump();
        }
        Ok(Stmt::Return(Spanned { value, span: start.to(span) }))
    }

    /// `/** question */ const name: type = await request(arg);`. The semicolon is optional.
    fn request_stmt(&mut self, doc: Option<DocBlock>) -> Result<Stmt, Diagnostic> {
        let start = self.bump();
        let Some(doc) = doc else {
            return Err(Diagnostic::error(start, NO_VARIABLES));
        };
        let name = self.ident("answer name")?;
        self.expect(Tok::Colon, "`:`")?;
        let ty = self.param_type()?;
        if *self.peek() != Tok::Assign {
            return Err(Diagnostic::error(start, NO_VARIABLES));
        }
        self.bump();
        if !matches!(self.peek(), Tok::Ident(w) if w == "await") {
            return Err(Diagnostic::error(start, NO_VARIABLES));
        }
        self.bump();
        let callee = self.ident("`request`")?;
        if callee.value != "request" {
            return Err(Diagnostic::error(callee.span, "only `request` can be awaited"));
        }
        self.expect(Tok::LParen, "`(`")?;
        let arg = self.value()?;
        if *self.peek() == Tok::Comma {
            self.bump();
        }
        let close = self.expect(Tok::RParen, "`)`")?;
        if *self.peek() == Tok::Semi {
            self.bump();
        }
        let span = start.to(close);
        let description = doc.description.clone();
        let response = Input { span: name.span.to(close), name, ty, description, doc: Some(doc) };
        Ok(Stmt::Request { response: Box::new(response), callee, arg, span })
    }

    /// A string, a template literal, or a template call. A call is only allowed here, as the argument of `request`.
    fn value(&mut self) -> Result<Value, Diagnostic> {
        match self.peek() {
            Tok::Ref(_) | Tok::Ident(_) if *self.next_tok() == Tok::LParen => Ok(Value::Call(self.call()?)),
            _ => self.text_arg(),
        }
    }

    /// The token after the current one. The last token is `Eof`, so this never runs past the end.
    fn next_tok(&self) -> &Tok {
        &self.toks[(self.pos + 1).min(self.toks.len() - 1)].0
    }

    /// A template literal token, as a value.
    fn template_lit(&mut self) -> Result<TemplateLit, Diagnostic> {
        let Tok::Template(pieces) = self.peek().clone() else {
            return Err(self.unexpected("a template literal"));
        };
        let span = self.bump();
        Ok(TemplateLit { pieces, span })
    }

    /// `callee(arg, ...)`. The callee is a std name, or a reference. Each arg is a string or a template literal.
    fn call(&mut self) -> Result<Call, Diagnostic> {
        let callee = match self.peek().clone() {
            Tok::Ref(path) => {
                let span = self.bump();
                Callee::Ref(Spanned { value: path, span })
            }
            Tok::Ident(name) => {
                let span = self.bump();
                Callee::Std(Spanned { value: name, span })
            }
            _ => return Err(self.unexpected("a reference like `@./path`, or a std name")),
        };
        self.expect(Tok::LParen, "`(`")?;
        let mut args = Vec::new();
        while *self.peek() != Tok::RParen {
            args.push(self.text_arg()?);
            if *self.peek() != Tok::Comma {
                break;
            }
            self.bump();
        }
        let close = self.expect(Tok::RParen, "`)`")?;
        Ok(Call { span: callee.span().to(close), callee, args })
    }

    /// A string or a template literal, as an argument.
    fn text_arg(&mut self) -> Result<Value, Diagnostic> {
        match self.peek().clone() {
            Tok::Str(value) => {
                let span = self.bump();
                Ok(Value::Str(Spanned { value, span }))
            }
            Tok::Template(_) => Ok(Value::Template(self.template_lit()?)),
            _ => Err(self.unexpected("a string or a template literal")),
        }
    }

    fn block(&mut self) -> Result<Vec<Stmt>, Diagnostic> {
        self.expect(Tok::LBrace, "`{`")?;
        let mut stmts = Vec::new();
        while *self.peek() != Tok::RBrace {
            stmts.push(self.stmt()?);
        }
        self.bump();
        Ok(stmts)
    }

    fn expr(&mut self) -> Result<Expr, Diagnostic> {
        let mut left = self.and_expr()?;
        while *self.peek() == Tok::OrOr {
            self.bump();
            let right = self.and_expr()?;
            left = Expr::Or(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn and_expr(&mut self) -> Result<Expr, Diagnostic> {
        let mut left = self.unary()?;
        while *self.peek() == Tok::AndAnd {
            self.bump();
            let right = self.unary()?;
            left = Expr::And(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn unary(&mut self) -> Result<Expr, Diagnostic> {
        if *self.peek() == Tok::Bang {
            let span = self.bump();
            let inner = self.unary()?;
            return Ok(Expr::Not(Box::new(inner), span));
        }
        self.atom()
    }

    /// An error for `true` or `false` where a condition or a comparison value goes.
    fn bool_literal_error(&self) -> Option<Diagnostic> {
        match self.peek() {
            Tok::Ident(w) if w == "true" || w == "false" => {
                Some(Diagnostic::error(self.span(), "use the boolean parameter directly"))
            }
            _ => None,
        }
    }

    fn atom(&mut self) -> Result<Expr, Diagnostic> {
        if *self.peek() == Tok::LParen {
            self.bump();
            let inner = self.expr()?;
            self.expect(Tok::RParen, "`)`")?;
            return Ok(inner);
        }
        if let Some(d) = self.bool_literal_error() {
            return Err(d);
        }
        let var = self.ident("parameter name")?;
        let equal = match self.peek() {
            Tok::EqEqEq => true,
            Tok::NotEqEq => false,
            _ => return Ok(Expr::Var(var)),
        };
        self.bump();
        if let Some(d) = self.bool_literal_error() {
            return Err(d);
        }
        let value = self.string("a string in quotes, like \"green\"")?;
        Ok(Expr::Compare { var, value, equal })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) const SAMPLE: &str = r#"
// merge_pr.rein

/**
 * Decide whether a pull request can be merged.
 */
export default function mergePr(
  /** Does the PR change only documentation files? */
  docsOnly: boolean,
  /**
   * What is the CI status of the PR?
   */
  ci: "green" | "red" | "pending",
) {
  if (docsOnly) {
    return "approved";
  }
  switch (ci) {
    case "green":
      return @./review_code;
    case "red":
      return @./ci/fix;
    case "pending":
      return "wait for CI, then run mergePr again";
  }
}
"#;

    fn ok(src: &str) -> Function {
        let (f, diags) = parse(src);
        assert!(diags.is_empty(), "{diags:?}");
        f
    }

    fn one_error(src: &str) -> String {
        let (_, diags) = parse(src);
        assert_eq!(diags.len(), 1, "{diags:?}");
        diags[0].message.clone()
    }

    #[test]
    fn parses_full_sample() {
        let f = ok(SAMPLE);
        assert_eq!(f.description.unwrap().value, "Decide whether a pull request can be merged.");
        assert_eq!(f.inputs.len(), 2);
        assert_eq!(f.inputs[0].ty, InputType::Bool);
        assert_eq!(f.inputs[0].description.value, "Does the PR change only documentation files?");
        assert_eq!(f.inputs[1].description.value, "What is the CI status of the PR?");
        assert_eq!(f.inputs[1].ty.options().unwrap(), ["green", "red", "pending"]);
        assert_eq!(f.body.len(), 2);
        assert!(matches!(&f.body[0], Stmt::If { .. }));
        let Stmt::Switch { cases, .. } = &f.body[1] else { panic!("expected switch") };
        assert_eq!(cases.len(), 3);
        assert!(matches!(&cases[1].body[0], Stmt::Return(r) if matches!(&r.value, ReturnValue::Ref(s) if s.value == "./ci/fix")));
    }

    #[test]
    fn parameter_doc_block_can_be_multi_line() {
        let f = ok("/** d */\nexport default function f(\n  /**\n   * Is it VIP?\n   * Check the CRM.\n   */\n  vip: boolean,\n) { return @./a }");
        assert_eq!(f.inputs[0].description.value, "Is it VIP?\nCheck the CRM.");
    }

    #[test]
    fn parameter_without_doc_has_empty_description() {
        let f = ok("/** d */\nexport default function f(a: boolean) { return \"x\" }");
        assert_eq!(f.inputs[0].description.value, "");
        assert!(f.inputs[0].doc.is_none());
        assert_eq!(f.inputs[0].description.span, f.inputs[0].name.span);
    }

    #[test]
    fn grouped_case_labels_share_one_body() {
        let f = ok(
            "/** d */\nexport default function f(c: \"a\" | \"b\" | \"c\") {\n\
             switch (c) {\n  case \"a\":\n  case \"b\":\n    return \"ab\";\n  case \"c\": { return \"c\"; }\n}\n}",
        );
        let Stmt::Switch { cases, .. } = &f.body[0] else { panic!("expected switch") };
        assert_eq!(cases.len(), 2);
        assert_eq!(cases[0].values.len(), 2);
        assert_eq!(cases[0].values[1].value, "b");
        assert_eq!(cases[1].values.len(), 1);
    }

    #[test]
    fn single_quotes_leading_pipe_and_missing_semicolons() {
        let f = ok(
            "/** d */\nexport default function f(\n  /** x */\n  c: | 'x' | 'y',\n  /** y */\n  b: boolean,\n) {\n  if (c === 'x') { return @../a } return 'z' \n}",
        );
        assert_eq!(f.inputs[0].ty.options().unwrap(), ["x", "y"]);
        assert_eq!(f.inputs.len(), 2);
    }

    #[test]
    fn parses_guard_chain_and_logic() {
        let src = "/** d */\nexport default function f(\n  /** a */ a: boolean,\n  /** b */ b: boolean,\n  /** c */ c: \"x\" | \"y\",\n) {\n\
             if (!(a && b) || c === \"x\") { return \"one\" }\n\
             if (a) { return \"two\" }\n\
             return \"three\"\n}";
        let f = ok(src);
        assert_eq!(f.inputs.len(), 3);
        let Stmt::If { cond, .. } = &f.body[0] else { panic!("expected if") };
        assert!(matches!(cond, Expr::Or(..)));
        assert!(matches!(&f.body[1], Stmt::If { .. }));
    }

    #[test]
    fn else_is_not_in_rein() {
        let msg = one_error("/** d */\nexport default function f(\n  /** a */ a: boolean,\n) {\n  if (a) { return \"1\" }\n  else { return \"2\" }\n}");
        assert_eq!(msg, "Rein has no `else`: end the `if` block with a return, and handle the other case after it");
    }

    #[test]
    fn return_needs_a_value() {
        let msg = one_error("/** d */\nexport default function f() {\n  return;\n}");
        assert_eq!(msg, "`return` needs a value: a string, a template, or `@./function`");
    }

    #[test]
    fn return_without_value_is_one_error_with_partial_tree() {
        let (f, diags) = parse("/** x */\nexport default function f() {\n  return\n}");
        assert_eq!(diags.len(), 1, "{diags:?}");
        assert!(diags[0].message.contains("`return` needs a value"), "{}", diags[0].message);
        assert!(f.description.is_some());
    }

    #[test]
    fn imports_are_not_in_rein() {
        let msg = one_error("import a from \"./a\";\n/** d */\nexport default function f() { return @./a }");
        assert_eq!(msg, "Rein has no imports; reference functions with `@./path` and use std functions directly");
    }

    #[test]
    fn bare_name_is_not_a_return_value() {
        let msg = one_error("/** d */\nexport default function f() { return nope; }");
        assert!(msg.contains("expected a string, a template literal, or a reference"), "{msg}");
    }

    #[test]
    fn switch_default_is_not_in_rein() {
        let msg = one_error(
            "/** d */\nexport default function f(c: \"x\" | \"y\") {\n switch (c) { case \"x\": return \"a\"; default: return \"b\" }\n}",
        );
        assert_eq!(msg, "Rein has no `default`; add a `case` for every member");
    }

    #[test]
    fn function_doc_must_be_right_before_export() {
        for src in [
            "/** d */\nexport default function f() { return \"x\" }\n/** y */",
            "export default function f(/** a */ a: boolean) { return \"x\" }\n/** y */",
        ] {
            let msg = one_error(src);
            assert!(msg.contains("the doc comment must go right before"), "{src}: {msg}");
        }
    }

    #[test]
    fn doc_in_a_wrong_place_inside_the_body_is_an_error() {
        let msg = one_error("/** d */\nexport default function f() {\n  /** note */\n  return \"x\"\n}");
        assert!(msg.contains("the doc comment must go right before"), "{msg}");
    }

    #[test]
    fn one_function_per_file() {
        let msg = one_error("/** d */\nexport default function f() { return \"x\" }\nexport default function g() { return \"y\" }");
        assert!(msg.contains("a file holds one `export default function`"), "{msg}");
    }

    #[test]
    fn loops_and_variables_have_helpful_errors() {
        let head = "/** d */\nexport default function f(a: boolean) {\n";
        assert_eq!(one_error(&format!("{head} for (;;) {{ }}\n}}")), "Rein has no loops yet");
        assert_eq!(one_error(&format!("{head} while (a) {{ }}\n}}")), "Rein has no loops yet");
        assert_eq!(one_error(&format!("{head} do {{ }}\n}}")), "Rein has no loops yet");
        assert!(one_error(&format!("{head} const x = \"a\"\n}}")).starts_with("Rein has no variables"));
        assert_eq!(one_error(&format!("{head} let x = \"a\"\n}}")), "Rein has no variables");
        assert_eq!(one_error(&format!("{head} var x = \"a\"\n}}")), "Rein has no variables");
    }

    #[test]
    fn true_and_false_in_conditions_have_a_helpful_error() {
        let head = "/** d */\nexport default function f(\n  /** a */ a: boolean,\n) {\n";
        assert_eq!(one_error(&format!("{head} if (true) {{ return \"x\" }}\n}}")), "use the boolean parameter directly");
        assert_eq!(one_error(&format!("{head} if (a === true) {{ return \"x\" }}\n}}")), "use the boolean parameter directly");
    }

    #[test]
    fn old_syntax_is_a_syntax_error() {
        let (_, diags) = parse("input docs_only: bool\nreturn \"x\"");
        assert_eq!(diags.len(), 1, "{diags:?}");
    }

    #[test]
    fn lex_error_gives_one_error_and_no_tree() {
        let (f, diags) = parse("/** abc");
        assert_eq!(diags.len(), 1);
        assert!(f.description.is_none());
        assert!(f.doc.is_none());
    }

    #[test]
    fn template_return_parses() {
        let f = ok(r#"/** d */
export default function f(
  /** C */ c: "x" | "y",
) {
  return `Pick ${c}`;
}"#);
        let Stmt::Return(r) = &f.body[0] else { panic!("expected return") };
        let ReturnValue::Template(lit) = &r.value else { panic!("expected template, got {:?}", r.value) };
        assert_eq!(lit.pieces.len(), 2);
    }

    #[test]
    fn call_return_parses_with_literal_and_template_args() {
        let f = ok(r#"/** d */
export default function f(
  /** C */ c: "x" | "y",
) {
  return @./tpl("a", `b ${c}`);
}"#);
        let Stmt::Return(r) = &f.body[0] else { panic!("expected return") };
        let ReturnValue::Call(call) = &r.value else { panic!("expected call, got {:?}", r.value) };
        assert_eq!(call.callee.key(), "./tpl");
        assert_eq!(call.args.len(), 2);
        assert!(matches!(&call.args[0], Value::Str(s) if s.value == "a"));
        assert!(matches!(&call.args[1], Value::Template(_)));
    }

    #[test]
    fn call_arguments_cannot_be_calls() {
        let msg = one_error(r#"/** d */
export default function f() {
  return @./tpl(other("x"));
}"#);
        assert!(msg.contains("expected a string or a template literal"), "{msg}");
    }

    #[test]
    fn string_parameter_type_is_text() {
        let f = ok("/** d */\nexport default function f(\n  /** A */ a: string,\n) { return `${a}` }");
        assert_eq!(f.inputs[0].ty, InputType::Text);
    }

    #[test]
    fn request_const_parses_with_its_doc_and_type() {
        let f = ok(r#"/** d */
export default async function f() {
  /** Did it work? */
  const ok: boolean = await request("Do it.");
  if (!ok) { return "n" }
  return "y"
}"#);
        let Stmt::Request { response, arg, callee, .. } = &f.body[0] else { panic!("expected request") };
        assert_eq!(response.name.value, "ok");
        assert_eq!(response.ty, InputType::Bool);
        assert_eq!(response.description.value, "Did it work?");
        assert_eq!(callee.value, "request");
        assert!(matches!(arg, Value::Str(s) if s.value == "Do it."));
        assert!(f.is_async);
    }

    #[test]
    fn request_argument_can_be_a_std_template_call() {
        let f = ok(r#"/** d */
export default async function f() {
  /** Did it work? */
  const ok: boolean = await request(delegate("m", "t"));
  return "y"
}"#);
        let Stmt::Request { arg, .. } = &f.body[0] else { panic!("expected request") };
        assert!(matches!(arg, Value::Call(c) if c.callee.key() == "delegate" && c.args.len() == 2));
    }

    #[test]
    fn request_answer_needs_a_doc_comment() {
        let msg = one_error("/** d */\nexport default async function f() {\n  const ok: boolean = await request(\"x\");\n  return \"y\"\n}");
        assert!(msg.contains("Rein has no variables"), "{msg}");
    }

    #[test]
    fn only_request_can_be_awaited() {
        let msg = one_error("/** d */\nexport default async function f() {\n  /** Q */\n  const x: boolean = await fetch(\"a\");\n  return \"y\"\n}");
        assert!(msg.contains("only `request` can be awaited"), "{msg}");
    }
}
