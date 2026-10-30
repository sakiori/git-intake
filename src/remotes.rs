//! Upstream remotes: adding missing ones from the configuration, refusing
//! mismatched ones, and fetching.

use anyhow::{Result, bail};

use crate::config::Upstream;
use crate::git::{Git, tracking_ref};
use crate::repo::bullet;

/// What has to happen before an upstream's remote can be fetched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteCheck {
    /// The remote exists with the configured URL, or no URL is configured.
    Ready,
    /// The remote is missing and will be added from the configured URL.
    Add(String),
    /// The remote cannot be used; the message names the problem.
    Refused(String),
}

/// Checks one upstream's remote without changing anything.
pub fn check(git: &Git, up: &Upstream) -> Result<RemoteCheck> {
    let actual = git.remote_url(&up.remote)?;
    Ok(match (actual, &up.url) {
        (Some(actual), Some(configured)) if actual != *configured => RemoteCheck::Refused(format!(
            "upstream '{}': remote '{}' points to {actual}, but the configuration says {configured}; \
             the tool does not rewrite existing remotes",
            up.name, up.remote
        )),
        (Some(_), _) => RemoteCheck::Ready,
        (None, Some(url)) => RemoteCheck::Add(url.clone()),
        (None, None) => RemoteCheck::Refused(format!(
            "upstream '{}': remote '{}' does not exist and no url is configured; add it with `git remote add {} <url>`",
            up.name, up.remote, up.remote
        )),
    })
}

/// Makes one upstream's remote usable: adds it when missing, or returns the
/// refusal message.
pub fn ensure_one(git: &Git, up: &Upstream) -> Result<std::result::Result<bool, String>> {
    match check(git, up)? {
        RemoteCheck::Ready => Ok(Ok(false)),
        RemoteCheck::Add(url) => {
            git.run(&["remote", "add", &up.remote, &url])?;
            Ok(Ok(true))
        }
        RemoteCheck::Refused(why) => Ok(Err(why)),
    }
}

/// Checks every upstream's remote first and refuses if any is unusable;
/// otherwise adds the missing ones. Returns the upstreams whose remote was added.
pub fn ensure_all(git: &Git, ups: &[&Upstream]) -> Result<Vec<String>> {
    let mut refusals = Vec::new();
    for up in ups {
        if let RemoteCheck::Refused(why) = check(git, up)? {
            refusals.push(why);
        }
    }
    if !refusals.is_empty() {
        bail!(
            "cannot use the remotes of the selected upstreams:\n{}",
            bullet(&refusals)
        );
    }
    let mut added = Vec::new();
    for up in ups {
        if let Ok(true) = ensure_one(git, up)? {
            added.push(up.name.clone());
        }
    }
    Ok(added)
}

/// Fetches one upstream's remote. Returns the git error text on failure.
pub fn fetch(git: &Git, up: &Upstream) -> Result<std::result::Result<(), String>> {
    let out = git.raw(&["fetch", "--quiet", "--prune", "--no-tags", &up.remote])?;
    if !out.success() {
        return Ok(Err(out.into_error().to_string()));
    }
    if git.commit(&tracking_ref(&up.remote, &up.branch))?.is_none() {
        return Ok(Err(format!(
            "remote '{}' has no branch '{}'",
            up.remote, up.branch
        )));
    }
    Ok(Ok(()))
}
