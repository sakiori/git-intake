//! Carrying out the plan of a port upstream on the update branch: each
//! commit marked `take` is copied in plan order, and the `skip` and `have`
//! decisions are then committed to the journal in one commit.

use std::collections::HashMap;
use std::fs;

use anyhow::{Context, Result, bail};

use crate::Exit;
use crate::commands::update::{
    label_conflicts, list_or_none, merge_output_path, plural, print_conflicted_files,
    rerere_resolved, save_merge_output, settle,
};
use crate::git::{Git, abbrev, split_z};
use crate::port::{self, Decision, Journal, Verb, Verdict};
use crate::repo::{Repo, bullet};
use crate::state::{CopyStep, PortPlan, UpdateState, UpstreamStep, Work};
use crate::sync_record;

/// The note of a `have` decision recorded because a copy changed nothing.
const EMPTY_COPY_NOTE: &str = "taken, but its copy changed nothing";

/// The note of a `have` decision recorded because a copy changed only the
/// paths a copy never changes.
const OWN_FILES_COPY_NOTE: &str =
    "taken, but its copy changed nothing outside .git-intake.toml and .git-intake/";

/// The paths a copy never changes: the repository's own git-intake files.
const OWN_PATHS: [&str; 2] = [crate::config::FILE_NAME, port::JOURNAL_DIR];

/// Copies the remaining takes of upstream `i`, then commits the journal.
/// Returns the exit status when a copy stopped on conflicts or its commit
/// failed.
pub fn carry_out(repo: &Repo, state: &mut UpdateState, i: usize) -> Result<Option<Exit>> {
    let wt = Git::worktree(&state.worktree);
    let name = state.upstreams[i].name.clone();
    let takes = plan_of(state, i)?.takes.clone();
    for index in takes {
        let entry = plan_of(state, i)?.commits[index].clone();
        match entry.copy {
            CopyStep::Copied | CopyStep::Empty => continue,
            CopyStep::Conflict | CopyStep::CommitFailed => bail!(
                "the copy of {} has stopped; complete it and run `git intake continue`",
                abbrev(&entry.commit)
            ),
            CopyStep::None => {}
        }
        if let Some(head_before) = &entry.head_before {
            // A copy that began before an interruption. Nothing in the
            // worktree is thrown away: a copy already applied is committed.
            let head = wt.line(&["rev-parse", "HEAD"])?;
            if head != *head_before
                && sync_record::copy_of(&wt, &head, &name)?.as_deref() == Some(&entry.commit)
            {
                record_copy(repo, state, i, index, Some(head))?;
                continue;
            }
            if head != *head_before {
                bail!(
                    "the update branch moved from {} to {} while {} was being copied; \
                     run `git intake abort` and start again",
                    abbrev(head_before),
                    abbrev(&head),
                    abbrev(&entry.commit)
                );
            }
            if !wt.unmerged_files()?.is_empty() {
                let output = fs::read_to_string(merge_output_path(repo, &name)).unwrap_or_default();
                return stop_on_conflict(repo, state, i, index, &output).map(Some);
            }
            if !wt.tracked_changes()?.is_empty() {
                if let Some(exit) = finish_copy(repo, state, i, index)? {
                    return Ok(Some(exit));
                }
                continue;
            }
        }

        let head_before = wt.line(&["rev-parse", "HEAD"])?;
        plan_mut(state, i)?.commits[index].head_before = Some(head_before);
        repo.save_state(state)?;
        println!(
            "{name}: copying {} {}",
            abbrev(&entry.commit),
            entry.subject
        );
        let mut args = vec!["cherry-pick", "--no-commit"];
        if entry.merge {
            args.extend(["-m", "1"]);
        }
        args.push(&entry.commit);
        let output_path = merge_output_path(repo, &name);
        crate::state::remove(&output_path)?;
        let out = wt.raw(&args)?;
        save_merge_output(&output_path, &out)?;
        if !out.success() {
            if wt.unmerged_files()?.is_empty() {
                return Err(anyhow::Error::new(out.into_error())).with_context(|| {
                    format!(
                        "copying {} of upstream '{name}' failed",
                        abbrev(&entry.commit)
                    )
                });
            }
            leave_out_own_paths(repo, state, i, index)?;
            if !wt.unmerged_files()?.is_empty() {
                let output = format!("{}{}", out.stdout, out.stderr);
                return stop_on_conflict(repo, state, i, index, &output).map(Some);
            }
        }
        if let Some(exit) = finish_copy(repo, state, i, index)? {
            return Ok(Some(exit));
        }
    }
    find_overlaps(repo, state, i)?;
    commit_journal(repo, state, i)?;
    let plan = plan_of(state, i)?;
    let copied = plan
        .commits
        .iter()
        .filter(|c| c.copy == CopyStep::Copied)
        .count() as u64;
    let empty = plan
        .commits
        .iter()
        .filter(|c| c.copy == CopyStep::Empty)
        .count() as u64;
    let overlapping = plan
        .commits
        .iter()
        .filter(|c| !c.overlaps.is_empty())
        .count() as u64;
    println!(
        "{name}: plan carried out: {copied} commit{} copied{}",
        plural(copied),
        if empty > 0 {
            format!(", {empty} recorded as present because the copy changed nothing")
        } else {
            String::new()
        }
    );
    if overlapping > 0 {
        println!(
            "{name}: {overlapping} taken commit{} change{} files that earlier pending commits change too; \
             `git intake report` names them",
            plural(overlapping),
            if overlapping == 1 { "s" } else { "" }
        );
    }
    state.upstreams[i].step = UpstreamStep::Merged;
    state.verification = None;
    repo.save_state(state)?;
    Ok(None)
}

