//! Publication through a merge request.
//!
//! `finish` pushes the verified commit to a branch of its own on the publish
//! remote and requests, through git push options, a merge request into the
//! integration branch. No local branch moves, and the update waits for the
//! merge request. A later `finish` completes the update once the publish
//! remote's integration branch contains the verified commit; `abort`
//! withdraws the branch.

use anyhow::{Context, Result, bail};

use crate::Exit;
use crate::commands::finish;
use crate::commands::update::{plural, require_worktree};
use crate::config::Publish;
use crate::git::{Git, abbrev, branch_ref, tracking_ref};
use crate::repo::Repo;
use crate::state::{
    Finishing, MERGE_REQUEST_BRANCH_PREFIX, MergeKind, MergeRequest, PlannedMove, UpdateState,
};

/// What git prints when the receiving end does not accept push options.
/// Git runs with `LC_ALL=C`, so the message is stable.
const NO_PUSH_OPTIONS: &str = "the receiving end does not support push options";

/// How a merge request must be merged for the update to complete.
const MERGE_METHOD: &str = "with a merge commit or by fast-forward, not by squash or rebase";

/// Pushes the verified commit to its merge request branch and requests the
/// merge request. Applies the same checks as a push and moves no branch.
pub fn open(repo: &Repo, mut state: UpdateState) -> Result<Exit> {
    require_worktree(repo, &state)?;
    let publish = publish_of(&state)?;
    let moves = finish::plan(repo, &state)?;
    let commit = moves[0].new.clone();
    let integration = state.config.integration.clone();
    let branch = format!("{MERGE_REQUEST_BRANCH_PREFIX}{}", abbrev(&commit));
    let refspec = format!("{commit}:{}", branch_ref(&branch));

    let mut args: Vec<String> = vec!["push".into()];
    for option in push_options(&repo.git, &state, &integration, &commit)? {
        args.push("-o".into());
        args.push(option);
    }
    args.push(publish.remote.clone());
    args.push(refspec.clone());
    let out = repo.git.raw(&args)?;
    let requested = if out.success() {
        relay_remote_messages(&out.stderr);
        true
    } else if out.stderr.contains(NO_PUSH_OPTIONS) {
        // Refused before anything was written: push the branch alone.
        let retry = repo.git.raw(&["push", &publish.remote, &refspec])?;
        if !retry.success() {
            bail!(
                "pushing the merge request branch '{branch}' to '{}' failed: {}\n\
                 Nothing moved, and the update stays verified; fix the cause and run `git intake finish` again",
                publish.remote,
                retry.into_error()
            );
        }
        relay_remote_messages(&retry.stderr);
        false
    } else {
        bail!(
            "pushing the merge request branch '{branch}' to '{}' failed: {}\n\
             Nothing moved, and the update stays verified; fix the cause and run `git intake finish` again",
            publish.remote,
            out.into_error()
        );
    };

    let mr = MergeRequest {
        remote: publish.remote.clone(),
        branch: branch.clone(),
        commit: commit.clone(),
        requested,
    };
    state.merge_request = Some(mr.clone());
    repo.save_state(&state)?;
    repo.remove_update_worktree()?;

    println!(
        "pushed the verified commit {} to '{}' as branch '{branch}'",
        abbrev(&commit),
        publish.remote
    );
    if requested {
        println!(
            "requested a merge request from '{branch}' into '{integration}' on '{}'",
            publish.remote
        );
    } else {
        println!(
            "'{}' does not accept push options, so no merge request was opened; open one from '{branch}' into '{integration}'",
            publish.remote
        );
    }
    println!("no local branch moved; the update waits for the merge request");
    println!("next: {}", waiting_next(&mr, &integration));
    Ok(Exit::Success)
}

