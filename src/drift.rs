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

#[derive(Debug, Clone, Serialize)]
pub struct Drift {
    pub last_synced: Option<String>,
    pub tip: Option<String>,
    pub commits_behind: Option<u64>,
    pub state: DriftState,
    pub detail: Option<String>,
}

impl Drift {
    fn unavailable(last_synced: Option<String>, detail: String) -> Self {
        Self {
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
    let last_synced = last.map(|r| r.target.clone());
    let tip = match fetched_tip(git, up)? {
        Ok(tip) => tip,
        Err(why) => return Ok(Drift::unavailable(last_synced, why)),
    };
    if let Some(ls) = &last_synced
        && (!git.object_exists(ls)? || !git.is_ancestor(ls, &tip)?)
    {
        return Ok(Drift {
            detail: Some(format!(
                "{} no longer contains the last synced commit {}",
                up.source(),
                abbrev(ls)
            )),
            last_synced,
            tip: Some(tip),
            commits_behind: None,
            state: DriftState::Rewritten,
        });
    }
    let behind = git.count(integration_tip, &tip)?;
    let state = if behind == 0 {
        DriftState::Current
    } else if last_synced.is_none() && git.merge_base(integration_tip, &tip)?.is_none() {
        DriftState::NotImported
    } else {
        DriftState::Behind
    };
    Ok(Drift {
        last_synced,
        tip: Some(tip),
        commits_behind: Some(behind),
        state,
        detail: None,
    })
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
