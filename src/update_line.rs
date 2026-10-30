//! The update line: the commits of an update branch that are the update's.
//!
//! Every commit between an update's head and its base is the wrong set,
//! because a sync merge brings in the upstream's whole history, which may
//! carry sync records of its own. The update line walks from the head along
//! first parents, as the update branch grows, through the commits the base
//! lacks, and crosses to the second parent only at a catch-up merge, which
//! carries `Intake-Catch-Up`, and at a head that merges the update into a
//! commit the base contains without a sync record, as a merged results
//! pipeline tests. A merge the forge's update button makes has the branch's
//! previous head as its first parent and stays on the line.
//!
//! The update's own commits are the sync merges, copies and changes to a
//! port upstream's journal on that line: from them an update describes
//! itself, whoever made it and wherever it is read.

use std::collections::{HashMap, HashSet};

use anyhow::Result;

use crate::config::{Config, Mode};
use crate::git::{Git, lines};
use crate::port::{self, Decision};
use crate::sync_record::{
    self, CATCH_UP_TRAILER, TAG_TRAILER, TAKEN_TRAILER, TARGET_TRAILER, UPSTREAM_TRAILER,
};

const FIELD: char = '\u{1f}';
const RECORD: char = '\u{1e}';

/// The commits of the update line from `head` over `base`, head first.
pub fn walk(git: &Git, head: &str, base: &str) -> Result<Vec<String>> {
    let listing = git.run(&["rev-list", "--parents", head, &format!("^{base}"), "--"])?;
    let parents: HashMap<String, Vec<String>> = listing
        .lines()
        .filter_map(|l| {
            let mut ids = l.split_whitespace().map(str::to_string);
            Some((ids.next()?, ids.collect()))
        })
        .collect();
    let merges = parents
        .iter()
        .filter(|(_, p)| p.len() > 1)
        .map(|(c, _)| c.as_str());
    let catch_ups = sync_record::catch_ups(git, merges)?;
    let head_record = match parents.get(head) {
        Some(p) if p.len() > 1 => sync_record::record_of(git, head)?.is_some(),
        _ => false,
    };
    Ok(line(head, &parents, &catch_ups, head_record))
}

/// The line through `parents`, which holds the commits the base lacks.
fn line(
    head: &str,
    parents: &HashMap<String, Vec<String>>,
    catch_ups: &HashSet<String>,
    head_has_record: bool,
) -> Vec<String> {
    let mut line = Vec::new();
    let mut seen = HashSet::new();
    let mut next = Some(head.to_string());
    while let Some(commit) = next.take() {
        let Some(ps) = parents.get(&commit) else {
            break;
        };
        if !seen.insert(commit.clone()) {
            break;
        }
        let merged_results =
            commit == head && !head_has_record && ps.len() > 1 && !parents.contains_key(&ps[0]);
        next = if catch_ups.contains(&commit) || merged_results {
            ps.get(1).cloned()
        } else {
            ps.first().cloned()
        };
        line.push(commit);
    }
    line
}

/// A follow upstream's newest sync merge on the update line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FollowTaken {
    pub name: String,
    pub target: String,
    pub tag: Option<String>,
    pub merge: String,
}

/// One copy on the update line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Copy {
    /// The copy.
    pub commit: String,
    /// The upstream commit it took.
    pub taken: String,
    pub subject: String,
}

/// What the update line holds of a port upstream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortTaken {
    pub name: String,
    /// Oldest first.
    pub copies: Vec<Copy>,
    /// The decisions the update added to the journal, in the journal's order.
    pub decisions: Vec<Decision>,
    /// The newest commit of the line that changed the journal.
    pub journal_commit: Option<String>,
}

/// What an update carries, read from its update line.
#[derive(Debug, Clone, Default)]
pub struct History {
    /// Head first.
    pub line: Vec<String>,
    /// In configuration order.
    pub follows: Vec<FollowTaken>,
    /// In configuration order.
    pub ports: Vec<PortTaken>,
}

impl History {
    /// Whether the line carries an update: a sync merge, a copy or a
    /// change to a port upstream's journal.
    pub fn carries_update(&self) -> bool {
        !self.follows.is_empty() || !self.ports.is_empty()
    }