/// `finish` while the update waits for its merge request: completes the
/// update once the publish remote's integration branch contains the
/// verified commit, reports that it still waits, or refuses.
pub fn complete(repo: &Repo, mut state: UpdateState, no_publish: bool) -> Result<Exit> {
    let mr = state
        .merge_request
        .clone()
        .context("the update does not wait for a merge request")?;
    // A worktree left by an interrupted `finish` is no longer needed.
    repo.remove_update_worktree()?;
    let integration = state.config.integration.clone();

    let finishing = match &state.finishing {
        Some(f) => {
            println!("resuming finish");
            f.clone()
        }
        None => {
            let Some(tip) = merged_tip(repo, &state, &mr)? else {
                println!(
                    "the update waits for the merge request from '{}' into '{integration}' on '{}'",
                    mr.branch, mr.remote
                );
                if !mr.requested {
                    println!(
                        "'{}' did not accept push options; open the merge request if it is not open yet",
                        mr.remote
                    );
                }
                println!("merge it {MERGE_METHOD}, then run `git intake finish` again");
                if no_publish {
                    println!(
                        "`--no-publish` moves no branch before the merge request is merged; \
                         to withdraw the merge request, run `git intake abort`"
                    );
                }
                return Ok(Exit::Negative);
            };
            let mut moves = Vec::new();
            if let Some(m) = integration_move(&repo.git, &integration, &mr.remote, &tip)? {
                moves.push(m);
            }
            moves.extend(finish::plan_mirrors(repo, &state)?);
            finish::validate_checkouts(repo, &moves)?;
            let f = Finishing {
                moves,
                push_error: None,
                unpublished: Vec::new(),
            };
            state.finishing = Some(f.clone());
            repo.save_state(&state)?;
            f
        }
    };

    for m in &finishing.moves {
        finish::apply(repo, m)?;
    }
    let mirrors: Vec<PlannedMove> = finishing
        .moves
        .iter()
        .filter(|m| m.branch != integration)
        .cloned()
        .collect();
    let names: Vec<String> = mirrors.iter().map(|m| m.branch.clone()).collect();

    let mut unpublished = Vec::new();
    if no_publish {
        if !names.is_empty() {
            println!(
                "not published to '{}' (advanced locally only): {}",
                mr.remote,
                names.join(", ")
            );
        }
        unpublished = names;
    } else {
        if !mirrors.is_empty() {
            if let Err(err) = finish::push_exact(repo, &mr.remote, &mirrors)? {
                if let Some(f) = state.finishing.as_mut() {
                    f.push_error = Some(err.clone());
                }
                repo.save_state(&state)?;
                bail!(
                    "'{integration}' is up to date with the merged merge request and the local mirror branches are advanced, \
                     but pushing them to '{}' was rejected: {err}\n\
                     No mirror branch was published, and the update stays in its finishing step. Run `git intake finish` \
                     to retry the same push, or `git intake finish --no-publish` to close the update locally and publish \
                     the mirror branches by hand. Never force-push.",
                    mr.remote
                );
            }
            println!("pushed {} to '{}'", names.join(", "), mr.remote);
        }
        delete_branch(&repo.git, &mr)?;
    }
    finish::close(repo, state, unpublished)
}

