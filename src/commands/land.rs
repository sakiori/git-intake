//! `land`: carries the update's open merge request to the integration
//! branch.
//!
//! Each round reads the merge request with the pipeline of its head. While
//! that runs, `land` waits. A failed pipeline stops it. A passed one is
//! merged at its head when the head contains the integration tip; when the
//! integration branch moved past it, `land` catches the merge request up as
//! `sync` does and waits for the new pipeline. With a merge queue named in
//! `[publish]`, the queue is paused before that catch-up, so that its merges
//! cannot move the branch again while the last pipeline runs, and the pause
//! ends on every way out. What a pause holds is recorded in a reference on
//! the publish remote (see [`crate::pause`]).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use signal_hook::consts::{SIGHUP, SIGINT, SIGTERM};
use signal_hook::flag;

use crate::Exit;
use crate::commands::merge_request::{
    self, branch_gone, connect, named, refuse_elsewhere, requests,
};
use crate::commands::sync::{self, Caller};
use crate::commands::update::require_published;
use crate::config::{Config, Publish, PublishVia};
use crate::drift;
use crate::forge::{
    Account, Failure, Forge, Kind, Merge, MergeRequest, PipelineState, Queued, State, may_pass,
    not_found, patiently,
};
use crate::git::{Git, abbrev, branch_ref};
use crate::identity;
use crate::output::{esay, say};
use crate::pause::{self, Held, PAUSE_REF, Pause, Recorded, Started, Store, Swap, listed};
use crate::repo::Repo;
use crate::repo::bullet;
use crate::upstreams::Fetches;

/// How `land` waits.
pub struct Waits {
    /// How long it waits in all before it gives up.
    pub timeout: Duration,
    /// How long it waits between two reads of the forge.
    pub poll: Duration,
}

/// The least time between two assignments that end a pause, so that a queue
/// ordering by assignment time keeps the recorded order.
const RETURN_GAP: Duration = Duration::from_secs(1);

/// How many rounds in a row may fail on what may pass, a 5xx or a network
/// error, before `land` gives up its wait.
const FAILED_POLLS: u32 = 10;

/// How many times a step outside the rounds of the wait is tried, as each
/// step of ending a pause is.
const END_ATTEMPTS: u32 = 5;

/// The longest pause between two attempts at such a step.
const END_RETRY_GAP: Duration = Duration::from_secs(15);

/// How many rounds `land` waits for a merge request in flight whose pipeline
/// passed, which a queue should take at once, before it holds that too.
const PASSED_ROUNDS: u32 = 5;

/// How many times a merge the forge refuses while it reads the merge request
/// as mergeable is tried again.
const CONTESTED_MERGES: u32 = 5;

/// Waits between two returns. A test records the wait instead of taking it;
/// builds without debug assertions ignore that.
fn wait_between_returns() {
    #[cfg(debug_assertions)]
    if let Ok(file) = std::env::var("GIT_INTAKE_TEST_RETURN_GAPS") {
        use std::io::Write;
        if let Ok(mut gaps) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(file)
        {
            let _ = writeln!(gaps, "{}", RETURN_GAP.as_millis());
        }
        return;
    }
    std::thread::sleep(RETURN_GAP);
}

/// How often a wait looks for a signal.
const SIGNAL_CHECK: Duration = Duration::from_millis(200);

/// SIGINT, SIGTERM and SIGHUP during a run. The first asks it to end its
/// pause and exit, which the waits and the rounds notice; a second SIGINT or
/// SIGTERM ends the process at once, which leaves a pause for `--resume`. A
/// terminal that hangs up can deliver SIGHUP twice, so a second one counts for
/// nothing. A signal the process was started ignoring, as `nohup` starts it
/// for SIGHUP, stays ignored.
struct Signals {
    caught: Arc<AtomicUsize>,
    hung_up: Arc<AtomicBool>,
}

impl Signals {
    fn watch() -> Result<Self> {
        let caught = Arc::new(AtomicUsize::new(0));
        let hung_up = Arc::new(AtomicBool::new(false));
        let armed = Arc::new(AtomicBool::new(false));
        for signal in [SIGINT, SIGTERM] {
            if ignored(signal) {
                continue;
            }
            // In this order, so that the first delivery finds it unarmed.
            flag::register_conditional_shutdown(signal, 128 + signal, Arc::clone(&armed))?;
            flag::register(signal, Arc::clone(&armed))?;
            flag::register_usize(signal, Arc::clone(&caught), signal as usize)?;
        }
        if !ignored(SIGHUP) {
            flag::register(SIGHUP, Arc::clone(&hung_up))?;
        }
        Ok(Self { caught, hung_up })
    }

