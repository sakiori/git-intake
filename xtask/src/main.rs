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
        [task, archive, epoch] if task == "check-archive" => check_archive(
            Path::new(archive),
            epoch
                .parse()
                .context("the epoch is not a number of seconds")?,
        ),
        _ => bail!(
            "usage: cargo run -p xtask -- man <dir> | notices <dir> <target> | \
             check-archive <archive> <epoch>"
        ),
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
        for line in content.lines().filter(|l| is_copyright_line(l)) {
            let files = lines.entry(normalized(line)).or_default();
            if !files.contains(&shown) {
                files.push(shown.clone());
            }
        }
    }
    Ok(lines)
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
            .filter(|(line, _)| !notices.contains(line.as_str()))
            .collect();
        if missing.is_empty() {
            continue;
        }
        section.push_str(&format!("\n{name} {version}\n"));
        for (line, files) in missing {
            section.push_str(&format!("\n  {line}\n    in {}\n", files.join(", ")));
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
            if !notices.contains(&line) {
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

/// musl's COPYRIGHT and the Zig distribution's LICENSE, from the zig
/// installation that cargo-zigbuild links the static Linux binaries with:
/// cargo-zigbuild leaves out the toolchain's own musl, and zig links the musl
/// it ships, its own libc (libzigc) and its compiler_rt.
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
    let lib_dir = PathBuf::from(field("lib_dir")?);
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
    // Zig's own libc (libzigc) and compiler_rt, which it builds from its lib
    // directory and links statically, are under the license of the Zig
    // distribution, at the root of the installation.
    let zig_license = lib_dir
        .parent()
        .map(|root| root.join("LICENSE"))
        .context("zig's lib directory has no parent")?;
    let zig_license = fs::read_to_string(&zig_license)
        .with_context(|| format!("cannot read zig's license {}", zig_license.display()))?;
    let zig_version = field("version")?;
    Ok(format!(
        "\n{rule}\nmusl libc {version} (linked statically, as Zig {zig_version} ships it)\n\n{}\n\
         \n{rule}\nZig {zig_version} (its libc and compiler_rt, linked statically)\n\n{}\n",
        copyright.trim_end(),
        zig_license.trim_end(),
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

/// Checks the raw headers of a release archive, a gzip-compressed tar file:
/// the gzip header records no file name, comment or time, and every tar
/// header is a plain GNU or ustar header of a file or directory, with no PAX
/// or GNU extension record, owner and group 0 without names, the
/// modification time `epoch`, and mode 0755 for directories and executables
/// or 0644 for other files. A packer that leaked the build host's user,
/// times or extended attributes fails it.
fn check_archive(archive: &Path, epoch: u64) -> Result<()> {
    let bytes = fs::read(archive).with_context(|| format!("cannot read {}", archive.display()))?;
    if bytes.len() < 10 || bytes[0..3] != [0x1f, 0x8b, 8] {
        bail!("{} is not a gzip file", archive.display());
    }
    if bytes[3] & !0x02 != 0 {
        bail!("its gzip header carries a file name, comment or extra field");
    }
    if bytes[4..8] != [0, 0, 0, 0] {
        bail!("its gzip header carries a time");
    }
    let mut tar = Vec::new();
    flate2::read::GzDecoder::new(&bytes[..])
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
    let mut at = 0;
    let mut entries = 0;
    while at + 512 <= tar.len() {
        let header = &tar[at..at + 512];
        if header.iter().all(|&b| b == 0) {
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
        let size = octal(header, 124, 12)?;
        at += 512 + size.div_ceil(512) as usize * 512;
        entries += 1;
    }
    if entries == 0 {
        bail!("{} holds no entry", archive.display());
    }
    println!(
        "{}: {entries} entries with deterministic headers",
        archive.display()
    );
    Ok(())
}