/// The tip of the publish remote's integration branch when it contains the
/// verified commit; `None` while the merge request waits. Refuses when the
/// merge request was merged without the verified commit, was closed, or can
/// no longer be merged as it is.
fn merged_tip(repo: &Repo, state: &UpdateState, mr: &MergeRequest) -> Result<Option<String>> {
    let git = &repo.git;
    let integration = &state.config.integration;
    // The branch is looked up before the integration branch is fetched: a
    // merge that removes the branch afterwards is then seen by the fetch.
    let branch_present = remote_branch(git, &mr.remote, &mr.branch)?.is_some();
    let out = git.raw(&[
        "fetch",
        "--quiet",
        "--no-tags",
        &mr.remote,
        &format!(
            "+{}:{}",
            branch_ref(integration),
            tracking_ref(&mr.remote, integration)
        ),
    ])?;
    if !out.success() {
        bail!(
            "cannot fetch '{integration}' from the publish remote '{}': {}",
            mr.remote,
            out.into_error()
        );
    }
    let tip = git
        .commit(&tracking_ref(&mr.remote, integration))?
        .with_context(|| format!("'{}' has no branch '{integration}'", mr.remote))?;
    let remote_branch_name = format!("{}/{integration}", mr.remote);

    if git.is_ancestor(&mr.commit, &tip)? {
        println!(
            "'{remote_branch_name}' at {} contains the verified commit {}",
            abbrev(&tip),
            abbrev(&mr.commit)
        );
        return Ok(Some(tip));
    }
    let carried = carried_records(git, state, &tip, &mr.commit)?;
    if !carried.is_empty() {
        bail!(
            "'{remote_branch_name}' at {} carries the sync records of this update ({}) but not the verified commit {}: \
             the merge request was merged by squash or rebase, which replaces the verified commit with commits that were never verified. \
             No branch moved. A merge request of git intake must be merged {MERGE_METHOD}; set the project's merge method accordingly. \
             To take the upstreams in properly, run `git intake abort`, bring '{integration}' up to date with '{remote_branch_name}', \
             and run `git intake start`",
            abbrev(&tip),
            carried.join(", "),
            abbrev(&mr.commit)
        );
    }
    if tip != state.base && !git.is_ancestor(&tip, &mr.commit)? {
        bail!(
            "'{remote_branch_name}' moved to {} since the update started, so the verified commit {} no longer contains its tip \
             and the merge request cannot be merged as it is. No branch moved. Run `git intake abort`, which deletes the merge \
             request branch, bring '{integration}' up to date with '{remote_branch_name}', and run `git intake start` to merge \
             the upstreams on the new tip",
            abbrev(&tip),
            abbrev(&mr.commit)
        );
    }
    if !branch_present {
        bail!(
            "the merge request branch '{}' no longer exists on '{}', and '{remote_branch_name}' does not contain the verified commit {}: \
             the merge request was closed without merging, or its branch was deleted. No branch moved. Run `git intake abort`, \
             then `git intake start` to begin a new update",
            mr.branch,
            mr.remote,
            abbrev(&mr.commit)
        );
    }
    Ok(None)
}

/// The move of the local integration branch to the merged remote tip, or
/// `None` when it already contains it.
fn integration_move(
    git: &Git,
    integration: &str,
    remote: &str,
    tip: &str,
) -> Result<Option<PlannedMove>> {
    let local = git.commit(&branch_ref(integration))?;
    Ok(match local {
        None => Some(PlannedMove {
            branch: integration.to_string(),
            old: None,
            new: tip.to_string(),
        }),
        Some(l) if l == tip || git.is_ancestor(tip, &l)? => None,
        Some(l) if git.is_ancestor(&l, tip)? => Some(PlannedMove {
            branch: integration.to_string(),
            old: Some(l),
            new: tip.to_string(),
        }),
        Some(l) => bail!(
            "'{integration}' at {} holds commits that '{remote}/{integration}' at {} lacks, so it cannot be fast-forwarded; \
             merge '{remote}/{integration}' into '{integration}' by hand, then run `git intake finish` again",
            abbrev(&l),
            abbrev(tip)
        ),
    })
}

/// Names of the queued upstreams whose sync record for this update's target
/// appears on `tip` outside the verified commit, as a squash or a rebase
/// leaves it. The whole message is searched, because a squash commit joins
/// the messages of the commits it replaces.
fn carried_records(
    git: &Git,
    state: &UpdateState,
    tip: &str,
    verified: &str,
) -> Result<Vec<String>> {
    let mut found = Vec::new();
    for q in &state.upstreams {
        let out = git.run(&[
            "log",
            "--format=%H",
            "--fixed-strings",
            &format!(
                "--grep={}: {}",
                crate::sync_record::TARGET_TRAILER,
                q.target
            ),
            tip,
            "--not",
            verified,
            "--",
        ])?;
        if !out.trim().is_empty() {
            found.push(q.name.clone());
        }
    }
    Ok(found)
}

/// The commit a branch of the remote points at, or `None` when it does not
/// exist there.
fn remote_branch(git: &Git, remote: &str, branch: &str) -> Result<Option<String>> {
    let full = branch_ref(branch);
    let out = git.raw(&["ls-remote", "--heads", remote, &full])?;
    if !out.success() {
        bail!(
            "cannot list the branches of the publish remote '{remote}': {}",
            out.into_error()
        );
    }
    Ok(out.stdout.lines().find_map(|line| {
        let (oid, name) = line.split_once('\t')?;
        (name.trim() == full).then(|| oid.trim().to_string())
    }))
}

