//! `log`: upstream commits the integration branch does not contain.

use anyhow::{Context, Result};
use serde::Serialize;

use crate::Exit;
use crate::drift;
use crate::git::{abbrev, branch_ref};
use crate::output::{Render, emit};
use crate::repo::Repo;

#[derive(Serialize)]
pub struct LogDoc {
    pub integration: String,
    pub upstreams: Vec<UpstreamLog>,
}

#[derive(Serialize)]
pub struct UpstreamLog {
    pub name: String,
    pub path: Option<String>,
    pub branch: String,
    pub tip: Option<String>,
    pub available: bool,
    pub detail: Option<String>,
    pub commits: Vec<LogCommit>,
}

#[derive(Serialize)]
pub struct LogCommit {
    pub commit: String,
    pub subject: String,
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
        let (tip, detail, commits) = match drift::fetched_tip(git, up)? {
            Err(why) => (None, Some(why), Vec::new()),
            Ok(tip) => {
                let out = git.run(&[
                    "log",
                    "--format=%H%x1f%s",
                    &format!("{integration}..{tip}"),
                    "--",
                ])?;
                let commits = out
                    .lines()
                    .filter_map(|l| l.split_once('\u{1f}'))
                    .map(|(c, s)| LogCommit {
                        commit: c.to_string(),
                        subject: s.to_string(),
                    })
                    .collect();
                (Some(tip), None, commits)
            }
        };
        upstreams.push(UpstreamLog {
            name: up.name.clone(),
            path: up.path.clone(),
            branch: up.source(),
            available: tip.is_some(),
            tip,
            detail,
            commits,
        });
    }
    emit(
        &LogDoc {
            integration: config.integration.clone(),
            upstreams,
        },
        json,
    )?;
    Ok(Exit::Success)
}

impl Render for LogDoc {
    fn render(&self) -> String {
        let mut out = String::new();
        for (i, u) in self.upstreams.iter().enumerate() {
            if i > 0 {
                out.push('\n');
            }
            let path = u.path.as_deref().unwrap_or("/");
            if !u.available {
                out.push_str(&format!(
                    "{} ({path}, {}): unavailable: {}\n",
                    u.name,
                    u.branch,
                    u.detail.as_deref().unwrap_or_default()
                ));
                continue;
            }
            let n = u.commits.len() as u64;
            out.push_str(&format!(
                "{} ({path}, {}): {} commit{} not in {}\n",
                u.name,
                u.branch,
                n,
                crate::commands::update::plural(n),
                self.integration
            ));
            for c in &u.commits {
                out.push_str(&format!("  {}  {}\n", abbrev(&c.commit), c.subject));
            }
        }
        out
    }
}
