//! The state of an update, kept in the repository's common git directory so
//! that every worktree and every shell sees the same update.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::port::Verb;

/// The branch an update is built on.
pub const UPDATE_BRANCH: &str = "git-intake/update";

/// Version 4: the update worktree lives in the main worktree, at
/// [`UPDATE_WORKTREE`], instead of in the git directory. Version 3 fetched
/// upstreams by URL into references the update holds, dropped mirror
/// branches, and took an upstream in `follow` or `port` mode. Version 2 made
/// the publication setting a table and let an update wait for its merge
/// request.
const STATE_VERSION: u32 = 4;

/// The format `migrate` carries over to [`STATE_VERSION`]. Format 3 differs
/// from format 4 only in the recorded worktree, which lay in the git
/// directory, so it reads into the same shape.
const PREVIOUS_STATE_VERSION: u32 = 3;

/// The release of this binary, as recorded in the states it starts. A build
/// from source reports `0.0.0`.
pub const RELEASE: &str = env!("CARGO_PKG_VERSION");

/// The releases that keep the update state in each format, so that a
/// refusal can name the release to finish an update with.
const FORMAT_RELEASES: &[(u64, &str)] = &[
    (1, "v0.1.0"),
    (2, "v0.2.0 to v0.2.2"),
    (3, "v0.3.0 to v0.3.1"),
    (4, "v0.3.2 and later"),
];

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

/// Prefix of the branch a merge request is opened from.
pub const MERGE_REQUEST_BRANCH_PREFIX: &str = "git-intake/update-";

/// Locations under `<git-common-dir>/git-intake/`. The update worktree is not
/// among them; see [`UPDATE_WORKTREE`].
#[derive(Debug, Clone)]
pub struct Paths {
    pub root: PathBuf,
    pub state: PathBuf,
    pub last_update: PathBuf,
    pub logs: PathBuf,
    /// The plan of a port update.
    pub plan: PathBuf,
}

