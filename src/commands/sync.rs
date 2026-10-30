//! `sync`: takes the repository's follow upstreams in without local state,
//! as a scheduled CI job runs it, and as a person runs it to take over
//! where a job stopped.
//!
//! Each run derives everything from the publish remote, its forge and the
//! upstreams. In merge-request mode, with the update's merge request open,
//! it catches the merge request up when the integration branch moved past
//! its head, and otherwise changes nothing. Without one, it plans an update
//! of every follow upstream that is behind and publishes it as a merge
//! request, unless the merge request closed or merged most recently was
//! closed without merging and recorded every target it would merge. In push
//! mode it starts, verifies and publishes in one run.
//!
//! A conflict, or failed checks in push mode, stops it with the update in
//! progress, for a person to resolve with `continue` and `finish`. Any
//! other end leaves no update it created, so that the next run meets the
//! same situation.

use anyhow::{Context, Result, bail};

use crate::Exit;
use crate::commands::catch_up::{self, Advice};
use crate::commands::merge_request::{
    self, Branch, Publication, Published, classify, connect, delete_branch, fetch_integration,
    merged_without_history, named, publish, refuse_elsewhere, remote_branch, requests,
    require_publishable, unknown_branch,
};
use crate::commands::update::{
    self, Advanced, Fetched, Queued, advance, begin, plural, queue_follow, refuse_leftovers,
    require_published, step_name,
};
use crate::commands::{env, finish, verify};
use crate::config::{Config, Mode, Publish, PublishVia};
use crate::description::{self, Targets};
use crate::forge::{Forge, MergeRequest, State};
use crate::git::{Git, abbrev, branch_ref};
use crate::port::{self, Standing, Verb};
use crate::repo::Repo;
use crate::state::{
    self, CarriedMergeRequest, CopyStep, FollowMerge, MergeKind, PlannedCommit, PortPlan,
    QueuedUpstream, UPDATE_BRANCH, UpdateState, UpstreamStep, Work,
};
use crate::sync_record;
use crate::tags::UpstreamTags;
use crate::update_line::{self, History};
use crate::upstreams::{self, Fetches};

pub fn run(repo: &Repo) -> Result<Exit> {
    if let Some(state) = repo.state()? {
        merge_request::refuse_earlier(&state)?;
        let wt = Git::worktree(&state.worktree);
        bail!(
            "an update is in progress, at the step {}; next: {}. `git intake sync` starts nothing while it lasts",
            step_name(&state, &wt)?.replace('_', " "),
            update::next_step(&state, &wt)?
        );
    }
    let (config, publish) = prepare(repo)?;
    match publish.via {
        PublishVia::Push => by_push(repo, config),
        PublishVia::MergeRequest => by_merge_request(repo, config, publish),
    }
}

/// Brings the local integration branch up to date with the publish
/// remote's, as `start` does, creating it when it is missing, and reads the
/// configuration again at its tip.
fn prepare(repo: &Repo) -> Result<(Config, Publish)> {
    let head = repo.head().context(
        "HEAD names no commit; run `git intake sync` in a checkout of the integration branch",
    )?;
    let at_head = repo.config_at(head)?.with_context(|| {
        format!(
            "no {} is committed at HEAD; run `git intake sync` in a checkout of the integration branch",
            crate::config::FILE_NAME
        )
    })?;
    let integration = at_head.integration.clone();
    let publish = at_head.publish.clone().with_context(|| {
        format!(
            "`git intake sync` publishes what it takes in, but the {} at HEAD has no [publish] setting; add one \
             naming the publish remote",
            crate::config::FILE_NAME
        )
    })?;
    let git = &repo.git;
    if git.commit(&branch_ref(&integration))?.is_none() {
        let tip = fetch_integration(git, &publish.remote, &integration)?;
        git.run(&[
            "update-ref",
            "-m",
            "git-intake: sync",
            &branch_ref(&integration),
            &tip,
            "",
        ])?;
        println!(
            "created '{integration}' at {} from '{}/{integration}'",
            abbrev(&tip),
            publish.remote
        );
    } else {
        repo.require_clean(&integration)?;
        finish::follow_published(repo, &publish.remote, &integration)?;
    }
    let config = repo.load_config()?.config;
    let reread = config.publish.clone().with_context(|| {
        format!(
            "the {} at the tip of '{integration}' has no [publish] setting; `git intake sync` publishes what it \
             takes in",
            crate::config::FILE_NAME
        )
    })?;
    if reread.remote != publish.remote {
        bail!(
            "the {} at the tip of '{integration}' names '{}' as the publish remote instead of '{}'; run \
             `git intake sync` again to use that configuration",
            crate::config::FILE_NAME,
            reread.remote,
            publish.remote
        );
    }
    Ok((config, reread))
}

