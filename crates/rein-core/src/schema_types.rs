//! Schema types: string types that a user defines with a JSON Schema file, such as `ticket.schema.json` for
//! `import { Ticket } from "./ticket.schema";`. A schema type goes where `string` goes. The checker treats it as a
//! `string`; the run checks each value against the schema, with the `jsonschema` crate.
//! A built-in format type and a schema type are both [`FormatType`]: the rest of the code asks one type for its name,
//! its article, its JSON name and its check.

use std::sync::Arc;

use jsonschema::Validator;
use serde_json::Value;

use crate::ast::Spanned;
use crate::formats::Format;

/// An import path of a schema file ends with this, such as `./ticket.schema` for `ticket.schema.json`.
pub const SCHEMA_SUFFIX: &str = ".schema";
/// The file of a schema type: the import path plus this, such as `ticket.schema.json`.
pub const SCHEMA_FILE_EXT: &str = ".schema.json";

/// A JSON Schema file that defines a string type. Its `title` is the type name the import should use. This PR does
/// not check that the two match.
pub struct UserSchema {
    /// The name the schema declares, such as `Ticket`.
    pub title: String,
    schema: Value,
    validator: Validator,
}

impl UserSchema {
    /// Reads the text of a schema file. The schema must be a string type with a `title`.
    pub fn load(text: &str) -> Result<UserSchema, String> {
        let schema: Value = serde_json::from_str(text).map_err(|e| format!("cannot read schema: {e}"))?;
        let Some(object) = schema.as_object() else {
            return Err("a schema type is a JSON object".into());
        };
        if object.get("type") != Some(&Value::String("string".into())) {
            return Err("a schema type must have `\"type\": \"string\"`".into());
        }
        let Some(title) = object.get("title").and_then(Value::as_str) else {
            return Err("a schema type needs a string `title`, such as `\"title\": \"Ticket\"`".into());
        };
        let validator = jsonschema::options()
            .should_validate_formats(true)
            .build(&schema)
            .map_err(|e| format!("invalid schema: {e}"))?;
        Ok(UserSchema {
            title: title.to_owned(),
            schema,
            validator,
        })
    }

    /// Checks that `raw`, the value of `name`, matches the schema. Returns the first error when it does not.
    pub fn check(&self, name: &str, raw: &str) -> Result<(), String> {
        match self.validator.iter_errors(&Value::String(raw.to_owned())).next() {
            None => Ok(()),
            Some(e) => Err(format!("`{name}` does not match `{}`: {e}", self.title)),
        }
    }

    /// The schema as the file wrote it.
    pub fn schema(&self) -> &Value {
        &self.schema
    }
}

/// A use of a schema import as a type. `loaded` is set when the project loads: `None` until then.
#[derive(Clone)]
pub struct SchemaRef {
    /// The local name as written, such as `Ticket`.
    pub name: Spanned<String>,
    /// The import path, such as `./ticket.schema`.
    pub path: String,
    /// The loaded schema, or why it does not load. `None` until the project binds it.
    pub loaded: Option<Result<Arc<UserSchema>, String>>,
}

impl std::fmt::Debug for SchemaRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SchemaRef")
            .field("name", &self.name.value)
            .field("path", &self.path)
            .finish()
    }
}

impl PartialEq for SchemaRef {
    fn eq(&self, other: &Self) -> bool {
        self.name.value == other.name.value && self.path == other.path
    }
}

/// A string type with a fixed shape: a built-in format, or a schema type.
#[derive(Debug, Clone, PartialEq)]
pub enum FormatType {
    Builtin(Format),
    Schema(SchemaRef),
}

impl FormatType {
    /// The type name as a program writes it, such as `IRI` or `Ticket`.
    pub fn name(&self) -> String {
        match self {
            FormatType::Builtin(f) => f.name().to_string(),
            FormatType::Schema(r) => r.name.value.clone(),
        }
    }

    /// The article before the type name in a message: `an` for `IRI`, `a` for `Ticket`. A schema type takes `an`
    /// when its name starts with a vowel letter.
    pub fn article(&self) -> &'static str {
        match self {
            FormatType::Builtin(f) => f.article(),
            FormatType::Schema(r) => match r.name.value.chars().next() {
                Some('A' | 'E' | 'I' | 'O' | 'U' | 'a' | 'e' | 'i' | 'o' | 'u') => "an",
                _ => "a",
            },
        }
    }

    /// The type name in JSON, such as the `type` of a question or a field. A schema type is a `string`.
    pub fn json_name(&self) -> &str {
        match self {
            FormatType::Builtin(f) => f.json_name(),
            FormatType::Schema(_) => "string",
        }
    }

    /// Checks that `raw`, the value of `name`, has this type. Returns the error text when it does not.
    pub fn check(&self, name: &str, raw: &str) -> Result<(), String> {
        match self {
            FormatType::Builtin(f) => f.check(name, raw),
            FormatType::Schema(r) => match &r.loaded {
                None => Err(format!("schema type `{}` is not loaded", r.name.value)),
                Some(Err(msg)) => Err(msg.clone()),
                Some(Ok(s)) => s.check(name, raw),
            },
        }
    }
}

#[cfg(test)]
#[path = "schema_types_tests.rs"]
mod tests;
