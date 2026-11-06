//! Whether `sync` replaces the update's open merge request with an update
//! at a newer release.
//!
//! With `replace_after` set, `sync` replaces the merge request when the
//! update it plans from the integration tip, as it would start one, takes a
//! newer release of an upstream with `tags`, no pause of the merge queue is
//! recorded, the plan leaves no follow upstream out, takes every follow
//! upstream the merge request's update takes and moves none of them back,
//! and the merge request is untouched and was opened at least
//! `replace_after` ago. Untouched means that its branch holds only the
//! tool's sync merges, at targets its own rules chose, and catch-up merges,
//! none of which needed a resolution, and that the forge shows no approval
//! or review, no requested reviewer, no comment other than its system notes,
//! no reaction, no assignee, no draft flag and no automatic merge. Whatever
//! the forge does not let the token read counts as touched, so every doubt
//! keeps the merge request.
//!
//! The cheap checks, which ask nothing of the forge, come first, then the
//! plan with its conflict predictions, then the reads of the activity. How
//! `sync` carries a replacement out lives with `sync`; the reads just before
//! the close and the close itself live here.

use anyhow::{Result, bail};

use crate::Exit;
use crate::commands::conflicts::{self, ByHand};
use crate::commands::merge_request::{named, remote_refs};
use crate::commands::sync;
use crate::config::{Config, ReplaceAfter};
use crate::description;
use crate::forge::{Forge, Kind, MergeRequest, Signal, State, patiently};
use crate::git::{Git, abbrev, branch_ref};
use crate::output::say;
use crate::pause::{PAUSE_REF, Paused};
use crate::repo::Repo;
use crate::state::QueuedUpstream;
use crate::tags::TagPattern;
use crate::update_line::{History, LineCommit};

/// What `sync` does with the update's open merge request.
pub enum Decision {
    /// It keeps the merge request; the reason, when there is one worth
    /// saying, is said already.
    Keep,
    /// It replaces the merge request with an update at these releases, each
    /// as `` `name` tag ``.
    Replace(Vec<String>),
}

/// The update's open merge request as `sync` read it.
pub struct Open<'a> {
    pub mr: &'a MergeRequest,
    /// Its branch's head.
    pub head: &'a str,
    /// What the update line from that head carries.
    pub history: &'a History,
}

/// Why the cheap checks keep the merge request, before any plan.
pub enum Hold {
    /// GitLab reports it `locked`: it is being merged.
    Locked,
    /// The forge did not say when it was opened.
    NoAge,
    /// It is younger than `replace_after`: opened at the first instant, and
    /// replaceable from the second, or from never when that overflows.
    Young(jiff::Timestamp, Option<jiff::Timestamp>),
    /// Its branch is not untouched, for this reason.
    Branch(String),
    /// A pause of the merge queue is recorded, or its record cannot be read.
    Paused(Paused),
}

impl Hold {
    /// Why the merge request is kept, once `releases` came out.
    fn describe(&self, releases: &str, term: &str, after: &ReplaceAfter) -> String {
        match self {
            Hold::Locked => {
                format!("{releases} came out, but GitLab is merging the {term}")
            }
            Hold::NoAge => {
                format!("the forge did not say when the {term} was opened, so it counts as touched")
            }
            Hold::Young(created, from) => format!(
                "{releases} came out, but the {term} was opened at {} and may be replaced {}, `replace_after` \
                 being {}",
                shown(*created),
                match from {
                    Some(from) => format!("from {} on", shown(*from)),
                    None => "only once that much time has passed".to_string(),
                },
                after.text()
            ),
            Hold::Branch(why) => why.clone(),
            Hold::Paused(Paused::Unreadable) => format!(
                "{releases} came out, but whether a pause of the merge queue is recorded cannot be read, as the \
                 warning above says"
            ),
            Hold::Paused(_) => format!(
                "{releases} came out, but a pause of the merge queue is recorded, as the warning above says, so a \
                 landing may be under way"
            ),
        }
    }
}

