//! Publication through the forge's merge request.
//!
//! In merge-request mode the merge request is the update once it is
//! published: `finish` and `sync` push the update head without force to the
//! update's branch on the publish remote, open the merge request through
//! the forge's API, or rewrite the title and description of the open one,
//! and the local update ends. Before every push the forge's settings must
//! guarantee that only a tree the required pipeline tested can be merged.
//!
//! The update's merge requests are those into the integration branch from
//! `git-intake/update`, or from a branch `git-intake/update-<commit>` an
//! earlier release used.

use anyhow::{Context, Result, bail};

use crate::Exit;
use crate::commands::update::{plural, require_worktree};
use crate::commands::{catch_up, finish, sync};
use crate::config::{self, Config, Publish};
use crate::description::{self, Content, Targets};
use crate::forge::{self, Failure, Forge, Kind, MergeRequest, State, may_pass};
use crate::git::{Git, abbrev, branch_ref, split_z};
use crate::identity;
use crate::output::say;
use crate::port::Verb;
use crate::redact::redacted;
use crate::repo::{Bootstrap, NamedBy, Repo, bullet};
use crate::state::{
    CopyStep, EarlierMergeRequest, MergeKind, UPDATE_BRANCH, UpdateState, Waiting, Work,
};
use crate::sync_record;
use crate::update_line::{self, History};
use crate::upstreams::{self, Fetches};

/// The prefix of the branches an earlier release published updates on, as
/// `git-intake/update-<commit>`.
const EARLIER_BRANCH_PREFIX: &str = "git-intake/update-";

/// Whether `branch` is one an earlier release published an update on.
fn is_earlier_branch(branch: &str) -> bool {
    branch
        .strip_prefix(EARLIER_BRANCH_PREFIX)
        .is_some_and(|c| !c.is_empty() && c.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// Reaches the forge of the publish remote, refusing when it cannot.
pub fn connect(repo: &Repo, publish: &Publish) -> Result<Box<dyn Forge>> {
    forge::connect(&repo.git, publish, forge::Access::MergeRequests)
}

/// The update's merge requests as the forge lists them.
pub struct Requests {
    /// The open one into the integration branch.
    pub open: Option<MergeRequest>,
    /// Open ones into other branches.
    pub elsewhere: Vec<MergeRequest>,
    /// Merged or closed ones into the integration branch, the one that
    /// ended most recently first.
    pub ended: Vec<MergeRequest>,
}

impl Requests {
    /// The merge request closed or merged most recently.
    pub fn latest_ended(&self) -> Option<&MergeRequest> {
        self.ended.first()
    }

    /// The merge request merged most recently.
    pub fn latest_merged(&self) -> Option<&MergeRequest> {
        self.ended.iter().find(|m| m.state == State::Merged)
    }
}

/// Lists the update's merge requests into `integration`: those from
/// `git-intake/update`, from the branches of earlier releases on the publish
/// remote, and from `also`.
pub fn requests(
    repo: &Repo,
    forge: &dyn Forge,
    remote: &str,
    integration: &str,
    also: &[String],
) -> Result<Requests> {
    let mut sources = vec![UPDATE_BRANCH.to_string()];
    for b in earlier_branches(&repo.git, remote)?
        .into_iter()
        .chain(also.iter().cloned())
    {
        if !sources.contains(&b) {
            sources.push(b);
        }
    }
    let mut all: Vec<MergeRequest> = Vec::new();
    for source in &sources {
        for mr in forge.merge_requests(source, None, false)? {
            if mr.source == *source && !all.iter().any(|m| m.number == mr.number) {
                all.push(mr);
            }
        }
    }
    let open = all
        .iter()
        .find(|m| m.state == State::Open && m.target == integration)
        .cloned();
    let elsewhere = all
        .iter()
        .filter(|m| m.state == State::Open && m.target != integration)
        .cloned()
        .collect();
    let mut ended: Vec<MergeRequest> = all
        .into_iter()
        .filter(|m| m.state != State::Open && m.target == integration)
        .collect();
    forge::latest_first(&mut ended);
    Ok(Requests {
        open,
        elsewhere,
        ended,
    })
}

/// The branches `git-intake/update-<commit>` of earlier releases on the
/// publish remote, where it is fetched from.
fn earlier_branches(git: &Git, remote: &str) -> Result<Vec<String>> {
    let pattern = format!("refs/heads/{EARLIER_BRANCH_PREFIX}*");
    let out = git.raw(&["ls-remote", "--heads", remote, &pattern])?;
    if !out.success() {
        return Err(Failure::unanswered(format!(
            "cannot list the branches of the publish remote '{remote}': {}",
            out.into_error()
        ))
        .into());
    }
    Ok(out
        .stdout
        .lines()
        .filter_map(|l| l.split_once('\t'))
        .filter_map(|(_, name)| name.trim().strip_prefix("refs/heads/"))
        .filter(|b| is_earlier_branch(b))
        .map(str::to_string)
        .collect())
}

/// Refuses, pushing nothing, while one of the update's merge requests into
/// another branch than the integration branch is open.
pub fn refuse_elsewhere(kind: Kind, requests: &Requests, integration: &str) -> Result<()> {
    if requests.elsewhere.is_empty() {
        return Ok(());
    }
    let term = kind.term();
    let listed: Vec<String> = requests
        .elsewhere
        .iter()
        .map(|m| {
            format!(
                "{term} {} from '{}' into '{}' ({})",
                kind.reference(m.number),
                m.source,
                m.target,
                m.url
            )
        })
        .collect();
    bail!(
        "the update's {term}s go into '{integration}', but this one is open into another branch:\n{}\n\
         Nothing was pushed. Close it, or change its target branch to '{integration}', then run the command again",
        bullet(&listed)
    )
}

/// Whether the merged merge request `mr` landed without the update's
/// history: the integration branch at `tip` does not contain its final
/// head, or the integration line lacks a sync record for a follow target
/// its description records.
pub fn merged_without_history(git: &Git, mr: &MergeRequest, tip: &str) -> Result<bool> {
    if mr.head.is_empty() || !git.object_exists(&mr.head)? || !git.is_ancestor(&mr.head, tip)? {
        return Ok(true);
    }
    let Some(targets) = description::targets(&mr.description) else {
        return Ok(false);
    };
    if targets.is_empty() {
        return Ok(false);
    }
    let records = sync_record::records(git, tip, &[])?;
    Ok(targets.iter().any(|(name, target)| {
        !records
            .iter()
            .any(|r| r.upstream == *name && r.target.eq_ignore_ascii_case(target))
    }))
}

/// A merge request as messages name it: `merge request !7 (<address>)`.
pub fn named(kind: Kind, mr: &MergeRequest) -> String {
    format!("{} {} ({})", kind.term(), kind.reference(mr.number), mr.url)
}

/// Where a publication stands for a guard of [`publish`].
pub enum Stage<'a> {
    /// The push is next.
    BeforePush,
    /// The update head was pushed to the branch.
    AfterPush(&'a str),
}

/// What [`publish`] asks just before and right after the push: whether to
/// go on, or why it stops.
pub type Guard<'g> = &'g mut dyn FnMut(Stage) -> Result<Option<Stopped>>;

/// Why a guard of [`publish`] ends the publication, and the exit status.
pub struct Stopped {
    pub exit: Exit,
    pub message: String,
    /// The merge request when it is no longer open, and what became of the
    /// branch a push recreated after it ended.
    pub ended: Option<(MergeRequest, Recreated)>,
}

/// The update's branch when its merge request ended during the run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recreated {
    /// Nothing was pushed.
    NotPushed,
    /// The push recreated the branch, which was deleted again.
    Deleted,
    /// The push recreated the branch, and deleting it failed, so it was kept.
    Kept,
}

impl Recreated {
    /// What a message says of the branch.
    pub fn describe(self) -> &'static str {
        match self {
            Recreated::NotPushed => "nothing was pushed",
            Recreated::Deleted => "the branch the push recreated after it ended was deleted again",
            Recreated::Kept => {
                "the branch the push recreated after it ended could not be deleted, as said above; \
                 `git intake abort --merge-request` removes it once what it holds is fetched"
            }
        }
    }
}

