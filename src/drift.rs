//! How far each upstream is from the integration branch.

use anyhow::Result;
use serde::Serialize;

use std::collections::HashSet;

use crate::config::{Config, Upstream};
use crate::git::{Git, abbrev, lines, tracking_ref};
use crate::sync_record::Record;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DriftState {
    Current,
    Behind,
    NotImported,
    Rewritten,
    Unavailable,
}

impl DriftState {
    pub fn label(self) -> &'static str {
        match self {
            DriftState::Current => "current",
            DriftState::Behind => "behind",
            DriftState::NotImported => "not imported",
            DriftState::Rewritten => "rewritten",
            DriftState::Unavailable => "unavailable",
        }
    }
}

/// Where an upstream's last synced commit comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncSource {
    /// The newest sync record on the integration branch.
    Record,
    /// No sync record: the newest upstream commit the integration branch
    /// contains, for a fork created by cloning or a path imported by other means.
    History,
}

#[derive(Debug, Clone, Serialize)]
pub struct Drift {
    pub last_synced: Option<String>,
    pub last_synced_source: Option<SyncSource>,
    pub tip: Option<String>,
    pub commits_behind: Option<u64>,
    pub state: DriftState,
    pub detail: Option<String>,
}

impl Drift {
    fn unavailable(last_synced: Option<String>, detail: String) -> Self {
        Self {
            last_synced_source: last_synced.as_ref().map(|_| SyncSource::Record),
            last_synced,
            tip: None,
            commits_behind: None,
            state: DriftState::Unavailable,
            detail: Some(detail),
        }
    }
}

/// The fetched tip of an upstream, or why there is none.
pub fn fetched_tip(git: &Git, up: &Upstream) -> Result<std::result::Result<String, String>> {
    if git.remote_url(&up.remote)?.is_none() {
        return Ok(Err(format!("remote '{}' does not exist", up.remote)));
    }
    match git.commit(&tracking_ref(&up.remote, &up.branch))? {
        Some(tip) => Ok(Ok(tip)),
        None => Ok(Err(format!(
            "branch '{}' of remote '{}' has not been fetched or does not exist",
            up.branch, up.remote
        ))),
    }
}

/// Evaluates one upstream against the integration tip using the fetched
/// remote-tracking ref.
pub fn evaluate(
    git: &Git,
    integration_tip: &str,
    up: &Upstream,
    last: Option<&Record>,
    config: &Config,
) -> Result<Drift> {
    let recorded = last.map(|r| r.target.clone());
    let tip = match fetched_tip(git, up)? {
        Ok(tip) => tip,
        Err(why) => return Ok(Drift::unavailable(recorded, why)),
    };
    if let Some(ls) = &recorded
        && (!git.object_exists(ls)? || !git.is_ancestor(ls, &tip)?)
    {
        return Ok(Drift {
            detail: Some(format!(
                "{} no longer contains the last synced commit {}",
                up.source(),
                abbrev(ls)
            )),
            last_synced: recorded,
            last_synced_source: Some(SyncSource::Record),
            tip: Some(tip),
            commits_behind: None,
            state: DriftState::Rewritten,
        });
    }
    let (last_synced, source) = match recorded {
        Some(r) => (Some(r), Some(SyncSource::Record)),
        None => match adopted_base(git, integration_tip, up, &tip, config)? {
            Some(mb) => (Some(mb), Some(SyncSource::History)),
            None => (None, None),
        },
    };
    let behind = git.count(integration_tip, &tip)?;
    // An upstream is imported once a sync record exists, or when the
    // integration branch contains part of its own history (commits no other
    // upstream has): at the root, or at a path that exists (an import by
    // other means, such as `git subtree add`).
    let imported = last_synced.is_some();
    let state = if !imported {
        DriftState::NotImported
    } else if behind == 0 {
        DriftState::Current
    } else {
        DriftState::Behind
    };
    Ok(Drift {
        last_synced,
        last_synced_source: source,
        tip: Some(tip),
        commits_behind: Some(behind),
        state,
        detail: None,
    })
}

/// For an upstream without a sync record: the newest commit of its own
/// history that the integration branch contains, or `None`.
///
/// "Own" excludes every commit another configured upstream's fetched branch
/// contains, so ancestry shared only through a common template (or any other
/// upstream) never makes this upstream look imported. A path upstream also
/// needs its path to exist; shared ancestry with an absent path is not an
/// import.
pub fn adopted_base(
    git: &Git,
    integration_tip: &str,
    up: &Upstream,
    tip: &str,
    config: &Config,
) -> Result<Option<String>> {
    if let Some(path) = &up.path
        && git.object_type(integration_tip, path)?.is_none()
    {
        return Ok(None);
    }
    newest_own_contained(git, integration_tip, tip, &other_tips(git, config, up)?)
}

/// Fetched tips of every configured upstream other than `up`.
pub fn other_tips(git: &Git, config: &Config, up: &Upstream) -> Result<Vec<String>> {
    let mut tips = Vec::new();
    for other in config.upstreams.iter().filter(|o| o.name != up.name) {
        if let Some(t) = git.commit(&tracking_ref(&other.remote, &other.branch))? {
            tips.push(t);
        }
    }
    Ok(tips)
}

/// The newest commit reachable from `tip` but from none of `others` that
/// `integration_tip` contains.
///
/// Such commits are closed under ancestry within the upstream's own history,
/// so the answer is the first commit, in topological order, of
/// `tip --not others` that is missing from `tip --not others integration_tip`.
/// Two `rev-list` walks, linear in the upstream's own history. Without other
/// upstreams this is the merge base.
pub fn newest_own_contained(
    git: &Git,
    integration_tip: &str,
    tip: &str,
    others: &[String],
) -> Result<Option<String>> {
    if others.is_empty() {
        return git.merge_base(integration_tip, tip);
    }
    let mut own: Vec<String> = vec![
        "rev-list".into(),
        "--topo-order".into(),
        tip.into(),
        "--not".into(),
    ];
    own.extend(others.iter().cloned());
    let own = lines(&git.run(&own)?);
    if own.is_empty() {
        return Ok(None);
    }
    let mut pending: Vec<String> = vec!["rev-list".into(), tip.into(), "--not".into()];
    pending.extend(others.iter().cloned());
    pending.push(integration_tip.into());
    let pending: HashSet<String> = lines(&git.run(&pending)?).into_iter().collect();
    Ok(own.into_iter().find(|c| !pending.contains(c)))
}

/// Files a merge of `tip` into `integration_tip` would conflict in, predicted
/// with `git merge-tree` without touching any ref, index or worktree.
pub fn predict_conflicts(
    git: &Git,
    integration_tip: &str,
    tip: &str,
    path: Option<&str>,
) -> Result<Vec<String>> {
    let mut args: Vec<String> = vec![
        "merge-tree".into(),
        "--write-tree".into(),
        "--name-only".into(),
        "-z".into(),
        "--no-messages".into(),
    ];
    if let Some(p) = path {
        args.push("-X".into());
        args.push(format!("subtree={p}"));
    }
    args.push(integration_tip.into());
    args.push(tip.into());
    let out = git.raw(&args)?;
    match out.code {
        Some(0) => Ok(Vec::new()),
        // `<tree>\0<path>\0<path>\0\0`: the conflicted paths end at the
        // first empty entry.
        Some(1) => Ok(out
            .stdout
            .split('\0')
            .skip(1)
            .take_while(|p| !p.is_empty())
            .map(str::to_string)
            .collect()),
        _ => Err(out.into_error().into()),
    }
}
