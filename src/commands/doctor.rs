//! `doctor`: read-only checks of configuration, remotes, history and state.

use anyhow::Result;
use serde::Serialize;

use crate::Exit;
use crate::commands::update::unrelated_content;
use crate::config::Config;
use crate::drift::adopted_base;
use crate::git::{Git, abbrev, branch_ref, tracking_ref};
use crate::output::{Render, emit};
use crate::remotes::{self, RemoteCheck};
use crate::repo::{MainWorktree, Repo};
use crate::state::{UPDATE_BRANCH, UpstreamStep};
use crate::sync_record;

#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Pass,
    Fail,
    Warn,
}

#[derive(Serialize)]
pub struct DoctorCheck {
    /// Stable identifier of the kind of check.
    pub check: String,
    /// The upstream the check is about, if any.
    pub upstream: Option<String>,
    pub status: CheckStatus,
    pub detail: String,
}

#[derive(Serialize)]
pub struct DoctorDoc {
    pub healthy: bool,
    pub checks: Vec<DoctorCheck>,
}

struct Checks(Vec<DoctorCheck>);

impl Checks {
    fn add(&mut self, check: &str, upstream: Option<&str>, status: CheckStatus, detail: String) {
        self.0.push(DoctorCheck {
            check: check.to_string(),
            upstream: upstream.map(str::to_string),
            status,
            detail,
        });
    }

    fn pass_or_fail(&mut self, check: &str, upstream: Option<&str>, ok: bool, detail: String) {
        let status = if ok {
            CheckStatus::Pass
        } else {
            CheckStatus::Fail
        };
        self.add(check, upstream, status, detail);
    }
}

pub fn run(repo: &Repo, json: bool) -> Result<Exit> {
    let mut checks = Checks(Vec::new());
    match repo.load_config() {
        Ok(loaded) => {
            checks.add(
                "configuration",
                None,
                CheckStatus::Pass,
                format!(
                    "the configuration on '{}' is valid",
                    loaded.config.integration
                ),
            );
            upstream_checks(repo, &loaded.config, &mut checks)?;
        }
        Err(err) => checks.add("configuration", None, CheckStatus::Fail, format!("{err:#}")),
    }
    checkout_checks(repo, &mut checks)?;
    state_checks(repo, &mut checks)?;

    let healthy = checks.0.iter().all(|c| c.status != CheckStatus::Fail);
    emit(
        &DoctorDoc {
            healthy,
            checks: checks.0,
        },
        json,
    )?;
    Ok(if healthy {
        Exit::Success
    } else {
        Exit::Negative
    })
}

