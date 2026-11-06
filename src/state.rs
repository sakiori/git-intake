//! The state of an update, kept in the repository's common git directory so
//! that every worktree and every shell sees the same update.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::config::{Config, Publish};
use crate::port::Verb;

/// The branch an update is built on.
pub const UPDATE_BRANCH: &str = "git-intake/update";

/// Version 6: the configuration an update records carries named checks,
/// and an update records a partial run of its checks beside its
/// verification. Version 5: an update catches up with a moved integration
/// branch and keeps the base it started from. Version 4: the update
/// worktree lives in the main worktree, at [`UPDATE_WORKTREE`], instead of
/// in the git directory. Version 3 fetched upstreams by URL into references
/// the update holds, dropped mirror branches, and took an upstream in
/// `follow` or `port` mode. Version 2 made the publication setting a table
/// and let an update wait for its merge request.
const STATE_VERSION: u32 = 6;

/// The formats whose update this release still reads when it waits for its
/// merge request, only to hand it over or abort it; see [`Waiting`].
const WAITING_FORMATS: [u32; 2] = [4, 5];

/// The release of this binary, as recorded in the states it starts.
pub const RELEASE: &str = env!("CARGO_PKG_VERSION");

/// The releases that keep the update state in each format, so that a
/// refusal can name the release to finish an update with.
const FORMAT_RELEASES: &[(u64, &str)] = &[
    (1, "v0.1.0"),
    (2, "v0.2.0 to v0.2.2"),
    (3, "v0.3.0 to v0.3.1"),
    (4, "v0.3.2 to v0.3.5, also carried on by v0.3.6"),
    (5, "v0.3.6"),
    (6, "from v0.3.7"),
];

/// The format whose update an earlier release carries over with `migrate`,
/// and that release, which then finishes it.
const MIGRATED_FORMAT: (u64, &str) = (3, "v0.3.6");

/// The update worktree, relative to the main worktree. It lives under
/// `.worktrees/<branch>`, where these repositories keep their worktrees, and
/// never in the git directory: tools such as compilers, bundlers and
/// watchers ignore every path that contains `/.git`, so checks run there
/// would not see the tree as it is.
pub const UPDATE_WORKTREE: &str = ".worktrees/git-intake/update";

/// The update worktree of a bare repository, which has no main worktree,
/// relative to its git directory.
pub const BARE_UPDATE_WORKTREE: &str = "git-intake/worktree";

/// The line `start` adds to the repository's `info/exclude` when the update
/// worktree is not ignored yet, so that the main checkout does not show it.
pub const WORKTREES_EXCLUDE: &str = "/.worktrees/";

/// Locations under `<git-common-dir>/git-intake/`. The update worktree is not
/// among them; see [`UPDATE_WORKTREE`].
#[derive(Debug, Clone)]
pub struct Paths {
    /// The repository's common git directory, which holds `root`.
    common: PathBuf,
    pub root: PathBuf,
    pub state: PathBuf,
    pub last_update: PathBuf,
    /// Everything the update logs: the logs of its runs of the checks and
    /// the saved output of its merges.
    pub logs: PathBuf,
    /// The logs of the verification, a full run of the checks.
    pub verify_logs: PathBuf,
    /// The logs of the partial run, kept apart from the verification's.
    pub partial_logs: PathBuf,
    /// The plan of a port update.
    pub plan: PathBuf,
}

impl Paths {
    pub fn new(common_dir: &Path) -> Self {
        let root = common_dir.join("git-intake");
        let logs = root.join("logs");
        Self {
            state: root.join("state.json"),
            last_update: root.join("last-update.json"),
            verify_logs: logs.join("verify"),
            partial_logs: logs.join("partial"),
            logs,
            plan: root.join("plan"),
            root,
            common: common_dir.to_path_buf(),
        }
    }

    /// Removes `path`, one of these locations or a file or directory in
    /// them, if it exists: a directory with what it holds, anything else
    /// itself. Returns whether something was removed. Refuses, removing
    /// nothing, a path outside the tool's directory or reached through a
    /// symbolic link or something other than a directory, or written with
    /// `..`, which could lead out of the git directory.
    pub fn remove(&self, path: &Path) -> Result<bool> {
        // `..` would lead anywhere once the path is resolved.
        if path.components().any(|c| {
            matches!(
                c,
                std::path::Component::ParentDir | std::path::Component::CurDir
            )
        }) {
            anyhow::bail!(
                "{} holds `.` or `..`, so it may lie outside the tool's directory; it was not removed",
                path.display()
            );
        }
        let Ok(inside) = path.strip_prefix(&self.common) else {
            anyhow::bail!(
                "{} lies outside the git directory {}; it was not removed",
                path.display(),
                self.common.display()
            );
        };
        if !inside.starts_with(self.root.strip_prefix(&self.common).unwrap_or(&self.root)) {
            anyhow::bail!(
                "{} lies outside {}, where the tool keeps its files; it was not removed",
                path.display(),
                self.root.display()
            );
        }
        let mut dir = self.common.clone();
        let parents: Vec<_> = inside.components().collect();
        for component in &parents[..parents.len().saturating_sub(1)] {
            dir.push(component);
            match fs::symlink_metadata(&dir) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
                Err(e) => return Err(e).with_context(|| format!("cannot read {}", dir.display())),
                Ok(meta) if meta.file_type().is_symlink() || !meta.is_dir() => anyhow::bail!(
                    "{} is {}, not a directory of the tool's own; {} was not removed",
                    dir.display(),
                    if meta.file_type().is_symlink() {
                        "a symbolic link"
                    } else {
                        "not a directory"
                    },
                    path.display()
                ),
                Ok(_) => {}
            }
        }
        let removed = match fs::symlink_metadata(path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(e) => Err(e),
            // `remove_dir_all` removes a symbolic link itself, never what
            // it points to.
            Ok(meta) if meta.is_dir() => fs::remove_dir_all(path),
            Ok(_) => fs::remove_file(path),
        };
        removed.with_context(|| format!("cannot remove {}", path.display()))?;
        Ok(true)
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
    /// A port upstream whose plan waits to be edited and carried out.
    Plan,
    Pending,
    Conflict,
    /// Merged, or for a port upstream, its plan carried out to the journal
    /// commit.
    Merged,
}

