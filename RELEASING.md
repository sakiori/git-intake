# Releasing

This is the maintainers' runbook for releases on GitLab.com: how a release runs, the group and project settings it relies on, the Linux and macOS runners, the first release, how to finish a release that stopped halfway, and the later move to crates.io.

## How a release runs

1. Every push to `main` runs the jobs `release-plz-release` and then `release-plz-pr` of `.gitlab-ci.yml`, with release-plz 0.4.2 installed from its release archive after checking the archive's SHA-256 (`.gitlab/scripts/install-tools.sh`). `release-plz-pr` opens or updates the release merge request from a branch `release-plz-*`: it raises the version in `Cargo.toml` and `Cargo.lock` and adds the release's entry to `CHANGELOG.md`, computed from the Conventional Commits since the last release tag (see [CONTRIBUTING.md](CONTRIBUTING.md#commits)). Its commit is authored and committed as `Sakiori <oss@sakiori.app>` and pushed by the release service account with its token in `RELEASE_PLZ_TOKEN`, which also opens the merge request (see *The release token*).
2. The release merge request's pipeline runs like any other: the checks, and `check-build-linux`, which builds, packs and checks the release archive of each Linux target exactly as a release does and publishes nothing. The macOS targets are not built before the release tag (see *The macOS runner*).
3. Merging the release merge request releases. In the push pipeline of `main`, `release-plz-release` finds that `Cargo.toml` holds a version `X.Y.Z` without a tag `vX.Y.Z` and creates that tag on the merged commit with the token; `release-plz.toml` sets `git_only`, so the release tags, not a registry, tell which versions are released. Any commit that sets an untagged version releases the same way, so only the release merge request changes the version.
4. The tag starts the tag pipeline:
   - `release-guard` refuses a tag that is not a protected tag `vX.Y.Z`, does not point at the pipeline's commit, whose commit is not on `main` or whose `Cargo.toml` holds another version, a version without a `CHANGELOG.md` entry, and a tag whose GitLab Release exists already. Nothing is built before it passes.
   - `release-build-linux` builds the archives of the two Linux targets on the group's protected Linux runner, from a fresh clone of the tag's commit alone, without a cache, and `release-build-macos` those of the two macOS targets on the project's macOS runner.
   - `sign` waits for all four archives, then writes `SHA256SUMS` over the archives and signs every archive and `SHA256SUMS` with cosign, keyless: the job's GitLab ID token (audience `sigstore`) obtains a short-lived Sigstore certificate whose identity is `https://gitlab.com/sakiori-oss/git-intake//.gitlab-ci.yml@refs/tags/vX.Y.Z`, and each signature is recorded in Sigstore's transparency log. It writes `<file>.sigstore.json` beside each file and verifies every bundle against that identity. It is the only job that holds an ID token.
   - `publish` uploads every file to the generic package registry, package `git-intake`, version `vX.Y.Z`, and then creates the GitLab Release with the changelog entry and a link to every file. Each link is named exactly as its file, which mise's GitLab backend matches, and has the direct asset path `/binaries/<file>`, which gives the file the permanent URL `https://gitlab.com/sakiori-oss/git-intake/-/releases/vX.Y.Z/downloads/binaries/<file>` that cargo-binstall and the README use. The release is created last, so it never lists a file that is missing. One `publish` job runs at a time.

The Linux targets are built on the group's protected x86_64 Linux runner (`sakiori-oss-linux-protected`) with cargo-zigbuild, which links them statically with zig 0.17.0 against the musl zig ships. The macOS targets are built natively with Cargo and the Xcode Command Line Tools in a fresh macOS virtual machine on the project's macOS runner, for macOS 11 or later (`MACOSX_DEPLOYMENT_TARGET`), linked against the system's Security and CoreFoundation frameworks; Apple's SDK license permits building against the SDK only on Apple hardware, so they are not cross-compiled. Every target is built with the Rust release that `RUST_VERSION` names. The build remaps the build host's paths out of the binary (`--remap-path-prefix` for the workspace, `CARGO_HOME` and the toolchain's sysroot, since Cargo's `trim-paths` is not stable; for macOS also from the object paths that Apple's linker hashes into the binary's UUID, with `-oso_prefix`, so that the same commit gives the same binary in any directory), and `xtask pack` packs the archive deterministically: GNU tar headers without PAX records, sorted names, owner and group 0 without names, every time the commit's time, modes 0755 and 0644, no extended attributes, and gzip without a file name or time. `xtask check-archive` reads the raw headers back and scans every file, the binary included, for absolute paths of the build host, the Xcode tools' on macOS included. The third-party notices are generated for the target. The `x86_64-unknown-linux-musl`, `aarch64-apple-darwin` and `x86_64-apple-darwin` binaries are run once to check that they report the release's version, the last through Rosetta; the `aarch64-unknown-linux-musl` binary is not run in CI, and the tests run on Linux only.

The crate is not published to crates.io yet; see *Publishing on crates.io*.

## Group and project settings

### The group `sakiori-oss`

- Public, with at least two owners, so that the project never depends on one account.
- *Settings > Packages and registries > Duplicate packages*: in the *Generic* row, turn off *Allow duplicates*, so that no file of a published package can be uploaded again under the same name.

### The project `sakiori-oss/git-intake`

- *Settings > General > Visibility*: public, with the package registry visible to everyone, so that the release links download without a token.
- *Settings > Repository > Protected branches*: `main`, *Allowed to merge*: Maintainers, *Allowed to push and merge*: No one, force push off. Until the first push of `main`, *Allowed to push and merge* is Maintainers (see *The first release*).
- *Settings > Repository > Protected tags*: `v*`, *Allowed to create*: Maintainers. The release service account has the Maintainer role and creates every release tag. A tag, once created, is never moved or deleted.
- *Settings > Merge requests*:
  - merge method *Fast-forward merge*, so that the commit on `main` holds the tree the merge request pipeline tested and no merge commit lands beside it;
  - *Squash commits when merging*: *Require*, so that every merge request, the release merge request included, lands as one commit whose message release-plz reads (see [CONTRIBUTING.md](CONTRIBUTING.md#commits));
  - *Pipelines must succeed*.
- *Settings > CI/CD > Variables*: keep *Allow merge request pipelines to access protected variables and runners* off, so that no merge request pipeline reaches the protected Linux runner, and set *Minimum role to use pipeline variables* to *No one allowed*, so that no pipeline variable can override the predefined variables the release jobs and the macOS runner check, such as `CI_COMMIT_REF_PROTECTED`, or the runner and the Git strategy that the workflow rules choose, `LINUX_RUNNER` and `GIT_STRATEGY`. No project or group CI/CD variable carries either name: `LINUX_RUNNER` would send a protected pipeline to the unprotected runner, `GIT_STRATEGY` would let its jobs reuse a working copy.
- *Settings > CI/CD > Auto DevOps*: off, so that a commit without `.gitlab-ci.yml` starts no pipeline.
- The `sub` claim of the project's ID tokens names the project by its id, not its path: `ci_id_token_sub_claim_components` is `["project_id", "ref_type", "ref"]`, set through the API alone (`PUT /projects/:id`, with a JSON body). The path `sakiori-oss/git-intake` once belonged to another project, and GitLab refuses an ID token whose `sub` names such a path, which fails `sign` with `id_token_burned_project_path`. The Sigstore certificate's identity comes from the pipeline's configuration, not from `sub`, so the bundles verify as before.
- *Build > Pipeline schedules*: a weekly schedule on `main`, which runs `deny`, so that a new RustSec advisory fails without a change.
- Issues stay enabled; SECURITY.md asks for vulnerability reports as confidential issues, which every tier has.

GitLab's protected environments need the Premium tier, so the release token is kept from other jobs by a protected variable with an environment scope instead (below). Merge request pipelines run `check-build-linux`, the most expensive job, only when a change can affect the build.

### The release token

release-plz pushes the release branch and the release tag and opens the release merge request as the group's release service account. The job token would not do: its pushes start no pipeline, so the tag would not start the release, and it cannot create merge requests. Project access tokens, which come with a bot user of their own, need the Premium tier on GitLab.com.

- The group's [service account](https://docs.gitlab.com/user/profile/service_accounts/) `sakiori-oss-release`, created by a group owner under the group's *Settings > Service accounts*, with the Maintainer role in the group, so that the same account releases every project of the group, and used for nothing else. Its token therefore reaches every project of the group; the variable that holds it is protected and limited to the release-plz jobs (below). Every project of the group that releases with this account keeps its token under the same protections, since a token that leaks from any of them can release this project. The account takes no seat and cannot sign in.
- The account's personal access token, created by a group owner with the scopes `api` and `write_repository` and an expiry of at most a year. Note the expiry date and replace the token before then.
- *Settings > CI/CD > Variables* of the project, not of the group: `RELEASE_PLZ_TOKEN` with that token, *Protect variable* on, visibility *Masked and hidden*, environment scope `release-plz`. Only the two release-plz jobs name that environment, and a protected variable reaches only pipelines of `main` and the protected tags.

The service account creates the release merge request and the release tag. The release merge request's commit is authored and committed as `Sakiori <oss@sakiori.app>`, which the jobs set with `git config`.

### Dependency updates

No bot proposes dependency updates. Renovate is GitLab's counterpart of Dependabot; running it needs its own bot account and token, as a hosted app or a scheduled pipeline, so it is an owner's decision. Until then, update `Cargo.lock` by hand (`cargo update`), committed as `fix(deps): ...`, since it changes the shipped binary; the weekly `deny` job reports advisories in the meantime.

## The Linux runners

Every job but the macOS builds runs on one of the group's two Linux runners, Docker executors whose jobs run in unprivileged containers and keep no lasting credentials there: a job's tokens exist only while it runs. Both take tagged jobs only, and neither carries the other's tag.

- `sakiori-oss-linux-protected`, *Protected*, so that it takes jobs of protected branches and tags only, runs every pipeline whose `CI_COMMIT_REF_PROTECTED` is `true`, the predicate by which GitLab gives a pipeline protected runners: the release tags' pipelines and every pipeline of `main`, the release-plz jobs and the weekly schedule among them. It runs build code, the dependencies' build scripts included, beside the jobs that hold the release's credentials, so each of its jobs clones a fresh working copy, and neither the release tag's jobs nor the release-plz jobs restore a cache.
- `sakiori-oss-linux` runs every other pipeline, the merge request pipelines of the group's projects among them, upstream's code included. A merge request from a fork outside the group finds no runner; a member of this project with at least the Developer role, having read the change, runs its pipeline here, on this runner.

The workflow rules of `.gitlab-ci.yml` choose the runner of the whole pipeline, so a protected pipeline never falls back to the other one: while the protected runner is offline, the jobs of protected pipelines wait for it.

## The macOS runner

The macOS targets are built by a runner of the project's own on an Apple silicon Mac. `release-build-macos` runs the code of the release tag's commit and of every dependency's build script, so each job runs in a fresh macOS virtual machine that [Tart](https://tart.run/) clones from the image `MACOS_IMAGE` names and deletes after the job: nothing a build runs persists on the host or reaches a later build, and the virtual machine cannot reach the host's local network. The license of macOS 26 permits up to two additional instances of macOS in virtual machines on an Apple computer that already runs macOS, for software development among other purposes. The image is Cirrus Labs' base image, which has the Xcode Command Line Tools, gitlab-runner, which uploads the job's artifacts, and Rosetta; the job installs its own Rust toolchain with the pinned rustup-init and needs nothing else.

- *Settings > CI/CD > Runners > New project runner* in `sakiori-oss/git-intake`, or the API's `POST /user/runners` with `runner_type=project_type`: tag `git-intake-macos`, *Run untagged jobs* off, *Protected* on, so that it takes jobs only from protected branches and tags, *Lock to current projects* on, and a maximum job timeout of one hour. Only `release-build-macos` names the tag, and only for a protected tag `vX.Y.Z`; merge request pipelines never reach the runner.
- The Mac runs macOS 26 or later, which Softnet requires, and a macOS release at least as new as the image's, since a host cannot run a newer macOS in a virtual machine; a new `MACOS_IMAGE` may need the host updated first.
- Softnet passes the virtual machine's traffic to globally routable addresses and to the host's vmnet gateway, which is the host itself. The `tart` the wrapper puts first on the executor's `PATH` blocks every connection to the gateway (`--net-softnet-block='out @host'`), and the wrapper allows DNS to it alone (`out proto udp @host port 53`), so that no service of the host is reachable from a build. A host whose resolver answers with addresses outside the globally routable space, as a proxy's fake-IP mode does, lists those CIDRs, comma-separated, in `/usr/local/etc/git-intake-tart-executor/softnet-allow`, owned by root, and the wrapper allows them too. That file holds only such a range, never one with the local network's addresses in it; an address of the host's own within the range, such as the proxy's interface address, goes into `/usr/local/etc/git-intake-tart-executor/softnet-block` beside it, and the `tart` below blocks it, since Softnet prefers the longest prefix. Once set up, check it in a virtual machine of the image started as the executor starts it, `tart run --net-softnet --net-softnet-allow='out proto udp @host port 53,<CIDRs of the allow file>' --net-softnet-block='out @host,<CIDRs of the block file>'`, without the lists the host has no file for: `nslookup gitlab.com` and `curl -I https://gitlab.com` succeed, and `nc -z -G2 <gateway> 22` and every other port of the host's services fail, at the gateway and at every address the block file lists.
- An administrator installs Tart, [Softnet](https://github.com/openai/softnet), with the SUID bit its README asks for, and Cirrus Labs' [gitlab-tart-executor](https://github.com/cirruslabs/gitlab-tart-executor), and allows connections to the virtual machines' private addresses without the *Local Network* prompt, as the executor's README describes. Tart is source-available under the Functional Source License (FSL-1.1-ALv2), which permits any use but a competing product or service and grants the Apache License 2.0 two years after each release.
- GitLab Runner runs as the launch agent of a user signed in on the Mac, as GitLab Runner does on macOS, in a session that stays signed in; after a restart, sign the user in again. The runner is registered there with `gitlab-runner register --url https://gitlab.com --token <runner authentication token> --executor custom` and joins that user's `~/.gitlab-runner/config.toml` with the section below, with `limit = 1` and `concurrent` large enough for every runner there. The jobs run only in the virtual machines; on the host, the runner runs only the wrapper below. Every other runner of that user, and every job it takes, can rewrite that file, read the runner's token and change the cached image, so the user runs no other runner, or only runners whose jobs are trusted as much as the release builds; a dedicated standard user account that runs this runner alone is the safe choice.
- As the runner's user, `tart pull` the image `MACOS_IMAGE` names before the first release and after every change of it. The executor does not pull it for each job; a job that finds no cached copy pulls it in its prepare stage, tens of gigabytes that can exceed the job's timeout.
- The executor is the wrapper below, `/usr/local/libexec/git-intake-tart-executor`, with the `tart` below as `/usr/local/libexec/git-intake-tart/tart`, both owned by root and not writable by the runner's user, for all four stages. Before any virtual machine starts, it refuses every job but one of `sakiori-oss/git-intake` for a protected tag `vX.Y.Z`, and a job that sets a variable of gitlab-tart-executor's, which reads its settings from the job's variables. It turns Softnet on with the allowances above, leaves the cached image unpulled, and gives the virtual machine half of the host's CPUs and memory.
- The runner's section of `~/.gitlab-runner/config.toml` is the one below, with no `--builds-dir` or `--cache-dir`, so that no host directory is mounted into the virtual machine, and `TART_EXECUTOR_HOST_DIR` off, as the wrapper keeps it.

```sh
#!/bin/sh
# The executor of the macOS runner of sakiori-oss/git-intake: the
# gitlab-tart-executor stage the runner calls, with the runner's arguments,
# for a release build of the project only and with fixed settings.
# gitlab-tart-executor reads its settings from the job's variables
# (CUSTOM_ENV_TART_EXECUTOR_*) in every stage, so a job that sets one is
# refused, and the settings below are set here. A refusal fails the job
# before a virtual machine is started.
#
# usage: git-intake-tart-executor config|prepare|cleanup
#        git-intake-tart-executor run <script> <sub-stage>
set -eu

# The tart beside this wrapper first, then Homebrew's.
PATH=/usr/local/libexec/git-intake-tart:/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin
export PATH

refuse() {
  echo "this runner builds only the protected release tags of sakiori-oss/git-intake: $1" >&2
  exit "${BUILD_FAILURE_EXIT_CODE:-1}"
}

[ "${CUSTOM_ENV_CI_PROJECT_PATH:-}" = sakiori-oss/git-intake ] \
  || refuse "the project is '${CUSTOM_ENV_CI_PROJECT_PATH:-}'"
[ "${CUSTOM_ENV_CI_COMMIT_REF_PROTECTED:-}" = true ] \
  || refuse "the ref is not protected"
tag=${CUSTOM_ENV_CI_COMMIT_TAG:-}
# Only v, digits and dots, so that grep's line matching sees the whole tag.
case "$tag" in
  *[!v0-9.]* | "") refuse "'$tag' is not a release tag vX.Y.Z" ;;
esac
printf '%s\n' "$tag" | grep -Eqx 'v[0-9]+\.[0-9]+\.[0-9]+' \
  || refuse "'$tag' is not a release tag vX.Y.Z"
if env | grep -q '^CUSTOM_ENV_TART_EXECUTOR_'; then
  refuse "the job sets a TART_EXECUTOR_ variable"
fi

# Softnet: the virtual machine reaches globally routable addresses and the
# host's vmnet gateway, the host itself, not the host's local network. The
# tart beside this wrapper blocks the gateway, and only DNS to it is allowed
# here, with the CIDRs a host lists in the file below, for a resolver that
# answers with addresses outside the globally routable space.
CUSTOM_ENV_TART_EXECUTOR_SOFTNET=true
allow='out proto udp @host port 53'
extra=/usr/local/etc/git-intake-tart-executor/softnet-allow
if [ -f "$extra" ]; then
  cidrs=$(tr -d ' \t\n' < "$extra")
  case "$cidrs" in
    *[!0-9./,]*)
      echo "$extra holds something other than comma-separated CIDRs" >&2
      exit "${SYSTEM_FAILURE_EXIT_CODE:-1}"
      ;;
  esac
  [ -z "$cidrs" ] || allow="$allow,$cidrs"
fi
CUSTOM_ENV_TART_EXECUTOR_SOFTNET_ALLOW=$allow
# The image is pinned by digest and pulled ahead, so it is not pulled for
# every job; without a cached copy, tart clone pulls it within the job.
CUSTOM_ENV_TART_EXECUTOR_ALWAYS_PULL=false
export CUSTOM_ENV_TART_EXECUTOR_SOFTNET CUSTOM_ENV_TART_EXECUTOR_SOFTNET_ALLOW
export CUSTOM_ENV_TART_EXECUTOR_ALWAYS_PULL

case "${1:-}" in
  config | run | cleanup)
    exec gitlab-tart-executor "$@"
    ;;
  prepare)
    shift
    # Half of the host's CPUs and memory, which leaves the host usable; the
    # runner runs one job at a time. Only a Cirrus Labs base image pinned by
    # digest is cloned.
    exec gitlab-tart-executor prepare --concurrency 2 --cpu auto --memory auto \
      --allow-image 'ghcr.io/cirruslabs/macos-*-base@sha256:*' "$@"
    ;;
  *)
    echo "usage: git-intake-tart-executor config|prepare|run|cleanup [<argument>...]" >&2
    exit "${SYSTEM_FAILURE_EXIT_CODE:-1}"
    ;;
esac
```

```sh
#!/bin/sh
# tart for the macOS runner of sakiori-oss/git-intake, first on the PATH that
# git-intake-tart-executor gives gitlab-tart-executor: a virtual machine it
# runs cannot open a connection to the host's vmnet gateway, the host itself,
# beyond what the executor's Softnet allowance permits, DNS.
set -eu
PATH=/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin
export PATH
# A host that allows CIDRs holding addresses of its own, such as a proxy's
# fake-IP range with the proxy's interface address in it, lists those
# addresses, comma-separated, in the file below, and they are blocked too.
if [ "${1:-}" = run ]; then
  shift
  block='out @host'
  extra=/usr/local/etc/git-intake-tart-executor/softnet-block
  if [ -f "$extra" ]; then
    cidrs=$(tr -d ' \t\n' < "$extra")
    case "$cidrs" in
      *[!0-9./,]*)
        echo "$extra holds something other than comma-separated CIDRs" >&2
        exit 1
        ;;
    esac
    [ -z "$cidrs" ] || block="$block,$cidrs"
  fi
  exec tart run --net-softnet-block="$block" "$@"
fi
exec tart "$@"
```

```toml
[[runners]]
  name = "git-intake-macos"
  url = "https://gitlab.com"
  token = "<runner authentication token>"
  executor = "custom"
  limit = 1
  [runners.feature_flags]
    FF_RESOLVE_FULL_TLS_CHAIN = false
  [runners.custom]
    config_exec = "/usr/local/libexec/git-intake-tart-executor"
    config_args = ["config"]
    prepare_exec = "/usr/local/libexec/git-intake-tart-executor"
    prepare_args = ["prepare"]
    run_exec = "/usr/local/libexec/git-intake-tart-executor"
    run_args = ["run"]
    cleanup_exec = "/usr/local/libexec/git-intake-tart-executor"
    cleanup_args = ["cleanup"]
```

While the runner is offline, `release-build-macos` waits, and `sign` and `publish` with it; GitLab fails a job that no runner picks up, after an hour when no runner for it is online and after a day otherwise; then bring the runner online and retry the job, as *The tag pipeline failed* describes, and `sign` and `publish` follow. The runner's authentication token has the rights of the runner only; if it leaks, delete the runner in the project and register a new one.

## The first release

The project starts empty and receives the repository's history as it is published. The push of `main` releases v0.4.0 at once, so everything the release needs is ready before it.

1. Make the settings above, with one exception: protect `main` with *Allowed to push and merge*: Maintainers for now, so that the history can be pushed. Create the release service account and its token. Bring the group's Linux runners and the macOS runner online.
2. Push the earlier release tags, each by name and never with `--tags`, with CI skipped: `git push -o ci.skip origin v0.1.0 v0.2.0 v0.2.1 v0.2.2 v0.3.0 v0.3.1 v0.3.2 v0.3.3 v0.3.4 v0.3.5 v0.3.6 v0.3.7`. Their commits hold no `.gitlab-ci.yml`, so with Auto DevOps off they would start no pipeline anyway; they are tags only.
3. Push `main`: `git push origin main`. Its push pipeline runs `release-plz-release`, which tags `v0.4.0`, the version `Cargo.toml` holds, and the tag's pipeline publishes the release. `release-plz-pr` then finds no commit since the tag that asks for a release.
4. Set *Allowed to push and merge* of `main` to No one, as the settings above ask; from now on `main` changes only through merge requests.
5. Check the release as under *Checking a release*, and install it once through each way the README describes.

## Recovery

### The tag pipeline failed

Retry the failed job. A macOS build that failed because the runner was offline is retried once the runner is back. A retried `publish` job uploads only the files the package lacks, skips those it holds with the same SHA-256 and refuses one it holds with other content, so retrying it after an upload failed is safe; retried after it created the release, it succeeds when the release links exactly the release's files, and fails otherwise. A new pipeline for the tag (*Build > Pipelines > New pipeline* on the tag) builds and signs again, while no release of the tag exists; its signatures, and possibly its archives, differ from those of an earlier run, so if that run uploaded files, delete the package version `git-intake` `vX.Y.Z` under *Deploy > Package registry* first. Once the release exists, the guard refuses every new pipeline of the tag.

### The tag is missing

`release-plz-release` failed or did not run. Run the pipeline of `main` again, or create the tag by hand on the commit of `main` that set the version:

```sh
git tag -a vX.Y.Z -m vX.Y.Z <commit>
git push origin refs/tags/vX.Y.Z
```

### A published release is wrong

A published release and its files are never replaced. Fix forward: revert or fix on `main` and release the next version.

## Checking a release

Anyone can check that an archive was signed by this project's pipeline for its tag, as the README's *Verifying the signature* shows:

```sh
cosign verify-blob --bundle git-intake-vX.Y.Z-<target>.tar.gz.sigstore.json \
  --certificate-identity "https://gitlab.com/sakiori-oss/git-intake//.gitlab-ci.yml@refs/tags/vX.Y.Z" \
  --certificate-oidc-issuer https://gitlab.com \
  git-intake-vX.Y.Z-<target>.tar.gz
```

The certificate also records the commit the pipeline ran on, in Sigstore's *Source Repository Digest* extension (OID 1.3.6.1.4.1.57264.1.13). To tie the archive to the commit the tag names, compare the two, in a clone of `https://gitlab.com/sakiori-oss/git-intake`, never of a fork or mirror whose tags may differ:

```sh
git fetch origin tag vX.Y.Z
git rev-parse 'vX.Y.Z^{commit}'
jq -r '.verificationMaterial.certificate.rawBytes' git-intake-vX.Y.Z-<target>.tar.gz.sigstore.json \
  | base64 -d | openssl x509 -inform DER -noout -text | grep -A1 '1.3.6.1.4.1.57264.1.13:'
```

## Pinned versions

Nothing updates the pins automatically. The images are pinned by digest in `.gitlab-ci.yml`: `RUST_IMAGE`, `NODE_IMAGE`, and `MACOS_IMAGE`, the macOS virtual machine image of the macOS builds, which the runner pulls ahead (*The macOS runner*). The OpenSpec CLI is pinned in `.gitlab/openspec/package.json`, and its whole dependency tree, with every package's integrity hash, in `.gitlab/openspec/package-lock.json`, which `npm ci --ignore-scripts` installs; update both with `npm install --ignore-scripts --prefix .gitlab/openspec @fission-ai/openspec@<version> --save-exact` in `NODE_IMAGE`. The oldest git the tests run with, with its source archive's SHA-256, in the `test-git` job. `RUST_VERSION` names the Rust release of `RUST_IMAGE`, which the macOS builds install, and the Linux builds fail when the two differ, so a new Rust release changes both. Every other tool is pinned by version and SHA-256 in `.gitlab/scripts/install-tools.sh`: for x86_64 Linux zig, cargo-zigbuild, cargo-about, cargo-deny, release-plz, cosign, jq and shellcheck, and for Apple silicon macOS rustup-init and cargo-about. The Xcode Command Line Tools are those of `MACOS_IMAGE`, so a new image may bring new ones; the macOS binaries record the SDK version they were built with. cargo-about is pinned also in `xtask/src/main.rs`, which refuses to generate the notices with another release.

## Publishing on crates.io

The crate is not on crates.io yet. Its publication there takes these steps, in this order:

1. Publish the first version by hand with an API token: from a clean checkout of the commit of a release tag, `cargo publish --locked`, never `--allow-dirty`, so that the published crate records the commit it came from.
2. In the crate's settings on crates.io, under *Trusted Publishing*, add a GitLab publisher: namespace `sakiori-oss`, project `git-intake`, workflow filepath `.gitlab-ci.yml`. Trusted Publishing applies only to a crate that exists, hence the first publication by token.
3. Turn `git_only` off in `release-plz.toml` and add a publish job to the release: it obtains a short-lived token by exchanging a GitLab ID token with audience `crates.io` at crates.io's Trusted Publishing endpoint, and runs `release-plz release` with it as `CARGO_REGISTRY_TOKEN`; release-plz obtains Trusted Publishing tokens by itself only in GitHub Actions. release-plz then publishes the crate before it creates the tag, so the binaries are built from the commit the crate came from.
4. Enable *Trusted Publishing only* for the crate, so that no API token can publish it, and revoke the token of the first publication.
