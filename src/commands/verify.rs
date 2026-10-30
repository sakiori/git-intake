//! `verify`: run the install command and checks on the update.

use std::fs::{self, File};
use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};

use crate::Exit;
use crate::commands::update::{require_merging, require_worktree};
use crate::config;
use crate::git::{Git, abbrev};
use crate::port::Verb;
use crate::repo::{Repo, bullet};
use crate::state::{CheckOutcome, CheckResult, UpdateState, Verification, Work};

/// Prefix of the environment variables the install command and the checks
/// receive. Variables with this prefix inherited from the caller are not
/// passed on, so a command sees only those of the update it verifies.
pub const ENV_PREFIX: &str = "GIT_INTAKE_";

pub fn run(repo: &Repo) -> Result<Exit> {
    let mut state = repo.require_state()?;
    require_merging(repo, &state)?;
    require_worktree(repo, &state)?;
    let open: Vec<String> = state
        .unmerged()
        .iter()
        .map(|u| format!("{} ({})", u.name, u.step.describe(u.port().is_some())))
        .collect();
    if !open.is_empty() {
        bail!(
            "cannot verify before every queued upstream is merged or its plan carried out; \
             these still need `git intake continue`:\n{}",
            bullet(&open)
        );
    }
    if let Some(c) = state.catching_up() {
        bail!(
            "the catch-up merge with '{}' at {} is not concluded; run `git intake continue` before verifying",
            state.config.integration,
            abbrev(&c.tip)
        );
    }
    let wt = Git::worktree(&state.worktree);
    if wt.merge_in_progress()? {
        bail!(
            "a merge is in progress in {}; conclude it before verifying",
            state.worktree.display()
        );
    }
    let changes = wt.tracked_changes()?;
    if !changes.is_empty() {
        bail!(
            "the update worktree {} has uncommitted changes; commit them on the update branch or discard them, then verify:\n{}",
            state.worktree.display(),
            bullet(&changes)
        );
    }
    let head = wt.line(&["rev-parse", "HEAD"])?;
    if verify_head(repo, &mut state, &head)? {
        println!("verified {}; next: git intake finish", abbrev(&head));
        Ok(Exit::Success)
    } else {
        println!(
            "verification failed; fix it on the update branch in {}, commit, and run `git intake verify` again",
            state.worktree.display()
        );
        Ok(Exit::Negative)
    }
}

/// Runs the install command and the checks on `head`, the commit the update
/// worktree is at, records the verification in the state, and returns
/// whether it passed. `finish` runs it on a catch-up merge it made.
pub fn verify_head(repo: &Repo, state: &mut UpdateState, head: &str) -> Result<bool> {
    let wt = Git::worktree(&state.worktree);
    let logs = &repo.paths.logs;
    if logs.exists() {
        fs::remove_dir_all(logs).with_context(|| format!("cannot clear {}", logs.display()))?;
    }
    fs::create_dir_all(logs).with_context(|| format!("cannot create {}", logs.display()))?;

    let mut commands: Vec<(&str, &str)> = Vec::new();
    if let Some(install) = &state.config.install {
        commands.push(("install", install));
    }
    for check in &state.config.checks {
        commands.push(("check", check));
    }

    println!("verifying {} in {}", abbrev(head), state.worktree.display());
    let environment = check_environment(state, head);
    let mut results = Vec::new();
    let mut problems = Vec::new();
    if let Err(why) = merged_config_problem(&wt, head, &state.config.integration)? {
        println!("FAILED   {why}");
        problems.push(why);
    }
    // A broken merged configuration fails verification; the commands are skipped.
    let mut install_failed = !problems.is_empty();
    for (i, (kind, command)) in commands.iter().enumerate() {
        let log = logs.join(format!("{:02}-{kind}.log", i + 1));
        if install_failed {
            println!("skipped  {command}");
            results.push(CheckResult {
                kind: kind.to_string(),
                command: command.to_string(),
                outcome: CheckOutcome::Skipped,
                exit_code: None,
                log,
            });
            continue;
        }
        let code = run_logged(command, &state.worktree, &log, &environment)?;
        let outcome = if code == Some(0) {
            println!("passed   {command}  (log: {})", log.display());
            CheckOutcome::Passed
        } else {
            let status = code.map_or("terminated by a signal".to_string(), |c| {
                format!("exit {c}")
            });
            println!("FAILED   {command}  ({status}; log: {})", log.display());
            if *kind == "install" {
                install_failed = true;
            }
            CheckOutcome::Failed
        };
        results.push(CheckResult {
            kind: kind.to_string(),
            command: command.to_string(),
            outcome,
            exit_code: code,
            log,
        });
    }

    let head_after = wt.line(&["rev-parse", "HEAD"])?;
    if head_after != head {
        let why = "the update branch moved during verification; verify again".to_string();
        println!("{why}");
        problems.push(why);
    }
    let changed = wt.tracked_changes()?;
    if !changed.is_empty() {
        let why = format!(
            "the commands changed tracked files, so the verified tree would differ from the commit:\n{}",
            bullet(&changed)
        );
        println!("{why}");
        problems.push(why);
    }
    let passed = problems.is_empty() && results.iter().all(|r| r.outcome == CheckOutcome::Passed);

    state.verification = Some(Verification {
        commit: head.to_string(),
        passed,
        results,
        problems,
    });
    repo.save_state(state)?;
    Ok(passed)
}

