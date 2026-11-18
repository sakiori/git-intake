//! The forge that hosts the publish remote, GitLab or GitHub, reached
//! through its REST API.
//!
//! Every command uses the [`Forge`] interface only: it finds the update's
//! merge requests, opens one, rewrites its title and description, closes
//! it, reads whether people approved, reviewed or commented on one, reads
//! whether the project merges only a tree its required pipeline tested,
//! reads whether a release exists for a tag, creates a release tag and
//! tells whether the token may, reads the pipeline of a merge request's
//! head, merges it, and reads and changes the assignments a merge queue
//! works by. [`gitlab`] and [`github`] implement it over
//! [`http`].

pub mod github;
pub mod gitlab;
pub mod http;
pub mod locate;

use std::time::Duration;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::config::Publish;
use crate::git::Git;
use crate::output::{esay, say};

pub use locate::{Access, Location, TOKEN_VARIABLES};

/// A kind of forge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    GitLab,
    GitHub,
}

impl Kind {
    /// The forge's name.
    pub fn name(self) -> &'static str {
        match self {
            Kind::GitLab => "GitLab",
            Kind::GitHub => "GitHub",
        }
    }

    /// What the forge calls a merge request.
    pub fn term(self) -> &'static str {
        match self {
            Kind::GitLab => "merge request",
            Kind::GitHub => "pull request",
        }
    }

    /// A merge request's number as the forge writes it, such as `!7` or `#7`.
    pub fn reference(self, number: u64) -> String {
        match self {
            Kind::GitLab => format!("!{number}"),
            Kind::GitHub => format!("#{number}"),
        }
    }
}

/// Where a merge request stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Open,
    Merged,
    /// Closed without merging.
    Closed,
}

impl State {
    pub fn describe(self) -> &'static str {
        match self {
            State::Open => "open",
            State::Merged => "merged",
            State::Closed => "closed without merging",
        }
    }
}

/// A merge request or pull request as the forge reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeRequest {
    pub number: u64,
    /// Its address on the forge.
    pub url: String,
    pub state: State,
    /// Its head commit: the source branch's tip while it is open, and the
    /// final head once it is merged or closed.
    pub head: String,
    pub source: String,
    pub target: String,
    pub description: String,
    /// When it was opened.
    pub created_at: Option<jiff::Timestamp>,
    /// When it was merged or closed.
    pub ended_at: Option<jiff::Timestamp>,
    /// Whether it is marked as a draft; `None` when the forge's answer does
    /// not say.
    pub draft: Option<bool>,
    /// Whether someone is assigned to it; `None` when the forge's answer
    /// does not say.
    pub assigned: Option<bool>,
    /// Whether it is set to merge automatically; `None` when the forge's
    /// answer does not say.
    pub auto_merge: Option<bool>,
    /// Whether anyone is asked to review it, a person or a team; `None`
    /// when the forge's answer does not say.
    pub review_requested: Option<bool>,
    /// GitLab: whether it is `locked`, the moment of its merge.
    pub locked: bool,
    /// GitLab: whether someone set a time after which it may be merged,
    /// `merge_after`, which instances before the field never set.
    pub merge_scheduled: bool,
}

/// Whether people left a trace of one kind on a merge request, or, as
/// `Err`, why the forge did not let the token read it.
pub type Signal = std::result::Result<bool, String>;

/// A time the forge writes, in RFC 3339, as an instant; `None` when absent
/// or unreadable.
fn instant(value: &serde_json::Value, key: &str) -> Option<jiff::Timestamp> {
    optional_text(value, key).and_then(|t| t.parse().ok())
}

/// Orders merge requests by when they ended, the one that ended most
/// recently first and those that did not end last.
pub fn latest_first(list: &mut [MergeRequest]) {
    list.sort_by_key(|m| std::cmp::Reverse(m.ended_at));
}