/// The follow upstreams an update would take in, against the integration
/// tip `base`, and what it leaves out.
struct Plan {
    queue: Vec<QueuedUpstream>,
    /// Follow upstreams it cannot take, each with the reason.
    left_out: Vec<String>,
}

impl Plan {
    fn targets(&self) -> Targets {
        self.queue
            .iter()
            .filter_map(|q| q.follow().map(|f| (q.name.clone(), f.target.clone())))
            .collect()
    }

    /// The exit status of a run that went as far as it could.
    fn exit(&self) -> Exit {
        if self.left_out.is_empty() {
            Exit::Success
        } else {
            Exit::Negative
        }
    }
}

/// Fetches every upstream and plans an update of each follow upstream that
/// is behind, as `start` would with no upstream named. An upstream for
/// which `start` would refuse, were it the only one selected, is left out
/// and named; a port upstream is never taken, and named when it has pending
/// commits.
fn plan(repo: &Repo, config: &Config, base: &str, fetches: &mut Fetches) -> Result<Plan> {
    let git = &repo.git;
    let mut tips = Vec::new();
    let mut fetched = Vec::new();
    for up in &config.upstreams {
        let tip = fetches.upstream(up)?;
        if let Ok(t) = &tip {
            tips.push(t.clone());
        }
        fetched.push(tip);
    }
    let latest = sync_record::latest(git, &sync_record::records(git, base, &tips)?)?;
    let mut plan = Plan {
        queue: Vec::new(),
        left_out: Vec::new(),
    };
    for (up, tip) in config.upstreams.iter().zip(&fetched) {
        let tip = match tip {
            Ok(tip) => tip,
            Err(why) => {
                match up.mode {
                    Mode::Follow => plan
                        .left_out
                        .push(format!("upstream '{}' cannot be fetched: {why}", up.name)),
                    Mode::Port => println!(
                        "{}: port upstream not taken by sync, and it cannot be fetched: {why}",
                        up.name
                    ),
                }
                continue;
            }
        };
        match up.mode {
            Mode::Port => match port::standing(git, base, up, latest.get(&up.name), tip, &tips) {
                Ok(Standing::Known(a)) => {
                    let n = a.pending().len() as u64;
                    if n > 0 {
                        println!(
                            "{}: port upstream with {n} pending commit{}; sync never takes a port upstream, \
                                 `git intake start {}` plans them for review",
                            up.name,
                            plural(n),
                            up.name
                        );
                    }
                }
                Ok(Standing::Rewritten(rewrite)) => println!(
                    "{}: port upstream not taken by sync, and it was rewritten: {}",
                    up.name,
                    rewrite.describe(&up.name)
                ),
                Ok(Standing::NoBase) => println!(
                    "{}: port upstream not taken by sync: {}",
                    up.name,
                    port::no_base_advice(&up.name)
                ),
                Err(err) => println!("{}: port upstream not taken by sync: {err:#}", up.name),
            },
            Mode::Follow => {
                let tags = match UpstreamTags::list(git, up)? {
                    Ok(tags) => tags,
                    Err(why) => {
                        plan.left_out.push(why);
                        continue;
                    }
                };
                match queue_follow(
                    repo,
                    base,
                    up,
                    latest.get(&up.name),
                    Fetched { tip, tags: &tags },
                    None,
                )? {
                    Queued::Work(q) => plan.queue.push(*q),
                    Queued::Current(message) => println!("{message}"),
                    Queued::Refused(why) => plan.left_out.push(why),
                }
            }
        }
    }
    for why in &plan.left_out {
        println!("left out: {why}");
    }
    Ok(plan)
}