/// The checks that ask nothing of the forge, cheapest first: whether the
/// merge request is being merged, its age, its branch, and a pause of the
/// merge queue. Says nothing; [`decide`] says the reason when the plan
/// takes a newer release.
pub fn screen(
    git: &Git,
    config: &Config,
    open: &Open,
    after: &ReplaceAfter,
    paused: Paused,
) -> Result<Option<Hold>> {
    if open.mr.locked {
        return Ok(Some(Hold::Locked));
    }
    let Some(created) = open.mr.created_at else {
        return Ok(Some(Hold::NoAge));
    };
    // A sum past the last instant is a time not reached yet.
    let from = created.checked_add(after.duration()).ok();
    if from.is_none_or(|from| jiff::Timestamp::now() < from) {
        return Ok(Some(Hold::Young(created, from)));
    }
    if let Some(why) = touched_branch(git, config, open.history)? {
        return Ok(Some(Hold::Branch(why)));
    }
    if paused != Paused::No {
        return Ok(Some(Hold::Paused(paused)));
    }
    Ok(None)
}

/// Decides whether `sync` replaces the open merge request `open` with the
/// update of `queue` it planned, which left a follow upstream out when
/// `left_out`, given what [`screen`] held. Says why it keeps the merge
/// request when the plan takes a newer release.
#[allow(clippy::too_many_arguments)]
pub fn decide(
    git: &Git,
    config: &Config,
    forge: &dyn Forge,
    open: &Open,
    queue: &[QueuedUpstream],
    left_out: bool,
    held: Option<Hold>,
    after: &ReplaceAfter,
) -> Result<Decision> {
    let Open { mr, head, history } = *open;
    let kind = forge.kind();
    let term = kind.term();
    let mut releases = Vec::new();
    let mut moved = false;
    for q in queue {
        let Some(f) = q.follow() else {
            continue;
        };
        if git.is_ancestor(&f.target, head)? {
            continue;
        }
        moved = true;
        let tagged = config.upstream(&q.name).is_some_and(|u| u.tags.is_some());
        if let (true, Some(tag)) = (tagged, &f.tag) {
            releases.push(format!("`{}` {tag}", q.name));
        }
    }
    let keep = |reason: String| {
        say!("the {} is not replaced: {reason}", named(kind, mr));
        Ok(Decision::Keep)
    };
    if releases.is_empty() {
        if moved {
            return keep(
                "only a newer release of an upstream with `tags` replaces it, and none came out since it was opened"
                    .to_string(),
            );
        }
        return Ok(Decision::Keep);
    }
    let tags = releases;
    let releases = tags.join(", ");
    if let Some(hold) = held {
        return keep(hold.describe(&releases, term, after));
    }
    if left_out {
        return keep(format!(
            "{releases} came out, but sync left out the follow upstreams named above, and a replacement leaves none out"
        ));
    }
    let missing: Vec<&str> = history
        .follows
        .iter()
        .filter(|f| {
            !queue
                .iter()
                .any(|q| q.name == f.name && q.follow().is_some())
        })
        .map(|f| f.name.as_str())
        .collect();
    if !missing.is_empty() {
        return keep(format!(
            "{releases} came out, but an update planned from '{}' would not take {}, which its update takes",
            config.integration,
            missing.join(", ")
        ));
    }
    if let Some(why) = moved_back(git, config, history, queue)? {
        return keep(format!("{releases} came out, but {why}"));
    }
    if let Some(touch) = activity(forge, mr)? {
        return keep(touch.describe(term));
    }
    Ok(Decision::Replace(tags))
}

/// Why the planned update would move a follow upstream of the merge
/// request's update back: its planned target does not contain the merge
/// request's, or, for an upstream with `tags`, is no higher version by its
/// rules. `None` when every upstream keeps its target or moves forward.
fn moved_back(
    git: &Git,
    config: &Config,
    history: &History,
    queue: &[QueuedUpstream],
) -> Result<Option<String>> {
    for taken in &history.follows {
        let Some(planned) = queue
            .iter()
            .find(|q| q.name == taken.name)
            .and_then(|q| q.follow())
        else {
            continue;
        };
        if planned.target == taken.target {
            continue;
        }
        let from = crate::commands::update::described(&taken.target, taken.tag.as_deref());
        let to = crate::commands::update::described(&planned.target, planned.tag.as_deref());
        if !git.is_ancestor(&taken.target, &planned.target)? {
            return Ok(Some(format!(
                "the update planned from '{}' would take '{}' at {to}, which does not contain {from}, the target of \
                 the merge request's update; a replacement never moves an upstream back",
                config.integration, taken.name
            )));
        }
        let Some(up) = config.upstream(&taken.name) else {
            continue;
        };
        if let Some(pattern) = up.tags.as_deref() {
            let pattern = match TagPattern::parse(pattern) {
                Ok(pattern) => pattern,
                Err(why) => bail!("upstream '{}' tags: {why}", up.name),
            };
            let version =
                |tag: Option<&str>| tag.and_then(|t| pattern.target_version(t, up.prereleases));
            let higher = match (
                version(planned.tag.as_deref()),
                version(taken.tag.as_deref()),
            ) {
                (Some(new), Some(old)) => new > old,
                _ => false,
            };
            if !higher {
                return Ok(Some(format!(
                    "the update planned from '{}' would take '{}' at {to}, which is no higher version than {from}, \
                     the target of the merge request's update; a replacement never moves an upstream back",
                    config.integration, taken.name
                )));
            }
        }
    }
    Ok(None)
}