/// What the forge's settings mean for an update's merge request.
#[derive(Debug, Clone, Default)]
pub struct Review {
    /// Each setting to change before the forge merges only a tree its
    /// required pipeline tested, or each requirement the token could not
    /// confirm, with the ways to let it.
    pub refusals: Vec<String>,
    /// Merges that stay selectable and drop the update's history, where the
    /// tool cannot ask the forge not to make them.
    pub warnings: Vec<String>,
    /// Settings worth knowing that ask for no change.
    pub notes: Vec<String>,
    /// Where the forge reads its CI definition, relative to the repository
    /// root; a path ending in `/` is a directory.
    pub ci_paths: Vec<String>,
}

/// Whether the forge has a release for a tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReleaseLookup {
    Published,
    Unpublished,
    /// The forge does not show the project to the credential sent, so its
    /// releases cannot be read.
    Hidden,
}

/// An annotated tag to create on the forge.
#[derive(Debug, Clone, Copy)]
pub struct NewTag<'a> {
    pub name: &'a str,
    pub commit: &'a str,
    pub message: &'a str,
    /// GitHub: the tagger's name and email. GitLab takes none and writes
    /// the token's account.
    pub tagger: Option<(&'a str, &'a str)>,
}

/// What became of a request to create a tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TagCreation {
    Created,
    /// The forge has a tag of that name already.
    Taken,
}

/// Whether the token may create a tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TagAccess {
    /// It may, as far as the forge's answers tell.
    Allowed {
        /// The token's account, when the forge names it.
        account: Option<String>,
        /// What the forge's answers leave unproven.
        unproven: Option<String>,
    },
    /// It may not, for the reason given.
    Denied(String),
}

/// The outcome of a pipeline, as far as it has one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipelineState {
    /// Created, waiting or running: no outcome yet.
    Running,
    Passed,
    /// Failed, or skipped, which tests nothing.
    Failed,
    Canceled,
}

/// The pipeline of a merge request's head.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pipeline {
    pub state: PipelineState,
    /// Its address on the forge.
    pub url: String,
}

/// What became of a request to merge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Merge {
    /// The forge merged it, or took it to merge.
    Accepted,
    /// The head moved away from the commit named.
    HeadMoved,
    /// The forge is still deciding whether it can be merged.
    NotYet,
    /// The forge refused, saying why, though it reads the merge request as
    /// mergeable, as it may while another process holds the merge.
    Contested(String),
    /// The forge refused, saying why.
    Refused(String),
}

/// An account on the forge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    pub id: u64,
    pub username: String,
}

/// A merge request as a merge queue sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Queued {
    pub number: u64,
    pub url: String,
    pub state: State,
    /// Its assignees' ids.
    pub assignees: Vec<u64>,
    /// Whether its head contains the tip of its target branch.
    pub current: bool,
    /// The pipeline of its head.
    pub pipeline: Option<PipelineState>,
}

/// The requests the tool makes of a forge.
pub trait Forge {
    fn kind(&self) -> Kind;

    fn location(&self) -> &Location;

    /// Reads the project, which tells whether the token is accepted.
    fn check_access(&self) -> Result<()>;

    /// The merge requests from `source`, into `target` when given, newest
    /// first: only the open ones with `open_only`.
    fn merge_requests(
        &self,
        source: &str,
        target: Option<&str>,
        open_only: bool,
    ) -> Result<Vec<MergeRequest>>;

    fn merge_request(&self, number: u64) -> Result<MergeRequest>;

    /// Opens a merge request from `source` into `target`, or returns `None`
    /// when the forge refuses it because one is open already. On GitLab it
    /// asks for squashing off and the source branch removed on merge.
    fn open(
        &self,
        source: &str,
        target: &str,
        title: &str,
        description: &str,
    ) -> Result<Option<MergeRequest>>;

    /// Rewrites a merge request's title and description; on GitLab it asks
    /// again for squashing off.
    fn update(&self, number: u64, title: &str, description: &str) -> Result<MergeRequest>;

    /// Closes a merge request without merging it, replacing its
    /// description in the same request when `description` is given.
    fn close(&self, number: u64, description: Option<&str>) -> Result<MergeRequest>;

    /// Whether anyone approved the merge request, on GitLab, or reviewed
    /// it in any state, on GitHub.
    fn reviewed(&self, number: u64) -> Result<Signal>;

