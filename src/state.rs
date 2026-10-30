//! The state of an update, kept in the repository's common git directory so
//! that every worktree and every shell sees the same update.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::config::Config;

/// The branch an update is built on.
pub const UPDATE_BRANCH: &str = "git-intake/update";

/// Version 2: the publication setting became a table, and an update can wait
/// for its merge request.
const STATE_VERSION: u32 = 2;

/// Prefix of the branch a merge request is opened from.
pub const MERGE_REQUEST_BRANCH_PREFIX: &str = "git-intake/update-";

/// Locations under `<git-common-dir>/git-intake/`.
#[derive(Debug, Clone)]
pub struct Paths {
    pub root: PathBuf,
    pub state: PathBuf,
    pub last_update: PathBuf,
    pub worktree: PathBuf,
    pub logs: PathBuf,
}

impl Paths {
    pub fn new(common_dir: &Path) -> Self {
        let root = common_dir.join("git-intake");
        Self {
            state: root.join("state.json"),
            last_update: root.join("last-update.json"),
            worktree: root.join("worktree"),
            logs: root.join("logs"),
            root,
        }
    }
}

/// How a queued upstream is merged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MergeKind {
    /// A plain merge at the repository root.
    Root,
    /// The first import of a path upstream: its tree is read under the path.
    Import,
    /// A later update of a path upstream, merged with `-X subtree=<path>`.
    Subtree,
}

/// Progress of one queued upstream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpstreamStep {
    Pending,
    Conflict,
    Merged,
}

/// Where a conflicted file's resolution came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Resolution {
    /// Not resolved yet.
    Unresolved,
    /// Resolved by git's recorded resolution (rerere).
    Recorded,
    /// Resolved by hand.
    Manual,
}

impl Resolution {
    pub fn describe(self) -> &'static str {
        match self {
            Resolution::Unresolved => "unresolved",
            Resolution::Recorded => "resolved by a recorded resolution",
            Resolution::Manual => "resolved by hand",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConflictFile {
    pub path: String,
    pub resolution: Resolution,
}

/// One upstream queued by `start`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueuedUpstream {
    pub name: String,
    /// The fixed target: the upstream tip fetched by `start`, or the commit
    /// of the revision named for the upstream.
    pub target: String,
    /// The revision named on the command line for the target, if any.
    #[serde(default)]
    pub rev: Option<String>,
    /// The last synced commit before this update, if any.
    pub previous: Option<String>,
    /// Upstream commits the base did not contain.
    pub commits: u64,
    pub kind: MergeKind,
    pub step: UpstreamStep,
    /// Update branch head before this upstream's merge began.
    pub head_before: Option<String>,
    pub merge_commit: Option<String>,
    pub conflicts: Vec<ConflictFile>,
}

/// Result of one install or check command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckOutcome {
    Passed,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckResult {
    /// `install` or `check`.
    pub kind: String,
    pub command: String,
    pub outcome: CheckOutcome,
    pub exit_code: Option<i32>,
    pub log: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Verification {
    /// The worktree commit the checks ran on.
    pub commit: String,
    pub passed: bool,
    pub results: Vec<CheckResult>,
    /// Reasons the verification failed other than a failed command.
    #[serde(default)]
    pub problems: Vec<String>,
}

/// One branch move planned by `finish`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannedMove {
    pub branch: String,
    /// Value before `finish`; `None` when the branch is created.
    pub old: Option<String>,
    pub new: String,
}

/// Progress of `finish`, persisted before any ref moves so that an
/// interrupted or rejected `finish` can be run again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finishing {
    pub moves: Vec<PlannedMove>,
    /// The error of the last rejected or failed push, if any.
    pub push_error: Option<String>,
    /// Branches `finish --no-publish` advanced locally without publishing.
    #[serde(default)]
    pub unpublished: Vec<String>,
}

/// The merge request an update waits for in merge-request mode.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergeRequest {
    /// The publish remote the branch was pushed to.
    pub remote: String,
    /// The branch on the publish remote that holds the verified commit.
    pub branch: String,
    /// The verified commit the branch points at.
    pub commit: String,
    /// Whether the push that carried the options asking the remote to open
    /// the merge request transferred the branch; when it did not, the user
    /// opens the merge request.
    pub requested: bool,
    /// The number of the merge request, when the remote's reply named it.
    /// Its head as the remote keeps it decides what the merge landed.
    #[serde(default)]
    pub number: Option<u64>,
    /// Where the merge request branch pointed when last seen on the remote.
    #[serde(default)]
    pub last_seen_head: Option<String>,
}

/// An update from `start` to `finish`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateState {
    pub version: u32,
    /// The configuration read from the integration tip at `start`.
    pub config: Config,
    /// Integration tip at `start`.
    pub base: String,
    pub branch: String,
    pub worktree: PathBuf,
    pub upstreams: Vec<QueuedUpstream>,
    pub verification: Option<Verification>,
    /// Set once `finish` has planned its ref moves.
    #[serde(default)]
    pub finishing: Option<Finishing>,
    /// Set once `finish` pushed the merge request branch in merge-request mode.
    #[serde(default)]
    pub merge_request: Option<MergeRequest>,
    /// Set once `finish` completed.
    pub finished: bool,
    /// The main worktree as `start` found it. Git cannot locate a main
    /// worktree whose git directory lies elsewhere from any other worktree,
    /// and the later commands of the update may run in one.
    #[serde(default)]
    pub main_worktree: Option<PathBuf>,
}

