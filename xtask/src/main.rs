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
//! toolchain's compiler-builtins and, for a musl target, of musl libc, and
//! `RUST-LIBRARY-COPYRIGHT.html`, the toolchain's notices of the Rust standard
//! library.

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
        [task, dir, target] if task == "notices" => notices(Path::new(dir), target),
        _ => bail!("usage: cargo run -p xtask -- man <dir> | notices <dir> <target>"),
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
/// - for a musl target, musl's COPYRIGHT, from the zig installation whose
///   musl cargo-zigbuild links statically.
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
        crates.push((name.to_string(), version.to_string(), source));
    }
    check_clarified(root, &crates, &text)?;

    let sysroot = rust_sysroot()?;
    text.push_str(&rust_notices(&sysroot)?);
    if target.ends_with("-linux-musl") {
        text.push_str(&musl_notices()?);
    }

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

/// Fails unless every copyright line of every source file of a crate that
/// `about.toml` clarifies is in the notices. A clarification replaces what
/// cargo-about harvests, so a notice it does not name would be lost.
fn check_clarified(root: &Path, crates: &[(String, String, PathBuf)], text: &str) -> Result<()> {
    let config: toml::Table = toml::from_str(
        &fs::read_to_string(root.join("about.toml")).context("cannot read about.toml")?,
    )
    .context("cannot parse about.toml")?;
    let notices = text.lines().map(normalized).collect::<Vec<_>>().join(" ");
    let mut missing = Vec::new();
    for (name, version, source) in crates {
        let clarified = config
            .get(name)
            .and_then(|c| c.as_table())
            .is_some_and(|c| c.contains_key("clarify"));
        if !clarified {
            continue;
        }
        for file in source_files(source)? {
            let Ok(content) = fs::read_to_string(&file) else {
                continue;
            };
            for line in content.lines().filter(|l| is_copyright_line(l)) {
                let line = normalized(line);
                if !notices.contains(&line) {
                    let path = file.strip_prefix(source).unwrap_or(&file);
                    missing.push(format!("{name} {version}, {}: {line}", path.display()));
                }
            }
        }
    }
    if !missing.is_empty() {
        bail!(
            "the notices lack copyright lines of clarified crates; add the files to about.toml:\n{}",
            missing.join("\n")
        );
    }
    Ok(())
}

/// Whether a line states a copyright: `Copyright` followed by a year, `(c)`
/// or `©`, in any letter case, as in `// Copyright 2016 Brian Smith.`.
fn is_copyright_line(line: &str) -> bool {
    let lower = line.to_lowercase();
    lower.match_indices("copyright").any(|(at, word)| {
        let after_word = lower[at..].chars().nth(word.len());
        let before = lower[..at].chars().next_back();
        if before.is_some_and(char::is_alphanumeric)
            || after_word.is_some_and(char::is_alphanumeric)
        {
            return false;
        }
        let rest = lower[at + word.len()..].trim_start();
        rest.starts_with(|c: char| c.is_ascii_digit())
            || rest.starts_with("(c)")
            || rest.starts_with('©')
    })
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

/// musl's COPYRIGHT, from the zig installation that cargo-zigbuild links the
/// static Linux binaries with: cargo-zigbuild leaves out the toolchain's own
/// musl, and zig links the musl it ships.
fn musl_notices() -> Result<String> {
    let zig = std::env::var_os("CARGO_ZIGBUILD_ZIG_PATH").unwrap_or_else(|| "zig".into());
    let out = Process::new(&zig)
        .arg("env")
        .output()
        .context("cannot run zig, whose musl the static Linux binaries link")?;
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
    let musl = Path::new(&field("lib_dir")?).join("libc/musl");
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
        "\n{rule}\nmusl libc {version} (linked statically, as zig {} ships it)\n\n{}\n",
        field("version")?,
        copyright.trim_end(),
        rule = "-".repeat(80),
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