/// Checks the configuration as it will be committed on the integration branch
/// after `finish`: it must exist, be valid, and name the same integration branch.
fn merged_config_problem(
    wt: &Git,
    head: &str,
    integration: &str,
) -> Result<std::result::Result<(), String>> {
    let Some(text) = wt.show_file(head, config::FILE_NAME)? else {
        return Ok(Err(format!("the merged tree has no {}", config::FILE_NAME)));
    };
    Ok(match config::parse(&text) {
        Err(err) => Err(format!(
            "the merged {} is invalid: {err:#}",
            config::FILE_NAME
        )),
        Ok(cfg) if cfg.integration != integration => Err(format!(
            "the merged {} names '{}' as the integration branch instead of '{integration}'",
            config::FILE_NAME,
            cfg.integration
        )),
        Ok(_) => Ok(()),
    })
}

/// One upstream as the variables describe it.
pub struct Described {
    pub name: String,
    pub mode: config::Mode,
    /// The follow target, or the newest upstream commit a port update takes
    /// or decides on.
    pub target: Option<String>,
    /// A follow upstream's last synced commit before the update.
    pub previous: Option<String>,
}

/// The variables that describe an update with the base `base`, at `head`,
/// of the integration branch `integration`, taking `upstreams` in queue
/// order, as full commit ids. `verify` and `env` give the same ones.
pub fn variables(
    base: &str,
    head: &str,
    integration: &str,
    upstreams: &[Described],
) -> Vec<(String, String)> {
    let mut vars = vec![
        (format!("{ENV_PREFIX}BASE"), base.to_string()),
        (format!("{ENV_PREFIX}HEAD"), head.to_string()),
        (format!("{ENV_PREFIX}INTEGRATION"), integration.to_string()),
        (
            format!("{ENV_PREFIX}UPSTREAMS"),
            upstreams
                .iter()
                .map(|u| u.name.as_str())
                .collect::<Vec<_>>()
                .join(" "),
        ),
    ];
    for u in upstreams {
        let prefix = format!("{ENV_PREFIX}UPSTREAM_{}_", config::env_name(&u.name));
        vars.push((format!("{prefix}MODE"), u.mode.as_str().to_string()));
        if let Some(target) = &u.target {
            vars.push((format!("{prefix}TARGET"), target.clone()));
        }
        if let Some(previous) = &u.previous {
            vars.push((format!("{prefix}PREVIOUS"), previous.clone()));
        }
    }
    vars
}

/// The environment that describes the update to the install command and the
/// checks, as full commit ids. The base is the integration tip the update
/// lands on: the tip it started from, or the one it last caught up with; a
/// catch-up changes none of the upstreams' variables. Every commit named
/// stays in the repository while they run: the base and the head are on the
/// update branch, and each upstream's commits are in the history of the
/// reference the update holds for it.
fn check_environment(state: &UpdateState, head: &str) -> Vec<(String, String)> {
    let upstreams: Vec<Described> = state
        .upstreams
        .iter()
        .map(|q| match &q.work {
            Work::Follow(f) => Described {
                name: q.name.clone(),
                mode: config::Mode::Follow,
                target: Some(f.target.clone()),
                previous: f.previous.clone(),
            },
            // The newest commit the update takes or decides on, which is
            // what it moves the upstream to in the journal; commits left
            // `later` stay pending.
            Work::Port(p) => Described {
                name: q.name.clone(),
                mode: config::Mode::Port,
                target: p
                    .commits
                    .iter()
                    .rev()
                    .find(|c| matches!(c.verb, Some(Verb::Take | Verb::Skip | Verb::Have)))
                    .map(|c| c.commit.clone()),
                previous: None,
            },
        })
        .collect();
    variables(&state.base, head, &state.config.integration, &upstreams)
}

/// Keeps the `GIT_INTAKE_` variables the tool inherited from `cmd`, so that
/// it sees only those of the update it is given.
pub fn clear_inherited(cmd: &mut Command) {
    for (var, _) in std::env::vars_os() {
        if var.to_str().is_some_and(|v| v.starts_with(ENV_PREFIX)) {
            cmd.env_remove(var);
        }
    }
}

/// Runs a shell command in `dir` with the variables of `environment`,
/// writing its output to `log`.
fn run_logged(
    command: &str,
    dir: &Path,
    log: &Path,
    environment: &[(String, String)],
) -> Result<Option<i32>> {
    let file = File::create(log).with_context(|| format!("cannot create {}", log.display()))?;
    let err = file.try_clone()?;
    let mut cmd = Command::new("sh");
    cmd.arg("-c")
        .arg(command)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(file)
        .stderr(err);
    for var in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_PREFIX",
        "GIT_COMMON_DIR",
    ] {
        cmd.env_remove(var);
    }
    clear_inherited(&mut cmd);
    cmd.envs(environment.iter().map(|(k, v)| (k, v)));
    let status = cmd
        .status()
        .with_context(|| format!("cannot run `{command}`"))?;
    Ok(status.code())
}
