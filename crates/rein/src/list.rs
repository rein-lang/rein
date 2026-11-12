//! `rein list`: the visible programs, listed the way `ls` lists files. Operands are name globs. Without `-a`,
//! shadowed programs are hidden. Exit 2 when an operand matches no program, as `ls` does.

use std::cmp::Reverse;
use std::fs;
use std::io::{self, IsTerminal};
use std::time::{SystemTime, UNIX_EPOCH};

use clap::Args;
use glob::Pattern;
use rein_core::flows::Flow;
use serde_json::Value;

use crate::programs;
use crate::scopes;

const USAGE: u8 = 2;

/// Flags and operands of `rein list`. Each flag means what its `ls` counterpart means, where one applies.
#[derive(Args)]
pub struct ListArgs {
    /// Print the full records as JSON. Honors the operands, -a, -r and -t. -l and -1 do not apply.
    #[arg(long)]
    json: bool,
    /// Long format: one program per line, with scope, version, name, path and description.
    #[arg(short = 'l')]
    long: bool,
    /// One name per line, even on a terminal.
    #[arg(short = '1')]
    one: bool,
    /// Also show shadowed programs, marked with the scope that wins the name.
    #[arg(short = 'a', long)]
    all: bool,
    /// Reverse the order.
    #[arg(short = 'r', long)]
    reverse: bool,
    /// Sort by the modification time of each program's entry file, newest first. Default: by name.
    #[arg(short = 't')]
    time: bool,
    /// Show only the programs that these names match. Shell-style globs such as `pr-*`. Default: every program.
    patterns: Vec<String>,
}

/// Prints the programs the operands name, in `ls` order. Returns 2 when an operand matches no program.
pub fn run(args: &ListArgs) -> u8 {
    let found = scopes::discover_here();
    let visible: Vec<&Flow> = found.iter().filter(|f| args.all || f.shadowed_by.is_none()).collect();
    let (mut chosen, matched) = select(&visible, &args.patterns);
    order(&mut chosen, args);
    print_programs(&chosen, args);
    if matched {
        0
    } else {
        USAGE
    }
}

/// The visible programs that the operands match, in their own order. With no operands, all of them. Each operand
/// that matches none is reported on stderr. The flag is false when any operand failed. A shadowed program has the
/// name of its visible winner, so an operand that matches only a hidden program cannot happen.
fn select<'a>(visible: &[&'a Flow], operands: &[String]) -> (Vec<&'a Flow>, bool) {
    if operands.is_empty() {
        return (visible.to_vec(), true);
    }
    let mut globs = Vec::new();
    let mut all_matched = true;
    for text in operands {
        match Pattern::new(text) {
            Ok(glob) if visible.iter().any(|f| glob.matches(&f.name)) => globs.push(glob),
            Ok(_) => {
                all_matched = false;
                eprintln!("error: no program matches `{text}`");
            }
            Err(e) => {
                all_matched = false;
                eprintln!("error: bad pattern `{text}`: {e}");
            }
        }
    }
    let chosen = visible.iter().copied().filter(|f| globs.iter().any(|g| g.matches(&f.name))).collect();
    (chosen, all_matched)
}

/// Orders the programs as `ls` does: by name, or with `-t` by the entry file's modification time, newest first.
/// Equal names keep their precedence order. `-r` reverses the result.
fn order(chosen: &mut [&Flow], args: &ListArgs) {
    if args.time {
        chosen.sort_by_cached_key(|f| (Reverse(mtime(f)), f.name.clone()));
    } else {
        chosen.sort_by_cached_key(|f| f.name.clone());
    }
    if args.reverse {
        chosen.reverse();
    }
}

/// The modification time of the program's entry file. The epoch when it cannot be read, so such programs sort last.
fn mtime(f: &Flow) -> SystemTime {
    let file = f.root.join(format!("{}.rein", f.entry));
    fs::metadata(file).and_then(|m| m.modified()).unwrap_or(UNIX_EPOCH)
}

/// Prints the chosen programs: JSON records, `-l` rows, names, or names in columns on a terminal.
fn print_programs(chosen: &[&Flow], args: &ListArgs) {
    if args.json {
        let records: Vec<Value> = chosen.iter().map(|f| programs::with_first_step(f, programs::record(f))).collect();
        println!("{}", Value::Array(records));
        return;
    }
    let lines = if args.long {
        long_lines(chosen)
    } else {
        let names: Vec<String> = chosen.iter().map(|f| name_cell(f)).collect();
        if args.one || !io::stdout().is_terminal() {
            names
        } else {
            column_lines(&names)
        }
    };
    for line in lines {
        println!("{line}");
    }
}

/// The name, and the shadow mark when the program is shadowed (only shown with -a).
fn name_cell(f: &Flow) -> String {
    match f.shadowed_by {
        Some(by) => format!("{} (shadowed by {})", f.name, by.label()),
        None => f.name.clone(),
    }
}

/// `-l` rows: scope, version, name and path, each column padded to its widest cell. Then the description and
/// the marks. The version is `-` when the package has none.
fn long_lines(chosen: &[&Flow]) -> Vec<String> {
    let rows: Vec<[String; 4]> = chosen
        .iter()
        .map(|f| {
            [
                f.scope.label().to_string(),
                f.version.clone().unwrap_or_else(|| "-".to_string()),
                f.name.clone(),
                f.root.display().to_string(),
            ]
        })
        .collect();
    let mut width = [0; 4];
    for row in &rows {
        for (w, cell) in width.iter_mut().zip(row) {
            *w = (*w).max(cell.chars().count());
        }
    }
    rows.iter()
        .zip(chosen)
        .map(|(row, f)| {
            let cells: Vec<String> = row.iter().zip(width).map(|(c, w)| format!("{c:<w$}")).collect();
            format!("{}  {}{}", cells.join("  "), f.description, marks(f)).trim_end().to_string()
        })
        .collect()
}

/// The shadow and warning marks that `-l` puts after the description.
fn marks(f: &Flow) -> String {
    let shadow = f.shadowed_by.map(|by| format!(" (shadowed by {})", by.label())).unwrap_or_default();
    let warning = programs::warning(f).map(|w| format!(" (warning: {w})")).unwrap_or_default();
    format!("{shadow}{warning}")
}

/// Names in columns for a terminal, filled down each column as `ls` fills them.
// ponytail: fixed 80-column width, since std has no terminal size query. Add a crate when a wider terminal matters.
fn column_lines(names: &[String]) -> Vec<String> {
    let width = names.iter().map(|n| n.chars().count()).max().unwrap_or(0) + 2;
    let cols = (80 / width).max(1);
    let rows = names.len().div_ceil(cols);
    (0..rows)
        .map(|r| {
            let line: String = (0..cols)
                .filter_map(|c| names.get(c * rows + r))
                .map(|n| format!("{n:<width$}"))
                .collect();
            line.trim_end().to_string()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::column_lines;

    #[test]
    fn columns_fill_down_then_across() {
        let names: Vec<String> = ["a", "b", "c", "d", "e"].iter().map(|s| s.to_string()).collect();
        // Width 3 per cell, so 80 / 3 = 26 columns fit: one row of five names.
        assert_eq!(column_lines(&names), vec!["a  b  c  d  e"]);
        let long: Vec<String> = (0..60).map(|i| format!("p{i:02}-long-name")).collect();
        let lines = column_lines(&long);
        assert!(lines.iter().all(|l| l.chars().count() <= 80), "{lines:?}");
        assert!(lines[0].starts_with("p00-long-name"), "{lines:?}");
    }
}
