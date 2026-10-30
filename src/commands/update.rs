//! `start`, `continue` and `abort`: the merge phase of an update.

use std::fs;

use anyhow::{Context, Result, bail};

use crate::Exit;
use crate::commands::merge_request::{self, Landing};
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
        if let Some(mr) = state
            .merge_request
            .as_ref()
            .filter(|_| state.finishing.is_none())
        {
            bail!(
                "the previous update waits for its merge request from '{}' into '{}' on '{}'; \
                 run `git intake finish` once it is merged, or `git intake abort` to withdraw it",
                mr.branch,
                state.config.integration,
                mr.remote
            );
        }
        if state.finishing.is_some() {
            bail!(
                "the previous update is finishing; run `git intake finish` to publish it, \
                 or `git intake finish --no-publish` to close it locally"
            );
        }
        bail!(
            "an update is already in progress (worktree {}); run `git intake status` to see it, then `continue`, `verify`, `finish` or `abort`",
            state.worktree.display()
        );
    }
    if repo.update_worktree_registered()?
        || repo.paths.worktree.exists()
        || repo.update_branch_exists()?
    {
        bail!(
            "an interrupted update left its worktree or the branch '{UPDATE_BRANCH}' behind; run `git intake abort` to remove them"
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
        require_not_behind_published(git, &publish.remote, &config.integration, &base)?;
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
        let has_record = d.last_synced_source == Some(drift::SyncSource::Record);
        match merge_kind(git, &base, up, &target, has_record)? {
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
///
/// An upstream counts as synced when it has a sync record or the history
/// shows a last synced commit (see [`drift::adopted_base`]); that decision
/// depends only on the integration branch and the upstream's own branch. A
/// root upstream must be synced. A synced path upstream is merged with the
/// subtree strategy; an unsynced one is imported when its path is absent and
/// refused when the path holds other content.
fn merge_kind(
    git: &Git,
    base: &str,
    up: &Upstream,
    target: &str,
    has_record: bool,
) -> Result<std::result::Result<MergeKind, String>> {
    let synced = has_record || drift::adopted_base(git, base, up, target)?.is_some();
    let Some(path) = &up.path else {
        return Ok(if synced {
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
    let exists = git.object_type(base, path)?.is_some();
    Ok(match (synced, exists) {
        (true, true) => Ok(MergeKind::Subtree),
        (true, false) => Err(format!(
            "upstream '{}' was imported before, but path '{path}' was removed on the integration branch; \
             restore the path or correct the configuration",
            up.name
        )),
        (false, false) => match blocking_prefix(git, base, path)? {
            None => Ok(MergeKind::Import),
            Some(prefix) => Err(format!(
                "upstream '{}': '{prefix}' is a file on the integration branch; importing under '{path}' would replace it",
                up.name
            )),
        },
        (false, true) => Err(unrelated_content(up, path)),
    })
}

/// Why a path that holds content outside the upstream's history cannot take it.
pub fn unrelated_content(up: &Upstream, path: &str) -> String {
    format!(
        "upstream '{}': path '{path}' holds content unrelated to the history of {}; \
         no merge on the integration branch brought that history into the path, so it cannot be merged there",
        up.name,
        up.source()
    )
}

/// The first leading component of `path` that exists in `rev` as something
/// other than a directory.
pub fn blocking_prefix(git: &Git, rev: &str, path: &str) -> Result<Option<String>> {
    let parts: Vec<&str> = path.split('/').collect();
    for n in 1..parts.len() {
        let prefix = parts[..n].join("/");
        match git.object_type(rev, &prefix)?.as_deref() {
            None => return Ok(None),
            Some("tree") => {}
            Some(_) => return Ok(Some(prefix)),
        }
    }
    Ok(None)
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
        let kind = state.upstreams[i].kind;

        // A merge that completed before the state could be saved.
        if carries_record(&wt, &up, &target)? {
            let commit = confirm_merged(&wt, &up, &target)?;
            mark_merged(repo, state, i, commit)?;
            continue;
        }
        if wt.merge_in_progress()? {
            let ours = wt.commit("MERGE_HEAD")?.as_deref() == Some(target.as_str());
            if ours && kind != MergeKind::Import {
                // A merge of this upstream that stopped before the state was
                // saved: its output, saved right after the merge, still says
                // which files rerere resolved. Without it, nothing is
                // labelled as resolved by a recorded resolution.
                state.upstreams[i].head_before = Some(wt.line(&["rev-parse", "HEAD"])?);
                let output =
                    fs::read_to_string(merge_output_path(repo, &up.name)).unwrap_or_default();
                return stop_on_conflict(repo, state, i, &output);
            }
            wt.run(&["merge", "--abort"])?;
        }

        let msg = merge_message(&repo.git, &up, &target)?;
        let head_before = wt.line(&["rev-parse", "HEAD"])?;
        state.upstreams[i].head_before = Some(head_before.clone());
        repo.save_state(state)?;
        println!(
            "{}: merging {} at {}",
            up.name,
            up.source(),
            abbrev(&target)
        );

        let failure = match kind {
            MergeKind::Root | MergeKind::Subtree => {
                let mut args: Vec<String> = ["merge", "--no-ff", "--no-log", "--no-edit"]
                    .map(String::from)
                    .to_vec();
                if kind == MergeKind::Subtree {
                    args.push("-X".into());
                    args.push(format!(
                        "subtree={}",
                        up.path.as_deref().unwrap_or_default()
                    ));
                }
                args.extend(["-m".to_string(), msg, target.clone()]);
                let output_path = merge_output_path(repo, &up.name);
                state::remove(&output_path)?;
                let out = wt.raw(&args)?;
                save_merge_output(&output_path, &out)?;
                (!out.success()).then_some(out)
            }
            MergeKind::Import => {
                import(&wt, &up, &target, &msg)?;
                None
            }
        };

        match failure {
            None => {
                let commit = confirm_merged(&wt, &up, &target)?;
                mark_merged(repo, state, i, commit)?;
            }
            Some(out) => {
                if !wt.merge_in_progress()? {
                    return Err(anyhow::Error::new(out.into_error()))
                        .with_context(|| format!("merging upstream '{}' failed", up.name));
                }
                let output = format!("{}{}", out.stdout, out.stderr);
                return stop_on_conflict(repo, state, i, &output);
            }
        }
    }
    println!("every queued upstream is merged; next: git intake verify");
    Ok(Exit::Success)
}

/// Where a merge's output is kept until the update's state records it.
fn merge_output_path(repo: &Repo, name: &str) -> std::path::PathBuf {
    repo.paths.logs.join(format!("merge-{name}.out"))
}

/// Saves what a merge printed before anything else can fail, so that a
/// recovery after an interruption still sees rerere's report.
fn save_merge_output(path: &std::path::Path, out: &crate::git::Output) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
    }
    fs::write(path, format!("{}{}", out.stdout, out.stderr))
        .with_context(|| format!("cannot write {}", path.display()))
}

fn mark_merged(repo: &Repo, state: &mut UpdateState, i: usize, commit: String) -> Result<()> {
    println!("{}: merged as {}", state.upstreams[i].name, abbrev(&commit));
    let q = &mut state.upstreams[i];
    for c in &mut q.conflicts {
        if c.resolution == Resolution::Unresolved {
            c.resolution = Resolution::Manual;
        }
    }
    q.merge_commit = Some(commit);
    q.step = UpstreamStep::Merged;
    state.verification = None;
    repo.save_state(state)
}

/// Records the conflicts of the merge stopped in the worktree and reports them.
/// `output` is what the merge printed; rerere reports there the files it
/// resolved from a recorded resolution.
fn stop_on_conflict(repo: &Repo, state: &mut UpdateState, i: usize, output: &str) -> Result<Exit> {
    let wt = Git::worktree(&state.worktree);
    let conflicts = label_conflicts(&wt.unmerged_files()?, &rerere_resolved(output));
    let q = &mut state.upstreams[i];
    q.conflicts = conflicts;
    q.step = UpstreamStep::Conflict;
    repo.save_state(state)?;
    print_conflict(state, i, output);
    Ok(Exit::Negative)
}

/// Files git's rerere reports as resolved from a recorded resolution, either
/// left in the worktree (`Resolved '<path>' ...`) or already staged under
/// `rerere.autoupdate` (`Staged '<path>' ...`). Git runs with `LC_ALL=C`, so
/// these messages are stable.
fn rerere_resolved(output: &str) -> Vec<String> {
    output
        .lines()
        .filter_map(|l| {
            let rest = l
                .strip_prefix("Resolved '")
                .or_else(|| l.strip_prefix("Staged '"))?;
            rest.strip_suffix("' using previous resolution.")
        })
        .map(str::to_string)
        .collect()
}

/// Conflicted files and where each resolution comes from: a file rerere
/// reported is resolved by a recorded resolution; every other file left
/// unmerged waits for a resolution by hand.
fn label_conflicts(unmerged: &[String], recorded: &[String]) -> Vec<ConflictFile> {
    let mut files: Vec<ConflictFile> = unmerged
        .iter()
        .map(|p| ConflictFile {
            path: p.clone(),
            resolution: if recorded.contains(p) {
                Resolution::Recorded
            } else {
                Resolution::Unresolved
            },
        })
        .collect();
    for p in recorded {
        if !unmerged.contains(p) {
            files.push(ConflictFile {
                path: p.clone(),
                resolution: Resolution::Recorded,
            });
        }
    }
    files
}

/// First import of a path upstream: keep its history with an `ours` merge and
/// read its tree under the path. All or nothing: on any failure the merge is
/// aborted and the upstream stays pending.
fn import(wt: &Git, up: &Upstream, target: &str, msg: &str) -> Result<()> {
    let path = up.path.as_deref().unwrap_or_default();
    let steps = || -> Result<()> {
        wt.run(&[
            "merge",
            "--no-ff",
            "--no-log",
            "-s",
            "ours",
            "--no-commit",
            "--allow-unrelated-histories",
            target,
        ])?;
        wt.run(&["read-tree", &format!("--prefix={path}/"), "-u", target])?;
        wt.run(&["commit", "--no-edit", "-m", msg])?;
        Ok(())
    };
    if let Err(err) = steps() {
        if wt.merge_in_progress().unwrap_or(false) {
            wt.run(&["merge", "--abort"])
                .context("aborting the failed import")?;
        }
        return Err(err.context(format!(
            "importing upstream '{}' under '{path}' failed; the import was undone and the upstream is still pending. \
             Fix the cause, then run `git intake continue`, or run `git intake abort`",
            up.name
        )));
    }
    Ok(())
}

/// Whether HEAD is a merge carrying this upstream's sync record for `target`.
fn carries_record(wt: &Git, up: &Upstream, target: &str) -> Result<bool> {
    Ok(sync_record::record_of(wt, "HEAD")?
        .is_some_and(|r| r.upstream == up.name && r.target == target))
}

/// Checks that HEAD is a complete merge of the upstream before it is counted
/// as merged: the sync record names the upstream and target, the target is
/// merged, and a path upstream's path exists. Returns HEAD.
fn confirm_merged(wt: &Git, up: &Upstream, target: &str) -> Result<String> {
    let head = wt.line(&["rev-parse", "HEAD"])?;
    let mut problems = Vec::new();
    if !carries_record(wt, up, target)? {
        problems.push(format!(
            "the commit message does not end with the sync record trailers for upstream '{}' and target {}",
            up.name,
            abbrev(target)
        ));
    }
    if !wt.is_ancestor(target, &head)? {
        problems.push(format!("the target {} is not merged", abbrev(target)));
    }
    if let Some(path) = &up.path
        && wt.object_type(&head, path)?.as_deref() != Some("tree")
    {
        problems.push(format!("path '{path}' does not exist in the merge"));
    }
    if !problems.is_empty() {
        bail!(
            "the merge of upstream '{}' at {} in {} is not a complete sync merge:\n{}\n\
             a hook or configuration may have changed the merge; correct the commit (expected message below) \
             and run `git intake continue`, or run `git intake abort`:\n\n{}",
            up.name,
            abbrev(&head),
            wt.dir().display(),
            bullet(&problems),
            sync_record::message(&up.name, &up.source(), abbrev(target), target)
        );
    }
    Ok(head)
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
        "resolve the conflicts in {}, stage them with `git add`, then run `git intake continue`",
        state.worktree.display()
    );
}

/// `continue`: concludes the merge stopped on conflicts and merges the rest.
pub fn resume(repo: &Repo) -> Result<Exit> {
    let mut state = repo.require_state()?;
    require_merging(&state)?;
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
                    "the merge in progress in {} is not the merge of upstream '{}'; conclude or abort it by hand, or run `git intake abort`",
                    state.worktree.display(),
                    up.name
                );
            }
            let unmerged = wt.unmerged_files()?;
            if !unmerged.is_empty() {
                bail!(
                    "upstream '{}' still has unresolved conflicts in {}:\n{}\nresolve them, stage them with `git add`, then run `git intake continue`",
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
                    "the merge of upstream '{}' is no longer in progress in {}; run `git intake abort` and start again",
                    up.name,
                    state.worktree.display()
                );
            }
        }
        let commit = confirm_merged(&wt, &up, &target)?;
        mark_merged(repo, &mut state, i, commit)?;
    } else if state.all_merged() {
        println!(
            "every queued upstream is already merged; next: {}",
            next_step(&state, &wt)?
        );
        return Ok(Exit::Success);
    }
    merge_pending(repo, &mut state)
}

