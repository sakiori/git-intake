//! `start`, `continue` and `abort`: the merge phase of an update.

use std::fs;

use anyhow::{Context, Result, bail};

use crate::Exit;
use crate::config::Upstream;
use crate::drift::{self, DriftState};
use crate::git::{Git, abbrev, branch_ref, tracking_ref};
use crate::remotes;
use crate::repo::{Repo, bullet};
use crate::state::{
    self, ConflictFile, MergeKind, QueuedUpstream, Resolution, UPDATE_BRANCH, UpdateState,
    UpstreamStep,
};
use crate::sync_record;

pub fn start(repo: &Repo, names: &[String]) -> Result<Exit> {
    if let Some(state) = repo.state()? {
        bail!(
            "an update is already in progress (worktree {}); run `git fork-sync status` to see it, then `continue`, `verify`, `finish` or `abort`",
            state.worktree.display()
        );
    }
    if repo.update_worktree_registered()?
        || repo.paths.worktree.exists()
        || repo.update_branch_exists()?
    {
        bail!(
            "an interrupted update left its worktree or the branch '{UPDATE_BRANCH}' behind; run `git fork-sync abort` to remove them"
        );
    }
    let config = repo.load_config()?.config;
    let selected = config.select(names)?;
    repo.require_clean(&config.integration)?;
    let git = &repo.git;

    remotes::ensure_all(git, &selected)?;
    let mut fetch_failures = Vec::new();
    for up in &selected {
        if let Err(why) = remotes::fetch(git, up)? {
            fetch_failures.push(format!("upstream '{}': {why}", up.name));
        }
    }
    if !fetch_failures.is_empty() {
        bail!(
            "cannot fetch the selected upstreams:\n{}",
            bullet(&fetch_failures)
        );
    }

    let integration_ref = branch_ref(&config.integration);
    let base = git.commit(&integration_ref)?.with_context(|| {
        format!(
            "the integration branch '{}' does not exist",
            config.integration
        )
    })?;
    if let Some(publish) = &config.publish {
        require_not_behind_published(git, publish, &config.integration, &base)?;
    }

    let latest = sync_record::latest(&sync_record::records(git, &base, &config)?);
    let mut refusals = Vec::new();
    let mut queue = Vec::new();
    let mut current = Vec::new();
    for up in &selected {
        let d = drift::evaluate(git, &base, up, latest.get(&up.name))?;
        let target = match (d.state, &d.tip) {
            (DriftState::Rewritten, _) => {
                refusals.push(format!(
                    "upstream '{}' was rewritten: {}",
                    up.name,
                    d.detail.unwrap_or_default()
                ));
                continue;
            }
            (_, Some(tip)) => tip.clone(),
            (_, None) => {
                refusals.push(format!(
                    "upstream '{}' is unavailable: {}",
                    up.name,
                    d.detail.unwrap_or_default()
                ));
                continue;
            }
        };
        if d.state == DriftState::Current {
            current.push((up.name.clone(), target));
            continue;
        }
        match merge_kind(git, &base, up, &target, d.last_synced.is_some())? {
            Ok(kind) => queue.push(QueuedUpstream {
                name: up.name.clone(),
                commits: d.commits_behind.unwrap_or_default(),
                target,
                previous: d.last_synced.clone(),
                kind,
                step: UpstreamStep::Pending,
                head_before: None,
                merge_commit: None,
                conflicts: Vec::new(),
            }),
            Err(why) => refusals.push(why),
        }
    }
    if !refusals.is_empty() {
        bail!("cannot start the update:\n{}", bullet(&refusals));
    }
    for (name, tip) in &current {
        println!("{name}: current at {}", abbrev(tip));
    }
    if queue.is_empty() {
        println!("every selected upstream is current; nothing to update");
        return Ok(Exit::Success);
    }

    git.run(&["config", "rerere.enabled", "true"])?;
    fs::create_dir_all(&repo.paths.root)
        .with_context(|| format!("cannot create {}", repo.paths.root.display()))?;
    git.run(&[
        "worktree",
        "add",
        "--quiet",
        "-b",
        UPDATE_BRANCH,
        &repo.paths.worktree.to_string_lossy(),
        &base,
    ])?;
    let mut state = UpdateState::new(config, base.clone(), repo.paths.worktree.clone(), queue);
    repo.save_state(&state)?;
    println!(
        "update started from {} at {} in {}",
        state.config.integration,
        abbrev(&base),
        state.worktree.display()
    );
    for q in &state.upstreams {
        println!(
            "{}: queued at {} ({} new commit{})",
            q.name,
            abbrev(&q.target),
            q.commits,
            plural(q.commits)
        );
    }
    merge_pending(repo, &mut state)
}

