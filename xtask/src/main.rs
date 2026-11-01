//! Development tasks of git-intake that stay out of the shipped binary.
//!
//! `cargo run --locked -p xtask -- man <dir>` writes the manual pages
//! `git-intake.1` and `git-intake-<command>.1` into `<dir>`, generated from
//! the same command-line definition the binary is built from.
//!
//! `cargo run --locked -p xtask -- notices <dir> <target>` writes the
//! third-party notices of the release binary for `<target>` into `<dir>`:
//! `THIRD-PARTY-NOTICES.txt`, with the notices of the crates it is built from
//! (cargo-about, `about.toml` and the template `about.hbs`), of the Rust
//! toolchain's compiler-builtins and, for a Linux (musl) target, of the Zig
//! distribution that links it and of musl libc, and
//! `RUST-LIBRARY-COPYRIGHT.html`, the toolchain's notices of the Rust standard
//! library.
//!
//! `cargo run --locked -p xtask -- pack <dir> <archive> <epoch>` packs the
//! files under `<dir>` as the gzip-compressed tar archive `<archive>`,
//! deterministically, with every modification time `<epoch>`; `check-archive
//! <archive> <epoch> [<host path>...]` checks such an archive.

use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
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
        [task, dir, target] if task == "notices" => notices(Path::new(dir), target),
        [task, dir, archive, epoch] if task == "pack" => pack(
            Path::new(dir),
            Path::new(archive),
            epoch
                .parse()
                .context("the epoch is not a number of seconds")?,
        ),
        [task, archive, epoch, forbidden @ ..] if task == "check-archive" => check_archive(
            Path::new(archive),
            epoch
                .parse()
                .context("the epoch is not a number of seconds")?,
            forbidden,
        ),
        _ => bail!(
            "usage: cargo run -p xtask -- man <dir> | notices <dir> <target> | \
             pack <dir> <archive> <epoch> | check-archive <archive> <epoch> [<host path>...]"
        ),
    }
}

/// Writes `git-intake.1`, and one page per command such as
/// `git-intake-finish.1` or `git-intake-release-next.1`, into `dir`.
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
        command_pages(dir, sub, version)?;
    }
    Ok(())
}

/// Writes the page of a command, such as `git-intake-release.1`, and one
/// page per command of a group, such as `git-intake-release-next.1`.
fn command_pages(dir: &Path, cmd: &Command, version: &str) -> Result<()> {
    let man = Man::new(cmd.clone()).source(format!("git-intake {version}"));
    let mut page = Vec::new();
    man.render_title(&mut page)?;
    man.render_name_section(&mut page)?;
    synopsis(cmd).to_writer(&mut page)?;
    man.render_description_section(&mut page)?;
    man.render_options_section(&mut page)?;
    if cmd.has_subcommands() {
        man.render_subcommands_section(&mut page)?;
    }
    if cmd.get_after_long_help().is_some() || cmd.get_after_help().is_some() {
        man.render_extra_section(&mut page)?;
    }
    if cmd.get_version().is_some() || cmd.get_long_version().is_some() {
        man.render_version_section(&mut page)?;
    }
    write(&dir.join(man.get_filename()), &page)?;
    for sub in cmd.get_subcommands().filter(|s| !s.is_hide_set()) {
        command_pages(dir, sub, version)?;
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

/// The notices file and the copy of the toolchain's notices of the standard
/// library that `notices` writes.
const NOTICES: &str = "THIRD-PARTY-NOTICES.txt";
const RUST_LIBRARY_COPYRIGHT: &str = "RUST-LIBRARY-COPYRIGHT.html";

/// Writes the third-party notices of the binary for `target` into `dir`:
///
/// - the license texts cargo-about finds for every crate of the target's
///   dependency graph, followed by the NOTICE files of those crates, which
///   the Apache License asks a redistribution to carry and cargo-about does
///   not collect;
/// - the license of compiler-builtins and its libm, which the Rust toolchain
///   links into every binary, from the toolchain's rust-src component, and a
///   copy of the toolchain's COPYRIGHT-library.html, the notices of the
///   standard library and the crates it is built from;
/// - for a Linux (musl) target, the license of the Zig distribution that
///   cargo-zigbuild links the binary with and musl's COPYRIGHT, from that zig
///   installation. Apple's toolchain links a macOS target against the
///   system's libraries, which the binary does not carry, so a macOS target
///   needs neither a Zig notice nor a zig installation.
///
/// Fails when a crate's license cannot be read or is not accepted, on any
/// warning of cargo-about, such as a clarification in `about.toml` whose file
/// no longer matches, and when a source file of a clarified crate carries a
/// copyright line the notices lack.
fn notices(dir: &Path, target: &str) -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .context("xtask is not inside the workspace")?;
    let dir =
        std::path::absolute(dir).with_context(|| format!("cannot resolve {}", dir.display()))?;
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
            .args(["about", "generate", "--locked", "--fail"])
            .args(["--config", "about.toml", "--target", target])
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
    let mut crates = Vec::new();
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
        let source = Path::new(manifest).parent().unwrap_or(root).to_path_buf();
        for notice in notice_files(&source)? {
            let body = fs::read_to_string(&notice)
                .with_context(|| format!("cannot read {}", notice.display()))?;
            let file_name = notice.file_name().unwrap_or_default().to_string_lossy();
            text.push_str(&format!(
                "\n{}\n{file_name} of {name} {version}\n\n{}\n",
                "-".repeat(80),
                body.trim_end()
            ));
        }
        // The workspace's own crate is the subject of the notices, not a part
        // of them; its license ships beside them.
        if package["source"].is_string() {
            crates.push((name.to_string(), version.to_string(), source));
        }
    }
    let additional = additional_notices(&graph, &crates)?;
    text.push_str(&additional);
    check_copyright_lines(&crates, &text)?;

    let sysroot = rust_sysroot()?;
    text.push_str(&rust_notices(&sysroot)?);
    text.push_str(&zig_notices(target)?);

    fs::create_dir_all(&dir).with_context(|| format!("cannot create {}", dir.display()))?;
    write(&dir.join(NOTICES), text.as_bytes())?;
    let library = sysroot.join("share/doc/rust/COPYRIGHT-library.html");
    let library = fs::read(&library).with_context(|| {
        format!(
            "cannot read {}, the Rust toolchain's notices of the standard library",
            library.display()
        )
    })?;
    write(&dir.join(RUST_LIBRARY_COPYRIGHT), &library)
}