/// Concludes the copy that stopped on conflicts, or whose commit failed,
/// with what the worktree now holds. Returns the exit status when its
/// commit fails again.
pub fn conclude_copy(repo: &Repo, state: &mut UpdateState, i: usize) -> Result<Option<Exit>> {
    let wt = Git::worktree(&state.worktree);
    let name = state.upstreams[i].name.clone();
    let Some(index) = plan_of(state, i)?
        .commits
        .iter()
        .position(|c| matches!(c.copy, CopyStep::Conflict | CopyStep::CommitFailed))
    else {
        bail!("upstream '{name}' has no copy that stopped");
    };
    let entry = plan_of(state, i)?.commits[index].clone();
    let head = wt.line(&["rev-parse", "HEAD"])?;
    if Some(&head) != entry.head_before.as_ref()
        && sync_record::copy_of(&wt, &head, &name)?.as_deref() == Some(&entry.commit)
    {
        // Committed by hand with its trailers.
        state.upstreams[i].step = UpstreamStep::Pending;
        record_copy(repo, state, i, index, Some(head))?;
        return Ok(None);
    }
    if Some(&head) != entry.head_before.as_ref() {
        bail!(
            "the update branch moved to {} while the copy of {} was stopped; commit the copy with the \
             trailers of upstream '{name}', or run `git intake abort` and start again",
            abbrev(&head),
            abbrev(&entry.commit)
        );
    }
    leave_out_own_paths(repo, state, i, index)?;
    let unmerged = wt.unmerged_files()?;
    if !unmerged.is_empty() {
        bail!(
            "the copy of {} from upstream '{name}' still has unresolved conflicts in {}:\n{}\n\
             resolve them, stage them with `git add`, then run `git intake continue`",
            abbrev(&entry.commit),
            state.worktree.display(),
            bullet(&unmerged)
        );
    }
    finish_copy(repo, state, i, index)
}

/// Removes from the copy staged in the worktree its changes to the paths a
/// copy never changes, and records which ones it left out.
fn leave_out_own_paths(repo: &Repo, state: &mut UpdateState, i: usize, index: usize) -> Result<()> {
    let wt = Git::worktree(&state.worktree);
    let mut args = vec!["diff", "--cached", "--name-only", "-z", "HEAD", "--"];
    args.extend(OWN_PATHS);
    let mut paths = split_z(&wt.run(&args)?);
    for p in wt.unmerged_files()? {
        let own = OWN_PATHS
            .iter()
            .any(|o| p == *o || p.starts_with(&format!("{o}/")));
        if own && !paths.contains(&p) {
            paths.push(p);
        }
    }
    if paths.is_empty() {
        return Ok(());
    }
    let mut restore: Vec<&str> = vec!["restore", "--source=HEAD", "--staged", "--worktree", "--"];
    restore.extend(paths.iter().map(String::as_str));
    wt.run(&restore)?;
    let name = state.upstreams[i].name.clone();
    let entry = &mut plan_mut(state, i)?.commits[index];
    println!(
        "{name}: left out of the copy of {}: {}; a copy never changes .git-intake.toml or .git-intake/",
        abbrev(&entry.commit),
        paths.join(", ")
    );
    for p in paths {
        if !entry.left_out.contains(&p) {
            entry.left_out.push(p);
        }
    }
    repo.save_state(state)
}

