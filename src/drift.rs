//! How far each upstream is from the integration branch.

use anyhow::Result;
use serde::Serialize;

use crate::config::Upstream;
use crate::git::{Git, abbrev, tracking_ref};
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
        None => match adopted_base(git, integration_tip, up, &tip)? {
            Some(mb) => (Some(mb), Some(SyncSource::History)),
            None => (None, None),
        },
    };
    let behind = git.count(integration_tip, &tip)?;
    // An upstream is imported once a sync record exists, or when the
    // integration branch contains part of its history: at the root, or at a
    // path that exists (an import by other means, such as `git subtree add`).
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

/// For an upstream without a sync record whose history is partly contained
/// in the integration branch: the newest upstream commit it contains. A root
/// upstream needs only shared history (for example a fork created by cloning);
/// a path upstream also needs its path to exist, since shared ancestry with an
/// absent path (such as a common template) is not an import.
pub fn adopted_base(
    git: &Git,
    integration_tip: &str,
    up: &Upstream,
    tip: &str,
) -> Result<Option<String>> {
    if let Some(path) = &up.path
        && git.object_type(integration_tip, path)?.is_none()
    {
        return Ok(None);
    }
    git.merge_base(integration_tip, tip)
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
        Some(1) => Ok(out
            .stdout
            .lines()
            .skip(1)
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_string)
            .collect()),
        _ => Err(out.into_error().into()),
    }
}