/// An instant as messages show it, to the second.
fn shown(at: jiff::Timestamp) -> String {
    at.strftime("%Y-%m-%dT%H:%M:%SZ").to_string()
}

/// Why the branch of the merge request whose update line is `history` is
/// not untouched: a commit on the line that is neither a sync merge of a
/// follow upstream nor a catch-up merge, a sync merge whose record does not
/// show that the tool's rules chose its target, a sync merge of an upstream
/// with `tags` that names no tag its newest-release rules admit as a target,
/// or a merge whose tree is not the clean merge of its parents, which
/// `conflicts` recomputes: one that needed a resolution or holds more.
pub fn touched_branch(git: &Git, config: &Config, history: &History) -> Result<Option<String>> {
    for (commit, kind) in history.line.iter().zip(&history.kinds) {
        let (parents, path) = match kind {
            LineCommit::Other => {
                return Ok(Some(format!(
                    "its branch holds {}, which is neither a sync merge nor a catch-up merge: a commit someone \
                     pushed, a merge by the forge's update button, or a port upstream's copy or journal change",
                    abbrev(commit)
                )));
            }
            LineCommit::CatchUp { parents } => (parents, None),
            LineCommit::SyncMerge {
                upstream,
                tag,
                chosen,
                parents,
            } => {
                let up = config.upstream(upstream);
                if chosen.is_none() {
                    return Ok(Some(format!(
                        "its sync merge {} of '{upstream}' does not record that git intake chose its target, so a \
                         person named that revision, or an earlier release made the merge",
                        abbrev(commit)
                    )));
                }
                if let Some(up) = up
                    && let Some(pattern) = up.tags.as_deref()
                {
                    let admitted = tag.as_deref().is_some_and(|t| {
                        TagPattern::parse(pattern)
                            .is_ok_and(|p| p.target_version(t, up.prereleases).is_some())
                    });
                    if !admitted {
                        return Ok(Some(format!(
                            "its sync merge {} of '{upstream}' names no tag the upstream's newest-release rules \
                             admit, so a person chose that revision",
                            abbrev(commit)
                        )));
                    }
                }
                (parents, up.and_then(|u| u.path.as_deref()))
            }
        };
        let merge = if matches!(kind, LineCommit::CatchUp { .. }) {
            "catch-up merge"
        } else {
            "sync merge"
        };
        match conflicts::by_hand(git, commit, parents, path)? {
            ByHand::Nothing => {}
            ByHand::Resolved(files) => {
                return Ok(Some(format!(
                    "its {merge} {} needed a resolution in {}, which a person made",
                    abbrev(commit),
                    files
                        .iter()
                        .map(|p| format!("`{p}`"))
                        .collect::<Vec<_>>()
                        .join(", ")
                )));
            }
            ByHand::Changed => {
                return Ok(Some(format!(
                    "its {merge} {} holds changes beyond the clean merge of its parents, which a person made",
                    abbrev(commit)
                )));
            }
        }
    }
    Ok(None)
}

/// A trace of a person on a merge request, or what the forge did not let
/// the token read.
pub enum Touch {
    Touched(String),
    Unreadable(String),
}

impl Touch {
    /// Why the merge request is kept.
    pub fn describe(&self, term: &str) -> String {
        match self {
            Touch::Touched(what) => what.clone(),
            Touch::Unreadable(what) => {
                format!("{what}, so the {term} counts as touched")
            }
        }
    }
}

