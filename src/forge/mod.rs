//! The forge that hosts the publish remote, GitLab or GitHub, reached
//! through its REST API.
//!
//! Every command uses the [`Forge`] interface only: it finds the update's
//! merge requests, opens one, rewrites its title and description, closes
//! it, and reads whether the project merges only a tree its required
//! pipeline tested. [`gitlab`] and [`github`] implement it over [`http`].

pub mod github;
pub mod gitlab;
pub mod http;
pub mod locate;

use anyhow::{Result, bail};
use url::Url;

use crate::config::Publish;
use crate::git::Git;

pub use locate::{Location, TOKEN_VARIABLES};

/// A kind of forge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    GitLab,
    GitHub,
}

impl Kind {
    /// The forge's name.
    pub fn name(self) -> &'static str {
        match self {
            Kind::GitLab => "GitLab",
            Kind::GitHub => "GitHub",
        }
    }

    /// What the forge calls a merge request.
    pub fn term(self) -> &'static str {
        match self {
            Kind::GitLab => "merge request",
            Kind::GitHub => "pull request",
        }
    }

    /// A merge request's number as the forge writes it, such as `!7` or `#7`.
    pub fn reference(self, number: u64) -> String {
        match self {
            Kind::GitLab => format!("!{number}"),
            Kind::GitHub => format!("#{number}"),
        }
    }
}

/// Where a merge request stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Open,
    Merged,
    /// Closed without merging.
    Closed,
}

impl State {
    pub fn describe(self) -> &'static str {
        match self {
            State::Open => "open",
            State::Merged => "merged",
            State::Closed => "closed without merging",
        }
    }
}

/// A merge request or pull request as the forge reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeRequest {
    pub number: u64,
    /// Its address on the forge.
    pub url: String,
    pub state: State,
    /// Its head commit: the source branch's tip while it is open, and the
    /// final head once it is merged or closed.
    pub head: String,
    pub source: String,
    pub target: String,
    pub description: String,
    /// When it was merged or closed.
    pub ended_at: Option<jiff::Timestamp>,
}

/// A time the forge writes, in RFC 3339, as an instant; `None` when absent
/// or unreadable.
fn instant(value: &serde_json::Value, key: &str) -> Option<jiff::Timestamp> {
    optional_text(value, key).and_then(|t| t.parse().ok())
}

/// Orders merge requests by when they ended, the one that ended most
/// recently first and those that did not end last.
pub fn latest_first(list: &mut [MergeRequest]) {
    list.sort_by_key(|m| std::cmp::Reverse(m.ended_at));
}

/// What the forge's settings mean for an update's merge request.
#[derive(Debug, Clone, Default)]
pub struct Review {
    /// Each setting to change before the forge merges only a tree its
    /// required pipeline tested, or each requirement the token could not
    /// confirm, with the ways to let it.
    pub refusals: Vec<String>,
    /// Merges that stay selectable and drop the update's history.
    pub warnings: Vec<String>,
    /// Where the forge reads its CI definition, relative to the repository
    /// root; a path ending in `/` is a directory.
    pub ci_paths: Vec<String>,
}

/// The requests the tool makes of a forge.
pub trait Forge {
    fn kind(&self) -> Kind;

    fn location(&self) -> &Location;

    /// Reads the project, which tells whether the token is accepted.
    fn check_access(&self) -> Result<()>;

    /// The merge requests from `source`, into `target` when given, newest
    /// first: only the open ones with `open_only`.
    fn merge_requests(
        &self,
        source: &str,
        target: Option<&str>,
        open_only: bool,
    ) -> Result<Vec<MergeRequest>>;

    fn merge_request(&self, number: u64) -> Result<MergeRequest>;

    /// Opens a merge request from `source` into `target`, or returns `None`
    /// when the forge refuses it because one is open already. On GitLab it
    /// asks for squashing off and the source branch removed on merge.
    fn open(
        &self,
        source: &str,
        target: &str,
        title: &str,
        description: &str,
    ) -> Result<Option<MergeRequest>>;

    /// Rewrites a merge request's title and description; on GitLab it asks
    /// again for squashing off.
    fn update(&self, number: u64, title: &str, description: &str) -> Result<MergeRequest>;

    fn close(&self, number: u64) -> Result<MergeRequest>;

    /// Reads the settings that decide how a merge request into `target` is
    /// merged.
    fn review(&self, target: &str) -> Result<Review>;

    /// The longest description the forge takes, in characters.
    fn description_limit(&self) -> usize;
}

/// Reaches the forge of the publish remote that `publish` names, refusing
/// before anything is pushed when it cannot be located or no token is set.
pub fn connect(git: &Git, publish: &Publish) -> Result<Box<dyn Forge>> {
    let url = fetch_url(git, &publish.remote)?;
    let env = |name: &str| std::env::var(name).ok();
    let around = locate::Surroundings {
        env: &env,
        ssh_host: &ssh_host,
    };
    let location = match locate::locate(publish, url.as_deref(), &around) {
        Ok(location) => location,
        Err(why) => bail!("{why}"),
    };
    Ok(match location.kind {
        Kind::GitLab => Box::new(gitlab::GitLab::new(location)),
        Kind::GitHub => Box::new(github::GitHub::new(location)),
    })
}