impl UpstreamStep {
    /// The step as shown to the user.
    pub fn describe(self, port: bool) -> &'static str {
        match self {
            UpstreamStep::Plan => "at its plan",
            UpstreamStep::Pending => "pending",
            UpstreamStep::Conflict => "in conflict",
            UpstreamStep::Merged if port => "carried out",
            UpstreamStep::Merged => "merged",
        }
    }
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
    /// The upstream tip `start` fetched. The update's reference to the
    /// upstream keeps it, so that the target and the planned commits stay
    /// in the repository until the update ends.
    pub fetched: String,
    /// The revision named on the command line, if any: the target of a
    /// follow upstream, the limit of a port upstream's plan.
    pub rev: Option<String>,
    pub step: UpstreamStep,
    #[serde(flatten)]
    pub work: Work,
}

/// What an update does with a queued upstream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum Work {
    Follow(FollowMerge),
    Port(PortPlan),
}

impl QueuedUpstream {
    pub fn follow(&self) -> Option<&FollowMerge> {
        match &self.work {
            Work::Follow(f) => Some(f),
            Work::Port(_) => None,
        }
    }

    pub fn port(&self) -> Option<&PortPlan> {
        match &self.work {
            Work::Port(p) => Some(p),
            Work::Follow(_) => None,
        }
    }

    /// The conflicted files of the merge or copy that stopped, if any.
    pub fn conflicts(&self) -> &[ConflictFile] {
        match &self.work {
            Work::Follow(f) => &f.conflicts,
            Work::Port(p) => p
                .stopped()
                .map(|c| c.conflicts.as_slice())
                .unwrap_or_default(),
        }
    }
}

/// The merge of a follow upstream up to its target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FollowMerge {
    /// The fixed target: the upstream tip fetched by `start`, or the commit
    /// of the revision named for the upstream.
    pub target: String,
    /// The upstream tag that points at the target, which the sync record
    /// names: the tag the revision named, the release `tags` selected, or
    /// another tag of the target.
    #[serde(default)]
    pub tag: Option<String>,
    /// The rule by which the tool chose the target, which the sync record
    /// names; `None` for a revision a person named.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chosen: Option<crate::sync_record::Chosen>,
    /// The last synced commit before this update, if any.
    pub previous: Option<String>,
    /// Upstream commits the base did not contain.
    pub commits: u64,
    pub kind: MergeKind,
    /// Update branch head before this upstream's merge began.
    pub head_before: Option<String>,
    pub merge_commit: Option<String>,
    pub conflicts: Vec<ConflictFile>,
}

impl FollowMerge {
    /// What the target is shown as named by: its upstream tag, or else the
    /// revision `start` was given.
    pub fn named_by<'a>(&'a self, q: &'a QueuedUpstream) -> Option<&'a str> {
        self.tag.as_deref().or(q.rev.as_deref())
    }
}

/// The plan of a port upstream and how far it is carried out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortPlan {
    /// The plan file `start` wrote and `continue` reads.
    pub plan: PathBuf,
    /// The journal's base, or the base of the journal the plan creates.
    pub base: String,
    /// Whether the journal exists at the update's base.
    pub journal_exists: bool,
    /// Pending commits the revision named to `start` leaves out of the plan.
    pub left_out: u64,
    /// The planned commits, oldest first.
    pub commits: Vec<PlannedCommit>,
    /// The commits marked `take`, as indexes into `commits`, in plan order.
    /// Set when `continue` reads the plan.
    pub takes: Vec<usize>,
    /// The commit that recorded the decisions in the journal, once made.
    pub journal_commit: Option<String>,
    /// Update branch head before the journal commit.
    pub journal_head_before: Option<String>,
}

impl PortPlan {
    /// The taken commit whose copy stopped on conflicts, if any.
    pub fn stopped(&self) -> Option<&PlannedCommit> {
        self.commits.iter().find(|c| c.copy == CopyStep::Conflict)
    }
}