fn require_not_behind_published(
    git: &Git,
    publish: &str,
    integration: &str,
    local: &str,
) -> Result<()> {
    if git.remote_url(publish)?.is_none() {
        bail!("the publish remote '{publish}' does not exist");
    }
    let out = git.raw(&["fetch", "--quiet", "--no-tags", publish])?;
    if !out.success() {
        bail!(
            "cannot fetch the publish remote '{publish}' to compare '{integration}': {}",
            out.into_error()
        );
    }
    if let Some(published) = git.commit(&tracking_ref(publish, integration))? {
        let missing = git.count(local, &published)?;
        if missing > 0 {
            bail!(
                "the integration branch '{integration}' is behind '{publish}/{integration}' by {missing} commit{}; \
                 bring it up to date first (for example `git merge --ff-only {publish}/{integration}` on '{integration}')",
                plural(missing)
            );
        }
    }
    Ok(())
}

/// Decides how a queued upstream is merged, or why it cannot be.
fn merge_kind(
    git: &Git,
    base: &str,
    up: &Upstream,
    target: &str,
    has_record: bool,
) -> Result<std::result::Result<MergeKind, String>> {
    let shared = has_record || git.merge_base(base, target)?.is_some();
    let Some(path) = &up.path else {
        return Ok(if shared {
            Ok(MergeKind::Root)
        } else {
            Err(format!(
                "upstream '{}' at the repository root shares no history with the integration branch; \
                 create the integration branch from {} first",
                up.name,
                up.source()
            ))
        });
    };
    let exists = git.path_exists(base, path)?;
    Ok(match (exists, shared) {
        (false, false) => Ok(MergeKind::Import),
        (true, true) => Ok(MergeKind::Subtree),
        (true, false) => Err(format!(
            "upstream '{}': path '{path}' already holds content that does not come from the upstream's history; \
             move that content away or choose another path",
            up.name
        )),
        (false, true) => Err(format!(
            "upstream '{}': its history is part of the integration branch but path '{path}' does not exist; \
             restore the path or correct the configuration",
            up.name
        )),
    })
}

/// Merges every pending upstream in order, stopping at the first conflict.
fn merge_pending(repo: &Repo, state: &mut UpdateState) -> Result<Exit> {
    let wt = Git::worktree(&state.worktree);
    for i in 0..state.upstreams.len() {
        if state.upstreams[i].step != UpstreamStep::Pending {
            continue;
        }
        let up = upstream_of(state, i)?;
        let target = state.upstreams[i].target.clone();
        let msg = merge_message(&repo.git, &up, &target)?;
        state.upstreams[i].head_before = Some(wt.line(&["rev-parse", "HEAD"])?);
        repo.save_state(state)?;
        println!(
            "{}: merging {} at {}",
            up.name,
            up.source(),
            abbrev(&target)
        );

        let failure = match state.upstreams[i].kind {
            MergeKind::Root => {
                failed(wt.raw(&["merge", "--no-ff", "--no-edit", "-m", &msg, &target])?)
            }
            MergeKind::Subtree => failed(wt.raw(&[
                "merge",
                "--no-ff",
                "--no-edit",
                "-X",
                &format!("subtree={}", up.path.as_deref().unwrap_or_default()),
                "-m",
                &msg,
                &target,
            ])?),
            MergeKind::Import => import(&wt, &up, &target, &msg)?,
        };

        match failure {
            None => {
                let commit = wt.line(&["rev-parse", "HEAD"])?;
                println!("{}: merged as {}", up.name, abbrev(&commit));
                let q = &mut state.upstreams[i];
                q.merge_commit = Some(commit);
                q.step = UpstreamStep::Merged;
                repo.save_state(state)?;
            }
            Some(out) => {
                let unmerged = wt.unmerged_files()?;
                if unmerged.is_empty() && !wt.merge_in_progress()? {
                    return Err(anyhow::Error::new(out.into_error()))
                        .with_context(|| format!("merging upstream '{}' failed", up.name));
                }
                let remaining = wt.rerere_remaining()?;
                let q = &mut state.upstreams[i];
                q.conflicts = unmerged
                    .iter()
                    .map(|p| ConflictFile {
                        path: p.clone(),
                        resolution: if remaining.contains(p) {
                            Resolution::Unresolved
                        } else {
                            Resolution::Recorded
                        },
                    })
                    .collect();
                q.step = UpstreamStep::Conflict;
                repo.save_state(state)?;
                print_conflict(state, i, &out.stderr);
                return Ok(Exit::Negative);
            }
        }
    }
    println!("every queued upstream is merged; next: git fork-sync verify");
    Ok(Exit::Success)
}