/// The copyright lines of a crate's sources that the notices must carry, by
/// line, with the files that hold each, relative to the crate's root.
///
/// Every file of the crate's package counts, except what is not part of the
/// binary or not a notice of the crate's license:
/// - files under a directory named `tests`, `test`, `testdata`, `test-data`,
///   `benches`, `examples`, `fuzz` or starting with `.`: test data,
///   benchmarks, examples and repository metadata;
/// - files at the crate's root whose name starts with `LICENSE`, `LICENCE`,
///   `COPYING`, `UNLICENSE`, `NOTICE` or `COPYRIGHT`: the license texts, whose
///   notices the output carries when the license is the one it carries for the
///   crate, and which belong to an alternative license otherwise; NOTICE files
///   are carried whole.
fn copyright_lines(source: &Path) -> Result<BTreeMap<String, Vec<String>>> {
    const SKIPPED_DIRS: &[&str] = &[
        "tests",
        "test",
        "testdata",
        "test-data",
        "benches",
        "examples",
        "fuzz",
    ];
    const LICENSE_FILES: &[&str] = &[
        "LICENSE",
        "LICENCE",
        "COPYING",
        "UNLICENSE",
        "NOTICE",
        "COPYRIGHT",
    ];
    let mut lines: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for file in source_files(source)? {
        let path = file.strip_prefix(source).unwrap_or(&file);
        let parts: Vec<String> = path
            .components()
            .map(|c| c.as_os_str().to_string_lossy().to_string())
            .collect();
        let (dirs, name) = parts.split_at(parts.len() - 1);
        if dirs
            .iter()
            .any(|d| d.starts_with('.') || SKIPPED_DIRS.contains(&d.as_str()))
        {
            continue;
        }
        let upper = name[0].to_ascii_uppercase();
        if dirs.is_empty() && LICENSE_FILES.iter().any(|l| upper.starts_with(l)) {
            continue;
        }
        let Ok(content) = fs::read_to_string(&file) else {
            continue;
        };
        let shown = parts.join("/");
        for notice in copyright_notices(&content) {
            let files = lines.entry(notice).or_default();
            if !files.contains(&shown) {
                files.push(shown.clone());
            }
        }
    }
    Ok(lines)
}

/// The copyright notices of a file, each the copyright line with the lines
/// that continue its list of holders, without comment markers and joined by
/// newlines. A line continues the notice while the notice's last line ends
/// with `,`, `;`, `&`, `and` or `\\`, and it is not empty and not a copyright
/// line of its own, as in clap_derive's header, whose copyright line ends
/// with `,` and is followed by two more holders.
fn copyright_notices(content: &str) -> Vec<String> {
    let lines: Vec<&str> = content.lines().collect();
    let mut notices = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if !is_copyright_line(lines[i]) {
            i += 1;
            continue;
        }
        let mut notice = vec![normalized(lines[i])];
        i += 1;
        while i < lines.len() {
            let last = notice.last().map(String::as_str).unwrap_or_default();
            let continues = last.ends_with([',', ';', '&', '\\'])
                || last.ends_with(" and")
                || last.eq_ignore_ascii_case("and");
            let next = normalized(lines[i]);
            if !continues || next.is_empty() || is_copyright_line(lines[i]) {
                break;
            }
            notice.push(next);
            i += 1;
        }
        notices.push(notice.join("\n"));
    }
    notices
}

/// A notice's lines joined as `normalized` text joins the lines of the
/// notices, for a search in them.
fn flattened(notice: &str) -> String {
    notice.split('\n').collect::<Vec<_>>().join(" ")
}

