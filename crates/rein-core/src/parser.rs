//! Recursive descent parser: tokens to a `Function` syntax tree.

use crate::ast::{Case, Expr, Function, Input, InputType, ReturnValue, Spanned, Stmt};
use crate::diagnostic::{Diagnostic, Span};
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

    /// Fills `f` in place, so a failed parse keeps what came before the error.
    fn file(&mut self, f: &mut Function) -> Result<(), Diagnostic> {
        if *self.peek() == Tok::Description {
            self.bump();
            f.description = Some(self.string("description string")?);
        }
        while *self.peek() == Tok::Input {
            f.inputs.push(self.input()?);
        }
        while *self.peek() != Tok::Eof {
            f.body.push(self.stmt()?);
        }
        Ok(())
    }

    fn input(&mut self) -> Result<Input, Diagnostic> {
        let start = self.bump();
        let name = self.ident("input name")?;
        self.expect(Tok::Colon, "`:`")?;
        let ty = self.input_type()?;
        let description = self.string("input description")?;
        Ok(Input { span: start.to(description.span), name, ty, description })
    }

    fn input_type(&mut self) -> Result<InputType, Diagnostic> {
        match self.peek() {
            Tok::Bool => {
                self.bump();
                Ok(InputType::Bool)
            }
            Tok::Choice => {
                self.bump();
                Ok(InputType::Choice(self.choice_options()?))
            }
            _ => Err(self.unexpected("`bool` or `choice`")),
        }
    }

    fn choice_options(&mut self) -> Result<Vec<Spanned<String>>, Diagnostic> {
        self.expect(Tok::LParen, "`(`")?;
        let mut options = vec![self.string("choice option")?];
        while *self.peek() == Tok::Comma {
            self.bump();
            if *self.peek() == Tok::RParen {
                break;
            }
            options.push(self.string("choice option")?);
        }
        self.expect(Tok::RParen, "`)`")?;
        Ok(options)
    }

    fn stmt(&mut self) -> Result<Stmt, Diagnostic> {
        match self.peek() {
            Tok::If => self.if_stmt(),
            Tok::Switch => self.switch_stmt(),
            Tok::Return => self.return_stmt(),
            _ => Err(self.unexpected("statement")),
        }
    }

    fn if_stmt(&mut self) -> Result<Stmt, Diagnostic> {
        let start = self.bump();
        let cond = self.expr()?;
        let then = self.block()?;
        let otherwise = if *self.peek() == Tok::Else {
            self.bump();
            if *self.peek() == Tok::If {
                Some(vec![self.if_stmt()?])
            } else {
                Some(self.block()?)
            }
        } else {
            None
        };
        Ok(Stmt::If { cond, then, otherwise, span: start.to(self.last_span()) })
    }

    fn switch_stmt(&mut self) -> Result<Stmt, Diagnostic> {
        let start = self.bump();
        let subject = self.ident("input name")?;
        self.expect(Tok::LBrace, "`{`")?;
        let mut cases = Vec::new();
        let mut default = None;
        loop {
            if *self.peek() == Tok::Case {
                let span = self.bump();
                if default.is_some() {
                    return Err(Diagnostic::error(span, "`default` must be the last branch of a switch"));
                }
                let value = self.string("case value")?;
                let body = self.block()?;
                cases.push(Case { value, body });
            } else if *self.peek() == Tok::Default {
                let span = self.bump();
                if default.is_some() {
                    return Err(Diagnostic::error(span, "duplicate `default`"));
                }
                default = Some(self.block()?);
            } else {
                break;
            }
        }
        self.expect(Tok::RBrace, "`}`")?;
        Ok(Stmt::Switch { subject, cases, default, span: start.to(self.last_span()) })
    }

    fn return_stmt(&mut self) -> Result<Stmt, Diagnostic> {
        let start = self.bump();
        let (value, span) = match self.peek().clone() {
            Tok::Str(s) => (ReturnValue::Text(s), self.bump()),
            Tok::Ref(r) => (ReturnValue::Ref(r), self.bump()),
            _ => return Err(self.unexpected("string or `@function`")),
        };
        Ok(Stmt::Return(Spanned { value, span: start.to(span) }))
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
        while *self.peek() == Tok::Or {
            self.bump();
            let right = self.and_expr()?;
            left = Expr::Or(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn and_expr(&mut self) -> Result<Expr, Diagnostic> {
        let mut left = self.unary()?;
        while *self.peek() == Tok::And {
            self.bump();
            let right = self.unary()?;
            left = Expr::And(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn unary(&mut self) -> Result<Expr, Diagnostic> {
        if *self.peek() == Tok::Not {
            let span = self.bump();
            let inner = self.unary()?;
            return Ok(Expr::Not(Box::new(inner), span));
        }
        self.atom()
    }

    fn atom(&mut self) -> Result<Expr, Diagnostic> {
        if *self.peek() == Tok::LParen {
            self.bump();
            let inner = self.expr()?;
            self.expect(Tok::RParen, "`)`")?;
            return Ok(inner);
        }
        let var = self.ident("condition")?;
        let equal = match self.peek() {
            Tok::EqEq => true,
            Tok::NotEq => false,
            _ => return Ok(Expr::Var(var)),
        };
        self.bump();
        let value = self.string("string")?;
        Ok(Expr::Compare { var, value, equal })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) const SAMPLE: &str = r#"
// merge_pr.rein
description "Decide whether a pull request can be merged."

input docs_only: bool "Does the PR change only documentation files?"
input ci: choice("green", "red", "pending") "What is the CI status of the PR?"

if docs_only {
    return "approved"
}

switch ci {
    case "green" { return @review_code }
    case "red" { return @ci/fix }
    default { return "wait for CI, then run merge_pr again" }
}
"#;

    #[test]
    fn parses_full_sample() {
        let (f, diags) = parse(SAMPLE);
        assert!(diags.is_empty(), "{diags:?}");
        assert_eq!(f.description.unwrap().value, "Decide whether a pull request can be merged.");
        assert_eq!(f.inputs.len(), 2);
        assert_eq!(f.inputs[0].ty, InputType::Bool);
        let opts = f.inputs[1].ty.options().unwrap();
        assert_eq!(opts, ["green", "red", "pending"]);
        assert_eq!(f.body.len(), 2);
        assert!(matches!(&f.body[0], Stmt::If { otherwise: None, .. }));
        let Stmt::Switch { cases, default, .. } = &f.body[1] else { panic!("expected switch") };
        assert_eq!(cases.len(), 2);
        assert!(matches!(&cases[1].body[0], Stmt::Return(r) if r.value == ReturnValue::Ref("ci/fix".into())));
        assert!(default.is_some());
    }

    #[test]
    fn parses_else_if_logic_and_escapes() {
        let src = r#"description "Escapes: \"quoted\" and \\ and \n and ünïcode"
input a: bool "a"
input b: bool "b"
input c: choice("x", "y",) "c"
if not (a and b) or c == "x" { return "one" }
else if a { return "two" }
else { return "three" }
"#;
        let (f, diags) = parse(src);
        assert!(diags.is_empty(), "{diags:?}");
        assert_eq!(f.description.unwrap().value, "Escapes: \"quoted\" and \\ and \n and ünïcode");
        assert_eq!(f.inputs.len(), 3);
        let Stmt::If { cond, otherwise: Some(else_branch), .. } = &f.body[0] else {
            panic!("expected if")
        };
        assert!(matches!(cond, Expr::Or(..)));
        assert!(matches!(&else_branch[0], Stmt::If { otherwise: Some(_), .. }));
    }

    #[test]
    fn return_without_value_is_one_error_with_partial_tree() {
        let (f, diags) = parse("description \"x\"\ninput a: bool \"a\"\nreturn");
        assert_eq!(diags.len(), 1, "{diags:?}");
        assert!(diags[0].message.contains("expected string or `@function`"), "{}", diags[0].message);
        assert!(f.description.is_some());
        assert_eq!(f.inputs.len(), 1);
        assert!(f.body.is_empty());
    }

    #[test]
    fn default_must_be_last_switch_branch() {
        let (_, diags) = parse(
            "description \"d\"\ninput c: choice(\"x\", \"y\") \"c\"\nswitch c { default { return \"a\" } case \"x\" { return \"b\" } }",
        );
        assert_eq!(diags.len(), 1, "{diags:?}");
        assert!(
            diags[0].message.contains("`default` must be the last branch of a switch"),
            "{}",
            diags[0].message
        );
    }

    #[test]
    fn lex_error_gives_one_error_and_no_tree() {
        let (f, diags) = parse("description \"abc");
        assert_eq!(diags.len(), 1);
        assert!(f.description.is_none());
    }
}
