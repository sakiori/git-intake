//! Publication through a merge request.
//!
//! `finish` pushes the verified commit to a branch of its own on the publish
//! remote and asks the remote, through git push options, to open a merge
//! request into the integration branch. No local branch moves, and the update
//! waits for the merge request. A later `finish` completes the update once
//! the publish remote's integration branch received the verified tree;
//! `abort` withdraws the branch.

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

/// Pushes the verified commit to its merge request branch and asks the remote
/// to open the merge request. Applies the same checks as a push and moves no
/// branch.
pub fn open(repo: &Repo, mut state: UpdateState) -> Result<Exit> {
    require_worktree(repo, &state)?;
    let publish = publish_of(&state)?;
    let moves = finish::plan(repo, &state)?;
    let commit = moves[0].new.clone();
    let integration = state.config.integration.clone();
    let branch = format!("{MERGE_REQUEST_BRANCH_PREFIX}{}", abbrev(&commit));
    let refspec = format!("{commit}:{}", branch_ref(&branch));

    let mut args: Vec<String> = vec!["push".into(), "--porcelain".into()];
    for option in push_options(&repo.git, &state, &integration, &commit)? {
        args.push("-o".into());
        args.push(option);
    }
    args.push(publish.remote.clone());
    args.push(refspec.clone());
    let out = repo.git.raw(&args)?;
    let mut number = None;
    let (requested, options_refused) = if out.success() {
        relay_remote_messages(&out.stderr);
        number = merge_request_number(&out.stderr);
        // The remote applies push options only to a push that updates a ref:
        // a branch already in place, as after an interrupted `finish`, opens
        // no merge request.
        (transferred(&out.stdout, &branch), false)
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
        (false, true)
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
        number: number.filter(|_| requested),
        last_seen_head: Some(commit.clone()),
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
            "asked '{}' to open a merge request from '{branch}' into '{integration}'",
            publish.remote
        );
        if let Some(n) = mr.number {
            println!("'{}' opened merge request !{n}", publish.remote);
        }
    } else if options_refused {
        println!(
            "'{}' does not accept push options, so no merge request was opened; open one from '{branch}' into '{integration}'",
            publish.remote
        );
    } else {
        println!(
            "'{branch}' was already on '{}', so this push asked for no merge request; if none is open, open one from '{branch}' into '{integration}'",
            publish.remote
        );
    }
    println!("no local branch moved; the update waits for the merge request");
    println!("next: {}", waiting_next(&mr, &integration));
    Ok(Exit::Success)
}

/// The number of the merge request the remote's reply links to, as GitLab
/// prints it (`https://<host>/<project>/-/merge_requests/<number>`). A link
/// to create one (`.../-/merge_requests/new?...`) names no number.
fn merge_request_number(stderr: &str) -> Option<u64> {
    const MARKER: &str = "/-/merge_requests/";
    stderr.lines().find_map(|line| {
        let rest = &line[line.find(MARKER)? + MARKER.len()..];
        let rest = rest.trim_end();
        (!rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()))
            .then(|| rest.parse().ok())
            .flatten()
    })
}

/// Whether `git push --porcelain` output reports that the push updated the
/// branch, rather than finding it up to date (flag `=`).
fn transferred(porcelain: &str, branch: &str) -> bool {
    let target = format!(":{}", branch_ref(branch));
    porcelain.lines().any(|line| {
        let mut fields = line.split('\t');
        match (fields.next(), fields.next()) {
            (Some(flag), Some(refs)) => refs.ends_with(&target) && flag != "=",
            _ => false,
        }
    })
}

/// What the publish remote's integration branch shows about the merge request.
pub enum Landing {
    /// It received the verified commit by fast-forward, or a merge of it
    /// whose tree is the verified tree. `unattributed` lists commits above a
    /// fast-forwarded verified commit when neither the merge request's head
    /// nor its branch could tell whether they came with it.
    Landed {
        tip: String,
        unattributed: Vec<String>,
    },
    /// It contains the verified commit, but the merge request's head was
    /// another commit (`head`), or the commit that brought it in does not
    /// carry the verified tree.
    Unverified {
        tip: String,
        landing: String,
        head: Option<String>,
    },
    /// It changed without receiving the verified commit; `squash` when a
    /// commit with the verified tree, or the update's sync records, landed.
    Changed { tip: String, squash: bool },
    /// The merge request branch is gone and nothing was merged.
    Closed,
    /// Not merged yet; `head` is the merge request's head, or where its
    /// branch points, and `from_merge_request` says which.
    Waiting {
        head: String,
        from_merge_request: bool,
    },
}