impl UpdateState {
    pub fn new(
        config: Config,
        base: String,
        worktree: PathBuf,
        upstreams: Vec<QueuedUpstream>,
    ) -> Self {
        Self {
            version: STATE_VERSION,
            config,
            base,
            branch: UPDATE_BRANCH.to_string(),
            worktree,
            upstreams,
            verification: None,
            finishing: None,
            merge_request: None,
            finished: false,
            main_worktree: None,
        }
    }

    /// Upstreams that are not merged yet.
    pub fn unmerged(&self) -> Vec<&QueuedUpstream> {
        self.upstreams
            .iter()
            .filter(|u| u.step != UpstreamStep::Merged)
            .collect()
    }

    pub fn all_merged(&self) -> bool {
        self.unmerged().is_empty()
    }
}

/// Reads the state of the update in progress, or `None` when there is none.
/// A state written by a release with another state format is refused: its
/// update has to be finished or aborted with that release.
pub fn load(path: &Path) -> Result<Option<UpdateState>> {
    match read(path)? {
        None => Ok(None),
        Some(Versioned::Current(state)) => Ok(Some(*state)),
        Some(Versioned::Other(version)) => anyhow::bail!(
            "the update in progress was started by a release that keeps its state in format {version}; \
             this release understands format {STATE_VERSION}. Finish or abort the update with the release \
             that started it ({})",
            path.display()
        ),
    }
}

/// Reads the record of the last finished update, or `None` when there is
/// none or it was written in another state format; it only feeds `report`.
pub fn load_finished(path: &Path) -> Result<Option<UpdateState>> {
    Ok(match read(path)? {
        Some(Versioned::Current(state)) => Some(*state),
        _ => None,
    })
}

enum Versioned {
    Current(Box<UpdateState>),
    Other(String),
}

fn read(path: &Path) -> Result<Option<Versioned>> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).with_context(|| format!("cannot read {}", path.display())),
    };
    let unreadable = || format!("the update state in {} is unreadable", path.display());
    let value: serde_json::Value = serde_json::from_str(&text).with_context(unreadable)?;
    // The version is checked before the shape, so that a state written by
    // another release is named as such rather than as unreadable.
    let version = value.get("version").and_then(serde_json::Value::as_u64);
    if version != Some(u64::from(STATE_VERSION)) {
        return Ok(Some(Versioned::Other(
            version.map_or("unknown".to_string(), |v| v.to_string()),
        )));
    }
    let state: UpdateState = serde_json::from_value(value).with_context(unreadable)?;
    Ok(Some(Versioned::Current(Box::new(state))))
}

/// Writes a state file atomically.
pub fn save(path: &Path, state: &UpdateState) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
    }
    let tmp = path.with_extension("json.tmp");
    let text = serde_json::to_string_pretty(state)?;
    fs::write(&tmp, text).with_context(|| format!("cannot write {}", tmp.display()))?;
    fs::rename(&tmp, path).with_context(|| format!("cannot write {}", path.display()))?;
    Ok(())
}

/// Removes a file if it exists.
pub fn remove(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).with_context(|| format!("cannot remove {}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config;

    #[test]
    fn round_trips_through_json() {
        let cfg = config::parse(
            "integration = \"main\"\n[[upstream]]\nname = \"up\"\nremote = \"up\"\nbranch = \"main\"\n",
        )
        .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("git-intake").join("state.json");
        assert!(load(&path).unwrap().is_none());
        let state = UpdateState::new(
            cfg,
            "b".repeat(40),
            dir.path().join("wt"),
            vec![QueuedUpstream {
                name: "up".into(),
                target: "a".repeat(40),
                rev: Some("v1.0".into()),
                previous: None,
                commits: 3,
                kind: MergeKind::Root,
                step: UpstreamStep::Conflict,
                head_before: None,
                merge_commit: None,
                conflicts: vec![ConflictFile {
                    path: "f".into(),
                    resolution: Resolution::Recorded,
                }],
            }],
        );
        save(&path, &state).unwrap();
        assert_eq!(load(&path).unwrap().unwrap(), state);
        assert!(!state.all_merged());
        remove(&path).unwrap();
        remove(&path).unwrap();
        assert!(load(&path).unwrap().is_none());
    }

    #[test]
    fn a_queued_upstream_without_a_revision_is_read() {
        let cfg = config::parse(
            "integration = \"main\"\n[[upstream]]\nname = \"up\"\nremote = \"up\"\nbranch = \"main\"\n",
        )
        .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let mut queued = QueuedUpstream {
            name: "up".into(),
            target: "a".repeat(40),
            rev: Some("v1.0".into()),
            previous: None,
            commits: 1,
            kind: MergeKind::Root,
            step: UpstreamStep::Pending,
            head_before: None,
            merge_commit: None,
            conflicts: Vec::new(),
        };
        let state = UpdateState::new(
            cfg,
            "b".repeat(40),
            dir.path().join("wt"),
            vec![queued.clone()],
        );
        // A state written before queued upstreams carried their revision.
        let mut value = serde_json::to_value(&state).unwrap();
        value["upstreams"][0].as_object_mut().unwrap().remove("rev");
        fs::write(&path, value.to_string()).unwrap();
        let loaded = load(&path).unwrap().unwrap();
        queued.rev = None;
        assert_eq!(loaded.upstreams, vec![queued]);
    }

    #[test]
    fn a_state_of_another_version_is_named_as_such() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        fs::write(
            &path,
            r#"{"version": 1, "config": {"integration": "main", "publish": "origin"}}"#,
        )
        .unwrap();
        let err = format!("{:#}", load(&path).unwrap_err());
        assert!(
            err.contains("keeps its state in format 1; this release understands format 2"),
            "{err}"
        );
        assert!(load_finished(&path).unwrap().is_none());
    }
}