/// What [`publish`] publishes.
pub struct Publication<'a> {
    pub forge: &'a dyn Forge,
    pub remote: &'a str,
    /// The branch the merge request comes from.
    pub branch: &'a str,
    /// The update's open merge request from that branch, if any.
    pub open: Option<&'a MergeRequest>,
    /// The merge request merged without its history whose targets the
    /// update proposes again.
    pub retry_of: Option<&'a MergeRequest>,
    /// The merge request the update replaces, and the releases that caused
    /// the replacement.
    pub replaces: Option<description::Replaces>,
}

/// How [`publish`] ended.
pub enum Published {
    Done(MergeRequest),
    Stopped(Stopped),
}

/// Publishes the update at `head` as a merge request: requires the forge's
/// merge settings, names the changes the update makes to its own checks,
/// pushes the head without force to the branch where it is not there yet,
/// and opens the merge request, or rewrites the title and description of
/// the open one. `guard`, when given, is asked just before and right after
/// the push whether to go on. Moves no local branch.
pub fn publish(
    repo: &Repo,
    state: &mut UpdateState,
    head: &str,
    p: Publication,
    mut guard: Option<Guard>,
) -> Result<Published> {
    let forge = p.forge;
    let kind = forge.kind();
    let term = kind.term();
    let integration = state.config.integration.clone();
    let review = require_mergeable(forge, &integration)?;
    let changes = check_changes(&repo.git, state, head, &review.ci_paths)?;
    if !changes.is_empty() {
        say!(
            "the update changes its own checks: {}; the pipeline that verifies it runs what the update brings",
            changes.join(", ")
        );
    }
    let names: Vec<String> = state.upstreams.iter().map(|q| q.name.clone()).collect();
    let title = description::title(&names);
    let content = Content {
        upstreams: upstream_lines(&repo.git, state)?,
        term,
        integration: integration.clone(),
        check_changes: changes,
        retry_of: p
            .retry_of
            .map(|m| format!("{} ({})", kind.reference(m.number), m.url)),
        replaces: p.replaces,
        targets: follow_targets(state),
    };
    let body = description::render(&content, forge.description_limit());
    if let Some(guard) = guard.as_mut()
        && let Some(stop) = guard(Stage::BeforePush)?
    {
        return Ok(Published::Stopped(stop));
    }
    state.published_head = Some(head.to_string());
    repo.save_state(state)?;
    let upstreams = identity::Upstreams::new(&repo.git, &state.config.upstreams)?;
    let pushed = push_branch(repo, p.remote, p.branch, head, term, &upstreams)?;
    if pushed {
        say!(
            "pushed the update head {} to '{}' as branch '{}'",
            abbrev(head),
            p.remote,
            p.branch
        );
    } else {
        say!(
            "'{}' on '{}' is already at the update head {}; nothing was pushed",
            p.branch,
            p.remote,
            abbrev(head)
        );
    }
    if let Some(guard) = guard.as_mut()
        && let Some(stop) = guard(Stage::AfterPush(head))?
    {
        return Ok(Published::Stopped(stop));
    }
    // Past the push a new run finds nothing to do, so what is left of the
    // publication is tried again here.
    let (mr, created) = match p.open {
        Some(open) => (
            forge::patiently(sync::ATTEMPTS, sync::retry_gap(), || {
                forge.update(open.number, &title, &body)
            })?,
            false,
        ),
        None => match forge::patiently(sync::ATTEMPTS, sync::retry_gap(), || {
            forge.open(p.branch, &integration, &title, &body)
        })? {
            Some(mr) => (mr, true),
            // Opened meanwhile, by another run: that one carries the update.
            None => {
                let open = forge
                    .merge_requests(p.branch, Some(&integration), true)?
                    .into_iter()
                    .next()
                    .with_context(|| {
                        format!(
                            "{} refused to open a {term} from '{}' into '{integration}' because one is open, \
                             but lists none",
                            kind.name(),
                            p.branch
                        )
                    })?;
                (
                    forge::patiently(sync::ATTEMPTS, sync::retry_gap(), || {
                        forge.update(open.number, &title, &body)
                    })?,
                    false,
                )
            }
        },
    };
    if created {
        say!("opened {}", named(kind, &mr));
    } else {
        say!("rewrote the title and description of {}", named(kind, &mr));
    }
    Ok(Published::Done(mr))
}

