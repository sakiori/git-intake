//! `git intake`: verified, resumable updates of a downstream repository from
//! one or more upstreams, merged as a whole or ported commit by commit.

mod cli;
mod commands;
mod config;
mod description;
mod drift;
mod forge;
mod git;
mod integration_line;
mod output;
mod port;
mod repo;
mod state;
mod sync_record;
mod tags;
mod target;
mod update_line;
mod upstreams;

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
        Command::Start { upstreams } => commands::update::start(&repo, &upstreams),
        Command::Continue => commands::update::resume(&repo),
        Command::Abort => commands::update::abort(&repo),
        Command::Verify => commands::verify::run(&repo),
        Command::Finish { no_publish } => commands::finish::run(&repo, no_publish),
        Command::Migrate => commands::migrate::run(&repo),
        Command::Status { check, json } => commands::status::run(&repo, check, json),
        Command::Report { json } => commands::report::run(&repo, json),
        Command::Log { upstreams, json } => commands::log::run(&repo, &upstreams, json),
        Command::Branches { upstreams, json } => commands::branches::run(&repo, &upstreams, json),
        Command::Doctor { json } => commands::doctor::run(&repo, json),
        Command::Contribute { upstream, branch } => {
            commands::contribute::run(&repo, &upstream, &branch)
        }
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