    /// Stops the run once a signal came.
    fn check(&self) -> Result<()> {
        match self.caught.load(Ordering::Relaxed) {
            0 if self.hung_up.load(Ordering::Relaxed) => Err(Interrupted(SIGHUP as u8).into()),
            0 => Ok(()),
            signal => Err(Interrupted(signal as u8).into()),
        }
    }
}

/// Whether the process was started with `signal` ignored.
fn ignored(signal: i32) -> bool {
    // SAFETY: `sigaction` with a null new action only reads the current one
    // into the zeroed struct it is given.
    unsafe {
        let mut current: libc::sigaction = std::mem::zeroed();
        libc::sigaction(signal, std::ptr::null(), &mut current) == 0
            && current.sa_sigaction == libc::SIG_IGN
    }
}

/// The error that stops a run a signal reached.
#[derive(Debug)]
struct Interrupted(u8);

impl std::fmt::Display for Interrupted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "`git intake land` was interrupted by signal {}", self.0)
    }
}

impl std::error::Error for Interrupted {}

/// What a run says when the record of a pause is no longer what it read.
fn another_run() -> String {
    format!(
        "{PAUSE_REF} on the publish remote changed under this run, most likely through another `git intake land`; \
         this run touched no assignment after that and stops. Look at `git intake status`: `git intake land` carries \
         the pause on, and `git intake land --resume` ends it"
    )
}

pub fn run(repo: &Repo, resume: bool, waits: Waits) -> Result<Exit> {
    if resume {
        return end_left_behind(repo, &waits);
    }
    let signals = Signals::watch()?;
    if let Some(state) = repo.state()? {
        let wt = Git::worktree(&state.worktree);
        bail!(
            "an update is in progress, at the step {}; next: {}, then `git intake land`{}",
            crate::commands::update::step_name(&state, &wt)?.replace('_', " "),
            crate::commands::update::next_step(&state, &wt)?,
            match sync::configured(repo).and_then(|(config, publish)| pause::look(
                repo,
                &publish,
                &config.upstreams
            )) {
                Ok(Some(_)) =>
                    ". A pause of the merge queue is recorded; `git intake land --resume` ends it now",
                _ => "",
            }
        );
    }
    let (config, publish) = attempt(&waits, || sync::prepare(repo))?;
    refuse_push_mode(&config, &publish)?;
    // A catch-up push needs a pipeline, and one made with the job token
    // starts none, so the queue would stand until the timeout.
    if let Err(err) = crate::forge::refuse_job_token_push(&repo.git, &publish.remote) {
        bail!(
            "{err:#}{}",
            match pause::look(repo, &publish, &config.upstreams) {
                Ok(Some(_)) => format!(
                    ". A pause of the merge queue is recorded; `git intake land --resume`, run where git can push \
                     to '{}', ends it",
                    publish.remote
                ),
                _ => String::new(),
            }
        );
    }
    let forge = connect(repo, &publish)?;
    let kind = forge.kind();
    let term = kind.term();
    let reqs = attempt(&waits, || {
        requests(
            repo,
            forge.as_ref(),
            &publish.remote,
            &config.integration,
            &[],
        )
    })?;
    let recorded = attempt(&waits, || pause::look(repo, &publish, &config.upstreams))?;
    let Some(open) = reqs.open.clone() else {
        if let Some(r) = recorded {
            say!(
                "the update has no open {term}, and a pause of the merge queue is still on; ending it"
            );
            give_back(
                forge.as_ref(),
                &Store::new(repo, &publish.remote, &config.upstreams),
                r,
                &waits,
            )?;
        }
        say!(
            "the update has no open {term} into '{}'; nothing to land",
            config.integration
        );
        return Ok(Exit::Success);
    };
    refuse_elsewhere(kind, &reqs, &config.integration)?;
    // A pause keeps the account it was started with, whatever `queue` says
    // by now, so that the same account is taken and returned.
    let account = match (&recorded, &publish.queue) {
        (Some(r), _) => Some(r.pause.account.clone()),
        (None, None) => None,
        (None, Some(name)) => Some(attempt(&waits, || forge.account(name))?.with_context(|| {
            format!(
                "publish queue: {} knows no account '{name}'; name the account of the merge queue",
                kind.name()
            )
        })?),
    };
    let mut lander = Lander {
        repo,
        forge: forge.as_ref(),
        config,
        publish,
        number: open.number,
        account,
        pause: recorded,
        checked: false,
        accepted: None,
        contested: 0,
        passed_rounds: 0,
        waiting: None,
        started: Instant::now(),
        waits,
        signals,
    };
    if let Some(r) = &lander.pause {
        say!(
            "carrying on the pause of the merge queue started for the {} {}: {}",
            kind.term(),
            kind.reference(r.pause.merge_request.number),
            listed(&r.pause.numbers(), kind)
        );
    }
    let landed = lander.land();
    let ended = match lander.pause.take() {
        Some(r) => end_pause(forge.as_ref(), &lander.store(), r, &lander.waits),
        None => Ok(()),
    };
    const RESUME: &str = "run `git intake land --resume` to end it";
    match (landed, ended) {
        (Ok(exit), Ok(())) => Ok(exit),
        (Err(err), Ok(())) if err.is::<Interrupted>() => {
            let signal = err.downcast_ref::<Interrupted>().map_or(0, |i| i.0);
            say!("{err}; the merge request stays as it is, and `git intake land` carries it on");
            Ok(Exit::Code(128 + signal))
        }
        (Err(err), Ok(())) => Err(err),
        (Ok(_), Err(end)) => Err(end.context(format!(
            "the pause of the merge queue could not be ended; {RESUME}"
        ))),
        (Err(err), Err(end)) => Err(anyhow::anyhow!(
            "{err:#}; and the pause of the merge queue could not be ended: {end:#}; {RESUME}"
        )),
    }
}

