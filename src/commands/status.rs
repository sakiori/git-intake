//! `status`: per-upstream drift, the update in progress, and the cutover gate.

use std::path::PathBuf;

use anyhow::Result;
use serde::Serialize;

use crate::Exit;
use crate::commands::merge_request;
use crate::commands::release::{self, ReleaseState};
use crate::commands::update::{
    blocking_prefix, described, limited, next_step, no_release, plural, step_name,
    unrelated_content,
};
use crate::config::{Check, Mode, PublishVia, Upstream};
use crate::drift::{self, Drift, DriftState, SyncSource};
use crate::git::{Git, abbrev};
use crate::output::{Render, emit, table};
use crate::port::{self, OffLine, Standing, Verb};
use crate::repo::Repo;
use crate::state::{
    CheckOutcome, ConflictFile, CopyStep, MergeKind, Stored, UPDATE_BRANCH, UpdateState,
    UpstreamStep, Waiting, Work,
};
use crate::sync_record::{self, Record};
use crate::tags::{TagPattern, UpstreamTags};
use crate::upstreams::Fetches;

#[derive(Serialize)]
pub struct StatusDoc {
    pub integration: String,
    /// The integration tip: the publish remote's integration branch, as
    /// `status` fetched it.
    pub integration_tip: String,
    /// The publish remote.
    pub publish: String,
    /// `push` or `merge_request`.
    pub publish_via: String,
    /// Merge-request mode: `replace_after` as the configuration writes it,
    /// or `null`.
    pub replace_after: Option<String>,
    pub install: Option<String>,
    /// The checks of the configuration at the integration tip, each with its
    /// `name` and the command it runs, `run`.
    pub checks: Vec<Check>,
    pub config_working_copy_differs: bool,
    /// Whether `--check` predicted conflicts and gated on drift.
    pub checked: bool,
    pub all_current: bool,
    pub upstreams: Vec<UpstreamStatus>,
    /// Merge-request mode: the update's open merge request, or `null`.
    pub merge_request: Option<OpenRequestView>,
    /// Merge-request mode: the newest merge request of the update merged
    /// without its history, or `null`.
    pub merged_without_history: Option<RequestView>,
    /// Merge-request mode: a pause of the merge queue the publish remote
    /// records, or `null`.
    pub pause: Option<PauseView>,
    /// Merge-request mode: why the record of a pause could not be read, or
    /// `null`.
    pub pause_error: Option<String>,
    /// Merge-request mode: why the forge could not be read, or `null`.
    pub forge_error: Option<String>,
    /// The forge's term for a merge request, for the text form.
    #[serde(skip)]
    pub term: &'static str,
    pub update: Option<UpdateView>,
    /// With `[release]`: the release state of the publish remote's
    /// integration tip, as `release next --json` writes it, or `null`.
    pub release: Option<ReleaseState>,
    /// With `[release]`: why the release state could not be read, or
    /// `null`.
    pub release_error: Option<String>,
}

#[derive(Serialize)]
pub struct OpenRequestView {
    pub number: u64,
    pub url: String,
    /// Its head commit.
    pub head: String,
    /// Whether its head contains the integration branch's tip.
    pub current: bool,
    /// How messages write its number, such as `!7` or `#7`.
    #[serde(skip)]
    pub reference: String,
}

#[derive(Serialize)]
pub struct PauseView {
    /// The merge request it was started for.
    pub number: u64,
    pub url: String,
    /// The merge queue's account.
    pub account: String,
    /// The paused merge requests, in the order they return in.
    pub merge_requests: Vec<u64>,
    /// The merge requests the queue was landing when it began.
    pub in_flight: Vec<u64>,
    /// What the text form says of it.
    #[serde(skip)]
    pub described: String,
}

#[derive(Serialize)]
pub struct RequestView {
    pub number: u64,
    pub url: String,
    #[serde(skip)]
    pub reference: String,
}