/// `abort`: removes the update worktree, branch, logs and state, and
/// withdraws a merge request the update waits for.
pub fn abort(repo: &Repo) -> Result<Exit> {
    let state = repo.state()?;
    if let Some(finishing) = state.as_ref().and_then(|s| s.finishing.as_ref()) {
        let mut moved = Vec::new();
        for m in &finishing.moves {
            if repo.git.commit(&branch_ref(&m.branch))? != m.old {
                moved.push(m.branch.as_str());
            }
        }
        if !moved.is_empty() {
            bail!(
                "`finish` has already moved {}; the update can no longer be aborted. Run `git intake finish` to complete it, \
                 or `git intake finish --no-publish` to close it without publishing",
                moved.join(", ")
            );
        }
    }
    let merge_request = state.as_ref().and_then(|s| s.merge_request.clone());
    let mut unverified = None;
    if let (Some(state), Some(mr)) = (&state, &merge_request) {
        let landing = if state.finishing.is_some() {
            // `finish` plans its moves only once the verified tree landed.
            None
        } else {
            Some(merge_request::inspect(repo, &mut state.clone())?)
        };
        match landing {
            None | Some(Landing::Landed { .. }) => bail!(
                "the merge request from '{}' was merged with the verified commit; the update can no longer be aborted. \
                 Run `git intake finish` to complete it",
                mr.branch
            ),
            Some(Landing::Unverified { tip, landing, head }) => {
                unverified = Some((tip, landing, head))
            }
            Some(_) => {}
        }
        merge_request::delete_branch(&repo.git, mr)?;
    }
    let mut removed = repo.remove_update_artifacts()?;
    removed.extend(repo.remove_logs()?);
    state::remove(&repo.paths.state)?;
    if state.is_none() && removed.is_empty() {
        println!("no update is in progress; nothing to abort");
        return Ok(Exit::Success);
    }
    for r in &removed {
        println!("removed {r}");
    }
    match state {
        Some(state) => {
            println!("update aborted; {}", unchanged_branches(&state));
            if let (Some(mr), Some((tip, landing, head))) = (&merge_request, &unverified) {
                let integration = &state.config.integration;
                let how = match head {
                    Some(head) => format!(
                        "the merge request's head {} landed instead of the verified commit",
                        abbrev(head)
                    ),
                    None => format!(
                        "{} brought in '{}' without the verified tree",
                        abbrev(landing),
                        mr.branch
                    ),
                };
                println!(
                    "'{}/{integration}' at {} holds an unverified merge: {how}; review it before building on it",
                    mr.remote,
                    abbrev(tip)
                );
            } else if let Some(mr) = merge_request {
                let integration = &state.config.integration;
                if mr.requested {
                    println!(
                        "close the merge request from '{}' into '{integration}' on '{}'",
                        mr.branch, mr.remote
                    );
                } else {
                    println!(
                        "if a merge request from '{}' into '{integration}' was opened on '{}', close it",
                        mr.branch, mr.remote
                    );
                }
            }
        }
        None => println!("removed the remains of an interrupted update"),
    }
    Ok(Exit::Success)
}

