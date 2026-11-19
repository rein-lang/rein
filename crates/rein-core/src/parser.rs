//! Recursive descent parser: tokens to a `Function` syntax tree.

use crate::ast::{
    Arg, Awaited, Call, Callee, Case, DocBlock, Expr, Function, Import, Input, InputType, Param, Piece, ReturnValue, Spanned,
    Stmt, Struct, StructParam, StructRef, TemplateLit, Value,
};
use crate::diagnostic::{Diagnostic, Span};
use crate::doc::parse_doc;
use crate::imports;
use crate::lexer::{lex, Tok};

/// Parses one function file. Stops at the first error and returns the
/// tree built so far, plus that one error.
pub fn parse(src: &str) -> (Function, Vec<Diagnostic>) {
    let mut f = Function::default();
    let toks = match lex(src) {
        Ok(toks) => toks,
        Err(d) => return (f, vec![d]),
    };
    let mut p = Parser { toks, pos: 0, imports: Vec::new(), items: false };
    let diags = match p.file(&mut f) {
        Ok(()) => vec![],
        Err(d) => vec![d],
    };
    f.imports = p.imports;
    (f, diags)
}

struct Parser {
    toks: Vec<(Tok, Span)>,
    pos: usize,
    /// The `import` lines so far. A use of a local name reads its path, see [`imports::reference`].
    imports: Vec<Import>,
    /// True once the file has a doc block, an interface or an export. An `import` after that is an error.
    items: bool,
}

const IMPORTS_FIRST: &str = "imports come first; move this `import` above the doc comment and the declarations";

const DOC_PLACES: &str =
    "the doc comment must go right before `export default function`, an `interface`, a parameter, a field, or a const answer";