/// Reads the forge's settings for merge requests into `integration`,
/// refusing, before anything is pushed, when they do not guarantee that only
/// a tree the required pipeline tested is merged.
pub fn require_mergeable(forge: &dyn Forge, integration: &str) -> Result<forge::Review> {
    let kind = forge.kind();
    let review = forge.review(integration)?;
    if !review.refusals.is_empty() {
        bail!(
            "the {} project '{}' does not guarantee that only a tree its required pipeline tested is merged, \
             so the update is not published as a {}. Nothing was pushed. Change these settings:\n{}",
            kind.name(),
            forge.location().project,
            kind.term(),
            bullet(&review.refusals)
        );
    }
    Ok(review)
}

/// Reads the merge request's state and head and the remote's integration
/// branch again just before the push, and the merge request's state right
/// after it.
#[allow(clippy::too_many_arguments)]
pub fn guard<'a>(
    repo: &'a Repo,
    forge: &'a dyn Forge,
    remote: &'a str,
    open: &'a MergeRequest,
    head: &'a str,
    tip: &'a str,
    upstreams: &'a identity::Upstreams,
    after_a_change: &'a str,
) -> impl FnMut(Stage) -> Result<Option<Stopped>> + 'a {
    let kind = forge.kind();
    move |stage| {
        let now = forge.merge_request(open.number)?;
        match stage {
            Stage::BeforePush => {
                if now.state != State::Open {
                    return Ok(Some(Stopped {
                        exit: Exit::Success,
                        message: format!(
                            "the {} was {} before the update was pushed to it; nothing was pushed, and no {} was opened",
                            named(kind, &now),
                            now.state.describe(),
                            kind.term()
                        ),
                        ended: Some((now, Recreated::NotPushed)),
                    }));
                }
                let integration = format!("refs/heads/{}", open.target);
                let source = branch_ref(&open.source);
                let listed =
                    remote_refs(&repo.git, remote, &[integration.clone(), source.clone()])?;
                let unchanged = now.head == open.head
                    && listed.get(&source).map(String::as_str) == Some(head)
                    && listed.get(&integration).map(String::as_str) == Some(tip);
                if !unchanged {
                    return Ok(Some(Stopped {
                        exit: Exit::Code(2),
                        message: format!(
                            "the {} or '{remote}/{}' changed meanwhile; nothing was pushed. {after_a_change}",
                            named(kind, &now),
                            open.target
                        ),
                        ended: None,
                    }));
                }
                Ok(None)
            }
            Stage::AfterPush(pushed) => {
                if now.state == State::Open {
                    return Ok(None);
                }
                // The push recreated a branch the merge removed; it is
                // deleted while it is still at the pushed commit.
                let recreated =
                    if delete_branch(&repo.git, remote, &open.source, pushed, upstreams)? {
                        Recreated::Kept
                    } else {
                        Recreated::Deleted
                    };
                Ok(Some(Stopped {
                    exit: Exit::Success,
                    message: format!(
                        "the {} was {} just before the push; {}",
                        named(kind, &now),
                        now.state.describe(),
                        recreated.describe()
                    ),
                    ended: Some((now, recreated)),
                }))
            }
        }
    }
}

/// The heads whose changes to the update's own checks one run of `finish`
/// or `sync` has named, so that each is named once in it.
#[derive(Debug, Default)]
pub struct Announced(Vec<String>);

impl Announced {
    /// Names the changes the update makes, from its base to `head`, to its
    /// own checks, when `finish` or `sync` verifies or pushes the update
    /// itself: before checks run on `head`, and before it is pushed to the
    /// integration branch, as publication through a merge request names
    /// them. Each head is named once per run. The CI
    /// definition counts when the tool can tell the forge without its API.
    pub fn check_changes(&mut self, repo: &Repo, state: &UpdateState, head: &str) -> Result<()> {
        if self.0.iter().any(|h| h == head) {
            return Ok(());
        }
        self.0.push(head.to_string());
        announce_check_changes(repo, state, head)
    }
}

/// Names the changes the update makes, from its base to `head`, to its own
/// checks; [`Announced`] calls it once per head.
fn announce_check_changes(repo: &Repo, state: &UpdateState, head: &str) -> Result<()> {
    let ci_paths = crate::forge::ci_paths_without_api(&repo.git, &state.config.publish)?;
    let changes = check_changes(&repo.git, state, head, &ci_paths)?;
    if !changes.is_empty() {
        say!(
            "the update changes its own checks: {}; the checks that verify it are the ones it brings",
            changes.join(", ")
        );
    }
    Ok(())
}

/// The changes the update makes, from its base to `head`, to its own
/// checks: the forge's CI definition at `ci_paths`, and the install command
/// and checks of the configuration.
fn check_changes(
    git: &Git,
    state: &UpdateState,
    head: &str,
    ci_paths: &[String],
) -> Result<Vec<String>> {
    let mut changes = Vec::new();
    if !ci_paths.is_empty() {
        let mut args = vec![
            "diff".to_string(),
            "--name-only".into(),
            "-z".into(),
            state.base.clone(),
            head.to_string(),
            "--".into(),
        ];
        args.extend(ci_paths.iter().cloned());
        for file in split_z(&git.run(&args)?) {
            changes.push(format!("`{file}`"));
        }
    }
    let before = git.show_file(&state.base, config::FILE_NAME)?;
    let after = git.show_file(head, config::FILE_NAME)?;
    if before != after {
        let parse = |text: Option<String>| text.and_then(|t| config::parse(&t).ok());
        changes.extend(config_changes(
            parse(before).as_ref(),
            parse(after).as_ref(),
        ));
    }
    Ok(changes)
}

