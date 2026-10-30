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
use crate::port::Verb;
use crate::repo::Repo;
use crate::state::{
    CopyStep, Finishing, MERGE_REQUEST_BRANCH_PREFIX, MergeKind, MergeRequest, PlannedMove,
    UpdateState, Work,
};
use crate::sync_record::{TAKEN_TRAILER, TARGET_TRAILER};

/// What git prints when the receiving end does not accept push options.
/// Git runs with `LC_ALL=C`, so the message is stable.
const NO_PUSH_OPTIONS: &str = "the receiving end does not support push options";

/// How a merge request must be merged for the update to complete.
const MERGE_METHOD: &str = "with a merge commit or by fast-forward, not by squash or rebase";

/// How a merge request opened by hand must be set up: a squash would replace
/// the verified commit, and GitLab otherwise applies the project's default.
const SQUASH_OFF: &str = "with squashing off";

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

    // A branch already in place, as after an interrupted `finish`, needs no
    // push. Pushing it again with options would update no ref, and git then
    // writes the options to a remote that has stopped reading, which can end
    // the push with SIGPIPE. It is looked up, and pushed, only where pushes
    // go and the branch is not yet at the verified commit.
    let places = match pushed_branch(&repo.git, &publish.remote, &branch) {
        Ok(places) => places,
        Err(err) => bail!(
            "cannot check whether the merge request branch '{branch}' is on '{}': {err:#}\n\
             Nothing moved, and the update stays verified; fix the cause and run `git intake finish` again",
            publish.remote
        ),
    };
    let targets = push_targets(&publish.remote, &places, |at| at != Some(commit.as_str()));
    let (mut requested, mut options_refused, mut number) = (false, false, None);
    let mut received: Vec<&str> = Vec::new();
    for target in &targets {
        match push_merge_request_branch(repo, &state, target, &branch, &commit)? {
            Ok(pushed) => {
                requested |= pushed.requested;
                options_refused |= pushed.options_refused;
                number = number.or(pushed.number);
                received.push(target);
            }
            Err(why) => {
                // A push through the remote goes to each of its push URLs in
                // turn and can stop after some of them received the branch.
                let moved = if !received.is_empty() {
                    format!(
                        "The branch is already on {}; no other ref moved",
                        received.join(", ")
                    )
                } else if *target == publish.remote && places.len() > 1 {
                    format!(
                        "The branch may already be on some of the push URLs of '{}'; no other ref moved",
                        publish.remote
                    )
                } else {
                    "Nothing moved".to_string()
                };
                bail!(
                    "{why}\n{moved}, and the update stays verified; fix the cause and run `git intake finish` again, \
                     which pushes only where the branch is missing"
                );
            }
        }
    }

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

    if !targets.is_empty() {
        println!(
            "pushed the verified commit {} to '{}' as branch '{branch}'",
            abbrev(&commit),
            publish.remote
        );
    }
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
            "'{}' does not accept push options, so no merge request was opened; open one from '{branch}' into '{integration}' {SQUASH_OFF}",
            publish.remote
        );
    } else if targets.is_empty() {
        println!(
            "'{branch}' was already on '{}', so nothing was pushed and no merge request was asked for; if none is open, open one from '{branch}' into '{integration}' {SQUASH_OFF}",
            publish.remote
        );
    } else {
        println!(
            "'{branch}' was already on '{}', so this push asked for no merge request; if none is open, open one from '{branch}' into '{integration}' {SQUASH_OFF}",
            publish.remote
        );
    }
    println!("no local branch moved; the update waits for the merge request");
    println!("next: {}", waiting_next(&mr, &integration));
    Ok(Exit::Success)
}

/// What one push of the merge request branch achieved.
struct Pushed {
    /// A push carrying the options transferred the branch.
    requested: bool,
    /// The remote refused push options, so the branch was pushed without.
    options_refused: bool,
    /// The number of the merge request the remote's reply names.
    number: Option<u64>,
}