/// Fetches the publish remote's integration branch and classifies it.
/// Records where the merge request branch was last seen in the state.
pub fn inspect(repo: &Repo, state: &mut UpdateState) -> Result<Landing> {
    let git = &repo.git;
    let mr = state
        .merge_request
        .clone()
        .context("the update does not wait for a merge request")?;
    let integration = state.config.integration.clone();
    // The heads are looked up before the integration branch is fetched: a
    // merge that removes the branch afterwards is then seen by the fetch.
    let branch_ref_name = branch_ref(&mr.branch);
    let head_ref = mr.number.map(|n| format!("refs/merge-requests/{n}/head"));
    let mut wanted = vec![branch_ref_name.clone()];
    wanted.extend(head_ref.clone());
    let listed = remote_refs(git, &mr.remote, &wanted)?;
    let branch = listed.get(&branch_ref_name).cloned();
    // The head the remote keeps for the merge request outlives its branch.
    let mr_head = head_ref.and_then(|r| listed.get(&r).cloned());
    if let (Some(b), Some(m)) = (&branch, state.merge_request.as_mut()) {
        m.last_seen_head = Some(b.clone());
    }
    let seen_branch = branch.clone().or(mr.last_seen_head.clone());

    let out = git.raw(&[
        "fetch",
        "--quiet",
        "--no-tags",
        &mr.remote,
        &format!(
            "+{}:{}",
            branch_ref(&integration),
            tracking_ref(&mr.remote, &integration)
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
        .commit(&tracking_ref(&mr.remote, &integration))?
        .with_context(|| format!("'{}' has no branch '{integration}'", mr.remote))?;
    let verified_tree = tree(git, &mr.commit)?;

    if git.is_ancestor(&mr.commit, &tip)? {
        let landing = landing_commit(git, &tip, &state.base, &mr.commit)?;
        // A merge request whose final head is not the verified commit did not
        // land the verified tree, whichever merge method was used. Without
        // its head, the branch as last seen stands in for it when the
        // integration branch took it in.
        let mut changed_head = mr_head.clone().filter(|h| *h != mr.commit);
        if changed_head.is_none()
            && mr_head.is_none()
            && let Some(h) = seen_branch.as_ref().filter(|h| **h != mr.commit)
            && git.object_exists(h)?
            && git.is_ancestor(h, &tip)?
        {
            changed_head = Some(h.clone());
        }
        if changed_head.is_some() {
            return Ok(Landing::Unverified {
                tip,
                landing,
                head: changed_head,
            });
        }
        let landed = landing == mr.commit || {
            let parents = parents(git, &landing)?;
            let mut contained = true;
            for p in &parents {
                contained &= git.is_ancestor(p, &mr.commit)?;
            }
            contained && tree(git, &landing)? == verified_tree
        };
        if !landed {
            return Ok(Landing::Unverified {
                tip,
                landing,
                head: None,
            });
        }
        let unattributed = if landing == mr.commit && mr_head.is_none() && branch.is_none() {
            crate::git::lines(&git.run(&[
                "log",
                "--first-parent",
                "--format=%h %s",
                &tip,
                &format!("^{}", mr.commit),
                "--",
            ])?)
        } else {
            Vec::new()
        };
        return Ok(Landing::Landed { tip, unattributed });
    }
    if tip != state.base && !git.is_ancestor(&tip, &mr.commit)? {
        let mut squash = false;
        for c in first_parent_line(git, &tip, &state.base)? {
            if tree(git, &c)? == verified_tree {
                squash = true;
                break;
            }
        }
        if !squash {
            squash = !carried_records(git, state, &tip, &mr.commit)?.is_empty();
        }
        return Ok(Landing::Changed { tip, squash });
    }
    Ok(match branch {
        None => Landing::Closed,
        Some(branch) => match mr_head {
            Some(head) => Landing::Waiting {
                head,
                from_merge_request: true,
            },
            None => Landing::Waiting {
                head: branch,
                from_merge_request: false,
            },
        },
    })
}

/// Why `finish` cannot complete the update, for every landing but
/// [`Landing::Landed`] and [`Landing::Waiting`].
fn refusal(state: &UpdateState, mr: &MergeRequest, landing: &Landing) -> String {
    let integration = &state.config.integration;
    let remote_branch = format!("{}/{integration}", mr.remote);
    let verified = abbrev(&mr.commit);
    match landing {
        Landing::Unverified {
            tip,
            head: Some(head),
            ..
        } => format!(
            "'{remote_branch}' at {} contains the verified commit {verified}, but the merge request's head is {}, \
             not the verified commit: the merge request carried commits that were never verified. No branch moved. \
             Run `git intake abort` to end this update, then bring '{integration}' up to date and run `git intake start`",
            abbrev(tip),
            abbrev(head)
        ),
        Landing::Unverified { tip, landing, .. } => format!(
            "'{remote_branch}' at {} contains the verified commit {verified}, but what the integration branch received \
             with {} differs from the verified tree: the merge request branch received other commits, or the merge request \
             was merged after '{integration}' moved. No branch moved. That merge was never verified; run `git intake abort` \
             to end this update, then bring '{integration}' up to date and run `git intake start`",
            abbrev(tip),
            abbrev(landing)
        ),
        Landing::Changed { tip, squash: true } => format!(
            "'{remote_branch}' at {} holds the update's changes but not the verified commit {verified}: the merge request was \
             merged by squash or rebase, which replaces the verified commit with commits that were never verified. No branch moved. \
             A merge request of git intake must be merged {MERGE_METHOD}; set the project's merge method accordingly. \
             To take the upstreams in properly, run `git intake abort`, bring '{integration}' up to date with '{remote_branch}', \
             and run `git intake start`",
            abbrev(tip)
        ),
        Landing::Changed { tip, squash: false } => format!(
            "'{remote_branch}' moved to {} without receiving the verified commit {verified}, which no longer contains its tip \
             and the merge request cannot be merged as it is: another change landed first, or the merge request was merged \
             by squash or rebase. No branch moved. Run `git intake abort`, which deletes the merge request branch, bring \
             '{integration}' up to date with '{remote_branch}', and run `git intake start` to merge the upstreams on the new tip",
            abbrev(tip)
        ),
        Landing::Closed => format!(
            "the merge request branch '{}' no longer exists on '{}', and '{remote_branch}' did not receive the verified commit \
             {verified}: the merge request was closed without merging, or its branch was deleted. No branch moved. Run \
             `git intake abort`, then `git intake start` to begin a new update",
            mr.branch, mr.remote
        ),
        Landing::Landed { .. } | Landing::Waiting { .. } => String::new(),
    }
}

/// `finish` while the update waits for its merge request: completes the
/// update once the publish remote's integration branch received the
/// verified tree, reports that it still waits, or refuses.
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
            let landing = inspect(repo, &mut state)?;
            repo.save_state(&state)?;
            let (tip, unattributed) = match landing {
                Landing::Landed { tip, unattributed } => (tip, unattributed),
                Landing::Waiting {
                    head,
                    from_merge_request,
                } => {
                    report_waiting(&state, &mr, &head, from_merge_request, no_publish);
                    return Ok(Exit::Negative);
                }
                other => bail!("{}", refusal(&state, &mr, &other)),
            };
            println!(
                "'{}/{integration}' at {} received the verified commit {}",
                mr.remote,
                abbrev(&tip),
                abbrev(&mr.commit)
            );
            if !unattributed.is_empty() {
                const SHOWN: usize = 5;
                println!(
                    "the merge request's head is unknown, so {} commit{} above the verified commit on '{}/{integration}' \
                     could not be attributed to the merge request; check that {} not part of it:",
                    unattributed.len(),
                    plural(unattributed.len() as u64),
                    mr.remote,
                    if unattributed.len() == 1 {
                        "it was"
                    } else {
                        "they were"
                    }
                );
                for c in unattributed.iter().take(SHOWN) {
                    println!("  {c}");
                }
                if unattributed.len() > SHOWN {
                    println!("  and {} more", unattributed.len() - SHOWN);
                }
            }
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

fn report_waiting(
    state: &UpdateState,
    mr: &MergeRequest,
    head: &str,
    from_merge_request: bool,
    no_publish: bool,
) {
    let integration = &state.config.integration;
    println!(
        "the update waits for the merge request from '{}' into '{integration}' on '{}'",
        mr.branch, mr.remote
    );
    if head != mr.commit {
        println!(
            "{} no longer points at the verified commit {} but at {}; merging it would not land \
             the verified tree, and `finish` would refuse it. Restore the branch to {}, or run `git intake abort`",
            if from_merge_request {
                "the merge request's head"
            } else {
                "the merge request branch"
            },
            abbrev(&mr.commit),
            abbrev(head),
            abbrev(&mr.commit)
        );
    } else if !mr.requested {
        println!("open the merge request if it is not open yet");
    }
    println!("merge it {MERGE_METHOD}, then run `git intake finish` again");
    if no_publish {
        println!(
            "`--no-publish` moves no branch before the merge request is merged; \
             to withdraw the merge request, run `git intake abort`"
        );
    }
}

/// The commit on the first-parent line of `tip` that brought `verified` in:
/// the oldest commit of that line, above `base`, that contains it.
fn landing_commit(git: &Git, tip: &str, base: &str, verified: &str) -> Result<String> {
    let line = first_parent_line(git, tip, base)?;
    // Containment is a prefix of the line (newest first): search its end.
    let (mut lo, mut hi) = (0usize, line.len());
    while lo < hi {
        let mid = (lo + hi) / 2;
        if git.is_ancestor(verified, &line[mid])? {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    Ok(match lo {
        0 => tip.to_string(),
        n => line[n - 1].clone(),
    })
}

/// The first-parent line from `tip` down to, but excluding, `base`, newest
/// first.
fn first_parent_line(git: &Git, tip: &str, base: &str) -> Result<Vec<String>> {
    Ok(crate::git::lines(&git.run(&[
        "rev-list",
        "--first-parent",
        tip,
        &format!("^{base}"),
        "--",
    ])?))
}

fn tree(git: &Git, commit: &str) -> Result<String> {
    git.line(&["rev-parse", &format!("{commit}^{{tree}}")])
}

fn parents(git: &Git, commit: &str) -> Result<Vec<String>> {
    let line = git.line(&["rev-list", "--parents", "-n", "1", commit, "--"])?;
    Ok(line
        .split_whitespace()
        .skip(1)
        .map(str::to_string)
        .collect())
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
pub fn remote_branch(git: &Git, remote: &str, branch: &str) -> Result<Option<String>> {
    let full = branch_ref(branch);
    Ok(remote_refs(git, remote, std::slice::from_ref(&full))?.remove(&full))
}

/// The commits the remote's refs with exactly these names point at; a ref
/// the remote does not have is absent from the result.
fn remote_refs(
    git: &Git,
    remote: &str,
    names: &[String],
) -> Result<std::collections::HashMap<String, String>> {
    let mut args = vec!["ls-remote".to_string(), remote.to_string()];
    args.extend(names.iter().cloned());
    let out = git.raw(&args)?;
    if !out.success() {
        bail!(
            "cannot list the refs of the publish remote '{remote}': {}",
            out.into_error()
        );
    }
    Ok(out
        .stdout
        .lines()
        .filter_map(|line| {
            let (oid, name) = line.split_once('\t')?;
            let name = name.trim();
            names
                .iter()
                .any(|n| n == name)
                .then(|| (name.to_string(), oid.trim().to_string()))
        })
        .collect())
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
            "unless a merge request from '{}' into '{integration}' is open on '{}', open one; then ",
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_merge_request_number_from_the_reply() {
        let reply = "remote: \nremote: View merge request for git-intake/update-0123456789ab:\n\
                     remote:   https://gitlab.example.com/group/project/-/merge_requests/42        \n";
        assert_eq!(merge_request_number(reply), Some(42));
        let create = "remote: To create a merge request for git-intake/update-0123456789ab, visit:\n\
                      remote:   https://gitlab.example.com/group/project/-/merge_requests/new?merge_request%5Bsource_branch%5D=x\n";
        assert_eq!(merge_request_number(create), None);
        assert_eq!(merge_request_number(""), None);
    }

    #[test]
    fn porcelain_tells_an_update_from_an_up_to_date_branch() {
        let b = "git-intake/update-0123456789ab";
        let new = format!("To ../r.git\n*\tabc:refs/heads/{b}\t[new branch]\nDone\n");
        let same = format!("To ../r.git\n=\tabc:refs/heads/{b}\t[up to date]\nDone\n");
        assert!(transferred(&new, b));
        assert!(!transferred(&same, b));
    }
}
