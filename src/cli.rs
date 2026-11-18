//! The command-line interface. This module depends on clap alone, so that the
//! manual page generator (`xtask`) builds the same definition without the rest
//! of the tool.

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "git-intake",
    bin_name = "git intake",
    version,
    about = "Verified, resumable updates of a repository from its upstreams, and its releases",
    long_about = "Verified, resumable updates of a repository from its upstreams, and its releases.\n\n\
        Each upstream is fetched by its URL, with no remote kept for it, and taken in one of two modes. \
        In follow mode an update merges it at the repository root or under its own path, with a sync \
        record in the merge commit. In port mode an update copies the upstream commits a plan takes, \
        and a journal on the integration branch records the commits skipped or already present. The \
        work happens in a separate worktree, the repository's own checks run on exactly the tree that \
        gets published, and the integration branch moves only by fast-forward. With a [release] \
        table, the release commands cut, check and version releases of the integration branch \
        whose versions name the upstream release they carry. The repository describes itself in a \
        .git-intake.toml committed on its integration branch, which names the publish remote: the \
        integration branch is always read from that remote, fetched by every command, and no local \
        integration branch is read, created or moved.",
    after_long_help = "Exit status: 0 on success; 1 on a negative result (drift found by status --check, \
        a failed doctor check, a failed verify, an upstream that status, log or branches could not \
        fetch, a stop on conflicts or on the failed commit of a copy, failed checks in finish or \
        sync, a replacement that sync found conflicting, an integration branch that moved again while \
        finish made or verified a catch-up merge, an upstream sync left out, a failed or canceled pipeline \
        or a catch-up conflict in land, a release at the integration tip that release tag finds \
        unpublished, or a problem release check finds); 2 on an error (invalid configuration, a refused \
        command, such as start with an upstream that cannot be fetched, a git or forge failure, a release \
        tag another run pushed first, land after its timeout, or invalid usage). env -- <command> exits with the command's status."
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Start an update: fix targets and merge, or plan the commits to port
    ///
    /// Fetches the publish remote's integration branch, whose tip becomes the update's base, and
    /// reads the configuration there. The update branch `git-intake/update` is created at the base
    /// in the worktree `.worktrees/git-intake/update`; no other branch and no file of the main
    /// checkout changes.
    ///
    /// Fetches the selected upstreams. A follow upstream gets a fixed target: its fetched branch
    /// tip, or, with `tags` in its configuration, the newest release tag on that branch: of the
    /// upstream's tags that match the pattern and are a semantic version without build metadata,
    /// and without a prerelease part unless the upstream sets `prereleases = true`, after the
    /// pattern's literal prefix and an optional `v` or `V`, the highest version. With
    /// `max_conflicts`, it is the newest such tag the integration branch lacks whose merge is
    /// predicted to conflict in at most that many files, or the oldest of them when none is, and
    /// `start` names the tags it passed over. For an upstream named as
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
        /// Upstream names, each optionally with the revision to take it at; all upstreams when none
        /// is given. A follow upstream named without a revision is taken at its fetched tip, or with
        /// `tags` at its newest release tag
        #[arg(value_name = "UPSTREAM[=REV]", value_parser = parse_upstream_target)]
        upstreams: Vec<UpstreamTarget>,
    },
    /// Carry out the plan, or conclude a resolved merge, copy or catch-up merge, and take in the rest
    Continue,
    /// Remove the update worktree, branch, plan, references, logs and state
    ///
    /// Works at any step: nothing of an update is published before `finish` pushes it, and what it
    /// pushed stays published. Never closes a merge request: with no update in progress in
    /// merge-request mode, it names the update's open merge request, which `--merge-request`
    /// withdraws.
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
    /// Fetches the publish remote's integration branch. When it moved past the update's base,
    /// `finish` first catches up: it merges the update into the new tip with a merge commit that
    /// carries an `Intake-Catch-Up` trailer. It makes at most one catch-up merge per run; a
    /// conflict stops it for `continue`. A tip that does not contain the base was rewritten, and
    /// `finish` refuses it.
    ///
    /// With `via = "push"`, `finish` requires a verified update, which a partial run of `verify`
    /// does not make, and verifies a catch-up merge itself, running every check. It then pushes
    /// exactly the verified commit to the publish remote's integration branch, without force, and
    /// moves no local branch. A rejected push publishes nothing and keeps the update; `finish` run
    /// again pushes again, or catches up first when the remote moved.
    ///
    /// With `via = "merge-request"`, `finish` requires no local verification: the merge request's
    /// pipeline verifies the update. It reads the forge's merge settings and refuses a project
    /// that does not merge only a tree its required pipeline tested, catches up with the publish
    /// remote's integration branch without running the checks, pushes the update head without
    /// force to the branch `git-intake/update` on the publish remote, opens the merge request through the forge's API,
    /// or rewrites the title and description of the open one, and removes the local update: the
    /// merge request carries it from then on. No local branch moves. The forge's token comes from
    /// `GITLAB_TOKEN`, or `GH_TOKEN` or `GITHUB_TOKEN` (`GH_ENTERPRISE_TOKEN` or
    /// `GITHUB_ENTERPRISE_TOKEN` for GitHub Enterprise Server).
    ///
    /// An update an earlier release left waiting for its merge request is handed over to the
    /// merge request instead.
    Finish,
    /// Take the follow upstreams in without local state, as a scheduled CI job does
    ///
    /// Reads the integration branch from the publish remote, as every command does, and creates or
    /// moves no local branch but the update's. In merge-request mode, with the update's merge
    /// request open, it catches the merge request up with the integration branch when that moved,
    /// by a catch-up merge pushed without force, and otherwise changes nothing; with `replace_after`, it replaces an untouched merge
    /// request open that long with a new one when an upstream with `tags` published a newer release,
    /// unless a pause of the merge queue is recorded. Without an open merge request it starts an update
    /// of every follow upstream the integration branch lacks the target of and publishes it as `finish`
    /// does, unless a person closed the last merge request of those very targets without merging it.
    /// Merge-request mode runs no install command and no check. In push mode it starts, verifies and
    /// publishes in one run. Upstreams it cannot take are left out and named; port upstreams with
    /// pending commits are named, never taken. A conflict, or failed checks in push mode, stop it with
    /// the update in progress, as `start` stops.
    Sync,
    /// Carry the update's open merge request to the integration branch
    ///
    /// Works from any clone, without an update in progress. Waits for the pipeline of the merge
    /// request's head. When it passed and the head contains the integration tip, merges that
    /// head, naming it so that a head moved meanwhile is not merged, never squashed and never
    /// rebased, and exits once the forge reports the merge. When it passed and the integration
    /// branch moved, catches the merge request up by a catch-up merge as `sync` does and waits for
    /// the new pipeline. A failed or canceled pipeline, or a conflict in the catch-up merge, stops
    /// it with status 1; `continue`, `finish` and `land` carry on after a conflict. Before merging
    /// it reads the forge's merge settings and refuses as `finish` does.
    ///
    /// With `queue` in `[publish]`, the account of a merge queue that takes the merge requests
    /// assigned to it, `land` pauses the queue once the update's pipeline passed and the
    /// integration branch moved, unless the merge request cannot merge anyway or the catch-up
    /// will conflict: it records the queue's merge requests, in the order of their latest
    /// assignment to the account, in the reference refs/git-intake/pause on the publish remote,
    /// then takes the account's assignment from each but one the queue is landing, which it
    /// waits for. A merge request assigned to the queue during the pause is held too. The pause
    /// ends on every way out, SIGINT, SIGTERM and SIGHUP included: each held merge request still
    /// open and assigned as the pause left it gets the account's assignment back, in the
    /// recorded order, at least a second apart, and the reference goes.
    Land {
        /// End the pause the publish remote records, left by an interrupted run, and do nothing
        /// else; works in any state of an update, whichever merge request the pause began for
        #[arg(long)]
        resume: bool,
        /// Give up, with status 2, after this long: seconds, or a number with `s`, `m` or `h`
        #[arg(long, value_name = "DURATION", default_value = "6h", value_parser = parse_duration)]
        timeout: u64,
        /// Wait this many seconds between two reads of the forge
        #[arg(long, value_name = "SECONDS", default_value_t = 60)]
        poll: u64,
    },
    /// Print or pass on the update variables of a commit, read from history
    ///
    /// Sets, for COMMIT (`HEAD` by default), the `GIT_INTAKE_*` variables `verify` gives the
    /// checks, read from history alone, so that a merge request pipeline gives its checks what
    /// `verify` gives them locally. The integration tip is the publish remote's, fetched for the
    /// command. A commit the integration branch contains, or that carries no update, gets no
    /// variable.
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
    /// Show how the update changed the downstream's changes on top of its upstream
    ///
    /// Compares, with `git range-diff`, two one-commit series: the tree of the update's base on
    /// the upstream's previous target, and the tree of COMMIT (`HEAD` by default) on its target.
    /// What the update changed in the downstream's delta shows; the upstream's changes that
    /// merged cleanly do not. COMMIT is resolved from history as `env` resolves it, so the
    /// command works in an update in progress, on an update's merge request head and in a merge
    /// request pipeline. The two commits it makes are reachable from no reference. It refuses
    /// a commit that carries no update, an update of anything but exactly one follow upstream at
    /// the repository root, and an upstream without a previous target.
    Delta {
        /// The commit to compare; `HEAD` when none is given
        #[arg(value_name = "COMMIT")]
        rev: Option<String>,
    },
    /// List the files the updates on the integration branch needed resolving in
    ///
    /// Recomputes, from each merge's parents with `git merge-tree`, the files that conflicted in
    /// the sync merges and catch-up merges on the integration line, including a sync merge that
    /// lies on a catch-up merge's second parent, and prints them one per line, sorted and
    /// without repeats. The integration tip is the publish remote's, fetched for the command.
    Conflicts {
        /// Only the merges the integration branch received after COMMIT; its whole history
        /// when none is given
        #[arg(long, value_name = "COMMIT")]
        since: Option<String>,
        /// Keep the paths that match this glob, where `*` stays within a directory and `**`
        /// crosses them; repeat it to keep the paths that match any of several
        #[arg(long = "path", value_name = "GLOB")]
        paths: Vec<String>,
        /// Write one JSON document, with each merge, its upstream or `catch_up`, and its paths
        #[arg(long)]
        json: bool,
    },
    /// Fetch every upstream and show its drift, and the update in progress
    ///
    /// A follow upstream with `tags` is compared with its newest release tag instead of its
    /// branch tip, and shows the state `no release` while its branch has none. With `--check`
    /// and `max_conflicts`, it names the tag an update would take and predicts that tag's
    /// conflicts. In merge-request
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
    /// Cut, check and version the releases of the integration branch
    ///
    /// With a `[release]` table in the configuration, the integration branch is released. With
    /// `versioning = "dated"`, the default, a release is `v<base>-<label>.<YYYYMMDD>.<n>`.
    /// `<base>` is the next patch version after the newest release of the release upstream that
    /// the commit contains: a tag of that upstream matching the release `tags` pattern whose name,
    /// after the pattern's literal prefix and an optional `v` or `V`, is a semantic version
    /// without a prerelease part or build metadata. `<label>` is the first of `labels`,
    /// `<YYYYMMDD>` the UTC day, and `<n>` one past the highest number any tag of release form of
    /// that day took, from 1 to 99. Every release sorts after the earlier ones, and
    /// `YYYYMMDD * 100 + n` grows with every release.
    ///
    /// With `versioning = "semver"`, a release is `v<x.y.z>`, raised from the previous release
    /// by the largest change among the commits since it, merges excluded and the upstream commits
    /// that updates brought in included: a breaking change raises the major version, `feat` the
    /// minor version, `fix`, `perf`, `revert` and a subject that is not a Conventional Commit the
    /// patch version, and other types nothing. An upstream release outside the caret range of
    /// the previous release's upstream release raises the major version. The first semver release
    /// is 1.0.0, or the next major version above earlier releases at 1.0.0 or above; dated
    /// releases with one of `labels` stay releases. A semver tag records the upstream release
    /// it carries in the trailers `Intake-Upstream` and `Intake-Tag` of its annotation, which
    /// later versions compare with.
    ///
    /// Releases count only on the downstream's line: the first-parent commits of the publish
    /// remote's integration branch that the last synced commit of no upstream contains. A
    /// release is published when the forge of the publish remote has a release for its tag,
    /// which the project's pipeline creates as its last step. The commands read the forge with
    /// `GITLAB_TOKEN`, or in a GitLab CI job of the remote's own host with `CI_JOB_TOKEN`, and on
    /// GitHub with the variables `finish` reads; without a token they ask without credentials.
    ///
    /// The commands fetch the publish remote's integration branch and tags into references of
    /// their own, and change no local branch or tag.
    Release {
        #[command(subcommand)]
        command: ReleaseCommand,
    },
}