    /// Whether the merge request has a comment other than the forge's own
    /// system notes.
    fn commented(&self, number: u64) -> Result<Signal>;

    /// Whether anyone reacted to the merge request itself with an emoji.
    fn reacted(&self, number: u64) -> Result<Signal>;

    /// Replaces the description of a merge request, in any state.
    fn set_description(&self, number: u64, description: &str) -> Result<MergeRequest>;

    /// Reads the settings that decide how a merge request into `target` is
    /// merged.
    fn review(&self, target: &str) -> Result<Review>;

    /// The longest description the forge takes, in characters.
    fn description_limit(&self) -> usize;

    /// Whether the project has a release for `tag`. The project's list of
    /// releases is read first, so that a project the forge hides is never
    /// taken for one without the release.
    fn release(&self, tag: &str) -> Result<ReleaseLookup>;

    /// Creates an annotated tag, only where the forge has no tag of its
    /// name.
    fn create_tag(&self, tag: &NewTag) -> Result<TagCreation>;

    /// Whether the token may create a tag named `name`, as far as the
    /// forge's answers tell, without creating anything; an error when they
    /// do not tell.
    fn tag_access(&self, name: &str) -> Result<TagAccess>;

    /// Reads a merge request with the pipeline of its head, the one the
    /// forge's rule that pipelines must succeed reads; `None` before the
    /// forge has one.
    fn with_pipeline(&self, number: u64) -> Result<(MergeRequest, Option<Pipeline>)>;

    /// Merges a merge request at `head`, never squashed and never rebased,
    /// as the project's merge method merges a branch that contains its
    /// target. On GitLab it asks the forge to remove the source branch, which
    /// GitHub's merge has no way to ask.
    fn merge(&self, number: u64, head: &str) -> Result<Merge>;

    /// What keeps a merge request from merging that no catch-up or wait
    /// changes, as the forge names it; `None` when nothing does.
    fn merge_blocker(&self, number: u64) -> Result<Option<String>>;

    /// The account named `username`, or `None` when the forge knows none.
    fn account(&self, username: &str) -> Result<Option<Account>>;

    /// The numbers of the open merge requests into `target` assigned to
    /// `account`.
    fn assigned(&self, target: &str, account: &Account) -> Result<Vec<u64>>;

    /// A merge request as a merge queue sees it.
    fn queued(&self, number: u64) -> Result<Queued>;

    /// When a merge request was last assigned to `account`, as merge queues
    /// read the order they take merge requests in.
    fn assigned_at(&self, number: u64, account: &Account) -> Result<Option<jiff::Timestamp>>;

    /// Sets a merge request's assignees.
    fn assign(&self, number: u64, assignees: &[u64]) -> Result<()>;

    /// Writes a note on a merge request.
    fn comment(&self, number: u64, body: &str) -> Result<()>;
}

/// A request that failed: the forge's answer, or none at all. A `git fetch`,
/// `git ls-remote` or `git push` of the publish remote that fails counts as no
/// answer, whatever git says: a busy SSH endpoint can answer `Permission
/// denied (publickey)` to a credential that worked seconds before.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    /// The forge's status; `None` when it gave no answer.
    pub status: Option<u16>,
    message: String,
}

impl Failure {
    pub fn answered(status: u16, message: String) -> Self {
        Self {
            status: Some(status),
            message,
        }
    }

