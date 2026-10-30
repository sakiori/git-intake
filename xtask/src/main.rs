//! Development tasks of git-intake that stay out of the shipped binary.
//!
//! `cargo run --locked -p xtask -- man <dir>` writes the manual pages
//! `git-intake.1` and `git-intake-<command>.1` into `<dir>`, generated from
//! the same command-line definition the binary is built from.
//!
//! `cargo run --locked -p xtask -- notices <file>` writes the third-party
//! notices of the crates the binary is built from into `<file>`, with
//! cargo-about, `about.toml` and the template `about.hbs`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command as Process;

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
        [task, file] if task == "notices" => notices(Path::new(file)),
        _ => bail!("usage: cargo run -p xtask -- man <dir> | notices <file>"),
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

/// The cargo-about release the notices are generated with; CI installs the
/// same one.
const CARGO_ABOUT_VERSION: &str = "0.9.2";

/// Writes the third-party notices into `file`: the license texts cargo-about
/// finds for every crate of the release targets' dependency graph, followed by
/// the NOTICE files of those crates, which the Apache License asks a
/// redistribution to carry and cargo-about does not collect. Fails when a
/// crate's license cannot be read or is not accepted, and on any warning, such
/// as a clarification in `about.toml` whose file no longer matches.
fn notices(file: &Path) -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .context("xtask is not inside the workspace")?;
    let file =
        std::path::absolute(file).with_context(|| format!("cannot resolve {}", file.display()))?;
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());

    let version = Process::new(&cargo)
        .args(["about", "--version"])
        .output()
        .context("cannot run cargo-about")?;
    let version = String::from_utf8_lossy(&version.stdout);
    if version.split_whitespace().nth(1) != Some(CARGO_ABOUT_VERSION) {
        bail!(
            "the notices are generated with cargo-about {CARGO_ABOUT_VERSION}, not '{}'; install it \
             with `cargo install --locked --features cli cargo-about@{CARGO_ABOUT_VERSION}`",
            version.trim()
        );
    }

    let about = |args: &[&str]| -> Result<Vec<u8>> {
        let out = Process::new(&cargo)
            .current_dir(root)
            .args([
                "about",
                "generate",
                "--locked",
                "--fail",
                "--config",
                "about.toml",
            ])
            .args(args)
            .output()
            .context("cannot run cargo-about")?;
        let stderr = String::from_utf8_lossy(&out.stderr);
        eprint!("{stderr}");
        if !out.status.success() {
            bail!("cargo-about failed with {}", out.status);
        }
        if stderr.contains("WARN") || stderr.contains("ERROR") {
            bail!("cargo-about warned; the notices would be incomplete");
        }
        Ok(out.stdout)
    };

    let mut text =
        String::from_utf8(about(&["about.hbs"])?).context("the notices are not UTF-8")?;
    let graph: serde_json::Value = serde_json::from_slice(&about(&["--format", "json"])?)
        .context("cannot parse cargo-about's JSON output")?;
    for krate in graph["crates"]
        .as_array()
        .context("cargo-about listed no crates")?
    {
        let package = &krate["package"];
        let (Some(name), Some(version), Some(manifest)) = (
            package["name"].as_str(),
            package["version"].as_str(),
            package["manifest_path"].as_str(),
        ) else {
            bail!("cargo-about listed a crate without name, version or manifest path");
        };
        for notice in notice_files(Path::new(manifest).parent().unwrap_or(root))? {
            let body = fs::read_to_string(&notice)
                .with_context(|| format!("cannot read {}", notice.display()))?;
            let file_name = notice.file_name().unwrap_or_default().to_string_lossy();
            text.push_str(&format!(
                "\n{}\n{file_name} of {name} {version}\n\n{}\n",
                "-".repeat(80),
                body.trim_end()
            ));
        }
    }

    if let Some(dir) = file.parent() {
        fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
    }
    write(&file, text.as_bytes())
}

/// The files at a crate's root whose name starts with `NOTICE`, in any letter
/// case, sorted by name.
fn notice_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for entry in fs::read_dir(dir).with_context(|| format!("cannot list {}", dir.display()))? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_ascii_uppercase();
        if name.starts_with("NOTICE") && entry.file_type()?.is_file() {
            files.push(entry.path());
        }
    }
    files.sort();
    Ok(files)
}