/// `land --resume`: ends the pause the publish remote records, whatever the
/// clone is doing, and nothing else.
fn end_left_behind(repo: &Repo, waits: &Waits) -> Result<Exit> {
    let (config, publish) = sync::configured(repo)?;
    refuse_push_mode(&config, &publish)?;
    // Ending a pause needs no pipeline, and a refused push keeps the record.
    crate::forge::warn_of_job_token_push(&repo.git, &publish.remote)?;
    let Some(recorded) = attempt(waits, || pause::look(repo, &publish, &config.upstreams))? else {
        say!(
            "no pause of the merge queue is recorded on '{}'; nothing changed",
            publish.remote
        );
        return Ok(Exit::Success);
    };
    let forge = connect(repo, &publish)?;
    let store = Store::new(repo, &publish.remote, &config.upstreams);
    give_back(forge.as_ref(), &store, recorded, waits)?;
    Ok(Exit::Success)
}

fn refuse_push_mode(config: &Config, publish: &Publish) -> Result<()> {
    if publish.via != PublishVia::MergeRequest {
        bail!(
            "`git intake land` carries the update's merge request to '{}', and with `via = \"push\"` the update has \
             none: `finish` and `sync` publish it",
            config.integration
        );
    }
    Ok(())
}

/// One run of `land` on the update's open merge request.
struct Lander<'a> {
    repo: &'a Repo,
    forge: &'a dyn Forge,
    config: Config,
    publish: Publish,
    number: u64,
    /// The merge queue's account, with `queue` set.
    account: Option<Account>,
    /// The pause on, if any, as this run last read or wrote it.
    pause: Option<Recorded>,
    /// Whether the forge's merge settings were read.
    checked: bool,
    /// The head the forge took to merge.
    accepted: Option<String>,
    /// How many merges the forge refused though it read as mergeable.
    contested: u32,
    /// How many rounds in a row no pipeline ran of the merge requests in
    /// flight.
    passed_rounds: u32,
    /// What `land` last said it waits for, so that it says it once.
    waiting: Option<String>,
    started: Instant,
    waits: Waits,
    signals: Signals,
}

