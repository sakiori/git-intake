//! `start`, `continue` and `abort`: taking the upstreams in on the update
//! branch. A follow upstream is merged up to its target here; a port
//! upstream's plan is carried out by [`port_plan`](super::port_plan).

use std::collections::HashMap;
use std::fs;

use anyhow::{Context, Result, bail};

use crate::Exit;
use crate::cli::UpstreamTarget;
use crate::commands::merge_request::{self, Landing};
use crate::commands::{catch_up, finish, port_plan};
use crate::config::{Mode, Upstream};
use crate::drift::{self, DriftState};
use crate::git::{Git, abbrev, branch_ref};
use crate::port::{self, PlanLine};
use crate::repo::{Repo, bullet};
use crate::state::{
    self, CatchUpStep, ConflictFile, CopyStep, FollowMerge, MergeKind, PlannedCommit, PortPlan,
    QueuedUpstream, Resolution, UPDATE_BRANCH, UpdateState, UpstreamStep, Work,
};
use crate::sync_record::{self, Record};
use crate::tags::{self, TagPattern, UpstreamTags};
use crate::target;
use crate::upstreams::{self, Fetches};

/// `start`: fetches the selected upstreams and queues each one with work to
/// do. A follow upstream gets a fixed target, the fetched tip or the commit
/// its revision names, and is merged; a port upstream gets a plan of its
/// pending commits, and the update stops for the plan to be edited.
pub fn start(repo: &Repo, targets: &[UpstreamTarget]) -> Result<Exit> {
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
    let worktree = repo.place_update_worktree()?;
    if repo.update_worktree_registered()? || worktree.exists() || repo.update_branch_exists()? {
        bail!(
            "an interrupted update left its worktree or the branch '{UPDATE_BRANCH}' behind; run `git intake abort` to remove them"
        );
    }
    let names: Vec<String> = targets.iter().map(|t| t.name.clone()).collect();
    let mut repeated: Vec<&str> = names
        .iter()
        .enumerate()
        .filter(|(i, n)| names[..*i].contains(n))
        .map(|(_, n)| n.as_str())
        .collect();
    repeated.sort_unstable();
    repeated.dedup();
    if !repeated.is_empty() {
        bail!(
            "each upstream may be named once; named more than once: {}",
            repeated.join(", ")
        );
    }
    let mut config = repo.load_config()?.config;
    config.select(&names)?;
    repo.require_clean(&config.integration)?;
    let git = &repo.git;
    if let Some(publish) = config.publish.clone()
        && finish::follow_published(repo, &publish.remote, &config.integration)?
    {
        // The configuration at the new tip decides the update. Reading it
        // refuses one that names another integration branch; it must also
        // name the remote the branch was brought up to date with.
        let reread = repo.load_config()?.config;
        let remote = reread.publish.as_ref().map(|p| p.remote.clone());
        if remote.as_deref() != Some(publish.remote.as_str()) {
            bail!(
                "the {} at the new tip of '{}' names {} as the publish remote instead of '{}'; the fast-forward is kept, \
                 and nothing else changed. Run `git intake start` again to start the update with that configuration",
                crate::config::FILE_NAME,
                config.integration,
                remote.map_or("none".to_string(), |r| format!("'{r}'")),
                publish.remote
            );
        }
        config = reread;
    }
    let selected = config.select(&names)?;

    let mut fetches = Fetches::new(repo);
    let mut tips = HashMap::new();
    let mut fetch_failures = Vec::new();
    for up in &selected {
        match fetches.upstream(up)? {
            Ok(tip) => {
                tips.insert(up.name.clone(), tip);
            }
            Err(why) => fetch_failures.push(format!("upstream '{}': {why}", up.name)),
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

    let fetched_tips: Vec<String> = tips.values().cloned().collect();
    let latest = sync_record::latest(git, &sync_record::records(git, &base, &fetched_tips)?)?;
    let mut refusals = Vec::new();
    let mut queue = Vec::new();
    let mut current = Vec::new();
    for up in &selected {
        let tip = &tips[&up.name];
        let rev = targets
            .iter()
            .find(|t| t.name == up.name)
            .and_then(|t| t.rev.clone());
        // A follow target is named by the upstream's tags, and a revision
        // may be one of them.
        let tags = if up.mode == Mode::Follow || rev.is_some() {
            match UpstreamTags::list(git, up)? {
                Ok(tags) => tags,
                Err(why) => {
                    refusals.push(why);
                    continue;
                }
            }
        } else {
            UpstreamTags::default()
        };
        let fetched = Fetched { tip, tags: &tags };
        let queued = match up.mode {
            Mode::Follow => queue_follow(repo, &base, up, latest.get(&up.name), fetched, rev)?,
            Mode::Port => queue_port(
                repo,
                &base,
                up,
                latest.get(&up.name),
                fetched,
                &fetched_tips,
                rev,
            )?,
        };
        match queued {
            Queued::Work(q) => queue.push(*q),
            Queued::Current(message) => current.push(message),
            Queued::Refused(why) => refusals.push(why),
        }
    }
    if !refusals.is_empty() {
        bail!("cannot start the update:\n{}", bullet(&refusals));
    }
    for message in &current {
        println!("{message}");
    }
    if queue.is_empty() {
        println!("every selected upstream is current; nothing to update");
        return Ok(Exit::Success);
    }

    repo.exclude_update_worktree()?;
    git.run(&["config", "rerere.enabled", "true"])?;
    fs::create_dir_all(&repo.paths.root)
        .with_context(|| format!("cannot create {}", repo.paths.root.display()))?;
    git.run(&[
        "worktree",
        "add",
        "--quiet",
        "-b",
        UPDATE_BRANCH,
        &worktree.to_string_lossy(),
        &base,
    ])?;
    let mut state = UpdateState::new(config, base.clone(), worktree, queue);
    state.main_worktree = repo.main_checkout()?;
    for q in &state.upstreams {
        if let Some(plan) = q.port() {
            let listed: Vec<(String, String)> = plan
                .commits
                .iter()
                .map(|c| (c.commit.clone(), c.subject.clone()))
                .collect();
            fs::write(&plan.plan, port::render_plan(&q.name, &listed))
                .with_context(|| format!("cannot write {}", plan.plan.display()))?;
        }
    }
    repo.save_state(&state)?;
    // The update holds its references from now on.
    upstreams::hold(git, &state)?;
    drop(fetches);
    println!(
        "update started from {} at {} in {}",
        state.config.integration,
        abbrev(&base),
        state.worktree.display()
    );
    for q in &state.upstreams {
        match &q.work {
            Work::Follow(f) => println!(
                "{}: queued at {} ({} new commit{})",
                q.name,
                described(&f.target, f.named_by(q)),
                f.commits,
                plural(f.commits)
            ),
            Work::Port(p) => {
                let n = p.commits.len() as u64;
                let limit = match &q.rev {
                    Some(rev) => format!(
                        " up to {rev}; {} later pending commit{} left out",
                        p.left_out,
                        plural(p.left_out)
                    ),
                    None => String::new(),
                };
                println!("{}: {n} pending commit{} planned{limit}", q.name, plural(n));
            }
        }
    }
    advance(repo, &mut state)
}

/// What `start` fetched of one selected upstream: its branch tip, and the
/// tags of its repository when it needs them.
#[derive(Clone, Copy)]
struct Fetched<'a> {
    tip: &'a str,
    tags: &'a UpstreamTags,
}

/// What `start` makes of one selected upstream.
enum Queued {
    Work(Box<QueuedUpstream>),
    /// Nothing to do; the message says so.
    Current(String),
    Refused(String),
}

/// Fixes a follow upstream's target and decides how it is merged.
///
/// The target is the commit `rev` names, the newest release tag on the
/// fetched branch when the upstream has a `tags` pattern, or the fetched
/// tip. The upstream tag that names it goes into the sync record.
fn queue_follow(
    repo: &Repo,
    base: &str,
    up: &Upstream,
    last: Option<&Record>,
    Fetched { tip, tags }: Fetched,
    rev: Option<String>,
) -> Result<Queued> {
    let git = &repo.git;
    let d = drift::evaluate(git, base, up, last, Ok(tip))?;
    if d.state == DriftState::Rewritten {
        return Ok(Queued::Refused(format!(
            "upstream '{}' was rewritten: {}",
            up.name,
            d.detail.unwrap_or_default()
        )));
    }
    let pattern = match up.tags.as_deref().map(TagPattern::parse).transpose() {
        Ok(pattern) => pattern,
        Err(why) => bail!("upstream '{}' tags: {why}", up.name),
    };
    let (target, tag) = match (&rev, &pattern) {
        (Some(rev), _) => match target::resolve(git, &repo.here, up, tags, rev, tip)? {
            Ok(resolved) => {
                let tag = resolved
                    .tag
                    .filter(|t| tags::recordable(t))
                    .or_else(|| tags.naming(pattern.as_ref(), &resolved.commit));
                (resolved.commit, tag)
            }
            Err(why) => return Ok(Queued::Refused(why)),
        },
        (None, Some(pattern)) => match tags.release(git, pattern, tip)? {
            Some(release) => {
                // The newest release is already taken in, even when the
                // branch went past it by a revision named before.
                if matches!(d.state, DriftState::Current | DriftState::Behind)
                    && git.is_ancestor(&release.commit, base)?
                {
                    return Ok(Queued::Current(format!(
                        "{}: current at {}",
                        up.name,
                        described(&release.commit, Some(&release.tag))
                    )));
                }
                (release.commit, Some(release.tag))
            }
            None => return Ok(Queued::Refused(no_release(up))),
        },
        (None, None) => (tip.to_string(), tags.naming(None, tip)),
    };
    let named = tag.as_deref().or(rev.as_deref());
    if let Some(last) = &d.last_synced
        && *last != target
        && git.is_ancestor(&target, last)?
    {
        return Ok(Queued::Refused(format!(
            "upstream '{}': the target {} is older than the last synced commit {}; \
             an update never moves an upstream back",
            up.name,
            described(&target, named),
            abbrev(last)
        )));
    }
    if matches!(d.state, DriftState::Current | DriftState::Behind)
        && git.is_ancestor(&target, base)?
    {
        return Ok(Queued::Current(format!(
            "{}: current at {}",
            up.name,
            described(&target, named)
        )));
    }
    if let Some(last) = &d.last_synced
        && !git.is_ancestor(last, &target)?
    {
        return Ok(Queued::Refused(format!(
            "upstream '{}': the target {} does not contain the last synced commit {}; \
             name a commit of {} that contains it",
            up.name,
            described(&target, named),
            abbrev(last),
            up.source()
        )));
    }
    let has_record = d.last_synced_source == Some(drift::SyncSource::Record);
    Ok(match merge_kind(git, base, up, &target, has_record)? {
        Ok(kind) => Queued::Work(Box::new(QueuedUpstream {
            name: up.name.clone(),
            fetched: tip.to_string(),
            rev,
            step: UpstreamStep::Pending,
            work: Work::Follow(FollowMerge {
                commits: git.count(base, &target)?,
                target,
                tag,
                previous: d.last_synced.clone(),
                kind,
                head_before: None,
                merge_commit: None,
                conflicts: Vec::new(),
            }),
        })),
        Err(why) => Queued::Refused(why),
    })
}

/// Why `start` cannot take a follow upstream at a release.
pub fn no_release(up: &Upstream) -> String {
    format!(
        "upstream '{}': no tag of {} matching '{}' is a release version on {} as fetched; \
         name the target with `git intake start {}=<rev>`",
        up.name,
        up.url,
        up.tags.as_deref().unwrap_or_default(),
        up.source(),
        up.name
    )
}

/// Plans a port upstream: its pending commits, limited to those the named
/// revision contains.
fn queue_port(
    repo: &Repo,
    base: &str,
    up: &Upstream,
    last: Option<&Record>,
    Fetched { tip, tags }: Fetched,
    upstream_tips: &[String],
    rev: Option<String>,
) -> Result<Queued> {
    let git = &repo.git;
    let accounting = match port::standing(git, base, up, last, tip, upstream_tips)
        .and_then(|standing| port::require_plannable(&up.name, standing))
    {
        Ok(a) => a,
        Err(err) => return Ok(Queued::Refused(format!("{err:#}"))),
    };
    let pending = accounting.pending();
    let limit = match &rev {
        None => None,
        Some(rev) => match target::resolve(git, &repo.here, up, tags, rev, tip)? {
            Ok(resolved) => Some(resolved.commit),
            Err(why) => return Ok(Queued::Refused(why)),
        },
    };
    let mut planned = Vec::new();
    for c in &pending {
        if let Some(limit) = &limit
            && !git.is_ancestor(&c.commit, limit)?
        {
            continue;
        }
        planned.push(PlannedCommit {
            commit: c.commit.clone(),
            subject: c.subject.clone(),
            merge: c.merge,
            verb: None,
            note: None,
            copy: CopyStep::None,
            head_before: None,
            copied: None,
            conflicts: Vec::new(),
            failure: None,
            left_out: Vec::new(),
            overlaps: Vec::new(),
        });
    }
    if planned.is_empty() {
        return Ok(Queued::Current(match (&rev, pending.len()) {
            (Some(rev), n) if n > 0 => format!(
                "{}: current up to {rev}; {n} later commit{} pending",
                up.name,
                plural(n as u64)
            ),
            _ => format!(
                "{}: current; every commit after the base {} is taken or decided",
                up.name,
                abbrev(&accounting.base)
            ),
        }));
    }
    Ok(Queued::Work(Box::new(QueuedUpstream {
        name: up.name.clone(),
        fetched: tip.to_string(),
        rev,
        step: UpstreamStep::Plan,
        work: Work::Port(PortPlan {
            plan: repo.paths.plan.clone(),
            base: accounting.base.clone(),
            journal_exists: accounting.journal,
            left_out: (pending.len() - planned.len()) as u64,
            commits: planned,
            takes: Vec::new(),
            journal_commit: None,
            journal_head_before: None,
        }),
    })))
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

/// Takes in every pending upstream in order, stopping at the first conflict
/// or at a plan to edit.
pub fn advance(repo: &Repo, state: &mut UpdateState) -> Result<Exit> {
    for i in 0..state.upstreams.len() {
        let stopped = match (state.upstreams[i].step, &state.upstreams[i].work) {
            (UpstreamStep::Merged, _) => None,
            (UpstreamStep::Plan, Work::Port(plan)) => {
                println!(
                    "{}: the plan is in {}",
                    state.upstreams[i].name,
                    plan.plan.display()
                );
                println!(
                    "mark each commit take, skip, have or later, then run `git intake continue`"
                );
                return Ok(Exit::Success);
            }
            (_, Work::Follow(_)) => merge_follow(repo, state, i)?,
            (_, Work::Port(_)) => port_plan::carry_out(repo, state, i)?,
        };
        if let Some(exit) = stopped {
            return Ok(exit);
        }
    }
    println!("every queued upstream is merged or its plan carried out; next: git intake verify");
    Ok(Exit::Success)
}

/// Merges one pending follow upstream. Returns the exit status when the
/// merge stopped on conflicts.
fn merge_follow(repo: &Repo, state: &mut UpdateState, i: usize) -> Result<Option<Exit>> {
    let wt = Git::worktree(&state.worktree);
    let up = upstream_of(state, i)?;
    let (target, tag, kind) = {
        let f = follow_of(state, i)?;
        (f.target.clone(), f.tag.clone(), f.kind)
    };
    let tag = tag.as_deref();

    // A merge that completed before the state could be saved.
    if carries_record(&wt, &up, &target, tag)? {
        let commit = confirm_merged(&wt, &up, &target, tag)?;
        mark_merged(repo, state, i, commit)?;
        return Ok(None);
    }
    if wt.merge_in_progress()? {
        let ours = wt.commit("MERGE_HEAD")?.as_deref() == Some(target.as_str());
        if ours && kind != MergeKind::Import {
            // A merge of this upstream that stopped before the state was
            // saved: its output, saved right after the merge, still says
            // which files rerere resolved. Without it, nothing is
            // labelled as resolved by a recorded resolution.
            follow_mut(state, i)?.head_before = Some(wt.line(&["rev-parse", "HEAD"])?);
            let output = fs::read_to_string(merge_output_path(repo, &up.name)).unwrap_or_default();
            return stop_on_conflict(repo, state, i, &output).map(Some);
        }
        wt.run(&["merge", "--abort"])?;
    }

    let msg = merge_message(&repo.git, &up, &target, tag)?;
    let head_before = wt.line(&["rev-parse", "HEAD"])?;
    follow_mut(state, i)?.head_before = Some(head_before);
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
            let commit = confirm_merged(&wt, &up, &target, tag)?;
            mark_merged(repo, state, i, commit)?;
            Ok(None)
        }
        Some(out) => {
            if !wt.merge_in_progress()? {
                return Err(anyhow::Error::new(out.into_error()))
                    .with_context(|| format!("merging upstream '{}' failed", up.name));
            }
            let output = format!("{}{}", out.stdout, out.stderr);
            stop_on_conflict(repo, state, i, &output).map(Some)
        }
    }
}