/// The changes between two versions of the configuration that differ to its
/// install command and checks: each check added, removed or given another
/// command, by its name, or only their order. A renamed check is one removed
/// and one added. When either side cannot be read, the install command and
/// the checks are named as changed without names.
fn config_changes(before: Option<&Config>, after: Option<&Config>) -> Vec<String> {
    let file = config::FILE_NAME;
    let (Some(before), Some(after)) = (before, after) else {
        return vec![format!("the install command and the checks of `{file}`")];
    };
    let mut changes = Vec::new();
    if before.install != after.install {
        changes.push(format!("the install command of `{file}`"));
    }
    let named = |names: Vec<&str>| {
        let quoted: Vec<String> = names.iter().map(|n| format!("`{n}`")).collect();
        let list = match quoted.split_last() {
            Some((last, [])) => last.clone(),
            Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
            None => String::new(),
        };
        (if names.len() == 1 { "check" } else { "checks" }, list)
    };
    let find =
        |checks: &[config::Check], name: &str| checks.iter().find(|c| c.name == name).cloned();
    let added: Vec<&str> = after
        .checks
        .iter()
        .filter(|c| find(&before.checks, &c.name).is_none())
        .map(|c| c.name.as_str())
        .collect();
    let removed: Vec<&str> = before
        .checks
        .iter()
        .filter(|c| find(&after.checks, &c.name).is_none())
        .map(|c| c.name.as_str())
        .collect();
    let changed: Vec<&str> = after
        .checks
        .iter()
        .filter(|c| find(&before.checks, &c.name).is_some_and(|b| b.run != c.run))
        .map(|c| c.name.as_str())
        .collect();
    if !added.is_empty() {
        let (what, list) = named(added.clone());
        changes.push(format!("the {what} {list} added to `{file}`"));
    }
    if !removed.is_empty() {
        let (what, list) = named(removed.clone());
        changes.push(format!("the {what} {list} removed from `{file}`"));
    }
    if !changed.is_empty() {
        let (what, list) = named(changed.clone());
        changes.push(format!(
            "the {what} {list} given another command in `{file}`"
        ));
    }
    if added.is_empty() && removed.is_empty() && changed.is_empty() && before.checks != after.checks
    {
        changes.push(format!("the order of the checks of `{file}`"));
    }
    changes
}

/// The follow targets of the update, as its merge request records them.
fn follow_targets(state: &UpdateState) -> Targets {
    state
        .upstreams
        .iter()
        .filter_map(|q| q.follow().map(|f| (q.name.clone(), f.target.clone())))
        .collect()
}

/// One line per upstream for the description: a follow upstream's merged
/// range with its target's tag, a port upstream's taken commits and
/// decisions.
fn upstream_lines(git: &Git, state: &UpdateState) -> Result<Vec<String>> {
    let mut lines = Vec::new();
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
                lines.push(format!(
                    "`{}` from {source}: {range} ({} commit{}{tag})",
                    q.name,
                    f.commits,
                    plural(f.commits)
                ));
            }
            Work::Port(p) => {
                let list = |picked: Vec<&str>| {
                    if picked.is_empty() {
                        "none".to_string()
                    } else {
                        picked.join(" ")
                    }
                };
                let taken: Vec<&str> = p
                    .commits
                    .iter()
                    .filter(|c| c.copy == CopyStep::Copied)
                    .map(|c| abbrev(&c.commit))
                    .collect();
                let skipped: Vec<&str> = p
                    .commits
                    .iter()
                    .filter(|c| c.verb == Some(Verb::Skip))
                    .map(|c| abbrev(&c.commit))
                    .collect();
                let present: Vec<&str> = p
                    .commits
                    .iter()
                    .filter(|c| c.verb == Some(Verb::Have) || c.copy == CopyStep::Empty)
                    .map(|c| abbrev(&c.commit))
                    .collect();
                lines.push(format!(
                    "`{}` from {source}, ported: took {}; skipped {}; already present {}",
                    q.name,
                    list(taken),
                    list(skipped),
                    list(present)
                ));
            }
        }
    }
    Ok(lines)
}

/// Pushes `head` without force to `branch` on each place pushes to `remote`
/// go where the branch is not at `head`. Returns whether anything was
/// pushed.
fn push_branch(
    repo: &Repo,
    remote: &str,
    branch: &str,
    head: &str,
    term: &str,
    upstreams: &identity::Upstreams,
) -> Result<bool> {
    upstreams.refuse_push(&repo.git, remote)?;
    let places = match pushed_branch(&repo.git, remote, branch) {
        Ok(places) => places,
        Err(err) => {
            let why = format!(
                "cannot check where the branch '{branch}' is on '{remote}': {err:#}\nNothing was pushed"
            );
            return Err(match may_pass(&err) {
                true => Failure::unanswered(why).into(),
                false => anyhow::anyhow!(why),
            });
        }
    };
    let targets = push_targets(remote, &places, |at| at != Some(head));
    for target in &targets {
        upstreams.refuse_push(&repo.git, target)?;
    }
    let mut received: Vec<&str> = Vec::new();
    for target in &targets {
        let out = repo.git.raw(&[
            "push",
            "--quiet",
            target,
            &format!("{head}:{}", branch_ref(branch)),
        ])?;
        if !out.success() {
            let moved = if !received.is_empty() {
                format!(
                    "The branch is already on {}",
                    redacted(&received.join(", "))
                )
            } else if *target == remote && places.len() > 1 {
                format!("The branch may already be on some of the push URLs of '{remote}'")
            } else {
                "Nothing was pushed".to_string()
            };
            // Rejected for what the branch holds is final: another run pushed
            // first. Anything else may be the connection.
            let final_ = out.stderr.contains("[rejected]");
            let why = format!(
                "pushing the update to the branch '{branch}' on '{}' was rejected: {}\n{moved}, and no {term} \
                 was opened or changed. The branch is never forced: when another run pushed it meanwhile, run the \
                 command again, which pushes only where the branch is not at the update head",
                redacted(target),
                out.into_error()
            );
            return Err(match final_ {
                true => anyhow::anyhow!(why),
                false => Failure::unanswered(why).into(),
            });
        }
        received.push(target);
    }
    Ok(!targets.is_empty())
}

