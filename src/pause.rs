//! A pause of the project's merge queue, recorded in one reference on the
//! publish remote.
//!
//! `land` takes the queue's assignments away while the update's last
//! pipeline runs. Before it takes anything it records what it takes in
//! `refs/git-intake/pause`, so that any later run, in any clone, can end a
//! pause a dead one left, whichever merge request it was started for. The
//! reference points at a parentless commit that nothing else reaches, whose
//! message holds the record, and it exists exactly while the pause lasts.
//!
//! Reading takes git and no forge API. Writing takes the right to push, and
//! a push with a lease on the value read is an atomic swap, which also keeps
//! two runs apart: the one that loses it touches no assignment.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::commands::merge_request::remote_refs;
use crate::config::{Publish, Upstream};
use crate::forge::{self, Account, Failure, Kind};
use crate::git::{Git, Output, abbrev};
use crate::identity;
use crate::output::esay;
use crate::repo::Repo;
use crate::upstreams::Fetches;

/// The reference on the publish remote that records a pause.
pub const PAUSE_REF: &str = "refs/git-intake/pause";

/// The first line of the record's commit message.
const SUBJECT: &str = "git intake: a pause of the merge queue";

/// A merge request a pause took from the queue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Held {
    pub number: u64,
    /// The assignees it kept besides the queue's account.
    pub others: Vec<u64>,
}

/// The merge request a pause was started for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Started {
    pub number: u64,
    pub url: String,
}

/// A pause of the merge queue whose account is `account`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pause {
    pub account: Account,
    pub merge_request: Started,
    /// In the order they return in.
    pub held: Vec<Held>,
    /// The merge requests the queue was landing when the pause began, which
    /// keep their assignment.
    pub in_flight: Vec<u64>,
}

impl Pause {
    pub fn numbers(&self) -> Vec<u64> {
        self.held.iter().map(|h| h.number).collect()
    }
}

/// A pause as the publish remote holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recorded {
    /// The commit that holds it, the value a swap requires.
    pub oid: String,
    pub pause: Pause,
}

/// What became of a swap of the reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Swap<T> {
    Done(T),
    /// The reference no longer held the value the swap required.
    Lost,
}

/// The reference on the publish remote of a repository.
pub struct Store<'a> {
    repo: &'a Repo,
    remote: &'a str,
    upstreams: &'a [Upstream],
}

impl<'a> Store<'a> {
    pub fn new(repo: &'a Repo, remote: &'a str, upstreams: &'a [Upstream]) -> Self {
        Self {
            repo,
            remote,
            upstreams,
        }
    }

    fn git(&self) -> &Git {
        &self.repo.git
    }

    pub fn remote(&self) -> &str {
        self.remote
    }

    /// What the publish remote's reference holds now, without its record,
    /// which is all a run needs to tell whether it still holds the pause.
    pub fn listed(&self) -> Result<Option<String>> {
        let names = [PAUSE_REF.to_string()];
        Ok(remote_refs(self.git(), self.remote, &names)?.remove(PAUSE_REF))
    }

    /// The pause the publish remote records, if one is on. A record that
    /// cannot be read is an error, never taken for no pause.
    pub fn read(&self) -> Result<Option<Recorded>> {
        if self.listed()?.is_none() {
            return Ok(None);
        }
        let mut fetches = Fetches::new(self.repo);
        let oid = match fetches.remote_ref(self.remote, PAUSE_REF)? {
            Ok(oid) => oid,
            // Ended between the listing and the fetch.
            Err(_) if self.listed()?.is_none() => return Ok(None),
            Err(why) => return Err(Failure::unanswered(why).into()),
        };
        let message = self.git().run(&["log", "-1", "--format=%B", &oid])?;
        let pause = parse(&message).with_context(|| {
            format!(
                "the record of a pause of the merge queue, {PAUSE_REF} at {} on '{}', cannot be read; show it with \
                 `git fetch {} {PAUSE_REF}` and `git log -1 FETCH_HEAD`, give the merge requests it holds back by \
                 hand, then delete it with `git push {} :{PAUSE_REF}`",
                abbrev(&oid),
                self.remote,
                self.remote,
                self.remote
            )
        })?;
        Ok(Some(Recorded { oid, pause }))
    }

    /// Records a pause, when none is recorded.
    pub fn create(&self, pause: &Pause) -> Result<Swap<Recorded>> {
        self.write(None, pause)
    }

    /// Replaces the record `from` with `pause`, when it is still the one
    /// recorded.
    pub fn update(&self, from: &Recorded, pause: &Pause) -> Result<Swap<Recorded>> {
        self.write(Some(&from.oid), pause)
    }

