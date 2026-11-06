//! `env`: the variables `verify` gives the checks, for a commit, read from
//! history alone.
//!
//! A merge request pipeline runs its own jobs on a fresh clone with no
//! update state. For a commit H and the integration tip T, the base is the
//! merge base of H and T: the tip the update started from or last caught up
//! with, or, for a merged results pipeline whose commit merges the update
//! into the target, that target. The upstreams and their targets come from
//! the update line of H over the base, their previous synced commits from
//! the integration line at the base.

use std::process::Command;

use anyhow::{Context, Result};
use serde_json::{Map, Value};

use crate::Exit;
use crate::commands::merge_request::fetch_integration;
use crate::commands::verify::{self, Described};
use crate::config::{self, Config, Mode};
use crate::drift;
use crate::git::{abbrev, branch_ref};
use crate::output::esay;
use crate::repo::Repo;
use crate::sync_record;
use crate::update_line::{self, History};

pub fn run(
    repo: &Repo,
    rev: Option<&str>,
    dotenv: bool,
    json: bool,
    command: &[String],
) -> Result<Exit> {
    let rev = rev.unwrap_or("HEAD");
    // A failure to read the variables ends the command here, with status 2,
    // before a command could run without them.
    let (commit, vars) = variables(repo, rev)?;
    if vars.is_none() {
        esay!(
            "{} carries no update; no {}* variable is set",
            abbrev(&commit),
            verify::ENV_PREFIX
        );
    }
    let vars = vars.unwrap_or_default();
    if let Some((program, args)) = command.split_first() {
        let mut cmd = Command::new(program);
        cmd.args(args);
        verify::clear_inherited(&mut cmd);
        cmd.envs(vars.iter().map(|(k, v)| (k, v)));
        let status = cmd.status().with_context(|| {
            format!(
                "cannot run `{}`",
                crate::redact::redacted(&command.join(" "))
            )
        })?;
        return Ok(match status.code() {
            Some(0) => Exit::Success,
            Some(code) => Exit::Code(u8::try_from(code).unwrap_or(1)),
            None => Exit::Code(signal_status(&status)),
        });
    }
    let mut out = String::new();
    if json {
        let object: Map<String, Value> = vars
            .iter()
            .map(|(k, v)| (k.clone(), Value::String(v.clone())))
            .collect();
        out = serde_json::to_string_pretty(&Value::Object(object))? + "\n";
    } else {
        for (k, v) in &vars {
            if dotenv {
                out.push_str(&format!("{k}={v}\n"));
            } else {
                out.push_str(&format!("export {k}={}\n", shell_quote(v)));
            }
        }
    }
    crate::output::print(&out)?;
    Ok(Exit::Success)
}

/// The status a shell gives a command a signal ended: 128 and the signal.
fn signal_status(status: &std::process::ExitStatus) -> u8 {
    use std::os::unix::process::ExitStatusExt;
    status
        .signal()
        .and_then(|s| u8::try_from(128 + s).ok())
        .unwrap_or(1)
}

/// A value quoted for the POSIX shell.
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

/// Variables by name, in the order they are given.
type Variables = Vec<(String, String)>;

/// The commit `rev` names and its variables, or `None` when it carries no
/// update.
fn variables(repo: &Repo, rev: &str) -> Result<(String, Option<Variables>)> {
    let (commit, update) = update_at(repo, rev)?;
    let Some(update) = update else {
        return Ok((commit, None));
    };
    let upstreams = described(&repo.git, &update.config, &update.history, &update.base)?;
    let vars = verify::variables(
        &update.base,
        &commit,
        &update.config.integration,
        &upstreams,
    );
    Ok((commit, Some(vars)))
}

/// The update a commit carries, read from history alone.
pub struct Update {
    /// The configuration at the integration tip.
    pub config: Config,
    /// The integration tip the update started from or last caught up with.
    pub base: String,
    pub history: History,
}

/// The commit `rev` names and the update it carries, or `None` when the
/// integration branch contains it or it carries none.
pub fn update_at(repo: &Repo, rev: &str) -> Result<(String, Option<Update>)> {
    let git = &repo.git;
    let commit = repo
        .here
        .commit(rev)?
        .with_context(|| format!("'{rev}' names no commit"))?;
    let named = match repo.config_at(&commit)? {
        Some(config) => config,
        None => repo
            .head()
            .map(|head| repo.config_at(head))
            .transpose()?
            .flatten()
            .with_context(|| {
                format!(
                    "no {} is committed at {} or at HEAD",
                    config::FILE_NAME,
                    abbrev(&commit)
                )
            })?,
    };
    let integration = named.integration.clone();
    let tip = match &named.publish {
        Some(publish) => fetch_integration(git, &publish.remote, &integration)?,
        None => git
            .commit(&branch_ref(&integration))?
            .with_context(|| format!("the integration branch '{integration}' does not exist"))?,
    };
    // The upstreams are those the integration tip configures.
    let config = repo.config_at(&tip)?.unwrap_or(named);
    if git.is_ancestor(&commit, &tip)? {
        return Ok((commit, None));
    }
    let Some(base) = git.merge_base(&commit, &tip)? else {
        return Ok((commit, None));
    };
    let history = update_line::describe(git, &config, &commit, &base)?;
    if !history.carries_update() {
        return Ok((commit, None));
    }
    Ok((
        commit,
        Some(Update {
            config,
            base,
            history,
        }),
    ))
}

/// The upstreams of the update `history` holds over `base`, in queue order,
/// as the variables describe them.
pub fn described(
    git: &crate::git::Git,
    config: &Config,
    history: &History,
    base: &str,
) -> Result<Vec<Described>> {
    // The records of the upstreams' own histories, below their targets, do
    // not count as this repository's.
    let targets: Vec<String> = history.follows.iter().map(|f| f.target.clone()).collect();
    let latest = sync_record::latest(git, &sync_record::records(git, base, &targets)?)?;
    let mut upstreams = Vec::new();
    for name in history.names(config) {
        let Some(up) = config.upstream(&name) else {
            continue;
        };
        if let Some(f) = history.follow(&name) {
            let previous = match latest.get(&name) {
                Some(record) => Some(record.target.clone()),
                None => drift::adopted_base(git, base, up, &f.target)?,
            };
            upstreams.push(Described {
                name,
                mode: Mode::Follow,
                target: Some(f.target.clone()),
                previous,
            });
        } else if let Some(p) = history.ports.iter().find(|p| p.name == name) {
            let recorded: Vec<String> = p
                .copies
                .iter()
                .map(|c| c.taken.clone())
                .chain(p.decisions.iter().map(|d| d.commit.clone()))
                .collect();
            upstreams.push(Described {
                name,
                mode: Mode::Port,
                target: update_line::newest(git, &recorded)?,
                previous: None,
            });
        }
    }
    Ok(upstreams)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_are_quoted_for_the_shell() {
        assert_eq!(shell_quote("app-a app-b"), "'app-a app-b'");
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
        assert_eq!(shell_quote(""), "''");
    }
}