/// Says that nothing is behind.
fn nothing_behind(plan: &Plan) {
    if plan.left_out.is_empty() {
        println!("every follow upstream is current; nothing to update");
    } else {
        println!("every follow upstream sync can take is current; nothing to update");
    }
}

/// `sync` in push mode: starts, verifies and publishes in one run.
fn by_push(repo: &Repo, config: Config) -> Result<Exit> {
    let integration = config.integration.clone();
    let base = repo
        .git
        .commit(&branch_ref(&integration))?
        .with_context(|| format!("the integration branch '{integration}' does not exist"))?;
    let mut fetches = Fetches::new(repo);
    let plan = plan(repo, &config, &base, &mut fetches)?;
    if plan.queue.is_empty() {
        nothing_behind(&plan);
        return Ok(plan.exit());
    }
    let worktree = repo.place_update_worktree()?;
    refuse_leftovers(repo, &worktree)?;
    let exit = plan.exit();
    let mut state = begin(repo, config, &base, worktree, plan.queue)?;
    drop(fetches);
    let verified = (|| -> Result<Option<Exit>> {
        match advance(repo, &mut state)? {
            Advanced::Merged => {}
            Advanced::Plan | Advanced::Stopped(_) => return Ok(Some(stopped_on_conflict())),
        }
        let head = Git::worktree(&state.worktree).line(&["rev-parse", "HEAD"])?;
        if !verify::verify_head(repo, &mut state, &head)? {
            println!(
                "the checks failed; nothing was published, and the update stays in progress, unverified, in {}. \
                 Fix it there and run `git intake verify` and `git intake finish`, or run `git intake abort`",
                state.worktree.display()
            );
            return Ok(Some(Exit::Negative));
        }
        Ok(None)
    })();
    match verified {
        Err(err) => {
            discard(repo, &state)?;
            Err(err)
        }
        Ok(Some(stop)) => Ok(stop),
        Ok(None) => Ok(match finish::run(repo, false)? {
            Exit::Success => exit,
            other => other,
        }),
    }
}

/// What a stop on conflicts says beyond what the merge printed.
fn stopped_on_conflict() -> Exit {
    println!(
        "sync stopped with the update in progress and nothing published; resolve the conflicts, then run \
         `git intake continue` and `git intake finish`"
    );
    Exit::Negative
}

