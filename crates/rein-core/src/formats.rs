//! Format types: string types with a fixed shape, such as `Iri`. A program uses one where it would use `string`.
//! The checker treats a format type as a `string`; the run checks each value against the format.
//!
//! The list of formats comes from the jsonschema crate: `ValidationOptions::known_formats` under draft 2020-12.
//! Rein keeps no list of its own. Each JSON Schema format name gives its type name by one rule: split the name
//! on `-`, upper-case the first letter of each part, and join the parts. So `iri` is `Iri`, `date-time` is
//! `DateTime`, and `relative-json-pointer` is `RelativeJsonPointer`. A format that a later jsonschema release
//! adds appears in rein once rein moves its `jsonschema` pin to that release. No rein code changes for it.

use std::sync::OnceLock;

use jsonschema::{Draft, Validator};
use serde_json::{Value, json};

/// A built-in format type: an index into the formats that the jsonschema crate knows. It is `Copy`, like a tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Format(usize);

/// One built-in format: its JSON Schema name, its type name, and the compiled check.
struct Entry {
    json: String,
    name: String,
    validator: Validator,
}

impl Format {
    /// Every built-in format, in the order that the jsonschema crate sorts their JSON names.
    #[cfg(test)]
    pub fn all() -> impl Iterator<Item = Format> {
        (0..entries().len()).map(Format)
    }

    /// The format that a program writes as `name`, such as `Iri`.
    pub fn from_name(name: &str) -> Option<Format> {
        entries().iter().position(|e| e.name == name).map(Format)
    }

    /// The type name as a program writes it, such as `Iri`.
    pub fn name(self) -> &'static str {
        &entries()[self.0].name
    }

    /// The JSON Schema format name, such as `iri`.
    pub fn json_name(self) -> &'static str {
        &entries()[self.0].json
    }

    /// Checks that `raw`, the value of `name`, has this format. Returns the error text when it does not.
    pub fn check(self, name: &str, raw: &str) -> Result<(), String> {
        let entry = &entries()[self.0];
        if entry.validator.is_valid(&Value::String(raw.to_owned())) {
            return Ok(());
        }
        Err(format!(
            "`{name}` must match format `{}`; got `{raw}`",
            entry.json
        ))
    }
}

/// The formats of the jsonschema crate under draft 2020-12, built once per process.
fn entries() -> &'static [Entry] {
    static ENTRIES: OnceLock<Vec<Entry>> = OnceLock::new();
    ENTRIES.get_or_init(|| {
        jsonschema::options()
            .known_formats(Draft::Draft202012)
            .into_iter()
            .map(|json| Entry {
                json: json.to_owned(),
                name: type_name(json),
                validator: compile(json),
            })
            .collect()
    })
}

/// The type name of a JSON Schema format name: each part of the name split on `-` has its first letter upper-cased.
fn type_name(json: &str) -> String {
    json.split('-')
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect(),
                None => String::new(),
            }
        })
        .collect()
}

/// Compiles `{"type": "string", "format": ...}` with format validation on.
fn compile(format: &str) -> Validator {
    let schema = json!({"type": "string", "format": format});
    jsonschema::options()
        .with_draft(Draft::Draft202012)
        .should_validate_formats(true)
        .build(&schema)
        .expect("a string schema with a known format compiles")
}

#[cfg(test)]
#[path = "formats_tests.rs"]
mod tests;