#[derive(Serialize)]
pub struct UpstreamStatus {
    pub name: String,
    pub path: Option<String>,
    pub url: String,
    pub branch: String,
    pub mode: Mode,
    /// The fetched tip; `null` when the upstream could not be fetched.
    pub tip: Option<String>,
    pub state: DriftState,
    pub detail: Option<String>,
    /// Follow mode: the last synced commit.
    pub last_synced: Option<String>,
    /// Follow mode: `record` or `history`; `null` when nothing is synced.
    pub last_synced_source: Option<SyncSource>,
    /// Follow mode: the upstream tag the newest sync record names.
    pub last_synced_tag: Option<String>,
    /// Follow mode: the configured pattern of release tags.
    pub tags: Option<String>,
    /// Follow mode with `tags`: the commit of the newest release tag on the
    /// fetched branch; `null` when there is none.
    pub release: Option<String>,
    /// Follow mode with `tags`: the name of that tag.
    pub release_tag: Option<String>,
    /// Follow mode with `tags`: commits of the fetched tip after the
    /// newest release.
    pub commits_after_release: Option<u64>,
    /// Follow mode: commits of the tip, or with `tags` of the newest
    /// release, that the integration branch lacks.
    pub commits_behind: Option<u64>,
    /// Port mode: the journal's base, or the last synced commit a new
    /// journal would start from.
    pub base: Option<String>,
    /// Port mode: whether the journal exists on the integration branch.
    pub journal: Option<bool>,
    /// Port mode: commits after the base that copies took.
    pub taken: Option<u64>,
    /// Port mode: commits after the base the journal decides.
    pub decided: Option<u64>,
    /// Port mode: commits after the base nobody has decided on yet.
    pub pending: Option<u64>,
    /// Port mode, when rewritten: what is no longer on the upstream's
    /// first-parent line.
    pub off_line: Option<OffLineView>,
    /// Files a merge of the tip would conflict in; only with `--check`, for
    /// a follow upstream. With `max_conflicts`, those of `next_tag`.
    pub conflicts: Option<Vec<String>>,
    /// With `--check` and `max_conflicts`, for an upstream behind: the tag
    /// an update would take, or `null`.
    pub next_tag: Option<String>,
    /// Its commit.
    pub next_target: Option<String>,
    /// Whether its predicted conflicts are within `max_conflicts`.
    pub next_within_limit: Option<bool>,
    /// The configured `max_conflicts`.
    #[serde(skip)]
    pub max_conflicts: Option<u32>,
}

impl UpstreamStatus {
    fn new(up: &Upstream, tip: Option<String>, state: DriftState) -> Self {
        Self {
            name: up.name.clone(),
            path: up.path.clone(),
            url: crate::redact::redacted(&up.url),
            branch: up.branch.clone(),
            mode: up.mode,
            tip,
            state,
            detail: None,
            last_synced: None,
            last_synced_source: None,
            last_synced_tag: None,
            tags: None,
            release: None,
            release_tag: None,
            commits_after_release: None,
            commits_behind: None,
            base: None,
            journal: None,
            taken: None,
            decided: None,
            pending: None,
            off_line: None,
            conflicts: None,
            next_tag: None,
            next_target: None,
            next_within_limit: None,
            max_conflicts: up.max_conflicts,
        }
    }
}

#[derive(Serialize)]
pub struct OffLineView {
    /// The base, when it is no longer on the line.
    pub base: Option<String>,
    /// Taken or decided commits no longer on the line and not retired.
    pub commits: Vec<OffLine>,
}

#[derive(Serialize)]
pub struct UpdateView {
    pub step: String,
    /// The integration tip the update lands on: the tip at `start`, or the
    /// one it last caught up with.
    pub base: String,
    /// The integration tip at `start`.
    pub start_base: String,
    pub branch: String,
    pub worktree: PathBuf,
    pub upstreams: Vec<QueuedView>,
    /// The names of the update's checks, in their order: those of the
    /// configuration at `start`, or at the last catch-up merge.
    pub checks: Vec<String>,
    /// The commit a full run of the checks verified.
    pub verified_commit: Option<String>,
    /// The last run that left out some of the checks, which never verifies
    /// the update, or `null`.
    pub partial_run: Option<PartialRunView>,
    /// The merge request an earlier release left the update waiting for.
    pub merge_request: Option<MergeRequestView>,
    /// The merge request whose update `sync` carried on, for an update it
    /// carried on.
    pub carried_from: Option<CarriedView>,
    /// The catch-up merge of the steps `catch_up_conflict` and `catch_up`.
    pub catch_up: Option<CatchUpView>,
    pub next: String,
}

#[derive(Serialize)]
pub struct PartialRunView {
    /// The commit it ran on.
    pub commit: String,
    /// The install command's outcome, or `null` when none is configured.
    pub install: Option<CheckOutcome>,
    /// The checks it ran, in their order.
    pub checks: Vec<CheckOutcomeView>,
    /// The names of the checks it left out.
    pub left_out: Vec<String>,
    pub passed: bool,
}

#[derive(Serialize)]
pub struct CheckOutcomeView {
    pub name: String,
    pub outcome: CheckOutcome,
}

#[derive(Serialize)]
pub struct CatchUpView {
    /// The integration tip caught up with.
    pub tip: String,
    /// The update branch head the catch-up merges into it.
    pub head_before: String,
    /// The catch-up merge, once committed.
    pub merge_commit: Option<String>,
    pub conflicts: Vec<ConflictFile>,
}

