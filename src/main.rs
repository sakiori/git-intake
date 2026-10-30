//! `git fork-sync`: merge-based, verified, resumable updates of a downstream
//! repository from one or more upstreams.

mod commands;
mod config;
mod drift;
mod git;
mod output;
mod remotes;
mod repo;
mod state;
mod sync_record;

use std::process::ExitCode;

use clap::{Parser, Subcommand};

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

#[derive(Parser)]
#[command(
    name = "git-fork-sync",
    bin_name = "git fork-sync",
    version,
    about = "Merge-based, verified, resumable updates of a repository from its upstreams"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Fetch the named upstreams, or all of them, adding missing remotes
    Fetch {
        /// Upstream names; all upstreams when none is given
        upstreams: Vec<String>,
    },
    /// Start an update: fix targets and merge each upstream with new commits
    Start {
        /// Upstream names; all upstreams when none is given
        upstreams: Vec<String>,
    },
    /// Conclude a resolved merge and merge the remaining queued upstreams
    Continue,
    /// Remove the update worktree, branch and state
    Abort,
    /// Run the install command and checks on the merged update
    Verify,
    /// Fast-forward and publish the verified update
    Finish,
    /// Show each upstream's drift and the update in progress
    Status {
        /// Fetch every upstream, predict conflicts, and fail unless all are current
        #[arg(long)]
        check: bool,
        /// Write one JSON document to standard output
        #[arg(long)]
        json: bool,
    },
    /// Summarise the update in progress or the one just finished
    Report {
        /// Write one JSON document to standard output
        #[arg(long)]
        json: bool,
    },
    /// List upstream commits the integration branch does not contain
    Log {
        /// Upstream names; all upstreams when none is given
        upstreams: Vec<String>,
        /// Write one JSON document to standard output
        #[arg(long)]
        json: bool,
    },
    /// List upstream branches holding commits the integration branch lacks
    Branches {
        /// Upstream names; all upstreams when none is given
        upstreams: Vec<String>,
        /// Write one JSON document to standard output
        #[arg(long)]
        json: bool,
    },
    /// Check the configuration, remotes, history and update state
    Doctor {
        /// Write one JSON document to standard output
        #[arg(long)]
        json: bool,
    },
}

fn run(command: Command) -> anyhow::Result<Exit> {
    let repo = repo::Repo::open()?;
    match command {
        Command::Fetch { upstreams } => commands::fetch::run(&repo, &upstreams),
        Command::Start { upstreams } => commands::update::start(&repo, &upstreams),
        Command::Continue => commands::update::resume(&repo),
        Command::Abort => commands::update::abort(&repo),
        Command::Verify => commands::verify::run(&repo),
        Command::Finish => commands::finish::run(&repo),
        Command::Status { check, json } => commands::status::run(&repo, check, json),
        Command::Report { json } => commands::report::run(&repo, json),
        Command::Log { upstreams, json } => commands::log::run(&repo, &upstreams, json),
        Command::Branches { upstreams, json } => commands::branches::run(&repo, &upstreams, json),
        Command::Doctor { json } => commands::doctor::run(&repo, json),
    }
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