fn failed(out: crate::git::Output) -> Option<crate::git::Output> {
    (!out.success()).then_some(out)
}

/// First import of a path upstream: keep its history with an `ours` merge and
/// read its tree under the path.
fn import(wt: &Git, up: &Upstream, target: &str, msg: &str) -> Result<Option<crate::git::Output>> {
    let path = up.path.as_deref().unwrap_or_default();
    let out = wt.raw(&[
        "merge",
        "--no-ff",
        "-s",
        "ours",
        "--no-commit",
        "--allow-unrelated-histories",
        target,
    ])?;
    if !out.success() {
        return Ok(Some(out));
    }
    wt.run(&["read-tree", &format!("--prefix={path}/"), "-u", target])?;
    Ok(failed(wt.raw(&["commit", "--no-edit", "-m", msg])?))
}

fn upstream_of(state: &UpdateState, i: usize) -> Result<Upstream> {
    let name = &state.upstreams[i].name;
    state
        .config
        .upstream(name)
        .cloned()
        .with_context(|| format!("the update state names unknown upstream '{name}'"))
}

fn merge_message(git: &Git, up: &Upstream, target: &str) -> Result<String> {
    let short = git.short(target)?;
    Ok(sync_record::message(&up.name, &up.source(), &short, target))
}

fn print_conflict(state: &UpdateState, i: usize, stderr: &str) {
    let q = &state.upstreams[i];
    let up = state.config.upstream(&q.name);
    println!(
        "{}: conflicts merging {} at {} in {}",
        q.name,
        up.map(Upstream::source).unwrap_or_default(),
        abbrev(&q.target),
        state.worktree.display()
    );
    if q.conflicts.is_empty() {
        println!(
            "the merge stopped without conflicted files:\n{}",
            stderr.trim()
        );
    }
    for c in &q.conflicts {
        println!("  {}  ({})", c.path, c.resolution.describe());
    }
    if q.conflicts
        .iter()
        .any(|c| c.resolution == Resolution::Recorded)
    {
        println!(
            "files resolved by a recorded resolution already hold the earlier resolution; review them and stage them"
        );
    }
    let merged: Vec<&str> = state
        .upstreams
        .iter()
        .filter(|u| u.step == UpstreamStep::Merged)
        .map(|u| u.name.as_str())
        .collect();
    let pending: Vec<&str> = state
        .upstreams
        .iter()
        .filter(|u| u.step == UpstreamStep::Pending)
        .map(|u| u.name.as_str())
        .collect();
    println!("merged: {}", list_or_none(&merged));
    println!("pending: {}", list_or_none(&pending));
    println!(
        "resolve the conflicts in {}, stage them with `git add`, then run `git fork-sync continue`",
        state.worktree.display()
    );
}

