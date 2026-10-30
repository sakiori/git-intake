//! The command-line interface. This module depends on clap alone, so that the
//! manual page generator (`xtask`) builds the same definition without the rest
//! of the tool.

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "git-intake",
    bin_name = "git intake",
    version,
    about = "Verified, resumable updates of a repository from its upstreams",
    long_about = "Verified, resumable updates of a repository from its upstreams.\n\n\
        Each upstream is fetched by its URL, with no remote kept for it, and taken in one of two modes. \
        In follow mode an update merges it at the repository root or under its own path, with a sync \
        record in the merge commit. In port mode an update copies the upstream commits a plan takes, \
        and a journal on the integration branch records the commits skipped or already present. The \
        work happens in a separate worktree, the repository's own checks run on exactly the tree that \
        gets published, and the integration branch moves only by fast-forward. The repository \
        describes itself in a .git-intake.toml committed on its integration branch.",
    after_long_help = "Exit status: 0 on success; 1 on a negative result (drift found by status --check, \
        a failed doctor check, a failed verify, an upstream that could not be fetched, a stop on \
        conflicts, an integration branch that moved again while finish verified a catch-up merge, or a \
        merge request that no longer holds the verified commit); 2 on an error (invalid configuration, \
        a refused command, a git failure, or invalid usage)."
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Start an update: fix targets and merge, or plan the commits to port
    ///
    /// First brings the integration branch up to date with the publish remote's when it is strictly
    /// behind it, by fast-forward, and refuses when the two have diverged.
    ///
    /// Fetches the selected upstreams. A follow upstream gets a fixed target: its fetched branch
    /// tip, or, with `tags` in its configuration, the newest release tag on that branch: of the
    /// upstream's tags that match the pattern and are a semantic version without a prerelease
    /// part or build metadata after the pattern's literal prefix and an optional `v` or `V`, the
    /// highest version. For an upstream named as
    /// `UPSTREAM=REV`, the target is the commit REV names. REV is a tag of the upstream's
    /// repository (its URL is asked for its tags), a commit id, or anything else git resolves,
    /// but not one of the repository's own tags; its commit must be on the fetched upstream branch
    /// and contain the upstream's last synced commit. Each follow upstream whose target the
    /// integration branch lacks is merged in the update worktree with its own merge commit, whose
    /// sync record names the upstream tag of the target, if any, in an `Intake-Tag` trailer.
    ///
    /// A port upstream gets a plan of its pending commits, limited to those REV contains when one
    /// is named; the update stops for the plan to be edited, and `continue` carries it out.
    Start {
        /// Upstream names, each optionally with the revision to take it at; all upstreams at their
        /// fetched tips when none is given
        #[arg(value_name = "UPSTREAM[=REV]", value_parser = parse_upstream_target)]
        upstreams: Vec<UpstreamTarget>,
    },
    /// Carry out the plan, or conclude a resolved merge, copy or catch-up merge, and take in the rest
    Continue,
    /// Remove the update worktree, branch, plan, references, logs and state
    ///
    /// Leaves the integration branch where `start`, or the last fast-forward to the publish remote,
    /// left it, and is refused once `finish` has moved it. While the update waits for its merge
    /// request, it deletes the merge request branch from the publish remote; the merge request
    /// itself is closed on the forge.
    Abort,
    /// Run the install command and checks on the update
    ///
    /// Each command runs through `sh -c` in the update worktree with variables that describe
    /// the update, as full commit ids: `GIT_INTAKE_BASE`, the integration tip the update started
    /// from, or the one it last caught up with; `GIT_INTAKE_HEAD`, the commit being verified; `GIT_INTAKE_INTEGRATION`, the
    /// integration branch; `GIT_INTAKE_UPSTREAMS`, the names of the upstreams the update takes,
    /// separated by spaces. For each of them, `GIT_INTAKE_UPSTREAM_<NAME>_MODE` is `follow` or
    /// `port`, and `GIT_INTAKE_UPSTREAM_<NAME>_TARGET` is the target merged, or for a port the
    /// newest commit of the plan; a follow upstream with a last synced commit before the update
    /// also gets it as `GIT_INTAKE_UPSTREAM_<NAME>_PREVIOUS`. `<NAME>` is the upstream's name in
    /// upper case with every character other than a letter or a digit replaced by `_`.
    Verify,
    /// Publish the verified update by push or merge request, and complete it
    ///
    /// When the integration branch moved past the update's base, `finish` first catches up: it
    /// merges the update into the new tip with a merge commit that carries an `Intake-Catch-Up`
    /// trailer, runs the install command and the checks on it, and publishes it once they pass.
    /// It makes at most one catch-up merge per run; a conflict stops it for `continue`, and an
    /// integration branch that moves again while it verifies stops it for another run.
    ///
    /// With `via = "push"`, fetches the publish remote, fast-forwards a local integration branch
    /// strictly behind it, fast-forwards the integration branch to the verified commit, then pushes
    /// it without force.
    ///
    /// With `via = "merge-request"`, pushes the verified commit to the branch `git-intake/update` on
    /// the publish remote, requests a merge request into the integration branch with squashing
    /// off, and moves no local branch; the update then waits. When the remote's integration branch
    /// moves before the merge, `finish` catches up with it and pushes the result to the same
    /// branch without force, so that the open merge request carries it. Once the merge request is
    /// merged with a merge commit or by fast-forward, `finish` fast-forwards the integration branch
    /// to the remote one and deletes the merge request branch. While the merge request waits,
    /// `finish` reports so and exits with status 0, or with status 1 when the merge request no
    /// longer holds the verified commit; `status --check` fails until the update completes, and
    /// `--no-publish` is refused until the merge.
    Finish {
        /// Complete only the local moves and close the update without fetching or pushing; a
        /// catch-up takes in only the local integration branch, and after a rejected push the
        /// update lands a verified catch-up merge, or closes at the planned commit after dropping
        /// one still in conflict
        #[arg(long)]
        no_publish: bool,
    },
    /// Carry an update in progress over from state format 3
    ///
    /// Rewrites the state of an update started by a release with state format 3 in the current
    /// format, in place, keeping its branch, commits, merge in progress, index, uncommitted and untracked
    /// files, plan, logs and references. An update worktree in the git directory moves to
    /// `.worktrees/git-intake/update` in the main worktree; stop the processes, shells and editors
    /// that use it before running this command. A verification run in the old location is dropped
    /// unless `finish` has begun. Run again after an interruption, it completes the migration. An
    /// update of state format 4 needs no migration: this release carries it on.
    Migrate,
    /// Fetch every upstream and show its drift, and the update in progress
    ///
    /// A follow upstream with `tags` is compared with its newest release tag instead of its
    /// branch tip, and shows the state `no release` while its branch has none.
    Status {
        /// Predict conflicts, and fail unless every upstream is current
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
    /// List the upstream commits the integration branch lacks, or still to decide on
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
    /// Check the configuration, upstreams, history, journals and update state
    Doctor {
        /// Write one JSON document to standard output
        #[arg(long)]
        json: bool,
    },
    /// Create a local branch at an upstream's fetched tip, for a contribution to it
    ///
    /// Fetches the upstream and creates BRANCH at its fetched tip, or at the commit REV names on
    /// the fetched branch, resolved as for `start`. The branch holds only upstream history; copy
    /// the change onto it and push it to where pull requests to the upstream come from. No tracking
    /// branch is set, no checkout changes, and the tool never pushes the branch.
    Contribute {
        /// The upstream, optionally with the revision to start the branch at
        #[arg(value_name = "UPSTREAM[=REV]", value_parser = parse_upstream_target)]
        upstream: UpstreamTarget,
        /// The new local branch
        branch: String,
    },
}

