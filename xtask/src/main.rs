//! Development tasks of git-intake that stay out of the shipped binary.
//!
//! `cargo run --locked -p xtask -- man <dir>` writes the manual pages
//! `git-intake.1` and `git-intake-<command>.1` into `<dir>`, generated from
//! the same command-line definition the binary is built from.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use clap::{Arg, ArgAction, Command, CommandFactory};
use clap_mangen::Man;
use clap_mangen::roff::{Inline, Roff, bold, italic, roman};

// The binary's command-line definition, which depends on clap alone.
#[path = "../../src/cli.rs"]
#[allow(dead_code)]
mod cli;

/// The manifest of the git-intake package, whose version the page carries.
const MANIFEST: &str = include_str!("../../Cargo.toml");

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [task, dir] if task == "man" => man(Path::new(dir)),
        _ => bail!("usage: cargo run -p xtask -- man <dir>"),
    }
}

/// Writes `git-intake.1`, and one page per command such as
/// `git-intake-finish.1`, into `dir`.
fn man(dir: &Path) -> Result<()> {
    let manifest: toml::Table = toml::from_str(MANIFEST).context("cannot parse Cargo.toml")?;
    let version = manifest
        .get("package")
        .and_then(|p| p.get("version"))
        .and_then(|v| v.as_str())
        .context("Cargo.toml has no package version")?
        .to_string();
    // The definition's `version` attribute expands in this crate; the pages
    // carry the version of the binary instead.
    let version: &'static str = Box::leak(version.into_boxed_str());
    let mut cmd = cli::Cli::command()
        .version(version)
        .disable_help_subcommand(true);
    cmd.build();
    fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;

    let man = Man::new(cmd.clone());
    let mut page = Vec::new();
    man.render_title(&mut page)?;
    man.render_name_section(&mut page)?;
    synopsis(&cmd).to_writer(&mut page)?;
    man.render_description_section(&mut page)?;
    man.render_options_section(&mut page)?;
    man.render_subcommands_section(&mut page)?;
    if let Some(text) = cmd.get_after_long_help() {
        let text = text.to_string();
        let text = text.strip_prefix("Exit status: ").unwrap_or(&text);
        let mut roff = Roff::new();
        roff.control("SH", ["EXIT STATUS"]);
        roff.text([roman(text)]);
        roff.to_writer(&mut page)?;
    }
    man.render_version_section(&mut page)?;
    write(&dir.join(man.get_filename()), &page)?;

    for sub in cmd.get_subcommands().filter(|s| !s.is_hide_set()) {
        let man = Man::new(sub.clone()).source(format!("git-intake {version}"));
        let mut page = Vec::new();
        man.render_title(&mut page)?;
        man.render_name_section(&mut page)?;
        synopsis(sub).to_writer(&mut page)?;
        man.render_description_section(&mut page)?;
        man.render_options_section(&mut page)?;
        if sub.get_after_long_help().is_some() || sub.get_after_help().is_some() {
            man.render_extra_section(&mut page)?;
        }
        if sub.get_version().is_some() || sub.get_long_version().is_some() {
            man.render_version_section(&mut page)?;
        }
        write(&dir.join(man.get_filename()), &page)?;
    }
    Ok(())
}

/// The SYNOPSIS section of a page, in the notation of clap's usage line, but
/// with every option spelled out instead of `[OPTIONS]`: an option keeps its
/// value name (`--only <NAME>`), a repeatable option or argument is followed
/// by `...`, and the arguments after `--` keep it (`[-- <COMMAND>...]`).
fn synopsis(cmd: &Command) -> Roff {
    let name = cmd.get_bin_name().unwrap_or_else(|| cmd.get_name());
    let mut line = vec![bold(name)];
    for arg in cmd
        .get_arguments()
        .filter(|a| !a.is_hide_set() && !a.is_positional())
    {
        line.push(roman(" "));
        line.extend(option(arg));
    }
    for arg in cmd.get_positionals().filter(|a| !a.is_hide_set()) {
        line.push(roman(" "));
        line.extend(positional(arg));
    }
    if cmd.has_subcommands() {
        let value = cmd.get_subcommand_value_name().unwrap_or("COMMAND");
        line.extend([roman(" <"), italic(value), roman(">")]);
    }
    let mut roff = Roff::new();
    roff.control("SH", ["SYNOPSIS"]);
    roff.text(line);
    roff
}

/// `[-h|--help]`, `[--json]` or `[--only <NAME>]...`.
fn option(arg: &Arg) -> Vec<Inline> {
    let required = arg.is_required_set();
    let mut out = vec![roman(if required { "" } else { "[" })];
    match (arg.get_short(), arg.get_long()) {
        (Some(short), Some(long)) => {
            out.extend([
                bold(format!("-{short}")),
                roman("|"),
                bold(format!("--{long}")),
            ]);
        }
        (Some(short), None) => out.push(bold(format!("-{short}"))),
        (None, Some(long)) => out.push(bold(format!("--{long}"))),
        (None, None) => {}
    }
    if arg.get_action().takes_values() {
        out.extend([roman(" <"), italic(value_name(arg)), roman(">")]);
    }
    out.push(roman(if required { "" } else { "]" }));
    if repeats(arg) {
        out.push(roman("..."));
    }
    out
}

/// `<BRANCH>`, `[COMMIT]`, `[UPSTREAM[=REV]]...` or `[-- <COMMAND>...]`.
fn positional(arg: &Arg) -> Vec<Inline> {
    let required = arg.is_required_set();
    let value = italic(value_name(arg));
    let dots = if repeats(arg) { "..." } else { "" };
    if arg.is_last_set() {
        let (open, close) = if required {
            ("-- <", ">")
        } else {
            ("[-- <", ">")
        };
        let close = format!("{close}{dots}{}", if required { "" } else { "]" });
        return vec![roman(open), value, roman(close)];
    }
    let (open, close) = if required { ("<", ">") } else { ("[", "]") };
    vec![roman(open), value, roman(format!("{close}{dots}"))]
}

fn value_name(arg: &Arg) -> String {
    arg.get_value_names()
        .map(|names| names.join(" "))
        .unwrap_or_else(|| arg.get_id().as_str().to_uppercase())
}

/// Whether the argument can be given more than once.
fn repeats(arg: &Arg) -> bool {
    matches!(arg.get_action(), ArgAction::Append | ArgAction::Count)
        || arg.get_num_args().is_some_and(|n| n.max_values() > 1)
}

fn write(path: &Path, page: &[u8]) -> Result<()> {
    fs::write(path, page).with_context(|| format!("cannot write {}", path.display()))?;
    println!("wrote {}", path.display());
    Ok(())
}