    fn write(&self, from: Option<&str>, pause: &Pause) -> Result<Swap<Recorded>> {
        let oid = self.commit(pause)?;
        Ok(match self.swap(from, Some(&oid))? {
            Swap::Done(()) => Swap::Done(Recorded {
                oid,
                pause: pause.clone(),
            }),
            Swap::Lost => Swap::Lost,
        })
    }

    /// Removes the record `from`, which ends the pause, when it is still the
    /// one recorded.
    pub fn delete(&self, from: &Recorded) -> Result<Swap<()>> {
        self.swap(Some(&from.oid), None)
    }

    /// A commit, reachable from no reference, that holds the record.
    fn commit(&self, pause: &Pause) -> Result<String> {
        let git = self.git();
        let tree = git.run_with_input(&["mktree"], String::new())?;
        let message = format!("{SUBJECT}\n\n{}\n", serde_json::to_string_pretty(pause)?);
        git.line(&[
            "-c",
            "user.name=git-intake",
            "-c",
            "user.email=git-intake@localhost",
            "commit-tree",
            tree.trim(),
            "-m",
            &message,
        ])
    }

    /// Removes the record `from` from where git pushes it, judged by the push
    /// alone: a record that does not read back from where git fetches cannot
    /// be judged by a read from there.
    pub fn withdraw(&self, from: &Recorded) -> Result<()> {
        let out = self.push(Some(&from.oid), None)?;
        if out.success() {
            return Ok(());
        }
        Err(out.into_error().into())
    }

    /// Pushes `to`, or the reference's removal, under a lease on `from`,
    /// where `None` is the reference being absent.
    fn swap(&self, from: Option<&str>, to: Option<&str>) -> Result<Swap<()>> {
        let out = self.push(from, to)?;
        if out.success() {
            return Ok(Swap::Done(()));
        }
        // Why it failed shows in what the reference holds now: an answer lost
        // on the way back leaves it as asked.
        let now = self.listed()?;
        if now.as_deref() == to {
            Ok(Swap::Done(()))
        } else if now.as_deref() == from {
            Err(Failure::unanswered(format!(
                "cannot push {PAUSE_REF} to '{}': {}",
                self.remote,
                out.into_error()
            ))
            .into())
        } else {
            Ok(Swap::Lost)
        }
    }

    fn push(&self, from: Option<&str>, to: Option<&str>) -> Result<Output> {
        let git = self.git();
        identity::Upstreams::new(git, self.upstreams)?.refuse_push(git, self.remote)?;
        let lease = format!(
            "--force-with-lease={PAUSE_REF}:{}",
            from.unwrap_or_default()
        );
        let spec = match to {
            Some(oid) => format!("{oid}:{PAUSE_REF}"),
            None => format!(":{PAUSE_REF}"),
        };
        // A hook of the clone has no say over a record that holds no code.
        git.raw(&["push", "--quiet", "--no-verify", &lease, self.remote, &spec])
    }
}

/// The pause the publish remote of merge-request mode records, from git
/// alone. A forge known to be GitHub has none, so nothing is asked of it.
pub fn look(repo: &Repo, publish: &Publish, upstreams: &[Upstream]) -> Result<Option<Recorded>> {
    if forge::kind_of_remote(&repo.git, publish)? == Some(Kind::GitHub) {
        return Ok(None);
    }
    Store::new(repo, &publish.remote, upstreams).read()
}

/// What to say of a pause that stands, for `status`, `doctor`, `sync` and
/// `start`.
pub fn described(remote: &str, pause: &Pause) -> String {
    let kind = Kind::GitLab;
    format!(
        "a pause of the merge queue of @{} is recorded on '{remote}', started for the merge request {} ({}){}; \
         `git intake land` carries it on, and `git intake land --resume` ends it",
        pause.account.username,
        kind.reference(pause.merge_request.number),
        pause.merge_request.url,
        match pause.held.is_empty() {
            true => String::new(),
            false => format!(", which holds {}", listed(&pause.numbers(), kind)),
        }
    )
}

/// Warns, without stopping anything, of a pause that stands.
pub fn warn_if_on(repo: &Repo, publish: &Publish, upstreams: &[Upstream]) {
    match look(repo, publish, upstreams) {
        Ok(None) => {}
        Ok(Some(r)) => esay!("warning: {}", described(&publish.remote, &r.pause)),
        Err(err) => esay!(
            "warning: cannot tell whether the merge queue is paused: {}",
            crate::redact::redacted(&format!("{err:#}"))
        ),
    }
}

