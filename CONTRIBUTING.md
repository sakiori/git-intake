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

CI also builds the workspace with the minimum supported Rust version, runs the tests against the oldest supported git, and runs `cargo deny check` (configured in `deny.toml`) for advisories, licenses, bans and sources.

## Behaviour changes

The behaviour of `git intake` is specified with [OpenSpec](https://github.com/Fission-AI/OpenSpec) under `openspec/`: `openspec/specs/` holds the current specification, and `openspec/changes/archive/` the changes that led to it. A change of behaviour starts as a change under `openspec/changes/<name>/`, with a `proposal.md` and the spec deltas under `specs/`, following the repository's `proposal-spec` schema. The code, tests and README follow the agreed change, and the change is archived into the specification when it is done. Fixes that restore specified behaviour, refactorings and documentation need no change.

## Commits

Commit messages follow [Conventional Commits](https://www.conventionalcommits.org/) (`feat:`, `fix:`, `docs:`, `test:`, `ci:`, `chore:` and so on, with `!` for a breaking change). Release versions and the changelog are computed from them, so the type matters: in `0.x` a breaking change raises the minor version, and a feature, fix, performance change or revert (`revert:`, or the `Revert "..."` subject `git revert` writes) the patch version. Other types, such as `docs`, `test`, `ci` or `chore`, ask for no release. Dependabot's updates follow the same rule: a cargo dependency update changes the shipped binary and is committed as `fix(deps)`, a GitHub Actions update as `ci`.

## Security

Report vulnerabilities privately, as [SECURITY.md](SECURITY.md) describes, not in a public issue.