#[derive(Serialize)]
pub struct MergeRequestView {
    /// The merge request's number, when the earlier release recorded it.
    pub number: Option<u64>,
    pub remote: String,
    /// The branch on the publish remote the earlier release pushed to.
    pub source_branch: String,
    pub target_branch: String,
    /// The commit the earlier release last pushed to it.
    pub commit: String,
}

#[derive(Serialize)]
pub struct CarriedView {
    pub number: u64,
    pub url: String,
    /// The branch the merge request comes from.
    pub source_branch: String,
}

#[derive(Serialize)]
pub struct QueuedView {
    pub name: String,
    pub mode: Mode,
    /// The upstream tip `start` fetched, which the update's reference keeps.
    pub fetched: String,
    /// Follow mode: the fixed target.
    pub target: Option<String>,
    /// Follow mode: the upstream tag that names the target.
    pub tag: Option<String>,
    /// The revision `start` was given: the target of a follow upstream, the
    /// limit of a port plan; `null` for the fetched tip.
    pub rev: Option<String>,
    /// Follow mode: how the upstream is merged.
    pub kind: Option<MergeKind>,
    pub step: UpstreamStep,
    /// Follow mode: the merge commit.
    pub merge_commit: Option<String>,
    /// The conflicted files of the merge or copy that stopped.
    pub conflicts: Vec<ConflictFile>,
    /// Port mode: the plan file.
    pub plan: Option<PathBuf>,
    /// Port mode: the planned commits, oldest first.
    pub commits: Option<Vec<PlannedView>>,
    /// Port mode: the commit that recorded the decisions in the journal.
    pub journal_commit: Option<String>,
}

#[derive(Serialize)]
pub struct PlannedView {
    pub commit: String,
    pub subject: String,
    /// `take`, `skip`, `have` or `later`; `null` until the plan is read.
    pub verb: Option<Verb>,
    pub note: Option<String>,
    /// `none`, `conflict`, `commit_failed`, `copied` or `empty`.
    pub copy: CopyStep,
    pub copied: Option<String>,
    /// What git said when the commit of the copy failed.
    pub failure: Option<String>,
    /// Paths the copy left out: `.git-intake.toml` and `.git-intake/`.
    pub left_out: Vec<String>,
}