/// One commit of a port plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannedCommit {
    pub commit: String,
    pub subject: String,
    /// A merge is copied as its change against its first parent.
    pub merge: bool,
    /// The verb the plan gives it; `None` until the plan is read.
    pub verb: Option<Verb>,
    /// The note of a `skip` or `have` decision.
    pub note: Option<String>,
    pub copy: CopyStep,
    /// Update branch head before the copy began.
    pub head_before: Option<String>,
    /// The copy on the update branch.
    pub copied: Option<String>,
    pub conflicts: Vec<ConflictFile>,
    /// What git said when the commit of the copy failed.
    pub failure: Option<String>,
    /// Paths the copy would have changed but a copy never changes:
    /// `.git-intake.toml` and `.git-intake/`.
    pub left_out: Vec<String>,
    /// Earlier planned commits still pending that changed some of the same
    /// files; set for a taken commit once the plan is carried out.
    pub overlaps: Vec<String>,
}

/// Progress of the copy of one planned commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CopyStep {
    /// Not copied: not taken, or not reached yet.
    None,
    Conflict,
    /// The copy is staged, but committing it failed, as when a hook
    /// rejects it.
    CommitFailed,
    Copied,
    /// The copy changed nothing; the commit is recorded as `have`.
    Empty,
}

/// Result of the install command or a check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckOutcome {
    Passed,
    Failed,
    /// Not run, because the install command failed or the merged
    /// configuration is broken.
    Skipped,
}

impl CheckOutcome {
    /// The outcome as the output names it.
    pub fn as_str(self) -> &'static str {
        match self {
            CheckOutcome::Passed => "passed",
            CheckOutcome::Failed => "failed",
            CheckOutcome::Skipped => "skipped",
        }
    }
}

/// How one command of a run ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandResult {
    pub outcome: CheckOutcome,
    pub exit_code: Option<i32>,
    pub log: PathBuf,
}

/// How one check of a run ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckResult {
    pub name: String,
    #[serde(flatten)]
    pub result: CommandResult,
}

/// One run of the install command and the checks on a commit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Verification {
    /// The worktree commit the commands ran on.
    pub commit: String,
    pub passed: bool,
    /// The install command's result; `None` when none is configured.
    pub install: Option<CommandResult>,
    /// The checks it ran, in the order of the update's checks.
    pub checks: Vec<CheckResult>,
    /// Reasons the run failed other than a failed command.
    pub problems: Vec<String>,
}

/// A run that left out some of the update's checks. It never counts as
/// verification, and never changes or completes the verification beside
/// which it is recorded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartialRun {
    #[serde(flatten)]
    pub run: Verification,
    /// The names of the update's checks it left out, in their order.
    pub left_out: Vec<String>,
}

/// Progress of a catch-up merge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CatchUpStep {
    /// Recorded before the merge began: the merge may be in progress, or
    /// committed without the update branch moved to it yet.
    Merging,
    Conflict,
    /// The update branch is at the catch-up merge.
    Merged,
}

/// One catch-up merge: the integration tip it takes into the update, as
/// first parent, with the update branch head as second parent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatchUp {
    /// The integration tip caught up with, recorded before the merge.
    pub tip: String,
    /// The update branch head the catch-up started from.
    pub head_before: String,
    pub step: CatchUpStep,
    pub merge_commit: Option<String>,
    pub conflicts: Vec<ConflictFile>,
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

/// The merge request an update of an earlier release waits for. Releases
/// with state formats 4 and 5 kept an update alive after `finish` opened its
/// merge request; this release hands such an update over to its merge
/// request at the next `finish`, or ends it with `abort`, and never makes
/// one.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct EarlierMergeRequest {
    /// The publish remote the branch was pushed to.
    pub remote: String,
    /// The branch on the publish remote: `git-intake/update`, or
    /// `git-intake/update-<commit>` in format 4.
    pub branch: String,
    /// The commit the earlier release last pushed to the branch.
    pub commit: String,
    /// The number of the merge request, when the remote's reply named it.
    #[serde(default)]
    pub number: Option<u64>,
}

/// An update of state format 4 or 5 that waits for its merge request, as
/// this release reads it: only what handing it over to the merge request
/// and aborting it need. Its configuration, checks and verification are
/// those of the earlier release and are not read, so that the update can be
/// handed over while the committed configuration is still in the earlier
/// form. The state is never written: it stays as the earlier release left
/// it until it is removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Waiting {
    pub integration: String,
    /// The publication setting the update recorded at `start`.
    pub publish: Option<Publish>,
    pub base: String,
    /// The integration tip at `start`.
    pub start_base: String,
    pub branch: String,
    pub worktree: PathBuf,
    pub main_worktree: Option<PathBuf>,
    /// The queued upstreams with the tip `start` fetched for each, which
    /// the update's references hold.
    pub upstreams: Vec<(String, String)>,
    /// Whether the update stopped in a catch-up merge.
    pub catching_up: bool,
    pub merge_request: EarlierMergeRequest,
}

/// The merge request whose update `sync` carried on locally, to catch it up
/// with the integration branch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CarriedMergeRequest {
    pub number: u64,
    /// Its address on the forge.
    pub url: String,
    /// The branch it comes from, which the update is pushed to.
    pub branch: String,
}

