//! Development tasks of git-intake that stay out of the shipped binary.
//!
//! `cargo run --locked -p xtask -- man <dir>` writes the manual pages
//! `git-intake.1` and `git-intake-<command>.1` into `<dir>`, generated from
//! the same command-line definition the binary is built from.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use clap::CommandFactory;
use clap_mangen::Man;
use clap_mangen::roff::{Roff, roman};

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
    man.render_synopsis_section(&mut page)?;
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
        man.render(&mut page)?;
        write(&dir.join(man.get_filename()), &page)?;
    }
    Ok(())
}

fn write(path: &Path, page: &[u8]) -> Result<()> {
    fs::write(path, page).with_context(|| format!("cannot write {}", path.display()))?;
    println!("wrote {}", path.display());
    Ok(())
}
