//! Format types: string types with a fixed shape, such as `IRI`. A program uses one where it would use `string`.
//! The checker treats a format type as a `string`; the run checks each value against the format.
//! The checks are the built-in `format` keywords of the jsonschema crate, so no format logic lives here.
//! A new built-in format needs four edits: a variant in `Format`, an entry in `Format::ALL`, an entry in `Format::spec`, and an arm in `Format::placeholder`.
//! The compiler checks the variant, the `spec` entry and the `placeholder` arm. Nothing checks the `Format::ALL` entry.
//!
//! The jsonschema crate keeps its list of formats private, so `Format` repeats it by hand for now.
//! TODO: replace `Format` and `Format::spec` with `ValidationOptions::known_formats` once it is released:
//! https://github.com/Stranger6667/jsonschema/pull/2032 (issue https://github.com/Stranger6667/jsonschema/issues/2031).

use std::sync::OnceLock;

use jsonschema::Validator;
use serde_json::{Value, json};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Iri,
    IriReference,
    Uri,
    UriReference,
    UriTemplate,
    Email,
    IdnEmail,
    Hostname,
    IdnHostname,
    Ipv4,
    Ipv6,
    Date,
    Time,
    DateTime,
    Duration,
    Uuid,
    JsonPointer,
    RelativeJsonPointer,
    Regex,
}

impl Format {
    /// Every built-in format, in the order the docs list them.
    pub const ALL: [Format; 19] = [
        Format::Iri,
        Format::IriReference,
        Format::Uri,
        Format::UriReference,
        Format::UriTemplate,
        Format::Email,
        Format::IdnEmail,
        Format::Hostname,
        Format::IdnHostname,
        Format::Ipv4,
        Format::Ipv6,
        Format::Date,
        Format::Time,
        Format::DateTime,
        Format::Duration,
        Format::Uuid,
        Format::JsonPointer,
        Format::RelativeJsonPointer,
        Format::Regex,
    ];

    /// The format that a program writes as `name`, such as `IRI`.
    pub fn from_name(name: &str) -> Option<Format> {
        Format::ALL.into_iter().find(|f| f.name() == name)
    }

    /// The type name as a program writes it.
    pub fn name(self) -> &'static str {
        self.spec().0
    }

    /// The type name in JSON, such as the `type` of a question or a field.
    pub fn json_name(self) -> &'static str {
        self.spec().1
    }

    /// The indefinite article for the type name, by its first sound: `an IRI`, `a URI`, `an Email`.
    pub fn article(self) -> &'static str {
        match self {
            Format::Iri
            | Format::IriReference
            | Format::Email
            | Format::IdnEmail
            | Format::IdnHostname
            | Format::Ipv4
            | Format::Ipv6 => "an",
            _ => "a",
        }
    }

    /// A valid value that stands in for a real one when a walk has no value, such as `rein graph`.
    pub fn placeholder(self, name: &str) -> String {
        match self {
            Format::Iri
            | Format::IriReference
            | Format::Uri
            | Format::UriReference
            | Format::UriTemplate => format!("urn:rein:{name}"),
            Format::Email | Format::IdnEmail => "someone@example.com".to_owned(),
            Format::Hostname | Format::IdnHostname => "example.com".to_owned(),
            Format::Ipv4 => "192.0.2.1".to_owned(),
            Format::Ipv6 => "2001:db8::1".to_owned(),
            Format::Date => "2026-11-23".to_owned(),
            Format::Time => "13:45:00Z".to_owned(),
            Format::DateTime => "2026-11-23T13:45:00Z".to_owned(),
            Format::Duration => "P1DT2H".to_owned(),
            Format::Uuid => "123e4567-e89b-12d3-a456-426614174000".to_owned(),
            Format::JsonPointer => "/items/0".to_owned(),
            Format::RelativeJsonPointer => "1/items".to_owned(),
            Format::Regex => "^[a-z]+$".to_owned(),
        }
    }

    /// Checks that `raw`, the value of `name`, has this format. Returns the error text when it does not.
    pub fn check(self, name: &str, raw: &str) -> Result<(), String> {
        if self.validator().is_valid(&Value::String(raw.to_owned())) {
            return Ok(());
        }
        Err(format!("`{name}` must be {}; got `{raw}`", self.expected()))
    }

    /// The rein name, the JSON Schema format name, and a short description with an example.
    fn spec(self) -> (&'static str, &'static str, &'static str) {
        match self {
            Format::Iri => (
                "IRI",
                "iri",
                "an absolute IRI, like `https://example.com/page`",
            ),
            Format::IriReference => (
                "IRIReference",
                "iri-reference",
                "an IRI reference, like `/docs/page`",
            ),
            Format::Uri => (
                "URI",
                "uri",
                "an absolute URI, like `https://example.com/page`",
            ),
            Format::UriReference => (
                "URIReference",
                "uri-reference",
                "a URI reference, like `/docs/page`",
            ),
            Format::UriTemplate => (
                "URITemplate",
                "uri-template",
                "a URI template, like `https://example.com/items/{id}`",
            ),
            Format::Email => (
                "Email",
                "email",
                "an email address, like `someone@example.com`",
            ),
            Format::IdnEmail => (
                "IdnEmail",
                "idn-email",
                "an email address, like `someone@example.com`",
            ),
            Format::Hostname => ("Hostname", "hostname", "a hostname, like `example.com`"),
            Format::IdnHostname => (
                "IdnHostname",
                "idn-hostname",
                "a hostname, like `example.com`",
            ),
            Format::Ipv4 => ("IPv4", "ipv4", "an IPv4 address, like `192.0.2.1`"),
            Format::Ipv6 => ("IPv6", "ipv6", "an IPv6 address, like `2001:db8::1`"),
            Format::Date => ("Date", "date", "a date, like `2026-11-23`"),
            Format::Time => ("Time", "time", "a time with an offset, like `13:45:00Z`"),
            Format::DateTime => (
                "DateTime",
                "date-time",
                "a date and time, like `2026-11-23T13:45:00Z`",
            ),
            Format::Duration => (
                "Duration",
                "duration",
                "an ISO 8601 duration, like `P1DT2H`",
            ),
            Format::Uuid => (
                "UUID",
                "uuid",
                "a UUID, like `123e4567-e89b-12d3-a456-426614174000`",
            ),
            Format::JsonPointer => (
                "JSONPointer",
                "json-pointer",
                "a JSON Pointer, like `/items/0`",
            ),
            Format::RelativeJsonPointer => (
                "RelativeJSONPointer",
                "relative-json-pointer",
                "a relative JSON Pointer, like `1/items`",
            ),
            Format::Regex => ("Regex", "regex", "a regular expression, like `^[a-z]+$`"),
        }
    }

    fn expected(self) -> &'static str {
        self.spec().2
    }

    /// The compiled check for this format, built once per process.
    fn validator(self) -> &'static Validator {
        static VALIDATORS: OnceLock<Vec<(Format, Validator)>> = OnceLock::new();
        VALIDATORS
            .get_or_init(|| Format::ALL.into_iter().map(|f| (f, compile(f))).collect())
            .iter()
            .find(|(f, _)| *f == self)
            .map(|(_, v)| v)
            .expect("every format has a compiled check")
    }
}

/// Compiles `{"type": "string", "format": ...}` with format validation on.
fn compile(format: Format) -> Validator {
    let schema = json!({"type": "string", "format": format.json_name()});
    jsonschema::options()
        .should_validate_formats(true)
        .build(&schema)
        .expect("a string schema with a built-in format compiles")
}

#[cfg(test)]
#[path = "formats_tests.rs"]
mod tests;