/// `sync` in merge-request mode.
fn by_merge_request(repo: &Repo, config: Config, setting: Publish) -> Result<Exit> {
    let git = &repo.git;
    let integration = config.integration.clone();
    let remote = setting.remote.clone();
    require_published(repo, &remote, &integration)?;
    let forge = connect(repo, &setting)?;
    let kind = forge.kind();
    let term = kind.term();
    let tip = git
        .commit(&branch_ref(&integration))?
        .with_context(|| format!("the integration branch '{integration}' does not exist"))?;
    let reqs = requests(repo, forge.as_ref(), &remote, &integration, &[])?;
    refuse_elsewhere(kind, &reqs, &integration)?;
    let mut fetches = Fetches::new(repo);
    if let Some(open) = reqs.open.clone() {
        return sync_open(
            repo,
            config,
            forge.as_ref(),
            &remote,
            open,
            &tip,
            &mut fetches,
        );
    }
    let plan = plan(repo, &config, &tip, &mut fetches)?;
    let declined: Option<(MergeRequest, Targets)> = reqs
        .latest_ended()
        .filter(|m| m.state == State::Closed)
        .and_then(|m| description::targets(&m.description).map(|t| (m.clone(), t)));
    if let Some(at) = remote_branch(repo, &mut fetches, &remote, UPDATE_BRANCH)? {
        match classify(git, &config, &at, &tip, &reqs.ended, None)? {
            Branch::Leftover | Branch::Contained => {
                println!(
                    "the branch '{UPDATE_BRANCH}' on '{remote}' holds nothing but what landed or what a closed or \
                     merged {term} held; deleting it"
                );
                if delete_branch(git, &remote, UPDATE_BRANCH, &at)? {
                    bail!(
                        "the branch '{UPDATE_BRANCH}' remains on '{remote}'; nothing was pushed. Run `git intake sync` again"
                    );
                }
            }
            Branch::Update(history) => {
                if let Some((mr, targets)) = &declined
                    && !history.follows.is_empty()
                    && history
                        .follows
                        .iter()
                        .all(|f| targets.get(&f.name) == Some(&f.target))
                {
                    bail!(
                        "the branch '{UPDATE_BRANCH}' on '{remote}' holds an update of the targets the {} declined \
                         when it was closed without merging, perhaps with a fix pushed since; sync does not publish \
                         it again. Nothing was pushed. Run `git intake abort --merge-request` to delete the branch, \
                         after fetching what should be kept, or open a {term} from it by hand",
                        named(kind, mr)
                    );
                }
                println!(
                    "the branch '{UPDATE_BRANCH}' on '{remote}' at {} holds an update no {term} carries; carrying it on",
                    abbrev(&at)
                );
                let exit = carry_and_publish(
                    repo,
                    config,
                    forge.as_ref(),
                    &remote,
                    &at,
                    &history,
                    &tip,
                    None,
                )?;
                return Ok(worse(exit, plan.exit()));
            }
            Branch::Unknown => bail!("{}", unknown_branch(git, kind, &remote, &at)?),
        }
    }
    if plan.queue.is_empty() {
        nothing_behind(&plan);
        return Ok(plan.exit());
    }
    let planned = plan.targets();
    if let Some((mr, targets)) = &declined
        && planned.iter().all(|(n, t)| targets.get(n) == Some(t))
    {
        println!(
            "the {} was closed without merging, which declined these targets: {}. Nothing was started; the next \
             update comes when an upstream moves on, or by hand with `git intake start` and `git intake finish`",
            named(kind, mr),
            planned
                .iter()
                .map(|(n, t)| format!("{n} at {}", abbrev(t)))
                .collect::<Vec<_>>()
                .join(", ")
        );
        return Ok(plan.exit());
    }
    let mut retry_of = None;
    if let Some(merged) = reqs.latest_ended().filter(|m| m.state == State::Merged)
        && let Some(recorded) = description::targets(&merged.description)
        && !recorded.is_empty()
        && recorded.iter().all(|(n, t)| planned.get(n) == Some(t))
        && merged_without_history(git, merged, &tip)?
    {
        eprintln!(
            "warning: the {} was merged without the update's history, which a squash or rebase drops, so its sync \
             records never landed; sync proposes the same targets again, and merging the new {term} with its \
             history records them",
            named(kind, merged)
        );
        retry_of = Some(merged.clone());
    }
    let exit = plan.exit();
    let worktree = repo.place_update_worktree()?;
    refuse_leftovers(repo, &worktree)?;
    let mut state = begin(repo, config, &tip, worktree, plan.queue)?;
    drop(fetches);
    let published = (|| -> Result<Exit> {
        match advance(repo, &mut state)? {
            Advanced::Merged => {}
            Advanced::Plan | Advanced::Stopped(_) => return Ok(stopped_on_conflict()),
        }
        let head = require_publishable(&state)?;
        let publication = Publication {
            forge: forge.as_ref(),
            remote: &remote,
            branch: UPDATE_BRANCH,
            open: None,
            retry_of: retry_of.as_ref(),
        };
        let outcome = publish(repo, &mut state, &head, publication, None)?;
        conclude(repo, &state, outcome)
    })();
    match published {
        Err(err) => {
            discard(repo, &state)?;
            Err(err)
        }
        Ok(Exit::Success) => Ok(exit),
        Ok(other) => Ok(other),
    }
}