    pub fn unanswered(message: String) -> Self {
        Self {
            status: None,
            message,
        }
    }
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Failure {}

/// The failed request behind an error, if one is.
fn failure(err: &anyhow::Error) -> Option<&Failure> {
    err.chain().find_map(|e| e.downcast_ref::<Failure>())
}

/// Runs `step`, up to `attempts` times with `gap` between them, as long as it
/// fails on what may pass.
pub fn patiently<T>(
    attempts: u32,
    gap: Duration,
    mut step: impl FnMut() -> Result<T>,
) -> Result<T> {
    let mut tried = 1;
    loop {
        match step() {
            Err(err) if may_pass(&err) && tried < attempts => {
                tried += 1;
                say!(
                    "{}; trying again",
                    crate::redact::redacted(&format!("{err:#}"))
                );
                std::thread::sleep(gap);
            }
            done => return done,
        }
    }
}

/// Whether an error may pass when the request is made again: the forge had
/// no answer, or answered that it cannot serve it now.
pub fn may_pass(err: &anyhow::Error) -> bool {
    failure(err).is_some_and(|f| matches!(f.status, None | Some(429 | 500..=599)))
}

/// Reads a signal of a merge request with `read`: what the forge refused, or
/// answered with something unreadable, is a reason the signal could not be
/// read, and only what may pass when asked again stays an error.
fn signal(read: impl FnOnce() -> Result<bool>) -> Result<Signal> {
    match read() {
        Ok(found) => Ok(Ok(found)),
        Err(err) if may_pass(&err) => Err(err),
        Err(err) => Ok(Err(format!("{err:#}"))),
    }
}

/// Whether an error is the forge answering that it has no such thing.
pub fn not_found(err: &anyhow::Error) -> bool {
    failure(err).is_some_and(|f| f.status == Some(404))
}

/// Reads a pipeline state as GitLab and GitHub's commit statuses write it.
fn pipeline_state(status: &str) -> PipelineState {
    match status {
        "success" => PipelineState::Passed,
        "failed" | "failure" | "error" | "skipped" => PipelineState::Failed,
        "canceled" | "cancelled" => PipelineState::Canceled,
        _ => PipelineState::Running,
    }
}

/// Whether `body`, a note the forge wrote, records an assignment to
/// `username`, such as `assigned to @merge-bot` or `assigned to @a and
/// @merge-bot`, as merge queues read the order they take merge requests in.
pub fn assigns(body: &str, username: &str) -> bool {
    let Some(rest) = body.split("unassigned").next() else {
        return false;
    };
    let Some((_, mentioned)) = rest.split_once("assigned to ") else {
        return false;
    };
    let name = format!("@{username}");
    mentioned.match_indices(&name).any(|(at, _)| {
        mentioned[at + name.len()..]
            .chars()
            .next()
            .is_none_or(|c| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')))
    })
}

/// Reaches the forge of the publish remote that `publish` names, refusing
/// before anything is pushed when it cannot be located, or, for merge
/// requests, no token is set.
pub fn connect(git: &Git, publish: &Publish, access: Access) -> Result<Box<dyn Forge>> {
    let url = fetch_url(git, &publish.remote)?;
    let env = |name: &str| std::env::var(name).ok();
    let around = locate::Surroundings {
        env: &env,
        ssh_host: &ssh_host,
    };
    let location = match locate::locate(publish, url.as_deref(), &around, access) {
        Ok(location) => location,
        Err(why) => bail!("{why}"),
    };
    if location.kind == Kind::GitHub
        && let Some(queue) = &publish.queue
    {
        bail!(
            "publish queue: '{queue}' names a merge queue that takes the pull requests assigned to it, which \
             GitHub has no setting for; a repository's merge queue on GitHub is refused as a merge setting. \
             Remove `queue` from [publish]"
        );
    }
    Ok(match location.kind {
        Kind::GitLab => Box::new(gitlab::GitLab::new(location)),
        Kind::GitHub => Box::new(github::GitHub::new(location)),
    })
}

/// The kind of forge of the publish remote, when the tool can tell it without
/// the forge's API; `None` otherwise.
pub fn kind_of_remote(git: &Git, publish: &Publish) -> Result<Option<Kind>> {
    let url = fetch_url(git, &publish.remote)?;
    let env = |name: &str| std::env::var(name).ok();
    let around = locate::Surroundings {
        env: &env,
        ssh_host: &ssh_host,
    };
    Ok(locate::kind(publish, url.as_deref(), &around))
}

/// Refuses, in a GitLab CI job, a clone that would push to `remote` with the
/// job's own token. The runner configures git to, one way or another, and such
/// a push is refused or starts no pipeline, so this is said at the start of a
/// run, not at its first real push, which may be months of green scheduled jobs
/// away. Outside a job it checks nothing.
pub fn refuse_job_token_push(git: &Git, remote: &str) -> Result<()> {
    if let Some(how) = job_token_push(git, remote)? {
        bail!(
            "this GitLab CI job would push to '{remote}' with its job token: {how}. The job token may not push, and \
             where a project lets it, its pushes start no pipeline, so the first real push would fail or go untested \
             however green the runs before it. Nothing was pushed. Drop the runner's git configuration and push \
             with an access token, as `Running in CI` in the README shows"
        );
    }
    Ok(())
}

/// Warns, in a GitLab CI job, of a clone that would push to `remote` with the
/// job's own token, where a project may have chosen that: publication by push
/// needs no pipeline to land.
pub fn warn_of_job_token_push(git: &Git, remote: &str) -> Result<()> {
    if let Some(how) = job_token_push(git, remote)? {
        esay!(
            "warning: this GitLab CI job would push to '{remote}' with its job token: {how}. That fails unless the \
             project lets the job token push, and such a push starts no pipeline; `Running in CI` in the README \
             shows how to push with an access token"
        );
    }
    Ok(())
}

/// How a push to `remote` would use the job's own token, when it would.
fn job_token_push(git: &Git, remote: &str) -> Result<Option<&'static str>> {
    let Some(token) = std::env::var(locate::JOB_TOKEN)
        .ok()
        .filter(|t| !t.trim().is_empty())
    else {
        return Ok(None);
    };
    let urls = git.raw(&["remote", "get-url", "--push", "--all", remote])?;
    if !urls.success() {
        return Ok(None);
    }
    for url in crate::git::lines(&urls.stdout) {
        if let Some(how) = pushes_with(git, &url, token.trim())? {
            return Ok(Some(how));
        }
    }
    Ok(None)
}

/// How a push to `url` would use `token`, when it would.
fn pushes_with(git: &Git, url: &str, token: &str) -> Result<Option<&'static str>> {
    let Ok(parsed) = Url::parse(url) else {
        return Ok(None);
    };
    if !matches!(parsed.scheme(), "http" | "https") {
        return Ok(None);
    }
    if let Some(password) = parsed.password() {
        let password = percent_encoding::percent_decode_str(password).decode_utf8_lossy();
        return Ok((password == token).then_some("the push URL carries it"));
    }
    Ok((git.credential_password(url)?.as_deref() == Some(token))
        .then_some("git's credential helpers answer with it"))
}

