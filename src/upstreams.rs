//! Upstreams fetched by URL into references of the tool's own.
//!
//! An upstream is fetched by `git fetch --no-tags <url>`, in the main
//! worktree so that a relative URL keeps its meaning. No remote is added or
//! changed. Each command fetches into a namespace of its own,
//! `refs/git-intake/read/<token>/`, through [`Fetches`], and removes it when
//! it ends. An update holds `refs/git-intake/upstreams/<name>` for each of
//! its queued upstreams: `start` writes them, from what it fetched, and only
//! `finish` or `abort` removes them, so no other command ever moves them.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

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
        git.run(&["update-ref", "-m", "git-intake: start", &name, &tip])?;
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

/// Removes the references an update holds. Returns their names.
pub fn release(git: &Git, state: &UpdateState) -> Result<Vec<String>> {
    let mut removed = Vec::new();
    for name in held_by(state).into_keys() {
        if git.commit(&name)?.is_some() {
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
                up.branch, up.url
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

    fn fetch(&self, up: &Upstream, refspec: &str) -> Result<std::result::Result<(), String>> {
        let out = self.git.raw(&[
            "fetch",
            "--quiet",
            "--no-tags",
            "--no-write-fetch-head",
            "--end-of-options",
            &up.url,
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
                "warning: cannot remove the references this command fetched under {}: {err:#}; \
                 `git intake doctor` lists them",
                self.namespace
            );
        }
    }
}

/// Whether two URLs name the same repository: local paths and `file://`
/// URLs once resolved against `base` and made canonical, other URLs once a
/// trailing `/` and `.git` are dropped and the host is in lower case.
pub fn same_repository(a: &str, b: &str, base: &Path) -> bool {
    normalise_url(a, base) == normalise_url(b, base)
}

fn normalise_url(url: &str, base: &Path) -> String {
    let url = url.trim();
    if let Some(path) = local_path(url) {
        let path = Path::new(path);
        let full = if path.is_absolute() {
            path.to_path_buf()
        } else {
            base.join(path)
        };
        let full = std::fs::canonicalize(&full).unwrap_or_else(|_| clean(&full));
        return format!("file:{}", full.display());
    }
    // `scheme://[user@]host[:port]/path`, or the scp-like `[user@]host:path`.
    let (prefix, rest, sep) = match url.split_once("://") {
        Some((scheme, rest)) => (format!("{}://", scheme.to_ascii_lowercase()), rest, '/'),
        None => (String::new(), url, ':'),
    };
    let (authority, path) = rest.split_once(sep).unwrap_or((rest, ""));
    let authority = match authority.rsplit_once('@') {
        Some((user, host)) => format!("{user}@{}", host.to_ascii_lowercase()),
        None => authority.to_ascii_lowercase(),
    };
    let path = path.trim_end_matches('/');
    let path = path
        .strip_suffix(".git")
        .unwrap_or(path)
        .trim_end_matches('/');
    format!("{prefix}{authority}{sep}{path}")
}

/// The path of a URL that names a local repository, as git tells them
/// apart: a `file://` URL, or no scheme and no colon before the first slash.
fn local_path(url: &str) -> Option<&str> {
    if let Some(path) = url.strip_prefix("file://") {
        return Some(path);
    }
    if url.contains("://") {
        return None;
    }
    match (url.find(':'), url.find('/')) {
        (Some(colon), Some(slash)) if colon < slash => None,
        (Some(_), None) => None,
        _ => Some(url),
    }
}

/// A path without `.` and `..` components, for one that does not exist.
fn clean(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// The URLs a git remote is fetched from and pushed to, as configured.
pub fn remote_urls(git: &Git, remote: &str) -> Result<Vec<String>> {
    let mut urls = Vec::new();
    for key in ["url", "pushurl"] {
        let out = git.raw(&["config", "--get-all", &format!("remote.{remote}.{key}")])?;
        match out.code {
            Some(0) => urls.extend(lines(&out.stdout)),
            Some(1) => {}
            _ => return Err(out.into_error().into()),
        }
    }
    Ok(urls)
}

/// Every git remote of the repository.
pub fn remotes(git: &Git) -> Result<Vec<String>> {
    Ok(lines(&git.run(&["remote"])?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_naming_the_same_repository_match() {
        let base = Path::new("/srv/product");
        let same = |a: &str, b: &str| same_repository(a, b, base);
        assert!(same("../up.git", "/srv/up.git"));
        assert!(same("file:///srv/up.git", "/srv/./up.git/"));
        assert!(!same("../up.git", "/srv/up"));
        assert!(same(
            "https://Example.COM/team/up.git",
            "https://example.com/team/up/"
        ));
        assert!(same(
            "git@Example.com:team/up.git",
            "git@example.com:team/up"
        ));
        assert!(!same(
            "git@example.com:team/up",
            "git@example.com:team/other"
        ));
        assert!(!same(
            "https://example.com/Team/up",
            "https://example.com/team/up"
        ));
    }
}