pub fn run(repo: &Repo, check: bool, json: bool) -> Result<Exit> {
    let loaded = repo.load_config()?;
    let config = &loaded.config;
    let git = &repo.git;
    let tip = loaded.tip.clone();

    let mut fetches = Fetches::new(repo);
    let mut fetched = Vec::new();
    let mut tag_lists = Vec::new();
    for up in &config.upstreams {
        let mut result = fetches.upstream(up)?;
        // An upstream taken at its release tags needs them as much as its
        // branch.
        let mut tags = None;
        if result.is_ok() && up.tags.is_some() {
            match UpstreamTags::list(git, up)? {
                Ok(list) => tags = Some(list),
                Err(why) => result = Err(why),
            }
        }
        fetched.push(result);
        tag_lists.push(tags);
    }
    let mut known = crate::release::Known::default();
    for ((up, f), tags) in config.upstreams.iter().zip(&fetched).zip(&tag_lists) {
        if let Ok(tip) = f {
            known
                .tips
                .insert((up.url.clone(), up.branch.clone()), tip.clone());
        }
        if let Some(tags) = tags {
            known.tags.insert(up.url.clone(), tags.clone());
        }
    }
    let fetch_failed = fetched.iter().any(|f| f.is_err());
    let tips: Vec<String> = fetched.iter().filter_map(|f| f.clone().ok()).collect();
    let latest = sync_record::latest(git, &sync_record::records(git, &tip, &tips)?)?;
    let mut upstreams = Vec::new();
    for ((up, fetched), tags) in config.upstreams.iter().zip(&fetched).zip(&tag_lists) {
        let fetched = fetched.as_deref().map_err(String::as_str);
        let last = latest.get(&up.name);
        upstreams.push(match up.mode {
            Mode::Follow => follow_status(git, &tip, up, last, fetched, tags.as_ref(), check)?,
            Mode::Port => port_status(git, &tip, up, last, fetched, &tips),
        });
    }
    drop(fetches);

    let all_current = upstreams.iter().all(|u| u.state == DriftState::Current);
    let mut merge_request = None;
    let mut merged_without_history = None;
    let mut pause = None;
    let mut pause_error = None;
    let mut forge_error = None;
    let mut term = "merge request";
    let publish = &config.publish;
    if publish.via == PublishVia::MergeRequest {
        match crate::pause::look(repo, publish, &config.upstreams) {
            Ok(recorded) => {
                pause = recorded.map(|r| PauseView {
                    described: crate::pause::described(&publish.remote, &r.pause),
                    number: r.pause.merge_request.number,
                    url: r.pause.merge_request.url.clone(),
                    account: r.pause.account.username.clone(),
                    merge_requests: r.pause.numbers(),
                    in_flight: r.pause.in_flight.clone(),
                });
            }
            Err(err) => pause_error = Some(format!("{err:#}")),
        }
        match merge_request::overview(repo, config, &tip) {
            Ok(o) => {
                term = o.kind.term();
                if let Some(open) = o.open {
                    merge_request = Some(OpenRequestView {
                        current: git.object_exists(&open.head)?
                            && git.is_ancestor(&tip, &open.head)?,
                        reference: o.kind.reference(open.number),
                        number: open.number,
                        url: open.url,
                        head: open.head,
                    });
                }
                merged_without_history = o.merged_without_history.map(|m| RequestView {
                    reference: o.kind.reference(m.number),
                    number: m.number,
                    url: m.url,
                });
            }
            Err(err) => forge_error = Some(format!("{err:#}")),
        }
    }
    let (release, release_error) = if config.release.is_some() {
        let from = crate::repo::Bootstrap::of(config, crate::repo::NamedBy::Tip);
        match release::state(repo, &from, &known, false) {
            Ok(state) => (Some(state), None),
            Err(err) => (None, Some(format!("{err:#}"))),
        }
    } else {
        (None, None)
    };
    let update = match repo.stored()? {
        Some(Stored::Update(state)) => {
            repo.restore_held_references(&state)?;
            Some(update_view(&state)?)
        }
        Some(Stored::Waiting(w)) => Some(waiting_view(&w)),
        None => None,
    };
    let doc = StatusDoc {
        integration: config.integration.clone(),
        integration_tip: tip,
        publish: config.publish.remote.clone(),
        publish_via: config.publish.via.as_str().replace('-', "_"),
        replace_after: config
            .publish
            .replace_after
            .as_ref()
            .map(|a| a.text().to_string()),
        install: config.install.clone(),
        checks: config.checks.clone(),
        config_working_copy_differs: loaded.working_copy_differs,
        checked: check,
        all_current,
        upstreams,
        merge_request,
        merged_without_history,
        pause,
        pause_error,
        forge_error,
        term,
        update,
        release,
        release_error,
    };
    emit(&doc, json)?;
    Ok(if fetch_failed || (check && !all_current) {
        Exit::Negative
    } else {
        Exit::Success
    })
}

fn follow_status(
    git: &Git,
    tip: &str,
    up: &Upstream,
    last: Option<&Record>,
    fetched: std::result::Result<&str, &str>,
    tags: Option<&UpstreamTags>,
    check: bool,
) -> Result<UpstreamStatus> {
    let mut d: Drift = drift::evaluate(git, tip, up, last, fetched)?;
    if let Err(why) = fetched {
        d.detail = Some(format!("fetch failed: {why}"));
    }
    // With release tags, the newest release stands where the tip stands
    // otherwise: what the integration branch lacks is counted up to it.
    let mut release = None;
    let mut next = None;
    if let (Some(text), Some(tags), Some(upstream_tip)) = (&up.tags, tags, &d.tip) {
        let pattern = TagPattern::parse(text)
            .map_err(|why| anyhow::anyhow!("upstream '{}' tags: {why}", up.name))?;
        release = tags.release(git, &pattern, up.prereleases, upstream_tip)?;
        if check {
            next = limited(git, tip, up, &d, tags, &pattern, upstream_tip)?;
        }
        if matches!(d.state, DriftState::Current | DriftState::Behind) {
            match &release {
                Some(r) => {
                    let behind = git.count(tip, &r.commit)?;
                    d.commits_behind = Some(behind);
                    d.state = if behind == 0 {
                        DriftState::Current
                    } else {
                        DriftState::Behind
                    };
                }
                None => {
                    d.commits_behind = None;
                    d.state = DriftState::NoRelease;
                    d.detail = Some(no_release(up));
                }
            }
        }
        // `start` goes by the tag the limit picks, even one of a lower version
        // than a release already taken in.
        if let Some(next) = next.as_ref().filter(|_| d.state == DriftState::Current) {
            d.commits_behind = Some(git.count(tip, &next.release.commit)?);
            d.state = DriftState::Behind;
        }
    }
    let conflicts = match &next {
        Some(next) if d.state == DriftState::Behind => Some(next.conflicts.clone()),
        _ if check => {
            let target = release.as_ref().map(|r| r.commit.clone()).or(d.tip.clone());
            Some(predicted(git, tip, up, target.as_deref(), &mut d)?)
        }
        _ => None,
    };
    let mut status = UpstreamStatus::new(up, d.tip, d.state);
    status.detail = d.detail;
    status.last_synced = d.last_synced;
    status.last_synced_source = d.last_synced_source;
    status.last_synced_tag = last.and_then(|r| r.tag.clone());
    status.tags = up.tags.clone();
    if let Some(r) = release {
        if let Some(upstream_tip) = &status.tip {
            status.commits_after_release = Some(git.count(&r.commit, upstream_tip)?);
        }
        status.release = Some(r.commit);
        status.release_tag = Some(r.tag);
    }
    status.commits_behind = d.commits_behind;
    status.conflicts = conflicts;
    if let Some(next) = next.filter(|_| status.state == DriftState::Behind) {
        status.next_tag = Some(next.release.tag);
        status.next_target = Some(next.release.commit);
        status.next_within_limit = Some(next.within);
    }
    Ok(status)
}