impl Lander<'_> {
    fn store(&self) -> Store<'_> {
        Store::new(self.repo, &self.publish.remote, &self.config.upstreams)
    }

    fn land(&mut self) -> Result<Exit> {
        let mut failed = 0;
        loop {
            self.signals.check()?;
            let round = self.round();
            // The terminal's SIGINT reaches git's processes too, whose failure
            // then says nothing of its own.
            if !matches!(round, Ok(None | Some(Exit::Success))) {
                self.signals.check()?;
            }
            match round {
                Ok(Some(exit)) => return Ok(exit),
                Ok(None) => failed = 0,
                Err(err) if may_pass(&err) => {
                    failed += 1;
                    if failed == FAILED_POLLS {
                        return Err(err.context(format!(
                            "gave up after {FAILED_POLLS} failed rounds in a row"
                        )));
                    }
                    say!("{}; reading again", shown(&err));
                    self.wait("the forge to answer")?;
                }
                Err(err) => return Err(err),
            }
        }
    }

    /// One read of the merge request and what follows from it; `None` when
    /// `land` is to read it again.
    fn round(&mut self) -> Result<Option<Exit>> {
        let kind = self.forge.kind();
        let (mr, pipeline) = self.forge.with_pipeline(self.number)?;
        match (mr.state, &self.accepted) {
            (State::Merged, Some(head)) => {
                say!(
                    "merged the {} at {} into '{}'",
                    named(kind, &mr),
                    abbrev(head),
                    mr.target
                );
                if kind == Kind::GitHub {
                    self.remove_branch(&mr, head);
                }
                return Ok(Some(Exit::Success));
            }
            (State::Merged, None) => {
                say!("the {} is merged into '{}'", named(kind, &mr), mr.target);
                return Ok(Some(Exit::Success));
            }
            (State::Closed, Some(_)) => bail!(
                "{} took the {} to merge, and it was closed without merging",
                kind.name(),
                named(kind, &mr)
            ),
            (State::Closed, None) => {
                say!(
                    "the {} was closed without merging; nothing to land",
                    named(kind, &mr)
                );
                return Ok(Some(Exit::Success));
            }
            (State::Open, Some(_)) => {
                self.wait(&format!(
                    "{} to merge the {}",
                    kind.name(),
                    named(kind, &mr)
                ))?;
                return Ok(None);
            }
            (State::Open, None) => {}
        }
        if !self.checked {
            self.check_settings()?;
            self.checked = true;
        }
        if self.pause.is_some() {
            self.hold_newcomers()?;
        }
        let state = pipeline.as_ref().map(|p| p.state);
        match state {
            None => {
                self.wait(&format!("a pipeline of the head {}", abbrev(&mr.head)))?;
                return Ok(None);
            }
            Some(PipelineState::Running) => {
                let url = pipeline.map(|p| p.url).unwrap_or_default();
                self.wait(&format!(
                    "the pipeline of the head {} ({url})",
                    abbrev(&mr.head)
                ))?;
                return Ok(None);
            }
            Some(PipelineState::Failed | PipelineState::Canceled) => {
                let url = pipeline.map(|p| p.url).unwrap_or_default();
                say!(
                    "the pipeline of the head {} of the {} {} ({url}); nothing was merged. Push a fix to the \
                     branch '{}', then run `git intake land` again",
                    abbrev(&mr.head),
                    named(kind, &mr),
                    if state == Some(PipelineState::Failed) {
                        "failed"
                    } else {
                        "was canceled"
                    },
                    mr.source
                );
                return Ok(Some(Exit::Negative));
            }
            Some(PipelineState::Passed) => {}
        }
        let (head, tip) = self.positions(&mr)?;
        let Some(head) = head else {
            // A merge removes the branch; only an open merge request is
            // left without it.
            if self.forge.merge_request(self.number)?.state == State::Open {
                bail!("{}", branch_gone(kind, &mr, &self.publish.remote));
            }
            return Ok(None);
        };
        if head != mr.head {
            // The forge has not seen the latest push yet.
            self.wait(&format!("the forge to read the head {}", abbrev(&head)))?;
            return Ok(None);
        }
        let git = &self.repo.git;
        if git.is_ancestor(&tip, &head)? {
            match self.forge.merge(self.number, &head)? {
                Merge::Accepted => {
                    self.accepted = Some(head);
                    return Ok(None);
                }
                Merge::HeadMoved | Merge::NotYet => {
                    self.wait(&format!(
                        "{} to let the {} be merged",
                        kind.name(),
                        named(kind, &mr)
                    ))?;
                    return Ok(None);
                }
                Merge::Contested(_) if self.contested < CONTESTED_MERGES => {
                    self.contested += 1;
                    self.wait(&format!(
                        "{} to let the {} be merged",
                        kind.name(),
                        named(kind, &mr)
                    ))?;
                    return Ok(None);
                }
                Merge::Contested(why) | Merge::Refused(why) => {
                    let (_, now) = self.positions(&mr)?;
                    if now != tip || self.forge.merge_request(self.number)?.state != State::Open {
                        return Ok(None);
                    }
                    bail!(
                        "{} refused to merge the {}: {why}",
                        kind.name(),
                        named(kind, &mr)
                    );
                }
            }
        }
        // What refuses a catch-up refuses it here, with the queue still at work.
        let (config, _) = sync::prepare(self.repo)?;
        self.config = config.clone();
        require_published(self.repo, &self.publish.remote, &config.integration)?;
        if self.account.is_some() && self.pause.is_none() {
            if let Some(status) = self.forge.merge_blocker(self.number)? {
                bail!(
                    "the {} cannot merge whatever `land` does: its merge status is `{status}`. The merge queue was \
                     not paused, and nothing changed. Run `git intake land` again once that is settled",
                    named(kind, &mr)
                );
            }
            let conflicts = drift::predict_conflicts(git, &tip, &head, None)?;
            if conflicts.is_empty() {
                self.start_pause(&mr)?;
            } else {
                say!(
                    "the catch-up merge will conflict in {} file{}; the merge queue is not paused for it",
                    conflicts.len(),
                    if conflicts.len() == 1 { "" } else { "s" }
                );
            }
        }
        if self.lands_meanwhile()? {
            return Ok(None);
        }
        say!(
            "'{}' moved to {} past the head {}; catching the {} up",
            self.config.integration,
            abbrev(&tip),
            abbrev(&head),
            named(kind, &mr)
        );
        let mut fetches = Fetches::new(self.repo);
        let local = git
            .commit(&branch_ref(&config.integration))?
            .with_context(|| {
                format!(
                    "the integration branch '{}' does not exist",
                    config.integration
                )
            })?;
        match sync::sync_open(
            self.repo,
            config,
            self.forge,
            &self.publish.remote,
            mr.clone(),
            &local,
            &mut fetches,
            Caller::Land,
        )? {
            Exit::Success => {
                self.waiting = None;
                Ok(None)
            }
            // The only stop of a catch-up with this status: the merge request
            // or the branch changed just before the push, which pushed nothing.
            Exit::Code(2) => {
                self.wait(&format!(
                    "'{}' and the {} to settle",
                    self.config.integration,
                    named(kind, &mr)
                ))?;
                Ok(None)
            }
            other => {
                say!(
                    "once the catch-up merge is resolved and published, `git intake land` carries the {} on",
                    named(kind, &mr)
                );
                Ok(Some(other))
            }
        }
    }

    /// Removes the branch of a merge the forge does not remove it with, under
    /// a lease on the head merged. What stays is for the next `sync` to remove,
    /// so failing here is only a warning.
    fn remove_branch(&self, mr: &MergeRequest, head: &str) {
        let git = &self.repo.git;
        let removed = identity::Upstreams::new(git, &self.config.upstreams).and_then(|upstreams| {
            merge_request::delete_branch(git, &self.publish.remote, &mr.source, head, &upstreams)
        });
        if let Err(err) = removed {
            esay!(
                "warning: the branch '{}' could not be removed: {}",
                mr.source,
                shown(&err)
            );
        }
    }

    /// Refuses, as `finish` does, a project whose settings let a tree its
    /// required pipeline did not test be merged.
    fn check_settings(&self) -> Result<()> {
        let review = self.forge.review(&self.config.integration)?;
        if !review.refusals.is_empty() {
            let kind = self.forge.kind();
            bail!(
                "the {} project '{}' does not guarantee that only a tree its required pipeline tested is merged, \
                 so the update is not landed. Nothing was merged. Change these settings:\n{}",
                kind.name(),
                self.forge.location().project,
                bullet(&review.refusals)
            );
        }
        Ok(())
    }

    /// The merge request's head as its branch on the publish remote holds
    /// it, `None` without the branch, and the publish remote's integration
    /// tip, both fetched.
    fn positions(&self, mr: &MergeRequest) -> Result<(Option<String>, String)> {
        let git = &self.repo.git;
        let remote = &self.publish.remote;
        let tip = merge_request::fetch_integration(git, remote, &self.config.integration)?;
        let mut fetches = Fetches::new(self.repo);
        let head = merge_request::remote_branch(self.repo, &mut fetches, remote, &mr.source)?;
        Ok((head, tip))
    }

    /// Waits once more, unless that would pass the timeout.
    fn wait(&mut self, what: &str) -> Result<()> {
        if self.started.elapsed() + self.waits.poll >= self.waits.timeout {
            bail!(
                "gave up after {} waiting for {what}",
                seconds(self.waits.timeout)
            );
        }
        if self.waiting.as_deref() != Some(what) {
            say!("waiting for {what}");
            self.waiting = Some(what.to_string());
        }
        let until = Instant::now() + self.waits.poll;
        loop {
            self.signals.check()?;
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Ok(());
            }
            std::thread::sleep(left.min(SIGNAL_CHECK));
        }
    }

    fn account(&self) -> &Account {
        self.account
            .as_ref()
            .expect("a pause needs the queue's account")
    }

    /// The queue's merge requests other than the update's, in the order of
    /// their latest assignment to the account, those without one last.
    fn queue(&self, leave_out: &[u64]) -> Result<Vec<Queued>> {
        let account = self.account();
        let mut queued = Vec::new();
        for number in self.forge.assigned(&self.config.integration, account)? {
            if number != self.number && !leave_out.contains(&number) {
                let read = self
                    .forge
                    .queued(number)
                    .and_then(|q| Ok((self.forge.assigned_at(number, account)?, q)));
                match read {
                    Ok(entry) => queued.push(entry),
                    Err(err) if not_found(&err) => {}
                    Err(err) => return Err(err),
                }
            }
        }
        queued.sort_by_key(|(at, q)| (at.is_none(), *at, q.number));
        Ok(queued.into_iter().map(|(_, q)| q).collect())
    }

    /// Pauses the merge queue: records its merge requests on the publish
    /// remote, then takes the account's assignment from each but the one it
    /// is landing.
    fn start_pause(&mut self, mr: &MergeRequest) -> Result<()> {
        let kind = self.forge.kind();
        let queued = self.queue(&[])?;
        let (landing, held): (Vec<Queued>, Vec<Queued>) = queued.into_iter().partition(landing);
        let pause = Pause {
            account: self.account().clone(),
            merge_request: Started {
                number: mr.number,
                url: mr.url.clone(),
            },
            held: held.iter().map(|q| self.held(q)).collect(),
            in_flight: landing.iter().map(|q| q.number).collect(),
        };
        let recorded = match self.store().create(&pause)? {
            Swap::Done(recorded) => recorded,
            Swap::Lost => bail!(
                "{PAUSE_REF} appeared on the publish remote while this run was about to pause the merge queue: \
                 another `git intake land` paused it, or this run's own write went through unseen. This run touched \
                 no assignment and stops; `git intake status` shows the pause, and `git intake land --resume` ends it"
            ),
        };
        self.read_back(&recorded)?;
        self.pause = Some(recorded);
        write_note(self.forge, self.number, &pause::note(&pause, kind));
        say!(
            "paused the merge queue of @{}{}",
            pause.account.username,
            match held.is_empty() {
                true => String::new(),
                false => format!(": {}", listed(&pause.numbers(), kind)),
            }
        );
        if !pause.in_flight.is_empty() {
            say!(
                "the queue is landing {}; it keeps its assignment",
                listed(&pause.in_flight, kind)
            );
        }
        self.take_assignments(&held)
    }

    /// Stops the run, removing the record it wrote, when that does not read
    /// back from where every run reads it, as when the remote pushes to
    /// another repository than it fetches from; a mirror that lags gets the
    /// attempts of a step to catch up. A record that cannot be read at all
    /// stays, for `--resume` to end: a failed read tells nothing, so one
    /// answer with another value decides, whatever the attempts after it.
    fn read_back(&self, recorded: &Recorded) -> Result<()> {
        let store = self.store();
        let elsewhere = format!(
            "{PAUSE_REF} does not read back from '{}' as this run pushed it",
            self.publish.remote
        );
        let mut answered = false;
        let seen = attempt(&self.waits, || {
            let listed = store.listed()?;
            answered = true;
            match listed {
                Some(oid) if oid == recorded.oid => Ok(()),
                _ => Err(Failure::unanswered(elsewhere.clone()).into()),
            }
        });
        let Err(err) = seen else {
            return Ok(());
        };
        if !answered {
            bail!(
                "{}: this run recorded the pause on '{}' but cannot read the record back, so it touched no \
                 assignment and stops; `git intake land --resume` ends the pause once git reaches the remote again",
                shown(&err),
                self.publish.remote
            );
        }
        let removed = match store.withdraw(recorded) {
            Ok(()) => String::new(),
            Err(why) => format!(
                " It could not be removed from where it went ({}), so look for it there; `git intake land --resume` \
                 reads only where git fetches from",
                shown(&why)
            ),
        };
        bail!(
            "{elsewhere}: git pushes '{}' elsewhere than it fetches it from, through a `pushurl` or a rewriting of \
             the push URL, or fetches it from a mirror that lags. No run would find the pause, so this run touched \
             no assignment and stops; make the remote push where it fetches.{removed}",
            self.publish.remote
        )
    }

    /// Swaps the record this run holds for `next`, or stops the run when it
    /// no longer holds it.
    fn rewrite(&mut self, next: &Pause) -> Result<()> {
        let current = self.pause.clone().expect("a pause on");
        match self.store().update(&current, next)? {
            Swap::Done(recorded) => {
                self.pause = Some(recorded);
                Ok(())
            }
            Swap::Lost => {
                self.pause = None;
                bail!("{}", another_run())
            }
        }
    }

    /// Reads the record again, which tells whether this run still holds the
    /// pause, and pauses the merge requests assigned to the queue meanwhile,
    /// after the ones it holds.
    fn hold_newcomers(&mut self) -> Result<()> {
        let kind = self.forge.kind();
        let current = self.pause.clone().expect("a pause on");
        match self.store().listed()? {
            None => {
                self.pause = None;
                self.account = None;
                say!(
                    "the pause of the merge queue was ended elsewhere, most likely by `git intake land --resume`; \
                     this run holds no merge request any more"
                );
                return Ok(());
            }
            Some(oid) if oid != current.oid => {
                self.pause = None;
                bail!("{}", another_run());
            }
            Some(_) => {}
        }
        let newcomers = self.queue(&current.pause.in_flight)?;
        if newcomers.is_empty() {
            return Ok(());
        }
        let mut next = current.pause.clone();
        let mut joined = Vec::new();
        for q in &newcomers {
            if !next.held.iter().any(|h| h.number == q.number) {
                next.held.push(self.held(q));
                joined.push(q.number);
            }
        }
        // Even with nothing to add: taking one more assignment needs the
        // record to be this run's at that moment.
        self.rewrite(&next)?;
        if !joined.is_empty() {
            write_note(
                self.forge,
                self.number,
                &pause::joined_note(&next, &joined, kind),
            );
            say!(
                "paused {}, assigned to the merge queue during the pause",
                listed(&joined, kind)
            );
        }
        self.take_assignments(&newcomers)
    }

    fn held(&self, q: &Queued) -> Held {
        let account = self.account().id;
        Held {
            number: q.number,
            others: q
                .assignees
                .iter()
                .copied()
                .filter(|a| *a != account)
                .collect(),
        }
    }

    fn take_assignments(&self, queued: &[Queued]) -> Result<()> {
        for q in queued {
            match self.forge.assign(q.number, &self.held(q).others) {
                Err(err) if !not_found(&err) => return Err(err),
                _ => {}
            }
        }
        Ok(())
    }

    /// Whether a merge request the queue was landing when the pause began is
    /// still landing; `land` waits for it before it catches up. That is one
    /// pipeline at most: once one has merged or stopped being current the
    /// others cannot be next, so they are held, and so is one whose pipeline
    /// passed that the queue does not take.
    fn lands_meanwhile(&mut self) -> Result<bool> {
        let Some(current) = self.pause.clone() else {
            return Ok(false);
        };
        if current.pause.in_flight.is_empty() {
            return Ok(false);
        }
        let kind = self.forge.kind();
        let account = self.account().clone();
        let mut open = Vec::new();
        let mut settled = false;
        for &number in &current.pause.in_flight {
            let q = match self.forge.queued(number) {
                Ok(q) => q,
                Err(err) if not_found(&err) => continue,
                Err(err) => return Err(err),
            };
            settled |= q.state == State::Merged;
            if q.state == State::Open && q.assignees.contains(&account.id) {
                settled |= !q.current;
                open.push(q);
            }
        }
        let running = open
            .iter()
            .any(|q| q.pipeline == Some(PipelineState::Running));
        self.passed_rounds = if running { 0 } else { self.passed_rounds + 1 };
        let done_waiting = settled || self.passed_rounds > PASSED_ROUNDS;
        let (hold, still): (Vec<Queued>, Vec<Queued>) =
            open.into_iter().partition(|q| done_waiting || !landing(q));
        let mut next = current.pause.clone();
        next.in_flight = still.iter().map(|q| q.number).collect();
        next.held.extend(hold.iter().map(|q| self.held(q)));
        if next != current.pause {
            self.rewrite(&next)?;
        }
        if !hold.is_empty() {
            let numbers: Vec<u64> = hold.iter().map(|q| q.number).collect();
            write_note(
                self.forge,
                self.number,
                &pause::joined_note(&next, &numbers, kind),
            );
            say!(
                "paused {}, which the merge queue is not landing",
                listed(&numbers, kind)
            );
            self.take_assignments(&hold)?;
        }
        if next.in_flight.is_empty() {
            return Ok(false);
        }
        self.wait(&format!(
            "the merge queue to land {}",
            listed(&next.in_flight, kind)
        ))?;
        Ok(true)
    }
}

