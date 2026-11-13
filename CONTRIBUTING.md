# Contributing

## Building and testing

The workspace builds with Rust 1.88 or later (`rust-version` in `Cargo.toml`). The tests run the installed `git`, which must be 2.43 or later, and a POSIX `sh`.

```sh
cargo build --locked
cargo test --workspace --locked
cargo run --locked -p xtask -- man <dir>   # writes the manual pages into <dir>
```

Every change must pass the three checks that CI runs:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

Merge request pipelines run on Linux, on the group's unprotected Linux runner, tagged `sakiori-oss-linux`, so a merge request from a fork outside the group finds no runner, and a member of this project with at least the Developer role, having read the change, runs its pipeline here. They also build the workspace with the minimum supported Rust version, run the tests against the oldest supported git, run `cargo deny check` (configured in `deny.toml`) for advisories, licenses, bans and sources, package the crate, run shellcheck on `.gitlab/scripts/`, and, when a change can affect the build, build the release archive of both Linux targets with `.gitlab/scripts/build-archive.sh` (`check-build-linux`). That script builds a Linux target with [cargo-zigbuild](https://github.com/rust-cross/cargo-zigbuild) and zig on the `PATH`, and a macOS target with plain `cargo` on a Mac with the Xcode Command Line Tools (`xcode-select --install`); one Mac builds both macOS targets. Every target needs [cargo-about](https://github.com/EmbarkStudios/cargo-about) 0.9.2 for the third-party notices (`cargo install --locked --features cli cargo-about@0.9.2`) and the toolchain's `rust-src` component; the script packs the archive itself, with `cargo run -p xtask -- pack`. `.gitlab/scripts/install-tools.sh` installs the pinned tools on x86_64 Linux, and rustup-init and cargo-about on Apple silicon macOS (`cargo run --locked -p xtask -- notices <dir> <target>` writes the notices alone). A dependency whose license `about.toml` does not accept, or whose license files changed under a clarification there, fails that build. The copyright lines in a dependency's sources that its license text lacks are added to the notices automatically, under *Additional copyright notices*. The macOS targets are built only for a release, on the project's macOS runner, never in a merge request pipeline, so a change that can affect the macOS build or behave differently on macOS needs a build and a run there by hand: `.gitlab/scripts/build-archive.sh aarch64-apple-darwin` on an Apple silicon Mac builds the archive, checks it and runs the binary it holds.

## Behaviour changes

A merge request that changes the behaviour of `git intake` describes the change and its reason, and brings the README and the tests along with it. The plan of the work belongs in the merge request.

## Commits

Commit messages follow [Conventional Commits](https://www.conventionalcommits.org/) (`feat:`, `fix:`, `docs:`, `test:`, `ci:`, `chore:` and so on, with `!` for a breaking change). Release versions and the changelog are computed from them, so the type matters: in `0.x` a breaking change raises the minor version, and a feature, fix, performance change or revert (`revert:`, or the `Revert "..."` subject `git revert` writes) the patch version. Other types, such as `docs`, `test`, `ci` or `chore`, ask for no release. Dependency updates follow the same rule: a cargo dependency update changes the shipped binary and is committed as `fix(deps)`, an update of the pipeline's pinned images or tools as `ci`. A merge request lands on `main` as one squashed commit, and release-plz reads only that commit's message: give the merge request a Conventional Commit title, which GitLab takes as the squash commit message by default, and check the message before queuing it.

## Merging

A merge request lands through the merge queue: once it is ready, a maintainer assigns it to the `sakiori-merge` service account (`glab mr update <id> --ready --assignee sakiori-merge`). The bot takes the queue one merge request at a time: it rebases each onto `main`, waits for its pipeline and merges it. Apart from the two kinds below, nobody merges a merge request by hand, sets auto-merge on it or rebases one that is queued: a merge outside the queue sends every queued merge request back to a new pipeline. When the bot cannot merge one, it assigns it back with a comment that says why; after a conflict, the branch is rebased onto `main`, the conflict resolved, and the merge request assigned again.

Two kinds stay out of the queue, and a maintainer merges them with the queue held: the queued merge requests are unassigned first and assigned again once the merge has landed.

- The release merge request belongs to release-plz, which rewrites its branch in the `release-plz-pr` job of every push pipeline of `main` ([RELEASING.md](RELEASING.md)); the bot leaves its `release-plz-*` branch alone. Nobody rebases it, by hand or with GitLab's *Rebase*: that would release a version and changelog that miss what landed since. With the queue held, wait for `release-plz-pr` of the latest `main` pipeline, or run that pipeline again, and merge the release merge request once its own pipeline passes.
- A merge request from a fork outside the group gets its pipeline in this project only when a maintainer starts it: the maintainer runs it on the merge request rebased onto the current `main` and merges it once it passes.

## Releases

Releases are made by release-plz from the Conventional Commits; [RELEASING.md](RELEASING.md) is the maintainers' runbook.

## Security

Report vulnerabilities privately, as [SECURITY.md](SECURITY.md) describes, not in a public issue.