/// An upstream named on the command line of `start` or `contribute`, with
/// the revision to take it at when one is given.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpstreamTarget {
    pub name: String,
    pub rev: Option<String>,
}

/// Parses `UPSTREAM` or `UPSTREAM=REV`. An upstream name cannot contain `=`,
/// so the first `=` ends the name and the rest is the revision.
fn parse_upstream_target(arg: &str) -> Result<UpstreamTarget, String> {
    match arg.split_once('=') {
        None => Ok(UpstreamTarget {
            name: arg.to_string(),
            rev: None,
        }),
        Some(("", _)) => Err("the upstream name before '=' is empty".to_string()),
        Some((_, "")) => Err("the revision after '=' is empty".to_string()),
        Some((name, rev)) => Ok(UpstreamTarget {
            name: name.to_string(),
            rev: Some(rev.to_string()),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upstream_targets_split_at_the_first_equals_sign() {
        assert_eq!(
            parse_upstream_target("app-a").unwrap(),
            UpstreamTarget {
                name: "app-a".into(),
                rev: None
            }
        );
        assert_eq!(
            parse_upstream_target("app-a=refs/tags/a=b").unwrap(),
            UpstreamTarget {
                name: "app-a".into(),
                rev: Some("refs/tags/a=b".into())
            }
        );
        assert!(parse_upstream_target("=v1").is_err());
        assert!(parse_upstream_target("app-a=").is_err());
    }
}
