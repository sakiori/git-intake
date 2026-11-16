//! `log`: upstream commits the integration branch does not contain, or for a
//! port upstream, the commits still to decide on.

use anyhow::Result;
use serde::Serialize;

use crate::Exit;
use crate::commands::update::{no_release, plural};
use crate::config::Mode;
use crate::git::abbrev;
use crate::output::{Render, emit};
use crate::port::{self, Standing};
use crate::repo::Repo;
use crate::sync_record;
use crate::tags::{TagPattern, UpstreamTags};
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
    /// Follow mode with `tags`: the commit of the newest release tag on the
    /// fetched branch, up to which the commits are listed.
    pub release: Option<String>,
    /// Follow mode with `tags`: the name of that tag.
    pub release_tag: Option<String>,
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
    let loaded = repo.load_config()?;
    let config = loaded.config;
    let integration = loaded.tip;
    let selected = config.select(names)?;
    let git = &repo.git;
    let mut fetches = Fetches::new(repo);
    let mut fetched = Vec::new();
    for up in &selected {
        fetched.push(fetches.upstream(up)?);
    }
    let mut fetch_failed = fetched.iter().any(|f| f.is_err());
    let tips: Vec<String> = fetched.iter().filter_map(|f| f.clone().ok()).collect();
    let latest = sync_record::latest(git, &sync_record::records(git, &integration, &tips)?)?;
    let mut upstreams = Vec::new();
    for (up, fetched) in selected.iter().zip(fetched) {
        let mut entry = UpstreamLog {
            name: up.name.clone(),
            path: up.path.clone(),
            branch: up.source(),
            mode: up.mode,
            tip: None,
            release: None,
            release_tag: None,
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
                // With release tags, the commits up to the newest release.
                let mut target = tip.clone();
                if let Some(text) = &up.tags {
                    let tags = match UpstreamTags::list(git, up)? {
                        Ok(tags) => tags,
                        Err(why) => {
                            fetch_failed = true;
                            entry.detail = Some(why);
                            upstreams.push(entry);
                            continue;
                        }
                    };
                    let pattern = TagPattern::parse(text)
                        .map_err(|why| anyhow::anyhow!("upstream '{}' tags: {why}", up.name))?;
                    match tags.release(git, &pattern, up.prereleases, &tip)? {
                        Some(release) => {
                            target = release.commit.clone();
                            entry.release = Some(release.commit);
                            entry.release_tag = Some(release.tag);
                        }
                        None => {
                            entry.detail = Some(no_release(up));
                            entry.available = true;
                            upstreams.push(entry);
                            continue;
                        }
                    }
                }
                let out = git.run(&[
                    "log",
                    "--format=%H%x1f%s",
                    &format!("{integration}..{target}"),
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
                match port::standing(git, &integration, up, latest.get(&up.name), &tip, &tips) {
                    Err(err) => entry.detail = Some(format!("{err:#}")),
                    Ok(Standing::Rewritten(rewrite)) => {
                        entry.detail = Some(format!("rewritten: {}", rewrite.describe(&up.name)))
                    }
                    Ok(Standing::NoBase) => entry.detail = Some(port::no_base_advice(&up.name)),
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
                    "{} ({path}, {}): {n} commit{} not in {}{}\n",
                    u.name,
                    u.branch,
                    plural(n),
                    self.integration,
                    u.release_tag
                        .as_deref()
                        .map(|t| format!(" up to {t}"))
                        .unwrap_or_default()
                )),
                Mode::Port => out.push_str(&format!(
                    "{} ({path}, {}): {n} pending commit{}, oldest first\n",
                    u.name,
                    u.branch,
                    plural(n)
                )),
            }
            if let Some(detail) = &u.detail {
                out.push_str(&format!("  {detail}\n"));
            }
            for c in &u.commits {
                out.push_str(&format!("  {}  {}\n", abbrev(&c.commit), c.subject));
            }
        }
        out
    }
}
