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

/// Whether two URLs name the same repository: local paths and `file://`
/// URLs once resolved as git resolves them, against `base`, and made
/// canonical; other URLs once the scp-like form is written as `ssh://`, the
/// scheme and host are in lower case, a default port is dropped, and a
/// leading `/` and a trailing `/` and `.git` of the path are dropped.
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
        return format!("file:{}", local_repository(&full).display());
    }
    // `scheme://[user@]host[:port]/path`, or the scp-like `[user@]host:path`,
    // which is the `ssh` scheme.
    let (scheme, authority, path) = match url.split_once("://") {
        Some((scheme, rest)) => {
            let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
            (scheme.to_ascii_lowercase(), authority, path)
        }
        None => {
            let (authority, path) = url.split_once(':').unwrap_or((url, ""));
            ("ssh".to_string(), authority, path)
        }
    };
    let scheme = match scheme.as_str() {
        "git+ssh" | "ssh+git" => "ssh".to_string(),
        _ => scheme,
    };
    let (user, host) = match authority.rsplit_once('@') {
        Some((user, host)) => (format!("{user}@"), host),
        None => (String::new(), authority),
    };
    let mut host = host.to_ascii_lowercase();
    let default_port = match scheme.as_str() {
        "ssh" => Some(":22"),
        "https" => Some(":443"),
        "http" => Some(":80"),
        "git" => Some(":9418"),
        _ => None,
    };
    if let Some(port) = default_port
        && host.ends_with(port)
    {
        host.truncate(host.len() - port.len());
    }
    let path = path.trim_start_matches('/').trim_end_matches('/');
    let path = path
        .strip_suffix(".git")
        .unwrap_or(path)
        .trim_end_matches('/');
    format!("{scheme}://{user}{host}/{path}")
}

/// The repository a local path names, as git finds it: the first of
/// `<path>/.git`, `<path>`, `<path>.git/.git` and `<path>.git` that is a git
/// directory, made canonical; the path itself, cleaned, when none is.
fn local_repository(path: &Path) -> PathBuf {
    let plain = path.as_os_str().to_string_lossy();
    let plain = plain.trim_end_matches('/');
    for candidate in [
        format!("{plain}/.git"),
        plain.to_string(),
        format!("{plain}.git/.git"),
        format!("{plain}.git"),
    ] {
        let dir = Path::new(&candidate);
        if dir.join("HEAD").is_file()
            && let Ok(found) = std::fs::canonicalize(dir)
        {
            return found;
        }
    }
    std::fs::canonicalize(path).unwrap_or_else(|_| clean(path))
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
            "https://example.com:443/team/up/"
        ));
        assert!(same(
            "git@Example.com:team/up.git",
            "git@example.com:team/up"
        ));
        assert!(same(
            "git@example.com:team/up.git",
            "ssh://git@EXAMPLE.com:22/team/up"
        ));
        assert!(same(
            "example.com:/srv/up.git",
            "git+ssh://example.com/srv/up"
        ));
        assert!(!same(
            "git@example.com:team/up",
            "git@example.com:team/other"
        ));
        assert!(!same(
            "git@example.com:team/up",
            "ssh://git@example.com:2222/team/up"
        ));
        assert!(!same(
            "https://example.com/Team/up",
            "https://example.com/team/up"
        ));
    }

    #[test]
    fn local_paths_resolve_as_git_finds_repositories() {
        let dir = tempfile::tempdir().unwrap();
        let bare = dir.path().join("up.git");
        std::fs::create_dir_all(&bare).unwrap();
        std::fs::write(bare.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        let work = dir.path().join("work");
        std::fs::create_dir_all(work.join(".git")).unwrap();
        std::fs::write(work.join(".git").join("HEAD"), "ref: refs/heads/main\n").unwrap();
        let base = dir.path().join("product");
        std::fs::create_dir_all(&base).unwrap();
        let same = |a: &str, b: &str| same_repository(a, b, &base);
        let bare_path = bare.to_string_lossy();
        assert!(same("../up", &bare_path));
        assert!(same("../up/", &bare_path));
        assert!(same("../work", &format!("{}/.git", work.to_string_lossy())));
        assert!(!same("../work", &bare_path));
    }
}
