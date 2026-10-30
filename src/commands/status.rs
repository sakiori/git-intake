//! `status`: per-upstream drift, the update in progress, and the cutover gate.

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::Serialize;

use crate::Exit;
use crate::commands::update::{
    blocking_prefix, described, next_step, plural, step_name, unrelated_content,
};
use crate::config::{Config, Mode, Upstream};
use crate::drift::{self, Drift, DriftState, SyncSource};
use crate::git::{Git, abbrev, branch_ref};
use crate::output::{Render, emit, table};
use crate::port::{self, Standing, Verb};
use crate::repo::Repo;
use crate::state::{ConflictFile, CopyStep, MergeKind, UpdateState, UpstreamStep, Work};
use crate::sync_record::{self, Record};
use crate::upstreams::Fetches;

#[derive(Serialize)]
pub struct StatusDoc {
    pub integration: String,
    pub integration_tip: String,
    /// The publish remote.
    pub publish: Option<String>,
    /// `push` or `merge_request`; `null` without a publish remote.
    pub publish_via: Option<String>,
    pub install: Option<String>,
    pub checks: Vec<String>,
    pub config_working_copy_differs: bool,
    /// Whether `--check` predicted conflicts and gated on drift.
    pub checked: bool,
    pub all_current: bool,
    pub upstreams: Vec<UpstreamStatus>,
    pub update: Option<UpdateView>,
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
    /// Follow mode: commits of the tip the integration branch lacks.
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
    /// Files a merge of the tip would conflict in; only with `--check`, for
    /// a follow upstream.
    pub conflicts: Option<Vec<String>>,
}

impl UpstreamStatus {
    fn new(up: &Upstream, tip: Option<String>, state: DriftState) -> Self {
        Self {
            name: up.name.clone(),
            path: up.path.clone(),
            url: up.url.clone(),
            branch: up.branch.clone(),
            mode: up.mode,
            tip,
            state,
            detail: None,
            last_synced: None,
            last_synced_source: None,
            commits_behind: None,
            base: None,
            journal: None,
            taken: None,
            decided: None,
            pending: None,
            conflicts: None,
        }
    }
}

#[derive(Serialize)]
pub struct UpdateView {
    pub step: String,
    pub base: String,
    pub branch: String,
    pub worktree: PathBuf,
    pub upstreams: Vec<QueuedView>,
    pub verified_commit: Option<String>,
    /// The error of the last rejected push while finishing, if any.
    pub publish_error: Option<String>,
    /// The merge request the update waits for, in merge-request mode.
    pub merge_request: Option<MergeRequestView>,
    pub next: String,
}

#[derive(Serialize)]
pub struct MergeRequestView {
    /// The merge request's number, when the remote's reply named it.
    pub number: Option<u64>,
    pub remote: String,
    /// The branch on the publish remote that holds the verified commit.
    pub source_branch: String,
    pub target_branch: String,
    pub commit: String,
    /// Whether the push that carried the options asking the remote to open
    /// the merge request transferred the branch; when not, it is opened by hand.
    pub requested: bool,
}

#[derive(Serialize)]
pub struct QueuedView {
    pub name: String,
    pub mode: Mode,
    /// The upstream tip `start` fetched, which the update's reference keeps.
    pub fetched: String,
    /// Follow mode: the fixed target.
    pub target: Option<String>,
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
    /// `none`, `conflict`, `copied` or `empty`.
    pub copy: CopyStep,
    pub copied: Option<String>,
}

