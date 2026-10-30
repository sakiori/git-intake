//! Upstreams fetched by URL into references of the tool's own.
//!
//! An upstream is fetched by `git fetch --no-tags <url>`, in the main
//! worktree so that a relative URL keeps its meaning. No remote is added or
//! changed. Each command fetches into a namespace of its own,
//! `refs/git-intake/read/<token>/`, through [`Fetches`], and removes it when
//! it ends. An update holds `refs/git-intake/upstreams/<name>` for each of
//! its queued upstreams: `start` and `sync` write them, from what they
//! fetched or from the update a merge request carries, and only `finish`,
//! `abort` and `sync` remove them, so no other command ever moves them.

use std::collections::HashMap;

use anyhow::Result;

use crate::config::Upstream;
use crate::git::{Git, lines};
use crate::repo::Repo;
use crate::state::UpdateState;

/// Prefix of every reference the tool creates.
pub const REFS_PREFIX: &str = "refs/git-intake/";

/// Prefix of the references an update holds for its upstreams.
const UPSTREAM_PREFIX: &str = "refs/git-intake/upstreams/";

/// Prefix of the namespaces commands fetch into.
const READ_PREFIX: &str = "refs/git-intake/read/";

/// The reference an update holds for an upstream.
pub fn upstream_ref(name: &str) -> String {
    format!("{UPSTREAM_PREFIX}{name}")
}

/// Writes the references the update in `state` holds, each at the tip
/// `start` fetched, which keeps the update's targets and planned commits in
/// the repository until the update ends.
pub fn hold(git: &Git, state: &UpdateState) -> Result<()> {
    for (name, tip) in held_by(state) {
        // An update carried on from a merge request may name upstream
        // commits the repository does not have, as the taken commits of a
        // port: there is nothing to keep.
        if git.object_exists(&tip)? {
            git.run(&["update-ref", "-m", "git-intake: start", &name, &tip])?;
        }
    }
    Ok(())
}

/// The references an update holds for its queued upstreams, with the value
/// each one must keep.
pub fn held_by(state: &UpdateState) -> HashMap<String, String> {
    state
        .upstreams
        .iter()
        .map(|q| (upstream_ref(&q.name), q.fetched.clone()))
        .collect()
}

/// Restores the references the update in `state` holds that are missing,
/// as after a `start` interrupted before it wrote them, at the tips the state
/// names, when those commits are still in the repository. Returns the
/// references it restored. A reference at another value is left alone;
/// `doctor` reports it.
pub fn restore_held(git: &Git, state: &UpdateState) -> Result<Vec<String>> {
    let mut held: Vec<(String, String)> = held_by(state).into_iter().collect();
    held.sort();
    let mut restored = Vec::new();
    for (name, tip) in held {
        if git.commit(&name)?.is_none() && git.object_exists(&tip)? {
            // An empty old value creates the reference only where none exists.
            git.run(&["update-ref", "-m", "git-intake: restore", &name, &tip, ""])?;
            restored.push(name);
        }
    }
    Ok(restored)
}

/// Removes the references an update holds. Returns their names.
pub fn release(git: &Git, state: &UpdateState) -> Result<Vec<String>> {
    release_refs(git, state.upstreams.iter().map(|q| q.name.as_str()))
}

/// Removes the references an update holds for the upstreams `names`.
/// Returns their names.
pub fn release_refs<'a>(git: &Git, names: impl Iterator<Item = &'a str>) -> Result<Vec<String>> {
    let mut removed = Vec::new();
    for name in names.map(upstream_ref) {
        if !removed.contains(&name) && git.commit(&name)?.is_some() {
            git.run(&["update-ref", "-d", &name])?;
            removed.push(name);
        }
    }
    removed.sort();
    Ok(removed)
}

/// Every reference under `refs/git-intake/`, with the commit it points at.
pub fn existing(git: &Git) -> Result<Vec<(String, String)>> {
    let out = git.run(&[
        "for-each-ref",
        "--format=%(refname)%09%(objectname)",
        REFS_PREFIX,
    ])?;
    Ok(lines(&out)
        .into_iter()
        .filter_map(|l| {
            let (name, oid) = l.split_once('\t')?;
            Some((name.to_string(), oid.to_string()))
        })
        .collect())
}

