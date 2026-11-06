//! `delta`: how an update changed what the downstream carries on top of its
//! upstream.
//!
//! The downstream's delta is the difference between the upstream's commit
//! and the downstream's tree. Before the update it is the base's tree over
//! the previous target, after it the commit's tree over the target. Each
//! becomes a one-commit series of its own, and `git range-diff` compares
//! the two, so only what the update changed in the delta shows.

use std::io::IsTerminal;

use anyhow::{Result, bail};

use crate::Exit;
use crate::commands::env;
use crate::config::Mode;
use crate::git::{Git, abbrev};
use crate::repo::Repo;

/// The subject of both synthetic commits, the same so that `range-diff`
/// compares only their changes.
const SUBJECT: &str = "the downstream's changes on top of its upstream";

pub fn run(repo: &Repo, rev: Option<&str>) -> Result<Exit> {
    let git = &repo.git;
    let rev = rev.unwrap_or("HEAD");
    let (commit, update) = env::update_at(repo, rev)?;
    let Some(update) = update else {
        bail!(
            "{} carries no update; delta compares the update a commit carries over the integration branch",
            abbrev(&commit)
        );
    };
    let described = env::described(git, &update.config, &update.history, &update.base)?;
    let one = match described.as_slice() {
        [one] => one,
        _ => bail!(
            "the update at {} takes {}; delta compares an update of exactly one follow upstream at the \
             repository root",
            abbrev(&commit),
            names(&described)
        ),
    };
    let root = update
        .config
        .upstream(&one.name)
        .is_some_and(|up| up.mode == Mode::Follow && up.path.is_none());
    let (true, Some(target)) = (root, &one.target) else {
        bail!(
            "the update at {} takes upstream '{}', which is not a follow upstream at the repository root; \
             delta compares an update of exactly one follow upstream at the repository root",
            abbrev(&commit),
            one.name
        );
    };
    let Some(previous) = &one.previous else {
        bail!(
            "upstream '{}' has no previous target before the update at {}, so there is no earlier delta to \
             compare with",
            one.name,
            abbrev(&commit)
        );
    };
    let before = series(git, &update.base, previous)?;
    let after = series(git, &commit, target)?;
    let color = if std::io::stdout().is_terminal() {
        "--color=always"
    } else {
        "--no-color"
    };
    // Above 100% pairing two series always costs less than creating one
    // and deleting the other, so the two are compared however much differs.
    let out = git.run(&[
        "range-diff",
        "--creation-factor=200",
        color,
        &format!("{previous}..{before}"),
        &format!("{target}..{after}"),
    ])?;
    crate::output::print(&out)?;
    Ok(Exit::Success)
}

/// A commit, reachable from no reference, whose tree is that of `tree_of`
/// and whose one parent is `parent`.
fn series(git: &Git, tree_of: &str, parent: &str) -> Result<String> {
    git.line(&[
        "-c",
        "user.name=git-intake",
        "-c",
        "user.email=git-intake@localhost",
        "commit-tree",
        &format!("{tree_of}^{{tree}}"),
        "-p",
        parent,
        "-m",
        SUBJECT,
    ])
}

fn names(described: &[crate::commands::verify::Described]) -> String {
    if described.is_empty() {
        return "no upstream".to_string();
    }
    described
        .iter()
        .map(|d| format!("'{}'", d.name))
        .collect::<Vec<_>>()
        .join(", ")
}
