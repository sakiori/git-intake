# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.5.2](https://gitlab.com/sakiori-oss/git-intake/compare/v0.5.1...v0.5.2) - 2026-11-08

### Added

- semantic versions for downstream releases

## [0.5.1](https://gitlab.com/sakiori-oss/git-intake/compare/v0.5.0...v0.5.1) - 2026-11-06

### Added

- replace an untouched update merge request with a newer release

## [0.5.0](https://gitlab.com/sakiori-oss/git-intake/compare/v0.4.1...v0.5.0) - 2026-11-06

### Added

- land an update's merge request, pausing the project's merge queue

### Fixed

- [**breaking**] push from a GitLab job with an access token, in a checkout of any owner

## [0.4.1](https://gitlab.com/sakiori-oss/git-intake/compare/v0.4.0...v0.4.1) - 2026-11-01

### Added

- `git intake release next`, `tag`, `check`, `version` and `notes` cut, check, version and describe a downstream's own releases, configured by an optional `[release]` table. A release is `v<base>-<label>.<YYYYMMDD>.<n>`, whose base is the next patch version after the newest upstream release the integration tip contains, and its pipeline creates the forge release from the notes. `status` reports the release state

### Changed

- Fetches of the publish remote no longer apply the remote's configured refspecs, so no command moves the repository's own branches or tags

## [0.4.0](https://gitlab.com/sakiori-oss/git-intake/-/compare/v0.3.7...v0.4.0)

### Added

- Release archives for macOS 11 or later on Apple silicon and on Intel, `aarch64-apple-darwin` and `x86_64-apple-darwin`

### Changed

- Releases are published as GitLab releases of the public project `sakiori-oss/git-intake`. Install with mise's `gitlab:sakiori-oss/git-intake`, `cargo binstall --git` or `cargo install --git`, or download a release archive, which comes with a `.sha256` checksum and a Sigstore signature bundle
- A build from source reports its release's version instead of `0.0.0`

## [0.3.7](https://gitlab.com/sakiori-oss/git-intake/-/compare/v0.3.6...v0.3.7)

### Added

- Count a sync record only on a merge whose second parent is its target
- Add the forge interface, the update line and the merge request description
- Publish updates as merge requests through the forge's API, with sync and env
- Name the checks and run some of them as a partial run

### Fixed

- Find the forge from the fetch URL git uses
- Take only the project's own merge requests, ordered by instant
- Judge the open merge request in status against the remote's integration branch
- Keep credentials in URLs out of every message
- Hold the CI job's API base to the remote's own host
- Read the open merge request again around the push in finish
- Tell GitHub's own pull requests by repository id
- Keep an SSH URL's user and end its authority at a query or fragment
- Compare the open merge request's branch as the remote has it in finish
- Refuse in finish when the merge request ended during the run
- Report a recreated branch that was kept, and refuse a gone branch in finish
- Name the failed deletion of a recreated branch without guessing its cause
- Name each unknown check once in a refused selection
- Tighten partial-run refusals, selections and waiting updates
- Read the update once when verify checks a selection
- Take rustls-webpki 0.103.16, which rejects distrusted emSign intermediates (RUSTSEC-2026-0382)
- Keep value names, repetition and the -- separator in the manual page synopses
- Name release tags as start's default target and scope the fetch-failure exit status
- Ship the license and third-party notices in every release archive
- Carry every copyright notice of the linked code in the release archives
- Remove only the update worktree and files the tool created
- Show no credentials or tokens in any message
- Compare remotes with upstreams as git resolves them, before every push
- Compare hosted URLs by host and path, and resolve local paths with git
- Fold names in full Unicode case at the update worktree's place
- Refuse credentials in upstream URLs and hide only token-length values
- Ignore path case on every host and resolve SSH host aliases
- Keep the backtrace of a panic, redacted
- Add every crate's further copyright lines and Zig's license to the notices
- Pack release archives deterministically, without the build host's data
- Read an upstream URL whose path ends in a non-ASCII character
- Name changes to the update's own checks when publishing by push
- Recognise every common form of copyright line in the notices
- Remap the build host's paths out of the release binary, and check every archived file for them
- Fetch zig over HTTPS only in release builds
- Name changes to the update's own checks before they run and on a resumed finish
- Keep lowercase and multi-line copyright notices whole in the notices
- Name the update's own checks when finish --no-publish lands a catch-up

### Performance

- Order a port update's commits with one rev-list

## [0.3.6](https://gitlab.com/sakiori-oss/git-intake/-/compare/v0.3.5...v0.3.6)

### Added

- Catch up an update with a moved integration branch

### Fixed

- Catch up only past the base, once per run, and recover given-up catch-ups
- Refuse a rewound local integration branch and keep commits on a detached catch-up
- Tell unpublished local commits from a rewritten remote