/// The commands of `release`.
#[derive(Subcommand)]
pub enum ReleaseCommand {
    /// Show the release at the publish remote's integration tip, or the release a tag would take
    ///
    /// Reads the configuration at the publish remote's integration tip. Shows the newest
    /// upstream release the tip contains and, with dated versions, the release base from it, and
    /// either the release at the tip and whether it is published, or the next release, with semver
    /// versions the change that sets it or that none is due, and, when the newest release below
    /// the tip is not published, that release. Changes nothing.
    Next {
        /// Write one JSON object to standard output
        #[arg(long)]
        json: bool,
    },
    /// Cut the next release tag on the publish remote's integration tip
    ///
    /// Reads the configuration at the publish remote's integration tip. When no release points
    /// at the tip, creates the next release as an annotated tag `Release <version>` through the
    /// API of the publish remote's forge, with the token in `GITLAB_TOKEN` or, on GitHub, the
    /// variables `finish` reads, only where the forge has no tag of its name, with a warning when
    /// the newest release below the tip is not published. On GitLab the tagger is the token's
    /// account, on GitHub git's identity. When a published release points at the tip, does
    /// nothing. When a release that is not published points at the tip, creates nothing and exits
    /// with status 1: its pipeline is still running or failed, and a failed release is completed
    /// by running its pipeline's failed jobs again. With semver versions, when the commits since
    /// the previous release ask for no release, says so and creates nothing. Never moves,
    /// replaces or deletes a tag.
    Tag {
        /// Create nothing; check that the token may create the tag the next release takes, or,
        /// when none is due, a tag of the same form, and exit with status 1 when it may not
        #[arg(long)]
        dry_run: bool,
    },
    /// Check a release tag in its pipeline before anything is built
    ///
    /// Reads the configuration at the checked-out commit and checks TAG, or else
    /// `CI_COMMIT_TAG`: it is of release form, with dated versions with the current label, the
    /// publish remote has it, it points at the checked-out commit, that commit is on the
    /// downstream's line, the tag's base is the commit's release base or, with semver versions,
    /// its version is the one the commit takes, and it sorts after every other release, by
    /// version and, with dated versions, by day and number. Only then does it ask the forge
    /// whether the release is published already, which is a problem as well. Exits with status 1
    /// when it finds a problem.
    Check {
        /// The release tag; `CI_COMMIT_TAG` when none is given
        #[arg(value_name = "TAG")]
        tag: Option<String>,
    },
    /// Print the version a build makes
    ///
    /// Reads the configuration at the checked-out commit. For a release tag, named by `--tag` or
    /// `CI_COMMIT_TAG`, prints the tag's version, refusing a tag that is not of release form, with
    /// dated versions one without the current label or the release base of the tagged commit,
    /// with semver versions one without the version the tagged commit takes. Without one, prints
    /// the rehearsal version of the checked-out commit, `<base>-<label>.<YYYYMMDD>.0` or with
    /// semver versions `<next>-rehearsal.<YYYYMMDD>`, with the UTC day of `CI_PIPELINE_CREATED_AT`,
    /// or else of now, which no release ever takes.
    Version {
        /// The release tag; `CI_COMMIT_TAG` when none is given
        #[arg(long, value_name = "TAG")]
        tag: Option<String>,
        /// Write one JSON object to standard output
        #[arg(long, conflicts_with = "dotenv")]
        json: bool,
        /// Print `RELEASE_VERSION=<version>`, for a release tag `RELEASE_TAG=<tag>`, and with
        /// semver versions `RELEASE_UPSTREAM=<upstream>-<x.y.z>`, for a GitLab dotenv report or
        /// GitHub's environment file
        #[arg(long)]
        dotenv: bool,
    },
    /// Write the Markdown release notes of a release tag
    ///
    /// Reads the configuration at the checked-out commit. For TAG, or else `CI_COMMIT_TAG`,
    /// lists, newest first, the downstream's own changes since the previous release on the
    /// integration line: each commit's subject, and each update of an upstream by its name and
    /// the upstream tag it took, or else the target's abbreviated commit. The first release says
    /// so instead. With semver versions, a first line names the upstream release the tag carries
    /// and how many upstream commits after it, and breaking changes come first.
    Notes {
        /// The release tag; `CI_COMMIT_TAG` when none is given
        #[arg(value_name = "TAG")]
        tag: Option<String>,
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

/// Parses a duration as seconds, or as a number followed by `s`, `m` or
/// `h`, into seconds.
fn parse_duration(arg: &str) -> Result<u64, String> {
    let (number, unit) = match arg.char_indices().last() {
        Some((at, 's')) => (&arg[..at], 1),
        Some((at, 'm')) => (&arg[..at], 60),
        Some((at, 'h')) => (&arg[..at], 3600),
        _ => (arg, 1),
    };
    number
        .parse::<u64>()
        .ok()
        .and_then(|n| n.checked_mul(unit))
        .ok_or_else(|| format!("'{arg}' is not a duration such as 90, 30m or 6h"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_are_seconds_minutes_or_hours() {
        assert_eq!(parse_duration("90"), Ok(90));
        assert_eq!(parse_duration("90s"), Ok(90));
        assert_eq!(parse_duration("30m"), Ok(1800));
        assert_eq!(parse_duration("6h"), Ok(21600));
        assert!(parse_duration("h").is_err());
        assert!(parse_duration("1.5h").is_err());
        assert!(parse_duration("-1").is_err());
    }

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