/// An update from `start` to `finish`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateState {
    pub version: u32,
    /// The release that started the update, set only by `start`. Another
    /// release of the same state format carries it on with a warning; a
    /// state without it was started before releases recorded themselves, or
    /// does not come from `start`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release: Option<String>,
    /// The release that last warned that another release started the
    /// update, so that it warns once rather than at every command.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_warned: Option<String>,
    /// The configuration read from the integration tip at `start`, with the
    /// install command and the checks read again at each catch-up merge.
    pub config: Config,
    /// The integration tip the update lands on: the tip at `start`, or the
    /// tip of the last catch-up merge.
    pub base: String,
    /// The integration tip at `start`, which upstream ranges and the
    /// repository's own changes are measured from.
    pub start_base: String,
    pub branch: String,
    pub worktree: PathBuf,
    pub upstreams: Vec<QueuedUpstream>,
    /// The last full run of the checks; only it verifies the update.
    pub verification: Option<Verification>,
    /// The last run that left out some of the checks, recorded beside the
    /// verification; see [`PartialRun`].
    #[serde(default)]
    pub partial_run: Option<PartialRun>,
    /// Set once `finish` has planned its ref moves.
    #[serde(default)]
    pub finishing: Option<Finishing>,
    /// The merge request whose update `sync` carried on, for an update it
    /// carried on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub carried_from: Option<CarriedMergeRequest>,
    /// The update head `finish` pushed to the update's branch in
    /// merge-request mode, before the merge request was opened.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub published_head: Option<String>,
    /// The catch-up merges of the update, oldest first; the last one may be
    /// in progress.
    #[serde(default)]
    pub catch_ups: Vec<CatchUp>,
    /// The publish remote's integration tip as the update last took it:
    /// at `start`, when `finish` fetched it, or when the update caught up
    /// with it. A remote branch that no longer contains it was rewritten.
    #[serde(default)]
    pub remote_tip: Option<String>,
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
            release: Some(RELEASE.to_string()),
            release_warned: None,
            config,
            start_base: base.clone(),
            base,
            branch: UPDATE_BRANCH.to_string(),
            worktree,
            upstreams,
            verification: None,
            partial_run: None,
            finishing: None,
            carried_from: None,
            published_head: None,
            catch_ups: Vec::new(),
            remote_tip: None,
            finished: false,
            main_worktree: None,
        }
    }

    /// The integration tip at `start`.
    pub fn start_base(&self) -> &str {
        &self.start_base
    }

    /// The catch-up merge in progress: recorded, but the update branch not
    /// yet moved to it.
    pub fn catching_up(&self) -> Option<&CatchUp> {
        self.catch_ups
            .last()
            .filter(|c| c.step != CatchUpStep::Merged)
    }

    /// Whether a full run of the checks passed on `head`; a partial run
    /// never counts.
    pub fn verified_at(&self, head: &str) -> bool {
        self.verification
            .as_ref()
            .is_some_and(|v| v.passed && v.commit == head)
    }

    /// Upstreams that are not merged, or whose plan is not carried out, yet.
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

/// The update in progress as the state file holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stored {
    /// An update in this release's format.
    Update(Box<UpdateState>),
    /// An update of an earlier release that waits for its merge request.
    Waiting(Box<Waiting>),
}

impl Stored {
    /// The update worktree the state records.
    pub fn worktree(&self) -> &Path {
        match self {
            Stored::Update(s) => &s.worktree,
            Stored::Waiting(w) => &w.worktree,
        }
    }

    /// The main worktree the state records.
    pub fn main_worktree(&self) -> Option<&Path> {
        match self {
            Stored::Update(s) => s.main_worktree.as_deref(),
            Stored::Waiting(w) => w.main_worktree.as_deref(),
        }
    }

    /// The names of the queued upstreams, whose references the update holds.
    pub fn upstream_names(&self) -> Box<dyn Iterator<Item = &str> + '_> {
        match self {
            Stored::Update(s) => Box::new(s.upstreams.iter().map(|q| q.name.as_str())),
            Stored::Waiting(w) => Box::new(w.upstreams.iter().map(|(n, _)| n.as_str())),
        }
    }

    /// The references the update holds, with the value each one must keep.
    pub fn held(&self) -> std::collections::HashMap<String, String> {
        match self {
            Stored::Update(s) => crate::upstreams::held_by(s),
            Stored::Waiting(w) => w
                .upstreams
                .iter()
                .map(|(name, tip)| (crate::upstreams::upstream_ref(name), tip.clone()))
                .collect(),
        }
    }

    /// The integration branch of the update.
    pub fn integration(&self) -> &str {
        match self {
            Stored::Update(s) => &s.config.integration,
            Stored::Waiting(w) => &w.integration,
        }
    }
}