/// Commits the copy staged in the worktree, without the paths a copy never
/// changes, and records its outcome. A copy that changes nothing is not
/// committed. A commit that fails stops the update: the staged copy stays,
/// and `continue` commits it once the cause is fixed. Returns the exit
/// status then.
fn finish_copy(
    repo: &Repo,
    state: &mut UpdateState,
    i: usize,
    index: usize,
) -> Result<Option<Exit>> {
    leave_out_own_paths(repo, state, i, index)?;
    let wt = Git::worktree(&state.worktree);
    let name = state.upstreams[i].name.clone();
    let entry = plan_of(state, i)?.commits[index].clone();
    if wt.test(&["diff", "--cached", "--quiet"])? {
        let unstaged = wt.tracked_changes()?;
        if !unstaged.is_empty() {
            // Only what is staged is committed: a copy left in the worktree
            // would be recorded as present though nothing was taken.
            let why = format!(
                "nothing of the copy is staged, but tracked files have unstaged changes:\n{}",
                bullet(&unstaged)
            );
            stop_on_failed_commit(repo, state, i, index, &why)?;
            println!(
                "stage what belongs to the copy with `git add`, discard the rest, then run `git intake continue`"
            );
            return Ok(Some(Exit::Negative));
        }
        // Record a resolution made for this copy, so that a retry
        // reapplies it.
        let _ = wt.raw(&["rerere"]);
        state.upstreams[i].step = UpstreamStep::Pending;
        record_copy(repo, state, i, index, None)?;
        return Ok(None);
    }
    let [upstream, taken] = sync_record::copy_trailers(&name, &entry.commit);
    let out = wt.raw(&[
        "commit",
        "--quiet",
        "-C",
        &entry.commit,
        "--trailer",
        &upstream,
        "--trailer",
        &taken,
    ])?;
    if !out.success() {
        let why = format!("{}{}", out.stdout, out.stderr).trim().to_string();
        stop_on_failed_commit(repo, state, i, index, &why)?;
        println!(
            "the copy stays staged; fix the cause there, stage the result with `git add`, then run \
             `git intake continue`, which commits it as the copy with its author, message and trailers"
        );
        return Ok(Some(Exit::Negative));
    }
    let head = wt.line(&["rev-parse", "HEAD"])?;
    if sync_record::copy_of(&wt, &head, &name)?.as_deref() != Some(&entry.commit) {
        bail!(
            "the copy of {} at {} in {} does not end with the trailers of upstream '{name}'; a hook or configuration \
             may have changed its message. Correct the commit so that its message ends with\n\n{upstream}\n{taken}\n\n\
             and run `git intake continue`, or run `git intake abort`",
            abbrev(&entry.commit),
            abbrev(&head),
            wt.dir().display()
        );
    }
    state.upstreams[i].step = UpstreamStep::Pending;
    record_copy(repo, state, i, index, Some(head))?;
    Ok(None)
}

/// Records that the copy could not be committed, and why, and reports it.
fn stop_on_failed_commit(
    repo: &Repo,
    state: &mut UpdateState,
    i: usize,
    index: usize,
    why: &str,
) -> Result<()> {
    let name = state.upstreams[i].name.clone();
    let entry = &mut plan_mut(state, i)?.commits[index];
    entry.copy = CopyStep::CommitFailed;
    entry.failure = Some(why.to_string());
    let (commit, subject) = (entry.commit.clone(), entry.subject.clone());
    state.upstreams[i].step = UpstreamStep::Conflict;
    repo.save_state(state)?;
    println!(
        "{name}: the copy of {} {subject} could not be committed in {}:\n{why}",
        abbrev(&commit),
        state.worktree.display()
    );
    Ok(())
}

