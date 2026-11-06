//! Rendering of command results as text or as one JSON document, and the
//! messages the tool writes while it works.

use std::fmt;
use std::io::Write;

use anyhow::{Context, Result};
use serde::Serialize;

use crate::redact::redacted;

/// Writes a message line to stdout, as `println!` does, except that a
/// stdout that is gone, a closed pipe or a terminal that hung up, loses the
/// message instead of ending the process, so that a run still ends what it
/// started, such as a pause of the merge queue.
macro_rules! say {
    ($($arg:tt)*) => {
        $crate::output::line(&mut ::std::io::stdout(), format_args!($($arg)*))
    };
}

/// Writes a message line to stderr, as [`say`] does to stdout.
macro_rules! esay {
    ($($arg:tt)*) => {
        $crate::output::line(&mut ::std::io::stderr(), format_args!($($arg)*))
    };
}

pub(crate) use {esay, say};

pub fn line(out: &mut impl Write, args: fmt::Arguments) {
    let _ = writeln!(out, "{args}");
}

/// A command result that has a text form; its JSON form is its serialisation.
pub trait Render: Serialize {
    fn render(&self) -> String;
}

/// Prints a result in the requested form, through [`redacted`] as every
/// message the tool shows.
pub fn emit<T: Render>(value: &T, json: bool) -> Result<()> {
    let mut text = if json {
        redacted(&serde_json::to_string_pretty(value)?)
    } else {
        redacted(&value.render())
    };
    if !text.ends_with('\n') {
        text.push('\n');
    }
    print(&text)
}

/// Writes a command's result to stdout. Unlike a message, a result that
/// cannot be written is an error, so that nothing reads a cut-off one as
/// whole.
pub fn print(text: &str) -> Result<()> {
    let mut out = std::io::stdout().lock();
    out.write_all(text.as_bytes())
        .and_then(|()| out.flush())
        .context("cannot write the result to stdout")
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
