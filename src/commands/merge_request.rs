//! Publication through a merge request.
//!
//! `finish` pushes the verified commit to the branch `git-intake/update` on
//! the publish remote and asks the remote, through git push options, to open
//! a merge request into the integration branch. No local branch moves, and
//! the update waits for the merge request. When the remote's integration
//! branch moves past the base meanwhile, a later `finish` catches the update
//! up with it, verifies the result and pushes it to the same branch, which
//! only moves forward, so the open merge request carries it. A later
//! `finish` completes the update once the publish remote's integration
//! branch received one of the update's pushed commits with its tree;
//! `abort` withdraws the branch.

use anyhow::{Context, Result, bail};

use crate::Exit;
use crate::commands::catch_up;
use crate::commands::finish::{self, Ready};
use crate::commands::update::{plural, require_worktree};
use crate::config::Publish;
use crate::git::{Git, abbrev, branch_ref, tracking_ref};
use crate::port::Verb;
use crate::repo::{Repo, bullet};
use crate::state::{
    CopyStep, Finishing, MergeKind, MergeRequest, PlannedMove, UPDATE_BRANCH, UpdateState, Work,
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

/// Pushes the verified commit to the merge request branch and asks the
/// remote to open the merge request. Applies the same checks as a push,
/// catching up with the publish remote's integration branch first when it
/// moved past the base, and moves no branch.
pub fn open(repo: &Repo, mut state: UpdateState) -> Result<Exit> {
    require_worktree(repo, &state)?;
    let publish = publish_of(&state)?;
    let integration = state.config.integration.clone();
    let caught = match catch_up::conclude_for_finish(repo, &mut state)? {
        Ok(caught) => caught,
        Err(exit) => return Ok(exit),
    };
    let head = finish::require_finishable(&state)?;
    let branch = UPDATE_BRANCH.to_string();
    // A branch left on the remote is taken over only when the base contains
    // it, as after a merge request merged with its branch kept, or the
    // update head does, as after this update's own first push was
    // interrupted: any other may carry a merge request the tool knows
    // nothing of. Checked before anything is merged or verified.
    let places = listed_places(repo, &publish.remote, &branch)?;
    let mut leftovers = Vec::new();
    for place in &places {
        if let Some(at) = place.at.as_deref()
            && !(repo.git.object_exists(at)?
                && (repo.git.is_ancestor(at, &state.base)? || repo.git.is_ancestor(at, &head)?))
        {
            leftovers.push(format!("{} at {}", place.url, abbrev(at)));
        }
    }
    if !leftovers.is_empty() {
        bail!(
            "the branch '{branch}' already exists where '{}' pushes to, at a commit neither the update's base nor its \
             head contains:\n{}\n\
             It may carry a merge request the tool knows nothing of. Nothing was pushed, and the update stays verified; \
             delete or merge that branch, then run `git intake finish` again",
            publish.remote,
            bullet(&leftovers)
        );
    }
    let tip = fetch_integration(&repo.git, &publish.remote, &integration)?;
    let recheck = |repo: &Repo, state: &mut UpdateState| -> Result<Option<String>> {
        let now = fetch_integration(&repo.git, &publish.remote, &integration)?;
        Ok((!repo.git.is_ancestor(&now, &state.base)?).then(|| {
            format!(
                "'{}/{integration}' moved to {}",
                publish.remote,
                abbrev(&now)
            )
        }))
    };
    let run = finish::Run {
        moved: false,
        caught,
    };
    let commit = match finish::prepare(repo, &mut state, &tip, run, &recheck)? {
        Ready::Publish(head) => head,
        Ready::Holds(head) => return complete_held(repo, state, &publish.remote, &tip, &head),
        Ready::Stop(exit) => return Ok(exit),
    };
    finish::check(repo, &state, &commit)?;

    // A branch already in place, as after an interrupted `finish`, needs no
    // push. Pushing it again with options would update no ref, and git then
    // writes the options to a remote that has stopped reading, which can end
    // the push with SIGPIPE. It is looked up again, and pushed, only where
    // pushes go and the branch is not yet at the verified commit.
    let places = listed_places(repo, &publish.remote, &branch)?;
    let targets = push_targets(&publish.remote, &places, |at| at != Some(commit.as_str()));
    let (mut requested, mut options_refused, mut number) = (false, false, None);
    let mut received: Vec<&str> = Vec::new();
    for target in &targets {
        match push_merge_request_branch(repo, &state, target, &branch, &commit, true)? {
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
        pushed: vec![commit.clone()],
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

/// Where the merge request branch is on each place pushes to `remote` go,
/// refusing, with nothing moved, when that cannot be read.
fn listed_places(repo: &Repo, remote: &str, branch: &str) -> Result<Vec<Place>> {
    match pushed_branch(&repo.git, remote, branch) {
        Ok(places) => Ok(places),
        Err(err) => bail!(
            "cannot check whether the merge request branch '{branch}' is on '{remote}': {err:#}\n\
             Nothing moved, and the update stays verified; fix the cause and run `git intake finish` again"
        ),
    }
}

/// Completes an update whose verified commit `head` the publish remote's
/// integration branch at `tip` already holds, as when it was pushed there by
/// other means: the local integration branch follows it, and a merge
/// request branch an interrupted first push left at `head` is deleted.
fn complete_held(
    repo: &Repo,
    mut state: UpdateState,
    remote: &str,
    tip: &str,
    head: &str,
) -> Result<Exit> {
    let integration = state.config.integration.clone();
    let mut moves = Vec::new();
    if let Some(m) = integration_move(&repo.git, &integration, remote, tip)? {
        moves.push(m);
    }
    finish::validate_checkouts(repo, &moves)?;
    state.finishing = Some(Finishing {
        moves: moves.clone(),
        push_error: None,
        unpublished: Vec::new(),
    });
    repo.save_state(&state)?;
    for m in &moves {
        finish::apply(repo, m)?;
    }
    let left = pushed_branch(&repo.git, remote, UPDATE_BRANCH)?
        .iter()
        .any(|p| p.at.as_deref() == Some(head));
    if left {
        let mr = MergeRequest {
            remote: remote.to_string(),
            branch: UPDATE_BRANCH.to_string(),
            commit: head.to_string(),
            requested: false,
            number: None,
            last_seen_head: None,
            pushed: vec![head.to_string()],
        };
        delete_branch(&repo.git, &mr)?;
    }
    finish::close(repo, state, Vec::new())
}

/// Fetches the publish remote's integration branch into its tracking ref and
/// returns its tip, refusing with git's reason when that fails.
fn fetch_integration(git: &Git, remote: &str, integration: &str) -> Result<String> {
    let out = git.raw(&[
        "fetch",
        "--quiet",
        "--no-tags",
        remote,
        &format!(
            "+{}:{}",
            branch_ref(integration),
            tracking_ref(remote, integration)
        ),
    ])?;
    if !out.success() {
        bail!(
            "cannot fetch '{integration}' from the publish remote '{remote}': {}",
            out.into_error()
        );
    }
    git.commit(&tracking_ref(remote, integration))?
        .with_context(|| format!("'{remote}' has no branch '{integration}'"))
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
/// publish remote or one of its push URLs, without force, with the options
/// that set the merge request's title and description. With `create` they
/// ask the remote to open the merge request; without, as for a catch-up,
/// they only bring an open one up to date, so that a push that races with
/// its merge cannot open a second one. Returns what the push achieved, or
/// why it failed.
fn push_merge_request_branch(
    repo: &Repo,
    state: &UpdateState,
    target: &str,
    branch: &str,
    commit: &str,
    create: bool,
) -> Result<std::result::Result<Pushed, String>> {
    let refspec = format!("{commit}:{}", branch_ref(branch));
    let mut args: Vec<String> = vec!["push".into(), "--porcelain".into()];
    for option in push_options(&repo.git, state, &state.config.integration, commit, create)? {
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
            requested: create && transferred(&out.stdout, branch),
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

/// What the publish remote's integration branch shows about the merge
/// request, in the order `finish` acts on it.
pub enum Landing {
    /// It received `commit`, one of the update's pushed commits, by
    /// fast-forward, or a merge of it whose tree is that commit's tree.
    /// `unattributed` lists commits above a fast-forwarded commit when
    /// neither the merge request's head nor its branch could tell whether
    /// they came with it.
    Landed {
        tip: String,
        unattributed: Vec<String>,
    },
    /// It contains `commit`, one of the update's pushed commits, but the
    /// merge request's head was another commit (`head`), or the commit that
    /// brought it in does not carry its tree.
    Unverified {
        tip: String,
        commit: String,
        landing: String,
        head: Option<ChangedHead>,
    },
    /// It carries, without any of the update's pushed commits, a commit
    /// with a pushed commit's tree, or the update's sync records or copies,
    /// as after a squash or rebase merge.
    Squashed { tip: String },
    /// It no longer contains the base.
    Rewritten { tip: String },
    /// The merge request branch is gone and nothing was merged.
    Closed,
    /// Not merged yet; `head` is the merge request's head, or where its
    /// branch points, and `from_merge_request` says which.
    Waiting {
        head: String,
        from_merge_request: bool,
    },
    /// Not merged yet, and the integration branch moved past the base to
    /// `tip` in another way: the update catches up with it.
    Moved { tip: String },
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
/// Records where the merge request branch was last seen in the state, and a
/// catch-up push that went through before the state could record it.
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
    let pushed = mr.pushed();
    if let (Some(m), Some(b)) = (state.merge_request.as_mut(), &branch) {
        m.last_seen_head = Some(b.clone());
        if pushed.last() == Some(b) {
            m.commit = b.clone();
        }
    }
    let mr = state
        .merge_request
        .clone()
        .context("the update does not wait for a merge request")?;

    let tip = fetch_integration(git, &mr.remote, &integration)?;

    // The newest of the pushed commits the integration branch received.
    let mut received = None;
    for c in pushed.iter().rev() {
        if git.object_exists(c)? && git.is_ancestor(c, &tip)? {
            received = Some(c.clone());
            break;
        }
    }
    if let Some(commit) = received {
        let landing = landing_commit(git, &tip, &state.base, &commit)?;
        // A merge request whose final head is not that commit did not land
        // its tree, whichever merge method was used. Without its head, the
        // branch as last seen stands in for it when the integration branch
        // took it in.
        let mut changed_head = mr_head
            .clone()
            .filter(|h| *h != commit)
            .map(|c| ChangedHead {
                commit: c,
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
            && let Some((h, source)) = seen.filter(|(h, _)| **h != commit)
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
                commit,
                landing,
                head: changed_head,
            });
        }
        let landed = landing == commit || {
            let parents = parents(git, &landing)?;
            let mut contained = true;
            for p in &parents {
                contained &= git.is_ancestor(p, &commit)?;
            }
            contained && tree(git, &landing)? == tree(git, &commit)?
        };
        if !landed {
            return Ok(Landing::Unverified {
                tip,
                commit,
                landing,
                head: None,
            });
        }
        let unattributed = if landing == commit && mr_head.is_none() && branch.is_none() {
            crate::git::lines(&git.run(&[
                "log",
                "--first-parent",
                "--format=%h %s",
                &tip,
                &format!("^{commit}"),
                "--",
            ])?)
        } else {
            Vec::new()
        };
        return Ok(Landing::Landed { tip, unattributed });
    }
    let mut moved = None;
    if tip != state.base && !git.is_ancestor(&tip, &mr.commit)? {
        let mut trees = Vec::new();
        for c in &pushed {
            if git.object_exists(c)? {
                trees.push(tree(git, c)?);
            }
        }
        let mut squash = false;
        if git.is_ancestor(&state.base, &tip)? {
            for c in first_parent_line(git, &tip, &state.base)? {
                if trees.contains(&tree(git, &c)?) {
                    squash = true;
                    break;
                }
            }
        }
        if !squash {
            squash = !carried_records(git, state, &tip, &mr.commit)?.is_empty();
        }
        if squash {
            return Ok(Landing::Squashed { tip });
        }
        if !git.is_ancestor(&state.base, &tip)? {
            return Ok(Landing::Rewritten { tip });
        }
        moved = Some(tip);
    }
    let Some(branch) = branch else {
        return Ok(Landing::Closed);
    };
    let (head, from_merge_request) = match mr_head {
        // The remote moves the merge request's head after a push, so a head
        // still at an earlier pushed commit follows the branch.
        Some(head) if branch == mr.commit && pushed.contains(&head) => (branch, false),
        Some(head) => (head, true),
        None => (branch, false),
    };
    Ok(match moved {
        Some(tip) if head == mr.commit => Landing::Moved { tip },
        _ => Landing::Waiting {
            head,
            from_merge_request,
        },
    })
}

/// Why `finish` cannot complete the update, for every landing but
/// [`Landing::Landed`], [`Landing::Waiting`] and [`Landing::Moved`].
fn refusal(state: &UpdateState, mr: &MergeRequest, landing: &Landing) -> String {
    let integration = &state.config.integration;
    let remote_branch = format!("{}/{integration}", mr.remote);
    let verified = abbrev(&mr.commit);
    match landing {
        Landing::Unverified {
            tip,
            commit,
            head: Some(head),
            ..
        } => format!(
            "'{remote_branch}' at {} contains the verified commit {}, but {}, \
             not the verified commit: the merge request carried commits that were never verified. No branch moved. \
             Run `git intake abort` to end this update, then run `git intake start`, which brings '{integration}' up to date",
            abbrev(tip),
            abbrev(commit),
            head.describe()
        ),
        Landing::Unverified {
            tip,
            commit,
            landing,
            ..
        } => format!(
            "'{remote_branch}' at {} contains the verified commit {}, but what the integration branch received \
             with {} differs from the verified tree: the merge request branch received other commits, or the merge request \
             was merged after '{integration}' moved. No branch moved. That merge was never verified; run `git intake abort` \
             to end this update, then run `git intake start`, which brings '{integration}' up to date",
            abbrev(tip),
            abbrev(commit),
            abbrev(landing)
        ),
        Landing::Squashed { tip } => format!(
            "'{remote_branch}' at {} holds the update's changes but not the verified commit {verified}: the merge request was \
             merged by squash or rebase, which replaces the verified commit with commits that were never verified. No branch moved. \
             A merge request of git intake must be merged {MERGE_METHOD}; set the project's merge method accordingly. \
             The tool opens it {SQUASH_OFF}, but a project whose own or target branch rule's squash setting \
             requires squashing squashes every merge request regardless; such a project cannot take updates through merge requests until that setting \
             allows merging without squashing. \
             To take the upstreams in properly, run `git intake abort`, then `git intake start`, which brings '{integration}' \
             up to date with '{remote_branch}'",
            abbrev(tip)
        ),
        Landing::Rewritten { tip } => format!(
            "'{remote_branch}' at {} no longer contains the update's base {}: it was rewritten, and the update cannot catch \
             up with it. No branch moved. Run `git intake abort`, which deletes the merge request branch, then \
             `git intake start` to plan the update from the new history",
            abbrev(tip),
            abbrev(&state.base)
        ),
        Landing::Closed => format!(
            "the merge request branch '{}' no longer exists on '{}', and '{remote_branch}' did not receive the verified commit \
             {verified}: the merge request was closed without merging, or its branch was deleted. No branch moved. Run \
             `git intake abort`, then `git intake start` to begin a new update",
            mr.branch, mr.remote
        ),
        Landing::Landed { .. } | Landing::Waiting { .. } | Landing::Moved { .. } => String::new(),
    }
}

/// `finish` while the update waits for its merge request: completes the
/// update once the publish remote's integration branch received one of its
/// pushed commits with its tree, catches the update up when that branch moved
/// past the base, reports that it still waits, or refuses.
pub fn complete(repo: &Repo, mut state: UpdateState, no_publish: bool) -> Result<Exit> {
    let integration = state.config.integration.clone();

    let finishing = match &state.finishing {
        Some(f) => {
            println!("resuming finish");
            // A worktree left by an interrupted `finish` is no longer needed.
            repo.remove_update_worktree()?;
            f.clone()
        }
        None => {
            let landing = inspect(repo, &mut state)?;
            repo.save_state(&state)?;
            let mr = state
                .merge_request
                .clone()
                .context("the update does not wait for a merge request")?;
            let (tip, unattributed) = match landing {
                Landing::Landed { tip, unattributed } => (tip, unattributed),
                Landing::Waiting { .. } | Landing::Moved { .. } if no_publish => bail!(
                    "the update waits for the merge request from '{}' into '{integration}' on '{}'; \
                     `--no-publish` cannot close it before the merge request is merged, because the \
                     verified commit is already published on that branch. Merge the merge request \
                     {MERGE_METHOD}, then run `git intake finish --no-publish` to move only the local \
                     branch, or `git intake finish` to delete the merge request branch as well; \
                     `git intake abort` withdraws it",
                    mr.branch,
                    mr.remote
                ),
                Landing::Waiting {
                    head,
                    from_merge_request,
                } if head != mr.commit => {
                    report_waiting(&state, &mr, &head, from_merge_request);
                    // A head that moved off the verified commit needs someone
                    // to act before the update can complete.
                    return Ok(Exit::Negative);
                }
                Landing::Waiting { .. } if caught_up_unpushed(repo, &state)? => {
                    return catch_up_while_waiting(repo, state, None);
                }
                Landing::Waiting {
                    head,
                    from_merge_request,
                } => {
                    // A worktree left by an interrupted `finish` is no longer
                    // needed.
                    repo.remove_update_worktree()?;
                    report_waiting(&state, &mr, &head, from_merge_request);
                    // Waiting is the expected state, not a failure; `status
                    // --check` reports the upstreams as pending meanwhile.
                    return Ok(Exit::Success);
                }
                Landing::Moved { tip } => return catch_up_while_waiting(repo, state, Some(tip)),
                other => bail!("{}", refusal(&state, &mr, &other)),
            };
            repo.remove_update_worktree()?;
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
    if !no_publish && let Some(mr) = &state.merge_request {
        delete_branch(&repo.git, mr)?;
    }
    finish::close(repo, state, Vec::new())
}

/// Whether the update caught up while it waits and has not pushed the
/// result yet: a catch-up is in progress, or the update branch moved past
/// the commit the merge request branch holds.
fn caught_up_unpushed(repo: &Repo, state: &UpdateState) -> Result<bool> {
    let Some(mr) = &state.merge_request else {
        return Ok(false);
    };
    if state.catching_up().is_some() {
        return Ok(true);
    }
    Ok(repo
        .git
        .commit(&branch_ref(UPDATE_BRANCH))?
        .is_some_and(|head| head != mr.commit))
}

/// Catches the waiting update up with the publish remote's integration
/// branch at `tip`, or with none when a catch-up made earlier is still to
/// be verified or pushed, verifies the result, and pushes it to the merge
/// request branch without force, so that the open merge request carries it.
/// A conflict or a failed verification leaves the update waiting for its
/// merge request in the catch-up step.
fn catch_up_while_waiting(
    repo: &Repo,
    mut state: UpdateState,
    tip: Option<String>,
) -> Result<Exit> {
    let mr = state
        .merge_request
        .clone()
        .context("the update does not wait for a merge request")?;
    let integration = state.config.integration.clone();
    restore_worktree(repo, &state)?;
    let caught = match catch_up::conclude_for_finish(repo, &mut state)? {
        Ok(caught) => caught,
        Err(exit) => return Ok(exit),
    };
    let tip = tip.unwrap_or_else(|| state.base.clone());
    let run = finish::Run {
        moved: false,
        caught,
    };
    let head = match finish::prepare(repo, &mut state, &tip, run, &recheck_waiting)? {
        Ready::Publish(head) => head,
        Ready::Holds(head) => bail!(
            "'{}/{integration}' at {} already contains {}, which the update has not pushed to '{}'; \
             run `git intake abort` to end the update",
            mr.remote,
            abbrev(&tip),
            abbrev(&head),
            mr.branch
        ),
        Ready::Stop(exit) => {
            println!(
                "the update keeps waiting for the merge request from '{}' into '{integration}' on '{}', whose head is unchanged",
                mr.branch, mr.remote
            );
            return Ok(exit);
        }
    };
    finish::check(repo, &state, &head)?;
    // Checked again just before the push: a merge that slips in later cannot
    // land a verified tree, and the next `finish` refuses it.
    if let Some(why) = recheck_waiting(repo, &mut state)? {
        println!("{why}; nothing was pushed. Run `git intake finish` again");
        return Ok(Exit::Negative);
    }
    push_catch_up(repo, state, &head)
}

/// Checks out the update branch in the update worktree again, as a catch-up
/// while the update waits needs it.
fn restore_worktree(repo: &Repo, state: &UpdateState) -> Result<()> {
    if repo.update_worktree_registered()? && state.worktree.exists() {
        return Ok(());
    }
    repo.remove_update_worktree()?;
    repo.exclude_update_worktree()?;
    repo.git.run(&[
        "worktree",
        "add",
        "--quiet",
        &state.worktree.to_string_lossy(),
        UPDATE_BRANCH,
    ])?;
    Ok(())
}

/// Whether the merge request's head and branch and the publish remote's
/// integration branch are still as `finish` fetched them. When they are not,
/// a merge that cannot complete the update is refused as `finish` refuses
/// it; otherwise says what changed.
fn recheck_waiting(repo: &Repo, state: &mut UpdateState) -> Result<Option<String>> {
    let mr = state
        .merge_request
        .clone()
        .context("the update does not wait for a merge request")?;
    let integration = state.config.integration.clone();
    let branch = branch_ref(&mr.branch);
    let target = branch_ref(&integration);
    let head = mr.number.map(|n| format!("refs/merge-requests/{n}/head"));
    let mut wanted = vec![branch.clone(), target.clone()];
    wanted.extend(head.clone());
    let listed = remote_refs(&repo.git, &mr.remote, &wanted)?;
    // The remote moves the merge request's head after a push, so a head at
    // an earlier pushed commit is as fetched.
    let pushed = mr.pushed();
    let fetched = repo.git.commit(&tracking_ref(&mr.remote, &integration))?;
    let as_fetched = listed.get(&branch) == Some(&mr.commit)
        && head
            .as_ref()
            .is_none_or(|h| listed.get(h).is_none_or(|c| pushed.contains(c)))
        && listed.get(&target) == fetched.as_ref();
    if as_fetched {
        return Ok(None);
    }
    let landing = inspect(repo, state)?;
    repo.save_state(state)?;
    match landing {
        Landing::Landed { .. } | Landing::Waiting { .. } | Landing::Moved { .. } => {
            Ok(Some(format!(
                "the merge request from '{}' or '{}/{integration}' changed while the update caught up",
                mr.branch, mr.remote
            )))
        }
        other => bail!(
            "{}\nThe merge request changed while the update caught up; nothing was pushed",
            refusal(state, &mr, &other)
        ),
    }
}

/// Pushes the verified catch-up merge `head` to the merge request branch
/// without force, with the options that bring the open merge request's
/// title and description up to date and ask for no new one. The commit is
/// recorded as pushed before the push.
fn push_catch_up(repo: &Repo, mut state: UpdateState, head: &str) -> Result<Exit> {
    let integration = state.config.integration.clone();
    let mut mr = state
        .merge_request
        .clone()
        .context("the update does not wait for a merge request")?;
    let mut pushed = mr.pushed();
    if !pushed.iter().any(|c| c == head) {
        pushed.push(head.to_string());
    }
    mr.pushed = pushed;
    state.merge_request = Some(mr.clone());
    repo.save_state(&state)?;

    let places = match pushed_branch(&repo.git, &mr.remote, &mr.branch) {
        Ok(places) => places,
        Err(err) => bail!(
            "cannot check where the merge request branch '{}' is on '{}': {err:#}\n\
             Nothing was pushed, and the update keeps waiting for its merge request; fix the cause and run \
             `git intake finish` again",
            mr.branch,
            mr.remote
        ),
    };
    let targets = push_targets(&mr.remote, &places, |at| at != Some(head));
    let mut received: Vec<&str> = Vec::new();
    for target in &targets {
        if let Err(why) = push_merge_request_branch(repo, &state, target, &mr.branch, head, false)?
        {
            let moved = if received.is_empty() {
                "Nothing moved".to_string()
            } else {
                format!(
                    "The catch-up merge is already on {}; no other ref moved",
                    received.join(", ")
                )
            };
            bail!(
                "{why}\n{moved}, and the update keeps waiting for its merge request; fix the cause and run \
                 `git intake finish` again, which pushes only where the branch is not at the catch-up merge"
            );
        }
        received.push(target);
    }
    mr.commit = head.to_string();
    mr.last_seen_head = Some(head.to_string());
    state.merge_request = Some(mr.clone());
    repo.save_state(&state)?;
    repo.remove_update_worktree()?;
    if !targets.is_empty() {
        println!(
            "pushed the verified catch-up merge {} to '{}' as branch '{}'; the open merge request carries it, \
             and its title and description were brought up to date",
            abbrev(head),
            mr.remote,
            mr.branch
        );
    }
    println!("no local branch moved; the update waits for the merge request");
    println!("next: {}", waiting_next(&mr, &integration));
    Ok(Exit::Success)
}

fn report_waiting(state: &UpdateState, mr: &MergeRequest, head: &str, from_merge_request: bool) {
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
/// points at one of the update's pushed commits. A branch that is already
/// gone counts as deleted; a branch that moved on is left alone.
pub fn delete_branch(git: &Git, mr: &MergeRequest) -> Result<()> {
    // The branch is judged where the deletion is pushed, on each push URL.
    let places = pushed_branch(git, &mr.remote, &mr.branch)?;
    let pushed = mr.pushed();
    let ours = |at: Option<&str>| at.is_some_and(|at| pushed.iter().any(|c| c == at));
    // The lease makes each deletion conditional on the commit the branch is
    // at there; it never overwrites anything.
    let full = branch_ref(&mr.branch);
    let mut failed = None;
    let mut tried = false;
    let mut leases: Vec<&str> = places.iter().filter_map(|p| p.at.as_deref()).collect();
    leases.retain(|at| ours(Some(at)));
    leases.sort_unstable();
    leases.dedup();
    for lease in leases {
        for target in push_targets(&mr.remote, &places, |at| at == Some(lease)) {
            let out = git.raw(&[
                "push",
                "--quiet",
                &format!("--force-with-lease={full}:{lease}"),
                &target,
                &format!(":{full}"),
            ])?;
            tried = true;
            if !out.success() {
                failed = Some(out.into_error());
            }
        }
    }
    let after = if tried {
        Some(pushed_branch(git, &mr.remote, &mr.branch)?)
    } else {
        None
    };
    let several = places.len() > 1;
    let mut left = false;
    for (i, before) in places.iter().enumerate() {
        let tried = ours(before.at.as_deref());
        let place = after.as_ref().and_then(|a| a.get(i)).unwrap_or(before);
        let on = if several {
            format!("'{}' at {}", mr.remote, place.url)
        } else {
            format!("'{}'", mr.remote)
        };
        match &place.at {
            Some(c) if ours(Some(c)) => left = true,
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

/// Git push options that open the merge request on GitLab, or with `create`
/// off only set the title and the description of the open one. Option
/// values cannot contain line breaks, so the description is one line.
fn push_options(
    git: &Git,
    state: &UpdateState,
    integration: &str,
    commit: &str,
    create: bool,
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
                    (_, None) => match git.merge_base(state.start_base(), &f.target)? {
                        Some(b) => format!("{}..{}", abbrev(&b), abbrev(&f.target)),
                        None => format!("up to {}", abbrev(&f.target)),
                    },
                };
                let tag = f
                    .tag
                    .as_deref()
                    .map(|t| format!(", tag {t}"))
                    .unwrap_or_default();
                ranges.push(format!(
                    "{} from {source}: {range} ({} commit{}{tag})",
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
    if !create {
        return Ok(vec![
            format!("merge_request.title={title}"),
            format!("merge_request.description={description}"),
        ]);
    }
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
        let b = "git-intake/update";
        let reply = "remote: \nremote: View merge request for git-intake/update:\n\
                     remote:   https://gitlab.example.com/group/project/-/merge_requests/42        \n";
        assert_eq!(merge_request_number(reply, b), Some(42));
        let decoy_first = "remote: See https://gitlab.example.com/group/project/-/merge_requests/9\n\
                           remote: View merge request for git-intake/update:\n\
                           remote:   https://gitlab.example.com/group/project/-/merge_requests/42\n";
        assert_eq!(merge_request_number(decoy_first, b), Some(42));
        let decoy_only = "remote: See https://gitlab.example.com/group/project/-/merge_requests/9\n\
                          remote: View merge request for another-branch:\n\
                          remote:   https://gitlab.example.com/group/project/-/merge_requests/8\n";
        assert_eq!(merge_request_number(decoy_only, b), None);
        let create = "remote: To create a merge request for git-intake/update, visit:\n\
                      remote:   https://gitlab.example.com/group/project/-/merge_requests/new?merge_request%5Bsource_branch%5D=x\n";
        assert_eq!(merge_request_number(create, b), None);
        assert_eq!(merge_request_number("", b), None);
    }

    #[test]
    fn porcelain_tells_an_update_from_an_up_to_date_branch() {
        let b = "git-intake/update";
        let new = format!("To ../r.git\n*\tabc:refs/heads/{b}\t[new branch]\nDone\n");
        let same = format!("To ../r.git\n=\tabc:refs/heads/{b}\t[up to date]\nDone\n");
        assert!(transferred(&new, b));
        assert!(!transferred(&same, b));
    }
}
