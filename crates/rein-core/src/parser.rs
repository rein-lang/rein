//! Recursive descent parser: tokens to a `Function` syntax tree.

use crate::ast::{
    Arg, Call, Callee, Case, DocBlock, Expr, Function, Input, InputType, Param, ReturnValue, Spanned, Stmt, Struct, StructParam,
    StructRef, TemplateLit, Value,
};
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

const DOC_PLACES: &str =
    "the doc comment must go right before `export default function`, an `interface`, a parameter, a field, or a const answer";
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

    /// The file's items: one `export default function` and its inline `interface`s, or, in a `.struct.rein`
    /// file, one `export default interface`. The checker decides which items a file may hold.
    fn file(&mut self, f: &mut Function) -> Result<(), Diagnostic> {
        loop {
            if *self.peek() == Tok::Import {
                return Err(Diagnostic::error(
                    self.span(),
                    "Rein has no imports; reference functions with `@./path` and use std functions directly",
                ));
            }
            let doc = self.doc_block();
            if self.is_word("interface") {
                let s = self.struct_decl(doc, None, false)?;
                f.structs.push(s);
                continue;
            }
            if *self.peek() == Tok::Export {
                let found = self.peek().describe();
                let export = self.bump();
                let default = *self.peek() == Tok::Default;
                if default {
                    self.bump();
                }
                if self.is_word("interface") {
                    let s = self.struct_decl(doc, Some(export), default)?;
                    f.structs.push(s);
                    continue;
                }
                if !default {
                    return Err(self.unexpected("`default`"));
                }
                if f.name.is_some() {
                    return Err(Diagnostic::error(
                        export,
                        format!("expected end of file, found {found}; a file holds one `export default function`"),
                    ));
                }
                self.function(f, doc)?;
                continue;
            }
            if let Some(doc) = doc {
                return Err(Diagnostic::error(doc.span, DOC_PLACES));
            }
            match self.peek() {
                Tok::Eof if f.name.is_some() || f.exported_struct().is_some() => break,
                Tok::Eof => return Err(self.unexpected("`export default function`")),
                _ if f.name.is_some() => {
                    return Err(Diagnostic::error(
                        self.span(),
                        format!(
                            "expected end of file, found {}; a file holds one `export default function`",
                            self.peek().describe()
                        ),
                    ));
                }
                _ => return Err(self.unexpected("`export default function`")),
            }
        }
        // An inline struct parameter binds now. A `@./` struct binds when the project loads.
        if let Some(StructRef::Local(name)) = f.struct_param().map(|p| p.ty.clone()) {
            let found = f.structs.iter().find(|s| s.name.value == name.value).cloned().ok_or_else(|| {
                format!("cannot find struct `{0}`; declare `interface {0} {{ ... }}` in this file", name.value)
            });
            f.bind_struct(found);
        }
        Ok(())
    }

    fn is_word(&self, word: &str) -> bool {
        matches!(self.peek(), Tok::Ident(w) if w == word)
    }

    /// `function NAME(params) block`, after `export default`.
    fn function(&mut self, f: &mut Function, doc: Option<DocBlock>) -> Result<(), Diagnostic> {
        if let Some(doc) = doc {
            f.description = Some(doc.description.clone());
            f.doc = Some(doc);
        }
        if self.is_word("async") {
            f.async_span = Some(self.bump());
            f.is_async = true;
        }
        self.expect(Tok::Function, "`function`")?;
        f.name = Some(self.ident("function name")?);
        self.expect(Tok::LParen, "`(`")?;
        while *self.peek() != Tok::RParen {
            let doc = self.doc_block();
            if *self.peek() == Tok::LBrace {
                if let Some(doc) = doc {
                    return Err(Diagnostic::error(
                        doc.span,
                        "a struct parameter has no doc comment; document each field in its `interface`",
                    ));
                }
                f.params.push(Param::Struct(self.struct_param()?));
            } else {
                let input = self.param(doc, true)?;
                f.inputs.push(input.clone());
                f.params.push(Param::Scalar(input));
            }
            if *self.peek() != Tok::Comma {
                break;
            }
            self.bump();
        }
        self.expect(Tok::RParen, "`)`")?;
        f.body = self.block()?;
        Ok(())
    }

    /// `{ a, b }: Name` or `{ a, b }: @./name`.
    fn struct_param(&mut self) -> Result<StructParam, Diagnostic> {
        let start = self.bump();
        let mut fields = Vec::new();
        while *self.peek() != Tok::RBrace {
            fields.push(self.ident("field name")?);
            if *self.peek() != Tok::Comma {
                break;
            }
            self.bump();
        }
        self.expect(Tok::RBrace, "`}`")?;
        self.expect(Tok::Colon, "`:`")?;
        let ty = match self.peek().clone() {
            Tok::Ref(path) => StructRef::Ref(Spanned { value: path, span: self.bump() }),
            Tok::Ident(name) => StructRef::Local(Spanned { value: name, span: self.bump() }),
            _ => return Err(self.unexpected("a struct name, or a reference like `@./name`")),
        };
        Ok(StructParam { fields, ty, span: start.to(self.last_span()) })
    }

    /// `interface Name { /** doc */ field: type; ... }`. A field ends with `;`, `,`, or nothing.
    fn struct_decl(&mut self, doc: Option<DocBlock>, export: Option<Span>, default: bool) -> Result<Struct, Diagnostic> {
        let start = export.unwrap_or_else(|| self.span());
        self.bump();
        let name = self.ident("struct name")?;
        self.expect(Tok::LBrace, "`{`")?;
        let mut fields = Vec::new();
        while *self.peek() != Tok::RBrace {
            let doc = self.doc_block();
            fields.push(self.param(doc, false)?);
            if matches!(self.peek(), Tok::Semi | Tok::Comma) {
                self.bump();
            }
        }
        self.bump();
        Ok(Struct { name, doc, fields, export, default, span: start.to(self.last_span()) })
    }

    /// `IDENT : type`. `doc` is the doc block of the parameter or field, when it has one. A parameter names a
    /// struct type only when it is destructured, so `x: Name` gets its own message.
    fn param(&mut self, doc: Option<DocBlock>, param: bool) -> Result<Input, Diagnostic> {
        let name = self.ident(if param { "parameter name" } else { "field name" })?;
        self.expect(Tok::Colon, "`:`")?;
        let struct_type = match self.peek() {
            Tok::Ref(p) => Some(format!("@{p}")),
            Tok::Ident(w) if w != "string" => Some(w.clone()),
            _ => None,
        };
        if let (true, Some(ty)) = (param, struct_type) {
            return Err(Diagnostic::error(
                self.span(),
                format!("a struct parameter is destructured: write `{{ field, ... }}: {ty}`"),
            ));
        }
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

    /// `callee(name=value, ...)`. The callee is a std name, or a reference. Each arg is `name=value`.
    /// A positional value parses too, so the checker can report it with the parameter it should name.
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
            args.push(self.arg()?);
            if *self.peek() != Tok::Comma {
                break;
            }
            self.bump();
        }
        let close = self.expect(Tok::RParen, "`)`")?;
        Ok(Call { span: callee.span().to(close), callee, args })
    }

    /// One argument of a call: `name=value`. A name is an identifier followed by `=`.
    fn arg(&mut self) -> Result<Arg, Diagnostic> {
        let name = match self.peek().clone() {
            Tok::Ident(value) if *self.next_tok() == Tok::Assign => {
                let span = self.bump();
                self.bump();
                Some(Spanned { value, span })
            }
            _ => None,
        };
        let value = self.arg_value()?;
        let span = name.as_ref().map_or(value.span(), |n| n.span.to(value.span()));
        Ok(Arg { name, value, span })
    }

    /// The value of an argument: a string, a template literal, a bare name of a `string` parameter, or a call.
    fn arg_value(&mut self) -> Result<Value, Diagnostic> {
        if let Tok::Ident(name) = self.peek().clone() {
            if *self.next_tok() != Tok::LParen {
                let span = self.bump();
                return Ok(Value::Name(Spanned { value: name, span }));
            }
        }
        self.value()
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

interface Input {
  /** Does the PR change only documentation files? */
  docsOnly: boolean;
  /**
   * What is the CI status of the PR?
   */
  ci: "green" | "red" | "pending";
}

/**
 * Decide whether a pull request can be merged.
 */
export default function mergePr({ docsOnly, ci }: Input) {
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
  return @./tpl(a="a", b=`b ${c}`);
}"#);
        let Stmt::Return(r) = &f.body[0] else { panic!("expected return") };
        let ReturnValue::Call(call) = &r.value else { panic!("expected call, got {:?}", r.value) };
        assert_eq!(call.callee.key(), "./tpl");
        assert_eq!(call.args.len(), 2);
        assert_eq!(call.args[0].name.as_ref().map(|n| n.value.as_str()), Some("a"));
        assert!(matches!(&call.args[0].value, Value::Str(s) if s.value == "a"));
        assert_eq!(call.args[1].name.as_ref().map(|n| n.value.as_str()), Some("b"));
        assert!(matches!(&call.args[1].value, Value::Template(_)));
    }

    #[test]
    fn positional_argument_parses_with_no_name() {
        let f = ok(r#"/** d */
export default function f() {
  return @./tpl("a");
}"#);
        let Stmt::Return(r) = &f.body[0] else { panic!("expected return") };
        let ReturnValue::Call(call) = &r.value else { panic!("expected call") };
        assert!(call.args[0].name.is_none());
        assert!(matches!(&call.args[0].value, Value::Str(s) if s.value == "a"));
    }

    #[test]
    fn call_argument_can_be_a_template_call() {
        let f = ok(r#"/** d */
export default function f() {
  return @./tpl(a=@./inner(delegate(model="m", task="t")), b="b");
}"#);
        let Stmt::Return(r) = &f.body[0] else { panic!("expected return") };
        let ReturnValue::Call(call) = &r.value else { panic!("expected call, got {:?}", r.value) };
        assert_eq!(call.args.len(), 2);
        let Value::Call(inner) = &call.args[0].value else { panic!("expected nested call, got {:?}", call.args[0]) };
        assert_eq!(inner.callee.key(), "./inner");
        assert!(matches!(&inner.args[0].value, Value::Call(c) if c.callee.key() == "delegate"));
        assert!(matches!(&call.args[1].value, Value::Str(s) if s.value == "b"));
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
  const ok: boolean = await request(delegate(model="m", task="t"));
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

    #[test]
    fn call_arguments_can_be_bare_names() {
        let f = ok(r#"/** d */
export default function f(
  /** Link */ pr: string,
) {
  return @./merge(a=pr, b="x", c=`t ${pr}`);
}"#);
        let Stmt::Return(r) = &f.body[0] else { panic!("expected return") };
        let ReturnValue::Call(call) = &r.value else { panic!("expected call") };
        assert_eq!(call.callee.key(), "./merge");
        assert!(matches!(&call.args[0].value, Value::Name(n) if n.value == "pr"));
        assert!(matches!(&call.args[1].value, Value::Str(_)));
        assert!(matches!(&call.args[2].value, Value::Template(_)));
    }

    #[test]
    fn bare_name_can_be_a_std_call_argument() {
        let f = ok(r#"/** d */
export default function f(
  /** Link */ pr: string,
) {
  return delegate(model="use opus model", task=pr);
}"#);
        let Stmt::Return(r) = &f.body[0] else { panic!("expected return") };
        let ReturnValue::Call(call) = &r.value else { panic!("expected call") };
        assert!(matches!(&call.callee, Callee::Std(s) if s.value == "delegate"));
        assert!(matches!(&call.args[1].value, Value::Name(n) if n.value == "pr"));
    }

    #[test]
    fn bare_name_is_not_a_request_value() {
        let msg = one_error("/** d */\nexport default async function f(\n  /** Link */ pr: string,\n) {\n  /** Ok? */\n  const ok: boolean = await request(pr);\n  return \"y\"\n}");
        assert!(msg.contains("expected a string or a template literal"), "{msg}");
    }

    #[test]
    fn struct_parameter_binds_the_inline_interface_fields() {
        let src = "interface Input {\n  /** A? */\n  a: boolean;\n  /** B? */\n  b: \"x\" | \"y\";\n}\n/** d */\nexport default function f({ b, a }: Input) {\n  if (a) { return \"1\" }\n  return `${b}`\n}";
        let f = ok(src);
        assert_eq!(f.params.len(), 1);
        let names: Vec<&str> = f.inputs.iter().map(|i| i.name.value.as_str()).collect();
        assert_eq!(names, ["a", "b"]);
        assert_eq!(f.inputs[0].description.value, "A?");
        assert_eq!(f.inputs[0].ty, InputType::Bool);
        let span = f.inputs[0].name.span;
        assert_eq!(&src[span.start..span.end], "a");
        assert!(span.start > src.find("function").unwrap(), "{span:?}");
        assert_eq!(f.structs.len(), 1);
        assert!(matches!(&f.input_struct, Some(Ok(_))));
    }

    #[test]
    fn a_struct_type_must_be_destructured() {
        assert_eq!(
            one_error("/** d */\nexport default function f(x: Input) { return \"y\" }"),
            "a struct parameter is destructured: write `{ field, ... }: Input`"
        );
        assert_eq!(
            one_error("/** d */\nexport default function f(x: @./ticket) { return \"y\" }"),
            "a struct parameter is destructured: write `{ field, ... }: @./ticket`"
        );
    }

    #[test]
    fn a_struct_file_holds_its_exported_interface() {
        let f = ok("/** A ticket. */\nexport default interface Ticket {\n  /** Title. */\n  title: string\n}\n");
        assert!(f.name.is_none());
        let s = f.exported_struct().unwrap();
        assert_eq!(s.name.value, "Ticket");
        assert_eq!(s.fields.len(), 1);
        assert_eq!(s.fields[0].name.value, "title");
        assert_eq!(s.fields[0].ty, InputType::Text);
    }

    #[test]
    fn a_struct_parameter_has_no_doc_comment() {
        let msg = one_error("/** d */\nexport default function f(/** no */ { a }: Input) { return \"y\" }\ninterface Input {\n  /** A */ a: boolean;\n}");
        assert!(msg.contains("a struct parameter has no doc comment"), "{msg}");
    }
}
