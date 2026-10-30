# Releasing

This is the maintainers' runbook for releases on GitHub: how a release runs, the repository settings it relies on, the first release after the move to GitHub, and how to finish a release that stopped halfway.

## How a release runs

1. Every push to `main` runs the `release-pr` job of `.github/workflows/release.yml`. release-plz opens or updates the release pull request from a branch `release-plz-*`: it raises the version in `Cargo.toml` and `Cargo.lock` and adds the release's entry to `CHANGELOG.md`, computed from the Conventional Commits since the last release (see [CONTRIBUTING.md](CONTRIBUTING.md#commits)).
2. CI runs on the release pull request like on any other, including `release-build (<target>)` for each release target: it builds, packs and checks the release archive exactly as a release does, the third-party notices included, and publishes nothing.
3. Merging the release pull request with a merge commit starts the `release` job. It first requires that every `release-build` check passed on the pull request's last commit, since a crate version, once published, cannot be replaced. release-plz then publishes the crate to crates.io through Trusted Publishing and creates the tag `vX.Y.Z` on that last commit, which is the commit release-plz releases.
4. The tag starts `.github/workflows/binaries.yml`, in a run whose commit is the tag's. It checks the tag, that its commit is on `main`, that `Cargo.toml` holds its version and that the version is on crates.io; builds the archive of every target; attests each archive's build provenance and verifies that the attestation names this repository, `binaries.yml`, the tag and its commit; creates the draft GitHub Release with the changelog entry, attaches the archives and their `.sha256` files, and publishes the release, marked as the latest unless a newer release exists.

Runs of `binaries.yml` for one tag never overlap, and only one run at a time attaches files to or publishes any release; queued runs wait instead of being cancelled. A run never replaces a file of a published release.

## Repository setup

### The GitHub App

release-plz opens the release pull request and creates the tag with a GitHub App's installation token, because pull requests and tags created with a workflow's own `GITHUB_TOKEN` start no workflow runs.

- Create the App in the `sakiori` organisation, with no webhook, and install it on the `git-intake` repository only.
- Repository permissions: *Contents: Read and write* (release branch, tag), *Pull requests: Read and write* (release pull request) and *Metadata: Read*. No other permission, in particular not *Workflows*: a release pull request never changes `.github/workflows/`.
- Keep the App's client id and one private key; store them in the environments below, not as repository or organisation secrets.

### Environments

| Environment | Deployment branches and tags | Variables | Secrets | Used by |
| --- | --- | --- | --- | --- |
| `release-pr` | Selected branches: `main` | `RELEASE_APP_CLIENT_ID` | `RELEASE_APP_PRIVATE_KEY` | `release.yml`, job `release-pr` |
| `release` | Selected branches: `main` | `RELEASE_APP_CLIENT_ID` | `RELEASE_APP_PRIVATE_KEY` | `release.yml`, job `release` (crates.io Trusted Publishing) |
| `release-binaries` | Selected tags: `v*` | none | none | `binaries.yml`, job `finish` |

Turn off *Allow administrators to bypass configured protection rules* for each of them. The App's private key is the credential that can create release tags, so it is held only by environments that admit `main` alone; a pull request's workflow, which runs on another ref, cannot read it.

### crates.io Trusted Publishing

In the crate's settings on crates.io, under *Trusted Publishing*, add a GitHub publisher:

| Field | Value |
| --- | --- |
| Repository owner | `sakiori` |
| Repository name | `git-intake` |
| Workflow filename | `release.yml` |
| Environment | `release` |

Trusted Publishing applies only to a crate that exists, so the first version is published with an API token (see *The cutover*); revoke that token once the publisher is in place. Keep no `CARGO_REGISTRY_TOKEN` secret anywhere: release-plz uses Trusted Publishing only when none is set.

### Rulesets

**Branch ruleset for `main`** (target: the default branch):

