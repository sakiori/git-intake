//! `branches`: upstream branches that hold work the integration branch lacks.

use anyhow::{Context, Result};
use serde::Serialize;

use crate::Exit;
use crate::git::{branch_ref, lines};
use crate::output::{Render, emit};
use crate::repo::Repo;

#[derive(Serialize)]
pub struct BranchesDoc {
    pub integration: String,
    pub unmerged_work: bool,
    pub upstreams: Vec<UpstreamBranches>,
}

#[derive(Serialize)]
pub struct UpstreamBranches {
    pub name: String,
    pub remote: String,
    pub path: Option<String>,
    pub available: bool,
    pub detail: Option<String>,
    pub branches: Vec<UnmergedBranch>,
}

#[derive(Serialize)]
pub struct UnmergedBranch {
    pub branch: String,
    /// Whether this is the branch the configuration tracks.
    pub tracked: bool,
    pub unmerged_commits: u64,
}

pub fn run(repo: &Repo, names: &[String], json: bool) -> Result<Exit> {
    let config = repo.load_config()?.config;
    let selected = config.select(names)?;
    let git = &repo.git;
    let integration = git
        .commit(&branch_ref(&config.integration))?
        .with_context(|| {
            format!(
                "the integration branch '{}' does not exist",
                config.integration
            )
        })?;
    let mut upstreams = Vec::new();
    for up in selected {
        let mut entry = UpstreamBranches {
            name: up.name.clone(),
            remote: up.remote.clone(),
            path: up.path.clone(),
            available: true,
            detail: None,
            branches: Vec::new(),
        };
        if git.remote_url(&up.remote)?.is_none() {
            entry.available = false;
            entry.detail = Some(format!("remote '{}' does not exist", up.remote));
            upstreams.push(entry);
            continue;
        }
        let prefix = format!("refs/remotes/{}/", up.remote);
        let refs = lines(&git.run(&["for-each-ref", "--format=%(refname)", &prefix])?);
        if refs.is_empty() {
            entry.available = false;
            entry.detail = Some(format!("remote '{}' has not been fetched", up.remote));
        }
        for r in refs {
            let branch = r.strip_prefix(&prefix).unwrap_or(&r).to_string();
            if branch == "HEAD" {
                continue;
            }
            let n = git.count(&integration, &r)?;
            if n > 0 {
                entry.branches.push(UnmergedBranch {
                    tracked: branch == up.branch,
                    branch,
                    unmerged_commits: n,
                });
            }
        }
        upstreams.push(entry);
    }
    let doc = BranchesDoc {
        integration: config.integration.clone(),
        unmerged_work: upstreams.iter().any(|u| !u.branches.is_empty()),
        upstreams,
    };
    emit(&doc, json)?;
    Ok(Exit::Success)
}

impl Render for BranchesDoc {
    fn render(&self) -> String {
        let mut out = String::new();
        for u in &self.upstreams {
            if !u.available {
                out.push_str(&format!(
                    "{}: unavailable: {}\n",
                    u.name,
                    u.detail.as_deref().unwrap_or_default()
                ));
                continue;
            }
            if u.branches.is_empty() {
                continue;
            }
            out.push_str(&format!(
                "{} ({}):\n",
                u.name,
                u.path.as_deref().unwrap_or("/")
            ));
            for b in &u.branches {
                let n = b.unmerged_commits;
                out.push_str(&format!(
                    "  {}/{}  {} unmerged commit{}{}\n",
                    u.remote,
                    b.branch,
                    n,
                    crate::commands::update::plural(n),
                    if b.tracked { " (tracked branch)" } else { "" }
                ));
            }
        }
        if !self.unmerged_work {
            out.push_str(&format!(
                "no upstream holds unmerged work; every upstream branch is contained in {}\n",
                self.integration
            ));
        }
        out
    }
}