/// The paths of the forge's CI definition that an update may change, as the
/// tool can tell them without the forge's API, for publication by push:
/// `.github/workflows/` on GitHub, and on GitLab the default CI
/// configuration file `.gitlab-ci.yml`, since a project's own setting is
/// readable only through the API. Empty when the forge is not recognised.
pub fn ci_paths_without_api(git: &Git, publish: &Publish) -> Result<Vec<String>> {
    Ok(match kind_of_remote(git, publish)? {
        Some(Kind::GitHub) => vec![github::WORKFLOWS.to_string()],
        Some(Kind::GitLab) => vec![gitlab::DEFAULT_CI_FILE.to_string()],
        None => Vec::new(),
    })
}

/// The URL git fetches a remote from: its configured URL after git's
/// `url.<base>.insteadOf` rewriting, as `git remote get-url` reports it, so
/// that the forge is the one the branch actually lands on. `None` when the
/// remote does not exist.
pub fn fetch_url(git: &Git, remote: &str) -> Result<Option<String>> {
    if git.remote_url(remote)?.is_none() {
        return Ok(None);
    }
    Ok(Some(git.line(&["remote", "get-url", remote])?))
}

/// The host an SSH host alias stands for, as `ssh -G` resolves the client's
/// configuration; `None` without an `ssh` that answers.
fn ssh_host(alias: &str) -> Option<String> {
    crate::identity::ssh_destination(alias).map(|(host, _)| host)
}

/// `base` with `segments` appended, each encoded as one path segment, and
/// the query `query`.
pub fn endpoint(base: &Url, segments: &[&str], query: &[(&str, &str)]) -> Url {
    let mut url = base.clone();
    if let Ok(mut path) = url.path_segments_mut() {
        path.pop_if_empty().extend(segments);
    }
    if !query.is_empty() {
        url.query_pairs_mut().extend_pairs(query);
    }
    url
}