fn port_status(
    git: &Git,
    tip: &str,
    up: &Upstream,
    last: Option<&Record>,
    fetched: std::result::Result<&str, &str>,
    tips: &[String],
) -> UpstreamStatus {
    let upstream_tip = match fetched {
        Ok(t) => t,
        Err(why) => {
            let mut s = UpstreamStatus::new(up, None, DriftState::Unavailable);
            s.detail = Some(format!("fetch failed: {why}"));
            return s;
        }
    };
    let mut s = UpstreamStatus::new(up, Some(upstream_tip.to_string()), DriftState::Pending);
    match port::standing(git, tip, up, last, upstream_tip, tips) {
        Err(err) => {
            s.state = DriftState::Unavailable;
            s.detail = Some(format!("{err:#}"));
        }
        Ok(Standing::NoBase) => {
            s.state = DriftState::NoBase;
            s.journal = Some(false);
            s.detail = Some(port::no_base_advice(&up.name));
        }
        Ok(Standing::Rewritten(rewrite)) => {
            s.state = DriftState::Rewritten;
            s.detail = Some(rewrite.describe(&up.name));
            s.off_line = Some(OffLineView {
                base: rewrite.base.clone(),
                commits: rewrite.commits,
            });
        }
        Ok(Standing::Known(a)) => {
            let pending = a.pending().len() as u64;
            s.base = Some(a.base.clone());
            s.journal = Some(a.journal);
            s.taken = Some(a.taken.len() as u64);
            s.decided = Some(a.decided.len() as u64);
            s.pending = Some(pending);
            if pending == 0 {
                s.state = DriftState::Current;
            }
            if !a.journal {
                s.detail = Some(format!(
                    "no journal yet; the first update that carries out a plan starts it at the last synced commit {}",
                    abbrev(&a.base)
                ));
            }
        }
    }
    s
}

/// Predicted conflicts of a merge of `target` for one follow upstream; may
/// add a detail when a merge could not run at all.
fn predicted(
    git: &Git,
    tip: &str,
    up: &Upstream,
    target: Option<&str>,
    d: &mut Drift,
) -> Result<Vec<String>> {
    let Some(target) = target else {
        return Ok(Vec::new());
    };
    match d.state {
        DriftState::Behind => drift::predict_conflicts(git, tip, target, up.path.as_deref()),
        DriftState::NotImported => {
            if let Some(path) = &up.path {
                if git.path_exists(tip, path)? {
                    d.detail = Some(unrelated_content(up, path));
                } else if let Some(prefix) = blocking_prefix(git, tip, path)? {
                    d.detail = Some(format!(
                        "'{prefix}' is a file on the integration branch; importing under '{path}' would replace it"
                    ));
                }
            } else {
                d.detail = Some("shares no history with the integration branch".to_string());
            }
            Ok(Vec::new())
        }
        _ => Ok(Vec::new()),
    }
}