/// Reads the state of the update in progress, or `None` when there is none.
/// An update of format 4 or 5 is read only when it waits for its merge
/// request, as a [`Waiting`]; any other state in another format, older or
/// newer, is refused: its update has to be finished or aborted with a
/// release that understands that format.
pub fn load(path: &Path) -> Result<Option<Stored>> {
    let Some(value) = read_value(path)? else {
        return Ok(None);
    };
    let unreadable = || format!("the update state in {} is unreadable", path.display());
    // The version is checked before the shape, so that a state written by
    // another release is named as such rather than as unreadable.
    let version = value.get("version").and_then(serde_json::Value::as_u64);
    let release = value
        .get("release")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    if version == Some(u64::from(STATE_VERSION)) {
        let state = serde_json::from_value::<UpdateState>(value).with_context(unreadable)?;
        return Ok(Some(Stored::Update(Box::new(state))));
    }
    if let Some(v) = version.and_then(|v| u32::try_from(v).ok())
        && WAITING_FORMATS.contains(&v)
        && value.get("merge_request").is_some_and(|m| !m.is_null())
    {
        let raw = serde_json::from_value::<RawWaiting>(value).with_context(unreadable)?;
        return Ok(Some(Stored::Waiting(Box::new(raw.into_waiting()))));
    }
    anyhow::bail!("{}", other_format(path, version, release.as_deref()))
}

/// The update in progress for the commands that carry it on, or `None` when
/// there is none. An update of an earlier release waiting for its merge
/// request is refused, naming `finish`, which hands it over.
pub fn load_update(path: &Path) -> Result<Option<UpdateState>> {
    match load(path)? {
        None => Ok(None),
        Some(Stored::Update(state)) => Ok(Some(*state)),
        Some(Stored::Waiting(w)) => anyhow::bail!("{}", w.refusal()),
    }
}

/// The main worktree the update in progress recorded, so that every command
/// finds it, a waiting update's included.
pub fn recorded_main_worktree(path: &Path) -> Option<PathBuf> {
    load(path)
        .ok()
        .flatten()
        .and_then(|s| s.main_worktree().map(Path::to_path_buf))
}

/// A format 4 or 5 state as [`Waiting`] reads it; every other field of the
/// earlier release is ignored.
#[derive(Deserialize)]
struct RawWaiting {
    config: RawWaitingConfig,
    base: String,
    /// Format 4 did not record it; its base, which never moved, stands in.
    #[serde(default)]
    start_base: Option<String>,
    branch: String,
    worktree: PathBuf,
    #[serde(default)]
    main_worktree: Option<PathBuf>,
    upstreams: Vec<RawHeld>,
    #[serde(default)]
    catch_ups: Vec<RawCatchUp>,
    merge_request: EarlierMergeRequest,
}

#[derive(Deserialize)]
struct RawWaitingConfig {
    integration: String,
    publish: Option<Publish>,
}

#[derive(Deserialize)]
struct RawHeld {
    name: String,
    fetched: String,
}

#[derive(Deserialize)]
struct RawCatchUp {
    step: CatchUpStep,
}

impl RawWaiting {
    fn into_waiting(self) -> Waiting {
        Waiting {
            integration: self.config.integration,
            publish: self.config.publish,
            start_base: self.start_base.unwrap_or_else(|| self.base.clone()),
            base: self.base,
            branch: self.branch,
            worktree: self.worktree,
            main_worktree: self.main_worktree,
            upstreams: self
                .upstreams
                .into_iter()
                .map(|u| (u.name, u.fetched))
                .collect(),
            catching_up: self
                .catch_ups
                .last()
                .is_some_and(|c| c.step != CatchUpStep::Merged),
            merge_request: self.merge_request,
        }
    }
}

impl Waiting {
    /// What `finish` and `abort` do with the update.
    pub fn next(&self) -> String {
        format!(
            "`git intake finish` hands the update over to its merge request from '{}', which an earlier release left it \
             waiting for; `git intake abort` ends it here and keeps the merge request, and `git intake abort \
             --merge-request` also withdraws the merge request",
            self.merge_request.branch
        )
    }

    /// Why a command other than `finish` and `abort` does not carry it on.
    pub fn refusal(&self) -> String {
        format!(
            "the update in progress waits for its merge request from '{}', as an earlier release left it; {}",
            self.merge_request.branch,
            self.next()
        )
    }
}

/// Why a state of another format is refused, and what to do about it.
fn other_format(path: &Path, version: Option<u64>, release: Option<&str>) -> String {
    let this = format!("this release (git-intake {RELEASE}) understands format {STATE_VERSION}");
    let path = path.display();
    let Some(version) = version else {
        return format!(
            "the update state in {path} names no state format; {this}. Finish or abort the update \
             with the release that started it"
        );
    };
    let started = match release {
        Some(r) => format!("the update in progress was started by a release (git-intake {r})"),
        None => "the update in progress was started by a release".to_string(),
    };
    if version > u64::from(STATE_VERSION) {
        return format!(
            "{started} that keeps its state in format {version}; {this}, an older one. Finish or abort \
             the update with that release or a later one that understands format {version} ({path})"
        );
    }
    let carried = if version == MIGRATED_FORMAT.0 {
        format!(
            ", or carry it over with `git intake migrate` of git-intake {} and finish or abort it with that release",
            MIGRATED_FORMAT.1
        )
    } else {
        String::new()
    };
    format!(
        "{started} that keeps its state in format {version}; {this}. Finish or abort the update with \
         {}{carried}, then start it again with this release ({path})",
        keeping(version)
    )
}

/// The releases that keep the state in `version`, as a refusal names them.
fn keeping(version: u64) -> String {
    match FORMAT_RELEASES.iter().find(|(f, _)| *f == version) {
        Some((_, releases)) => {
            format!("a release that keeps format {version} (git-intake {releases})")
        }
        None => "the release that started it".to_string(),
    }
}

