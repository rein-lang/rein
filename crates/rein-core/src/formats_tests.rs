use super::*;

#[test]
fn iri_is_found_by_its_name() {
    assert_eq!(Format::from_name("IRI"), Some(Format::Iri));
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
        assert_eq!(Format::Iri.check("link", raw), Ok(()), "{raw}");
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
        let err = Format::Iri.check("link", raw).unwrap_err();
        assert!(
            err.starts_with("`link` must be an absolute IRI"),
            "{raw}: {err}"
        );
    }
}

#[test]
fn every_format_round_trips_by_name_and_names_are_unique() {
    for f in Format::ALL {
        assert_eq!(Format::from_name(f.name()), Some(f), "{}", f.name());
    }
    let mut names: Vec<&str> = Format::ALL.iter().map(|f| f.name()).collect();
    let mut json_names: Vec<&str> = Format::ALL.iter().map(|f| f.json_name()).collect();
    names.sort_unstable();
    json_names.sort_unstable();
    let count = Format::ALL.len();
    names.dedup();
    json_names.dedup();
    assert_eq!(names.len(), count);
    assert_eq!(json_names.len(), count);
}

/// A value that the format must reject. A regex accepts plain text, so it gets an unbalanced group.
fn invalid_value(format: Format) -> &'static str {
    match format {
        Format::Regex => "(",
        _ => "not valid !!",
    }
}

#[test]
fn every_format_rejects_an_invalid_value() {
    for f in Format::ALL {
        let raw = invalid_value(f);
        let err = f.check("x", raw).unwrap_err();
        let prefix = format!("`x` must be {}", f.expected());
        assert!(err.starts_with(&prefix), "{}: {err}", f.name());
    }
}

#[test]
fn email_accepts_an_address_and_rejects_a_bare_name() {
    assert_eq!(Format::Email.check("mail", "someone@example.com"), Ok(()));
    let err = Format::Email.check("mail", "someone").unwrap_err();
    assert!(err.starts_with("`mail` must be an email address"), "{err}");
}

#[test]
fn uuid_accepts_a_uuid_and_rejects_a_short_hex_string() {
    assert_eq!(
        Format::Uuid.check("id", "123e4567-e89b-12d3-a456-426614174000"),
        Ok(())
    );
    let err = Format::Uuid.check("id", "123e4567").unwrap_err();
    assert!(err.starts_with("`id` must be a UUID"), "{err}");
}

#[test]
fn date_accepts_a_real_date_and_rejects_an_impossible_one() {
    assert_eq!(Format::Date.check("day", "2026-11-23"), Ok(()));
    let err = Format::Date.check("day", "2026-02-30").unwrap_err();
    assert!(err.starts_with("`day` must be a date"), "{err}");
}

#[test]
fn date_time_accepts_an_offset_and_rejects_a_missing_one() {
    assert_eq!(Format::DateTime.check("at", "2026-11-23T13:45:00Z"), Ok(()));
    let err = Format::DateTime
        .check("at", "2026-11-23T13:45:00")
        .unwrap_err();
    assert!(err.starts_with("`at` must be a date and time"), "{err}");
}

#[test]
fn ipv4_accepts_a_dotted_quad_and_rejects_an_out_of_range_octet() {
    assert_eq!(Format::Ipv4.check("addr", "192.0.2.1"), Ok(()));
    let err = Format::Ipv4.check("addr", "192.0.2.256").unwrap_err();
    assert!(err.starts_with("`addr` must be an IPv4 address"), "{err}");
}

#[test]
fn hostname_accepts_a_domain_and_rejects_a_space() {
    assert_eq!(Format::Hostname.check("host", "example.com"), Ok(()));
    let err = Format::Hostname.check("host", "exa mple.com").unwrap_err();
    assert!(err.starts_with("`host` must be a hostname"), "{err}");
}