pub fn update_view(state: &UpdateState) -> Result<UpdateView> {
    let wt = Git::worktree(&state.worktree);
    let step = step_name(state, &wt)?;
    Ok(UpdateView {
        step: step.to_string(),
        base: state.base.clone(),
        start_base: state.start_base().to_string(),
        branch: UPDATE_BRANCH.to_string(),
        worktree: state.worktree.clone(),
        upstreams: state
            .upstreams
            .iter()
            .map(|q| {
                let mut view = QueuedView {
                    name: q.name.clone(),
                    mode: Mode::Follow,
                    fetched: q.fetched.clone(),
                    target: None,
                    tag: None,
                    rev: q.rev.clone(),
                    kind: None,
                    step: q.step,
                    merge_commit: None,
                    conflicts: q.conflicts().to_vec(),
                    plan: None,
                    commits: None,
                    journal_commit: None,
                };
                match &q.work {
                    Work::Follow(f) => {
                        view.target = Some(f.target.clone());
                        view.tag = f.tag.clone();
                        view.kind = Some(f.kind);
                        view.merge_commit = f.merge_commit.clone();
                    }
                    Work::Port(p) => {
                        view.mode = Mode::Port;
                        view.plan = Some(p.plan.clone());
                        view.journal_commit = p.journal_commit.clone();
                        view.commits = Some(
                            p.commits
                                .iter()
                                .map(|c| PlannedView {
                                    commit: c.commit.clone(),
                                    subject: c.subject.clone(),
                                    verb: c.verb,
                                    note: c.note.clone(),
                                    copy: c.copy,
                                    copied: c.copied.clone(),
                                    failure: c.failure.clone(),
                                    left_out: c.left_out.clone(),
                                })
                                .collect(),
                        );
                    }
                }
                view
            })
            .collect(),
        checks: state.config.checks.iter().map(|c| c.name.clone()).collect(),
        verified_commit: state
            .verification
            .as_ref()
            .filter(|v| v.passed)
            .map(|v| v.commit.clone()),
        partial_run: state.partial_run.as_ref().map(|p| PartialRunView {
            commit: p.run.commit.clone(),
            install: p.run.install.as_ref().map(|i| i.outcome),
            checks: p
                .run
                .checks
                .iter()
                .map(|c| CheckOutcomeView {
                    name: c.name.clone(),
                    outcome: c.result.outcome,
                })
                .collect(),
            left_out: p.left_out.clone(),
            passed: p.run.passed,
        }),
        merge_request: None,
        carried_from: state.carried_from.as_ref().map(|c| CarriedView {
            number: c.number,
            url: c.url.clone(),
            source_branch: c.branch.clone(),
        }),
        catch_up: state
            .catch_ups
            .last()
            .filter(|_| step.starts_with("catch_up"))
            .map(|c| CatchUpView {
                tip: c.tip.clone(),
                head_before: c.head_before.clone(),
                merge_commit: c.merge_commit.clone(),
                conflicts: c.conflicts.clone(),
            }),
        next: next_step(state, &wt)?,
    })
}

/// An update of an earlier release that waits for its merge request, as far
/// as this release reads it. Its queued upstreams are not shown: their
/// targets and steps are the earlier release's, which this release does not
/// read.
fn waiting_view(w: &Waiting) -> UpdateView {
    UpdateView {
        step: "waiting_for_merge_request".to_string(),
        base: w.base.clone(),
        start_base: w.start_base.clone(),
        branch: w.branch.clone(),
        worktree: w.worktree.clone(),
        upstreams: Vec::new(),
        checks: Vec::new(),
        verified_commit: None,
        partial_run: None,
        merge_request: Some(MergeRequestView {
            number: w.merge_request.number,
            remote: w.merge_request.remote.clone(),
            source_branch: w.merge_request.branch.clone(),
            target_branch: w.integration.clone(),
            commit: w.merge_request.commit.clone(),
        }),
        carried_from: None,
        catch_up: None,
        next: w.next(),
    }
}

fn opt_abbrev(oid: &Option<String>) -> String {
    oid.as_deref().map(abbrev).unwrap_or("-").to_string()
}

/// A commit with the tag that names it, or `-`.
fn shown(oid: &Option<String>, tag: &Option<String>) -> String {
    match oid {
        Some(oid) => described(oid, tag.as_deref()),
        None => "-".to_string(),
    }
}

fn config_lines(doc: &StatusDoc, out: &mut String) {
    out.push_str(&format!(
        "integration: {} on '{}' at {}\n",
        doc.integration,
        doc.publish,
        abbrev(&doc.integration_tip)
    ));
    out.push_str(&format!(
        "publish: {} (via {}{})\n",
        doc.publish,
        doc.publish_via.replace('_', " "),
        doc.replace_after
            .as_deref()
            .map(|a| format!(", replacing an untouched {} after {a}", doc.term))
            .unwrap_or_default()
    ));
    out.push_str(&format!(
        "install: {}\n",
        doc.install.as_deref().unwrap_or("none")
    ));
    if doc.checks.is_empty() {
        out.push_str("checks: none\n");
    } else {
        out.push_str("checks:\n");
        for c in &doc.checks {
            out.push_str(&format!("  {}: {}\n", c.name, c.run));
        }
    }
}