/// The update's branch on the publish remote, without an open merge
/// request, as `finish` and `sync` find it.
pub enum Branch {
    /// The update head contains it.
    Contained,
    /// It holds nothing but catch-up merges beyond the integration tip or
    /// the head of a closed or merged merge request of the update.
    Leftover,
    /// It holds an update no merge request carries.
    Update(Box<History>),
    /// It holds commits that carry no update.
    Unknown,
}

/// Where `branch` is at the publish remote's fetch URL, with its commits
/// fetched for the command, or `None` when it does not exist.
pub fn remote_branch(
    repo: &Repo,
    fetches: &mut Fetches,
    remote: &str,
    branch: &str,
) -> Result<Option<String>> {
    let full = branch_ref(branch);
    let listed = remote_refs(&repo.git, remote, std::slice::from_ref(&full))?;
    if !listed.contains_key(&full) {
        return Ok(None);
    }
    match fetches.remote_branch(remote, branch)? {
        Ok(at) => Ok(Some(at)),
        Err(why) => Err(Failure::unanswered(format!(
            "cannot fetch the branch '{branch}' from '{remote}': {why}"
        ))
        .into()),
    }
}

/// Tells what the update's branch at `at` holds, against the integration
/// tip `tip`, the heads of the update's closed or merged merge requests,
/// and the update head `head` when there is an update.
pub fn classify(
    git: &Git,
    config: &Config,
    at: &str,
    tip: &str,
    ended: &[MergeRequest],
    head: Option<&str>,
) -> Result<Branch> {
    if let Some(head) = head
        && git.is_ancestor(at, head)?
    {
        return Ok(Branch::Contained);
    }
    // Below its catch-up merges, the branch holds what it caught up.
    let mut below = at.to_string();
    while sync_record::catch_up_of(git, &below)?.is_some() {
        match sync_record::parents_of(git, &below)?.get(1) {
            Some(p) => below = p.clone(),
            None => break,
        }
    }
    if git.is_ancestor(&below, tip)? || ended.iter().any(|m| m.head == below) {
        return Ok(Branch::Leftover);
    }
    let base = git.merge_base(at, tip)?.unwrap_or_else(|| tip.to_string());
    let history = update_line::describe(git, config, at, &base)?;
    Ok(if history.carries_update() {
        Branch::Update(Box::new(history))
    } else {
        Branch::Unknown
    })
}

/// Why `finish` or `sync` will not take the update's branch at `at`, which
/// holds commits that carry no update.
pub fn unknown_branch(git: &Git, kind: Kind, remote: &str, at: &str) -> Result<String> {
    let url = redacted(&forge::fetch_url(git, remote)?.unwrap_or_default());
    Ok(format!(
        "the branch '{UPDATE_BRANCH}' on '{remote}' ({url}) is at {}, which holds commits that carry no update, and \
         no {} of the update points at it; it may be someone's work. Nothing was pushed. Fetch what should be kept, \
         then delete the branch with `git intake abort --merge-request`, and run the command again",
        abbrev(at),
        kind.term()
    ))
}

