//! `status`: per-upstream drift, the update in progress, and the cutover gate.

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::Serialize;

use crate::Exit;
use crate::commands::update::{blocking_prefix, next_step, step_name, unrelated_content};
use crate::drift::{self, Drift, DriftState, SyncSource};
use crate::git::{Git, abbrev, branch_ref};
use crate::output::{Render, emit, table};
use crate::remotes;
use crate::repo::Repo;
use crate::state::{ConflictFile, MergeKind, UpdateState, UpstreamStep};
use crate::sync_record;

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
    /// Whether `--check` fetched the upstreams and predicted conflicts.
    pub checked: bool,
    pub all_current: bool,
    pub upstreams: Vec<UpstreamStatus>,
    pub update: Option<UpdateView>,
}

#[derive(Serialize)]
pub struct UpstreamStatus {
    pub name: String,
    pub path: Option<String>,
    pub remote: String,
    pub branch: String,
    pub url: Option<String>,
    pub mirror: Option<String>,
    pub last_synced: Option<String>,
    /// `record` or `history`; `null` when nothing is synced.
    pub last_synced_source: Option<SyncSource>,
    pub tip: Option<String>,
    pub commits_behind: Option<u64>,
    pub state: DriftState,
    pub detail: Option<String>,
    /// Files a merge of the tip would conflict in; only with `--check`.
    pub conflicts: Option<Vec<String>>,
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
    pub target: String,
    pub kind: MergeKind,
    pub step: UpstreamStep,
    pub merge_commit: Option<String>,
    pub conflicts: Vec<ConflictFile>,
}

pub fn run(repo: &Repo, check: bool, json: bool) -> Result<Exit> {
    let loaded = repo.load_config()?;
    let config = &loaded.config;
    let git = &repo.git;

    let mut fetch_errors = vec![None; config.upstreams.len()];
    if check {
        for (i, up) in config.upstreams.iter().enumerate() {
            fetch_errors[i] = match remotes::ensure_one(git, up)? {
                Err(why) => Some(why),
                Ok(_) => remotes::fetch(git, up)?.err(),
            };
        }
    }

    let tip = git
        .commit(&branch_ref(&config.integration))?
        .with_context(|| {
            format!(
                "the integration branch '{}' does not exist",
                config.integration
            )
        })?;
    let latest = sync_record::latest(&sync_record::records(git, &tip, config)?);
    let mut upstreams = Vec::new();
    for (i, up) in config.upstreams.iter().enumerate() {
        let mut d: Drift = drift::evaluate(git, &tip, up, latest.get(&up.name))?;
        if let Some(why) = fetch_errors[i].take() {
            d.state = DriftState::Unavailable;
            d.detail = Some(format!("fetch failed: {why}"));
        }
        let conflicts = if check {
            Some(predicted(git, &tip, up, &mut d)?)
        } else {
            None
        };
        upstreams.push(UpstreamStatus {
            name: up.name.clone(),
            path: up.path.clone(),
            remote: up.remote.clone(),
            branch: up.branch.clone(),
            url: up.url.clone(),
            mirror: up.mirror.clone(),
            last_synced: d.last_synced,
            last_synced_source: d.last_synced_source,
            tip: d.tip,
            commits_behind: d.commits_behind,
            state: d.state,
            detail: d.detail,
            conflicts,
        });
    }
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
    Ok(if check && !all_current {
        Exit::Negative
    } else {
        Exit::Success
    })
}

/// Predicted conflicts for one upstream; may add a detail when a merge could
/// not run at all.
fn predicted(
    git: &Git,
    tip: &str,
    up: &crate::config::Upstream,
    d: &mut Drift,
) -> Result<Vec<String>> {
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
            .map(|q| QueuedView {
                name: q.name.clone(),
                target: q.target.clone(),
                kind: q.kind,
                step: q.step,
                merge_commit: q.merge_commit.clone(),
                conflicts: q.conflicts.clone(),
            })
            .collect(),
        verified_commit: state
            .verification
            .as_ref()
            .filter(|v| v.passed)
            .map(|v| v.commit.clone()),
        publish_error: state.finishing.as_ref().and_then(|f| f.push_error.clone()),
        merge_request: state.merge_request.as_ref().map(|mr| MergeRequestView {
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
                    format!("{}/{}", u.remote, u.branch),
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
                "BRANCH",
                "LAST SYNCED",
                "TIP",
                "BEHIND",
                "STATE",
            ],
            &rows,
        ));
        for u in &self.upstreams {
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
                if conflicts.is_empty() {
                    out.push_str(&format!(
                        "{}: {} commit{} behind, would merge without conflicts\n",
                        u.name,
                        u.commits_behind.unwrap_or_default(),
                        crate::commands::update::plural(u.commits_behind.unwrap_or_default())
                    ));
                } else {
                    out.push_str(&format!(
                        "{}: {} commit{} behind, would conflict in:\n",
                        u.name,
                        u.commits_behind.unwrap_or_default(),
                        crate::commands::update::plural(u.commits_behind.unwrap_or_default())
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
            "  merge request: {} into {} on {}{}\n",
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
        let step = match q.step {
            UpstreamStep::Pending => "pending",
            UpstreamStep::Conflict => "in conflict",
            UpstreamStep::Merged => "merged",
        };
        out.push_str(&format!(
            "  {}: target {}, {step}\n",
            q.name,
            abbrev(&q.target)
        ));
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