impl Paths {
    pub fn new(common_dir: &Path) -> Self {
        let root = common_dir.join("git-intake");
        Self {
            state: root.join("state.json"),
            last_update: root.join("last-update.json"),
            logs: root.join("logs"),
            plan: root.join("plan"),
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
            release: Some(RELEASE.to_string()),
            release_warned: None,
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

    /// The state in the current format with its update worktree at
    /// `worktree`, for an update `migrate` carries over. The release that
    /// started it stays unrecorded: the previous format predates releases
    /// recording themselves.
    pub fn into_current(mut self, worktree: PathBuf) -> Self {
        self.version = STATE_VERSION;
        self.worktree = worktree;
        self
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

/// Reads the state of the update in progress, or `None` when there is none.
/// A state in any other format, older or newer, is refused: its update has
/// to be finished or aborted with a release that understands that format,
/// or, in the previous format, carried over by `migrate`.
pub fn load(path: &Path) -> Result<Option<UpdateState>> {
    match load_stored(path)? {
        None => Ok(None),
        Some(Stored::Current(state)) => Ok(Some(*state)),
        Some(Stored::Previous(state)) => anyhow::bail!(
            "{}",
            other_format(
                path,
                Some(u64::from(PREVIOUS_STATE_VERSION)),
                state.release.as_deref()
            )
        ),
    }
}

/// The state of the update in progress as `migrate` reads it: in the
/// current format or the previous one.
pub enum Stored {
    Current(Box<UpdateState>),
    Previous(Box<UpdateState>),
}

/// Reads the state of the update in progress in the current or the previous
/// format, or `None` when there is none. Any other format is refused as by
/// [`load`].
pub fn load_stored(path: &Path) -> Result<Option<Stored>> {
    match read(path)? {
        None => Ok(None),
        Some(Versioned::Current(state)) => Ok(Some(Stored::Current(state))),
        Some(Versioned::Previous(state)) => Ok(Some(Stored::Previous(state))),
        Some(Versioned::Other { version, release }) => {
            anyhow::bail!("{}", other_format(path, version, release.as_deref()))
        }
    }
}

/// The main worktree the update in progress recorded, in the current or the
/// previous format, so that `migrate` finds it as every other command does.
pub fn recorded_main_worktree(path: &Path) -> Option<PathBuf> {
    match load_stored(path).ok().flatten()? {
        Stored::Current(state) | Stored::Previous(state) => state.main_worktree,
    }
}

/// The state format this release writes.
pub fn current_format() -> u32 {
    STATE_VERSION
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
        format!(
            "{started} that keeps its state in format {version}; {this}, an older one. Finish or abort \
             the update with that release or a later one that understands format {version} ({path})"
        )
    } else if version == u64::from(PREVIOUS_STATE_VERSION) {
        format!(
            "{started} that keeps its state in format {version}; {this}. Run `git intake migrate` to carry \
             the update over to this release, or finish or abort it with {} ({path})",
            keeping(version)
        )
    } else {
        format!(
            "{started} that keeps its state in format {version}; {this}. Finish or abort the update with \
             {}, then start it again with this release ({path})",
            keeping(version)
        )
    }
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
    let advice = if state.finishing.is_none() && state.merge_request.is_none() {
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
    Ok(match read(path)? {
        Some(Versioned::Current(state)) => Some(*state),
        _ => None,
    })
}

enum Versioned {
    Current(Box<UpdateState>),
    Previous(Box<UpdateState>),
    /// Another format, or none, with the release that wrote it when the
    /// state records one.
    Other {
        version: Option<u64>,
        release: Option<String>,
    },
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

fn read(path: &Path) -> Result<Option<Versioned>> {
    let Some(value) = read_value(path)? else {
        return Ok(None);
    };
    let unreadable = || format!("the update state in {} is unreadable", path.display());
    // The version is checked before the shape, so that a state written by
    // another release is named as such rather than as unreadable.
    let version = value.get("version").and_then(serde_json::Value::as_u64);
    let current = version == Some(u64::from(STATE_VERSION));
    if !current && version != Some(u64::from(PREVIOUS_STATE_VERSION)) {
        let release = value
            .get("release")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string);
        return Ok(Some(Versioned::Other { version, release }));
    }
    // The previous format has the shape of the current one.
    let state = Box::new(serde_json::from_value::<UpdateState>(value).with_context(unreadable)?);
    Ok(Some(if current {
        Versioned::Current(state)
    } else {
        Versioned::Previous(state)
    }))
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
        for queued in [follow, port] {
            let state = UpdateState::new(
                config(),
                "b".repeat(40),
                dir.path().join("wt"),
                vec![queued],
            );
            save(&path, &state).unwrap();
            assert_eq!(load(&path).unwrap().unwrap(), state);
            assert!(!state.all_merged());
        }
        remove(&path).unwrap();
        remove(&path).unwrap();
        assert!(load(&path).unwrap().is_none());
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
        assert_eq!(load(&path).unwrap().unwrap(), state);
    }

    #[test]
    fn a_state_of_the_previous_version_names_migrate() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let mut state = UpdateState::new(config(), "b".repeat(40), dir.path().join("wt"), vec![]);
        state.version = PREVIOUS_STATE_VERSION;
        state.release = None;
        save(&path, &state).unwrap();
        let err = format!("{:#}", load(&path).unwrap_err());
        assert!(
            err.contains(&format!(
                "keeps its state in format 3; this release (git-intake {RELEASE}) understands format 4. \
                 Run `git intake migrate` to carry the update over to this release, or finish or abort it \
                 with a release that keeps format 3 (git-intake v0.3.0 to v0.3.1)"
            )),
            "{err}"
        );
        let Some(Stored::Previous(read)) = load_stored(&path).unwrap() else {
            panic!("not read as the previous format");
        };
        let current = read.into_current(dir.path().join("new"));
        assert_eq!(current.version, STATE_VERSION);
        assert_eq!(current.release, None);
        save(&path, &current).unwrap();
        assert_eq!(
            load(&path).unwrap().unwrap().worktree,
            dir.path().join("new")
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
                "keeps its state in format 2; this release (git-intake {RELEASE}) understands format 4"
            )),
            "{err}"
        );
        assert!(
            err.contains("with a release that keeps format 2 (git-intake v0.2.0 to v0.2.2)"),
            "{err}"
        );
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
    }

    #[test]
    fn a_state_of_a_newer_format_names_its_release() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        fs::write(&path, r#"{"version": 5, "release": "0.4.0"}"#).unwrap();
        let err = format!("{:#}", load(&path).unwrap_err());
        assert!(
            err.contains(
                "started by a release (git-intake 0.4.0) that keeps its state in format 5"
            ),
            "{err}"
        );
        assert!(
            err.contains("with that release or a later one that understands format 5"),
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