/// The first trace of a person the forge shows on `mr`, as it read `mr`,
/// and its approvals or reviews, comments and reactions, read now; `None`
/// when the activity is untouched.
pub fn activity(forge: &dyn Forge, mr: &MergeRequest) -> Result<Option<Touch>> {
    let kind = forge.kind();
    if let Some(touch) = flags(mr) {
        return Ok(Some(touch));
    }
    let (reviewed, reviews) = match kind {
        Kind::GitLab => ("someone approved it", "its approvals"),
        Kind::GitHub => ("someone reviewed it", "its reviews"),
    };
    if let Some(touch) = signal(forge.reviewed(mr.number)?, reviewed, reviews) {
        return Ok(Some(touch));
    }
    if let Some(touch) = signal(
        forge.commented(mr.number)?,
        "someone commented on it",
        "its comments",
    ) {
        return Ok(Some(touch));
    }
    Ok(signal(
        forge.reacted(mr.number)?,
        "someone reacted to it",
        "its reactions",
    ))
}

/// The first trace of a person among what the forge said of `mr` when it
/// read it, which takes no request.
fn flags(mr: &MergeRequest) -> Option<Touch> {
    if mr.merge_scheduled {
        return Some(Touch::Touched(
            "someone scheduled its merge with `merge_after`".to_string(),
        ));
    }
    for (flag, touched, unknown) in [
        (mr.draft, "it is marked as a draft", "whether it is a draft"),
        (
            mr.assigned,
            "someone is assigned to it",
            "whether anyone is assigned to it",
        ),
        (
            mr.review_requested,
            "someone is asked to review it",
            "whether anyone is asked to review it",
        ),
        (
            mr.auto_merge,
            "it is set to merge automatically",
            "whether it is set to merge automatically",
        ),
    ] {
        match flag {
            Some(false) => {}
            Some(true) => return Some(Touch::Touched(touched.to_string())),
            None => {
                return Some(Touch::Unreadable(format!(
                    "the forge's answer did not say {unknown}"
                )));
            }
        }
    }
    None
}

fn signal(read: Signal, touched: &str, what: &str) -> Option<Touch> {
    match read {
        Ok(false) => None,
        Ok(true) => Some(Touch::Touched(touched.to_string())),
        Err(why) => Some(Touch::Unreadable(format!(
            "{what} could not be read: {why}"
        ))),
    }
}

/// Reads, just before the merge request `open` is closed, its activity,
/// then its branch, last read at `head`, the remote's integration branch,
/// last read at `tip`, and the record of a pause in one listing, and its
/// state, head and the traces its answer shows last. Returns the merge request as read last, or why the
/// replacement stops, with the exit status.
pub fn recheck(
    repo: &Repo,
    forge: &dyn Forge,
    remote: &str,
    open: &MergeRequest,
    head: &str,
    tip: &str,
) -> Result<std::result::Result<MergeRequest, (Exit, String)>> {
    let kind = forge.kind();
    let term = kind.term();
    let stopped = |exit, message: String| Ok(Err((exit, message)));
    let read = forge.merge_request(open.number)?;
    if read.state == State::Open
        && let Some(touch) = activity(forge, &read)?
    {
        return stopped(
            Exit::Success,
            format!(
                "the {} is no longer untouched: {}. It stays open, and nothing was closed or pushed",
                named(kind, &read),
                touch.describe(term)
            ),
        );
    }
    let integration = branch_ref(&open.target);
    let source = branch_ref(&open.source);
    let listed = remote_refs(
        &repo.git,
        remote,
        &[integration.clone(), source.clone(), PAUSE_REF.to_string()],
    )?;
    if listed.contains_key(PAUSE_REF) {
        return stopped(
            Exit::Success,
            format!(
                "a pause of the merge queue was recorded on '{remote}' while sync merged the replacement of the {}; \
                 it stays open, and nothing was closed or pushed",
                named(kind, &read)
            ),
        );
    }
    let now = forge.merge_request(open.number)?;
    if now.state != State::Open || now.locked {
        return stopped(
            Exit::Success,
            format!(
                "the {} was {} while sync merged its replacement; nothing was closed or pushed",
                named(kind, &now),
                if now.locked {
                    "being merged"
                } else {
                    now.state.describe()
                }
            ),
        );
    }
    if let Some(touch) = flags(&now) {
        return stopped(
            Exit::Success,
            format!(
                "the {} is no longer untouched: {}. It stays open, and nothing was closed or pushed",
                named(kind, &now),
                touch.describe(term)
            ),
        );
    }
    let unchanged = now.head == open.head
        && read.head == open.head
        && listed.get(&source).map(String::as_str) == Some(head)
        && listed.get(&integration).map(String::as_str) == Some(tip);
    if !unchanged {
        return stopped(
            Exit::Code(2),
            format!(
                "the {} or '{remote}/{}' changed while sync merged its replacement; nothing was closed or pushed. \
                 Run `git intake sync` again",
                named(kind, &now),
                open.target
            ),
        );
    }
    Ok(Ok(now))
}

