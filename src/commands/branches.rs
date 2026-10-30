//! `branches`: upstream branches that hold work the integration branch lacks.

use std::collections::HashSet;

use anyhow::{Context, Result};
use serde::Serialize;

use crate::Exit;
use crate::commands::update::plural;
use crate::config::Mode;
use crate::git::{branch_ref, lines};
use crate::output::{Render, emit};
use crate::repo::Repo;
use crate::sync_record;
use crate::upstreams::Fetches;

#[derive(Serialize)]
pub struct BranchesDoc {
    pub integration: String,
    pub unmerged_work: bool,
    pub upstreams: Vec<UpstreamBranches>,
}

#[derive(Serialize)]
pub struct UpstreamBranches {
    pub name: String,
    pub url: String,
    pub path: Option<String>,
    pub mode: Mode,
    pub available: bool,
    pub detail: Option<String>,
    pub branches: Vec<UnmergedBranch>,
}

#[derive(Serialize)]
pub struct UnmergedBranch {
    pub branch: String,
    /// Whether this is the branch the configuration tracks.
    pub tracked: bool,
    /// Commits the integration branch lacks; for a port upstream, other than
    /// those a copy names as taken.
    pub unmerged_commits: u64,
}

/// Fetches every branch of the selected upstreams for this command only, and
/// counts what each holds that the integration branch lacks.
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
    let mut fetches = Fetches::new(repo);
    let mut upstreams = Vec::new();
    for up in selected {
        let mut entry = UpstreamBranches {
            name: up.name.clone(),
            url: up.url.clone(),
            path: up.path.clone(),
            mode: up.mode,
            available: true,
            detail: None,
            branches: Vec::new(),
        };
        let refs = match fetches.branches(up)? {
            Ok(refs) => refs,
            Err(why) => {
                entry.available = false;
                entry.detail = Some(format!("fetch failed: {why}"));
                upstreams.push(entry);
                continue;
            }
        };
        let taken: HashSet<String> = match up.mode {
            Mode::Follow => HashSet::new(),
            Mode::Port => {
                let fetched: Vec<String> = refs.iter().map(|(_, r)| r.clone()).collect();
                sync_record::taken(git, &integration, &fetched, &up.name)?
                    .into_iter()
                    .collect()
            }
        };
        for (branch, r) in refs {
            let n = if taken.is_empty() {
                git.count(&integration, &r)?
            } else {
                lines(&git.run(&["rev-list", &r, "--not", &integration, "--"])?)
                    .iter()
                    .filter(|c| !taken.contains(*c))
                    .count() as u64
            };
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
    drop(fetches);
    let fetch_failed = upstreams.iter().any(|u| !u.available);
    let doc = BranchesDoc {
        integration: config.integration.clone(),
        unmerged_work: upstreams.iter().any(|u| !u.branches.is_empty()),
        upstreams,
    };
    emit(&doc, json)?;
    Ok(if fetch_failed {
        Exit::Negative
    } else {
        Exit::Success
    })
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
                    u.name,
                    b.branch,
                    n,
                    plural(n),
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