/// A warning when another release of the same state format carries on the
/// update and has not warned yet: the update keeps working, but what each
/// release does may differ. Starting over is advised only before `finish`:
/// once it moved a branch or published a merge request, `abort` would undo
/// or withdraw work that is already out.
pub fn release_warning(state: &UpdateState) -> Option<String> {
    let started = state.release.as_deref()?;
    if started == RELEASE || state.release_warned.as_deref() == Some(RELEASE) {
        return None;
    }
    let advice = if state.finishing.is_none()
        && state.carried_from.is_none()
        && state.published_head.is_none()
    {
        " To use one release throughout, run `git intake abort` and start the update again."
    } else {
        ""
    };
    Some(format!(
        "the update was started by git-intake {started} and is carried on by git-intake {RELEASE}; \
         both understand its state, but they may differ in what they do.{advice}"
    ))
}

/// Reads the record of the last finished update, or `None` when there is
/// none or it was written in another state format; it only feeds `report`.
pub fn load_finished(path: &Path) -> Result<Option<UpdateState>> {
    let Some(value) = read_value(path)? else {
        return Ok(None);
    };
    if value.get("version").and_then(serde_json::Value::as_u64) != Some(u64::from(STATE_VERSION)) {
        return Ok(None);
    }
    serde_json::from_value(value)
        .map(Some)
        .with_context(|| format!("the update record in {} is unreadable", path.display()))
}

