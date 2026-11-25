use super::*;

const TICKET: &str = r#"{ "title": "Ticket", "type": "string", "pattern": "^[A-Z]+-[0-9]+$", "maxLength": 20 }"#;

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
    assert_eq!(s.schema()["maxLength"], 20);
}

#[test]
fn a_schema_must_be_a_string_type() {
    let e = load_err(r#"{ "title": "Ticket", "type": "integer" }"#);
    assert!(e.contains("`\"type\": \"string\"`"), "{e}");
    let e = load_err(r#"{ "title": "Ticket" }"#);
    assert!(e.contains("`\"type\": \"string\"`"), "{e}");
}

#[test]
fn a_schema_needs_a_title() {
    let e = load_err(r#"{ "type": "string" }"#);
    assert!(e.contains("`title`"), "{e}");
}

#[test]
fn a_schema_title_must_be_a_name() {
    for title in ["", "Ticket Type", "a/b", "1Ticket", "for"] {
        let e = load_err(&format!(r#"{{ "title": "{title}", "type": "string" }}"#));
        assert!(e.contains("`title` must be a name such as `Ticket`"), "{title}: {e}");
    }
    let ok = UserSchema::load(r#"{ "title": "_Ticket2", "type": "string" }"#).unwrap();
    assert_eq!(ok.title, "_Ticket2");
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

fn ticket_ref(loaded: Option<Result<Arc<UserSchema>, String>>) -> FormatType {
    FormatType::Schema(SchemaRef {
        name: Spanned {
            value: "Ticket".into(),
            span: Default::default(),
        },
        path: "./ticket.schema".into(),
        loaded,
    })
}

#[test]
fn json_schema_of_a_schema_type_not_loaded_is_none() {
    assert_eq!(ticket_ref(None).json_schema(), None);
}

#[test]
fn json_schema_of_a_schema_type_that_failed_to_load_is_none() {
    let failed = ticket_ref(Some(Err("cannot read `./ticket.schema`".into())));
    assert_eq!(failed.json_schema(), None);
}

#[test]
fn json_schema_of_a_loaded_schema_type_is_the_file_schema() {
    // A file text no other fixture uses, so the expected value below cannot come from another schema.
    const ORDER: &str = r#"{ "title": "Order", "type": "string", "pattern": "^ORD-[0-9]{4}$", "minLength": 8, "maxLength": 8 }"#;
    let loaded = ticket_ref(Some(Ok(Arc::new(UserSchema::load(ORDER).unwrap()))));
    assert_eq!(
        loaded.json_schema(),
        Some(json!({"title": "Order", "type": "string", "pattern": "^ORD-[0-9]{4}$", "minLength": 8, "maxLength": 8}))
    );
}