    pub fn follow(&self, name: &str) -> Option<&FollowTaken> {
        self.follows.iter().find(|f| f.name == name)
    }

    /// The names of the upstreams the update takes, in configuration order.
    pub fn names(&self, config: &Config) -> Vec<String> {
        config
            .upstreams
            .iter()
            .filter(|u| {
                self.follow(&u.name).is_some() || self.ports.iter().any(|p| p.name == u.name)
            })
            .map(|u| u.name.clone())
            .collect()
    }
}

/// Reads what the update at `head` over `base` carries for the upstreams
/// `config` lists.
pub fn describe(git: &Git, config: &Config, head: &str, base: &str) -> Result<History> {
    let line = walk(git, head, base)?;
    let entries = read_trailers(git, &line)?;
    let mut follows = Vec::new();
    let mut ports = Vec::new();
    for up in &config.upstreams {
        match up.mode {
            Mode::Follow => {
                // Head first: the first record is the newest.
                if let Some(e) = entries.iter().find(|e| {
                    e.upstream == up.name
                        && !e.target.is_empty()
                        && sync_record::is_sync_merge(&e.parents, &e.target)
                }) {
                    follows.push(FollowTaken {
                        name: up.name.clone(),
                        target: e.target.to_ascii_lowercase(),
                        tag: (!e.tag.is_empty()).then(|| e.tag.clone()),
                        merge: e.commit.clone(),
                    });
                }
            }
            Mode::Port => {
                let copies: Vec<Copy> = entries
                    .iter()
                    .rev()
                    .filter(|e| e.upstream == up.name && !e.taken.is_empty())
                    .map(|e| Copy {
                        commit: e.commit.clone(),
                        taken: port::commit_id(&e.taken),
                        subject: e.subject.clone(),
                    })
                    .collect();
                let journal = port::journal_path(&up.name);
                let changed: HashSet<String> = lines(&git.run(&[
                    "log",
                    "--format=%H",
                    head,
                    &format!("^{base}"),
                    "--",
                    &journal,
                ])?)
                .into_iter()
                .collect();
                let journal_commit = line.iter().find(|c| changed.contains(*c)).cloned();
                let decisions = match &journal_commit {
                    None => Vec::new(),
                    Some(_) => {
                        let before = port::read_journal(git, base, &up.name)?;
                        let after = port::read_journal(git, head, &up.name)?;
                        let known: HashSet<String> = before
                            .map(|j| j.decisions.into_iter().map(|d| d.commit).collect())
                            .unwrap_or_default();
                        after
                            .map(|j| j.decisions)
                            .unwrap_or_default()
                            .into_iter()
                            .filter(|d| !known.contains(&d.commit))
                            .collect()
                    }
                };
                if !copies.is_empty() || journal_commit.is_some() {
                    ports.push(PortTaken {
                        name: up.name.clone(),
                        copies,
                        decisions,
                        journal_commit,
                    });
                }
            }
        }
    }
    Ok(History {
        line,
        follows,
        ports,
    })
}

/// The newest of the upstream commits a port update takes or decides,
/// given oldest first in the order the update recorded them: by ancestry
/// when the repository has them all, since they lie on the upstream's
/// first-parent line, and otherwise the last one recorded.
pub fn newest(git: &Git, recorded: &[String]) -> Result<Option<String>> {
    let mut present = true;
    for c in recorded {
        present &= git.object_exists(c)?;
    }
    if !present || recorded.len() < 2 {
        return Ok(recorded.last().cloned());
    }
    let mut args = vec!["merge-base".to_string(), "--independent".to_string()];
    args.extend(recorded.iter().cloned());
    let independent = lines(&git.run(&args)?);
    Ok(recorded
        .iter()
        .rev()
        .find(|c| independent.contains(c))
        .or(recorded.last())
        .cloned())
}