/// Pushes the verified commit to the merge request branch on `target`, the
/// publish remote or one of its push URLs, asking it to open the merge
/// request. Returns what the push achieved, or why it failed.
fn push_merge_request_branch(
    repo: &Repo,
    state: &UpdateState,
    target: &str,
    branch: &str,
    commit: &str,
) -> Result<std::result::Result<Pushed, String>> {
    let refspec = format!("{commit}:{}", branch_ref(branch));
    let mut args: Vec<String> = vec!["push".into(), "--porcelain".into()];
    for option in push_options(&repo.git, state, &state.config.integration, commit)? {
        args.push("-o".into());
        args.push(option);
    }
    args.push(target.to_string());
    args.push(refspec.clone());
    let out = repo.git.raw(&args)?;
    let failed = |err: crate::git::GitError| {
        format!("pushing the merge request branch '{branch}' to '{target}' failed: {err}")
    };
    Ok(if out.success() {
        relay_remote_messages(&out.stderr);
        // The remote applies push options only to a push that updates a ref:
        // a branch that appeared since it was looked up opens no merge
        // request.
        Ok(Pushed {
            requested: transferred(&out.stdout, branch),
            options_refused: false,
            number: merge_request_number(&out.stderr, branch),
        })
    } else if out.stderr.contains(NO_PUSH_OPTIONS) {
        // Refused before anything was written: push the branch alone.
        let retry = repo.git.raw(&["push", target, &refspec])?;
        if retry.success() {
            relay_remote_messages(&retry.stderr);
            Ok(Pushed {
                requested: false,
                options_refused: true,
                number: None,
            })
        } else {
            Err(failed(retry.into_error()))
        }
    } else {
        Err(failed(out.into_error()))
    })
}

/// The number of the merge request the remote's reply names for `branch`.
/// GitLab answers `View merge request for <branch>:` followed by a line with
/// its address, `https://<host>/<project>/-/merge_requests/<number>`; only
/// that line counts, so other links in the reply cannot supply a number. A
/// link to create one (`.../-/merge_requests/new?...`) names no number.
fn merge_request_number(stderr: &str, branch: &str) -> Option<u64> {
    const MARKER: &str = "/-/merge_requests/";
    let heading = format!("View merge request for {branch}:");
    let text = |line: &str| {
        line.strip_prefix("remote:")
            .unwrap_or(line)
            .trim()
            .to_string()
    };
    let lines: Vec<String> = stderr.lines().map(text).collect();
    let at = lines.iter().position(|l| *l == heading)?;
    let url = lines.get(at + 1)?;
    let rest = &url[url.find(MARKER)? + MARKER.len()..];
    (!rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()))
        .then(|| rest.parse().ok())
        .flatten()
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
        head: Option<ChangedHead>,
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