/// The worse of two exit statuses.
fn worse(a: Exit, b: Exit) -> Exit {
    match (a, b) {
        (Exit::Success, other) => other,
        (other, _) => other,
    }
}

/// Ends the local update once the merge request carries it, or after a
/// guard stopped the publication.
fn conclude(repo: &Repo, state: &UpdateState, published: Published) -> Result<Exit> {
    match published {
        Published::Done(_) => {
            finish::close(repo, state.clone(), Vec::new())?;
            Ok(Exit::Success)
        }
        Published::Stopped(stop) => {
            println!("{}", stop.message);
            discard(repo, state)?;
            Ok(stop.exit)
        }
    }
}

/// `sync` with the update's merge request open.
fn sync_open(
    repo: &Repo,
    config: Config,
    forge: &dyn Forge,
    remote: &str,
    open: MergeRequest,
    tip: &str,
    fetches: &mut Fetches,
) -> Result<Exit> {
    let git = &repo.git;
    let kind = forge.kind();
    let term = kind.term();
    let Some(head) = remote_branch(repo, fetches, remote, &open.source)? else {
        bail!("{}", merge_request::branch_gone(kind, &open, remote));
    };
    let base = git.merge_base(&head, tip)?;
    let history = match &base {
        Some(base) => update_line::describe(git, &config, &head, base)?,
        None => History::default(),
    };
    let rewritten = match description::targets(&open.description) {
        Some(recorded) => recorded
            .iter()
            .any(|(name, target)| history.follow(name).is_none_or(|f| f.target != *target)),
        None => !history.carries_update(),
    };
    if rewritten {
        bail!(
            "the {} was rewritten: its head {} lacks the sync records of the targets it records, as a rebase \
             drops them, so merging it would not record what it takes in. sync does not touch it, and nothing was \
             pushed. Withdraw it with `git intake abort --merge-request`, which declines its targets, so that sync \
             proposes them again only once an upstream moves on; `git intake start` and `git intake finish` \
             publish them again by hand",
            named(kind, &open),
            abbrev(&head)
        );
    }
    if git.is_ancestor(tip, &head)? {
        println!(
            "the update waits for the {} at {}, which contains '{}' at {}; nothing changed",
            named(kind, &open),
            abbrev(&head),
            config.integration,
            abbrev(tip)
        );
        let plan = plan(repo, &config, tip, fetches)?;
        for q in &plan.queue {
            if let Some(f) = q.follow()
                && !git.is_ancestor(&f.target, &head)?
            {
                println!(
                    "{}: moved on to {} since the {term} was opened; the next update takes it",
                    q.name,
                    abbrev(&f.target)
                );
            }
        }
        return Ok(Exit::Success);
    }
    carry_and_publish(
        repo,
        config,
        forge,
        remote,
        &head,
        &history,
        tip,
        Some(&open),
    )
}