/// The paths of the forge's CI definition that an update may change, as the
/// tool can tell them without the forge's API, for publication by push:
/// `.github/workflows/` on GitHub, and on GitLab the default CI
/// configuration file `.gitlab-ci.yml`, since a project's own setting is
/// readable only through the API. Empty when the forge is not recognised.
pub fn ci_paths_without_api(git: &Git, publish: &Publish) -> Result<Vec<String>> {
    let url = fetch_url(git, &publish.remote)?;
    let env = |name: &str| std::env::var(name).ok();
    let around = locate::Surroundings {
        env: &env,
        ssh_host: &ssh_host,
    };
    Ok(match locate::kind(publish, url.as_deref(), &around) {
        Some(Kind::GitHub) => vec![github::WORKFLOWS.to_string()],
        Some(Kind::GitLab) => vec![gitlab::DEFAULT_CI_FILE.to_string()],
        None => Vec::new(),
    })
}

/// The URL git fetches a remote from: its configured URL after git's
/// `url.<base>.insteadOf` rewriting, as `git remote get-url` reports it, so
/// that the forge is the one the branch actually lands on. `None` when the
/// remote does not exist.
pub fn fetch_url(git: &Git, remote: &str) -> Result<Option<String>> {
    if git.remote_url(remote)?.is_none() {
        return Ok(None);
    }
    Ok(Some(git.line(&["remote", "get-url", remote])?))
}

/// The host an SSH host alias stands for, as `ssh -G` resolves the client's
/// configuration; `None` without an `ssh` that answers.
fn ssh_host(alias: &str) -> Option<String> {
    crate::identity::ssh_destination(alias).map(|(host, _)| host)
}

/// `base` with `segments` appended, each encoded as one path segment, and
/// the query `query`.
pub fn endpoint(base: &Url, segments: &[&str], query: &[(&str, &str)]) -> Url {
    let mut url = base.clone();
    if let Ok(mut path) = url.path_segments_mut() {
        path.pop_if_empty().extend(segments);
    }
    if !query.is_empty() {
        url.query_pairs_mut().extend_pairs(query);
    }
    url
}

/// A JSON field as text, empty when absent.
fn text(value: &serde_json::Value, key: &str) -> String {
    value
        .get(key)
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// A JSON field as text, `None` when absent or null.
fn optional_text(value: &serde_json::Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoints_encode_each_segment() {
        let base = Url::parse("https://gitlab.example.com/gitlab/api/v4").unwrap();
        let url = endpoint(
            &base,
            &["projects", "group/sub app", "merge_requests"],
            &[("source_branch", "git-intake/update"), ("state", "opened")],
        );
        assert_eq!(
            url.as_str(),
            "https://gitlab.example.com/gitlab/api/v4/projects/group%2Fsub%20app/merge_requests?\
             source_branch=git-intake%2Fupdate&state=opened"
        );
        let base = Url::parse("https://api.github.com").unwrap();
        assert_eq!(
            endpoint(&base, &["repos", "team", "app"], &[]).as_str(),
            "https://api.github.com/repos/team/app"
        );
    }

    #[test]
    fn merge_requests_are_ordered_by_instant_across_offsets() {
        let mr = |number: u64, ended: Option<&str>| MergeRequest {
            number,
            url: String::new(),
            state: State::Closed,
            head: String::new(),
            source: String::new(),
            target: String::new(),
            description: String::new(),
            ended_at: ended.map(|t| t.parse().unwrap()),
        };
        // Across the change to summer time: 03:15+02:00 is 01:15Z, before
        // 01:30Z, though it sorts after it as text; and back to winter
        // time, 02:10+01:00 is 01:10Z, after 02:30+02:00, which is 00:30Z.
        let mut list = vec![
            mr(1, Some("2026-03-29T03:15:00+02:00")),
            mr(2, Some("2026-03-29T01:30:00Z")),
            mr(3, None),
            mr(4, Some("2026-10-25T02:30:00+02:00")),
            mr(5, Some("2026-10-25T02:10:00+01:00")),
        ];
        latest_first(&mut list);
        let order: Vec<u64> = list.iter().map(|m| m.number).collect();
        assert_eq!(order, [5, 4, 2, 1, 3]);
    }

    #[test]
    fn each_forge_has_its_term() {
        assert_eq!(Kind::GitLab.term(), "merge request");
        assert_eq!(Kind::GitHub.term(), "pull request");
        assert_eq!(Kind::GitLab.reference(7), "!7");
        assert_eq!(Kind::GitHub.reference(7), "#7");
    }
}
