//! `git intake`: merge-based, verified, resumable updates of a downstream
//! repository from one or more upstreams.

mod cli;
mod commands;
mod config;
mod drift;
mod git;
mod output;
mod remotes;
mod repo;
mod state;
mod sync_record;
mod target;

use std::process::ExitCode;

use clap::Parser;

use cli::{Cli, Command};

/// The outcome of a command that ran to completion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    /// Exit status 0.
    Success,
    /// Exit status 1: a negative result such as drift, a failed check or a
    /// stop on conflicts.
    Negative,
}

/// Exit status for errors: invalid configuration, refused command, git failure.
const ERROR_STATUS: u8 = 2;

fn run(command: Command) -> anyhow::Result<Exit> {
    let repo = repo::Repo::open()?;
    let result = match command {
        Command::Fetch { upstreams } => commands::fetch::run(&repo, &upstreams),
        Command::Start { upstreams } => commands::update::start(&repo, &upstreams),
        Command::Continue => commands::update::resume(&repo),
        Command::Abort => commands::update::abort(&repo),
        Command::Verify => commands::verify::run(&repo),
        Command::Finish { no_publish } => commands::finish::run(&repo, no_publish),
        Command::Status { check, json } => commands::status::run(&repo, check, json),
        Command::Report { json } => commands::report::run(&repo, json),
        Command::Log { upstreams, json } => commands::log::run(&repo, &upstreams, json),
        Command::Branches { upstreams, json } => commands::branches::run(&repo, &upstreams, json),
        Command::Doctor { json } => commands::doctor::run(&repo, json),
    };
    repo.note_removed_directory();
    result
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli.command) {
        Ok(Exit::Success) => ExitCode::SUCCESS,
        Ok(Exit::Negative) => ExitCode::from(1),
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::from(ERROR_STATUS)
        }
    }
}
