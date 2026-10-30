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
        conflicts, an integration branch that moved again while finish verified a catch-up merge, or \
        an upstream sync left out); 2 on an error (invalid configuration, a refused command, a git or \
        forge failure, or invalid usage). env -- <command> exits with the command's status."
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
    /// left it, and is refused once `finish` has moved it. Never closes a merge request: with no
    /// update in progress in merge-request mode, it names the update's open merge request, which
    /// `--merge-request` withdraws.
    Abort {
        /// Also withdraw the update's merge request through the forge: close it, and delete the
        /// update's branch on each push URL while it points at the commit read
        #[arg(long)]
        merge_request: bool,
    },
    /// Run the install command and checks on the update
    ///
    /// Runs the install command, then the update's checks in their order, and records the verified
    /// commit when every check passes. The update's checks are those of the configuration at the
    /// integration tip at `start`, or at the last catch-up merge. Each command's output goes to a log
    /// named after it, `install.log` or `check-<NAME>.log`.
    ///
    /// `--only` and `--skip` select checks by name. A run that leaves out a check is a partial run:
    /// it runs the install command and the selected checks the same way, is recorded beside the
    /// verification with logs of its own, and never verifies the update, nor changes or completes
    /// its verification. A later partial run replaces the earlier one; a full run and a catch-up
    /// merge drop it.
    ///
    /// Each command runs through `sh -c` in the update worktree with variables that describe
    /// the update, as full commit ids: `GIT_INTAKE_BASE`, the integration tip the update started
    /// from, or the one it last caught up with; `GIT_INTAKE_HEAD`, the commit being verified; `GIT_INTAKE_INTEGRATION`, the
    /// integration branch; `GIT_INTAKE_UPSTREAMS`, the names of the upstreams the update takes,
    /// separated by spaces. For each of them, `GIT_INTAKE_UPSTREAM_<NAME>_MODE` is `follow` or
    /// `port`, and `GIT_INTAKE_UPSTREAM_<NAME>_TARGET` is the target merged, or for a port the
    /// newest upstream commit the update takes or decides on; a follow upstream with a last synced
    /// commit before the update also gets it as `GIT_INTAKE_UPSTREAM_<NAME>_PREVIOUS`. `<NAME>` is
    /// the upstream's name in upper case with every character other than a letter or a digit
    /// replaced by `_`. `env` gives a merge request pipeline the same variables.
    Verify {
        /// Run only this check; repeat it to select several
        #[arg(long, value_name = "NAME", conflicts_with = "skip")]
        only: Vec<String>,
        /// Leave this check out; repeat it to leave out several
        #[arg(long, value_name = "NAME")]
        skip: Vec<String>,
    },
    /// Publish the update by push or as a merge request, and complete it
    ///
    /// When the integration branch moved past the update's base, `finish` first catches up: it
    /// merges the update into the new tip with a merge commit that carries an `Intake-Catch-Up`
    /// trailer. It makes at most one catch-up merge per run; a conflict stops it for `continue`.
    ///
    /// With `via = "push"`, `finish` requires a verified update, which a partial run of `verify`
    /// does not make, and verifies a catch-up merge itself, running every check. It fetches the publish remote, fast-forwards a local integration branch strictly
    /// behind it, fast-forwards the integration branch to the verified commit, then pushes it
    /// without force.
    ///
    /// With `via = "merge-request"`, `finish` requires no local verification: the merge request's
    /// pipeline verifies the update. It reads the forge's merge settings and refuses a project
    /// that does not merge only a tree its required pipeline tested, catches up with the publish
    /// remote's integration branch without running the checks, pushes the update head without
    /// force to the branch `git-intake/update`, opens the merge request through the forge's API,
    /// or rewrites the title and description of the open one, and removes the local update: the
    /// merge request carries it from then on. No local branch moves. The forge's token comes from
    /// `GITLAB_TOKEN`, or `GH_TOKEN` or `GITHUB_TOKEN` (`GH_ENTERPRISE_TOKEN` or
    /// `GITHUB_ENTERPRISE_TOKEN` for GitHub Enterprise Server).
    ///
    /// An update an earlier release left waiting for its merge request is handed over to the
    /// merge request instead.
    Finish {
        /// Complete only the local moves and close the update without fetching or pushing; a
        /// catch-up takes in only the local integration branch, and after a rejected push the
        /// update lands a verified catch-up merge, or closes at the planned commit after dropping
        /// an unverified or conflicted one
        #[arg(long)]
        no_publish: bool,
    },
    /// Take the follow upstreams in without local state, as a scheduled CI job does
    ///
    /// Brings the local integration branch up to date with the publish remote's, creating it when
    /// missing. In merge-request mode, with the update's merge request open, it catches the
    /// merge request up with the integration branch when that moved, by a catch-up merge pushed
    /// without force, and otherwise changes nothing. Without an open merge request it starts an
    /// update of every follow upstream the integration branch lacks the target of and publishes
    /// it as `finish` does, unless a person closed the last merge request of those very targets
    /// without merging it. Merge-request mode runs no install command and no check. In push mode
    /// it starts, verifies and publishes in one run. Upstreams it cannot take are left out and
    /// named; port upstreams with pending commits are named, never taken. A conflict, or failed
    /// checks in push mode, stop it with the update in progress, as `start` stops.
    Sync,
    /// Print or pass on the update variables of a commit, read from history
    ///
    /// Sets, for COMMIT (`HEAD` by default), the `GIT_INTAKE_*` variables `verify` gives the
    /// checks, read from history alone, so that a merge request pipeline gives its checks what
    /// `verify` gives them locally. The integration tip is the publish remote's, fetched for the
    /// command, or the local integration branch without a publish remote. A commit the
    /// integration branch contains, or that carries no update, gets no variable.
    ///
    /// Prints one `export` line per variable by default; `--dotenv` prints `NAME=value` lines
    /// for a GitLab dotenv report or GitHub's environment file; `--json` one JSON object. With a
    /// command after `--`, runs it with the variables and exits with its status, or with status 2
    /// without running it when the variables cannot be read.
    Env {
        /// The commit to describe; `HEAD` when none is given
        #[arg(value_name = "COMMIT")]
        rev: Option<String>,
        /// Print `NAME=value` lines, unquoted
        #[arg(long, conflicts_with_all = ["json", "command"])]
        dotenv: bool,
        /// Write one JSON object to standard output
        #[arg(long, conflicts_with = "command")]
        json: bool,
        /// The command to run with the variables
        #[arg(last = true, value_name = "COMMAND")]
        command: Vec<String>,
    },
    /// Fetch every upstream and show its drift, and the update in progress
    ///
    /// A follow upstream with `tags` is compared with its newest release tag instead of its
    /// branch tip, and shows the state `no release` while its branch has none. In merge-request
    /// mode it also shows the update's open merge request, and the newest one merged without the
    /// update's history.
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
    ///
    /// In merge-request mode it also checks that the forge is recognised, that a token is set
    /// and accepted, and that the merge settings qualify.
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