- Restrict deletions; block force pushes.
- Require a pull request before merging, with merge commits allowed: a release pull request is merged with a merge commit, so that the commit release-plz tags is the one CI checked.
- Require status checks to pass, with *Require branches to be up to date before merging*: `fmt`, `clippy`, `test (ubuntu-latest)`, `test (macos-latest)`, `test (git 2.43.0)`, `test (git 2.47.3)`, `msrv`, `deny`, and `release-build (x86_64-unknown-linux-musl)`, `release-build (aarch64-unknown-linux-musl)`, `release-build (aarch64-apple-darwin)`, `release-build (x86_64-apple-darwin)`.

**Tag ruleset for `v*`** (target: tags matching `v*`):

- Restrict creations, updates and deletions.
- Bypass list: the GitHub App, which creates every release tag, and the *Repository admin* role, for the recovery below. A tag, once created, is never moved or deleted.

## The cutover

Until the move to GitHub releases, `Cargo.toml` holds the placeholder version `0.0.0`, and both jobs of `release.yml` refuse to run. The first GitHub release is made by hand:

1. On a branch, make the cutover commit: it sets the next unreleased version `X.Y.Z` in `Cargo.toml` and `Cargo.lock` and seeds `CHANGELOG.md`. Let CI pass on its pull request, `release-build` included.
2. From a clean checkout of exactly that commit, publish the crate with an API token: `cargo publish --locked`, never `--allow-dirty`, so that the published crate records the commit it came from.
3. Create the annotated tag `vX.Y.Z` on that commit and push it, before the commit reaches `main`: `git tag -a vX.Y.Z -m vX.Y.Z <commit>` and `git push origin refs/tags/vX.Y.Z`. With the tag and the crates.io version in place, release-plz treats `X.Y.Z` as released when the commit reaches `main` and does not try to publish it again. The run of `binaries.yml` that the tag starts stops, because the commit is not on `main` yet; that is expected.
4. Merge the pull request with a merge commit, so that the tagged commit itself is on `main`.
5. Finish the release: `gh workflow run binaries.yml --ref vX.Y.Z`.
6. Add the Trusted Publisher on crates.io and revoke the API token.

## Recovery

### The crate is published, but the tag is missing

The `release` job failed after `cargo publish` and before the tag. Running it again does not help: release-plz skips a version that crates.io already has. Create the tag by hand on the commit the crate was published from, which the published crate records:

```sh
version=X.Y.Z
curl --fail --location --silent "https://static.crates.io/crates/git-intake/git-intake-$version.crate" \
  | tar -xzO "git-intake-$version/.cargo_vcs_info.json"
```

Check that the commit in `git.sha1` is the last commit of the merged release pull request and is on `main`, then, with the *Repository admin* bypass of the tag ruleset:

```sh
git tag -a "v$version" -m "v$version" <commit>
git push origin "refs/tags/v$version"
```

A tag pushed by a person starts `binaries.yml`, which finishes the release.

### The tag exists, but the release is unfinished

A run of `binaries.yml` failed, or the tag was created by a token whose events start no workflow. Run the workflow on the tag:

```sh
gh workflow run binaries.yml --ref vX.Y.Z
```

It builds, attests and attaches every archive again, replacing the files of the draft release, and publishes it; it creates the draft when it is missing. A run on any ref other than a release tag, or on a tag whose commit is not the run's, is refused. A release that is published already is left as it is: if a published release ever lacks a file, investigate before changing it by hand rather than replacing its files.

## Checking a release

Anyone can check that an archive was built by this repository's release workflow from the tag's commit:

```sh
gh attestation verify git-intake-vX.Y.Z-<target>.tar.gz \
  --repo sakiori/git-intake \
  --signer-workflow sakiori/git-intake/.github/workflows/binaries.yml \
  --source-ref refs/tags/vX.Y.Z
```

## Pinned versions

The workflows pin some tools by hand; Dependabot updates only the actions. Each workflow's header lists its pins: release-plz in `release.yml`; zig, cargo-zigbuild and cargo-about in `ci.yml` and `binaries.yml`, and cargo-about also in `xtask/src/main.rs`, which refuses to generate the notices with another release; cargo-deny in `ci.yml`.
