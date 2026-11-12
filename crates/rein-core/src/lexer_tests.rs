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
fn double_equals_is_the_same_as_triple() {
    assert_eq!(kinds("== != === !=="), vec![Tok::EqEqEq, Tok::NotEqEq, Tok::EqEqEq, Tok::NotEqEq, Tok::Eof]);
    let toks = lex("a == \"x\"").unwrap();
    assert_eq!(toks[1], (Tok::EqEqEq, Span::new(2, 4)));
    assert_eq!(lex("a != \"x\"").unwrap()[1].0, Tok::NotEqEq);
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
fn bare_at_name_is_a_program_reference() {
    let toks = lex("@pr-handling @pr-handling/main @./x").unwrap();
    let refs: Vec<&Tok> = toks.iter().map(|(t, _)| t).collect();
    assert_eq!(refs[0], &Tok::Ref("pr-handling".into()));
    assert_eq!(refs[1], &Tok::Ref("pr-handling/main".into()));
    assert_eq!(refs[2], &Tok::Ref("./x".into()));
}

#[test]
fn bad_program_reference_is_an_error() {
    for src in ["@", "@-x", "@x/", "@x//y", "@x/a-b"] {
        let err = lex(src).unwrap_err();
        assert!(err.message.contains("@program/function"), "{src}: {}", err.message);
    }
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

/// The text of a template with no `${}`.
fn template_text(src: &str) -> String {
    let toks = lex(src).unwrap();
    let Tok::Template(pieces) = &toks[0].0 else {
        panic!("{:?}", toks[0].0)
    };
    pieces
        .iter()
        .map(|p| match p {
            Piece::Text(t) => t.clone(),
            Piece::Name(n) => panic!("unexpected name {}", n.value),
        })
        .collect()
}

#[test]
fn template_dedents_the_indent_shared_by_its_lines() {
    assert_eq!(template_text("`Read the log.\n    Fix the test.`"), "Read the log.\nFix the test.");
    // Extra indent past the shared one stays, so a list keeps its nesting.
    assert_eq!(template_text("`a\n    b\n      c`"), "a\nb\n  c");
    // Blank lines do not count toward the shared indent.
    assert_eq!(template_text("`a\n    b\n\n    c`"), "a\nb\n\nc");
    // A line with no indent means no indent is dropped.
    assert_eq!(template_text("`a\n  b\nc`"), "a\n  b\nc");
    // The first line is not indented, and a template with no raw newline is unchanged.
    assert_eq!(template_text("`  a  `"), "  a  ");
    // An escaped `\n` is not a raw newline, so it does not trigger the dedent.
    assert_eq!(template_text("`a\\n    b`"), "a\n    b");
}

#[test]
fn template_dedent_keeps_interpolations() {
    let toks = lex("`a\n    ${b}\n    c`").unwrap();
    let Tok::Template(pieces) = &toks[0].0 else {
        panic!("{:?}", toks[0].0)
    };
    assert_eq!(pieces[0], Piece::Text("a\n".into()));
    assert!(matches!(&pieces[1], Piece::Name(n) if n.value == "b"));
    assert_eq!(pieces[2], Piece::Text("\nc".into()));
}

#[test]
fn a_name_may_read_one_field_of_a_struct_answer() {
    let toks = lex("pick.pr").unwrap();
    assert_eq!(toks[0].0, Tok::Ident("pick.pr".into()));
    let toks = lex("`${pick.pr}`").unwrap();
    let Tok::Template(pieces) = &toks[0].0 else { panic!("{:?}", toks[0].0) };
    assert!(matches!(&pieces[0], Piece::Name(n) if n.value == "pick.pr"));
}

#[test]
fn template_literal_only_takes_a_name() {
    let err = lex("`${a-b}`").unwrap_err();
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