/// Carries on the update the branch at `head` holds, catches it up with the
/// integration tip `tip` when that moved past it, without running the
/// checks, and publishes it to the merge request `open`, or opens one.
#[allow(clippy::too_many_arguments)]
fn carry_and_publish(
    repo: &Repo,
    config: Config,
    forge: &dyn Forge,
    remote: &str,
    head: &str,
    history: &History,
    tip: &str,
    open: Option<&MergeRequest>,
) -> Result<Exit> {
    let git = &repo.git;
    let kind = forge.kind();
    let term = kind.term();
    let base = git.merge_base(head, tip)?.with_context(|| {
        format!(
            "{} shares no history with the integration branch",
            abbrev(head)
        )
    })?;
    let worktree = repo.place_update_worktree()?;
    refuse_leftovers(repo, &worktree)?;
    let branch = open.map_or(UPDATE_BRANCH.to_string(), |m| m.source.clone());
    let carried = open.map(|m| CarriedMergeRequest {
        number: m.number,
        url: m.url.clone(),
        branch: m.source.clone(),
    });
    let mut state = carry_on(repo, config, history, head, &base, tip, worktree, carried)?;
    let request = open
        .map(|m| named(kind, m))
        .unwrap_or_else(|| format!("{term} to open"));
    let result = (|| -> Result<Exit> {
        if !git.is_ancestor(tip, head)? {
            catch_up::require_possible(repo, &state, tip, Advice::Sync(&request), true)?;
            if let Some(exit) = catch_up::run(repo, &mut state, tip)? {
                println!(
                    "sync stopped on the catch-up merge with the update in progress and nothing pushed{}; resolve \
                     the conflicts, then run `git intake continue` and `git intake finish`",
                    open.map(|m| format!(": the {} keeps its head", named(kind, m)))
                        .unwrap_or_default()
                );
                return Ok(exit);
            }
        }
        let published_head = require_publishable(&state)?;
        let publication = Publication {
            forge,
            remote,
            branch: &branch,
            open,
            retry_of: None,
        };
        let outcome = match open {
            Some(open) => {
                let mut guard = merge_request::guard(repo, forge, remote, open, head, tip);
                publish(
                    repo,
                    &mut state,
                    &published_head,
                    publication,
                    Some(&mut guard),
                )?
            }
            None => publish(repo, &mut state, &published_head, publication, None)?,
        };
        conclude(repo, &state, outcome)
    })();
    match result {
        Err(err) => {
            discard(repo, &state)?;
            Err(err)
        }
        ok => ok,
    }
}

/// Removes an update `sync` created, so that the next run meets the same
/// situation and not an update in progress.
fn discard(repo: &Repo, state: &UpdateState) -> Result<()> {
    repo.remove_update_artifacts()?;
    repo.remove_plan()?;
    upstreams::release(&repo.git, state)?;
    repo.remove_logs()?;
    state::remove(&repo.paths.state)?;
    Ok(())
}

/// Carries the update the branch at `head` holds on locally: the update
/// branch at `head` in the update worktree, the base at its merge base with
/// the integration tip, and each upstream whose sync merge, copies or
/// journal change is on the update line queued as merged or carried out.
#[allow(clippy::too_many_arguments)]
fn carry_on(
    repo: &Repo,
    config: Config,
    history: &History,
    head: &str,
    base: &str,
    tip: &str,
    worktree: std::path::PathBuf,
    carried: Option<CarriedMergeRequest>,
) -> Result<UpdateState> {
    let git = &repo.git;
    let queue = queued(repo, &config, history, base)?;
    repo.exclude_update_worktree()?;
    git.run(&["config", "rerere.enabled", "true"])?;
    std::fs::create_dir_all(&repo.paths.root)
        .with_context(|| format!("cannot create {}", repo.paths.root.display()))?;
    git.run(&[
        "worktree",
        "add",
        "--quiet",
        "-b",
        UPDATE_BRANCH,
        &worktree.to_string_lossy(),
        head,
    ])?;
    let mut state = UpdateState::new(config, base.to_string(), worktree, queue);
    state.main_worktree = repo.main_checkout()?;
    state.remote_tip = Some(tip.to_string());
    state.carried_from = carried;
    repo.save_state(&state)?;
    upstreams::hold(git, &state)?;
    let names: Vec<&str> = state.upstreams.iter().map(|q| q.name.as_str()).collect();
    println!(
        "carried on the update at {} in {}: {}",
        abbrev(head),
        state.worktree.display(),
        names.join(", ")
    );
    Ok(state)
}

