//! `git intake`: verified, resumable updates of a downstream repository from
//! one or more upstreams, merged as a whole or ported commit by commit.

// Messages go through `output::say` and `output::esay`, results through
// `output::print`.
#![warn(clippy::print_stdout, clippy::print_stderr)]

mod bump;
mod cli;
mod commands;
mod config;
mod description;
mod drift;
mod forge;
mod git;
mod identity;
mod integration_line;
mod output;
mod pause;
mod port;
mod redact;
mod release;
mod repo;
mod state;
mod sync_record;
mod tags;
mod target;
mod update_line;
mod upstreams;

use std::process::ExitCode;

use clap::Parser;

use cli::{Cli, Command, ReleaseCommand};
use output::esay;

/// The outcome of a command that ran to completion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    /// Exit status 0.
    Success,
    /// Exit status 1: a negative result such as drift, a failed check or a
    /// stop on conflicts.
    Negative,
    /// The exit status of a command `env` ran.
    Code(u8),
}

/// Exit status for errors: invalid configuration, refused command, git failure.
const ERROR_STATUS: u8 = 2;

fn run(command: Command) -> anyhow::Result<Exit> {
    let repo = repo::Repo::open()?;
    // The integration line needs the full history: a shallow clone cuts it
    // off, and an upstream's last sync record may lie below the cut. Only
    // `abort`, which ends an update, works without it.
    if !matches!(command, Command::Abort { .. }) {
        repo.refuse_shallow()?;
    }
    let result = match command {
        Command::Start { upstreams } => commands::update::start(&repo, &upstreams),
        Command::Continue => commands::update::resume(&repo),
        Command::Abort { merge_request } => commands::update::abort(&repo, merge_request),
        Command::Verify { only, skip } => commands::verify::run(&repo, &only, &skip),
        Command::Finish => commands::finish::run(&repo),
        Command::Sync => commands::sync::run(&repo),
        Command::Land {
            resume,
            timeout,
            poll,
        } => commands::land::run(
            &repo,
            resume,
            commands::land::Waits {
                timeout: std::time::Duration::from_secs(timeout),
                poll: std::time::Duration::from_secs(poll),
            },
        ),
        Command::Env {
            rev,
            dotenv,
            json,
            command,
        } => commands::env::run(&repo, rev.as_deref(), dotenv, json, &command),
        Command::Delta { rev } => commands::delta::run(&repo, rev.as_deref()),
        Command::Conflicts { since, paths, json } => {
            commands::conflicts::run(&repo, since.as_deref(), &paths, json)
        }
        Command::Status { check, json } => commands::status::run(&repo, check, json),
        Command::Report { json } => commands::report::run(&repo, json),
        Command::Log { upstreams, json } => commands::log::run(&repo, &upstreams, json),
        Command::Branches { upstreams, json } => commands::branches::run(&repo, &upstreams, json),
        Command::Doctor { json } => commands::doctor::run(&repo, json),
        Command::Contribute { upstream, branch } => {
            commands::contribute::run(&repo, &upstream, &branch)
        }
        Command::Release { command } => match command {
            ReleaseCommand::Next { json } => commands::release::next(&repo, json),
            ReleaseCommand::Tag => commands::release::tag(&repo),
            ReleaseCommand::Check { tag } => commands::release::check(&repo, tag.as_deref()),
            ReleaseCommand::Version { tag, json, dotenv } => {
                commands::release::version(&repo, tag.as_deref(), json, dotenv)
            }
            ReleaseCommand::Notes { tag } => commands::release::notes(&repo, tag.as_deref()),
        },
    };
    repo.note_removed_directory();
    result
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    // No message shows a token the tool may read, whatever quotes it.
    for name in forge::TOKEN_VARIABLES {
        if let Ok(value) = std::env::var(name) {
            redact::hold(value.trim());
        }
    }
    // A panic's message goes the same way as every other.
    std::panic::set_hook(Box::new(|info| {
        esay!("{}", redact::redacted(&info.to_string()));
        let backtrace = std::backtrace::Backtrace::capture();
        if backtrace.status() == std::backtrace::BacktraceStatus::Captured {
            esay!("{}", redact::redacted(&backtrace.to_string()));
        }
    }));
    match run(cli.command) {
        Ok(Exit::Success) => ExitCode::SUCCESS,
        Ok(Exit::Negative) => ExitCode::from(1),
        Ok(Exit::Code(code)) => ExitCode::from(code),
        Err(err) => {
            esay!("error: {}", redact::redacted(&format!("{err:#}")));
            ExitCode::from(ERROR_STATUS)
        }
    }
}
