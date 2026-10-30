//! Rendering of command results as text or as one JSON document.

use anyhow::Result;
use serde::Serialize;

use crate::redact::redacted;

/// A command result that has a text form; its JSON form is its serialisation.
pub trait Render: Serialize {
    fn render(&self) -> String;
}

/// Prints a result in the requested form, through [`redacted`] as every
/// message the tool shows.
pub fn emit<T: Render>(value: &T, json: bool) -> Result<()> {
    if json {
        println!("{}", redacted(&serde_json::to_string_pretty(value)?));
    } else {
        let text = redacted(&value.render());
        print!("{text}");
        if !text.ends_with('\n') {
            println!();
        }
    }
    Ok(())
}

/// Lays out rows as left-aligned columns separated by two spaces.
pub fn table(header: &[&str], rows: &[Vec<String>]) -> String {
    let mut widths: Vec<usize> = header.iter().map(|h| h.len()).collect();
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            widths[i] = widths[i].max(cell.chars().count());
        }
    }
    let mut out = String::new();
    let mut line = |cells: Vec<&str>| {
        let mut l = String::new();
        for (i, cell) in cells.iter().enumerate() {
            if i + 1 == cells.len() {
                l.push_str(cell);
            } else {
                l.push_str(&format!("{cell:<w$}  ", w = widths[i]));
            }
        }
        out.push_str(l.trim_end());
        out.push('\n');
    };
    line(header.to_vec());
    for row in rows {
        line(row.iter().map(String::as_str).collect());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_aligns_columns() {
        let t = table(
            &["A", "BB"],
            &[vec!["xxx".into(), "y".into()], vec!["z".into(), "".into()]],
        );
        assert_eq!(t, "A    BB\nxxx  y\nz\n");
    }
}
