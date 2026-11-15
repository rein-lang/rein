//! The programs a package may hand off to. As pnpm gives a package only the packages its `package.json` declares,
//! a program with a `package.json` reaches only the programs it declares there. A folder without a `package.json` is
//! not a package, so it has no such limit.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use serde::Deserialize;

use crate::flows::PACKAGE;

/// The declared dependencies of one package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dependencies {
    /// The package name, else the folder name. Error texts name it.
    pub package: String,
    /// Every name in `dependencies`, `devDependencies`, `optionalDependencies` and `peerDependencies`.
    pub names: BTreeSet<String>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PackageJson {
    name: Option<String>,
    dependencies: Option<BTreeMap<String, serde_json::Value>>,
    dev_dependencies: Option<BTreeMap<String, serde_json::Value>>,
    optional_dependencies: Option<BTreeMap<String, serde_json::Value>>,
    peer_dependencies: Option<BTreeMap<String, serde_json::Value>>,
}

/// The declared dependencies of the package at `root`, or `None` when `root` has no `package.json`. A
/// `package.json` that does not parse declares nothing, so every program reference in it fails.
pub fn read(root: &Path) -> Option<Dependencies> {
    let text = fs::read_to_string(root.join(PACKAGE)).ok()?;
    let package: PackageJson = serde_json::from_str(&text).unwrap_or_default();
    let folder = root.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    let names = [package.dependencies, package.dev_dependencies, package.optional_dependencies, package.peer_dependencies]
        .into_iter()
        .flatten()
        .flat_map(BTreeMap::into_keys)
        .collect();
    Some(Dependencies { package: package.name.unwrap_or_else(|| folder.to_string()), names })
}

impl Dependencies {
    /// Why this package may not hand off to `program`, or `None` when it declares it.
    pub fn undeclared(&self, program: &str) -> Option<String> {
        if self.names.contains(program) {
            return None;
        }
        let package = &self.package;
        Some(format!(
            "program `{program}` is not a dependency of `{package}`; add it to `dependencies` in package.json, \
             such as with `pnpm add {program}`"
        ))
    }
}

#[cfg(test)]
#[path = "dependencies_tests.rs"]
mod tests;