/// Names the integration branch and the configured mirror branches, which an
/// aborted update leaves as they were.
fn unchanged_branches(state: &UpdateState) -> String {
    let mut names = vec![format!("'{}'", state.config.integration)];
    names.extend(
        state
            .config
            .upstreams
            .iter()
            .filter_map(|u| u.mirror.as_ref())
            .map(|m| format!("'{m}'")),
    );
    match names.as_slice() {
        [one] => format!("{one} is unchanged"),
        [rest @ .., last] => format!("{} and {last} are unchanged", rest.join(", ")),
        [] => unreachable!("the integration branch is always named"),
    }
}

/// Refuses merge-phase commands once the update has left the merge phase.
pub fn require_merging(state: &UpdateState) -> Result<()> {
    if state.finishing.is_some() {
        bail!("`finish` is in progress; run `git intake finish` to complete it");
    }
    if let Some(mr) = &state.merge_request {
        bail!(
            "the update waits for its merge request from '{}' into '{}' on '{}'; run `git intake finish` once it is merged, \
             or `git intake abort` to withdraw it",
            mr.branch,
            state.config.integration,
            mr.remote
        );
    }
    Ok(())
}

/// Refuses when the update worktree is missing.
pub fn require_worktree(repo: &Repo, state: &UpdateState) -> Result<()> {
    if !state.worktree.exists() || !repo.update_worktree_registered()? {
        bail!(
            "the update worktree {} is missing; run `git intake abort` and start again",
            state.worktree.display()
        );
    }
    Ok(())
}