/// Records the outcome of one copy.
fn record_copy(
    repo: &Repo,
    state: &mut UpdateState,
    i: usize,
    index: usize,
    copied: Option<String>,
) -> Result<()> {
    let name = state.upstreams[i].name.clone();
    let entry = &mut plan_mut(state, i)?.commits[index];
    match &copied {
        Some(c) => println!("{name}: copied {} as {}", abbrev(&entry.commit), abbrev(c)),
        None => println!(
            "{name}: the copy of {} changes nothing; it is recorded as present",
            abbrev(&entry.commit)
        ),
    }
    entry.copy = if copied.is_some() {
        CopyStep::Copied
    } else {
        CopyStep::Empty
    };
    entry.copied = copied;
    entry.failure = None;
    settle(&mut entry.conflicts);
    state.verification = None;
    repo.save_state(state)
}

/// Records the conflicts of the copy stopped in the worktree and reports them.
fn stop_on_conflict(
    repo: &Repo,
    state: &mut UpdateState,
    i: usize,
    index: usize,
    output: &str,
) -> Result<Exit> {
    let wt = Git::worktree(&state.worktree);
    let conflicts = label_conflicts(&wt.unmerged_files()?, &rerere_resolved(output));
    let entry = &mut plan_mut(state, i)?.commits[index];
    entry.conflicts = conflicts;
    entry.copy = CopyStep::Conflict;
    state.upstreams[i].step = UpstreamStep::Conflict;
    repo.save_state(state)?;

    let plan = plan_of(state, i)?;
    let entry = &plan.commits[index];
    println!(
        "{}: conflicts copying {} {} in {}",
        state.upstreams[i].name,
        abbrev(&entry.commit),
        entry.subject,
        state.worktree.display()
    );
    print_conflicted_files(&entry.conflicts);
    let listed = |step: &dyn Fn(CopyStep) -> bool| -> Vec<&str> {
        plan.takes
            .iter()
            .map(|t| &plan.commits[*t])
            .filter(|c| step(c.copy))
            .map(|c| abbrev(&c.commit))
            .collect()
    };
    println!(
        "copied: {}",
        list_or_none(&listed(&|s| matches!(
            s,
            CopyStep::Copied | CopyStep::Empty
        )))
    );
    println!(
        "still to copy: {}",
        list_or_none(&listed(&|s| s == CopyStep::None))
    );
    println!(
        "resolve the conflicts in {}, stage them with `git add`, then run `git intake continue`",
        state.worktree.display()
    );
    Ok(Exit::Negative)
}

/// For each copied commit, the earlier planned commits left pending that
/// changed some of the same files: a likely dependency the copy lacks.
fn find_overlaps(repo: &Repo, state: &mut UpdateState, i: usize) -> Result<()> {
    let plan = plan_of(state, i)?;
    let later: Vec<usize> = (0..plan.commits.len())
        .filter(|j| plan.commits[*j].verb == Some(Verb::Later))
        .collect();
    let copied: Vec<usize> = (0..plan.commits.len())
        .filter(|j| plan.commits[*j].copy == CopyStep::Copied)
        .collect();
    let mut files: HashMap<usize, Vec<String>> = HashMap::new();
    let mut changed = |j: usize| -> Result<Vec<String>> {
        if let Some(f) = files.get(&j) {
            return Ok(f.clone());
        }
        let c = &plan.commits[j].commit;
        let out = repo.git.run(&[
            "diff-tree",
            "-r",
            "--name-only",
            "-z",
            "--no-renames",
            &format!("{c}^1"),
            c,
            "--",
        ])?;
        let f = split_z(&out);
        files.insert(j, f.clone());
        Ok(f)
    };
    let mut overlaps: Vec<(usize, Vec<String>)> = Vec::new();
    for &t in &copied {
        let mut found = Vec::new();
        let taken_files = changed(t)?;
        for &l in later.iter().filter(|l| **l < t) {
            if changed(l)?.iter().any(|f| taken_files.contains(f)) {
                found.push(plan.commits[l].commit.clone());
            }
        }
        overlaps.push((t, found));
    }
    let plan = plan_mut(state, i)?;
    for (t, found) in overlaps {
        plan.commits[t].overlaps = found;
    }
    Ok(())
}

