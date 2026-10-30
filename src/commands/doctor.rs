//! `doctor`: checks of configuration, upstreams, history, journals and
//! state. It changes nothing but the references of its own fetches, which it
//! removes before it exits.

use anyhow::Result;
use serde::Serialize;

use crate::Exit;
use crate::commands::update::unrelated_content;
use crate::config::{Config, Mode, Upstream};
use crate::drift::adopted_base;
use crate::git::{Git, abbrev, branch_ref, lines};
use crate::output::{Render, emit};
use crate::port::{self, Standing};
use crate::repo::{MainWorktree, Repo};
use crate::state::{UPDATE_BRANCH, UpstreamStep, Work};
use crate::sync_record::{self, Record};
use crate::upstreams::{self, Fetches, REFS_PREFIX};

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
    let loaded = repo.load_config();
    match &loaded {
        Ok(loaded) => checks.add(
            "configuration",
            None,
            CheckStatus::Pass,
            format!(
                "the configuration on '{}' is valid",
                loaded.config.integration
            ),
        ),
        Err(err) => checks.add("configuration", None, CheckStatus::Fail, format!("{err:#}")),
    }
    // Before this command fetches anything of its own.
    reference_checks(repo, &mut checks)?;
    if let Ok(loaded) = &loaded {
        upstream_checks(repo, &loaded.config, &mut checks)?;
        remote_checks(repo, &loaded.config, &mut checks)?;
        journal_checks(repo, &loaded.config, &mut checks)?;
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

/// The references the update in progress holds, and references under
/// `refs/git-intake/` that no update accounts for.
fn reference_checks(repo: &Repo, checks: &mut Checks) -> Result<()> {
    let state = repo.state().ok().flatten();
    let held = state.as_ref().map(upstreams::held_by).unwrap_or_default();
    let existing = upstreams::existing(&repo.git)?;
    let value = |name: &str| existing.iter().find(|(n, _)| n == name).map(|(_, v)| v);
    // The commits of held references that are missing: another reference
    // that still holds one keeps it in the repository and is not stray.
    let mut held_names: Vec<&String> = held.keys().collect();
    held_names.sort();
    let mut needed: Vec<&String> = Vec::new();
    for name in held_names {
        let tip = &held[name];
        let (ok, detail) = match value(name) {
            Some(v) if v == tip => (true, String::new()),
            Some(v) => (
                false,
                format!(
                    "the update's reference {name} is at {} instead of {}, the tip `start` fetched; \
                     set it back with `git update-ref {name} {tip}`",
                    abbrev(v),
                    abbrev(tip)
                ),
            ),
            None if repo.git.object_exists(tip)? => {
                needed.push(tip);
                (
                    false,
                    format!(
                        "the update's reference {name} is missing; `git intake status`, `continue`, `verify` or \
                         `finish` restores it at {} from the update state",
                        abbrev(tip)
                    ),
                )
            }
            None => (
                false,
                format!(
                    "the update's reference {name} is missing, and its commit {} is no longer in the repository; \
                     run `git intake abort` and start again",
                    abbrev(tip)
                ),
            ),
        };
        if !ok {
            checks.add("update_references", None, CheckStatus::Fail, detail);
        }
    }
    if !held.is_empty() && !checks.0.iter().any(|c| c.check == "update_references") {
        checks.add(
            "update_references",
            None,
            CheckStatus::Pass,
            "the update's references are at the tips `start` fetched".to_string(),
        );
    }
    let mut keeping = Vec::new();
    let mut stray = Vec::new();
    for (name, oid) in &existing {
        if held.contains_key(name) {
            continue;
        }
        let shown = format!("{name} at {}", abbrev(oid));
        if needed.contains(&oid) {
            keeping.push(shown);
        } else {
            stray.push(shown);
        }
    }
    if !keeping.is_empty() {
        checks.add(
            "references",
            None,
            CheckStatus::Warn,
            format!(
                "{} still hold{} commits the update in progress needs; keep {} until the update's references are restored",
                keeping.join(", "),
                if keeping.len() == 1 { "s" } else { "" },
                if keeping.len() == 1 { "it" } else { "them" }
            ),
        );
    }
    checks.pass_or_fail(
        "references",
        None,
        stray.is_empty(),
        if stray.is_empty() {
            format!("no reference under {REFS_PREFIX} is left without an update")
        } else {
            format!(
                "references under {REFS_PREFIX} are left without an update: {}; an interrupted command or update \
                 left them, and nothing uses them: remove them with `git update-ref -d <reference>`",
                stray.join(", ")
            )
        },
    );
    Ok(())
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
    let mut fetches = Fetches::new(repo);
    let mut fetched = Vec::new();
    for up in &config.upstreams {
        fetched.push(fetches.upstream(up)?);
    }
    let tips: Vec<String> = fetched.iter().filter_map(|f| f.clone().ok()).collect();
    let records = sync_record::records(git, &integration, &tips)?;
    let latest = sync_record::latest(git, &records)?;
    for (up, fetched) in config.upstreams.iter().zip(&fetched) {
        let name = Some(up.name.as_str());
        checks.pass_or_fail(
            "fetch",
            name,
            fetched.is_ok(),
            match fetched {
                Ok(t) => format!(
                    "branch '{}' of {} was fetched at {}",
                    up.branch,
                    up.url,
                    abbrev(t)
                ),
                Err(why) => format!("cannot fetch branch '{}' of {}: {why}", up.branch, up.url),
            },
        );
        let tip = fetched.as_ref().ok();
        let record = latest.get(&up.name);
        match up.mode {
            Mode::Follow => follow_checks(git, config, &integration, up, record, tip, checks)?,
            Mode::Port => port_checks(git, &integration, up, record, tip, &tips, checks),
        }
    }
    drop(fetches);
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

/// The path and the last synced commit of a follow upstream.
fn follow_checks(
    git: &Git,
    config: &Config,
    integration: &str,
    up: &Upstream,
    record: Option<&Record>,
    tip: Option<&String>,
    checks: &mut Checks,
) -> Result<()> {
    let name = Some(up.name.as_str());
    // Without a record, the last synced commit the history shows.
    let adopted = match (record, tip) {
        (None, Some(t)) => adopted_base(git, integration, up, t)?,
        _ => None,
    };
    if let Some(path) = &up.path {
        let exists = git.object_type(integration, path)?.is_some();
        // Synced: a record, or a merge in the history that brought the
        // upstream into its path; unknown when the branch is not fetched.
        let synced = match (record.is_some(), tip) {
            (true, _) => Some(true),
            (false, Some(_)) => Some(adopted.is_some()),
            (false, None) => None,
        };
        let (status, detail) = match (synced, exists) {
            (Some(true), true) => (
                CheckStatus::Pass,
                if record.is_some() {
                    format!("path '{path}' exists on '{}'", config.integration)
                } else {
                    format!(
                        "path '{path}' was imported without a sync record; the next update records it"
                    )
                },
            ),
            (Some(true), false) => (
                CheckStatus::Fail,
                format!(
                    "upstream is imported but path '{path}' does not exist on '{}'",
                    config.integration
                ),
            ),
            (_, false) => (
                CheckStatus::Pass,
                format!("not imported yet; path '{path}' will be created by the first update"),
            ),
            (Some(false), true) => (CheckStatus::Fail, unrelated_content(up, path)),
            (None, true) => (
                CheckStatus::Fail,
                format!(
                    "path '{path}' exists without a sync record and {} is not available to check it",
                    up.source()
                ),
            ),
        };
        checks.add("path", name, status, detail);
    }

    match (record, tip) {
        (None, Some(_)) => {
            let detail = match &adopted {
                Some(base) => format!(
                    "no sync record yet; last synced from history is {}, contained in {}",
                    abbrev(base),
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
                    format!(
                        "last synced commit {} is contained in {}",
                        abbrev(&r.target),
                        up.source()
                    )
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
    Ok(())
}

/// The journal of a port upstream, and its commits against the fetched
/// branch.
fn port_checks(
    git: &Git,
    integration: &str,
    up: &Upstream,
    record: Option<&Record>,
    tip: Option<&String>,
    tips: &[String],
    checks: &mut Checks,
) {
    let name = Some(up.name.as_str());
    let journal = port::journal_path(&up.name);
    let Some(tip) = tip else {
        let detail = match port::read_journal(git, integration, &up.name) {
            Err(err) => format!("{err:#}"),
            Ok(_) => format!(
                "cannot check the journal {journal} against {}: the upstream is not available",
                up.source()
            ),
        };
        checks.add("journal", name, CheckStatus::Fail, detail);
        return;
    };
    let (status, detail) = match port::standing(git, integration, up, record, tip, tips) {
        Err(err) => (CheckStatus::Fail, format!("{err:#}")),
        Ok(Standing::NoBase) => (CheckStatus::Fail, port::no_base_advice(&up.name)),
        Ok(Standing::Rewritten(rewrite)) => (
            CheckStatus::Fail,
            format!("the upstream was rewritten: {}", rewrite.describe(&up.name)),
        ),
        Ok(Standing::Known(a)) if !a.journal => (
            CheckStatus::Pass,
            format!(
                "no journal yet; the first update that carries out a plan starts it at the last synced commit {}",
                abbrev(&a.base)
            ),
        ),
        Ok(Standing::Known(a)) => (
            CheckStatus::Pass,
            format!(
                "the journal {journal} is valid, and its base {} and its decided and taken commits are on {}; \
                 {} taken, {} decided, {} pending",
                abbrev(&a.base),
                up.source(),
                a.taken.len(),
                a.decided.len(),
                a.pending().len()
            ),
        ),
    };
    checks.add("journal", name, status, detail);
}

/// Git remotes left at an upstream's URL.
fn remote_checks(repo: &Repo, config: &Config, checks: &mut Checks) -> Result<()> {
    let git = &repo.git;
    let promisor = git
        .raw(&["config", "--get", "extensions.partialClone"])?
        .stdout
        .trim()
        .to_string();
    let mut found = false;
    for remote in upstreams::remotes(git)? {
        let urls = upstreams::remote_urls(git, &remote)?;
        let at_upstream = |up: &&Upstream| {
            urls.iter()
                .any(|u| upstreams::same_repository(u, &up.url, git.dir()))
        };
        for up in config.upstreams.iter().filter(at_upstream) {
            found = true;
            let partial = remote == promisor
                || git
                    .raw(&[
                        "config",
                        "--bool",
                        "--get",
                        &format!("remote.{remote}.promisor"),
                    ])?
                    .stdout
                    .trim()
                    == "true";
            let mut detail = format!(
                "git remote '{remote}' names the repository of upstream '{}' ({}); the tool fetches upstreams by their URL \
                 and needs no remote for them, and other tools take the remote for a place of the repository's own: \
                 remove it with `git remote remove {remote}`",
                up.name, up.url
            );
            if partial {
                detail.push_str(&format!(
                    ". This partial clone fetches missing objects from '{remote}': fetch the objects the history \
                     still lacks before removing the remote, or that history can no longer be read"
                ));
            }
            checks.add("remote", Some(&up.name), CheckStatus::Fail, detail);
        }
    }
    if !found {
        checks.add(
            "remote",
            None,
            CheckStatus::Pass,
            "no git remote names an upstream's repository".to_string(),
        );
    }
    Ok(())
}

/// Journals on the integration branch that belong to no port upstream.
fn journal_checks(repo: &Repo, config: &Config, checks: &mut Checks) -> Result<()> {
    let git = &repo.git;
    let Some(integration) = git.commit(&branch_ref(&config.integration))? else {
        return Ok(());
    };
    let listed = lines(&git.run(&[
        "ls-tree",
        "--name-only",
        &integration,
        &format!("{}/", port::JOURNAL_DIR),
    ])?);
    let mut stale = false;
    for path in listed {
        let Some(name) = path
            .strip_prefix(&format!("{}/", port::JOURNAL_DIR))
            .and_then(|f| f.strip_suffix(".toml"))
        else {
            continue;
        };
        let why = match config.upstream(name) {
            Some(up) if up.mode == Mode::Port => continue,
            Some(_) => format!(
                "the journal {path} belongs to upstream '{name}', which is in follow mode, where a journal means nothing; \
                 remove it, or set the upstream's mode to port"
            ),
            None => format!("the journal {path} belongs to no configured upstream; remove it"),
        };
        stale = true;
        checks.add("stale_journal", Some(name), CheckStatus::Fail, why);
    }
    if !stale {
        checks.add(
            "stale_journal",
            None,
            CheckStatus::Pass,
            "every journal belongs to an upstream in port mode".to_string(),
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
        // A copy stops without a merge in progress; only a merge leaves one.
        let merging = state
            .upstreams
            .iter()
            .any(|u| u.step == UpstreamStep::Conflict && u.follow().is_some());
        if wt.merge_in_progress()? != merging {
            problems.push(if merging {
                "an upstream is recorded in conflict but no merge is in progress in the update worktree".to_string()
            } else {
                "a merge is in progress in the update worktree that the update state does not record".to_string()
            });
        }
        for q in &state.upstreams {
            if let Work::Port(plan) = &q.work
                && q.step == UpstreamStep::Plan
                && !plan.plan.exists()
            {
                problems.push(format!(
                    "upstream '{}' waits at its plan, but the plan {} is missing",
                    q.name,
                    plan.plan.display()
                ));
            }
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