/// The note, for people, that says a pause began. It decides nothing.
pub fn note(pause: &Pause, kind: Kind) -> String {
    let named = listed(&pause.numbers(), kind);
    format!(
        "git intake paused the merge queue of `@{}` while this {} lands. {}{}",
        pause.account.username,
        kind.term(),
        if named.is_empty() {
            "No merge request was waiting in it; one assigned to it meanwhile is held too, and gets its assignment \
             back once the pause ends."
                .to_string()
        } else {
            format!(
                "These merge requests lose their assignment and get it back in this order once the pause ends: \
                 {named}."
            )
        },
        match pause.in_flight.is_empty() {
            true => String::new(),
            false => format!(
                " The queue is landing {}, which keeps its assignment.",
                listed(&pause.in_flight, kind)
            ),
        }
    )
}

/// The note, for people, that says merge requests joined a pause.
pub fn joined_note(pause: &Pause, joined: &[u64], kind: Kind) -> String {
    format!(
        "git intake paused {} too, assigned to the merge queue of `@{}` during the pause. They get their assignment \
         back in this order once the pause ends: {}.",
        listed(joined, kind),
        pause.account.username,
        listed(&pause.numbers(), kind)
    )
}

/// The note, for people, that says a pause ended: the merge requests
/// assigned to the queue again, and those left as they were because they
/// moved on.
pub fn ended_note(pause: &Pause, returned: &[u64], left: &[u64], kind: Kind) -> String {
    let mut out = format!(
        "git intake ended the pause of the merge queue of `@{}`.",
        pause.account.username
    );
    if !returned.is_empty() {
        out.push_str(&format!(
            " Assigned to it again, in this order: {}.",
            listed(returned, kind)
        ));
    }
    if !left.is_empty() {
        out.push_str(&format!(
            " Left as they are, since they were merged, closed, deleted, assigned again or given other assignees meanwhile: {}.",
            listed(left, kind)
        ));
    }
    out
}

pub fn listed(numbers: &[u64], kind: Kind) -> String {
    numbers
        .iter()
        .map(|n| kind.reference(*n))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The pause a commit message records.
fn parse(message: &str) -> Result<Pause> {
    let Some((SUBJECT, body)) = message.split_once("\n\n") else {
        bail!("it does not start with \"{SUBJECT}\"");
    };
    let pause: Pause = serde_json::from_str(body)?;
    let numbers = pause.numbers();
    if (1..numbers.len()).any(|i| numbers[..i].contains(&numbers[i])) {
        bail!("it names a merge request twice");
    }
    Ok(pause)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pause() -> Pause {
        Pause {
            account: Account {
                id: 42,
                username: "merge-bot".into(),
            },
            merge_request: Started {
                number: 3,
                url: "https://gitlab.example.com/g/app/-/merge_requests/3".into(),
            },
            held: vec![
                Held {
                    number: 7,
                    others: Vec::new(),
                },
                Held {
                    number: 5,
                    others: vec![3, 4],
                },
            ],
            in_flight: vec![9],
        }
    }

    #[test]
    fn a_record_is_read_back_and_a_broken_one_refused() {
        let message = format!(
            "{SUBJECT}\n\n{}\n",
            serde_json::to_string_pretty(&pause()).unwrap()
        );
        assert_eq!(parse(&message).unwrap(), pause());
        for broken in [
            "someone's commit\n\n{}\n",
            "no body",
            "git intake: a pause of the merge queue\n\nnot json\n",
            "git intake: a pause of the merge queue\n\n{\"account\": 1}\n",
        ] {
            assert!(parse(broken).is_err(), "{broken}");
        }
        let mut twice = pause();
        twice.held[1].number = 7;
        let message = format!("{SUBJECT}\n\n{}", serde_json::to_string(&twice).unwrap());
        let err = parse(&message).unwrap_err().to_string();
        assert!(err.contains("names a merge request twice"), "{err}");
    }

    #[test]
    fn notes_for_people_name_the_merge_requests_and_hold_no_record() {
        let text = note(&pause(), Kind::GitLab);
        assert!(
            text.contains("in this order once the pause ends: !7, !5."),
            "{text}"
        );
        assert!(text.contains("The queue is landing !9"), "{text}");
        assert!(!text.contains("<!--"), "{text}");
        let ended = ended_note(&pause(), &[7], &[5], Kind::GitLab);
        assert!(
            ended.contains("in this order: !7. Left as they are"),
            "{ended}"
        );
        assert!(!ended.contains("<!--"), "{ended}");
    }
}
