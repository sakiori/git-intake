//! The command-line interface. This module depends on clap alone, so that the
//! manual page generator (`xtask`) builds the same definition without the rest
//! of the tool.

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "git-intake",
    bin_name = "git intake",
    version,
    about = "Merge-based, verified, resumable updates of a repository from its upstreams",
    long_about = "Merge-based, verified, resumable updates of a repository from its upstreams.\n\n\
        Each upstream is merged at the repository root or under its own path, in a separate worktree, \
        with a sync record in the merge commit. The repository's own checks run on exactly the tree \
        that gets published, and the integration branch moves only by fast-forward. The repository \
        describes itself in a .git-intake.toml committed on its integration branch.",
    after_long_help = "Exit status: 0 on success; 1 on a negative result (drift found by status --check, \
        a failed doctor check, a failed verify, an upstream that could not be fetched, a stop on conflicts, \
        or an update that still waits for its merge request); 2 on an error (invalid configuration, \
        a refused command, a git failure, or invalid usage)."
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
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
    /// Remove the update worktree, branch, logs and state; withdraw a waiting merge request
    ///
    /// Leaves the integration and mirror branches as they were, and is refused once `finish` has
    /// moved a branch. While the update waits for its merge request, it deletes the merge request
    /// branch from the publish remote; the merge request itself is closed on the forge.
    Abort,
    /// Run the install command and checks on the merged update
    Verify,
    /// Publish the verified update by push or merge request, and complete it
    ///
    /// With `via = "push"`, fast-forwards the integration branch to the verified commit and each
    /// merged upstream's mirror branch to its target, then pushes them in one atomic push without
    /// force.
    ///
    /// With `via = "merge-request"`, pushes the verified commit to the branch
    /// `git-intake/update-<commit>` on the publish remote, requests a merge request into the
    /// integration branch, and moves no local branch; the update then waits. Once the merge request
    /// is merged with a merge commit or by fast-forward, `finish` fast-forwards the integration
    /// branch to the remote one, advances and pushes the mirror branches, and deletes the merge
    /// request branch. While the merge request waits, `finish` exits with status 1.
    Finish {
        /// Complete only the local moves and close the update without pushing
        #[arg(long)]
        no_publish: bool,
    },
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