fn follow_of(state: &UpdateState, i: usize) -> Result<&FollowMerge> {
    state.upstreams[i].follow().with_context(|| {
        format!(
            "upstream '{}' is not merged in follow mode",
            state.upstreams[i].name
        )
    })
}

fn follow_mut(state: &mut UpdateState, i: usize) -> Result<&mut FollowMerge> {
    let name = state.upstreams[i].name.clone();
    match &mut state.upstreams[i].work {
        Work::Follow(f) => Ok(f),
        Work::Port(_) => bail!("upstream '{name}' is not merged in follow mode"),
    }
}

/// Where a merge's output is kept until the update's state records it.
pub fn merge_output_path(repo: &Repo, name: &str) -> std::path::PathBuf {
    repo.paths.logs.join(format!("merge-{name}.out"))
}

/// Saves what a merge or copy printed before anything else can fail, so
/// that a recovery after an interruption still sees rerere's report.
pub fn save_merge_output(path: &std::path::Path, out: &crate::git::Output) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
    }
    fs::write(path, format!("{}{}", out.stdout, out.stderr))
        .with_context(|| format!("cannot write {}", path.display()))
}

fn mark_merged(repo: &Repo, state: &mut UpdateState, i: usize, commit: String) -> Result<()> {
    println!("{}: merged as {}", state.upstreams[i].name, abbrev(&commit));
    let f = follow_mut(state, i)?;
    settle(&mut f.conflicts);
    f.merge_commit = Some(commit);
    state.upstreams[i].step = UpstreamStep::Merged;
    state.verification = None;
    repo.save_state(state)
}

