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

CI also builds the workspace with the minimum supported Rust version, runs the tests against the oldest supported git, runs `cargo deny check` (configured in `deny.toml`) for advisories, licenses, bans and sources, and builds the release archive of every release target with `.github/scripts/build-archive.sh`, which needs [cargo-about](https://github.com/EmbarkStudios/cargo-about) 0.9.2 for the third-party notices (`cargo install --locked --features cli cargo-about@0.9.2`; `cargo run --locked -p xtask -- notices <file>` writes them alone). A dependency whose license `about.toml` does not accept, or whose license files changed under a clarification there, fails that build.

## Behaviour changes

The behaviour of `git intake` is specified with [OpenSpec](https://github.com/Fission-AI/OpenSpec) under `openspec/`: `openspec/specs/git-intake/spec.md` holds the current specification, and `openspec/changes/archive/` the changes that led to it. A change of behaviour starts as a change under `openspec/changes/<name>/`. The code, tests and README follow the agreed change, and the change is archived into the specification when it is done. Fixes that restore specified behaviour, refactorings and documentation need no change.

The specification is checked with the OpenSpec CLI 1.14.3, which needs Node.js and runs without an install as `npx -y @fission-ai/openspec@1.14.3`; below, `openspec` stands for that command. A change follows the repository's schema [`proposal-spec`](openspec/schemas/proposal-spec/schema.yaml), which `openspec/config.yaml` selects, with the templates in [`openspec/schemas/proposal-spec/templates/`](openspec/schemas/proposal-spec/templates/). It has two parts:

- `proposal.md`: why the change is needed, what it changes and which capabilities it affects;
- `specs/git-intake/spec.md`: the spec deltas, requirements under `## ADDED Requirements`, `## MODIFIED Requirements` (with the requirement's full new text) or `## REMOVED Requirements`, each with `#### Scenario:` blocks of **WHEN** and **THEN** lines.

Add a `design.md` beside them only when the change makes a consequential technical choice that needs its reasons written down: one with real alternatives, a reason later readers need, and a cost to reverse it, as the archived changes' designs record. A change has no `tasks.md`; the plan of the work belongs in the pull request.

```sh
openspec new change keep-tags-on-sync --schema proposal-spec   # creates openspec/changes/keep-tags-on-sync/
# write proposal.md and specs/git-intake/spec.md from the templates
openspec validate keep-tags-on-sync --type change --strict
# once the code, tests and README follow the change:
openspec archive keep-tags-on-sync --yes                       # merges the deltas into openspec/specs/ and moves the change to openspec/changes/archive/
openspec validate git-intake --type spec --strict
```

## Commits

Commit messages follow [Conventional Commits](https://www.conventionalcommits.org/) (`feat:`, `fix:`, `docs:`, `test:`, `ci:`, `chore:` and so on, with `!` for a breaking change). Release versions and the changelog are computed from them, so the type matters: in `0.x` a breaking change raises the minor version, and a feature, fix, performance change or revert (`revert:`, or the `Revert "..."` subject `git revert` writes) the patch version. Other types, such as `docs`, `test`, `ci` or `chore`, ask for no release. Dependabot's updates follow the same rule: a cargo dependency update changes the shipped binary and is committed as `fix(deps)`, a GitHub Actions update as `ci`.

## Releases

Releases are made by release-plz from the Conventional Commits; [RELEASING.md](RELEASING.md) is the maintainers' runbook.

## Security

Report vulnerabilities privately, as [SECURITY.md](SECURITY.md) describes, not in a public issue.