/// Deletes the merge request branch from the publish remote while it still
/// points at the verified commit. A branch that is already gone counts as
/// deleted; a branch that moved on is left alone.
pub fn delete_branch(git: &Git, mr: &MergeRequest) -> Result<()> {
    let describe = |current: Option<String>| match current {
        None => println!(
            "the merge request branch '{}' is already gone from '{}'",
            mr.branch, mr.remote
        ),
        Some(c) => println!(
            "left the merge request branch '{}' on '{}': it moved from the verified commit {} to {}",
            mr.branch,
            mr.remote,
            abbrev(&mr.commit),
            abbrev(&c)
        ),
    };
    let current = remote_branch(git, &mr.remote, &mr.branch)?;
    if current.as_deref() != Some(mr.commit.as_str()) {
        describe(current);
        return Ok(());
    }
    // The lease makes the deletion conditional on the verified commit; it
    // never overwrites anything.
    let full = branch_ref(&mr.branch);
    let out = git.raw(&[
        "push",
        "--quiet",
        &format!("--force-with-lease={full}:{}", mr.commit),
        &mr.remote,
        &format!(":{full}"),
    ])?;
    if out.success() {
        println!(
            "deleted the merge request branch '{}' from '{}'",
            mr.branch, mr.remote
        );
        return Ok(());
    }
    let current = remote_branch(git, &mr.remote, &mr.branch)?;
    if current.as_deref() == Some(mr.commit.as_str()) {
        bail!(
            "cannot delete the merge request branch '{}' from '{}': {}",
            mr.branch,
            mr.remote,
            out.into_error()
        );
    }
    describe(current);
    Ok(())
}

/// The command to run while the update waits for its merge request.
pub fn waiting_next(mr: &MergeRequest, integration: &str) -> String {
    let open = if mr.requested {
        String::new()
    } else {
        format!(
            "open a merge request from '{}' into '{integration}' on '{}', ",
            mr.branch, mr.remote
        )
    };
    format!(
        "{open}merge the merge request from '{}' into '{integration}' {MERGE_METHOD}, then run `git intake finish`; \
         `git intake abort` withdraws it",
        mr.branch
    )
}

fn publish_of(state: &UpdateState) -> Result<Publish> {
    state
        .config
        .publish
        .clone()
        .context("no publish remote is configured")
}

/// Git push options that open the merge request on GitLab. Option values
/// cannot contain line breaks, so the description is one line.
fn push_options(
    git: &Git,
    state: &UpdateState,
    integration: &str,
    commit: &str,
) -> Result<Vec<String>> {
    let names: Vec<&str> = state.upstreams.iter().map(|q| q.name.as_str()).collect();
    let title = format!(
        "Merge upstream update{}: {}",
        plural(names.len() as u64),
        names.join(", ")
    );
    let mut ranges = Vec::new();
    for q in &state.upstreams {
        let source = state
            .config
            .upstream(&q.name)
            .map(|u| u.source())
            .unwrap_or_default();
        let range = match (q.kind, &q.previous) {
            (MergeKind::Import, _) => format!("full history up to {}", abbrev(&q.target)),
            (_, Some(p)) => format!("{}..{}", abbrev(p), abbrev(&q.target)),
            (_, None) => match git.merge_base(&state.base, &q.target)? {
                Some(b) => format!("{}..{}", abbrev(&b), abbrev(&q.target)),
                None => format!("up to {}", abbrev(&q.target)),
            },
        };
        ranges.push(format!(
            "{} from {source}: {range} ({} commit{})",
            q.name,
            q.commits,
            plural(q.commits)
        ));
    }
    let description = format!(
        "Upstream merges verified by git intake. {}. Verified commit: {commit}. \
         Merge {MERGE_METHOD}, so that the verified commit is what lands on {integration}.",
        ranges.join("; ")
    );
    Ok(vec![
        "merge_request.create".to_string(),
        format!("merge_request.target={integration}"),
        format!("merge_request.title={title}"),
        format!("merge_request.description={description}"),
        "merge_request.remove_source_branch".to_string(),
    ])
}

/// Shows what the remote said, such as the address of a new merge request.
fn relay_remote_messages(stderr: &str) {
    for line in stderr.lines() {
        if let Some(rest) = line.strip_prefix("remote:")
            && !rest.trim().is_empty()
        {
            println!("remote:{}", rest.trim_end());
        }
    }
}