fn upstream_checks(repo: &Repo, config: &Config, checks: &mut Checks) -> Result<()> {
    let git = &repo.git;
    let Some(integration) = git.commit(&branch_ref(&config.integration))? else {
        checks.add(
            "integration_branch",
            None,
            CheckStatus::Fail,
            format!(
                "the integration branch '{}' does not exist",
                config.integration
            ),
        );
        return Ok(());
    };
    let records = sync_record::records(git, &integration, config)?;
    let latest = sync_record::latest(&records);
    for up in &config.upstreams {
        let name = Some(up.name.as_str());
        match remotes::check(git, up)? {
            RemoteCheck::Ready => checks.add(
                "remote",
                name,
                CheckStatus::Pass,
                format!(
                    "remote '{}' exists{}",
                    up.remote,
                    if up.url.is_some() {
                        " with the configured url"
                    } else {
                        ""
                    }
                ),
            ),
            RemoteCheck::Add(url) => checks.add(
                "remote",
                name,
                CheckStatus::Fail,
                format!(
                    "remote '{}' does not exist; `git intake fetch` adds it from {url}",
                    up.remote
                ),
            ),
            RemoteCheck::Refused(why) => checks.add("remote", name, CheckStatus::Fail, why),
        }

        let tip = git.commit(&tracking_ref(&up.remote, &up.branch))?;
        checks.pass_or_fail(
            "branch",
            name,
            tip.is_some(),
            match &tip {
                Some(t) => format!("{} exists at {}", up.source(), abbrev(t)),
                None => format!("{} has not been fetched or does not exist", up.source()),
            },
        );

        let record = latest.get(&up.name);
        if let Some(path) = &up.path {
            let exists = git.object_type(&integration, path)?.is_some();
            // Synced: a record, or a merge in the history that brought the
            // upstream into its path; unknown when the branch is not fetched.
            let synced = match (record.is_some(), &tip) {
                (true, _) => Some(true),
                (false, Some(t)) => Some(adopted_base(git, &integration, up, t)?.is_some()),
                (false, None) => None,
            };
            let (status, detail) = match (record.is_some(), synced, exists) {
                (_, Some(true), true) => (
                    CheckStatus::Pass,
                    if record.is_some() {
                        format!("path '{path}' exists on '{}'", config.integration)
                    } else {
                        format!(
                            "path '{path}' was imported without a sync record; the next update records it"
                        )
                    },
                ),
                (_, Some(true), false) => (
                    CheckStatus::Fail,
                    format!(
                        "upstream is imported but path '{path}' does not exist on '{}'",
                        config.integration
                    ),
                ),
                (_, _, false) => (
                    CheckStatus::Pass,
                    format!("not imported yet; path '{path}' will be created by the first update"),
                ),
                (_, Some(false), true) => (CheckStatus::Fail, unrelated_content(up, path)),
                (_, None, true) => (
                    CheckStatus::Fail,
                    format!(
                        "path '{path}' exists without a sync record and {} is not available to check it",
                        up.source()
                    ),
                ),
            };
            checks.add("path", name, status, detail);
        }

        match (record, &tip) {
            (None, Some(t)) => {
                let detail = match adopted_base(git, &integration, up, t)? {
                    Some(base) => format!(
                        "no sync record yet; last synced from history is {}, contained in {}",
                        abbrev(&base),
                        up.source()
                    ),
                    None => "no sync record yet".to_string(),
                };
                checks.add("last_synced", name, CheckStatus::Pass, detail);
            }
            (None, None) => checks.add(
                "last_synced",
                name,
                CheckStatus::Pass,
                "no sync record yet".to_string(),
            ),
            (Some(r), Some(t)) => {
                let contained = git.object_exists(&r.target)? && git.is_ancestor(&r.target, t)?;
                checks.pass_or_fail(
                    "last_synced",
                    name,
                    contained,
                    if contained {
                        format!("last synced commit {} is contained in {}", abbrev(&r.target), up.source())
                    } else {
                        format!(
                            "{} no longer contains the last synced commit {}; the upstream was rewritten",
                            up.source(),
                            abbrev(&r.target)
                        )
                    },
                );
            }
            (Some(r), None) => checks.add(
                "last_synced",
                name,
                CheckStatus::Fail,
                format!(
                    "cannot check last synced commit {}: {} is not available",
                    abbrev(&r.target),
                    up.source()
                ),
            ),
        }
    }

    let mut dropped: Vec<&str> = Vec::new();
    for r in &records {
        if config.upstream(&r.upstream).is_none() && !dropped.contains(&r.upstream.as_str()) {
            dropped.push(&r.upstream);
        }
    }
    for name in dropped {
        checks.add(
            "dropped_upstream",
            Some(name),
            CheckStatus::Warn,
            format!(
                "upstream '{name}' has sync records on '{}' but was dropped from the configuration",
                config.integration
            ),
        );
    }
    Ok(())
}

