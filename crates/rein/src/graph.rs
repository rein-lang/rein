//! `rein graph`: prints the program graph as Mermaid, DOT, or JSON, or renders it with Graphviz.

use std::fs;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use clap::ValueEnum;
use rein_core::graph;
use rein_core::project::{Project, ENTRY};

use crate::scopes;

#[derive(Clone, Copy, ValueEnum)]
pub enum GraphFormat {
    Mermaid,
    Dot,
    Json,
}

pub fn run(program: Option<String>, root: Option<PathBuf>, refs: bool, format: GraphFormat, output: Option<&Path>) -> u8 {
    match write(program, root, refs, format, output) {
        Ok(()) => 0,
        Err(message) => {
            eprintln!("error: {message}");
            1
        }
    }
}

fn write(program: Option<String>, root: Option<PathBuf>, refs: bool, format: GraphFormat, output: Option<&Path>) -> Result<(), String> {
    let (root, entry) = match program {
        Some(p) => scopes::locate(&p)?,
        None => (root.unwrap_or_else(|| Project::find_root(Path::new("."))), ENTRY.to_string()),
    };
    let project = scopes::load_linked(&root).map_err(|e| format!("cannot read {}: {e}", root.display()))?;
    let g = if refs { graph::refs(&project, &entry) } else { graph::flow(&project, &entry) };
    let image = output.and_then(|p| p.extension()).and_then(|e| e.to_str()).filter(|e| matches!(*e, "svg" | "png"));
    if let (Some(path), Some(ext)) = (output, image) {
        return render(&graph::dot(&g), ext, path);
    }
    let text = match format {
        GraphFormat::Mermaid => graph::mermaid(&g),
        GraphFormat::Dot => graph::dot(&g),
        GraphFormat::Json => serde_json::to_string_pretty(&g).unwrap_or_default() + "\n",
    };
    match output {
        Some(path) => fs::write(path, text).map_err(|e| format!("cannot write {}: {e}", path.display())),
        None => {
            print!("{text}");
            Ok(())
        }
    }
}

/// Renders DOT source to an image with Graphviz `dot`.
fn render(source: &str, ext: &str, path: &Path) -> Result<(), String> {
    let mut child = Command::new("dot")
        .arg(format!("-T{ext}"))
        .arg("-o")
        .arg(path)
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|e| match e.kind() {
            ErrorKind::NotFound => format!(
                "writing {} needs Graphviz: `dot` is not on PATH. Install Graphviz, or write a .mmd, .dot or .json file",
                path.display()
            ),
            _ => format!("cannot run `dot`: {e}"),
        })?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(source.as_bytes()).map_err(|e| format!("cannot write to `dot`: {e}"))?;
    }
    let status = child.wait().map_err(|e| format!("cannot run `dot`: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("`dot` failed ({status}) while writing {}", path.display()))
    }
}