const NO_VARIABLES: &str = "Rein has no variables, except `/** question */ const x: type = await request(...)`";
const ONLY_AWAIT: &str = "only `request` or a reference like `@program` can be awaited";
const AWAIT_NEEDS_CONST: &str =
    "`await` gives an answer; write `/** question */ const name: Type = await @program(...);`, then read `name`";
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

    /// A tag: a quoted string in a type, a `case` label, or a comparison. A template there is an error that shows
    /// the quoted form.
    fn string(&mut self, expected: &str) -> Result<Spanned<String>, Diagnostic> {
        match self.peek().clone() {
            Tok::Str(value) => {
                let span = self.bump();
                Ok(Spanned { value, span })
            }
            Tok::Template(pieces) => {
                let text = match pieces.as_slice() {
                    [] => "",
                    [Piece::Text(t)] if !t.contains('\n') => t.as_str(),
                    _ => "...",
                };
                Err(Diagnostic::error(self.span(), format!("a tag uses quotes: write \"{text}\" instead of `{text}`")))
            }
            _ => Err(self.unexpected(expected)),
        }
    }

    /// A name of one word. A declaration cannot name a field, so `a.b` is an error here.
    fn ident(&mut self, expected: &str) -> Result<Spanned<String>, Diagnostic> {
        if matches!(self.peek(), Tok::Ident(v) if v.contains('.')) {
            return Err(Diagnostic::error(self.span(), "a name is one word; a `.` reads a field of a struct answer"));
        }
        self.path(expected)
    }

    /// A name, or a field of a struct answer such as `pick.pr`.
    fn path(&mut self, expected: &str) -> Result<Spanned<String>, Diagnostic> {
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
                if self.items {
                    return Err(Diagnostic::error(self.span(), IMPORTS_FIRST));
                }
                let import = imports::parse_import(&self.toks, &mut self.pos)?;
                imports::add(&mut self.imports, import)?;
                continue;
            }
            let doc = self.doc_block();
            if *self.peek() == Tok::Import {
                return Err(Diagnostic::error(doc.map_or(self.span(), |d| d.span), IMPORTS_FIRST));
            }
            self.items = true;
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
                if !default && matches!(self.peek(), Tok::LBrace | Tok::Star) {
                    let message = "`export ... from` is not supported; write `import { name } from \"path\";`";
                    return Err(Diagnostic::error(self.span(), message));
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
                self.function(f, doc, export)?;
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
        for (answer, r) in f.answer_struct_refs() {
            if let StructRef::Local(name) = r {
                let found = f.structs.iter().find(|s| s.name.value == name.value).cloned().ok_or_else(|| {
                    format!("cannot find struct `{0}`; declare `interface {0} {{ ... }}` in this file", name.value)
                });
                f.bind_answer_struct(&answer, found);
            }
        }
        if let Some(d) = imports::clash(&self.imports, f) {
            return Err(d);
        }
        Ok(())
    }

    fn is_word(&self, word: &str) -> bool {
        matches!(self.peek(), Tok::Ident(w) if w == word)
    }

    /// `function NAME(params) block`, after `export default`. `export` is the span of the `export` keyword.
    fn function(&mut self, f: &mut Function, doc: Option<DocBlock>, export: Span) -> Result<(), Diagnostic> {
        if let Some(doc) = doc {
            f.description = Some(doc.description.clone());
            f.doc = Some(doc);
        }
        if self.is_word("async") {
            f.async_span = Some(self.bump());
            f.is_async = true;
        }
        self.expect(Tok::Function, "`function`")?;
        let name = self.ident("function name")?;
        f.header = export.to(name.span);
        f.name = Some(name);
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
        f.span = export.to(self.last_span());
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
            Tok::Ident(name) => {
                let span = self.bump();
                imports::struct_ref(&mut self.imports, &name, span)?
            }
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

    /// The type of a `request` answer: a struct name or a `@./` reference, or a scalar type. The checker accepts a struct.
    fn answer_type(&mut self) -> Result<InputType, Diagnostic> {
        match self.peek().clone() {
            Tok::Ref(path) => Ok(InputType::Struct(StructRef::Ref(Spanned { value: path, span: self.bump() }))),
            Tok::Ident(name) if name != "string" => {
                let span = self.bump();
                Ok(InputType::Struct(imports::struct_ref(&mut self.imports, &name, span)?))
            }
            _ => self.param_type(),
        }
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
                if matches!(self.peek(), Tok::Ident(w) if w == "await") {
                    return Err(Diagnostic::error(self.span(), AWAIT_NEEDS_CONST));
                }
                if !matches!(self.peek(), Tok::Reserved(w) if w == "const") {
                    return Err(Diagnostic::error(doc.span, DOC_PLACES));
                }
                self.request_stmt(Some(doc))
            }
            Tok::Reserved(word) if word == "const" => self.request_stmt(None),
            Tok::Ident(word) if word == "await" => Err(Diagnostic::error(self.span(), AWAIT_NEEDS_CONST)),
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
        let subject = self.path("parameter name")?;
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
            Tok::Str(s) => {
                let span = self.bump();
                (ReturnValue::Text(Spanned { value: s, span }), span)
            }
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
            Tok::Ident(word) if word == "await" => return Err(Diagnostic::error(self.span(), AWAIT_NEEDS_CONST)),
            Tok::Ident(name) if *self.next_tok() != Tok::LParen && imports::is_import(&self.imports, &name) => {
                let span = self.bump();
                (ReturnValue::Ref(imports::reference(&mut self.imports, &name, span)?), span)
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
        let ty = self.answer_type()?;
        if *self.peek() != Tok::Assign {
            return Err(Diagnostic::error(start, NO_VARIABLES));
        }
        self.bump();
        if !matches!(self.peek(), Tok::Ident(w) if w == "await") {
            return Err(Diagnostic::error(start, NO_VARIABLES));
        }
        self.bump();
        let (awaited, close) = self.awaited()?;
        if *self.peek() == Tok::Semi {
            self.bump();
        }
        let span = start.to(close);
        let description = doc.description.clone();
        let response = Input { span: name.span.to(close), name, ty, description, doc: Some(doc) };
        Ok(Stmt::Request { response: Box::new(response), awaited, span })
    }

    /// What follows `await`: `request(arg)`, or a reference with optional arguments (a call with return).
    /// Returns it with the span of its last token.
    fn awaited(&mut self) -> Result<(Awaited, Span), Diagnostic> {
        if let Tok::Ref(path) = self.peek().clone() {
            if *self.next_tok() == Tok::LParen {
                let call = self.call()?;
                let span = call.span;
                return Ok((Awaited::Call(ReturnValue::Call(call)), span));
            }
            let span = self.bump();
            return Ok((Awaited::Call(ReturnValue::Ref(Spanned { value: path, span })), span));
        }
        if let Tok::Ident(name) = self.peek().clone() {
            if imports::is_import(&self.imports, &name) {
                if *self.next_tok() == Tok::LParen {
                    let call = self.call()?;
                    let span = call.span;
                    return Ok((Awaited::Call(ReturnValue::Call(call)), span));
                }
                let span = self.bump();
                let target = imports::reference(&mut self.imports, &name, span)?;
                return Ok((Awaited::Call(ReturnValue::Ref(target)), span));
            }
        }
        let callee = self.ident("`request` or a reference like `@program`")?;
        if callee.value != "request" {
            return Err(Diagnostic::error(callee.span, ONLY_AWAIT));
        }
        self.expect(Tok::LParen, "`(`")?;
        let arg = self.value()?;
        if *self.peek() == Tok::Comma {
            self.bump();
        }
        let close = self.expect(Tok::RParen, "`)`")?;
        Ok((Awaited::Request { callee, arg }, close))
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
                if imports::is_import(&self.imports, &name) {
                    Callee::Ref(imports::reference(&mut self.imports, &name, span)?)
                } else {
                    Callee::Std(Spanned { value: name, span })
                }
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
        let var = self.path("parameter name")?;
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
#[path = "parser_tests.rs"]
pub(crate) mod tests;