/// The section of the notices that carries, for every crate, the copyright
/// lines of its sources that the license texts cargo-about found for it lack,
/// with the files that hold them; empty when there are none.
fn additional_notices(
    graph: &serde_json::Value,
    crates: &[(String, String, PathBuf)],
) -> Result<String> {
    // The license texts the notices carry for each crate.
    let mut texts: BTreeMap<(String, String), String> = BTreeMap::new();
    for license in graph["licenses"]
        .as_array()
        .context("cargo-about listed no licenses")?
    {
        let text = license["text"].as_str().unwrap_or_default();
        for user in license["used_by"].as_array().into_iter().flatten() {
            let krate = &user["crate"];
            let key = (
                krate["name"].as_str().unwrap_or_default().to_string(),
                krate["version"].as_str().unwrap_or_default().to_string(),
            );
            let entry = texts.entry(key).or_default();
            entry.push_str(&text.lines().map(normalized).collect::<Vec<_>>().join(" "));
            entry.push(' ');
        }
    }
    let mut section = String::new();
    for (name, version, source) in crates {
        let carried = texts
            .get(&(name.clone(), version.clone()))
            .map(String::as_str)
            .unwrap_or_default();
        let mut notices = notice_files(source)?
            .iter()
            .filter_map(|f| fs::read_to_string(f).ok())
            .map(|t| t.lines().map(normalized).collect::<Vec<_>>().join(" "))
            .collect::<Vec<_>>()
            .join(" ");
        notices.push_str(carried);
        let missing: Vec<_> = copyright_lines(source)?
            .into_iter()
            .filter(|(notice, _)| !notices.contains(&flattened(notice)))
            .collect();
        if missing.is_empty() {
            continue;
        }
        section.push_str(&format!("\n{name} {version}\n"));
        for (line, files) in missing {
            section.push('\n');
            for text in line.split('\n') {
                section.push_str(&format!("  {text}\n"));
            }
            section.push_str(&format!("    in {}\n", files.join(", ")));
        }
    }
    if section.is_empty() {
        return Ok(section);
    }
    Ok(format!(
        "\n{rule}\nAdditional copyright notices\n\n\
         Copyright notices in the sources of these crates that the license texts above\n\
         do not carry, with the files that hold them. The files are under the license\n\
         the notices name for their crate.\n{section}",
        rule = "-".repeat(80),
    ))
}

/// Fails unless the notices carry every copyright line of every crate's
/// sources that `copyright_lines` counts.
fn check_copyright_lines(crates: &[(String, String, PathBuf)], text: &str) -> Result<()> {
    let notices = text.lines().map(normalized).collect::<Vec<_>>().join(" ");
    let mut missing = Vec::new();
    for (name, version, source) in crates {
        for (line, files) in copyright_lines(source)? {
            if !notices.contains(&flattened(&line)) {
                missing.push(format!("{name} {version}, {}: {line}", files.join(", ")));
            }
        }
    }
    if !missing.is_empty() {
        bail!(
            "the notices lack copyright lines of the crates' sources:\n{}",
            missing.join("\n")
        );
    }
    Ok(())
}

/// Whether a line states a copyright notice. These forms count:
///
/// - `SPDX-FileCopyrightText`, anywhere in the line;
/// - the word `copyright`, in any letter case, followed by a year, `©`, `ⓒ`
///   or `(c)` in either case, as in clap's `Copyright ⓒ 2015-2016 …`;
/// - a line that starts, past its comment markers and an optional
///   `Portions` in any letter case, with `copyright` in any letter case, an
///   optional `:`, and a holder, a word that starts with a capital letter,
///   as in `Copyright The Rust Project Developers` or `copyright Example
///   Author.`, unless that word is one of license prose, such as
///   `COPYRIGHT HOLDERS` or `copyright notice`;
/// - a line that starts with `©` or `ⓒ`.
fn is_copyright_line(line: &str) -> bool {
    const PROSE: &[&str] = &[
        "notice",
        "notices",
        "holder",
        "holders",
        "owner",
        "owners",
        "license",
        "licenses",
        "law",
        "laws",
        "act",
        "statement",
        "and",
        "or",
    ];
    if line.contains("SPDX-FileCopyrightText") {
        return true;
    }
    let lower = line.to_lowercase();
    let symbol_or_year = |rest: &str| {
        let rest = rest.trim_start();
        rest.starts_with(|c: char| c.is_ascii_digit())
            || rest.starts_with("(c)")
            || rest.starts_with('©')
            || rest.starts_with('ⓒ')
    };
    let noticed = lower.match_indices("copyright").any(|(at, word)| {
        let before = lower[..at].chars().next_back();
        let after = lower[at + word.len()..].chars().next();
        !before.is_some_and(char::is_alphanumeric)
            && !after.is_some_and(char::is_alphanumeric)
            && symbol_or_year(&lower[at + word.len()..])
    });
    if noticed {
        return true;
    }
    let text = normalized(line);
    let text = strip_prefix_ignore_case(&text, "portions ").unwrap_or(&text);
    if text.starts_with('©') || text.starts_with('ⓒ') {
        return true;
    }
    let Some(rest) = strip_prefix_ignore_case(text, "copyright") else {
        return false;
    };
    if !rest.starts_with([' ', ':']) {
        return false;
    }
    let holder = rest.trim_start_matches([' ', ':']);
    let word: String = holder.chars().take_while(|c| c.is_alphanumeric()).collect();
    word.starts_with(char::is_uppercase) && !PROSE.contains(&word.to_lowercase().as_str())
}

