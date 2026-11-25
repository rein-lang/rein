use super::*;

/// The format with this type name. The test fails when the library does not know it.
fn named(name: &str) -> Format {
    Format::from_name(name).unwrap_or_else(|| panic!("no format named `{name}`"))
}

#[test]
fn type_names_follow_the_rule() {
    assert_eq!(type_name("iri"), "Iri");
    assert_eq!(type_name("date-time"), "DateTime");
    assert_eq!(type_name("relative-json-pointer"), "RelativeJsonPointer");
    assert_eq!(type_name("ipv4"), "Ipv4");
    assert_eq!(type_name("idn-email"), "IdnEmail");
    assert_eq!(type_name("uuid"), "Uuid");
}

#[test]
fn names_are_found_by_type_name_only() {
    assert_eq!(Format::from_name("Iri").map(Format::json_name), Some("iri"));
    assert_eq!(Format::from_name("IRI"), None);
    assert_eq!(Format::from_name("iri"), None);
    assert_eq!(Format::from_name("string"), None);
}

#[test]
fn iri_accepts_absolute_iris_and_uris() {
    for raw in [
        "https://example.com/a?b=c#d",
        "urn:isbn:0451450523",
        "mailto:someone@example.com",
        "https://例え.jp/パス",
        "file:///tmp/x",
    ] {
        assert_eq!(named("Iri").check("link", raw), Ok(()), "{raw}");
    }
}

#[test]
fn iri_rejects_relative_and_malformed_values() {
    for raw in [
        "",
        "example.com",
        "/a/b",
        "not a link",
        "https://exa mple.com",
        "http://[::1",
    ] {
        let err = named("Iri").check("link", raw).unwrap_err();
        assert!(
            err.starts_with("`link` must match format `iri`"),
            "{raw}: {err}"
        );
    }
}

#[test]
fn every_format_round_trips_by_name_and_names_are_unique() {
    let formats: Vec<Format> = Format::all().collect();
    assert!(!formats.is_empty());
    for f in &formats {
        assert_eq!(Format::from_name(f.name()), Some(*f), "{}", f.name());
    }
    let mut names: Vec<&str> = formats.iter().map(|f| f.name()).collect();
    let mut json_names: Vec<&str> = formats.iter().map(|f| f.json_name()).collect();
    names.sort_unstable();
    json_names.sort_unstable();
    let count = formats.len();
    names.dedup();
    json_names.dedup();
    assert_eq!(names.len(), count);
    assert_eq!(json_names.len(), count);
}

/// A value that the format must reject. A regex accepts plain text, so it gets an unbalanced group.
fn invalid_value(format: Format) -> &'static str {
    match format.json_name() {
        "regex" => "(",
        _ => "not valid !!",
    }
}

#[test]
fn every_format_rejects_an_invalid_value() {
    for f in Format::all() {
        let raw = invalid_value(f);
        let err = f.check("x", raw).unwrap_err();
        let prefix = format!("`x` must match format `{}`", f.json_name());
        assert!(err.starts_with(&prefix), "{}: {err}", f.name());
    }
}

#[test]
fn email_accepts_an_address_and_rejects_a_bare_name() {
    assert_eq!(named("Email").check("mail", "someone@example.com"), Ok(()));
    let err = named("Email").check("mail", "someone").unwrap_err();
    assert!(err.starts_with("`mail` must match format `email`"), "{err}");
}

#[test]
fn uuid_accepts_a_uuid_and_rejects_a_short_hex_string() {
    assert_eq!(
        named("Uuid").check("id", "123e4567-e89b-12d3-a456-426614174000"),
        Ok(())
    );
    let err = named("Uuid").check("id", "123e4567").unwrap_err();
    assert!(err.starts_with("`id` must match format `uuid`"), "{err}");
}

#[test]
fn date_accepts_a_real_date_and_rejects_an_impossible_one() {
    assert_eq!(named("Date").check("day", "2026-11-23"), Ok(()));
    let err = named("Date").check("day", "2026-02-30").unwrap_err();
    assert!(err.starts_with("`day` must match format `date`"), "{err}");
}

#[test]
fn date_time_accepts_an_offset_and_rejects_a_missing_one() {
    assert_eq!(
        named("DateTime").check("at", "2026-11-23T13:45:00Z"),
        Ok(())
    );
    let err = named("DateTime")
        .check("at", "2026-11-23T13:45:00")
        .unwrap_err();
    assert!(
        err.starts_with("`at` must match format `date-time`"),
        "{err}"
    );
}

#[test]
fn ipv4_accepts_a_dotted_quad_and_rejects_an_out_of_range_octet() {
    assert_eq!(named("Ipv4").check("addr", "192.0.2.1"), Ok(()));
    let err = named("Ipv4").check("addr", "192.0.2.256").unwrap_err();
    assert!(err.starts_with("`addr` must match format `ipv4`"), "{err}");
}

#[test]
fn hostname_accepts_a_domain_and_rejects_a_space() {
    assert_eq!(named("Hostname").check("host", "example.com"), Ok(()));
    let err = named("Hostname").check("host", "exa mple.com").unwrap_err();
    assert!(
        err.starts_with("`host` must match format `hostname`"),
        "{err}"
    );
}