pub fn run(repo: &Repo, check: bool, json: bool) -> Result<Exit> {
    let loaded = repo.load_config()?;
    let config = &loaded.config;
    let git = &repo.git;
    let tip = git
        .commit(&branch_ref(&config.integration))?
        .with_context(|| {
            format!(
                "the integration branch '{}' does not exist",
                config.integration
            )
        })?;

    let mut fetches = Fetches::new(repo);
    let mut fetched = Vec::new();
    for up in &config.upstreams {
        fetched.push(fetches.upstream(up)?);
    }
    let fetch_failed = fetched.iter().any(|f| f.is_err());
    let latest = sync_record::latest(&sync_record::records(git, &tip, config)?);
    let mut upstreams = Vec::new();
    for (up, fetched) in config.upstreams.iter().zip(&fetched) {
        let fetched = fetched.as_deref().map_err(String::as_str);
        let last = latest.get(&up.name);
        upstreams.push(match up.mode {
            Mode::Follow => follow_status(git, &tip, up, last, fetched, check)?,
            Mode::Port => port_status(git, config, &tip, up, last, fetched),
        });
    }
    drop(fetches);

    let all_current = upstreams.iter().all(|u| u.state == DriftState::Current);
    let update = match repo.state()? {
        Some(state) => Some(update_view(&state)?),
        None => None,
    };
    let doc = StatusDoc {
        integration: config.integration.clone(),
        integration_tip: tip,
        publish: config.publish.as_ref().map(|p| p.remote.clone()),
        publish_via: config
            .publish
            .as_ref()
            .map(|p| p.via.as_str().replace('-', "_")),
        install: config.install.clone(),
        checks: config.checks.clone(),
        config_working_copy_differs: loaded.working_copy_differs,
        checked: check,
        all_current,
        upstreams,
        update,
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
    check: bool,
) -> Result<UpstreamStatus> {
    let mut d: Drift = drift::evaluate(git, tip, up, last, fetched)?;
    if let Err(why) = fetched {
        d.detail = Some(format!("fetch failed: {why}"));
    }
    let conflicts = if check {
        Some(predicted(git, tip, up, &mut d)?)
    } else {
        None
    };
    let mut status = UpstreamStatus::new(up, d.tip, d.state);
    status.detail = d.detail;
    status.last_synced = d.last_synced;
    status.last_synced_source = d.last_synced_source;
    status.commits_behind = d.commits_behind;
    status.conflicts = conflicts;
    Ok(status)
}

fn port_status(
    git: &Git,
    config: &Config,
    tip: &str,
    up: &Upstream,
    last: Option<&Record>,
    fetched: std::result::Result<&str, &str>,
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
    match port::standing(git, config, tip, up, last, upstream_tip) {
        Err(err) => {
            s.state = DriftState::Unavailable;
            s.detail = Some(format!("{err:#}"));
        }
        Ok(Standing::NoBase) => {
            s.journal = Some(false);
            s.detail = Some(format!(
                "no journal, and no sync record of the upstream or history shared with it to start one from; \
                 write the journal {} by hand",
                port::journal_path(&up.name)
            ));
        }
        Ok(Standing::Rewritten(why)) => {
            s.state = DriftState::Rewritten;
            s.detail = Some(why);
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

/// Predicted conflicts for one follow upstream; may add a detail when a
/// merge could not run at all.
fn predicted(git: &Git, tip: &str, up: &Upstream, d: &mut Drift) -> Result<Vec<String>> {
    let Some(upstream_tip) = d.tip.clone() else {
        return Ok(Vec::new());
    };
    match d.state {
        DriftState::Behind => drift::predict_conflicts(git, tip, &upstream_tip, up.path.as_deref()),
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
    Ok(UpdateView {
        step: step_name(state, &wt)?.to_string(),
        base: state.base.clone(),
        branch: state.branch.clone(),
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
                                })
                                .collect(),
                        );
                    }
                }
                view
            })
            .collect(),
        verified_commit: state
            .verification
            .as_ref()
            .filter(|v| v.passed)
            .map(|v| v.commit.clone()),
        publish_error: state.finishing.as_ref().and_then(|f| f.push_error.clone()),
        merge_request: state.merge_request.as_ref().map(|mr| MergeRequestView {
            number: mr.number,
            remote: mr.remote.clone(),
            source_branch: mr.branch.clone(),
            target_branch: state.config.integration.clone(),
            commit: mr.commit.clone(),
            requested: mr.requested,
        }),
        next: next_step(state, &wt)?,
    })
}

fn opt_abbrev(oid: &Option<String>) -> String {
    oid.as_deref().map(abbrev).unwrap_or("-").to_string()
}

fn config_lines(doc: &StatusDoc, out: &mut String) {
    out.push_str(&format!(
        "integration: {} at {}\n",
        doc.integration,
        abbrev(&doc.integration_tip)
    ));
    match (&doc.publish, &doc.publish_via) {
        (Some(remote), Some(via)) => out.push_str(&format!(
            "publish: {remote} (via {})\n",
            via.replace('_', " ")
        )),
        _ => out.push_str("publish: none\n"),
    }
    out.push_str(&format!(
        "install: {}\n",
        doc.install.as_deref().unwrap_or("none")
    ));
    if doc.checks.is_empty() {
        out.push_str("checks: none\n");
    } else {
        out.push_str("checks:\n");
        for c in &doc.checks {
            out.push_str(&format!("  {c}\n"));
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
                    opt_abbrev(&u.last_synced),
                    opt_abbrev(&u.tip),
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
                if conflicts.is_empty() {
                    out.push_str(&format!(
                        "{}: {behind} commit{} behind, would merge without conflicts\n",
                        u.name,
                        plural(behind)
                    ));
                } else {
                    out.push_str(&format!(
                        "{}: {behind} commit{} behind, would conflict in:\n",
                        u.name,
                        plural(behind)
                    ));
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
    out.push_str(&format!("  base: {}\n", abbrev(&u.base)));
    out.push_str(&format!("  branch: {}\n", u.branch));
    match &u.merge_request {
        // The worktree is removed once the merge request branch is pushed.
        Some(mr) => out.push_str(&format!(
            "  merge request{}: {} into {} on {}{}\n",
            mr.number.map(|n| format!(" !{n}")).unwrap_or_default(),
            mr.source_branch,
            mr.target_branch,
            mr.remote,
            if mr.requested {
                ""
            } else {
                " (the remote was not asked to open it; open it unless it is open)"
            }
        )),
        None => out.push_str(&format!("  worktree: {}\n", u.worktree.display())),
    }
    for q in &u.upstreams {
        let step = q.step.describe(q.mode == Mode::Port);
        match (&q.target, &q.plan, &q.commits) {
            (Some(target), _, _) => out.push_str(&format!(
                "  {}: target {}, {step}\n",
                q.name,
                described(target, q.rev.as_deref())
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
                            _ => String::new(),
                        };
                        out.push_str(&format!(
                            "    {verb} {} {}{outcome}\n",
                            abbrev(&c.commit),
                            c.subject
                        ));
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
    if let Some(v) = &u.verified_commit {
        out.push_str(&format!("  verified: {}\n", abbrev(v)));
    }
    if let Some(e) = &u.publish_error {
        out.push_str(&format!("  publish rejected: {e}\n"));
    }
    out.push_str(&format!("  next: {}\n", u.next));
    out
}