/// How the close of a replaced merge request ended.
pub enum Closed {
    /// It is closed, with a description that records no targets.
    Done(MergeRequest),
    /// The forge did not close it; its description is restored, and the
    /// replacement stops with this status and message.
    Stopped(Exit, String),
}

/// Closes the merge request `now`, as read last, replacing its description
/// with `note` in the same request, and checks what the forge answers: a
/// merge request the forge merged or left open gets its description back,
/// and one closed with its old description gets `note` in a second request,
/// so that a replaced merge request never declines its targets.
pub fn close(forge: &dyn Forge, now: &MergeRequest, note: &str) -> Result<Closed> {
    let kind = forge.kind();
    let closed = forge.close(now.number, Some(note))?;
    match closed.state {
        State::Closed => {
            if description::targets(&closed.description).is_some() {
                // Retried in place, as the request that opens a merge request
                // after the push: a run that went again would find the merge
                // request closed, declining its targets.
                let kept = |why: String| {
                    anyhow::anyhow!(
                        "{} closed the {} but kept its description, which still records its targets, so it declines \
                         them: {why}. Replace its description by hand with one that records no targets, then run \
                         `git intake sync` again",
                        kind.name(),
                        named(kind, &closed)
                    )
                };
                let again = patiently(sync::ATTEMPTS, sync::retry_gap(), || {
                    forge.set_description(now.number, note)
                })
                .map_err(|err| kept(format!("{err:#}")))?;
                if description::targets(&again.description).is_some() {
                    return Err(kept("the forge kept it again".to_string()));
                }
            }
            Ok(Closed::Done(closed))
        }
        State::Merged => {
            restore(forge, now, &closed, "merged")?;
            Ok(Closed::Stopped(
                Exit::Success,
                format!(
                    "the {} was merged just as sync closed it; its description was restored, and nothing was pushed",
                    named(kind, &closed)
                ),
            ))
        }
        State::Open if closed.locked => {
            restore(forge, now, &closed, "is merging")?;
            Ok(Closed::Stopped(
                Exit::Success,
                format!(
                    "{} is merging the {}, which sync was closing; its description was restored, and nothing was \
                     pushed",
                    kind.name(),
                    named(kind, &closed)
                ),
            ))
        }
        State::Open => {
            restore(forge, now, &closed, "left open")?;
            Ok(Closed::Stopped(
                Exit::Code(2),
                format!(
                    "{} left the {} open when sync closed it; its description was restored, and nothing was pushed. \
                     Run `git intake sync` again",
                    kind.name(),
                    named(kind, &closed)
                ),
            ))
        }
    }
}