/// `continue`: concludes the merge stopped on conflicts and merges the rest.
pub fn resume(repo: &Repo) -> Result<Exit> {
    let mut state = repo.require_state()?;
    require_worktree(repo, &state)?;
    let wt = Git::worktree(&state.worktree);
    if let Some(i) = state
        .upstreams
        .iter()
        .position(|u| u.step == UpstreamStep::Conflict)
    {
        let up = upstream_of(&state, i)?;
        let target = state.upstreams[i].target.clone();
        if wt.merge_in_progress()? {
            if wt.commit("MERGE_HEAD")?.as_deref() != Some(target.as_str()) {
                bail!(
                    "the merge in progress in {} is not the merge of upstream '{}'; conclude or abort it by hand, or run `git fork-sync abort`",
                    state.worktree.display(),
                    up.name
                );
            }
            let unmerged = wt.unmerged_files()?;
            if !unmerged.is_empty() {
                bail!(
                    "upstream '{}' still has unresolved conflicts in {}:\n{}\nresolve them, stage them with `git add`, then run `git fork-sync continue`",
                    up.name,
                    state.worktree.display(),
                    bullet(&unmerged)
                );
            }
            let msg = merge_message(&repo.git, &up, &target)?;
            wt.run(&["commit", "--no-edit", "-m", &msg])?;
        } else {
            let head = wt.line(&["rev-parse", "HEAD"])?;
            let concluded = Some(head.as_str()) != state.upstreams[i].head_before.as_deref()
                && wt.commit("HEAD^2")?.as_deref() == Some(target.as_str());
            if !concluded {
                bail!(
                    "the merge of upstream '{}' is no longer in progress in {}; run `git fork-sync abort` and start again",
                    up.name,
                    state.worktree.display()
                );
            }
            let record = sync_record::record_of(&wt, "HEAD")?;
            if !record.is_some_and(|r| r.upstream == up.name && r.target == target) {
                bail!(
                    "the merge commit of upstream '{}' lacks the sync record; amend it in {} with this message, then run `git fork-sync continue`:\n\n{}",
                    up.name,
                    state.worktree.display(),
                    merge_message(&repo.git, &up, &target)?
                );
            }
        }
        let commit = wt.line(&["rev-parse", "HEAD"])?;
        let q = &mut state.upstreams[i];
        for c in &mut q.conflicts {
            if c.resolution == Resolution::Unresolved {
                c.resolution = Resolution::Manual;
            }
        }
        q.merge_commit = Some(commit.clone());
        q.step = UpstreamStep::Merged;
        state.verification = None;
        repo.save_state(&state)?;
        println!("{}: merged as {}", up.name, abbrev(&commit));
    } else if state.all_merged() {
        println!(
            "every queued upstream is already merged; next: {}",
            next_step(&state, &wt)?
        );
        return Ok(Exit::Success);
    }
    merge_pending(repo, &mut state)
}

/// `abort`: removes the update worktree, branch and state.
pub fn abort(repo: &Repo) -> Result<Exit> {
    let state = repo.state()?;
    let removed = repo.remove_update_artifacts()?;
    state::remove(&repo.paths.state)?;
    if state.is_none() && removed.is_empty() {
        println!("no update is in progress; nothing to abort");
        return Ok(Exit::Success);
    }
    for r in &removed {
        println!("removed {r}");
    }
    if let Some(state) = state {
        println!(
            "update aborted; '{}' and the mirror branches are unchanged",
            state.config.integration
        );
    } else {
        println!("removed the remains of an interrupted update");
    }
    Ok(Exit::Success)
}

/// Refuses when the update worktree is missing.
pub fn require_worktree(repo: &Repo, state: &UpdateState) -> Result<()> {
    if !state.worktree.exists() || !repo.update_worktree_registered()? {
        bail!(
            "the update worktree {} is missing; run `git fork-sync abort` and start again",
            state.worktree.display()
        );
    }
    Ok(())
}

/// The step an update has reached, as a stable name.
pub fn step_name(state: &UpdateState, wt: &Git) -> Result<&'static str> {
    if state
        .upstreams
        .iter()
        .any(|u| u.step == UpstreamStep::Conflict)
    {
        return Ok("conflict");
    }
    if !state.all_merged() {
        return Ok("merging");
    }
    match &state.verification {
        Some(v) if v.passed && Some(v.commit.as_str()) == head(wt)?.as_deref() => Ok("verified"),
        Some(v) if !v.passed => Ok("verification_failed"),
        _ => Ok("merged"),
    }
}

/// The command to run next.
pub fn next_step(state: &UpdateState, wt: &Git) -> Result<String> {
    Ok(match step_name(state, wt)? {
        "conflict" => format!(
            "resolve the conflicts in {}, stage them, then run `git fork-sync continue`",
            state.worktree.display()
        ),
        "merging" => "git fork-sync continue".to_string(),
        "verified" => "git fork-sync finish".to_string(),
        "verification_failed" => format!(
            "fix the failures on the update branch in {}, commit, then run `git fork-sync verify`",
            state.worktree.display()
        ),
        _ => "git fork-sync verify".to_string(),
    })
}

fn head(wt: &Git) -> Result<Option<String>> {
    if !wt.dir().exists() {
        return Ok(None);
    }
    wt.commit("HEAD")
}

pub fn plural(n: u64) -> &'static str {
    if n == 1 { "" } else { "s" }
}

fn list_or_none(items: &[&str]) -> String {
    if items.is_empty() {
        "none".to_string()
    } else {
        items.join(", ")
    }
}