/// Whether the queue is landing a merge request it holds: its head contains
/// the integration tip and its pipeline runs, waits or passed.
fn landing(q: &Queued) -> bool {
    q.current
        && matches!(
            q.pipeline,
            Some(PipelineState::Running | PipelineState::Passed)
        )
}

/// A note for people on merge request `number`. It decides nothing, so one
/// that cannot be written is only a warning.
fn write_note(forge: &dyn Forge, number: u64, body: &str) {
    if let Err(err) = forge.comment(number, body) {
        esay!(
            "warning: the note on the {} {} could not be written: {}",
            forge.kind().term(),
            forge.kind().reference(number),
            crate::redact::redacted(&format!("{err:#}"))
        );
    }
}

/// An error as messages show it.
fn shown(err: &anyhow::Error) -> String {
    crate::redact::redacted(&format!("{err:#}"))
}

/// Runs a step that has no round of the wait to try it again, as those of
/// ending a pause, a few times while it fails on what may pass.
fn attempt<T>(waits: &Waits, step: impl FnMut() -> Result<T>) -> Result<T> {
    patiently(END_ATTEMPTS, waits.poll.min(END_RETRY_GAP), step)
}

/// Ends the pause this run held, unless it no longer is the one recorded.
fn end_pause(forge: &dyn Forge, store: &Store, recorded: Recorded, waits: &Waits) -> Result<()> {
    match attempt(waits, || store.listed()) {
        Ok(None) => {
            say!("the pause of the merge queue was ended elsewhere; nothing more to return");
            Ok(())
        }
        Ok(Some(oid)) if oid != recorded.oid => bail!("{}", another_run()),
        Ok(Some(_)) => give_back(forge, store, recorded, waits),
        // Without git the queue still gets its merge requests back from the
        // forge, and the record, which cannot go either, stays for `--resume`.
        Err(unread) if may_pass(&unread) => {
            say!(
                "{}; giving the merge requests back all the same",
                shown(&unread)
            );
            give_back(forge, store, recorded, waits)
        }
        Err(err) => Err(err),
    }
}