/// Gives a merge request the forge did not close its description back,
/// retried in place as the note of a close is. A run that went again would
/// read the merge request with the note of a replacement instead of its
/// targets, so a final failure is not one that may pass.
fn restore(forge: &dyn Forge, now: &MergeRequest, closed: &MergeRequest, what: &str) -> Result<()> {
    let kind = forge.kind();
    patiently(sync::ATTEMPTS, sync::retry_gap(), || {
        forge.set_description(now.number, &now.description)
    })
    .map(|_| ())
    .map_err(|err| {
        anyhow::anyhow!(
            "{} {what} the {} as sync closed it, and its description could not be restored: {err:#}. It now \
             says that it was replaced; put back its earlier description from its history, then run \
             `git intake sync` again",
            kind.name(),
            named(kind, closed)
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync_record::Chosen;

    fn config(tags: Option<&str>) -> Config {
        config_with(tags, "")
    }

    fn config_with(tags: Option<&str>, extra: &str) -> Config {
        let tags = tags
            .map(|t| format!("tags = \"{t}\"\n{extra}"))
            .unwrap_or_default();
        crate::config::parse(&format!(
            "integration = \"main\"\n[[upstream]]\nname = \"a\"\nurl = \"https://example.com/a.git\"\n\
             branch = \"main\"\npath = \"apps/a\"\n{tags}[[upstream]]\nname = \"b\"\n\
             url = \"https://example.com/b.git\"\nbranch = \"main\"\npath = \"apps/b\"\n"
        ))
        .unwrap()
    }

    fn history(kinds: Vec<LineCommit>) -> History {
        History {
            line: (0..kinds.len()).map(|i| format!("{i:040}")).collect(),
            kinds,
            ..History::default()
        }
    }

    /// A sync merge of `a` without parents, whose resolutions are none.
    fn sync_merge(tag: Option<&str>, chosen: Option<Chosen>) -> LineCommit {
        LineCommit::SyncMerge {
            upstream: "a".into(),
            tag: tag.map(str::to_string),
            chosen,
            parents: Vec::new(),
        }
    }

    fn catch_up() -> LineCommit {
        LineCommit::CatchUp {
            parents: Vec::new(),
        }
    }

    fn git() -> (tempfile::TempDir, Git) {
        let dir = tempfile::tempdir().unwrap();
        let git = Git::worktree(dir.path());
        (dir, git)
    }

    const RULES: Option<Chosen> = Some(Chosen::NewestRelease);

    #[test]
    fn sync_merges_the_rules_chose_and_catch_ups_leave_the_branch_untouched() {
        let (_dir, git) = git();
        let tagged = config(Some("v*"));
        let h = history(vec![
            catch_up(),
            sync_merge(Some("v1.2.0"), RULES),
            catch_up(),
        ]);
        assert_eq!(touched_branch(&git, &tagged, &h).unwrap(), None);
        // Without `tags`, a sync merge needs no tag.
        assert_eq!(
            touched_branch(
                &git,
                &config(None),
                &history(vec![sync_merge(None, Some(Chosen::BranchTip))])
            )
            .unwrap(),
            None
        );
    }

    #[test]
    fn anything_else_on_the_branch_touches_it() {
        let (_dir, git) = git();
        let tagged = config(Some("v*"));
        let why = touched_branch(
            &git,
            &tagged,
            &history(vec![LineCommit::Other, sync_merge(Some("v1.2.0"), RULES)]),
        )
        .unwrap()
        .unwrap();
        assert!(
            why.contains("neither a sync merge nor a catch-up merge"),
            "{why}"
        );
        for tag in [None, Some("v1.3.0-rc.1"), Some("hotfix")] {
            let why = touched_branch(&git, &tagged, &history(vec![sync_merge(tag, RULES)]))
                .unwrap()
                .unwrap();
            assert!(
                why.contains("names no tag the upstream's newest-release rules admit"),
                "{tag:?}: {why}"
            );
        }
        // A record without the rule: a revision a person named, or an
        // earlier release's merge.
        let why = touched_branch(
            &git,
            &tagged,
            &history(vec![sync_merge(Some("v1.2.0"), None)]),
        )
        .unwrap()
        .unwrap();
        assert!(
            why.contains("does not record that git intake chose its target"),
            "{why}"
        );
        // With `prereleases`, a prerelease tag is a target as well.
        let prereleases = config_with(Some("v*"), "prereleases = true\n");
        assert_eq!(
            touched_branch(
                &git,
                &prereleases,
                &history(vec![sync_merge(Some("v1.3.0-rc.1"), RULES)])
            )
            .unwrap(),
            None
        );
        let why = touched_branch(
            &git,
            &prereleases,
            &history(vec![sync_merge(Some("hotfix"), RULES)]),
        )
        .unwrap()
        .unwrap();
        assert!(why.contains("names no tag"), "{why}");
    }

    #[test]
    fn an_unreadable_signal_counts_as_touched() {
        let touch = signal(Err("403 Forbidden".into()), "x", "its comments").unwrap();
        let text = touch.describe("pull request");
        assert!(
            text.contains("its comments could not be read: 403 Forbidden"),
            "{text}"
        );
        assert!(
            text.contains("so the pull request counts as touched"),
            "{text}"
        );
        assert!(signal(Ok(false), "x", "y").is_none());
        assert!(matches!(
            signal(Ok(true), "someone commented on it", "y"),
            Some(Touch::Touched(t)) if t == "someone commented on it"
        ));
    }
}