/// Conflicted files still unresolved when the merge or copy is concluded
/// were resolved by hand.
pub fn settle(conflicts: &mut [ConflictFile]) {
    for c in conflicts {
        if c.resolution == Resolution::Unresolved {
            c.resolution = Resolution::Manual;
        }
    }
}

/// Records the conflicts of the merge stopped in the worktree and reports them.
/// `output` is what the merge printed; rerere reports there the files it
/// resolved from a recorded resolution.
fn stop_on_conflict(repo: &Repo, state: &mut UpdateState, i: usize, output: &str) -> Result<Exit> {
    let wt = Git::worktree(&state.worktree);
    let conflicts = label_conflicts(&wt.unmerged_files()?, &rerere_resolved(output));
    follow_mut(state, i)?.conflicts = conflicts;
    state.upstreams[i].step = UpstreamStep::Conflict;
    repo.save_state(state)?;
    print_conflict(state, i, output);
    Ok(Exit::Negative)
}

/// Files git's rerere reports as resolved from a recorded resolution, either
/// left in the worktree (`Resolved '<path>' ...`) or already staged under
/// `rerere.autoupdate` (`Staged '<path>' ...`). Git runs with `LC_ALL=C`, so
/// these messages are stable.
pub fn rerere_resolved(output: &str) -> Vec<String> {
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
pub fn label_conflicts(unmerged: &[String], recorded: &[String]) -> Vec<ConflictFile> {
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

/// Whether HEAD is a merge carrying this upstream's sync record for
/// `target`, naming its upstream tag `tag`.
fn carries_record(wt: &Git, up: &Upstream, target: &str, tag: Option<&str>) -> Result<bool> {
    Ok(sync_record::record_of(wt, "HEAD")?
        .is_some_and(|r| r.upstream == up.name && r.target == target && r.tag.as_deref() == tag))
}

/// Checks that HEAD is a complete merge of the upstream before it is counted
/// as merged: the sync record names the upstream and target, the target is
/// merged, and a path upstream's path exists. Returns HEAD.
fn confirm_merged(wt: &Git, up: &Upstream, target: &str, tag: Option<&str>) -> Result<String> {
    let head = wt.line(&["rev-parse", "HEAD"])?;
    let mut problems = Vec::new();
    if !carries_record(wt, up, target, tag)? {
        problems.push(format!(
            "the commit message does not end with the sync record trailers for upstream '{}' and target {}",
            up.name,
            described(target, tag)
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
            sync_record::message(&up.name, &up.source(), abbrev(target), target, tag)
        );
    }
    Ok(head)
}

pub fn upstream_of(state: &UpdateState, i: usize) -> Result<Upstream> {
    let name = &state.upstreams[i].name;
    state
        .config
        .upstream(name)
        .cloned()
        .with_context(|| format!("the update state names unknown upstream '{name}'"))
}

fn merge_message(git: &Git, up: &Upstream, target: &str, tag: Option<&str>) -> Result<String> {
    let short = git.short(target)?;
    Ok(sync_record::message(
        &up.name,
        &up.source(),
        &short,
        target,
        tag,
    ))
}

fn print_conflict(state: &UpdateState, i: usize, stderr: &str) {
    let q = &state.upstreams[i];
    let up = state.config.upstream(&q.name);
    let target = q.follow().map(|f| f.target.as_str()).unwrap_or_default();
    println!(
        "{}: conflicts merging {} at {} in {}",
        q.name,
        up.map(Upstream::source).unwrap_or_default(),
        abbrev(target),
        state.worktree.display()
    );
    if q.conflicts().is_empty() {
        println!(
            "the merge stopped without conflicted files:\n{}",
            stderr.trim()
        );
    }
    print_conflicted_files(q.conflicts());
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

/// Lists conflicted files with the source of their resolution.
pub fn print_conflicted_files(conflicts: &[ConflictFile]) {
    for c in conflicts {
        println!("  {}  ({})", c.path, c.resolution.describe());
    }
    if conflicts
        .iter()
        .any(|c| c.resolution == Resolution::Recorded)
    {
        println!(
            "files resolved by a recorded resolution already hold the earlier resolution; review them and stage them"
        );
    }
}

/// `continue`: carries out a plan, concludes the merge or copy stopped on
/// conflicts, and takes in the rest.
pub fn resume(repo: &Repo) -> Result<Exit> {
    let mut state = repo.require_state()?;
    require_merging(repo, &state)?;
    require_worktree(repo, &state)?;
    let wt = Git::worktree(&state.worktree);
    if state.catching_up().is_some() {
        if let Some(exit) = catch_up::conclude(repo, &mut state, true)? {
            return Ok(exit);
        }
        println!("next: git intake finish");
        return Ok(Exit::Success);
    }
    if let Some(i) = state
        .upstreams
        .iter()
        .position(|u| u.step == UpstreamStep::Conflict)
    {
        match &state.upstreams[i].work {
            Work::Follow(_) => conclude_merge(repo, &mut state, i)?,
            Work::Port(_) => {
                if let Some(exit) = port_plan::conclude_copy(repo, &mut state, i)? {
                    return Ok(exit);
                }
            }
        }
    } else if let Some(i) = state
        .upstreams
        .iter()
        .position(|u| u.step == UpstreamStep::Plan)
    {
        read_plan(repo, &mut state, i)?;
    } else if state.all_merged() {
        println!(
            "every queued upstream is already merged or its plan carried out; next: {}",
            next_step(&state, &wt)?
        );
        return Ok(Exit::Success);
    }
    advance(repo, &mut state)
}

/// Concludes the merge of a follow upstream stopped on conflicts.
fn conclude_merge(repo: &Repo, state: &mut UpdateState, i: usize) -> Result<()> {
    let wt = Git::worktree(&state.worktree);
    let up = upstream_of(state, i)?;
    let (target, tag, head_before) = {
        let f = follow_of(state, i)?;
        (f.target.clone(), f.tag.clone(), f.head_before.clone())
    };
    let tag = tag.as_deref();
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
        let msg = merge_message(&repo.git, &up, &target, tag)?;
        wt.run(&["commit", "--no-edit", "-m", &msg])?;
    } else {
        let head = wt.line(&["rev-parse", "HEAD"])?;
        let concluded = Some(head.as_str()) != head_before.as_deref()
            && wt.commit("HEAD^2")?.as_deref() == Some(target.as_str());
        if !concluded {
            bail!(
                "the merge of upstream '{}' is no longer in progress in {}; run `git intake abort` and start again",
                up.name,
                state.worktree.display()
            );
        }
    }
    let commit = confirm_merged(&wt, &up, &target, tag)?;
    mark_merged(repo, state, i, commit)
}

/// Reads the edited plan of a port upstream and records what it says. A
/// plan that cannot be carried out is refused, and nothing changes.
fn read_plan(repo: &Repo, state: &mut UpdateState, i: usize) -> Result<()> {
    let name = state.upstreams[i].name.clone();
    let Work::Port(plan) = &mut state.upstreams[i].work else {
        bail!("upstream '{name}' has no plan");
    };
    let text = fs::read_to_string(&plan.plan).with_context(|| {
        format!(
            "cannot read the plan {}; run `git intake abort` and start again",
            plan.plan.display()
        )
    })?;
    let listed: Vec<(String, String)> = plan
        .commits
        .iter()
        .map(|c| (c.commit.clone(), c.subject.clone()))
        .collect();
    let lines = port::parse_plan(&text, &listed).map_err(|why| {
        anyhow::anyhow!(
            "{why}\nnothing was changed; correct the plan {} and run `git intake continue` again",
            plan.plan.display()
        )
    })?;
    for c in &mut plan.commits {
        c.verb = Some(port::Verb::Later);
        c.note = None;
    }
    for PlanLine { index, verb, note } in &lines {
        plan.commits[*index].verb = Some(*verb);
        plan.commits[*index].note = note.clone();
    }
    plan.takes = lines
        .iter()
        .filter(|l| l.verb == port::Verb::Take)
        .map(|l| l.index)
        .collect();
    let count = |verb| plan.commits.iter().filter(|c| c.verb == Some(verb)).count() as u64;
    println!(
        "{name}: the plan takes {}, skips {}, records {} as present and leaves {} pending",
        count(port::Verb::Take),
        count(port::Verb::Skip),
        count(port::Verb::Have),
        count(port::Verb::Later)
    );
    state.upstreams[i].step = UpstreamStep::Pending;
    repo.save_state(state)
}

/// `abort`: removes the update worktree, branch, plan, upstream references,
/// logs and state, and withdraws a merge request the update waits for.
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
            Some(Landing::Unverified {
                tip, landing, head, ..
            }) => unverified = Some((tip, landing, head)),
            Some(_) => {}
        }
        merge_request::delete_branch(&repo.git, mr)?;
    }
    let mut removed = repo.remove_update_artifacts()?;
    removed.extend(repo.remove_plan()?);
    if let Some(state) = &state {
        removed.extend(
            upstreams::release(&repo.git, state)?
                .into_iter()
                .map(|r| format!("reference {r}")),
        );
    }
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
            println!(
                "update aborted; '{}' is unchanged",
                state.config.integration
            );
            if let (Some(mr), Some((tip, landing, head))) = (&merge_request, &unverified) {
                let integration = &state.config.integration;
                let how = match head {
                    Some(head) => format!(
                        "{}, and it landed instead of the verified commit",
                        head.describe()
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

/// Refuses merge-phase commands once the update has left the merge phase,
/// unless it has caught up with the integration branch in its worktree
/// since: `continue` and `verify` then work on the catch-up.
pub fn require_merging(repo: &Repo, state: &UpdateState) -> Result<()> {
    if (state.finishing.is_some() || state.merge_request.is_some())
        && moved_on(state)?
        && repo.update_worktree_registered()?
    {
        return Ok(());
    }
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

/// Whether the update worktree holds work beyond the commit `finish`
/// planned to publish or pushed to the merge request branch, as a catch-up
/// merge made after either, and what was committed on top of it.
pub fn moved_on(state: &UpdateState) -> Result<bool> {
    let published = match (&state.finishing, &state.merge_request) {
        (None, None) => return Ok(false),
        (Some(f), _) => f.moves.first().map(|m| m.new.clone()),
        (None, Some(mr)) => Some(mr.commit.clone()),
    };
    if state.catching_up().is_some() {
        return Ok(true);
    }
    Ok(head(&Git::worktree(&state.worktree))?.is_some_and(|h| Some(h) != published))
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
    if let Some(c) = state.catching_up() {
        let merging = wt.dir().exists() && wt.merge_in_progress()?;
        return Ok(if c.step == CatchUpStep::Conflict || merging {
            "catch_up_conflict"
        } else {
            "catch_up"
        });
    }
    if head(wt)?.is_some_and(|h| catch_up::unverified_head(state, &h)) {
        return Ok("catch_up");
    }
    if state.finishing.is_some() {
        return Ok("finishing");
    }
    if state.merge_request.is_some() {
        return Ok("waiting_for_merge_request");
    }
    if state
        .upstreams
        .iter()
        .filter_map(|u| u.port())
        .any(|p| p.commits.iter().any(|c| c.copy == CopyStep::CommitFailed))
    {
        return Ok("commit_failed");
    }
    if state
        .upstreams
        .iter()
        .any(|u| u.step == UpstreamStep::Conflict)
    {
        return Ok("conflict");
    }
    if state.upstreams.iter().any(|u| u.step == UpstreamStep::Plan) {
        return Ok("plan");
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
        "catch_up_conflict" => format!(
            "resolve the conflicts of the catch-up merge in {}, stage them, then run `git intake continue`",
            state.worktree.display()
        ),
        "catch_up" if state.catching_up().is_some() => "git intake continue".to_string(),
        "catch_up" if state.verification.as_ref().is_some_and(|v| !v.passed) => format!(
            "`git intake finish` to verify the catch-up merge again and publish it, or fix the failures on the \
             update branch in {}, commit, then run `git intake verify`",
            state.worktree.display()
        ),
        "catch_up" => "git intake finish".to_string(),
        "conflict" => format!(
            "resolve the conflicts in {}, stage them, then run `git intake continue`",
            state.worktree.display()
        ),
        "commit_failed" => format!(
            "fix what made the commit of the copy fail in {}, stage the result, then run `git intake continue`",
            state.worktree.display()
        ),
        "plan" => match state.upstreams.iter().find_map(|u| u.port()) {
            Some(plan) => format!(
                "mark each commit of the plan {} take, skip, have or later, then run `git intake continue`",
                plan.plan.display()
            ),
            None => "git intake continue".to_string(),
        },
        "merging" => "git intake continue".to_string(),
        "verified" => "git intake finish".to_string(),
        "waiting_for_merge_request" => match &state.merge_request {
            Some(mr) => merge_request::waiting_next(mr, &state.config.integration),
            None => "git intake finish".to_string(),
        },
        "finishing" => match state.finishing.as_ref().and_then(|f| f.push_error.as_ref()) {
            Some(_) => "`git intake finish` to retry the push, catching up first when the publish remote moved, \
                        or `git intake finish --no-publish` to close the update locally"
                .to_string(),
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

/// A target as shown to the user: its abbreviated commit id, followed by the
/// revision it was named by, if any.
pub fn described(target: &str, rev: Option<&str>) -> String {
    match rev {
        Some(rev) if !names_by_id(rev, target) => format!("{} ({rev})", abbrev(target)),
        _ => abbrev(target).to_string(),
    }
}

/// Whether `rev` is `target`'s own commit id, full or abbreviated, which
/// adds nothing next to the abbreviated target.
fn names_by_id(rev: &str, target: &str) -> bool {
    rev.len() >= 4
        && rev.bytes().all(|b| b.is_ascii_hexdigit())
        && target.starts_with(&rev.to_ascii_lowercase())
}

pub fn plural(n: u64) -> &'static str {
    if n == 1 { "" } else { "s" }
}

pub fn list_or_none(items: &[&str]) -> String {
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
    fn a_revision_is_shown_unless_it_is_the_target_id() {
        let target = "782fe977deb6a1b2c3d4e5f60718293a4b5c6d7e";
        assert_eq!(described(target, None), "782fe977deb6");
        assert_eq!(described(target, Some("782fe977deb6")), "782fe977deb6");
        assert_eq!(described(target, Some("782FE97")), "782fe977deb6");
        assert_eq!(described(target, Some(target)), "782fe977deb6");
        assert_eq!(described(target, Some("v1.2")), "782fe977deb6 (v1.2)");
        assert_eq!(
            described(target, Some("782fe977~1")),
            "782fe977deb6 (782fe977~1)"
        );
        assert_eq!(described(target, Some("abcdef")), "782fe977deb6 (abcdef)");
    }

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