/// The queued upstreams of an update carried on from history.
fn queued(
    repo: &Repo,
    config: &Config,
    history: &History,
    base: &str,
) -> Result<Vec<QueuedUpstream>> {
    let git = &repo.git;
    let described = env::described(git, config, history, base)?;
    let mut queue = Vec::new();
    for d in described {
        let Some(up) = config.upstream(&d.name) else {
            continue;
        };
        if let Some(f) = history.follow(&d.name) {
            let kind = match (&up.path, &d.previous) {
                (None, _) => MergeKind::Root,
                (Some(_), Some(_)) => MergeKind::Subtree,
                (Some(_), None) => MergeKind::Import,
            };
            queue.push(QueuedUpstream {
                name: d.name.clone(),
                fetched: f.target.clone(),
                rev: None,
                step: UpstreamStep::Merged,
                work: Work::Follow(FollowMerge {
                    commits: git.count(base, &f.target)?,
                    target: f.target.clone(),
                    tag: f.tag.clone(),
                    previous: d.previous.clone(),
                    kind,
                    head_before: None,
                    merge_commit: Some(f.merge.clone()),
                    conflicts: Vec::new(),
                }),
            });
        } else if let Some(p) = history.ports.iter().find(|p| p.name == d.name) {
            let recorded: Vec<String> = p
                .copies
                .iter()
                .map(|c| c.taken.clone())
                .chain(p.decisions.iter().map(|d| d.commit.clone()))
                .collect();
            let order = update_line::ordered(git, &recorded)?;
            let mut commits = Vec::new();
            let mut takes = Vec::new();
            for commit in &order {
                let planned = |verb: Verb, subject: String| PlannedCommit {
                    commit: commit.clone(),
                    subject,
                    merge: false,
                    verb: Some(verb),
                    note: None,
                    copy: CopyStep::None,
                    head_before: None,
                    copied: None,
                    conflicts: Vec::new(),
                    failure: None,
                    left_out: Vec::new(),
                    overlaps: Vec::new(),
                };
                if let Some(c) = p.copies.iter().find(|c| c.taken == *commit) {
                    let mut entry = planned(Verb::Take, c.subject.clone());
                    entry.copy = CopyStep::Copied;
                    entry.copied = Some(c.commit.clone());
                    takes.push(commits.len());
                    commits.push(entry);
                } else if let Some(decision) = p.decisions.iter().find(|x| x.commit == *commit) {
                    let verb = match decision.verdict {
                        port::Verdict::Skip => Verb::Skip,
                        port::Verdict::Have => Verb::Have,
                    };
                    let subject = git
                        .line(&["log", "-1", "--format=%s", commit, "--"])
                        .unwrap_or_default();
                    let mut entry = planned(verb, subject);
                    entry.note = decision.note.clone();
                    commits.push(entry);
                }
            }
            let journal = port::read_journal(git, base, &d.name)?;
            let journal_base = match &journal {
                Some(j) => j.base.clone(),
                None => port::read_journal(git, &history.line[0], &d.name)?
                    .map(|j| j.base)
                    .unwrap_or_default(),
            };
            queue.push(QueuedUpstream {
                name: d.name.clone(),
                fetched: d.target.clone().unwrap_or_default(),
                rev: None,
                step: UpstreamStep::Merged,
                work: Work::Port(PortPlan {
                    plan: repo.paths.plan.clone(),
                    base: journal_base,
                    journal_exists: journal.is_some(),
                    left_out: 0,
                    commits,
                    takes,
                    journal_commit: p.journal_commit.clone(),
                    journal_head_before: None,
                }),
            });
        }
    }
    Ok(queue)
}