/// A head of the merge request other than the verified commit, and where the
/// tool saw it.
pub struct ChangedHead {
    pub commit: String,
    pub seen: HeadSource,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum HeadSource {
    /// The merge request's head as the remote keeps it.
    MergeRequest,
    /// The merge request branch as listed now.
    Branch,
    /// The merge request branch as the tool last saw it.
    LastSeenBranch,
}

impl ChangedHead {
    /// Names the head, such as "the merge request's head is X".
    pub fn describe(&self) -> String {
        let c = abbrev(&self.commit);
        match self.seen {
            HeadSource::MergeRequest => format!("the merge request's head is {c}"),
            HeadSource::Branch => format!("the merge request branch is at {c}"),
            HeadSource::LastSeenBranch => format!("the merge request branch was at {c}"),
        }
    }
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
    // Everything here is read where the publish remote is fetched from, not
    // where it is pushed to: the local integration branch later moves to the
    // tracking ref this fetch writes, so the merge is judged on the same side.
    // A fetch side that lags behind the push side only makes the update wait.
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
        let mut changed_head =
            mr_head
                .clone()
                .filter(|h| *h != mr.commit)
                .map(|commit| ChangedHead {
                    commit,
                    seen: HeadSource::MergeRequest,
                });
        // The branch counts as well: a merge request head that matches does
        // not clear a branch the integration branch took in at another commit.
        let seen = match &branch {
            Some(b) => Some((b, HeadSource::Branch)),
            None => mr
                .last_seen_head
                .as_ref()
                .map(|h| (h, HeadSource::LastSeenBranch)),
        };
        if changed_head.is_none()
            && let Some((h, source)) = seen.filter(|(h, _)| **h != mr.commit)
            && git.object_exists(h)?
            && git.is_ancestor(h, &tip)?
        {
            changed_head = Some(ChangedHead {
                commit: h.clone(),
                seen: source,
            });
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
            "'{remote_branch}' at {} contains the verified commit {verified}, but {}, \
             not the verified commit: the merge request carried commits that were never verified. No branch moved. \
             Run `git intake abort` to end this update, then bring '{integration}' up to date and run `git intake start`",
            abbrev(tip),
            head.describe()
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
             The tool opens it {SQUASH_OFF}, but a project whose squash setting requires squashing squashes every \
             merge request regardless; such a project cannot take updates through merge requests until that setting \
             allows merging without squashing. \
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
                    // Waiting is the expected state, not a failure; `status
                    // --check` reports the upstreams as pending meanwhile.
                    report_waiting(&state, &mr, &head, from_merge_request, no_publish);
                    return Ok(Exit::Success);
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
    if !no_publish {
        delete_branch(&repo.git, &mr)?;
    }
    finish::close(repo, state, Vec::new())
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
        println!("if the merge request is not open yet, open it {SQUASH_OFF}");
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

/// Names of the queued upstreams whose sync record for this update's target,
/// or the trailer of one of its copies, appears on `tip` outside the verified
/// commit, as a squash or a rebase leaves it. The whole message is searched,
/// because a squash commit joins the messages of the commits it replaces.
fn carried_records(
    git: &Git,
    state: &UpdateState,
    tip: &str,
    verified: &str,
) -> Result<Vec<String>> {
    let mut found = Vec::new();
    for q in &state.upstreams {
        let trailers: Vec<String> = match &q.work {
            Work::Follow(f) => vec![format!("{TARGET_TRAILER}: {}", f.target)],
            Work::Port(p) => p
                .commits
                .iter()
                .filter(|c| c.copy == CopyStep::Copied)
                .map(|c| format!("{TAKEN_TRAILER}: {}", c.commit))
                .collect(),
        };
        for trailer in trailers {
            let out = git.run(&[
                "log",
                "--format=%H",
                "--fixed-strings",
                &format!("--grep={trailer}"),
                tip,
                "--not",
                verified,
                "--",
            ])?;
            if !out.trim().is_empty() {
                found.push(q.name.clone());
                break;
            }
        }
    }
    Ok(found)
}

/// One URL a push to the publish remote goes to, and the commit the branch
/// in question points at there, or `None` where it does not exist.
struct Place {
    url: String,
    at: Option<String>,
}

/// Where `branch` is on each URL a push to `remote` goes to: its push URLs,
/// or else its URL, as `git push` resolves them. A remote may be fetched from
/// one place and pushed to another, and what decides a push must be read
/// where the push goes.
fn pushed_branch(git: &Git, remote: &str, branch: &str) -> Result<Vec<Place>> {
    let out = git.raw(&["remote", "get-url", "--push", "--all", remote])?;
    if !out.success() {
        bail!(
            "cannot read where the publish remote '{remote}' pushes to: {}",
            out.into_error()
        );
    }
    let full = branch_ref(branch);
    crate::git::lines(&out.stdout)
        .into_iter()
        .map(|url| {
            let at = remote_refs(git, &url, std::slice::from_ref(&full))?.remove(&full);
            Ok(Place { url, at })
        })
        .collect()
}

/// What to push to so that exactly the places `wanted` selects receive the
/// push: the remote itself when that is all of them, so that its own
/// settings apply, and otherwise each selected URL.
fn push_targets(
    remote: &str,
    places: &[Place],
    wanted: impl Fn(Option<&str>) -> bool,
) -> Vec<String> {
    let selected: Vec<&Place> = places.iter().filter(|p| wanted(p.at.as_deref())).collect();
    if selected.is_empty() {
        Vec::new()
    } else if selected.len() == places.len() {
        vec![remote.to_string()]
    } else {
        selected.into_iter().map(|p| p.url.clone()).collect()
    }
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
    // The branch is judged where the deletion is pushed, on each push URL.
    let places = pushed_branch(git, &mr.remote, &mr.branch)?;
    let verified = |at: Option<&str>| at == Some(mr.commit.as_str());
    let targets = push_targets(&mr.remote, &places, verified);
    // The lease makes each deletion conditional on the verified commit; it
    // never overwrites anything.
    let full = branch_ref(&mr.branch);
    let mut failed = None;
    for target in &targets {
        let out = git.raw(&[
            "push",
            "--quiet",
            &format!("--force-with-lease={full}:{}", mr.commit),
            target,
            &format!(":{full}"),
        ])?;
        if !out.success() {
            failed = Some(out.into_error());
        }
    }
    let after = if targets.is_empty() {
        None
    } else {
        Some(pushed_branch(git, &mr.remote, &mr.branch)?)
    };
    let several = places.len() > 1;
    let mut left = false;
    for (i, before) in places.iter().enumerate() {
        let tried = verified(before.at.as_deref());
        let place = after.as_ref().and_then(|a| a.get(i)).unwrap_or(before);
        let on = if several {
            format!("'{}' at {}", mr.remote, place.url)
        } else {
            format!("'{}'", mr.remote)
        };
        match &place.at {
            Some(c) if verified(Some(c)) => left = true,
            Some(c) => println!(
                "left the merge request branch '{}' on {on}: it moved from the verified commit {} to {}",
                mr.branch,
                abbrev(&mr.commit),
                abbrev(c)
            ),
            None if tried => println!("deleted the merge request branch '{}' from {on}", mr.branch),
            None => println!(
                "the merge request branch '{}' is already gone from {on}",
                mr.branch
            ),
        }
    }
    if left {
        let why = failed.map(|e| format!(": {e}")).unwrap_or_default();
        bail!(
            "cannot delete the merge request branch '{}' from '{}'{why}",
            mr.branch,
            mr.remote
        );
    }
    Ok(())
}

/// The command to run while the update waits for its merge request.
pub fn waiting_next(mr: &MergeRequest, integration: &str) -> String {
    let open = if mr.requested {
        String::new()
    } else {
        format!(
            "unless a merge request from '{}' into '{integration}' is open on '{}', open one {SQUASH_OFF}; then ",
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
        match &q.work {
            Work::Follow(f) => {
                let range = match (f.kind, &f.previous) {
                    (MergeKind::Import, _) => format!("full history up to {}", abbrev(&f.target)),
                    (_, Some(p)) => format!("{}..{}", abbrev(p), abbrev(&f.target)),
                    (_, None) => match git.merge_base(&state.base, &f.target)? {
                        Some(b) => format!("{}..{}", abbrev(&b), abbrev(&f.target)),
                        None => format!("up to {}", abbrev(&f.target)),
                    },
                };
                ranges.push(format!(
                    "{} from {source}: {range} ({} commit{})",
                    q.name,
                    f.commits,
                    plural(f.commits)
                ));
            }
            Work::Port(p) => {
                let list = |pick: &dyn Fn(&crate::state::PlannedCommit) -> bool| {
                    let picked: Vec<&str> = p
                        .commits
                        .iter()
                        .filter(|c| pick(c))
                        .map(|c| abbrev(&c.commit))
                        .collect();
                    if picked.is_empty() {
                        "none".to_string()
                    } else {
                        picked.join(" ")
                    }
                };
                let taken: Vec<&str> = p
                    .takes
                    .iter()
                    .map(|t| &p.commits[*t])
                    .filter(|c| c.copy == CopyStep::Copied)
                    .map(|c| abbrev(&c.commit))
                    .collect();
                ranges.push(format!(
                    "{} from {source}, ported: took {}; skipped {}; already present {}",
                    q.name,
                    if taken.is_empty() {
                        "none".to_string()
                    } else {
                        taken.join(" ")
                    },
                    list(&|c| c.verb == Some(Verb::Skip)),
                    list(&|c| c.verb == Some(Verb::Have) || c.copy == CopyStep::Empty)
                ));
            }
        }
    }
    let description = format!(
        "Upstream changes verified by git intake. {}. Verified commit: {commit}. \
         Merge {MERGE_METHOD}, so that the verified commit is what lands on {integration}.",
        ranges.join("; ")
    );
    Ok(vec![
        "merge_request.create".to_string(),
        format!("merge_request.target={integration}"),
        format!("merge_request.title={title}"),
        format!("merge_request.description={description}"),
        "merge_request.remove_source_branch".to_string(),
        // An explicit value overrides the project's default squash setting;
        // only a project that requires squashing ignores it.
        "merge_request.squash=false".to_string(),
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
        let b = "git-intake/update-0123456789ab";
        let reply = "remote: \nremote: View merge request for git-intake/update-0123456789ab:\n\
                     remote:   https://gitlab.example.com/group/project/-/merge_requests/42        \n";
        assert_eq!(merge_request_number(reply, b), Some(42));
        let decoy_first = "remote: See https://gitlab.example.com/group/project/-/merge_requests/9\n\
                           remote: View merge request for git-intake/update-0123456789ab:\n\
                           remote:   https://gitlab.example.com/group/project/-/merge_requests/42\n";
        assert_eq!(merge_request_number(decoy_first, b), Some(42));
        let decoy_only = "remote: See https://gitlab.example.com/group/project/-/merge_requests/9\n\
                          remote: View merge request for another-branch:\n\
                          remote:   https://gitlab.example.com/group/project/-/merge_requests/8\n";
        assert_eq!(merge_request_number(decoy_only, b), None);
        let create = "remote: To create a merge request for git-intake/update-0123456789ab, visit:\n\
                      remote:   https://gitlab.example.com/group/project/-/merge_requests/new?merge_request%5Bsource_branch%5D=x\n";
        assert_eq!(merge_request_number(create, b), None);
        assert_eq!(merge_request_number("", b), None);
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