## [0.3.5](https://gitlab.com/sakiori-oss/git-intake/-/compare/v0.3.4...v0.3.5)

### Added

- Follow upstreams at their release tags and record the merged tag

## [0.3.4](https://gitlab.com/sakiori-oss/git-intake/-/compare/v0.3.3...v0.3.4)

### Added

- Checks receive the update's base, head and upstream targets

## [0.3.3](https://gitlab.com/sakiori-oss/git-intake/-/compare/v0.3.2...v0.3.3)

### Added

- Migrate an update in progress from the previous state format

### Fixed

- Finish opens the merge request with squashing off and treats waiting as success
- Show a target named by its commit id once
- Name the release and the format of a state this release cannot read
- Finish fails on a changed merge request and refuses --no-publish while waiting
- Warn once about another release, and name the releases of each state format

## [0.3.2](https://gitlab.com/sakiori-oss/git-intake/-/compare/v0.3.1...v0.3.2)

### Fixed

- Keep the update worktree out of the git directory
- Keep a bare repository's update worktree and find an interrupted one
- Never take a user's worktree on the update branch for the update worktree

## [0.3.1](https://gitlab.com/sakiori-oss/git-intake/-/compare/v0.3.0...v0.3.1)

### Fixed

- Keep sync records that a merge request merged with a merge commit. The tool reads its history along the integration line: the first-parent line, plus what each merge on it brought in through its second parent. `status`, `start`, `log` and `doctor` now see an update merged with a merge commit, including GitLab's semi-linear method, and a path imported on a merged branch.
- An older sync record merged later never moves an upstream's last synced commit back.

## [0.3.0](https://gitlab.com/sakiori-oss/git-intake/-/compare/v0.2.2...v0.3.0)

### Added

- [**breaking**] Take upstreams by URL, in follow or port mode. An upstream is fetched by its URL into a reference of the tool's own only while a command or an update needs it; the repository keeps no remote and no mirror branch of it.
- `port` mode takes chosen upstream commits through a plan (`take`, `skip`, `have`, `later`) and keeps a journal of the decisions at `.git-intake/<name>.toml`; commits lost to a rewritten upstream are acknowledged under `retired`.
- `contribute` creates a branch at an upstream's tip for a contribution.

### Removed

- [**breaking**] The `remote` and `mirror` settings and the `fetch` command. Give every upstream a `url`, and finish or abort an update started by an earlier release with that release.

## [0.2.2](https://gitlab.com/sakiori-oss/git-intake/-/compare/v0.2.1...v0.2.2)

### Fixed

- Say truthfully what a merge request push moved
- Push and delete the merge request branch only where each push URL needs it
- Look up the merge request branch where pushes to the publish remote go
- Push nothing when the merge request branch is already in place
- Take the main worktree from where the tool runs or from the update before guessing
- Locate the main worktree when the git directory lies outside it
- Complete finish and abort when run from inside the update worktree

## [0.2.1](https://gitlab.com/sakiori-oss/git-intake/-/compare/v0.2.0...v0.2.1)

### Added

- Find the repository's own tags even when a branch shares their name
- Resolve chosen targets only through upstream tags and check mirrors at start
- Take an upstream at a chosen revision with `start <upstream>=<rev>`

## [0.2.0](https://gitlab.com/sakiori-oss/git-intake/-/compare/v0.1.0...v0.2.0)

### Added

- Publication through a merge request: with `publish = { remote = "origin", via = "merge-request" }`, `finish` pushes the verified commit to a branch of its own and asks GitLab to open a merge request; a later `finish` completes the update once the integration branch received the verified tree, judged by the merge request's head, and refuses squash or rebase merges, merges after the target moved, and merge requests that picked up other commits
- Manual pages for every command, shipped in the release archives

### Changed

- [**breaking**] `publish` is a table with `remote` and `via` (`push` or `merge-request`); `publish = "<remote>"` becomes `publish = { remote = "<remote>" }`
- `abort` removes the update's logs and names only the branches the configuration has

## [0.1.0](https://gitlab.com/sakiori-oss/git-intake/-/tags/v0.1.0)

### Added

- `git intake`: merge-based, verified and resumable updates of a repository from one or more upstreams, each merged at the repository root or under its own path
- Configuration committed on the integration branch in `.git-intake.toml`, with validation of upstream names, remotes and paths
- Update cycle `start`, `continue`, `verify` and `finish`, with fixed targets, an isolated worktree, recorded conflict resolutions and atomic publication of exactly the verified commits; `abort` and `finish --no-publish` close an update
- Sync records in merge commit trailers, including adoption of paths imported earlier with `git subtree add`
- Reporting commands `status` (with `--check` as a cutover gate), `report`, `log`, `branches` and `doctor`, each with JSON output
- Prebuilt static binaries for `x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl` and `aarch64-apple-darwin`