/// The step an update has reached, as a stable name.
pub fn step_name(state: &UpdateState, wt: &Git) -> Result<&'static str> {
    if state.finishing.is_some() {
        return Ok("finishing");
    }
    if state.merge_request.is_some() {
        return Ok("waiting_for_merge_request");
    }
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
            "resolve the conflicts in {}, stage them, then run `git intake continue`",
            state.worktree.display()
        ),
        "merging" => "git intake continue".to_string(),
        "verified" => "git intake finish".to_string(),
        "waiting_for_merge_request" => match &state.merge_request {
            Some(mr) => merge_request::waiting_next(mr, &state.config.integration),
            None => "git intake finish".to_string(),
        },
        "finishing" => match state.finishing.as_ref().and_then(|f| f.push_error.as_ref()) {
            Some(_) => "`git intake finish` to retry the same push, or `git intake finish --no-publish` to close the update locally".to_string(),
            None => "git intake finish".to_string(),
        },
        "verification_failed" => format!(
            "fix the failures on the update branch in {}, commit, then run `git intake verify`",
            state.worktree.display()
        ),
        _ => "git intake verify".to_string(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rerere_reports_are_parsed() {
        let out = "Auto-merging a\nCONFLICT (content): Merge conflict in a\nResolved 'a' using previous resolution.\n\
                   Staged 'dir/b c' using previous resolution.\nRecorded preimage for 'd'\n";
        assert_eq!(rerere_resolved(out), vec!["a", "dir/b c"]);
    }

    #[test]
    fn only_reported_files_are_recorded() {
        let files = label_conflicts(&["a".into(), "d".into()], &["a".into(), "b".into()]);
        let labels: Vec<(&str, Resolution)> = files
            .iter()
            .map(|f| (f.path.as_str(), f.resolution))
            .collect();
        assert_eq!(
            labels,
            vec![
                ("a", Resolution::Recorded),
                ("d", Resolution::Unresolved),
                ("b", Resolution::Recorded)
            ]
        );
    }
}
