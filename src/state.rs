//! The state of an update, kept in the repository's common git directory so
//! that every worktree and every shell sees the same update.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::config::Config;

/// The branch an update is built on.
pub const UPDATE_BRANCH: &str = "fork-sync/update";

const STATE_VERSION: u32 = 1;

/// Locations under `<git-common-dir>/fork-sync/`.
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
        let root = common_dir.join("fork-sync");
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
    /// The fixed target: the upstream tip fetched by `start`.
    pub target: String,
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
    /// Set once `finish` advanced the integration branch.
    pub finished: bool,
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
            finished: false,
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

/// Reads a state file, or `None` when it does not exist.
pub fn load(path: &Path) -> Result<Option<UpdateState>> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).with_context(|| format!("cannot read {}", path.display())),
    };
    let state: UpdateState = serde_json::from_str(&text)
        .with_context(|| format!("the update state in {} is unreadable", path.display()))?;
    anyhow::ensure!(
        state.version == STATE_VERSION,
        "the update state in {} has version {}, this tool understands version {STATE_VERSION}",
        path.display(),
        state.version
    );
    Ok(Some(state))
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
        let path = dir.path().join("fork-sync").join("state.json");
        assert!(load(&path).unwrap().is_none());
        let state = UpdateState::new(
            cfg,
            "b".repeat(40),
            dir.path().join("wt"),
            vec![QueuedUpstream {
                name: "up".into(),
                target: "a".repeat(40),
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
}
