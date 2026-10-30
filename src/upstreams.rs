//! Upstreams fetched by URL into references of the tool's own.
//!
//! An upstream is fetched with `git fetch --no-tags <url>
//! +refs/heads/<branch>:refs/git-intake/upstreams/<name>`, in the main
//! worktree so that a relative URL keeps its meaning. No remote is added,
//! read or changed. The references live only as long as something needs
//! them: a command that reads upstreams removes those it fetched when it
//! ends, through [`Fetches`], and an update keeps the references of its
//! queued upstreams until `finish` or `abort` removes them.

use std::collections::HashMap;

use anyhow::Result;

use crate::config::Upstream;
use crate::git::{Git, lines};
use crate::repo::Repo;
use crate::state::UpdateState;

/// Prefix of every reference the tool creates.
pub const REFS_PREFIX: &str = "refs/git-intake/";

/// Prefix of the references that hold fetched upstream branches.
const UPSTREAM_PREFIX: &str = "refs/git-intake/upstreams/";

/// Prefix of the transient references `branches` fetches every branch of an
/// upstream into.
const BRANCHES_PREFIX: &str = "refs/git-intake/branches/";

/// The reference that holds an upstream's fetched branch.
pub fn upstream_ref(name: &str) -> String {
    format!("{UPSTREAM_PREFIX}{name}")
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

/// The fetches of one command. Every reference fetched through it is
/// removed when it is dropped, whatever the command's outcome, except the
/// references an update holds, which get back the value the update keeps,
/// and those handed to the update with [`Fetches::keep`].
pub struct Fetches<'r> {
    git: &'r Git,
    held: HashMap<String, String>,
    fetched: Vec<String>,
    namespaces: Vec<String>,
}

impl<'r> Fetches<'r> {
    /// Fetches for a command in `repo`. The update in progress, when its
    /// state can be read, names the references to restore afterwards.
    pub fn new(repo: &'r Repo) -> Self {
        let held = match repo.state() {
            Ok(Some(state)) => held_by(&state),
            _ => HashMap::new(),
        };
        Self {
            git: &repo.git,
            held,
            fetched: Vec::new(),
            namespaces: Vec::new(),
        }
    }

    /// Fetches an upstream's branch. Returns the fetched tip, or git's error.
    pub fn upstream(&mut self, up: &Upstream) -> Result<std::result::Result<String, String>> {
        let name = upstream_ref(&up.name);
        if !self.fetched.contains(&name) {
            self.fetched.push(name.clone());
        }
        let refspec = format!("+refs/heads/{}:{name}", up.branch);
        let out = self.git.raw(&[
            "fetch",
            "--quiet",
            "--no-tags",
            "--no-write-fetch-head",
            "--end-of-options",
            &up.url,
            &refspec,
        ])?;
        if !out.success() {
            return Ok(Err(out.into_error().to_string()));
        }
        Ok(match self.git.commit(&name)? {
            Some(tip) => Ok(tip),
            None => Err(format!(
                "fetching branch '{}' from {} left no commit",
                up.branch, up.url
            )),
        })
    }

    /// Fetches every branch of an upstream into a namespace of its own.
    /// Returns each branch with the reference that holds it, or git's error.
    pub fn branches(
        &mut self,
        up: &Upstream,
    ) -> Result<std::result::Result<Vec<(String, String)>, String>> {
        let prefix = format!("{BRANCHES_PREFIX}{}/", up.name);
        if !self.namespaces.contains(&prefix) {
            self.namespaces.push(prefix.clone());
        }
        let refspec = format!("+refs/heads/*:{prefix}*");
        let out = self.git.raw(&[
            "fetch",
            "--quiet",
            "--no-tags",
            "--no-write-fetch-head",
            "--end-of-options",
            &up.url,
            &refspec,
        ])?;
        if !out.success() {
            return Ok(Err(out.into_error().to_string()));
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

    /// Hands the reference of each named upstream to the update that starts
    /// now: it outlives the command, and the update keeps it at `tip`.
    pub fn keep(&mut self, kept: &[(String, String)]) {
        for (name, tip) in kept {
            self.held.insert(upstream_ref(name), tip.clone());
        }
    }

    fn clean_up(&self) -> Result<()> {
        for name in &self.fetched {
            match self.held.get(name) {
                Some(value) => {
                    if self.git.commit(name)?.as_deref() != Some(value.as_str()) {
                        self.git.run(&["update-ref", name, value])?;
                    }
                }
                None => {
                    if self.git.commit(name)?.is_some() {
                        self.git.run(&["update-ref", "-d", name])?;
                    }
                }
            }
        }
        for prefix in &self.namespaces {
            let refs = lines(
                &self
                    .git
                    .run(&["for-each-ref", "--format=%(refname)", prefix])?,
            );
            if !refs.is_empty() {
                let input: String = refs.iter().map(|r| format!("delete {r}\n")).collect();
                self.git.run_with_input(&["update-ref", "--stdin"], input)?;
            }
        }
        Ok(())
    }
}

impl Drop for Fetches<'_> {
    fn drop(&mut self) {
        if let Err(err) = self.clean_up() {
            eprintln!(
                "warning: cannot remove the upstream references this command fetched under {REFS_PREFIX}: {err:#}; \
                 `git intake doctor` lists them"
            );
        }
    }
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