/// The upstream commits a port update takes or decides, oldest first where
/// the repository can tell, as [`newest`] orders them.
pub fn ordered(git: &Git, recorded: &[String]) -> Result<Vec<String>> {
    let mut present = true;
    for c in recorded {
        present &= git.object_exists(c)?;
    }
    if !present {
        return Ok(recorded.to_vec());
    }
    let mut sorted = recorded.to_vec();
    // Commits of one first-parent line: one that contains another is newer.
    let mut swapped = true;
    while swapped {
        swapped = false;
        for i in 1..sorted.len() {
            if git.is_ancestor(&sorted[i], &sorted[i - 1])? && sorted[i] != sorted[i - 1] {
                sorted.swap(i, i - 1);
                swapped = true;
            }
        }
    }
    Ok(sorted)
}

struct Entry {
    commit: String,
    parents: Vec<String>,
    subject: String,
    upstream: String,
    target: String,
    tag: String,
    taken: String,
}

/// The trailers of the commits of the line, in its order.
fn read_trailers(git: &Git, line: &[String]) -> Result<Vec<Entry>> {
    if line.is_empty() {
        return Ok(Vec::new());
    }
    let input: String = line.iter().map(|c| format!("{c}\n")).collect();
    let trailer = |key: &str| format!("%(trailers:key={key},valueonly,separator=%x2C)");
    let format = format!(
        "--format=%H%x1f%P%x1f%s%x1f{}%x1f{}%x1f{}%x1f{}%x1f{}%x1e",
        trailer(UPSTREAM_TRAILER),
        trailer(TARGET_TRAILER),
        trailer(TAG_TRAILER),
        trailer(TAKEN_TRAILER),
        trailer(CATCH_UP_TRAILER),
    );
    let out = git.run_with_input(
        &[
            "log".to_string(),
            "--no-walk=unsorted".into(),
            "--stdin".into(),
            format,
            "--".into(),
        ],
        input,
    )?;
    let first = |s: &str| s.split(',').next().unwrap_or("").trim().to_string();
    Ok(out
        .split(RECORD)
        .filter_map(|entry| {
            let mut f = entry.trim_matches(['\n', '\r']).split(FIELD);
            let commit = f.next()?.trim().to_string();
            if commit.is_empty() {
                return None;
            }
            Some(Entry {
                commit,
                parents: f.next()?.split_whitespace().map(str::to_string).collect(),
                subject: f.next()?.to_string(),
                upstream: first(f.next()?),
                target: first(f.next()?),
                tag: first(f.next()?),
                taken: first(f.next()?),
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graph(edges: &[(&str, &[&str])]) -> HashMap<String, Vec<String>> {
        edges
            .iter()
            .map(|(c, ps)| (c.to_string(), ps.iter().map(|p| p.to_string()).collect()))
            .collect()
    }

    #[test]
    fn the_line_follows_first_parents_and_crosses_at_catch_ups() {
        // s1 merges the upstream's target u (outside the update, as the
        // base lacks it) onto the base b; f is a fix on top; c caught up
        // with the integration tip t; x is the forge's update button,
        // merging t2 into the branch.
        let parents = graph(&[
            ("x", &["c", "t2"]),
            ("t2", &["t"]),
            ("c", &["t", "f"]),
            ("t", &["b0"]),
            ("f", &["s1"]),
            ("s1", &["b", "u"]),
            ("u", &["u0"]),
        ]);
        let catch_ups: HashSet<String> = ["c".to_string()].into();
        assert_eq!(
            line("x", &parents, &catch_ups, false),
            ["x", "c", "f", "s1"]
        );
    }

    #[test]
    fn a_merged_results_head_crosses_to_the_update() {
        // m merges the update head h into the integration tip t, which the
        // base contains: it is not in the listing.
        let parents = graph(&[
            ("m", &["t", "h"]),
            ("h", &["s"]),
            ("s", &["t", "u"]),
            ("u", &[]),
        ]);
        assert_eq!(line("m", &parents, &HashSet::new(), false), ["m", "h", "s"]);
        // A head that is a sync merge onto the base stays on its first
        // parent, which ends the line.
        let parents = graph(&[("s", &["b", "u"]), ("u", &[])]);
        assert_eq!(line("s", &parents, &HashSet::new(), true), ["s"]);
    }
}