/// `text` without the ASCII `prefix`, compared in ASCII case.
fn strip_prefix_ignore_case<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let head = text.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| &text[prefix.len()..])
}

/// A line without the comment markers before it, with its whitespace
/// collapsed.
fn normalized(line: &str) -> String {
    let line = line.trim_start_matches(|c: char| c.is_whitespace() || "/*#;!@-".contains(c));
    line.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Every file under `dir`, sorted by path.
fn source_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let mut dirs = vec![dir.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        for entry in fs::read_dir(&dir).with_context(|| format!("cannot list {}", dir.display()))? {
            let entry = entry?;
            let kind = entry.file_type()?;
            if kind.is_dir() {
                dirs.push(entry.path());
            } else if kind.is_file() {
                files.push(entry.path());
            }
        }
    }
    files.sort();
    Ok(files)
}

/// The sysroot of the Rust toolchain that builds the binary.
fn rust_sysroot() -> Result<PathBuf> {
    let out = Process::new("rustc")
        .args(["--print", "sysroot"])
        .output()
        .context("cannot run rustc")?;
    if !out.status.success() {
        bail!("rustc --print sysroot failed with {}", out.status);
    }
    Ok(PathBuf::from(String::from_utf8(out.stdout)?.trim()))
}

/// The notices of the Rust toolchain's code in the binary: compiler-builtins,
/// with its libm, from the toolchain's rust-src component, and a pointer to
/// the copy of COPYRIGHT-library.html.
fn rust_notices(sysroot: &Path) -> Result<String> {
    let out = Process::new("rustc")
        .arg("--version")
        .output()
        .context("cannot run rustc")?;
    let rustc = String::from_utf8(out.stdout)?.trim().to_string();
    let builtins = sysroot.join("lib/rustlib/src/rust/library/compiler-builtins");
    let read = |file: &str| -> Result<String> {
        let path = builtins.join(file);
        fs::read_to_string(&path).with_context(|| {
            format!(
                "cannot read {}; the notices need the toolchain's rust-src component \
                 (`rustup component add rust-src`)",
                path.display()
            )
        })
    };
    Ok(format!(
        "\n{rule}\nThe Rust standard library and compiler-builtins ({rustc})\n\n\
         The binary links the Rust standard library of the toolchain that built it. The\n\
         notices of the standard library, and of the crates it is built from, are in\n\
         {RUST_LIBRARY_COPYRIGHT}, the toolchain's COPYRIGHT-library.html. The\n\
         compiler-builtins crate it also links, with its libm, is under these terms:\n\n\
         {}\n\n{rule}\nlibm of compiler-builtins ({rustc})\n\n{}\n",
        read("LICENSE.txt")?.trim_end(),
        read("libm/LICENSE.txt")?.trim_end(),
        rule = "-".repeat(80),
    ))
}

