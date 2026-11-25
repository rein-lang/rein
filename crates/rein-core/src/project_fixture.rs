//! Test helpers for a project on disk. A file on disk is read the way a real run reads it, so a `.schema.json` or a
//! `.struct.rein` file next to the `.rein` files loads. The step and import tests share these helpers.

use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::project::Project;

static NEXT: AtomicUsize = AtomicUsize::new(0);

/// A schema file for a string type named `Ticket`: a `pattern`, a `maxLength` and `examples`.
pub const TICKET_SCHEMA: &str = "{ \"title\": \"Ticket\", \"type\": \"string\", \"pattern\": \"^[A-Z]+-[0-9]+$\", \"maxLength\": 20, \"examples\": [\"ABC-123\"] }";

/// Writes `files` into a unique temp project, loads it, and removes the directory.
pub fn project(files: &[(&str, &str)]) -> Project {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let root =
        std::env::temp_dir().join(format!("rein-project-{}-{nanos}-{n}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    for (rel, text) in files {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    let loaded = Project::load(&root).unwrap();
    fs::remove_dir_all(&root).unwrap();
    loaded
}
