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

Merge request pipelines run on Linux. They also build the workspace with the minimum supported Rust version, run the tests against the oldest supported git, run `cargo deny check` (configured in `deny.toml`) for advisories, licenses, bans and sources, validate the OpenSpec specification, package the crate, run shellcheck on `.gitlab/scripts/`, and, when a change can affect the build, build the release archive of both Linux targets with `.gitlab/scripts/build-archive.sh` (`check-build-linux`). That script builds a Linux target with [cargo-zigbuild](https://github.com/rust-cross/cargo-zigbuild) and zig on the `PATH`, and a macOS target with plain `cargo` on a Mac with the Xcode Command Line Tools (`xcode-select --install`); one Mac builds both macOS targets. Every target needs [cargo-about](https://github.com/EmbarkStudios/cargo-about) 0.9.2 for the third-party notices (`cargo install --locked --features cli cargo-about@0.9.2`) and the toolchain's `rust-src` component; the script packs the archive itself, with `cargo run -p xtask -- pack`. `.gitlab/scripts/install-tools.sh` installs the pinned tools on x86_64 Linux, and rustup-init and cargo-about on Apple silicon macOS (`cargo run --locked -p xtask -- notices <dir> <target>` writes the notices alone). A dependency whose license `about.toml` does not accept, or whose license files changed under a clarification there, fails that build. The copyright lines in a dependency's sources that its license text lacks are added to the notices automatically, under *Additional copyright notices*. The macOS targets are built only for a release, on the project's macOS runner, never in a merge request pipeline, so a change that can affect the macOS build or behave differently on macOS needs a build and a run there by hand: `.gitlab/scripts/build-archive.sh aarch64-apple-darwin` on an Apple silicon Mac builds the archive, checks it and runs the binary it holds.

## Behaviour changes

The behaviour of `git intake` is specified with [OpenSpec](https://github.com/Fission-AI/OpenSpec) under `openspec/`: `openspec/specs/git-intake/spec.md` holds the current specification, and `openspec/changes/archive/` the changes that led to it. A change of behaviour starts as a change under `openspec/changes/<name>/`. The code, tests and README follow the agreed change, and the change is archived into the specification when it is done. Fixes that restore specified behaviour, refactorings and documentation need no change.

The specification is checked with the OpenSpec CLI that `.gitlab/openspec/package.json` pins, with its dependencies locked in `package-lock.json` beside it. It needs Node.js 20.19 or later; `npm ci --prefix .gitlab/openspec --ignore-scripts` installs it, and below, `openspec` stands for `.gitlab/openspec/node_modules/.bin/openspec`. A change follows the repository's schema [`proposal-spec`](openspec/schemas/proposal-spec/schema.yaml), which `openspec/config.yaml` selects, with the templates in [`openspec/schemas/proposal-spec/templates/`](openspec/schemas/proposal-spec/templates/). It has two parts:

- `proposal.md`: why the change is needed, what it changes and which capabilities it affects;
- `specs/git-intake/spec.md`: the spec deltas, requirements under `## ADDED Requirements`, `## MODIFIED Requirements` (with the requirement's full new text) or `## REMOVED Requirements`, each with `#### Scenario:` blocks of **WHEN** and **THEN** lines.

Add a `design.md` beside them only when the change makes a consequential technical choice that needs its reasons written down: one with real alternatives, a reason later readers need, and a cost to reverse it, as the archived changes' designs record. A change has no `tasks.md`; the plan of the work belongs in the merge request.

```sh
openspec new change keep-tags-on-sync --schema proposal-spec   # creates openspec/changes/keep-tags-on-sync/
# write proposal.md and specs/git-intake/spec.md from the templates
openspec validate keep-tags-on-sync --type change --strict
# once the code, tests and README follow the change:
openspec archive keep-tags-on-sync --yes                       # merges the deltas into openspec/specs/ and moves the change to openspec/changes/archive/
openspec validate git-intake --type spec --strict
```

## Commits

Commit messages follow [Conventional Commits](https://www.conventionalcommits.org/) (`feat:`, `fix:`, `docs:`, `test:`, `ci:`, `chore:` and so on, with `!` for a breaking change). Release versions and the changelog are computed from them, so the type matters: in `0.x` a breaking change raises the minor version, and a feature, fix, performance change or revert (`revert:`, or the `Revert "..."` subject `git revert` writes) the patch version. Other types, such as `docs`, `test`, `ci` or `chore`, ask for no release. Dependency updates follow the same rule: a cargo dependency update changes the shipped binary and is committed as `fix(deps)`, an update of the pipeline's pinned images or tools as `ci`. A merge request lands on `main` as one squashed commit, and release-plz reads only that commit's message: give the merge request a Conventional Commit title, which GitLab takes as the squash commit message by default, and check the message before merging.

## Releases

Releases are made by release-plz from the Conventional Commits; [RELEASING.md](RELEASING.md) is the maintainers' runbook.

## Security

Report vulnerabilities privately, as [SECURITY.md](SECURITY.md) describes, not in a public issue.