/// Deletes `branch` on each push URL of `remote` while it is at `lease`, and
/// says what happened on each. Returns whether it remains anywhere.
pub fn delete_branch(
    git: &Git,
    remote: &str,
    branch: &str,
    lease: &str,
    upstreams: &identity::Upstreams,
) -> Result<bool> {
    upstreams.refuse_push(git, remote)?;
    let places = pushed_branch(git, remote, branch)?;
    let full = branch_ref(branch);
    let mut failed = None;
    let mut tried = false;
    let targets = push_targets(remote, &places, |at| at == Some(lease));
    for target in &targets {
        upstreams.refuse_push(git, target)?;
    }
    for target in targets {
        // The lease makes the deletion conditional on the commit read; it
        // never overwrites anything.
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
    let after = if tried {
        pushed_branch(git, remote, branch)?
    } else {
        places
            .iter()
            .map(|p| Place {
                url: p.url.clone(),
                at: p.at.clone(),
            })
            .collect()
    };
    let several = places.len() > 1;
    let mut remains = false;
    for (before, now) in places.iter().zip(&after) {
        let on = if several {
            format!("'{remote}' at {}", redacted(&before.url))
        } else {
            format!("'{remote}'")
        };
        match (&before.at, &now.at) {
            (_, Some(c)) if c == lease => {
                remains = true;
                say!(
                    "could not delete the branch '{branch}' from {on}{}",
                    failed
                        .as_ref()
                        .map(|e| format!(": {e}"))
                        .unwrap_or_default()
                );
            }
            (_, Some(c)) => {
                remains = true;
                say!(
                    "kept the branch '{branch}' on {on}: it is at {}, not at {} as read, so it may hold someone's work",
                    abbrev(c),
                    abbrev(lease)
                );
            }
            (Some(_), None) => say!("deleted the branch '{branch}' from {on}"),
            (None, None) => say!("the branch '{branch}' is already gone from {on}"),
        }
    }
    Ok(remains)
}

/// `abort --merge-request` after the local update, if any, was removed:
/// closes the update's open merge request and deletes its branch, and the
/// update's branch, on each push URL while each points at the commit read.
/// `earlier` is the merge request an update of an earlier release waited
/// for.
pub fn withdraw(
    repo: &Repo,
    publish: &Publish,
    integration: &str,
    earlier: Option<&EarlierMergeRequest>,
    upstreams: &identity::Upstreams,
) -> Result<()> {
    let forge = connect(repo, publish)?;
    let kind = forge.kind();
    let term = kind.term();
    let remote = &publish.remote;
    let also: Vec<String> = earlier.map(|e| e.branch.clone()).into_iter().collect();
    let reqs = requests(repo, forge.as_ref(), remote, integration, &also)?;
    let mut done = false;
    let mut branches: Vec<String> = vec![UPDATE_BRANCH.to_string()];
    branches.extend(also);
    if let Some(open) = &reqs.open {
        forge.close(open.number, None)?;
        say!("closed {}", named(kind, open));
        done = true;
        delete_branch(&repo.git, remote, &open.source, &open.head, upstreams)?;
        branches.retain(|b| *b != open.source);
    }
    for branch in branches {
        let full = branch_ref(&branch);
        let listed = remote_refs(&repo.git, remote, std::slice::from_ref(&full))?;
        if let Some(at) = listed.get(&full) {
            done = true;
            delete_branch(&repo.git, remote, &branch, at, upstreams)?;
        }
    }
    if !done {
        say!(
            "no {term} of the update into '{integration}' is open, and its branch is not on '{remote}'; there is \
             nothing to withdraw"
        );
    } else {
        say!(
            "withdrew the update's {term}; its targets count as declined, so `git intake sync` proposes them again \
             only once an upstream moves on, and `git intake start` with `git intake finish` publishes them by hand"
        );
    }
    Ok(())
}

/// The publication setting of the integration tip for an update in
/// progress, or `recorded`, the one the update recorded, when the tip's
/// cannot be read.
pub fn publication(repo: &Repo, recorded: Option<&Publish>) -> Option<Publish> {
    match repo.load_config() {
        Ok(loaded) => Some(loaded.config.publish),
        Err(_) => recorded.cloned(),
    }
}

/// Finds the merge request an update of an earlier release waits for: by
/// its recorded number, else the newest from its branch.
fn earlier_request(
    forge: &dyn Forge,
    mr: &EarlierMergeRequest,
    integration: &str,
) -> Result<Option<MergeRequest>> {
    if let Some(n) = mr.number {
        return forge.merge_request(n).map(Some);
    }
    Ok(forge
        .merge_requests(&mr.branch, Some(integration), false)?
        .into_iter()
        .next())
}

/// What the forge says of the merge request an update of an earlier release
/// waits for, as `abort` reports it.
pub fn earlier_state(repo: &Repo, waiting: &Waiting) -> String {
    let mr = &waiting.merge_request;
    let found = publication(repo, waiting.publish.as_ref())
        .context("no publication setting is configured")
        .and_then(|publish| connect(repo, &publish))
        .and_then(|forge| {
            let found = earlier_request(forge.as_ref(), mr, &waiting.integration)?;
            Ok((forge.kind(), found))
        });
    match found {
        Ok((kind, Some(found))) => format!(
            "the {} the update waited for is {}",
            named(kind, &found),
            found.state.describe()
        ),
        Ok((kind, None)) => format!(
            "no {} from '{}' into '{}' was found",
            kind.term(),
            mr.branch,
            waiting.integration
        ),
        Err(err) => format!("could not read the forge: {err:#}"),
    }
}

/// `finish` on an update of an earlier release that waits for its merge
/// request: hands it over to the merge request. Says whether the merge
/// request is open, merged or closed, records the resolutions of a catch-up
/// step, and removes the update worktree, branch, upstream references and
/// state, keeping the merge request and its branch. The state is read as
/// [`Waiting`] and never written.
pub fn hand_over(repo: &Repo, waiting: &Waiting) -> Result<Exit> {
    let mr = &waiting.merge_request;
    let integration = &waiting.integration;
    let publish = publication(repo, waiting.publish.as_ref()).with_context(|| {
        format!(
            "the update waits for its merge request from '{}', but no publication setting is configured on '{integration}'",
            mr.branch
        )
    })?;
    let forge = connect(repo, &publish)?;
    let kind = forge.kind();
    match earlier_request(forge.as_ref(), mr, integration)? {
        Some(found) => say!(
            "the update waited for {}, which is {}; it carries the update from now on",
            named(kind, &found),
            found.state.describe()
        ),
        None => say!(
            "no {} from '{}' into '{integration}' was found on the forge",
            kind.term(),
            mr.branch
        ),
    }
    if waiting.catching_up && waiting.worktree.exists() {
        let wt = Git::worktree(&waiting.worktree);
        if wt.merge_in_progress()? {
            wt.run(&["rerere"])?;
            say!("recorded the resolutions made so far in the catch-up merge");
        }
    }
    let mut removed = repo.remove_update_artifacts()?;
    removed.extend(repo.remove_plan()?);
    removed.extend(
        upstreams::release_refs(
            &repo.git,
            waiting.upstreams.iter().map(|(name, _)| name.as_str()),
        )?
        .into_iter()
        .map(|r| format!("reference {r}")),
    );
    removed.extend(repo.remove_logs()?);
    repo.paths.remove(&repo.paths.state)?;
    for r in &removed {
        say!("removed {r}");
    }
    say!(
        "handed the update over to its {}; no update remains locally, the {} and its branch '{}' are unchanged, \
         and `git intake sync` carries it on",
        kind.term(),
        kind.term(),
        mr.branch
    );
    Ok(Exit::Success)
}

/// Refuses `start` in merge-request mode when the forge cannot be reached
/// as `finish` needs it, or the update's merge request is open.
pub fn require_startable(repo: &Repo, publish: &Publish, integration: &str) -> Result<()> {
    let forge = connect(repo, publish)?;
    let kind = forge.kind();
    let reqs = requests(repo, forge.as_ref(), &publish.remote, integration, &[])?;
    refuse_elsewhere(kind, &reqs, integration)?;
    if let Some(open) = &reqs.open {
        bail!(
            "the update's {} is open; it carries the update until it is merged or closed. Run `git intake sync` to \
             carry it on, or `git intake abort --merge-request` to withdraw it. Nothing was created",
            named(kind, open)
        );
    }
    Ok(())
}

/// `finish` in merge-request mode: publishes the update as the merge
/// request and ends the local update.
pub fn finish(repo: &Repo, mut state: UpdateState) -> Result<Exit> {
    require_worktree(repo, &state)?;
    let setting = state.config.publish.clone();
    let integration = state.config.integration.clone();
    let remote = setting.remote.clone();
    let forge = connect(repo, &setting)?;
    let kind = forge.kind();
    let term = kind.term();
    let caught = match catch_up::conclude_for_finish(repo, &mut state)? {
        Ok(caught) => caught,
        Err(exit) => return Ok(exit),
    };
    let mut head = require_publishable(&state)?;
    let tip = repo.integration_tip(&Bootstrap::of(&state.config, NamedBy::Update))?;
    if repo.git.is_ancestor(&head, &tip)? {
        say!(
            "'{remote}/{integration}' at {} already contains the update head {}: the update already landed. \
             Nothing was pushed, and no {term} was opened",
            abbrev(&tip),
            abbrev(&head)
        );
        return finish::close(repo, state);
    }
    let carried_branch: Vec<String> = state
        .carried_from
        .iter()
        .map(|c| c.branch.clone())
        .collect();
    let reqs = requests(repo, forge.as_ref(), &remote, &integration, &carried_branch)?;
    refuse_elsewhere(kind, &reqs, &integration)?;
    if let Some(carried) = &state.carried_from {
        let mr = forge.merge_request(carried.number)?;
        if mr.state != State::Open {
            bail!(
                "the update was carried on from {}, which is now {}; it is not published again. The update is kept: \
                 run `git intake abort` to end it",
                named(kind, &mr),
                mr.state.describe()
            );
        }
    }
    let mut fetches = Fetches::new(repo);
    let open = reqs.open.clone();
    if let Some(open) = &open
        && !(repo.git.object_exists(&open.head)? && repo.git.is_ancestor(&open.head, &head)?)
    {
        bail!(
            "the update's {} is open with the head {}, which this update does not contain. Nothing was pushed, and \
             the update is kept. Run `git intake abort`, then `git intake sync` to carry that {term} on; or \
             `git intake abort --merge-request` to withdraw it, which also removes this update and declines its \
             targets until `git intake start` and `git intake finish` publish them again",
            named(kind, open),
            abbrev(&open.head)
        );
    }
    // An open merge request is read again just before and right after the
    // push, as `sync` reads it, against its branch as the remote has it now.
    let read_head = match &open {
        Some(open) => {
            let full = branch_ref(&open.source);
            match remote_refs(&repo.git, &remote, std::slice::from_ref(&full))?.remove(&full) {
                Some(at) => at,
                None => bail!("{}", branch_gone(kind, open, &remote)),
            }
        }
        None => String::new(),
    };
    let branch = match &open {
        Some(open) => open.source.clone(),
        None => state
            .carried_from
            .as_ref()
            .map_or(UPDATE_BRANCH.to_string(), |c| c.branch.clone()),
    };
    if open.is_none()
        && let Some(at) = remote_branch(repo, &mut fetches, &remote, &branch)?
    {
        match classify(
            &repo.git,
            &state.config,
            &at,
            &tip,
            &reqs.ended,
            Some(&head),
        )? {
            Branch::Contained => {}
            Branch::Leftover => {
                say!(
                    "the branch '{branch}' on '{remote}' holds nothing but what landed or a closed {term} held; \
                     deleting it"
                );
                if delete_branch(
                    &repo.git,
                    &remote,
                    &branch,
                    &at,
                    &identity::Upstreams::new(&repo.git, &state.config.upstreams)?,
                )? {
                    bail!(
                        "the branch '{branch}' remains on '{remote}'; nothing was pushed. Run `git intake finish` again"
                    );
                }
            }
            Branch::Update(_) | Branch::Unknown => {
                bail!("{}", unknown_branch(&repo.git, kind, &remote, &at)?)
            }
        }
    }
    if tip != state.base {
        if caught {
            say!(
                "'{remote}/{integration}' moved to {} since the catch-up merge was made; nothing was pushed. Run \
                 `git intake finish` again to catch up with it",
                abbrev(&tip)
            );
            return Ok(Exit::Negative);
        }
        catch_up::require_possible(repo, &state, &tip, catch_up::Advice::Abort)?;
        if let Some(exit) = catch_up::run(repo, &mut state, &tip)? {
            return Ok(exit);
        }
        head = Git::worktree(&state.worktree).line(&["rev-parse", "HEAD"])?;
    }
    finish::check(repo, &state, &head)?;
    let publication = Publication {
        forge: forge.as_ref(),
        remote: &remote,
        branch: &branch,
        open: open.as_ref(),
        retry_of: None,
        replaces: None,
    };
    let published = match &open {
        Some(open) => {
            let upstreams = identity::Upstreams::new(&repo.git, &state.config.upstreams)?;
            let mut guard = guard(
                repo,
                forge.as_ref(),
                &remote,
                open,
                &read_head,
                &tip,
                &upstreams,
                "Run the command again",
            );
            publish(repo, &mut state, &head, publication, Some(&mut guard))?
        }
        None => publish(repo, &mut state, &head, publication, None)?,
    };
    let mr = match published {
        Published::Done(mr) => mr,
        // A merge request no longer open is refused as for an update carried
        // on from it, keeping the update.
        Published::Stopped(Stopped {
            ended: Some((mr, recreated)),
            ..
        }) => bail!(
            "the {} is now {}, so the update is not published to it; {}. The update is kept: run \
             `git intake abort` to end it",
            named(kind, &mr),
            mr.state.describe(),
            recreated.describe()
        ),
        Published::Stopped(stop) => {
            say!("{}", stop.message);
            return Ok(stop.exit);
        }
    };
    drop(fetches);
    let exit = finish::close(repo, state)?;
    say!(
        "the {term} carries the update from now on: {}",
        named(kind, &mr)
    );
    Ok(exit)
}

/// Why an open merge request whose branch is not on the publish remote is
/// left alone.
pub fn branch_gone(kind: Kind, open: &MergeRequest, remote: &str) -> String {
    format!(
        "the {} is open, but its branch '{}' is not on '{remote}'; nothing was pushed. Restore the branch, or \
         withdraw the {} with `git intake abort --merge-request`",
        named(kind, open),
        open.source,
        kind.term()
    )
}

/// Refuses, before anything moves, an update that cannot be published as a
/// merge request: a queued upstream is not merged, or its plan not carried
/// out, or the update worktree has uncommitted changes or a merge in
/// progress. Returns the update head.
pub fn require_publishable(state: &UpdateState) -> Result<String> {
    let open: Vec<&str> = state.unmerged().iter().map(|u| u.name.as_str()).collect();
    if !open.is_empty() {
        bail!(
            "upstreams are not merged, or their plans not carried out, yet: {}; run `git intake continue`",
            open.join(", ")
        );
    }
    let wt = Git::worktree(&state.worktree);
    if wt.merge_in_progress()? || !wt.tracked_changes()?.is_empty() {
        bail!(
            "the update worktree {} has uncommitted changes or a merge in progress; commit or discard them first",
            state.worktree.display()
        );
    }
    wt.line(&["rev-parse", "HEAD"])
}

/// The update's open merge request for `status`: the forge's kind, the
/// merge request, and the newest one merged without its history.
pub struct Overview {
    pub kind: Kind,
    pub open: Option<MergeRequest>,
    pub merged_without_history: Option<MergeRequest>,
}

/// Reads what `status` shows of the forge, against the integration tip
/// `tip`.
pub fn overview(repo: &Repo, config: &Config, tip: &str) -> Result<Overview> {
    let publish = &config.publish;
    let forge = connect(repo, publish)?;
    let reqs = requests(
        repo,
        forge.as_ref(),
        &publish.remote,
        &config.integration,
        &[],
    )?;
    let mut without_history = None;
    if let Some(merged) = reqs.latest_merged()
        && merged_without_history(&repo.git, merged, tip)?
    {
        without_history = Some(merged.clone());
    }
    let mut open = reqs.open;
    if let Some(mr) = &mut open
        && !mr.head.is_empty()
        && !repo.git.object_exists(&mr.head)?
    {
        // The head is read from the remote, so that `status` can tell
        // whether it contains the integration tip.
        let mut fetches = Fetches::new(repo);
        if let Ok(at) = fetches.remote_branch(&publish.remote, &mr.source)? {
            mr.head = at;
        }
    }
    Ok(Overview {
        kind: forge.kind(),
        open,
        merged_without_history: without_history,
    })
}

/// One URL a push to the publish remote goes to, and the commit the branch
/// in question points at there, or `None` where it does not exist.
pub struct Place {
    pub url: String,
    pub at: Option<String>,
}

/// Where `branch` is on each URL a push to `remote` goes to: its push URLs,
/// or else its URL, as `git push` resolves them. A remote may be fetched from
/// one place and pushed to another, and what decides a push must be read
/// where the push goes.
pub fn pushed_branch(git: &Git, remote: &str, branch: &str) -> Result<Vec<Place>> {
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
pub fn remote_refs(
    git: &Git,
    remote: &str,
    names: &[String],
) -> Result<std::collections::HashMap<String, String>> {
    let mut args = vec!["ls-remote".to_string(), remote.to_string()];
    args.extend(names.iter().cloned());
    let out = git.raw(&args)?;
    if !out.success() {
        return Err(Failure::unanswered(format!(
            "cannot list the refs of the publish remote '{}': {}",
            redacted(remote),
            out.into_error()
        ))
        .into());
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

#[cfg(test)]
mod tests {
    use super::*;

    fn config(install: Option<&str>, checks: &[(&str, &str)]) -> Config {
        Config {
            integration: "main".into(),
            publish: config::Publish {
                remote: "origin".into(),
                via: config::PublishVia::MergeRequest,
                forge: None,
                api: None,
                queue: None,
                replace_after: None,
            },
            install: install.map(str::to_string),
            checks: checks
                .iter()
                .map(|(name, run)| config::Check {
                    name: name.to_string(),
                    run: run.to_string(),
                })
                .collect(),
            upstreams: Vec::new(),
            release: None,
        }
    }

    #[test]
    fn changed_checks_are_named() {
        let before = config(
            Some("make deps"),
            &[("lint", "make lint"), ("test", "make test")],
        );
        let after = config(
            Some("make deps"),
            &[
                ("lint", "make lint"),
                ("test", "make test-all"),
                ("audit", "make audit"),
            ],
        );
        assert_eq!(
            config_changes(Some(&before), Some(&after)),
            [
                "the check `audit` added to `.git-intake.toml`",
                "the check `test` given another command in `.git-intake.toml`"
            ]
        );
        // A rename is a removal and an addition.
        let renamed = config(None, &[("style", "make lint"), ("test", "make test")]);
        assert_eq!(
            config_changes(Some(&before), Some(&renamed)),
            [
                "the install command of `.git-intake.toml`",
                "the check `style` added to `.git-intake.toml`",
                "the check `lint` removed from `.git-intake.toml`"
            ]
        );
        let none = config(Some("make deps"), &[]);
        assert_eq!(
            config_changes(Some(&before), Some(&none)),
            ["the checks `lint` and `test` removed from `.git-intake.toml`"]
        );
        // Only the order changed.
        let reordered = config(
            Some("make deps"),
            &[("test", "make test"), ("lint", "make lint")],
        );
        assert_eq!(
            config_changes(Some(&before), Some(&reordered)),
            ["the order of the checks of `.git-intake.toml`"]
        );
        // The same install command and checks: nothing to name.
        assert!(config_changes(Some(&before), Some(&before.clone())).is_empty());
        // A side that cannot be read.
        for (b, a) in [(None, Some(&after)), (Some(&before), None), (None, None)] {
            assert_eq!(
                config_changes(b, a),
                ["the install command and the checks of `.git-intake.toml`"]
            );
        }
    }

    #[test]
    fn update_branches_are_recognised() {
        assert!(!is_earlier_branch("git-intake/update"));
        assert!(is_earlier_branch("git-intake/update-0123abcdef01"));
        assert!(!is_earlier_branch("git-intake/update-"));
        assert!(!is_earlier_branch("git-intake/update-fix"));
        assert!(!is_earlier_branch("feature/git-intake/update"));
    }

    #[test]
    fn pushes_go_only_where_the_branch_is_not_at_the_head() {
        let places = vec![
            Place {
                url: "a".into(),
                at: Some("h".into()),
            },
            Place {
                url: "b".into(),
                at: None,
            },
        ];
        assert_eq!(push_targets("origin", &places, |at| at != Some("h")), ["b"]);
        assert_eq!(
            push_targets("origin", &places, |_| true),
            ["origin".to_string()]
        );
        assert!(push_targets("origin", &places, |_| false).is_empty());
    }
}