/// The state file as JSON, whatever its format, or `None` when there is none.
fn read_value(path: &Path) -> Result<Option<serde_json::Value>> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).with_context(|| format!("cannot read {}", path.display())),
    };
    let value = serde_json::from_str(&text)
        .with_context(|| format!("the update state in {} is unreadable", path.display()))?;
    Ok(Some(value))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config;

    fn config() -> Config {
        config::parse(
            "integration = \"main\"\n[[upstream]]\nname = \"up\"\nurl = \"../up.git\"\nbranch = \"main\"\n",
        )
        .unwrap()
    }

    #[test]
    fn round_trips_through_json() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("git-intake").join("state.json");
        assert!(load(&path).unwrap().is_none());
        let follow = QueuedUpstream {
            name: "up".into(),
            fetched: "c".repeat(40),
            rev: Some("v1.0".into()),
            step: UpstreamStep::Conflict,
            work: Work::Follow(FollowMerge {
                target: "a".repeat(40),
                tag: Some("v1.0".into()),
                chosen: None,
                previous: None,
                commits: 3,
                kind: MergeKind::Root,
                head_before: None,
                merge_commit: None,
                conflicts: vec![ConflictFile {
                    path: "f".into(),
                    resolution: Resolution::Recorded,
                }],
            }),
        };
        let port = QueuedUpstream {
            name: "up".into(),
            fetched: "c".repeat(40),
            rev: None,
            step: UpstreamStep::Plan,
            work: Work::Port(PortPlan {
                plan: dir.path().join("plan"),
                base: "d".repeat(40),
                journal_exists: false,
                left_out: 0,
                commits: vec![PlannedCommit {
                    commit: "e".repeat(40),
                    subject: "fix".into(),
                    merge: false,
                    verb: Some(Verb::Take),
                    note: None,
                    copy: CopyStep::Conflict,
                    head_before: None,
                    copied: None,
                    conflicts: Vec::new(),
                    failure: None,
                    left_out: Vec::new(),
                    overlaps: Vec::new(),
                }],
                takes: vec![0],
                journal_commit: None,
                journal_head_before: None,
            }),
        };
        let run = |checks: &[&str]| Verification {
            commit: "b".repeat(40),
            passed: false,
            install: Some(CommandResult {
                outcome: CheckOutcome::Passed,
                exit_code: Some(0),
                log: dir.path().join("install.log"),
            }),
            checks: checks
                .iter()
                .map(|name| CheckResult {
                    name: name.to_string(),
                    result: CommandResult {
                        outcome: CheckOutcome::Failed,
                        exit_code: Some(1),
                        log: dir.path().join(format!("check-{name}.log")),
                    },
                })
                .collect(),
            problems: vec!["moved".into()],
        };
        for queued in [follow, port] {
            let mut state = UpdateState::new(
                config(),
                "b".repeat(40),
                dir.path().join("wt"),
                vec![queued],
            );
            state.verification = Some(run(&["lint", "test"]));
            state.partial_run = Some(PartialRun {
                run: run(&["lint"]),
                left_out: vec!["test".into()],
            });
            save(&path, &state).unwrap();
            let written: serde_json::Value =
                serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
            assert_eq!(written["version"], 6);
            assert_eq!(
                written["partial_run"]["left_out"],
                serde_json::json!(["test"])
            );
            assert_eq!(written["partial_run"]["checks"][0]["name"], "lint");
            assert_eq!(load_update(&path).unwrap().unwrap(), state);
            assert!(!state.all_merged());
        }
        let paths = Paths::new(dir.path());
        assert_eq!(paths.state, path);
        assert!(paths.remove(&path).unwrap());
        assert!(!paths.remove(&path).unwrap());
        assert!(load(&path).unwrap().is_none());
    }

    #[test]
    fn only_the_tool_s_own_files_are_removed() {
        let dir = tempfile::tempdir().unwrap();
        let common = dir.path().join("repo.git");
        let elsewhere = dir.path().join("elsewhere");
        fs::create_dir_all(elsewhere.join("logs")).unwrap();
        fs::write(elsewhere.join("logs").join("kept.log"), "kept\n").unwrap();
        fs::create_dir_all(&common).unwrap();
        let paths = Paths::new(&common);
        // The tool's directory is a link to another.
        std::os::unix::fs::symlink(&elsewhere, &paths.root).unwrap();
        let err = paths.remove(&paths.logs).unwrap_err().to_string();
        assert!(err.contains("a symbolic link"), "{err}");
        assert!(elsewhere.join("logs").join("kept.log").exists());
        // Nothing outside the tool's directory.
        fs::remove_file(&paths.root).unwrap();
        fs::create_dir_all(&paths.root).unwrap();
        assert!(paths.remove(&elsewhere).is_err());
        assert!(paths.remove(&common.join("HEAD")).is_err());
        let escaping = paths.root.join("..").join("..").join("elsewhere");
        let err = paths.remove(&escaping).unwrap_err().to_string();
        assert!(err.contains("not removed"), "{err}");
        assert!(elsewhere.exists());
        // A link inside it is removed itself, not what it points to.
        std::os::unix::fs::symlink(&elsewhere, &paths.logs).unwrap();
        assert!(paths.remove(&paths.logs).unwrap());
        assert!(elsewhere.join("logs").join("kept.log").exists());
    }

    #[test]
    fn a_state_without_tags_reads_without_them() {
        // Releases before upstream tags were recorded write neither the
        // `tags` setting nor a follow target's tag.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let state = UpdateState::new(
            config(),
            "b".repeat(40),
            dir.path().join("wt"),
            vec![QueuedUpstream {
                name: "up".into(),
                fetched: "c".repeat(40),
                rev: None,
                step: UpstreamStep::Pending,
                work: Work::Follow(FollowMerge {
                    target: "a".repeat(40),
                    tag: None,
                    chosen: None,
                    previous: None,
                    commits: 1,
                    kind: MergeKind::Root,
                    head_before: None,
                    merge_commit: None,
                    conflicts: Vec::new(),
                }),
            }],
        );
        let mut value = serde_json::to_value(&state).unwrap();
        value["upstreams"][0].as_object_mut().unwrap().remove("tag");
        value["config"]["upstreams"][0]
            .as_object_mut()
            .unwrap()
            .remove("tags");
        fs::write(&path, value.to_string()).unwrap();
        assert_eq!(load_update(&path).unwrap().unwrap(), state);
    }

    /// `state` as an earlier release of `version` wrote it: checks as plain
    /// commands and a verification of that release.
    fn earlier(state: &UpdateState, version: u64) -> serde_json::Value {
        let mut value = serde_json::to_value(state).unwrap();
        value["version"] = version.into();
        value["config"]["checks"] = serde_json::json!(["make lint", "make test"]);
        value["verification"] = serde_json::json!({
            "commit": "b".repeat(40),
            "passed": true,
            "results": [{"kind": "check", "command": "make lint", "outcome": "passed", "exit_code": 0, "log": "l"}],
        });
        value.as_object_mut().unwrap().remove("partial_run");
        value
    }

    #[test]
    fn a_waiting_update_of_format_4_or_5_is_read_for_its_hand_over() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let mut state = UpdateState::new(
            config(),
            "b".repeat(40),
            dir.path().join("wt"),
            vec![QueuedUpstream {
                name: "up".into(),
                fetched: "c".repeat(40),
                rev: None,
                step: UpstreamStep::Merged,
                work: Work::Follow(FollowMerge {
                    target: "a".repeat(40),
                    tag: None,
                    chosen: None,
                    previous: None,
                    commits: 1,
                    kind: MergeKind::Root,
                    head_before: None,
                    merge_commit: None,
                    conflicts: Vec::new(),
                }),
            }],
        );
        state.config.publish = Some(Publish {
            remote: "origin".into(),
            via: crate::config::PublishVia::MergeRequest,
            forge: None,
            api: None,
            queue: None,
            replace_after: None,
        });
        state.release = Some("0.3.5".into());
        for version in [4, 5] {
            let mut value = earlier(&state, version);
            // Format 4 knew neither the starting base nor catch-ups.
            if version == 4 {
                let object = value.as_object_mut().unwrap();
                object.remove("start_base");
                object.remove("catch_ups");
            } else {
                // Format 5 records the base it started from beside the one a
                // catch-up moved it to.
                value["start_base"] = "d".repeat(40).into();
            }
            value["merge_request"] = serde_json::json!({
                "remote": "origin",
                "branch": "git-intake/update-bbbbbbbbbbbb",
                "commit": "c".repeat(40),
                "requested": true,
            });
            let text = value.to_string();
            fs::write(&path, &text).unwrap();
            let Some(Stored::Waiting(w)) = load(&path).unwrap() else {
                panic!("format {version} not read as waiting");
            };
            assert_eq!(w.integration, "main");
            assert_eq!(w.publish, state.config.publish);
            assert_eq!(w.upstreams, [("up".to_string(), "c".repeat(40))]);
            assert_eq!(w.merge_request.branch, "git-intake/update-bbbbbbbbbbbb");
            assert_eq!(w.merge_request.number, None);
            assert!(!w.catching_up);
            assert_eq!(w.base, "b".repeat(40));
            let started = if version == 4 { 'b' } else { 'd' };
            assert_eq!(w.start_base, started.to_string().repeat(40));
            assert!(w.refusal().contains("as an earlier release left it"));
            // Every command that carries an update on refuses it.
            let err = format!("{:#}", load_update(&path).unwrap_err());
            assert!(
                err.contains("`git intake finish` hands the update over"),
                "{err}"
            );
            assert_eq!(fs::read_to_string(&path).unwrap(), text);
        }
    }

    #[test]
    fn an_earlier_format_that_does_not_wait_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let mut state = UpdateState::new(config(), "b".repeat(40), dir.path().join("wt"), vec![]);
        state.release = Some("0.3.6".into());
        for (version, releases) in [
            (5, "a release that keeps format 5 (git-intake v0.3.6)"),
            (
                4,
                "a release that keeps format 4 (git-intake v0.3.2 to v0.3.5, also carried on by v0.3.6)",
            ),
        ] {
            let mut value = earlier(&state, version);
            value["merge_request"] = serde_json::Value::Null;
            fs::write(&path, value.to_string()).unwrap();
            let err = format!("{:#}", load(&path).unwrap_err());
            assert!(
                err.contains(&format!(
                    "started by a release (git-intake 0.3.6) that keeps its state in format {version}; \
                     this release (git-intake {RELEASE}) understands format 6. Finish or abort the update with {releases}, \
                     then start it again with this release"
                )),
                "{err}"
            );
        }
    }

    #[test]
    fn a_state_of_format_3_names_migrate_of_its_last_release() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let state = UpdateState::new(config(), "b".repeat(40), dir.path().join("wt"), vec![]);
        let mut value = earlier(&state, 3);
        value.as_object_mut().unwrap().remove("release");
        fs::write(&path, value.to_string()).unwrap();
        let err = format!("{:#}", load(&path).unwrap_err());
        assert!(
            err.contains(&format!(
                "keeps its state in format 3; this release (git-intake {RELEASE}) understands format 6. \
                 Finish or abort the update with a release that keeps format 3 (git-intake v0.3.0 to v0.3.1), \
                 or carry it over with `git intake migrate` of git-intake v0.3.6 and finish or abort it with that release"
            )),
            "{err}"
        );
    }

    #[test]
    fn a_state_of_another_version_is_named_as_such() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        fs::write(
            &path,
            r#"{"version": 2, "config": {"integration": "main", "upstreams": [{"remote": "up", "mirror": "m"}]}}"#,
        )
        .unwrap();
        let err = format!("{:#}", load(&path).unwrap_err());
        assert!(
            err.contains(&format!(
                "keeps its state in format 2; this release (git-intake {RELEASE}) understands format 6"
            )),
            "{err}"
        );
        assert!(
            err.contains("with a release that keeps format 2 (git-intake v0.2.0 to v0.2.2)"),
            "{err}"
        );
        assert!(!err.contains("migrate"), "{err}");
        assert!(load_finished(&path).unwrap().is_none());
    }

    #[test]
    fn every_format_up_to_this_one_names_its_releases() {
        for format in 1..=u64::from(STATE_VERSION) {
            assert!(
                FORMAT_RELEASES.iter().any(|(f, _)| *f == format),
                "{format}"
            );
        }
        assert_eq!(
            FORMAT_RELEASES.last().map(|(f, _)| *f),
            Some(u64::from(STATE_VERSION))
        );
    }

    #[test]
    fn a_state_of_a_newer_format_names_its_release() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        fs::write(&path, r#"{"version": 7, "release": "0.4.0"}"#).unwrap();
        let err = format!("{:#}", load(&path).unwrap_err());
        assert!(
            err.contains(
                "started by a release (git-intake 0.4.0) that keeps its state in format 7"
            ),
            "{err}"
        );
        assert!(
            err.contains("with that release or a later one that understands format 7"),
            "{err}"
        );
    }

    #[test]
    fn another_release_of_the_same_format_is_warned_about() {
        let mut state = UpdateState::new(config(), "b".repeat(40), PathBuf::from("wt"), vec![]);
        assert_eq!(state.release.as_deref(), Some(RELEASE));
        assert_eq!(release_warning(&state), None);
        state.release = Some("0.0.1-other".to_string());
        let warning = release_warning(&state).unwrap();
        assert!(
            warning.starts_with("the update was started by git-intake 0.0.1-other"),
            "{warning}"
        );
        assert!(warning.contains("`git intake abort`"), "{warning}");
        // Once `finish` began, starting over is not advised.
        state.finishing = Some(Finishing {
            moves: Vec::new(),
            push_error: None,
            unpublished: Vec::new(),
        });
        assert!(!release_warning(&state).unwrap().contains("abort"));
        // A release warns once.
        state.release_warned = Some(RELEASE.to_string());
        assert_eq!(release_warning(&state), None);
        state.release_warned = None;
        // A state from before releases recorded themselves names none.
        state.release = None;
        assert_eq!(release_warning(&state), None);
    }
}
