use super::*;

fn kinds(src: &str) -> Vec<Tok> {
    lex(src).unwrap().into_iter().map(|(t, _)| t).collect()
}

#[test]
fn keywords_and_reserved_words_are_not_names() {
    for kw in KEYWORDS.iter().chain(RESERVED.iter()) {
        let toks = lex(kw).unwrap();
        assert!(
            !matches!(toks[0].0, Tok::Ident(_)),
            "`{kw}` lexed as a name"
        );
    }
}

#[test]
fn old_words_are_plain_names() {
    assert_eq!(
        kinds("input choice description"),
        vec![
            Tok::Ident("input".into()),
            Tok::Ident("choice".into()),
            Tok::Ident("description".into()),
            Tok::Eof,
        ]
    );
}

#[test]
fn single_and_double_quotes_with_escapes() {
    assert_eq!(
        kinds(r#""a\"b" 'c\'d\\e\nü'"#),
        vec![
            Tok::Str("a\"b".into()),
            Tok::Str("c'd\\e\nü".into()),
            Tok::Eof,
        ]
    );
}

#[test]
fn operators_and_punctuation() {
    assert_eq!(
        kinds("=== !== || && | ! ; ( ) { } , :"),
        vec![
            Tok::EqEqEq,
            Tok::NotEqEq,
            Tok::OrOr,
            Tok::AndAnd,
            Tok::Pipe,
            Tok::Bang,
            Tok::Semi,
            Tok::LParen,
            Tok::RParen,
            Tok::LBrace,
            Tok::RBrace,
            Tok::Comma,
            Tok::Colon,
            Tok::Eof,
        ]
    );
}

#[test]
fn double_equals_is_an_error() {
    let err = lex("a == \"x\"").unwrap_err();
    assert_eq!(err.message, "use `===` instead of `==`");
    assert_eq!(err.span, Span::new(2, 4));
    let err = lex("a != \"x\"").unwrap_err();
    assert_eq!(err.message, "use `!==` instead of `!=`");
}

#[test]
fn doc_block_keeps_raw_inner_text() {
    let toks = lex("/**\n * Hi.\n */ export").unwrap();
    assert_eq!(toks[0].0, Tok::Doc("\n * Hi.\n ".into()));
    assert_eq!(toks[0].1, Span::new(0, 14));
    assert_eq!(toks[1].0, Tok::Export);
}

#[test]
fn empty_doc_block() {
    let toks = lex("/**/").unwrap();
    assert_eq!(toks[0].0, Tok::Doc(String::new()));
    assert_eq!(toks[0].1, Span::new(0, 4));
    let toks = lex("/***/").unwrap();
    assert_eq!(toks[0].0, Tok::Doc(String::new()));
}

#[test]
fn leading_bom_is_whitespace_and_spans_stay_byte_offsets() {
    let toks = lex("\u{feff}/** d */").unwrap();
    assert_eq!(toks[0].0, Tok::Doc(" d ".into()));
    assert_eq!(toks[0].1, Span::new(3, 11));
}

#[test]
fn unclosed_string_is_an_error() {
    let err = lex("\"abc").unwrap_err();
    assert!(err.message.contains("unclosed string"), "{}", err.message);
}

#[test]
fn unterminated_doc_is_an_error() {
    let err = lex("/** abc").unwrap_err();
    assert!(
        err.message.contains("unterminated doc comment"),
        "{}",
        err.message
    );
}

#[test]
fn plain_block_comment_is_rejected() {
    let err = lex("/* x */").unwrap_err();
    assert_eq!(
        err.message,
        "use `//` for comments or `/** */` for the doc block"
    );
    assert_eq!(err.span, Span::new(0, 2));
}

#[test]
fn at_sign_needs_a_relative_path() {
    let err = lex("return @x").unwrap_err();
    assert_eq!(err.message, "references are relative: write `@./name`");
}

#[test]
fn template_literal_splits_text_and_names() {
    let toks = lex("`a ${b} c`").unwrap();
    let Tok::Template(pieces) = &toks[0].0 else {
        panic!("{:?}", toks[0].0)
    };
    assert_eq!(pieces.len(), 3);
    assert_eq!(pieces[0], Piece::Text("a ".into()));
    assert!(matches!(&pieces[1], Piece::Name(n) if n.value == "b" && n.span == Span::new(5, 6)));
    assert_eq!(pieces[2], Piece::Text(" c".into()));
}

#[test]
fn template_literal_escapes_and_raw_newlines() {
    let toks = lex("`a\\n\\`b\\\\ ${c}\nd`").unwrap();
    let Tok::Template(pieces) = &toks[0].0 else {
        panic!("{:?}", toks[0].0)
    };
    assert_eq!(pieces[0], Piece::Text("a\n`b\\ ".into()));
    assert!(matches!(&pieces[1], Piece::Name(n) if n.value == "c"));
    assert_eq!(pieces[2], Piece::Text("\nd".into()));
}

#[test]
fn template_literal_only_takes_a_name() {
    let err = lex("`${a.b}`").unwrap_err();
    assert_eq!(
        err.message,
        "only a parameter name can go in `${}`; write `${name}`"
    );
    let err = lex("`${}`").unwrap_err();
    assert_eq!(
        err.message,
        "only a parameter name can go in `${}`; write `${name}`"
    );
}

#[test]
fn unclosed_template_literal_is_an_error() {
    let err = lex("`abc").unwrap_err();
    assert!(
        err.message.contains("unclosed template literal"),
        "{}",
        err.message
    );
}