/// For a Linux (musl) target, musl's COPYRIGHT and the Zig distribution's
/// LICENSE, from the zig installation that cargo-zigbuild links the binary
/// with: cargo-zigbuild leaves out the toolchain's own musl, and zig links the
/// musl it ships, its own libc (libzigc) and its compiler_rt statically. For
/// any other target, which zig does not link, nothing, without running zig.
fn zig_notices(target: &str) -> Result<String> {
    if !target.ends_with("-linux-musl") {
        return Ok(String::new());
    }
    let zig = std::env::var_os("CARGO_ZIGBUILD_ZIG_PATH").unwrap_or_else(|| "zig".into());
    let out = Process::new(&zig)
        .arg("env")
        .output()
        .context("cannot run zig, which links the release binaries")?;
    if !out.status.success() {
        bail!("zig env failed with {}", out.status);
    }
    let env = String::from_utf8(out.stdout)?;
    let field = |name: &str| -> Result<String> {
        env.lines()
            .find_map(|l| l.trim().strip_prefix(&format!(".{name} = \"")))
            .and_then(|v| v.strip_suffix("\","))
            .map(str::to_string)
            .with_context(|| format!("zig env names no {name}"))
    };
    let lib_dir = PathBuf::from(field("lib_dir")?);
    let zig_version = field("version")?;
    // Zig's own libc and compiler_rt, which it builds from its lib directory,
    // are under the license of the Zig distribution, at the root of the
    // installation.
    let zig_license = lib_dir
        .parent()
        .map(|root| root.join("LICENSE"))
        .context("zig's lib directory has no parent")?;
    let zig_license = fs::read_to_string(&zig_license)
        .with_context(|| format!("cannot read zig's license {}", zig_license.display()))?;
    let rule = "-".repeat(80);
    let musl = lib_dir.join("libc/musl");
    let header = fs::read_to_string(musl.join("src/internal/version.h"))
        .with_context(|| format!("cannot read musl's version in {}", musl.display()))?;
    let version = header
        .lines()
        .find_map(|l| l.strip_prefix("#define VERSION \""))
        .and_then(|v| v.strip_suffix('"'))
        .context("musl's version.h names no version")?;
    let copyright = fs::read_to_string(musl.join("COPYRIGHT"))
        .with_context(|| format!("cannot read musl's COPYRIGHT in {}", musl.display()))?;
    Ok(format!(
        "\n{rule}\nmusl libc {version} (linked statically, as Zig {zig_version} ships it)\n\n{}\n\
         \n{rule}\nZig {zig_version} (its libc and compiler_rt, linked statically)\n\n{}\n",
        copyright.trim_end(),
        zig_license.trim_end(),
    ))
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

/// Packs the files and directories under `dir` as the gzip-compressed tar
/// archive `archive`, deterministically, so that it carries nothing of the
/// build host and the same files give the same bytes on every host:
///
/// - GNU tar headers without PAX or GNU extension records, a directory's
///   name ending with `/`;
/// - the entries in the order of their names, every directory before its
///   contents and the contents of each directory sorted by name, as GNU tar's
///   `--sort=name` orders them;
/// - owner and group 0 without names, every modification time `epoch`, mode
///   0755 for directories and for files that any execute bit marks
///   executable, and 0644 for other files, and no extended attributes;
/// - gzip at the best compression, without a file name and with time 0.
///
/// Fails on anything but files and directories, such as a symbolic link, and
/// on a name too long for a plain header. It writes `<archive>.part` and
/// renames it, so that a failure leaves no archive behind. [`check_archive`]
/// checks the result.
fn pack(dir: &Path, archive: &Path, epoch: u64) -> Result<()> {
    let mut entries = Vec::new();
    tree(dir, "", &mut entries)?;
    if entries.is_empty() {
        bail!("{} holds nothing to pack", dir.display());
    }
    let mut part = archive.as_os_str().to_owned();
    part.push(".part");
    let part = PathBuf::from(part);
    let written = (|| -> Result<()> {
        let file =
            fs::File::create(&part).with_context(|| format!("cannot create {}", part.display()))?;
        let gzip = flate2::GzBuilder::new()
            .mtime(0)
            .operating_system(3)
            .write(std::io::BufWriter::new(file), flate2::Compression::best());
        let mut tar = tar::Builder::new(gzip);
        for (name, path, kind) in &entries {
            let mut header = tar::Header::new_gnu();
            header
                .set_path(name)
                .with_context(|| format!("cannot name {name} in a tar header"))?;
            header.set_uid(0);
            header.set_gid(0);
            header.set_mtime(epoch);
            let data = match kind {
                Kind::Directory => {
                    header.set_entry_type(tar::EntryType::Directory);
                    header.set_mode(0o755);
                    Vec::new()
                }
                Kind::File { executable } => {
                    header.set_entry_type(tar::EntryType::Regular);
                    header.set_mode(if *executable { 0o755 } else { 0o644 });
                    fs::read(path).with_context(|| format!("cannot read {}", path.display()))?
                }
            };
            header.set_size(u64::try_from(data.len())?);
            header.set_cksum();
            tar.append(&header, &data[..])
                .with_context(|| format!("cannot pack {name}"))?;
        }
        let file = tar.into_inner()?.finish()?;
        file.into_inner()
            .map_err(std::io::IntoInnerError::into_error)?
            .sync_all()?;
        Ok(())
    })();
    if let Err(err) = written {
        let _ = fs::remove_file(&part);
        return Err(err.context(format!("cannot write {}", part.display())));
    }
    fs::rename(&part, archive).with_context(|| format!("cannot write {}", archive.display()))?;
    println!("wrote {}", archive.display());
    Ok(())
}

/// An entry of an archive that [`pack`] writes.
enum Kind {
    Directory,
    File { executable: bool },
}

/// Appends the entries under `dir` to `entries`, each with its name in the
/// archive, `prefix` followed by its path under `dir`, in the order [`pack`]
/// writes them.
fn tree(dir: &Path, prefix: &str, entries: &mut Vec<(String, PathBuf, Kind)>) -> Result<()> {
    let mut children = fs::read_dir(dir)
        .with_context(|| format!("cannot list {}", dir.display()))?
        .collect::<std::io::Result<Vec<_>>>()
        .with_context(|| format!("cannot list {}", dir.display()))?;
    children.sort_by_key(|e| e.file_name());
    for child in children {
        let path = child.path();
        let file_name = child.file_name();
        let file_name = file_name
            .to_str()
            .with_context(|| format!("{} is not named in UTF-8", path.display()))?;
        let name = format!("{prefix}{file_name}");
        let metadata = fs::symlink_metadata(&path)
            .with_context(|| format!("cannot read {}", path.display()))?;
        if metadata.is_dir() {
            entries.push((format!("{name}/"), path.clone(), Kind::Directory));
            tree(&path, &format!("{name}/"), entries)?;
        } else if metadata.is_file() {
            use std::os::unix::fs::PermissionsExt;
            let executable = metadata.permissions().mode() & 0o111 != 0;
            entries.push((name, path, Kind::File { executable }));
        } else {
            bail!("{} is neither a file nor a directory", path.display());
        }
    }
    Ok(())
}

/// Prefixes of absolute paths of a build host that no file of a release
/// archive may hold: home directories on Linux and macOS, root's home, and
/// Windows user profiles.
const HOST_PATHS: &[&str] = &["/Users/", "/home/", "/root/", "C:\\Users"];

/// Occurrences of a host path in a file that are not traces of a build host,
/// by the archive's file name and the text that holds the path. None is
/// expected; an entry here names why the text is in the file.
const ALLOWED_HOST_PATHS: &[(&str, &str)] = &[];

/// Checks a release archive, a gzip-compressed tar file, from its raw bytes:
///
/// - the gzip stream, every member of it, records no file name, comment,
///   extra field or time;
/// - every tar header is a plain GNU or ustar header of a file or directory,
///   with no PAX or GNU extension record, no link name and no name prefix,
///   owner and group 0 without names, the modification time `epoch`, and
///   mode 0755 for directories and executables or 0644 for other files;
/// - nothing but the zero blocks that end the archive and pad its last
///   record follows the last entry;
/// - no file holds an absolute path of the build host: a prefix of
///   [`HOST_PATHS`], or one of `forbidden`, such as the workspace, CARGO_HOME
///   and the toolchain's sysroot of the build, unless [`ALLOWED_HOST_PATHS`]
///   lists the occurrence.
///
/// A packer that leaked the build host's user, times or extended
/// attributes, or a build that left the host's paths in the binary, fails it.
fn check_archive(archive: &Path, epoch: u64, forbidden: &[String]) -> Result<()> {
    let bytes = fs::read(archive).with_context(|| format!("cannot read {}", archive.display()))?;
    let mut member = 0;
    while member < bytes.len() {
        let header = &bytes[member..];
        if header.len() < 10 || header[0..3] != [0x1f, 0x8b, 8] {
            bail!("{} is not a gzip file", archive.display());
        }
        if header[3] & !0x02 != 0 {
            bail!("its gzip header carries a file name, comment or extra field");
        }
        if header[4..8] != [0, 0, 0, 0] {
            bail!("its gzip header carries a time");
        }
        // The length of this member: decode it alone.
        let mut decoder = flate2::bufread::GzDecoder::new(header);
        std::io::copy(&mut decoder, &mut std::io::sink()).context("cannot decompress it")?;
        let rest = decoder.into_inner().len();
        member = bytes.len() - rest;
    }
    let mut tar = Vec::new();
    flate2::read::MultiGzDecoder::new(&bytes[..])
        .read_to_end(&mut tar)
        .context("cannot decompress it")?;

    let field = |h: &[u8], at: usize, len: usize| -> String {
        let raw = &h[at..at + len];
        let end = raw.iter().position(|&b| b == 0).unwrap_or(len);
        String::from_utf8_lossy(&raw[..end]).trim().to_string()
    };
    let octal = |h: &[u8], at: usize, len: usize| -> Result<u64> {
        let text = field(h, at, len);
        u64::from_str_radix(if text.is_empty() { "0" } else { &text }, 8)
            .with_context(|| format!("a header field holds '{text}', not an octal number"))
    };
    let mut patterns: Vec<String> = HOST_PATHS.iter().map(|p| p.to_string()).collect();
    patterns.extend(forbidden.iter().filter(|p| !p.is_empty()).cloned());
    let mut traces = Vec::new();
    let mut at = 0;
    let mut entries = 0;
    let mut ended = false;
    while at + 512 <= tar.len() {
        let header = &tar[at..at + 512];
        if header.iter().all(|&b| b == 0) {
            ended = true;
            break;
        }
        let name = field(header, 0, 100);
        let kind = header[156];
        let magic = &header[257..265];
        if magic != b"ustar  \0" && magic != b"ustar\x0000" {
            bail!("{name}: not a GNU or ustar header");
        }
        match kind {
            b'0' | b'\0' | b'5' => {}
            other => bail!(
                "{name}: an entry of type '{}', such as a PAX or GNU extension record",
                char::from(other)
            ),
        }
        if !field(header, 157, 100).is_empty() {
            bail!("{name}: a link name");
        }
        if !field(header, 345, 155).is_empty() {
            bail!("{name}: a name prefix");
        }
        let mode = octal(header, 100, 8)? & 0o7777;
        let (uid, gid) = (octal(header, 108, 8)?, octal(header, 116, 8)?);
        let mtime = octal(header, 136, 12)?;
        let (uname, gname) = (field(header, 265, 32), field(header, 297, 32));
        if uid != 0 || gid != 0 || !uname.is_empty() || !gname.is_empty() {
            bail!("{name}: owner {uid} '{uname}' and group {gid} '{gname}', not 0 without names");
        }
        if mtime != epoch {
            bail!("{name}: modification time {mtime}, not {epoch}");
        }
        let executable = kind == b'5' || mode & 0o111 != 0;
        let expected = if executable { 0o755 } else { 0o644 };
        if mode != expected {
            bail!("{name}: mode {mode:o}, not {expected:o}");
        }
        let size = usize::try_from(octal(header, 124, 12)?)?;
        let data = tar
            .get(at + 512..at + 512 + size)
            .with_context(|| format!("{name}: the archive ends inside it"))?;
        for pattern in &patterns {
            for found in find_all(data, pattern.as_bytes()) {
                let context = text_around(data, found, pattern.len());
                let allowed = ALLOWED_HOST_PATHS
                    .iter()
                    .any(|(file, text)| *file == name && context.contains(text));
                if !allowed {
                    traces.push(format!("{name}: {context}"));
                }
            }
        }
        at += 512 + size.div_ceil(512) * 512;
        entries += 1;
    }
    if !ended {
        bail!(
            "{} lacks the blocks that end a tar archive",
            archive.display()
        );
    }
    if tar[at..].iter().any(|&b| b != 0) {
        bail!(
            "{} holds data after the end of its tar archive",
            archive.display()
        );
    }
    if entries == 0 {
        bail!("{} holds no entry", archive.display());
    }
    if !traces.is_empty() {
        let shown = traces.len().min(20);
        bail!(
            "{} holds {} absolute paths of the build host, such as:\n{}",
            archive.display(),
            traces.len(),
            traces[..shown].join("\n")
        );
    }
    println!(
        "{}: {entries} entries with deterministic headers and no host path",
        archive.display()
    );
    Ok(())
}

/// The offsets at which `needle` starts in `haystack`.
fn find_all<'a>(haystack: &'a [u8], needle: &'a [u8]) -> impl Iterator<Item = usize> + 'a {
    haystack
        .windows(needle.len().max(1))
        .enumerate()
        .filter(move |(_, w)| !needle.is_empty() && *w == needle)
        .map(|(at, _)| at)
}

/// The printable text around a match, for a message.
fn text_around(data: &[u8], at: usize, len: usize) -> String {
    let printable = |b: &u8| b.is_ascii_graphic() || *b == b' ';
    let mut start = at;
    while start > 0 && at - start < 40 && printable(&data[start - 1]) {
        start -= 1;
    }
    let mut end = at + len;
    while end < data.len() && end - at < 120 && printable(&data[end]) {
        end += 1;
    }
    String::from_utf8_lossy(&data[start..end]).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A packed archive passes `check-archive`, lists its entries in the
    /// order of their names with every directory before its contents, keeps
    /// the execute bit as mode 0755, and is the same bytes when packed again.
    #[test]
    fn pack_writes_a_deterministic_archive_that_check_archive_accepts() {
        let root = std::env::temp_dir().join(format!("xtask-pack-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let stage = root.join("stage");
        fs::create_dir_all(stage.join("man/man1")).unwrap();
        fs::write(stage.join("git-intake"), "binary").unwrap();
        fs::write(stage.join("LICENSE"), "license").unwrap();
        fs::write(stage.join("man/man1/git-intake.1"), "page").unwrap();
        fs::write(stage.join("a.txt"), "").unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(stage.join("git-intake"), fs::Permissions::from_mode(0o700))
                .unwrap();
            fs::set_permissions(stage.join("LICENSE"), fs::Permissions::from_mode(0o600)).unwrap();
        }
        let epoch = 1_700_000_000;
        let first = root.join("first.tar.gz");
        let second = root.join("second.tar.gz");
        pack(&stage, &first, epoch).unwrap();
        pack(&stage, &second, epoch).unwrap();
        check_archive(&first, epoch, &[]).unwrap();
        let bytes = fs::read(&first).unwrap();
        assert_eq!(bytes, fs::read(&second).unwrap());

        let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(&bytes[..]));
        let listing: Vec<(String, u32)> = archive
            .entries()
            .unwrap()
            .map(|e| {
                let e = e.unwrap();
                let name = String::from_utf8(e.path_bytes().into_owned()).unwrap();
                (name, e.header().mode().unwrap())
            })
            .collect();
        assert_eq!(
            listing,
            [
                ("LICENSE".to_string(), 0o644),
                ("a.txt".to_string(), 0o644),
                ("git-intake".to_string(), 0o755),
                ("man/".to_string(), 0o755),
                ("man/man1/".to_string(), 0o755),
                ("man/man1/git-intake.1".to_string(), 0o644),
            ]
        );
        assert!(!root.join("first.tar.gz.part").exists());
        fs::remove_dir_all(&root).unwrap();
    }

    /// A symbolic link is not packed, and the failure leaves no archive.
    #[test]
    fn pack_refuses_a_symbolic_link() {
        let root = std::env::temp_dir().join(format!("xtask-pack-link-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let stage = root.join("stage");
        fs::create_dir_all(&stage).unwrap();
        std::os::unix::fs::symlink("elsewhere", stage.join("link")).unwrap();
        let archive = root.join("out.tar.gz");
        let err = pack(&stage, &archive, 0).unwrap_err();
        assert!(
            err.to_string().contains("neither a file nor a directory"),
            "{err:#}"
        );
        assert!(!archive.exists());
        fs::remove_dir_all(&root).unwrap();
    }

    /// Only the Linux (musl) targets are linked by zig; the notices of a
    /// macOS target are made without running zig at all.
    #[test]
    fn zig_notices_are_only_for_the_musl_targets() {
        for target in ["aarch64-apple-darwin", "x86_64-apple-darwin"] {
            assert_eq!(zig_notices(target).unwrap(), "", "{target}");
        }
    }

    #[test]
    fn copyright_lines_are_recognised_in_every_common_form() {
        for line in [
            // The form of clap 4.6.7's and clap_builder 4.6.7's src/lib.rs.
            "// Copyright ⓒ 2015-2016 Example Authors and [`example` contributors](https://example.invalid/).",
            "// Copyright 2016 Example Authors.",
            "/* Copyright (c) 2014, Intel Corporation.",
            "# Copyright (C) 2019 Example Authors",
            "Copyright © 2005-2020 Example Authors, et al.",
            "© 2014 Unicode, Inc.",
            "// Copyright The Rust Project Developers",
            "Copyright: Example Project contributors",
            "// Portions Copyright (c) 2016, Google Inc.",
            "// SPDX-FileCopyrightText: 2023 Example Authors",
            "COPYRIGHT 2001 EXAMPLE CORP.",
            // The form of serde_json 1.0.151's src/lexical/mod.rs, line 9.
            "// copyright Example Author.",
        ] {
            assert!(is_copyright_line(line), "{line}");
        }
        for line in [
            "// The above copyright notice and this permission notice shall be included in",
            "// IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM,",
            "COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER",
            "// copyright, or license notices",
            "Copyright and related rights waived via CC0",
            "// characters such as '#' and '©'.",
            "let copyright = read();",
            "// copyright notice and this permission notice appear in all copies.",
        ] {
            assert!(!is_copyright_line(line), "{line}");
        }
    }

    #[test]
    fn a_notice_keeps_the_lines_that_continue_its_holders() {
        let header = "// Copyright 2018 Example Author A (@a),\n\
                      // Example Author B (@b), and\n\
                      // Example Author C (@c)\n\
                      //\n\
                      // Licensed under the Apache License, Version 2.0\n\
                      // Copyright 2019 D.\n\
                      fn main() {}\n";
        assert_eq!(
            copyright_notices(header),
            [
                "Copyright 2018 Example Author A (@a),\n\
                 Example Author B (@b), and\n\
                 Example Author C (@c)",
                "Copyright 2019 D.",
            ]
        );
    }

    /// The locked serde_json's lowercase notice and clap_derive's notice of
    /// three lines are among the copyright notices of their sources, whole.
    #[test]
    fn the_locked_lowercase_and_multi_line_notices_are_kept() {
        let source = |name: &str| {
            let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
            let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
            let out = Process::new(cargo)
                .current_dir(root)
                .args(["metadata", "--locked", "--format-version", "1"])
                .output()
                .unwrap();
            let metadata: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
            let manifest = metadata["packages"]
                .as_array()
                .unwrap()
                .iter()
                .find(|p| p["name"] == name)
                .and_then(|p| p["manifest_path"].as_str())
                .unwrap()
                .to_string();
            PathBuf::from(manifest).parent().unwrap().to_path_buf()
        };
        let line = |dir: &Path, file: &str, number: usize| {
            let text = fs::read_to_string(dir.join(file)).unwrap();
            normalized(text.lines().nth(number - 1).unwrap())
        };
        let serde_json = source("serde_json");
        let notice = line(&serde_json, "src/lexical/mod.rs", 9);
        assert!(notice.starts_with("copyright "), "{notice}");
        let notices = copyright_lines(&serde_json).unwrap();
        assert!(notices.contains_key(&notice), "{notices:?}");
        let clap_derive = source("clap_derive");
        let header = (1..=3)
            .map(|n| line(&clap_derive, "src/lib.rs", n))
            .collect::<Vec<_>>()
            .join("\n");
        let notices = copyright_lines(&clap_derive).unwrap();
        assert!(notices.contains_key(&header), "{header}\n{notices:?}");
    }

    /// The locked clap's header, which writes `Copyright ⓒ`, is among the
    /// copyright lines of its sources.
    #[test]
    fn the_locked_clap_header_is_a_copyright_line() {
        let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let out = Process::new(cargo)
            .current_dir(root)
            .args(["metadata", "--locked", "--format-version", "1"])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let metadata: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        for name in ["clap", "clap_builder"] {
            let manifest = metadata["packages"]
                .as_array()
                .unwrap()
                .iter()
                .find(|p| p["name"] == name)
                .and_then(|p| p["manifest_path"].as_str())
                .unwrap();
            let source = Path::new(manifest).parent().unwrap();
            let header = fs::read_to_string(source.join("src/lib.rs")).unwrap();
            let header = header.lines().next().unwrap();
            assert!(is_copyright_line(header), "{name}: {header}");
            let lines = copyright_lines(source).unwrap();
            assert!(
                lines
                    .get(&normalized(header))
                    .is_some_and(|files| files.iter().any(|f| f == "src/lib.rs")),
                "{name}: {lines:?}"
            );
        }
    }
}