/// The fetches of one command, into a namespace under
/// `refs/git-intake/read/` that no other command uses. The namespace is
/// removed when this is dropped, whatever the command's outcome.
pub struct Fetches<'r> {
    git: &'r Git,
    namespace: String,
}

impl<'r> Fetches<'r> {
    pub fn new(repo: &'r Repo) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        Self {
            git: &repo.git,
            namespace: format!("{READ_PREFIX}{}-{nanos:x}/", std::process::id()),
        }
    }

    /// Fetches an upstream's branch. Returns the fetched tip, or git's error.
    pub fn upstream(&mut self, up: &Upstream) -> Result<std::result::Result<String, String>> {
        let name = format!("{}upstreams/{}", self.namespace, up.name);
        let refspec = format!("+refs/heads/{}:{name}", up.branch);
        if let Err(why) = self.fetch(up, &refspec)? {
            return Ok(Err(why));
        }
        Ok(match self.git.commit(&name)? {
            Some(tip) => Ok(tip),
            None => Err(format!(
                "fetching branch '{}' from {} left no commit",
                up.branch,
                crate::redact::redacted(&up.url)
            )),
        })
    }

    /// Fetches every branch of an upstream. Returns each branch with the
    /// reference that holds it for this command, or git's error.
    pub fn branches(
        &mut self,
        up: &Upstream,
    ) -> Result<std::result::Result<Vec<(String, String)>, String>> {
        let prefix = format!("{}branches/{}/", self.namespace, up.name);
        if let Err(why) = self.fetch(up, &format!("+refs/heads/*:{prefix}*"))? {
            return Ok(Err(why));
        }
        let refs = lines(
            &self
                .git
                .run(&["for-each-ref", "--format=%(refname)", &prefix])?,
        );
        Ok(Ok(refs
            .into_iter()
            .map(|r| (r.strip_prefix(&prefix).unwrap_or(&r).to_string(), r))
            .collect()))
    }

    /// Fetches branch `branch` of the git remote `remote`, as the update's
    /// branch on the publish remote is read. Returns its tip, or git's error.
    pub fn remote_branch(
        &mut self,
        remote: &str,
        branch: &str,
    ) -> Result<std::result::Result<String, String>> {
        let name = format!("{}remotes/{remote}/{branch}", self.namespace);
        let refspec = format!("+refs/heads/{branch}:{name}");
        if let Err(why) = self.fetch_from(remote, &refspec)? {
            return Ok(Err(why));
        }
        Ok(match self.git.commit(&name)? {
            Some(tip) => Ok(tip),
            None => Err(format!(
                "fetching branch '{branch}' from '{remote}' left no commit"
            )),
        })
    }

    fn fetch(&self, up: &Upstream, refspec: &str) -> Result<std::result::Result<(), String>> {
        self.fetch_from(&up.url, refspec)
    }

    fn fetch_from(&self, source: &str, refspec: &str) -> Result<std::result::Result<(), String>> {
        let out = self.git.raw(&[
            "fetch",
            "--quiet",
            "--no-tags",
            "--no-write-fetch-head",
            "--end-of-options",
            source,
            refspec,
        ])?;
        Ok(if out.success() {
            Ok(())
        } else {
            Err(out.into_error().to_string())
        })
    }

    fn clean_up(&self) -> Result<()> {
        let refs =
            lines(
                &self
                    .git
                    .run(&["for-each-ref", "--format=%(refname)", &self.namespace])?,
            );
        if !refs.is_empty() {
            let input: String = refs.iter().map(|r| format!("delete {r}\n")).collect();
            self.git.run_with_input(&["update-ref", "--stdin"], input)?;
        }
        Ok(())
    }
}

impl Drop for Fetches<'_> {
    fn drop(&mut self) {
        if let Err(err) = self.clean_up() {
            eprintln!(
                "warning: cannot remove the references this command fetched under {}: {}; \
                 `git intake doctor` lists them",
                self.namespace,
                crate::redact::redacted(&format!("{err:#}"))
            );
        }
    }
}
