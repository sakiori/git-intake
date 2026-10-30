//! How far each upstream is from the integration branch.

use anyhow::Result;
use serde::Serialize;

use crate::config::Upstream;
use crate::git::{Git, abbrev};
use crate::integration_line;
use crate::sync_record::Record;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DriftState {
    Current,
    Behind,
    /// A port upstream with commits nobody has decided on yet.
    Pending,
    /// A port upstream without a journal or a last synced commit to start
    /// one from.
    NoBase,
    NotImported,
    /// A follow upstream taken at its release tags without a release tag
    /// on its fetched branch.
    NoRelease,
    /// A synced path upstream whose path no longer exists on the integration branch.
    PathRemoved,
    Rewritten,
    Unavailable,
}

impl DriftState {
    pub fn label(self) -> &'static str {
        match self {
            DriftState::Current => "current",
            DriftState::Behind => "behind",
            DriftState::Pending => "pending",
            DriftState::NoBase => "no base",
            DriftState::NotImported => "not imported",
            DriftState::NoRelease => "no release",
            DriftState::PathRemoved => "path removed",
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
    /// No sync record: derived from the history, for a fork created by
    /// cloning or a path imported by other means.
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

/// Evaluates one follow upstream against the integration tip, given its
/// freshly fetched tip or why it could not be fetched.
pub fn evaluate(
    git: &Git,
    integration_tip: &str,
    up: &Upstream,
    last: Option<&Record>,
    fetched: std::result::Result<&str, &str>,
) -> Result<Drift> {
    let recorded = last.map(|r| r.target.clone());
    let tip = match fetched {
        Ok(tip) => tip.to_string(),
        Err(why) => return Ok(Drift::unavailable(recorded, why.to_string())),
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
    // An upstream is imported once a sync record exists, or when the history
    // shows a last synced commit: shared history at the root, or a merge that
    // brought the upstream into its path (`git subtree add` or `pull`).
    let imported = last_synced.is_some();
    // Precedence: unavailable and rewritten are decided above; a synced path
    // upstream whose path is gone comes before behind or current.
    let removed = match &up.path {
        Some(path) if imported => git.object_type(integration_tip, path)?.is_none(),
        _ => false,
    };
    let state = if !imported {
        DriftState::NotImported
    } else if removed {
        DriftState::PathRemoved
    } else if behind == 0 {
        DriftState::Current
    } else {
        DriftState::Behind
    };
    let detail = removed.then(|| {
        format!(
            "path '{}' no longer exists on the integration branch; restore it or correct the configuration",
            up.path.as_deref().unwrap_or_default()
        )
    });
    Ok(Drift {
        last_synced,
        last_synced_source: source,
        tip: Some(tip),
        commits_behind: Some(behind),
        state,
        detail,
    })
}

/// For an upstream without a sync record: its last synced commit as the
/// history shows it, or `None`. Depends only on the integration branch and
/// this upstream's own branch, never on other upstreams.
///
/// - A root upstream: the newest upstream commit the integration branch
///   contains (their merge base), as for a fork created by cloning.
/// - A path upstream: the second parent of the newest merge on the
///   integration line whose second parent belongs to the upstream's branch
///   and which changes the upstream's path, the merge `git subtree add` or
///   `git subtree pull` leaves.
pub fn adopted_base(
    git: &Git,
    integration_tip: &str,
    up: &Upstream,
    tip: &str,
) -> Result<Option<String>> {
    match &up.path {
        None => git.merge_base(integration_tip, tip),
        Some(path) => newest_path_merge(git, integration_tip, path, tip),
    }
}

/// The second parent of the newest merge on the integration line ending at
/// `integration_tip` (see [`integration_line`]) that changes `path` and whose
/// second parent is contained in `tip`.
///
/// The line leaves out the upstream's own history, which holds no merge of
/// this repository. One walk lists it; one `cat-file --batch-check`
/// compares each merge's tree at `path` with its first parent's; ancestry is
/// checked only for the merges that change the path, newest first, stopping
/// at the first match.
fn newest_path_merge(
    git: &Git,
    integration_tip: &str,
    path: &str,
    tip: &str,
) -> Result<Option<String>> {
    // (merge, first parent, second parent), newest first.
    let merges: Vec<(String, String, String)> =
        integration_line::walk(git, integration_tip, &[tip.to_string()])?
            .commits
            .into_iter()
            .filter_map(|c| {
                let mut parents = c.parents.into_iter();
                Some((c.commit, parents.next()?, parents.next()?))
            })
            .collect();
    if merges.is_empty() {
        return Ok(None);
    }
    let mut input = String::new();
    for (m, p1, _) in &merges {
        input.push_str(&format!("{m}:{path}\n{p1}:{path}\n"));
    }
    let objects = git.run_with_input(&["cat-file", "--batch-check=%(objectname)"], input)?;
    let objects: Vec<&str> = objects.lines().collect();
    anyhow::ensure!(
        objects.len() == merges.len() * 2,
        "unexpected output from git cat-file --batch-check"
    );
    for (i, (_, _, p2)) in merges.iter().enumerate() {
        // A missing path prints "<rev> missing"; any difference, including
        // the path appearing, counts as a change.
        let (at_merge, at_parent) = (objects[2 * i], objects[2 * i + 1]);
        if at_merge.ends_with(" missing") || at_merge == at_parent {
            continue;
        }
        if git.is_ancestor(p2, tip)? {
            return Ok(Some(p2.clone()));
        }
    }
    Ok(None)
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