/// Commits the plan's `skip` and `have` decisions, and the copies that
/// changed nothing as `have`, to the journal in one commit, creating the
/// journal when it does not exist.
fn commit_journal(repo: &Repo, state: &mut UpdateState, i: usize) -> Result<()> {
    let wt = Git::worktree(&state.worktree);
    let name = state.upstreams[i].name.clone();
    let plan = plan_of(state, i)?.clone();
    let mut decisions = Vec::new();
    for c in &plan.commits {
        let (verdict, note) = match (c.verb, c.copy) {
            (_, CopyStep::Empty) if c.left_out.is_empty() => {
                (Verdict::Have, Some(EMPTY_COPY_NOTE.to_string()))
            }
            (_, CopyStep::Empty) => (Verdict::Have, Some(OWN_FILES_COPY_NOTE.to_string())),
            (Some(Verb::Skip), _) => (Verdict::Skip, c.note.clone()),
            (Some(Verb::Have), _) => (Verdict::Have, c.note.clone()),
            _ => continue,
        };
        decisions.push(Decision {
            commit: c.commit.clone(),
            verdict,
            note,
        });
    }
    let path = port::journal_path(&name);
    let existing = port::read_journal(&wt, "HEAD", &name)?;
    if decisions.is_empty() && existing.is_some() {
        return Ok(());
    }
    let created = existing.is_none();
    let mut journal = existing.unwrap_or(Journal {
        base: plan.base.clone(),
        retired: Vec::new(),
        decisions: Vec::new(),
    });
    let mut added = Vec::new();
    for d in decisions {
        if !journal.decisions.iter().any(|seen| seen.commit == d.commit) {
            added.push(d.clone());
            journal.decisions.push(d);
        }
    }
    let text = port::render_journal(&journal);
    let head = wt.line(&["rev-parse", "HEAD"])?;
    if wt.show_file("HEAD", &path)?.as_deref() == Some(text.as_str()) {
        // Committed before an interruption.
        if plan
            .journal_head_before
            .as_ref()
            .is_some_and(|before| *before != head)
        {
            plan_mut(state, i)?.journal_commit = Some(head);
        }
        return Ok(());
    }
    plan_mut(state, i)?.journal_head_before = Some(head);
    repo.save_state(state)?;

    let file = state.worktree.join(&path);
    if let Some(dir) = file.parent() {
        fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
    }
    fs::write(&file, &text).with_context(|| format!("cannot write {}", file.display()))?;
    wt.run(&["add", "--", &path])?;
    let message = journal_message(&wt, &name, &journal, created, &added)?;
    wt.run(&[
        "commit",
        "--quiet",
        "--no-edit",
        "-m",
        &message,
        "--",
        &path,
    ])?;
    let commit = wt.line(&["rev-parse", "HEAD"])?;
    println!(
        "{name}: recorded {} decision{} in {path} as {}",
        added.len(),
        plural(added.len() as u64),
        abbrev(&commit)
    );
    plan_mut(state, i)?.journal_commit = Some(commit);
    repo.save_state(state)
}

fn journal_message(
    git: &Git,
    name: &str,
    journal: &Journal,
    created: bool,
    added: &[Decision],
) -> Result<String> {
    let subject = if added.is_empty() {
        format!(
            "sync({name}): start the port journal at {}",
            git.short(&journal.base)?
        )
    } else {
        format!(
            "sync({name}): record {} port decision{}",
            added.len(),
            plural(added.len() as u64)
        )
    };
    let mut body = Vec::new();
    if created && !added.is_empty() {
        body.push(format!("The port starts at {}.", journal.base));
        body.push(String::new());
    }
    for d in added {
        let subject = git
            .line(&["log", "-1", "--format=%s", &d.commit, "--"])
            .unwrap_or_default();
        let mut line = format!("{} {} {subject}", d.verdict.as_str(), abbrev(&d.commit));
        if let Some(note) = &d.note {
            line.push_str(&format!(" # {note}"));
        }
        body.push(line);
    }
    Ok(if body.is_empty() {
        format!("{subject}\n")
    } else {
        format!("{subject}\n\n{}\n", body.join("\n"))
    })
}

fn plan_of(state: &UpdateState, i: usize) -> Result<&PortPlan> {
    state.upstreams[i].port().with_context(|| {
        format!(
            "upstream '{}' is not taken in port mode",
            state.upstreams[i].name
        )
    })
}

fn plan_mut(state: &mut UpdateState, i: usize) -> Result<&mut PortPlan> {
    let name = state.upstreams[i].name.clone();
    match &mut state.upstreams[i].work {
        Work::Port(p) => Ok(p),
        Work::Follow(_) => bail!("upstream '{name}' is not taken in port mode"),
    }
}
