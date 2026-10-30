//! `log`: upstream commits the integration branch does not contain, or for a
//! port upstream, the commits still to decide on.

use anyhow::{Context, Result};
use serde::Serialize;

use crate::Exit;
use crate::commands::update::plural;
use crate::config::Mode;
use crate::git::{abbrev, branch_ref};
use crate::output::{Render, emit};
use crate::port::{self, Standing};
use crate::repo::Repo;
use crate::sync_record;
use crate::upstreams::Fetches;

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
    pub mode: Mode,
    pub tip: Option<String>,
    /// Whether the commits could be listed: the upstream was fetched and,
    /// for a port upstream, its journal is valid and its history not
    /// rewritten.
    pub available: bool,
    pub detail: Option<String>,
    /// For a follow upstream, the commits the integration branch lacks; for
    /// a port upstream, the pending commits, oldest first.
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
    let mut fetches = Fetches::new(repo);
    let mut fetched = Vec::new();
    for up in &selected {
        fetched.push(fetches.upstream(up)?);
    }
    let fetch_failed = fetched.iter().any(|f| f.is_err());
    let latest = sync_record::latest(&sync_record::records(git, &integration, &config)?);
    let mut upstreams = Vec::new();
    for (up, fetched) in selected.iter().zip(fetched) {
        let mut entry = UpstreamLog {
            name: up.name.clone(),
            path: up.path.clone(),
            branch: up.source(),
            mode: up.mode,
            tip: None,
            available: false,
            detail: None,
            commits: Vec::new(),
        };
        let tip = match fetched {
            Ok(tip) => tip,
            Err(why) => {
                entry.detail = Some(format!("fetch failed: {why}"));
                upstreams.push(entry);
                continue;
            }
        };
        entry.tip = Some(tip.clone());
        match up.mode {
            Mode::Follow => {
                let out = git.run(&[
                    "log",
                    "--format=%H%x1f%s",
                    &format!("{integration}..{tip}"),
                    "--",
                ])?;
                entry.commits = out
                    .lines()
                    .filter_map(|l| l.split_once('\u{1f}'))
                    .map(|(c, s)| LogCommit {
                        commit: c.to_string(),
                        subject: s.to_string(),
                    })
                    .collect();
                entry.available = true;
            }
            Mode::Port => {
                match port::standing(git, &config, &integration, up, latest.get(&up.name), &tip) {
                    Err(err) => entry.detail = Some(format!("{err:#}")),
                    Ok(Standing::Rewritten(why)) => {
                        entry.detail = Some(format!("rewritten: {why}"))
                    }
                    Ok(Standing::NoBase) => {
                        entry.detail = Some(format!(
                            "no journal and no last synced commit to start one from; write the journal {} by hand",
                            port::journal_path(&up.name)
                        ))
                    }
                    Ok(Standing::Known(a)) => {
                        entry.commits = a
                            .pending()
                            .into_iter()
                            .map(|c| LogCommit {
                                commit: c.commit.clone(),
                                subject: c.subject.clone(),
                            })
                            .collect();
                        entry.available = true;
                    }
                }
            }
        }
        upstreams.push(entry);
    }
    drop(fetches);
    emit(
        &LogDoc {
            integration: config.integration.clone(),
            upstreams,
        },
        json,
    )?;
    Ok(if fetch_failed {
        Exit::Negative
    } else {
        Exit::Success
    })
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
            match u.mode {
                Mode::Follow => out.push_str(&format!(
                    "{} ({path}, {}): {n} commit{} not in {}\n",
                    u.name,
                    u.branch,
                    plural(n),
                    self.integration
                )),
                Mode::Port => out.push_str(&format!(
                    "{} ({path}, {}): {n} pending commit{}, oldest first\n",
                    u.name,
                    u.branch,
                    plural(n)
                )),
            }
            for c in &u.commits {
                out.push_str(&format!("  {}  {}\n", abbrev(&c.commit), c.subject));
            }
        }
        out
    }
}