impl Render for StatusDoc {
    fn render(&self) -> String {
        let mut out = String::new();
        config_lines(self, &mut out);
        out.push('\n');
        let rows: Vec<Vec<String>> = self
            .upstreams
            .iter()
            .map(|u| {
                vec![
                    u.name.clone(),
                    u.path.clone().unwrap_or_else(|| "/".into()),
                    u.mode.as_str().to_string(),
                    u.branch.clone(),
                    shown(&u.last_synced, &u.last_synced_tag),
                    match &u.tags {
                        Some(_) => shown(&u.release, &u.release_tag),
                        None => opt_abbrev(&u.tip),
                    },
                    u.commits_behind
                        .map(|n| n.to_string())
                        .unwrap_or("-".into()),
                    u.state.label().to_string(),
                ]
            })
            .collect();
        out.push_str(&table(
            &[
                "UPSTREAM",
                "PATH",
                "MODE",
                "BRANCH",
                "LAST SYNCED",
                "TIP",
                "BEHIND",
                "STATE",
            ],
            &rows,
        ));
        for u in &self.upstreams {
            if let (Some(base), Some(taken), Some(decided), Some(pending)) =
                (&u.base, u.taken, u.decided, u.pending)
            {
                out.push_str(&format!(
                    "{}: base {}, {taken} taken, {decided} decided, {pending} pending\n",
                    u.name,
                    abbrev(base)
                ));
            }
            if let (Some(pattern), Some(tip)) = (&u.tags, &u.tip) {
                match (&u.release, &u.release_tag) {
                    (Some(release), Some(tag)) if release != tip => {
                        let past = u.commits_after_release.unwrap_or_default();
                        out.push_str(&format!(
                            "{}: takes release tags '{pattern}'; the branch tip {} is {past} commit{} past {tag}\n",
                            u.name,
                            abbrev(tip),
                            plural(past)
                        ));
                    }
                    (Some(_), Some(tag)) => out.push_str(&format!(
                        "{}: takes release tags '{pattern}'; the branch tip is {tag}\n",
                        u.name
                    )),
                    _ => {}
                }
            }
            if u.last_synced_source == Some(SyncSource::History) {
                out.push_str(&format!(
                    "{}: no sync record yet; last synced is the newest upstream commit the integration branch contains\n",
                    u.name
                ));
            }
            if let Some(detail) = &u.detail {
                out.push_str(&format!("{}: {detail}\n", u.name));
            }
            if let Some(conflicts) = &u.conflicts
                && u.state == DriftState::Behind
            {
                let behind = u.commits_behind.unwrap_or_default();
                let lead = match (&u.next_tag, u.next_within_limit, u.max_conflicts) {
                    (Some(tag), Some(within), Some(limit)) => format!(
                        "{}: {behind} commit{} behind; an update would take {tag}, {} max_conflicts = {limit}, and",
                        u.name,
                        plural(behind),
                        if within {
                            "within"
                        } else {
                            "the oldest newer tag, as no newer tag is within"
                        }
                    ),
                    _ => format!("{}: {behind} commit{} behind,", u.name, plural(behind)),
                };
                if conflicts.is_empty() {
                    out.push_str(&format!("{lead} would merge without conflicts\n"));
                } else {
                    out.push_str(&format!("{lead} would conflict in:\n"));
                    for f in conflicts {
                        out.push_str(&format!("  {f}\n"));
                    }
                }
            }
        }
        if self.checked {
            out.push_str(if self.all_current {
                "every upstream is current\n"
            } else {
                "drift: not every upstream is current\n"
            });
        }
        let term = self.term;
        if let Some(mr) = &self.merge_request {
            out.push_str(&format!(
                "\n{term} {} ({}) at {}: ",
                mr.reference,
                mr.url,
                abbrev(&mr.head)
            ));
            if mr.current {
                out.push_str(&format!(
                    "it contains '{}' and waits to be merged\n",
                    self.integration
                ));
            } else {
                out.push_str(&format!(
                    "behind '{}'; run `git intake sync` to catch it up\n",
                    self.integration
                ));
            }
        }
        if let Some(m) = &self.merged_without_history {
            out.push_str(&format!(
                "{term} {} ({}) was merged without the update's history, which a squash or rebase drops; its sync \
                 records never landed, and `git intake sync` proposes its targets again\n",
                m.reference, m.url
            ));
        }
        if let Some(p) = &self.pause {
            out.push_str(&format!("{}\n", p.described));
        }
        if let Some(why) = &self.pause_error {
            out.push_str(&format!(
                "\ncannot tell whether the merge queue is paused: {why}\n"
            ));
        }
        if let Some(why) = &self.forge_error {
            out.push_str(&format!("\ncannot read the forge: {why}\n"));
        }
        if let Some(release) = &self.release {
            out.push('\n');
            out.push_str(&release.render());
        }
        if let Some(why) = &self.release_error {
            out.push_str(&format!("\ncannot read the release state: {why}\n"));
        }
        if let Some(update) = &self.update {
            out.push('\n');
            out.push_str(&render_update(update));
        }
        out
    }
}