/// A JSON field as text, empty when absent.
fn text(value: &serde_json::Value, key: &str) -> String {
    value
        .get(key)
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// A JSON field as text, `None` when absent or null.
fn optional_text(value: &serde_json::Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoints_encode_each_segment() {
        let base = Url::parse("https://gitlab.example.com/gitlab/api/v4").unwrap();
        let url = endpoint(
            &base,
            &["projects", "group/sub app", "merge_requests"],
            &[("source_branch", "git-intake/update"), ("state", "opened")],
        );
        assert_eq!(
            url.as_str(),
            "https://gitlab.example.com/gitlab/api/v4/projects/group%2Fsub%20app/merge_requests?\
             source_branch=git-intake%2Fupdate&state=opened"
        );
        let base = Url::parse("https://api.github.com").unwrap();
        assert_eq!(
            endpoint(&base, &["repos", "team", "app"], &[]).as_str(),
            "https://api.github.com/repos/team/app"
        );
    }

    #[test]
    fn merge_requests_are_ordered_by_instant_across_offsets() {
        let mr = |number: u64, ended: Option<&str>| MergeRequest {
            number,
            url: String::new(),
            state: State::Closed,
            head: String::new(),
            source: String::new(),
            target: String::new(),
            description: String::new(),
            created_at: None,
            ended_at: ended.map(|t| t.parse().unwrap()),
            draft: None,
            assigned: None,
            auto_merge: None,
            review_requested: None,
            locked: false,
            merge_scheduled: false,
        };
        // Across the change to summer time: 03:15+02:00 is 01:15Z, before
        // 01:30Z, though it sorts after it as text; and back to winter
        // time, 02:10+01:00 is 01:10Z, after 02:30+02:00, which is 00:30Z.
        let mut list = vec![
            mr(1, Some("2026-03-29T03:15:00+02:00")),
            mr(2, Some("2026-03-29T01:30:00Z")),
            mr(3, None),
            mr(4, Some("2026-10-25T02:30:00+02:00")),
            mr(5, Some("2026-10-25T02:10:00+01:00")),
        ];
        latest_first(&mut list);
        let order: Vec<u64> = list.iter().map(|m| m.number).collect();
        assert_eq!(order, [5, 4, 2, 1, 3]);
    }

    #[test]
    fn assignment_notes_name_the_account() {
        assert!(assigns("assigned to @merge-bot", "merge-bot"));
        assert!(assigns("assigned to @a and @merge-bot", "merge-bot"));
        assert!(assigns(
            "assigned to @merge-bot and unassigned @a",
            "merge-bot"
        ));
        assert!(!assigns(
            "assigned to @a and unassigned @merge-bot",
            "merge-bot"
        ));
        assert!(!assigns("unassigned @merge-bot", "merge-bot"));
        assert!(!assigns("assigned to @merge-bot-2", "merge-bot"));
        assert!(!assigns("requested review from @merge-bot", "merge-bot"));
        assert_eq!(pipeline_state("skipped"), PipelineState::Failed);
        assert_eq!(pipeline_state("manual"), PipelineState::Running);
    }

    #[test]
    fn failures_that_may_pass_are_told_from_refusals() {
        let may = |f: Failure| may_pass(&anyhow::Error::from(f).context("reading the forge"));
        assert!(may(Failure::unanswered("no route".into())));
        assert!(may(Failure::answered(502, "502".into())));
        assert!(may(Failure::answered(429, "429".into())));
        assert!(!may(Failure::answered(403, "403".into())));
        assert!(!may(Failure::answered(404, "404".into())));
        assert!(!may_pass(&anyhow::anyhow!("something else")));
        assert!(not_found(&Failure::answered(404, "404".into()).into()));
        assert!(!not_found(&Failure::answered(403, "403".into()).into()));
    }

    #[test]
    fn each_forge_has_its_term() {
        assert_eq!(Kind::GitLab.term(), "merge request");
        assert_eq!(Kind::GitHub.term(), "pull request");
        assert_eq!(Kind::GitLab.reference(7), "!7");
        assert_eq!(Kind::GitHub.reference(7), "#7");
    }
}