fn checkout_checks(repo: &Repo, checks: &mut Checks) -> Result<()> {
    let main = match repo.main_worktree() {
        MainWorktree::Bare => return Ok(()),
        MainWorktree::At(main) => main.clone(),
        // Nothing is wrong with the repository; the checks need to run in
        // the main worktree.
        MainWorktree::Unknown => {
            checks.add(
                "main_checkout_clean",
                None,
                CheckStatus::Warn,
                format!("not checked: {}", repo.unknown_main_worktree()),
            );
            return Ok(());
        }
    };
    let git = Git::worktree(&main);
    let changes = git.tracked_changes()?;
    checks.pass_or_fail(
        "main_checkout_clean",
        None,
        changes.is_empty(),
        if changes.is_empty() {
            "the main checkout has no uncommitted changes".to_string()
        } else {
            format!(
                "the main checkout has uncommitted changes: {}",
                changes.join(", ")
            )
        },
    );
    let merging = git.merge_in_progress()?;
    checks.pass_or_fail(
        "no_merge_in_progress",
        None,
        !merging,
        if merging {
            "a merge is in progress in the main checkout".to_string()
        } else {
            "no merge is in progress in the main checkout".to_string()
        },
    );
    Ok(())
}

fn state_checks(repo: &Repo, checks: &mut Checks) -> Result<()> {
    let registered = repo.update_worktree_registered()?;
    let branch = repo.update_branch_exists()?;
    let state = match repo.state() {
        Ok(s) => s,
        Err(err) => {
            checks.add("update_state", None, CheckStatus::Fail, format!("{err:#}"));
            return Ok(());
        }
    };
    let Some(state) = state else {
        let leftover = registered || branch || repo.paths.worktree.exists();
        checks.pass_or_fail(
            "update_state",
            None,
            !leftover,
            if leftover {
                format!(
                    "no update is in progress, but its worktree or the branch '{UPDATE_BRANCH}' remains; run `git intake abort`"
                )
            } else {
                "no update is in progress".to_string()
            },
        );
        return Ok(());
    };
    if let Some(mr) = &state.merge_request {
        // The worktree was removed when the merge request branch was pushed.
        let detail = if state.finishing.is_some() {
            format!(
                "`finish` is completing the update merged through '{}'; run `git intake finish`",
                mr.branch
            )
        } else {
            format!(
                "the update waits for its merge request from '{}' into '{}' on '{}'",
                mr.branch, state.config.integration, mr.remote
            )
        };
        checks.add("update_state", None, CheckStatus::Pass, detail);
        return Ok(());
    }
    let mut problems = Vec::new();
    if !registered || !state.worktree.exists() {
        problems.push(format!(
            "the update worktree {} is missing",
            state.worktree.display()
        ));
    }
    if !branch {
        problems.push(format!("the branch '{UPDATE_BRANCH}' is missing"));
    }
    if problems.is_empty() {
        let wt = Git::worktree(&state.worktree);
        let head_branch = wt.raw(&["symbolic-ref", "--short", "-q", "HEAD"])?;
        if head_branch.stdout.trim() != UPDATE_BRANCH {
            problems.push(format!("the update worktree is not on '{UPDATE_BRANCH}'"));
        }
        let in_conflict = state
            .upstreams
            .iter()
            .any(|u| u.step == UpstreamStep::Conflict);
        if wt.merge_in_progress()? != in_conflict {
            problems.push(if in_conflict {
                "an upstream is recorded in conflict but no merge is in progress in the update worktree".to_string()
            } else {
                "a merge is in progress in the update worktree that the update state does not record".to_string()
            });
        }
    }
    checks.pass_or_fail(
        "update_state",
        None,
        problems.is_empty(),
        if problems.is_empty() {
            format!(
                "the update in progress matches its worktree and branch ({})",
                state.worktree.display()
            )
        } else {
            format!(
                "{}; run `git intake abort` to remove the update",
                problems.join("; ")
            )
        },
    );
    Ok(())
}

impl Render for DoctorDoc {
    fn render(&self) -> String {
        let mut out = String::new();
        for c in &self.checks {
            let tag = match c.status {
                CheckStatus::Pass => "pass",
                CheckStatus::Fail => "FAIL",
                CheckStatus::Warn => "warn",
            };
            match &c.upstream {
                Some(u) => out.push_str(&format!("[{tag}] {}: {}\n", u, c.detail)),
                None => out.push_str(&format!("[{tag}] {}\n", c.detail)),
            }
        }
        out.push_str(if self.healthy {
            "all checks passed\n"
        } else {
            "some checks failed\n"
        });
        out
    }
}