pub fn render_update(u: &UpdateView) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "update in progress: {}\n",
        u.step.replace('_', " ")
    ));
    if u.start_base == u.base {
        out.push_str(&format!("  base: {}\n", abbrev(&u.base)));
    } else {
        out.push_str(&format!(
            "  base: {}, caught up from {}\n",
            abbrev(&u.base),
            abbrev(&u.start_base)
        ));
    }
    out.push_str(&format!("  branch: {}\n", u.branch));
    match &u.merge_request {
        Some(mr) => out.push_str(&format!(
            "  waiting, as an earlier release left it, for the merge request{} from {} into {} on {}\n",
            mr.number.map(|n| format!(" {n}")).unwrap_or_default(),
            mr.source_branch,
            mr.target_branch,
            mr.remote
        )),
        None => out.push_str(&format!("  worktree: {}\n", u.worktree.display())),
    }
    if let Some(c) = &u.carried_from {
        out.push_str(&format!(
            "  carried on from {} on {}\n",
            c.url, c.source_branch
        ));
    }
    for q in &u.upstreams {
        let step = q.step.describe(q.mode == Mode::Port);
        match (&q.target, &q.plan, &q.commits) {
            (Some(target), _, _) => out.push_str(&format!(
                "  {}: target {}, {step}\n",
                q.name,
                described(target, q.tag.as_deref().or(q.rev.as_deref()))
            )),
            (None, Some(plan), Some(commits)) => {
                let n = commits.len() as u64;
                out.push_str(&format!(
                    "  {}: plan {} ({n} commit{}{}), {step}\n",
                    q.name,
                    plan.display(),
                    plural(n),
                    q.rev
                        .as_deref()
                        .map(|r| format!(" up to {r}"))
                        .unwrap_or_default()
                ));
                if q.step != UpstreamStep::Plan {
                    for c in commits {
                        let verb = c.verb.map(Verb::as_str).unwrap_or("later");
                        let outcome = match (c.copy, &c.copied) {
                            (CopyStep::Copied, Some(copy)) => {
                                format!(", copied as {}", abbrev(copy))
                            }
                            (CopyStep::Empty, _) => {
                                ", changed nothing: recorded as present".to_string()
                            }
                            (CopyStep::Conflict, _) => ", in conflict".to_string(),
                            (CopyStep::CommitFailed, _) => ", its commit failed".to_string(),
                            _ => String::new(),
                        };
                        out.push_str(&format!(
                            "    {verb} {} {}{outcome}\n",
                            abbrev(&c.commit),
                            c.subject
                        ));
                        if !c.left_out.is_empty() {
                            out.push_str(&format!("      left out: {}\n", c.left_out.join(", ")));
                        }
                        if let Some(failure) = &c.failure {
                            for line in failure.lines() {
                                out.push_str(&format!("      {line}\n"));
                            }
                        }
                    }
                }
            }
            _ => out.push_str(&format!("  {}: {step}\n", q.name)),
        }
        if q.step == UpstreamStep::Conflict {
            for c in &q.conflicts {
                out.push_str(&format!("    {}  ({})\n", c.path, c.resolution.describe()));
            }
        }
    }
    if let Some(c) = &u.catch_up {
        let merge = c
            .merge_commit
            .as_deref()
            .map(|m| format!(" as {}", abbrev(m)))
            .unwrap_or_default();
        out.push_str(&format!(
            "  catching up with the integration tip {}{merge}\n",
            abbrev(&c.tip)
        ));
        for f in &c.conflicts {
            out.push_str(&format!("    {}  ({})\n", f.path, f.resolution.describe()));
        }
    }
    if !u.checks.is_empty() {
        out.push_str(&format!("  checks: {}\n", u.checks.join(", ")));
    }
    if let Some(v) = &u.verified_commit {
        out.push_str(&format!("  verified: {}\n", abbrev(v)));
    }
    if let Some(p) = &u.partial_run {
        let mut outcomes: Vec<String> = Vec::new();
        if let Some(install) = p.install {
            outcomes.push(format!("install command {}", install.as_str()));
        }
        outcomes.extend(
            p.checks
                .iter()
                .map(|c| format!("{} {}", c.name, c.outcome.as_str())),
        );
        if outcomes.is_empty() {
            outcomes.push("nothing run".to_string());
        }
        out.push_str(&format!(
            "  partial run at {}, which does not verify the update: {}; {}; left out: {}\n",
            abbrev(&p.commit),
            if p.passed { "passed" } else { "failed" },
            outcomes.join(", "),
            p.left_out.join(", ")
        ));
    }
    out.push_str(&format!("  next: {}\n", u.next));
    out
}
