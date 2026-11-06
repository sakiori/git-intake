//! `conflicts`: the files the updates on the integration line needed
//! resolving in, recomputed from each merge's parents.

use std::collections::{BTreeSet, HashMap};

use anyhow::{Context, Result};
use serde::Serialize;

use crate::Exit;
use crate::commands::merge_request::fetch_integration;
use crate::config::{self, Config};
use crate::drift;
use crate::git::{Git, branch_ref};
use crate::integration_line;
use crate::output::{Render, emit};
use crate::repo::Repo;
use crate::sync_record;

#[derive(Serialize)]
struct ConflictsDoc {
    /// The sync and catch-up merges on the integration line, newest first.
    merges: Vec<MergeConflicts>,
    /// Every path of `merges`, sorted, without repeats.
    paths: Vec<String>,
}

#[derive(Serialize)]
struct MergeConflicts {
    commit: String,
    /// The upstream of a sync merge; `null` for a catch-up merge.
    upstream: Option<String>,
    catch_up: bool,
    paths: Vec<String>,
}

impl Render for ConflictsDoc {
    fn render(&self) -> String {
        self.paths.iter().map(|p| format!("{p}\n")).collect()
    }
}

pub fn run(repo: &Repo, since: Option<&str>, globs: &[String], json: bool) -> Result<Exit> {
    let git = &repo.git;
    let patterns = globs
        .iter()
        .map(|g| glob::Pattern::new(g).with_context(|| format!("--path '{g}' is not a valid glob")))
        .collect::<Result<Vec<_>>>()?;
    let (config, tip) = integration(repo)?;
    let exclude = match since {
        None => Vec::new(),
        Some(rev) => vec![
            repo.here
                .commit(rev)?
                .with_context(|| format!("'{rev}' names no commit"))?,
        ],
    };
    let line = integration_line::walk(git, &tip, &exclude)?;
    let merges = line
        .commits
        .iter()
        .filter(|c| c.parents.len() > 1)
        .map(|c| c.commit.as_str());
    let catch_ups = sync_record::catch_ups(git, merges)?;
    let options = glob::MatchOptions {
        require_literal_separator: true,
        ..Default::default()
    };
    let mut doc = ConflictsDoc {
        merges: Vec::new(),
        paths: Vec::new(),
    };
    let records: HashMap<&str, &sync_record::Record> = line
        .records
        .iter()
        .map(|r| (r.commit.as_str(), r))
        .collect();
    let mut all = BTreeSet::new();
    for c in &line.commits {
        let record = records.get(c.commit.as_str()).copied();
        let catch_up = catch_ups.contains(&c.commit);
        if record.is_none() && !catch_up {
            continue;
        }
        let path = record
            .and_then(|r| config.upstream(&r.upstream))
            .and_then(|up| up.path.as_deref());
        let paths: Vec<String> = recomputed(git, &c.parents, path)?
            .into_iter()
            .filter(|p| patterns.is_empty() || patterns.iter().any(|g| g.matches_with(p, options)))
            .collect();
        all.extend(paths.iter().cloned());
        doc.merges.push(MergeConflicts {
            commit: c.commit.clone(),
            upstream: record.filter(|_| !catch_up).map(|r| r.upstream.clone()),
            catch_up,
            paths,
        });
    }
    doc.paths = all.into_iter().collect();
    // An empty list is no output at all, not an empty line.
    if json || !doc.paths.is_empty() {
        emit(&doc, json)?;
    }
    Ok(Exit::Success)
}

/// The files a merge of `parents` conflicts in, sorted; none for the first
/// import of a path upstream, whose histories share nothing.
fn recomputed(git: &Git, parents: &[String], path: Option<&str>) -> Result<Vec<String>> {
    Ok(merged(git, parents, path)?
        .map(|m| m.conflicts)
        .unwrap_or_default())
}

/// The clean merge of `parents`, recomputed, its conflicted files sorted;
/// `None` for the first import of a path upstream, whose histories share
/// nothing.
fn merged(git: &Git, parents: &[String], path: Option<&str>) -> Result<Option<drift::MergeTree>> {
    let [first, second, ..] = parents else {
        return Ok(None);
    };
    if git.merge_base(first, second)?.is_none() {
        return Ok(None);
    }
    let mut merged = drift::merge_tree(git, first, second, path)?;
    merged.conflicts.sort();
    merged.conflicts.dedup();
    Ok(Some(merged))
}

/// What a person did to a merge beyond merging its parents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ByHand {
    /// Nothing: its tree is the clean merge of its parents.
    Nothing,
    /// It needed a resolution in these files.
    Resolved(Vec<String>),
    /// It merged cleanly, but its tree differs from that merge.
    Changed,
}

/// What a person did to the merge `commit` of `parents`, recomputed as
/// `conflicts` recomputes it. The first import of a path upstream counts as
/// done by the tool.
pub fn by_hand(git: &Git, commit: &str, parents: &[String], path: Option<&str>) -> Result<ByHand> {
    let Some(merged) = merged(git, parents, path)? else {
        return Ok(ByHand::Nothing);
    };
    if !merged.conflicts.is_empty() {
        return Ok(ByHand::Resolved(merged.conflicts));
    }
    let tree = git.line(&["rev-parse", &format!("{commit}^{{tree}}")])?;
    Ok(if tree == merged.tree {
        ByHand::Nothing
    } else {
        ByHand::Changed
    })
}

/// The configuration at the integration tip and that tip: the publish
/// remote's integration branch, fetched for the command, or the local one
/// without a publish remote, as `env` reads them.
fn integration(repo: &Repo) -> Result<(Config, String)> {
    let git = &repo.git;
    let named = repo
        .head()
        .map(|head| repo.config_at(head))
        .transpose()?
        .flatten()
        .with_context(|| format!("no {} is committed at HEAD", config::FILE_NAME))?;
    let integration = named.integration.clone();
    let tip = match &named.publish {
        Some(publish) => fetch_integration(git, &publish.remote, &integration)?,
        None => git
            .commit(&branch_ref(&integration))?
            .with_context(|| format!("the integration branch '{integration}' does not exist"))?,
    };
    let config = repo.config_at(&tip)?.unwrap_or(named);
    Ok((config, tip))
}
