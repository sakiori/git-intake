//! `verify`: run the install command and checks on the update.

use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};

use crate::Exit;
use crate::commands::update::{require_merging, require_worktree};
use crate::config::{self, Check};
use crate::git::{Git, abbrev};
use crate::port::Verb;
use crate::repo::{Repo, bullet};
use crate::state::{
    CheckOutcome, CheckResult, CommandResult, PartialRun, UpdateState, Verification, Work,
};

/// Prefix of the environment variables the install command and the checks
/// receive. Variables with this prefix inherited from the caller are not
/// passed on, so a command sees only those of the update it verifies.
pub const ENV_PREFIX: &str = "GIT_INTAKE_";

pub fn run(repo: &Repo, only: &[String], skip: &[String]) -> Result<Exit> {
    let mut state = repo.require_state()?;
    let selected: Vec<Check> = select(&state.config.checks, only, skip)?
        .into_iter()
        .cloned()
        .collect();
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
    if selected.len() < state.config.checks.len() {
        return partial_run(repo, &mut state, &head, &selected);
    }
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

/// The checks a run takes, in the order of the update's checks: those
/// `only` names, or all but those `skip` names. A name given twice counts
/// once. Refuses a name that is not one of the update's checks, and both
/// selections at once.
fn select<'a>(checks: &'a [Check], only: &[String], skip: &[String]) -> Result<Vec<&'a Check>> {
    if !only.is_empty() && !skip.is_empty() {
        bail!("`--only` and `--skip` cannot be given together; nothing was run");
    }
    let mut unknown: Vec<&str> = only
        .iter()
        .chain(skip)
        .map(String::as_str)
        .filter(|n| !checks.iter().any(|c| c.name == *n))
        .collect();
    unknown.dedup();
    if !unknown.is_empty() {
        let known = if checks.is_empty() {
            "the update has no checks".to_string()
        } else {
            format!(
                "the update's checks are {}",
                checks
                    .iter()
                    .map(|c| format!("'{}'", c.name))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        bail!(
            "unknown check {}; {known}. Nothing was run",
            unknown
                .iter()
                .map(|n| format!("'{n}'"))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    Ok(checks
        .iter()
        .filter(|c| {
            if only.is_empty() {
                !skip.contains(&c.name)
            } else {
                only.contains(&c.name)
            }
        })
        .collect())
}

/// Runs the install command and the checks `selected`, which leave out some
/// of the update's checks, on `head`, and records the run as the update's
/// partial run, replacing an earlier one with its logs. The verification
/// stays as it was.
fn partial_run(
    repo: &Repo,
    state: &mut UpdateState,
    head: &str,
    selected: &[Check],
) -> Result<Exit> {
    let left_out: Vec<String> = state
        .config
        .checks
        .iter()
        .filter(|c| !selected.contains(c))
        .map(|c| c.name.clone())
        .collect();
    let ran = if selected.is_empty() {
        "no check".to_string()
    } else {
        selected
            .iter()
            .map(|c| c.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };
    println!(
        "running {ran} on {} in {}, leaving out {}",
        abbrev(head),
        state.worktree.display(),
        left_out.join(", ")
    );
    let run = run_checks(state, head, selected, &repo.paths.partial_logs)?;
    let passed = run.passed;
    state.partial_run = Some(PartialRun { run, left_out });
    repo.save_state(state)?;
    let verified = state
        .verification
        .as_ref()
        .is_some_and(|v| v.passed && v.commit == head);
    let outcome = if passed { "passed" } else { "failed" };
    let standing = if verified {
        format!(
            "the update stays verified at {}, which a full run verified",
            abbrev(head)
        )
    } else {
        "the update is not verified; run `git intake verify` without `--only` or `--skip` to run every check \
         and verify it"
            .to_string()
    };
    println!(
        "this partial run {outcome} and did not verify the update: it left out some of the update's checks; {standing}"
    );
    if !passed {
        println!(
            "fix the failures on the update branch in {}, commit, and run `git intake verify` again",
            state.worktree.display()
        );
    }
    Ok(if passed {
        Exit::Success
    } else {
        Exit::Negative
    })
}

/// Runs the install command and every check on `head`, the commit the
/// update worktree is at, records the verification in the state, drops the
/// partial run, and returns whether it passed. `finish` runs it on a
/// catch-up merge it made.
pub fn verify_head(repo: &Repo, state: &mut UpdateState, head: &str) -> Result<bool> {
    println!("verifying {} in {}", abbrev(head), state.worktree.display());
    let checks = state.config.checks.clone();
    let run = run_checks(state, head, &checks, &repo.paths.verify_logs)?;
    let passed = run.passed;
    state.verification = Some(run);
    repo.drop_partial_run(state)?;
    repo.save_state(state)?;
    Ok(passed)
}

/// Runs the install command and `checks` on `head`, writing their logs into
/// `logs`, which is cleared first: the install command's to `install.log`,
/// each check's to `check-<name>.log`, so that no check's log takes the
/// install command's place.
fn run_checks(
    state: &UpdateState,
    head: &str,
    checks: &[Check],
    logs: &Path,
) -> Result<Verification> {
    let wt = Git::worktree(&state.worktree);
    if logs.exists() {
        fs::remove_dir_all(logs).with_context(|| format!("cannot clear {}", logs.display()))?;
    }
    fs::create_dir_all(logs).with_context(|| format!("cannot create {}", logs.display()))?;

    let environment = check_environment(state, head);
    let mut problems = Vec::new();
    if let Err(why) = merged_config_problem(&wt, head, &state.config.integration)? {
        println!("FAILED   {why}");
        problems.push(why);
    }
    // A broken merged configuration fails the run; the commands are skipped.
    let mut skip = !problems.is_empty();
    let install = match &state.config.install {
        Some(install) => {
            let log = logs.join("install.log");
            let result = run_command(
                "install command",
                install,
                log,
                skip,
                &state.worktree,
                &environment,
            )?;
            // The checks are skipped when the install command failed.
            skip = skip || result.outcome != CheckOutcome::Passed;
            Some(result)
        }
        None => None,
    };
    let mut results = Vec::new();
    for check in checks {
        let log = logs.join(format!("check-{}.log", check.name));
        results.push(CheckResult {
            name: check.name.clone(),
            result: run_command(
                &check.name,
                &check.run,
                log,
                skip,
                &state.worktree,
                &environment,
            )?,
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
    let passed = problems.is_empty()
        && install
            .iter()
            .chain(results.iter().map(|r| &r.result))
            .all(|r| r.outcome == CheckOutcome::Passed);
    Ok(Verification {
        commit: head.to_string(),
        passed,
        install,
        checks: results,
        problems,
    })
}

/// Runs one command of a run and reports it by `label`, or reports it
/// skipped without running it.
fn run_command(
    label: &str,
    command: &str,
    log: PathBuf,
    skipped: bool,
    dir: &Path,
    environment: &[(String, String)],
) -> Result<CommandResult> {
    if skipped {
        println!("skipped  {label}");
        return Ok(CommandResult {
            outcome: CheckOutcome::Skipped,
            exit_code: None,
            log,
        });
    }
    let code = run_logged(command, dir, &log, environment)?;
    let outcome = if code == Some(0) {
        println!("passed   {label}  (log: {})", log.display());
        CheckOutcome::Passed
    } else {
        let status = code.map_or("terminated by a signal".to_string(), |c| {
            format!("exit {c}")
        });
        println!("FAILED   {label}  ({status}; log: {})", log.display());
        CheckOutcome::Failed
    };
    Ok(CommandResult {
        outcome,
        exit_code: code,
        log,
    })
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