/// Ends a pause: assigns the account again to each paused merge request
/// still open and assigned as the pause left it, in the recorded order and
/// at least a second apart, then removes the record, and says so in a note.
/// The record goes last, so that a run killed before it can be resumed.
fn give_back(forge: &dyn Forge, store: &Store, recorded: Recorded, waits: &Waits) -> Result<()> {
    let kind = forge.kind();
    let pause = &recorded.pause;
    let mut returned = Vec::new();
    let mut left = Vec::new();
    // A 404 tells a deleted merge request only while the token sees the
    // project: a project it cannot see answers 404 to everything.
    let mut readable = false;
    let mut deleted = |err: anyhow::Error, number: u64| -> Result<()> {
        if !not_found(&err) {
            return Err(err);
        }
        if !readable {
            attempt(waits, || forge.check_access()).with_context(|| {
                format!(
                    "{} answers that the {} {} does not exist, and it does not show the project '{}' to the token \
                     either, so the pause stays recorded for `git intake land --resume` with a token that reads it",
                    kind.name(),
                    kind.term(),
                    kind.reference(number),
                    forge.location().project
                )
            })?;
            readable = true;
        }
        Ok(())
    };
    for h in &pause.held {
        let q = match attempt(waits, || forge.queued(h.number)) {
            Ok(q) => q,
            Err(err) => {
                deleted(err, h.number)?;
                left.push(h.number);
                continue;
            }
        };
        let mut now = q.assignees.clone();
        now.sort_unstable();
        let mut kept = h.others.clone();
        kept.sort_unstable();
        if q.state != State::Open || now != kept {
            left.push(h.number);
            continue;
        }
        if !returned.is_empty() {
            wait_between_returns();
        }
        let mut assignees = h.others.clone();
        assignees.push(pause.account.id);
        match attempt(waits, || forge.assign(h.number, &assignees)) {
            Ok(()) => returned.push(h.number),
            Err(err) => {
                deleted(err, h.number)?;
                left.push(h.number);
            }
        }
    }
    let removed = attempt(waits, || store.delete(&recorded)).with_context(|| {
        format!(
            "{}, but the record of the pause stays on '{}', since it could not be removed; `git intake land \
             --resume`, run where git can push to '{}', removes it",
            match returned.is_empty() {
                true => "no merge request went back to the merge queue".to_string(),
                false => format!("assigned to the merge queue again: {}", listed(&returned, kind)),
            },
            store.remote(),
            store.remote()
        )
    })?;
    if removed == Swap::Lost {
        bail!("{}", another_run());
    }
    write_note(
        forge,
        pause.merge_request.number,
        &pause::ended_note(pause, &returned, &left, kind),
    );
    say!(
        "ended the pause of the merge queue of @{}{}{}",
        pause.account.username,
        match returned.is_empty() {
            true => String::new(),
            false => format!("; assigned to it again: {}", listed(&returned, kind)),
        },
        match left.is_empty() {
            true => String::new(),
            false => format!(
                "; left as they are, merged, closed, deleted, assigned again or given other assignees meanwhile: {}",
                listed(&left, kind)
            ),
        }
    );
    Ok(())
}

fn seconds(d: Duration) -> String {
    let s = d.as_secs();
    match (s / 3600, (s / 60) % 60, s % 60) {
        (0, 0, s) => format!("{s}s"),
        (0, m, 0) => format!("{m}m"),
        (h, 0, 0) => format!("{h}h"),
        _ => format!("{s}s"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_returns_of_a_pause_lie_a_second_apart() {
        assert!(RETURN_GAP >= Duration::from_secs(1));
    }
}
