use super::*;

const TICKET: &str = r#"{ "title": "Ticket", "type": "string", "pattern": "^[A-Z]+-[0-9]+$", "maxLength": 20, "examples": ["ABC-123"] }"#;

fn load_err(text: &str) -> String {
    match UserSchema::load(text) {
        Err(e) => e,
        Ok(s) => panic!("loaded: {}", s.title),
    }
}

#[test]
fn loads_a_string_schema() {
    let s = UserSchema::load(TICKET).unwrap();
    assert_eq!(s.title, "Ticket");
    assert_eq!(s.example, "ABC-123");
    assert_eq!(s.schema()["maxLength"], 20);
}

#[test]
fn a_schema_must_be_a_string_type() {
    let e = load_err(r#"{ "title": "Ticket", "type": "integer", "examples": ["1"] }"#);
    assert!(e.contains("`\"type\": \"string\"`"), "{e}");
    let e = load_err(r#"{ "title": "Ticket", "examples": ["ABC-1"] }"#);
    assert!(e.contains("`\"type\": \"string\"`"), "{e}");
}

#[test]
fn a_schema_needs_a_title() {
    let e = load_err(r#"{ "type": "string", "examples": ["ABC-1"] }"#);
    assert!(e.contains("`title`"), "{e}");
}

#[test]
fn a_schema_needs_an_example_string() {
    let e = load_err(r#"{ "title": "Ticket", "type": "string" }"#);
    assert!(e.contains("`examples`"), "{e}");
    let e = load_err(r#"{ "title": "Ticket", "type": "string", "examples": [] }"#);
    assert!(e.contains("`examples`"), "{e}");
    let e = load_err(r#"{ "title": "Ticket", "type": "string", "examples": [1] }"#);
    assert!(e.contains("`examples`"), "{e}");
}

#[test]
fn the_example_must_pass_the_schema() {
    let e = load_err(r#"{ "title": "Ticket", "type": "string", "pattern": "^[A-Z]+$", "examples": ["abc"] }"#);
    assert!(e.contains("does not match the schema"), "{e}");
}

#[test]
fn a_schema_must_be_an_object_and_valid_json() {
    assert!(load_err("[1]").contains("JSON object"));
    assert!(load_err("{").contains("cannot read schema"));
}

#[test]
fn check_accepts_a_matching_value() {
    let s = UserSchema::load(TICKET).unwrap();
    assert_eq!(s.check("ticket", "XY-9"), Ok(()));
}

#[test]
fn check_names_the_title_and_the_error() {
    let s = UserSchema::load(TICKET).unwrap();
    let e = s.check("ticket", "abc").unwrap_err();
    assert!(e.starts_with("`ticket` does not match `Ticket`: "), "{e}");
}

#[test]
fn a_schema_type_is_a_string_for_json() {
    let r = SchemaRef {
        name: Spanned {
            value: "Ticket".into(),
            span: Default::default(),
        },
        path: "./ticket.schema".into(),
        loaded: None,
    };
    let t = FormatType::Schema(r);
    assert_eq!(t.name(), "Ticket");
    assert_eq!(t.json_name(), "string");
    assert_eq!(
        t.check("ticket", "ABC-1"),
        Err("schema type `Ticket` is not loaded".into())
    );
}
