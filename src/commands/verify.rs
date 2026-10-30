//! `verify`: run the install command and checks on the merged update.

use std::fs::{self, File};
use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};

use crate::Exit;
use crate::commands::update::require_worktree;
use crate::git::{Git, abbrev};
use crate::repo::{Repo, bullet};
use crate::state::{CheckOutcome, CheckResult, UpstreamStep, Verification};

pub fn run(repo: &Repo) -> Result<Exit> {
    let mut state = repo.require_state()?;
    require_worktree(repo, &state)?;
    let open: Vec<String> = state
        .unmerged()
        .iter()
        .map(|u| {
            let step = if u.step == UpstreamStep::Conflict {
                "in conflict"
            } else {
                "pending"
            };
            format!("{} ({step})", u.name)
        })
        .collect();
    if !open.is_empty() {
        bail!(
            "cannot verify before every queued upstream is merged; these still need `git fork-sync continue`:\n{}",
            bullet(&open)
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

    println!(
        "verifying {} in {}",
        abbrev(&head),
        state.worktree.display()
    );
    let mut results = Vec::new();
    let mut install_failed = false;
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
        let code = run_logged(command, &state.worktree, &log)?;
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

    let mut passed = results.iter().all(|r| r.outcome == CheckOutcome::Passed);
    let head_after = wt.line(&["rev-parse", "HEAD"])?;
    if head_after != head {
        passed = false;
        println!("the update branch moved during verification; verify again");
    }
    let changed = wt.tracked_changes()?;
    if !changed.is_empty() {
        passed = false;
        println!(
            "the commands changed tracked files, so the verified tree would differ from the commit:\n{}",
            bullet(&changed)
        );
    }

    state.verification = Some(Verification {
        commit: head.clone(),
        passed,
        results,
    });
    repo.save_state(&state)?;
    if passed {
        println!("verified {}; next: git fork-sync finish", abbrev(&head));
        Ok(Exit::Success)
    } else {
        println!(
            "verification failed; fix it on the update branch in {}, commit, and run `git fork-sync verify` again",
            state.worktree.display()
        );
        Ok(Exit::Negative)
    }
}

/// Runs a shell command in `dir`, writing its output to `log`.
fn run_logged(command: &str, dir: &Path, log: &Path) -> Result<Option<i32>> {
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
    let status = cmd
        .status()
        .with_context(|| format!("cannot run `{command}`"))?;
    Ok(status.code())
}
