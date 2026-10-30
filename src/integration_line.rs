//! The integration line: the commits through which the integration branch
//! received its own changes, which is where the tool reads its own history.
//!
//! The line is the first-parent line of the integration branch, where each
//! merge on it is followed by the first-parent line of its second parent
//! down to, but excluding, the first commit its first parent contains: the
//! commits that a merge request merged with a merge commit brought in, as
//! `git rev-list --first-parent M^1..M^2` lists them. Only that one level
//! counts; merges on those lines contribute nothing of their own. A sync
//! merge brings in nothing either: its second parent is the upstream's
//! target, and what lies below it is the upstream's history.

use std::collections::{HashMap, HashSet};

use anyhow::Result;

use crate::git::Git;
use crate::sync_record::{self, Record};

/// One commit of the integration line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineCommit {
    pub commit: String,
    pub parents: Vec<String>,
    /// The merge on the first-parent line whose second parent brought this
    /// commit in; `None` for a commit of the first-parent line itself.
    pub merged_by: Option<String>,
}

/// The integration line ending at some commit and the sync records on it.
#[derive(Debug, Clone, Default)]
pub struct Line {
    /// Newest first, each merge followed by the commits it brought in,
    /// newest first.
    pub commits: Vec<LineCommit>,
    /// The sync records among `commits`, in the same order.
    pub records: Vec<Record>,
}

/// The integration line ending at `rev`. Commits contained in any of
/// `exclude` are left out, and so is everything below them on the line.
///
/// One `git rev-list` lists the history with its parents; the line is then
/// walked in memory from its oldest commit, keeping the set of commits each
/// first parent contains, so the cost does not grow with the number of
/// merges. One `git log` then reads the sync records on the line, which
/// tell the sync merges whose second parents bring in nothing.
pub fn walk(git: &Git, rev: &str, exclude: &[String]) -> Result<Line> {
    let mut args: Vec<&str> = vec!["rev-list", "--parents", "--topo-order", rev];
    if !exclude.is_empty() {
        args.push("--not");
        args.extend(exclude.iter().map(String::as_str));
    }
    args.push("--");
    let commits = line(&git.run(&args)?);
    let records = sync_record::read(git, commits.iter().map(|c| c.commit.as_str()))?;
    Ok(without_upstream_history(commits, records))
}

/// Leaves out what a sync merge on the first-parent line brought in: a
/// merge carrying a sync record whose target is its second parent, as every
/// merge the tool makes is.
fn without_upstream_history(commits: Vec<LineCommit>, records: Vec<Record>) -> Line {
    let by_id: HashMap<&str, &LineCommit> =
        commits.iter().map(|c| (c.commit.as_str(), c)).collect();
    let sync_merges: HashSet<String> = records
        .iter()
        .filter(|r| {
            by_id.get(r.commit.as_str()).is_some_and(|c| {
                c.merged_by.is_none()
                    && c.parents
                        .get(1)
                        .is_some_and(|p| p.eq_ignore_ascii_case(&r.target))
            })
        })
        .map(|r| r.commit.clone())
        .collect();
    let commits: Vec<LineCommit> = commits
        .into_iter()
        .filter(|c| {
            c.merged_by
                .as_ref()
                .is_none_or(|m| !sync_merges.contains(m))
        })
        .collect();
    let kept: HashSet<&str> = commits.iter().map(|c| c.commit.as_str()).collect();
    let records = records
        .into_iter()
        .filter(|r| kept.contains(r.commit.as_str()))
        .collect();
    Line { commits, records }
}

/// The integration line from `git rev-list --parents --topo-order` output
/// for one tip, which therefore comes first.
fn line(listing: &str) -> Vec<LineCommit> {
    let entries: Vec<Vec<&str>> = listing
        .lines()
        .map(|l| l.split_whitespace().collect::<Vec<_>>())
        .filter(|ids| !ids.is_empty())
        .collect();
    let index: HashMap<&str, usize> = entries.iter().enumerate().map(|(i, e)| (e[0], i)).collect();
    // A parent outside the listing was excluded.
    let parents: Vec<Vec<Option<usize>>> = entries
        .iter()
        .map(|e| e[1..].iter().map(|p| index.get(p).copied()).collect())
        .collect();
    let parent = |i: usize, n: usize| parents[i].get(n).copied().flatten();

    let mut first = Vec::new();
    let mut next = (!entries.is_empty()).then_some(0);
    while let Some(i) = next {
        first.push(i);
        next = parent(i, 0);
    }

    // Oldest first: before each commit is visited, `contained` holds exactly
    // what its first parent contains.
    let mut contained = vec![false; entries.len()];
    let mut brought: Vec<Vec<usize>> = vec![Vec::new(); first.len()];
    for (k, &m) in first.iter().enumerate().rev() {
        let mut next = parent(m, 1);
        while let Some(i) = next {
            if contained[i] {
                break;
            }
            brought[k].push(i);
            next = parent(i, 0);
        }
        let mut stack = vec![m];
        while let Some(i) = stack.pop() {
            if !contained[i] {
                contained[i] = true;
                stack.extend(parents[i].iter().flatten());
            }
        }
    }

    let commit = |i: usize, merged_by: Option<usize>| LineCommit {
        commit: entries[i][0].to_string(),
        parents: entries[i][1..].iter().map(|p| p.to_string()).collect(),
        merged_by: merged_by.map(|m| entries[m][0].to_string()),
    };
    let mut result = Vec::new();
    for (k, &m) in first.iter().enumerate() {
        result.push(commit(m, None));
        result.extend(brought[k].iter().map(|&i| commit(i, Some(m))));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(line: &[LineCommit]) -> Vec<&str> {
        line.iter().map(|c| c.commit.as_str()).collect()
    }

    #[test]
    fn a_merge_is_followed_by_what_its_second_parent_brought_in() {
        // m merges f2 (on f1, on a) into b (on a); f1 merged x from a side
        // branch on a, which stays out.
        let listing = "m b f2\nf2 f1\nf1 a x\nx a\nb a\na\n";
        let line = line(listing);
        assert_eq!(ids(&line), ["m", "f2", "f1", "b", "a"]);
        assert_eq!(line[1].merged_by.as_deref(), Some("m"));
        assert_eq!(line[3].merged_by, None);
    }

    #[test]
    fn a_second_parent_the_first_parent_contains_brings_nothing() {
        let listing = "m b a\nb a\na\n";
        assert_eq!(ids(&line(listing)), ["m", "b", "a"]);
    }

    #[test]
    fn a_sync_merge_brings_in_nothing() {
        // s merged the upstream's target t, which carried a record of the
        // upstream's own; m merged the branch f, which carries a sync merge
        // g of its own whose target is not its second parent.
        let listing = "m s f\nf g\ng s u\nu\ns b t\nt\nb\n";
        let record = |commit: &str, target: &str| Record {
            commit: commit.into(),
            upstream: "up".into(),
            target: target.into(),
            tag: None,
        };
        let records = vec![record("g", "x"), record("s", "t"), record("t", "y")];
        let line = without_upstream_history(line(listing), records);
        assert_eq!(ids(&line.commits), ["m", "f", "g", "s", "b"]);
        let kept: Vec<&str> = line.records.iter().map(|r| r.commit.as_str()).collect();
        assert_eq!(kept, ["g", "s"]);
    }

    #[test]
    fn excluded_parents_end_the_line() {
        // u is excluded, as an upstream's commit is.
        let listing = "m b u\nb\n";
        assert_eq!(ids(&line(listing)), ["m", "b"]);
        assert_eq!(line("").len(), 0);
    }
}
