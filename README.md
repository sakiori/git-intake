# git-intake

`git intake` keeps a downstream repository current with one or more upstreams, and releases it. It takes upstream changes by merge commits or by copying chosen upstream commits, in a separate worktree, and never rewrites history or force-pushes. A conflict is resolved once. The repository's own checks run on exactly the tree that gets published: `verify` runs them before a push, and a merge request's pipeline runs them on a forge whose settings merge only a tree that pipeline tested. The integration branch moves only by fast-forward. `git intake sync` runs the whole cycle unattended, as a scheduled CI job; `git intake release` cuts and versions releases that name the upstream release they carry.

It covers three cases:

- **A fork**: one upstream at the repository root, merged as a whole. A branch stacked on another fork's integration branch is a fork with that branch as its upstream.
- **A monorepo assembled from several repositories**: each source is an upstream merged under its own path until cutover, which `status --check` gates.
- **A product built on an open-source project**: the product takes only the upstream commits it wants, skips others, and records which ones it already has.

The tool runs the installed `git` for every repository operation. Each repository describes itself in a committed `.git-intake.toml`. Upstreams are fetched by URL; the repository keeps no remote or branch of an upstream. The integration branch is the one on the downstream's publish remote, which every command fetches: the tool never reads, creates or moves a local branch of that name.

**Contents:** [How it differs from other tools](#how-it-differs-from-other-tools) · [Installation](#installation) · [Quickstart](#quickstart) · [Releases](#releases) · [Configuration](#configuration) · [The update cycle](#the-update-cycle) · [The sync record](#the-sync-record) · [Downstream releases](#downstream-releases) · [Commands](#commands) · [Exit status](#exit-status) · [Running in CI](#running-in-ci) · [Monorepo migration](#monorepo-migration) · [Safety properties](#safety-properties) · [License](#license)

## How it differs from other tools

- **GitHub's *Sync fork* and `gh repo sync`** fast-forward or hard-reset one branch from one source and refuse on conflicts. git intake takes any number of upstreams by URL, at the root or under paths, merges each at a fixed target with a recorded merge commit, has conflicts resolved in a worktree, and publishes only a tree its checks or a required pipeline passed.
- **The Pull app** is a hosted GitHub app that merges pull requests from the same fork network, possibly by squash, rebase or hard reset. git intake is a command-line tool, takes upstreams from any host, never squashes, rebases or resets the integration branch, and can take chosen commits one by one (port mode).
- **Copybara** transforms files and writes new commits in the destination. git intake transforms nothing: the upstream's commits arrive in the merge, and a ported copy keeps its author and message.
- **git subtree** is a local subdirectory merge. git intake adds fixed targets, recorded resolutions, checks, publication and drift reports to the same kind of merge, and adopts a path imported by `git subtree add` without `--squash`.
- **Renovate and Dependabot** bump version references; they bring in no upstream source history.

## Installation

Requires Git 2.43 or later and a POSIX `sh`. The tool is a single binary, `git-intake`; on the `PATH`, git runs it as `git intake`. It runs on Linux and macOS, not on Windows.

While the version is `0.x`, the formats of `.git-intake.toml` and of the update state may change between releases; a release that changes them says how to move to it, as *Upgrading to the publish remote's integration tip* does for this one.

Releases provide prebuilt binaries for `x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl`, `aarch64-apple-darwin` and `x86_64-apple-darwin`. The Linux binaries are static and run on any distribution and in minimal CI images. The macOS binaries need macOS 11 or later and check the forge's TLS certificate against the keychain. Release archives start with 0.4.0.

### With mise

Pin the version in the repository's tool configuration, so that an update of the tool is a reviewed change:

```sh
mise use gitlab:sakiori-oss/git-intake@X.Y.Z
```

mise installs the binary and the manual pages; no token is needed.

### With cargo

```sh
cargo binstall --git https://gitlab.com/sakiori-oss/git-intake git-intake
cargo install --locked --git https://gitlab.com/sakiori-oss/git-intake --tag vX.Y.Z git-intake
```

[cargo-binstall](https://github.com/cargo-bins/cargo-binstall) downloads the archive of the latest release, or builds from source while a just-merged release is not yet published. `cargo install` builds the tagged release and needs Rust 1.88 or later. Neither installs the manual pages (see *Help and the manual page*). The crate is not on crates.io yet.

### Direct download

Each [release](https://gitlab.com/sakiori-oss/git-intake/-/releases) lists its archives, `git-intake-vX.Y.Z-<target>.tar.gz`, each with a `.sha256` checksum and a `.sigstore.json` signature bundle, and `SHA256SUMS` over all archives with its own bundle. An archive holds:

```
git-intake
LICENSE
THIRD-PARTY-NOTICES.txt
RUST-LIBRARY-COPYRIGHT.html
man/man1/git-intake.1
man/man1/git-intake-<command>.1
```

These commands install the tool anywhere, a CI job included, without a token:

```sh
tag=vX.Y.Z  # the release to install
target=x86_64-unknown-linux-musl
file="git-intake-$tag-$target.tar.gz"
base="https://gitlab.com/sakiori-oss/git-intake/-/releases/$tag/downloads/binaries"
curl --fail --location --remote-name "$base/$file"
curl --fail --location --remote-name "$base/$file.sha256"
sha256sum -c "$file.sha256"   # on macOS: shasum -a 256 -c "$file.sha256"
tar -xzf "$file" -C /usr/local/bin git-intake
```

To install the manual pages as well, extract them into a manual directory, for example `tar -xzf "$file" -C /usr/local/share man`.

### Verifying the signature

The release pipeline signs every archive and `SHA256SUMS` with [Sigstore](https://www.sigstore.dev/)'s keyless signing, recorded in its public transparency log. With [cosign](https://docs.sigstore.dev/cosign/system_config/installation/), check that an archive was signed by the pipeline of its tag:

```sh
curl --fail --location --remote-name "$base/$file.sigstore.json"
cosign verify-blob --bundle "$file.sigstore.json" \
  --certificate-identity "https://gitlab.com/sakiori-oss/git-intake//.gitlab-ci.yml@refs/tags/$tag" \
  --certificate-oidc-issuer https://gitlab.com \
  "$file"
```

The double slash separates the project from the path of its CI configuration. `SHA256SUMS` is checked the same way, and `sha256sum -c SHA256SUMS --ignore-missing` then checks any archive.

### From source

`cargo install --locked --path .` installs a checkout; `cargo run --locked -p xtask -- man <dir>` writes the manual pages into `<dir>`.

### Help and the manual page

`git intake -h`, and `-h` or `--help` after a command, print the built-in help, which always describes the binary that runs. `git intake --help` becomes `git help intake`, which opens the manual page `git-intake(1)` and works only where `man` finds it.

The pages ship in every release archive under `man/man1/`, and mise installs them; with cargo, take them from the archive or write them with `cargo run --locked -p xtask -- man <dir>` from the release's tag. Unless `MANPATH` is set, `man` also searches the `man` subdirectory of every directory on `PATH`, so pages beside the binary are found, as with `mise activate`, but not through mise's shims, nor when `MANPATH` has no empty component (a leading or trailing `:`, or `::`).

Pages of an earlier release found first describe that release; `man -w git-intake-verify` names the file, and every page's footer names its release. Remove them, or put the current pages first:

```sh
export MANPATH="$(mise where gitlab:sakiori-oss/git-intake)/man:$MANPATH"
```

`mise where` answers for the directory it runs in. Pages extracted into `/usr/local/share` are on the default search path; replace them with every new release.

## Quickstart

The repository that receives the changes is the **downstream**; each repository it takes changes from is an **upstream**, fetched by its **URL**. The downstream's **integration branch**, usually `main`, is where updates land. It lives on the **publish remote**, a git remote of the downstream such as `origin`, which is never an upstream; a downstream without a hosted repository uses a local bare repository, as below.

The steps below make a fork in throwaway repositories and run as they are in an empty directory. The `git -C` and `git config` lines are setup.

1. An upstream with one commit, a bare repository standing in for the downstream's hosted one, and the downstream cloned from the upstream:

   ```sh
   git init -q -b main upstream
   git -C upstream config user.name "Sakiori"
   git -C upstream config user.email "oss@sakiori.app"
   echo "hello" > upstream/greeting.txt
   git -C upstream add greeting.txt
   git -C upstream commit -q -m "feat: greet"

   git init -q --bare -b main downstream.git
   git clone -q upstream downstream
   cd downstream
   git remote remove origin                 # the clone's origin is the upstream: no remote for it
   git remote add origin ../downstream.git  # the publish remote
   git config user.name "Sakiori"           # git intake commits merges with git's identity
   git config user.email "oss@sakiori.app"
   ```

2. The configuration, committed on the integration branch, with a check that the fork's greeting still says hello:

   ```sh
   cat > .git-intake.toml <<'EOF'
   integration = "main"
   checks = [
     { name = "greeting", run = "grep -q hello greeting.txt" },
   ]

   [publish]
   remote = "origin"

   [[upstream]]
   name = "upstream"
   url = "../upstream"   # relative to the downstream's main worktree
   branch = "main"
   EOF
   git add .git-intake.toml
   git commit -q -m "chore: configure git intake"
   git push -q -u origin main
   ```

3. With nothing new upstream, an update has nothing to do:

   ```sh
   git intake start
   ```

   ```
   upstream: current at 1a2b3c4d5e6f
   every selected upstream is current; nothing to update
   ```

   It exits with status 0 and creates nothing.

4. The upstream moves on, and an update takes it in:

   ```sh
   echo "hello, world" > ../upstream/greeting.txt
   git -C ../upstream commit -q -am "feat: greet the world"

   git intake status             # upstream: behind by 1
   git intake start              # merges it in .worktrees/git-intake/update
   git intake verify             # runs the check there
   git intake report             # the merged range, conflicts and changes
   git intake finish             # pushes the verified merge to origin's main
   ```

   `start` merges the upstream into `origin`'s `main` in the update worktree with a sync merge, `verify` passes the check, and `finish` pushes the merge to `origin` and removes the update. The local `main` does not move; `git pull` updates it if wanted. On a conflict, `start` stops with status 1: resolve the files in the update worktree, `git add` them and run `git intake continue`.

5. Every upstream is current again:

   ```sh
   git intake status --check     # exit status 0: no drift
   git intake start              # every selected upstream is current; nothing to update
   ```

[The update cycle](#the-update-cycle) describes each step; [Running in CI](#running-in-ci) runs the cycle on a schedule.

## Releases

Each release is a [GitLab release](https://gitlab.com/sakiori-oss/git-intake/-/releases) with the archives described under *Installation*. [CHANGELOG.md](CHANGELOG.md) lists the changes of every release. Versions come from the [Conventional Commits](https://www.conventionalcommits.org/) since the last release, through [release-plz](https://release-plz.dev/): in `0.x` a breaking change raises the minor version, and a feature, fix, performance change or revert the patch version. [RELEASING.md](RELEASING.md) is the maintainers' runbook.

## Configuration

`.git-intake.toml` lives at the repository root and is committed on the integration branch. The tool reads it at the integration tip (see *The integration tip*), except where the release commands read it at the checked-out commit. An uncommitted edit is ignored, with a note.

```toml
integration = "main"                  # the branch that receives the upstream changes
install = "pnpm install"              # optional: runs before the checks
checks = [                            # run in order by verify, each named
  { name = "lint", run = "pnpm lint" },
  { name = "test", run = "pnpm test" },
]

[publish]                             # required: where the integration branch is read and published
remote = "origin"                     # the publish remote, a git remote of the repository
via = "push"                          # optional: "push" (default) or "merge-request"
# forge = "gitlab"                    # optional: "gitlab" or "github", when the URL does not tell, as on a forge's own domain; in push mode it names the CI files
# api = "https://gitlab.example.com/gitlab/api/v4"  # optional, merge-request mode or [release]: the forge's REST API on the remote's host, when it is not at the usual address (see The forge)
# queue = "merge-bot"                 # optional, merge-request mode on GitLab: the account of a merge queue that land pauses
# replace_after = "7d"                # optional, merge-request mode: replace an untouched update merge request this old when a newer release comes out (see Unattended synchronisation)

[[upstream]]
name = "app-a"                        # unique; used on the command line, in sync records, trailers and check variables
url = "git@example.com:team/app-a.git"  # required: where the upstream is fetched from
branch = "main"                       # the upstream branch to take from
mode = "follow"                       # optional: "follow" (default) or "port"
path = "apps/app-a"                   # optional, follow mode only: merge under this path; absent = repository root
tags = "v*"                           # optional, follow mode only: take the newest release tag matching this pattern instead of the branch tip
# prereleases = true                  # optional, with tags: prerelease tags such as v1.2.3-nightly.20260102.8 count as targets
# max_conflicts = 150                 # optional, with tags: take the newest tag whose merge conflicts in at most this many files

[release]                             # optional: make the integration branch releasable (see Downstream releases)
upstream = "app-a"                    # the follow upstream whose version the releases carry
tags = "v*"                           # optional: the glob of its release tags; default: the upstream's own `tags`
labels = ["custom"]                   # optional: the label of new dated releases first, then labels earlier releases took
# versioning = "semver"               # optional, default "dated": how releases are versioned (see Downstream releases)
```

The tool refuses a configuration that breaks any of these rules, naming the entries involved:

- Unknown keys are rejected; `remote` and `mirror` of earlier releases are refused with what replaces them.
- `integration` is a valid branch name other than `git-intake/update`, which is reserved for updates.
- At least one `[[upstream]]` is required, each with a `url` without a password, nor a user for `http` and `https`: credentials belong to git's credential helper or SSH key.
- Upstream names are unique, and no two upstreams share URL and branch. A name has only letters, digits, `-`, `_` and `.`, does not start with `-` or `.`, contains no `..`, and does not end with `.` or `.lock`. No two names may differ only in letter case or in which of `-`, `_` and `.` they use, since they name the checks' variables (see [The environment of the checks](#the-environment-of-the-checks)).
- A path is normalised: no leading or trailing slash, no empty, `.`, `..` or `.git` segment (in any case), and not empty; omit `path` for the root. Paths may not nest. An upstream at the root must be the only upstream. A `port` upstream has no path.
- `tags` belongs to a `follow` upstream: a glob over tag names (`*`, `?`, `[...]`, matched against the whole name), without `refs/tags/`, such as `v*` or `release-[0-9]*`. `prereleases` and `max_conflicts` need `tags`; `max_conflicts` is a positive number of files.
- Each check is a table with exactly `name` and a non-empty `run`. A name has only ASCII letters, digits, `-`, `_` and `.`, does not start with `-` or `.`, and is unique ignoring case; it selects the check for `verify --only` and `--skip` and names it in the output, the merge request's description and its log. The plain string form of earlier releases, `checks = ["pnpm lint"]`, is refused with the table form to write.
- `[publish]` is required, with `remote`; without it the configuration is refused with the table to add. A downstream without a hosted repository adds a local bare repository as its publish remote, by a path or a `file://` URL. `via` is `push` or `merge-request`; `forge` is `gitlab` or `github`; `api` is an `https` URL with a host and without credentials, query or fragment (see *The forge*); `queue` is an account name of letters, digits, `-`, `_` and `.`, with or without its `@`, in merge-request mode only (see *The merge queue*). `replace_after` is a positive duration of weeks, days, hours and minutes, such as `7d`, `36h`, `2w` or `1d 12h`, a day being 24 hours; it needs merge-request mode and a `follow` upstream with `tags`. Write `[publish]` after the top-level keys, or inline as `publish = { remote = "origin", via = "merge-request" }`; the string form `publish = "origin"` is refused with the form to write.
- `release` is a table with exactly the keys `upstream`, `tags`, `labels` and `versioning`. `upstream` names an upstream in `follow` mode; `tags` is a pattern as for an upstream's `tags`, and without it the upstream must have one; `labels` is a non-empty list of distinct labels of ASCII letters, digits and `-`, none of digits alone; `versioning` is `dated` or `semver` (see *Downstream releases*).
- The publish remote's `url` and any `pushurl` may not name an upstream's repository: the tool never pushes to an upstream. URLs are compared as configured and after git's `insteadOf` and `pushInsteadOf` rewriting. Two URLs name the same repository when:
  - both are local paths or `file://` URLs (any host) that resolve against the main worktree to the same canonical repository, with or without `.git` appended: a relative or absolute path, a symbolic link to the repository and a linked worktree of it all match;
  - or both are hosted URLs with the same host and path, whatever the scheme, user, password and path case. `user@host:path` reads as `ssh://user@host/path` and `<transport>::<address>` as its address; hosts compare in lower case without a trailing `.`, default ports are dropped, and paths compare without doubled or trailing `/`, a `.git` suffix and a leading `~/`. An `ssh` URL also matches by the host and port `ssh -G` resolves, so a host alias names its host.

  `doctor` compares every remote with the upstreams the same way, and every push and branch deletion is checked again where git sends it.

### The integration tip

The **integration tip** is the commit the publish remote's integration branch is at. Every command that reads the integration branch first fetches it, with `git fetch --no-tags <remote> +refs/heads/<integration>:<reference>`, into a reference of its own under `refs/git-intake/read/` that it removes before it exits. It reads the configuration, the port journals, the sync records, the drift and the base of a new update there. `finish`, `sync` and `land` fetch again before they act. Only `continue`, `verify`, `report`, `abort` and the release commands that read the checked-out commit work without the tip; `abort` reads it only to name an open merge request.

No remote-tracking branch is written, and no local branch of the integration branch's name is read, created or moved: a clone may keep a local `main`, let it lag, or delete it.

Where to fetch from comes from the update in progress, or else from `integration`, the `remote` of `[publish]` and the upstreams' URLs in the `.git-intake.toml` at `HEAD` (`env` and `delta` look at the commit they describe first). So run the tool where `HEAD` carries the configuration, a CI clone's detached `HEAD` included. A publish remote that names an upstream's repository is refused before anything is fetched. When the configuration at the fetched tip names another integration branch or publish remote, the command refuses and names both; check out the new configuration, or finish or abort the update in progress, and run it again.

When the publish remote cannot be fetched, or lacks the branch, the command refuses with git's reason and changes nothing; `doctor` reports a failed check and skips the checks that need the tip.

### Upstreams by URL

An upstream is fetched with `git fetch --no-tags <url> +refs/heads/<branch>:<reference>`, in the main worktree so that a relative URL means what it means there, into a reference under `refs/git-intake/`. Outside `refs/heads` and `refs/remotes`, no branch listing, checkout, `git push --all` or hosting tool sees it.

- Every command that fetches (`status`, `log`, `branches`, `doctor`, `contribute`, `start`, `sync`, and every reader of the integration tip) uses a namespace of its own, `refs/git-intake/read/<token>/`, removed before it exits, whatever the outcome. `branches` fetches every branch of an upstream there.
- An update holds `refs/git-intake/upstreams/<name>` for each queued upstream at the tip `start` fetched, or for an update `sync` carries on from a merge request at its target, so its targets and planned commits stay in the repository. Only `start` and `sync` write them, only `finish`, `abort` and `sync` remove them, and `continue`, `verify` and `finish` fetch no upstream.
- If `start` is killed after saving the update and before writing them, `status`, `continue`, `verify` and `finish` restore them from the state and say so.
- `doctor` reports an update reference that is missing or elsewhere, warns to keep a reference that still holds the commit of a missing one, and reports any other reference under `refs/git-intake/`, such as a killed command's namespace, which `git update-ref -d <reference>` removes.

### Modes

Each upstream has a mode:

- `follow`, the default: an update merges the upstream branch up to a target, at the root or under the upstream's path. With `tags`, the target is the newest release tag (see *Release tags*).
- `port`: an update takes chosen upstream commits one by one, at the root; every upstream commit is taken, skipped, or recorded as already present.

Switching from `follow` to `port` changes one setting; the first port update starts the journal at the last synced commit (see *Port mode*). Switching back works too: the next merge finds its base from the last merge, and changes taken in port mode come back as the same changes. A journal of a `follow` upstream means nothing, and `doctor` reports it as stale.

## The update cycle

```sh
git intake start        # fetch, fix targets and merge each follow upstream, or plan a port
# port mode: edit the plan, then
git intake continue     # carry out the plan
# on conflict: resolve in the update worktree, `git add`, then
git intake continue     # conclude the merge or copy, take in the rest
git intake verify       # run install and checks on the result
git intake finish       # catch up if the integration branch moved, push the verified update, clean up
# with via = "merge-request": verify is optional; finish pushes the update, opens its merge
# request and ends the local update; the merge request carries it from then on
git intake delta        # in the update or on its merge request: what it changed in the downstream's delta
git intake land         # wait for the merge request's pipeline, catch it up if needed, merge it

git intake sync         # all of it, unattended, as a scheduled CI job runs it
```

Every command works the same from the main checkout, a linked worktree or the update worktree, in a submodule and with a git directory outside the checkout. Repository-wide operations such as fetching and pushing run in the main worktree, so a relative URL means what it means there; a revision named to `start` or `contribute` is resolved where the command runs. When `finish` or `abort` removes the update worktree it ran from, it completes and notes that the current directory is gone.

From a linked worktree, git cannot locate a main worktree whose git directory lies outside it unless the git directory names it; the tool then takes it from the update in progress. Without an update, run `start` in the main worktree; elsewhere it refuses, and a relative URL resolves against the git directory.

`start` refuses to begin if any of these hold:

- an update is already in progress, or an interrupted one left the update branch or worktree behind, which `abort` removes;
- `.worktrees`, or a directory between it and the update worktree, is a symbolic link or not a directory;
- the repository is bare and the path of its update worktree contains `/.git`; run the tool in a clone with a checkout;
- the integration tip cannot be fetched, or its configuration names another integration branch or publish remote (see *The integration tip*);
- the repository is a shallow clone (every command but `abort` refuses one, naming `git fetch --unshallow`);
- in merge-request mode, the forge cannot be reached as `finish` needs it, for example without a token, or the update's merge request is open, which `sync` carries on and `abort --merge-request` withdraws;
- a selected upstream is unknown, named more than once, cannot be fetched, or was rewritten;
- a revision named for an upstream is refused (see *Choosing a target*), or the upstream's tags cannot be listed;
- an upstream with `tags`, named without a revision, has no release tag on its fetched branch (see *Release tags*);
- a path upstream without a sync record finds its path holding content unrelated to the upstream's history, or a file where a directory of its path would go;
- a path upstream that was imported before finds its path removed on the integration branch;
- a port upstream has an invalid journal, or neither a journal nor a last synced commit to start one from.

No checkout needs to be clean: `start` works in a worktree of its own.

`start` then:

1. Fetches the integration tip and the selected upstreams, all of them when none is named.
2. Records each follow upstream's **target**: the fetched tip, the commit a revision names (see *Choosing a target*), or with `tags` the newest release tag (see *Release tags*), with the upstream tag that names it. The integration tip is the update's **base**. Targets never change during the update.
3. Queues, in configuration order, the follow upstreams whose target the base lacks and the port upstreams with pending commits; the others are reported as current. With nothing queued, nothing is created.
4. Creates the **update branch** `git-intake/update` at the base, in the worktree `.worktrees/git-intake/update` of the main worktree, never inside the git directory, whose `/.git` path compilers, bundlers and watchers ignore. Unless the path is ignored already, it adds `/.worktrees/` to the repository's `info/exclude` once; a committed `.gitignore` is never changed. The files of the main checkout are never touched, but a tool that crawls its dot-directories may see the update's tree. A bare repository's update worktree is `<git-common-dir>/git-intake/worktree`.
5. Enables `rerere.enabled`, so a resolution recorded once is reapplied when the same conflict recurs, in a merge or a copy; such files are reported as *resolved by a recorded resolution*, to review and stage like any other.
6. For a port upstream, writes the plan and stops (see *Port mode*). For follow upstreams, merges each queued target with its own merge commit:
   - a root upstream: `git merge --no-ff`;
   - a path upstream's first import (no sync record, path absent): an `ours` merge and `git read-tree --prefix=<path>/`, keeping the upstream's full history. History shared only through a common ancestor, such as a template, is no import. The import is all or nothing;
   - a path upstream, later updates (it has a sync record): `git merge --no-ff -X subtree=<path>`;
   - a path upstream imported by other means, such as `git subtree add` without `--squash` (no sync record, but a merge on the integration line brought the upstream's history into its path): the same subtree merge, which writes the first sync record. Any other content at the path is refused, a `git subtree add --squash` import included, which `status` shows as `not imported`.

   Merges run with `--no-log`, so git configuration cannot push the trailers out of the last paragraph. An upstream counts as merged once its commit carries the sync record, merges the target, and has the path of a path upstream; a hook that rewrote the message stops the tool. After an interruption, `continue` recognises a merge already committed with its record.

On a conflict the tool stops with exit status 1, naming the upstream, the conflicted files, and the upstreams merged and pending. Only files git's rerere reports as resolved (including those `rerere.autoupdate` staged) are labelled *resolved by a recorded resolution*. Resolve, stage and run `continue`. To leave one upstream for later, `abort` and `start` again with only the others named.

`verify` runs `install` and then every check, in order, through `sh -c` in the update worktree, once every queued upstream is merged or carried out. The checks are those of the configuration the update recorded: at the integration tip at `start`, or at its last catch-up merge. Each command gets the update's variables (see [The environment of the checks](#the-environment-of-the-checks)) and logs to `<git-common-dir>/git-intake/logs/verify/install.log` or `check-<name>.log`. A failed `install` skips the checks. Verification fails when:

- the update branch's `.git-intake.toml` is missing, invalid or names another integration branch (the commands are skipped);
- a command fails or is skipped;
- the commands change tracked files;
- the update branch moves while the checks run.

When it passes, `verify` records the commit it verified.

### Running some of the checks

`verify --only <name>` runs only the named check, `verify --skip <name>` every other one; each repeats, a repeated name counts once, and the two do not combine. The install command always runs, the checks keep the update's order, and the preconditions and variables are those of a full run. A name the update's checks lack is refused with status 2, naming the checks, and nothing runs. A check a catch-up merge brought counts from then on.

A run that leaves out a check is a **partial run**. It is recorded beside the verification, with its commit, the checks it ran, their outcomes and its own logs in `<git-common-dir>/git-intake/logs/partial/`; the verification's logs stay as they are. It never verifies the update nor changes its verification, and partial runs do not add up: `--only lint` then `--only test` leave the update unverified. A later partial run replaces it; a full run or a catch-up merge drops it with its logs. A selection that leaves nothing out is a full run; one that skips every check runs only `install`.

A partial run exits 0 when everything it ran passed and 1 otherwise, and says that it did not verify the update, and whether a full run still verifies the head. `status` shows it apart from the verified commit.

In push mode, `finish` checks before it pushes anything (for merge-request mode see *Publication through a merge request*):

- every upstream is merged or its plan carried out;
- the worktree head is the verified commit, or a catch-up merge the tool made, which `finish` verifies itself by running every check; a commit added after `verify` needs a new `verify`, and a head with only a partial run is refused, naming the checks left out;
- the base is still the integration tip; otherwise `finish` catches up first (see *Catching up with a moved integration branch*);
- the verified commit contains the base, every merged target, and every copy and journal commit of a port, so the integration branch only fast-forwards.

`finish` fetches the integration tip, refusing with git's reason when it cannot. When the tip already contains the verified commit, it pushes nothing and completes the update. Otherwise, in push mode (`via = "push"`, the default), it pushes exactly the verified commit, `<commit>:refs/heads/<integration>`, without force, and moves no local branch. It then removes the worktree, the update branch, the plan, the update's references, its logs and the state.

A rejected push, as when other work landed meanwhile or the branch is protected, publishes nothing and keeps the update, verified. Run `finish` again: it pushes again, or, when the tip moved past the base, catches up, verifies and pushes that. When the remote never accepts the push, `abort` ends the update; merge-request mode suits a protected integration branch.

### Catching up with a moved integration branch

When other work lands on the integration branch during an update, `finish`, or `sync` for an open merge request, takes the new tip in with a **catch-up merge** instead of starting over: its first parent is the new integration tip, its second the update branch head, and it carries a trailer:

```
catch-up: merge main at 1a2b3c4 into the update

Intake-Catch-Up: main
```

Every resolution and fix on the update branch stays as it is; only real conflicts between the new integration commits and the update stop it. The integration branch's first-parent line stays the line of commits it received, with the update on the second-parent side. The base becomes the tip caught up with and the update counts as unverified; targets, last synced commits and port plans stay. A further move adds another catch-up merge on top, so the update branch only moves forward.

The tool records the tip in the update state, checks it out detached in the update worktree, merges the update head into it, and moves the update branch to the merge with `git update-ref` against the recorded head, refusing and naming both commits when the branch moved meanwhile. After an interruption, `status`, `continue` and `finish` recognise a catch-up in progress or committed and never make it twice. A catch-up merge committed by hand while detached, with or without commits on top, is kept: the branch moves to the worktree's commit, and commits on top need `verify`. A catch-up given up by hand, with the merge aborted and the update branch checked out again, is forgotten once `continue` or `finish` sees it.

A conflicted catch-up stops `finish` or `sync` with status 1 in the step `catch_up_conflict`, naming the files; the integration branch is *ours*, the update *theirs*. Resolve, stage and run `continue`, which commits the catch-up merge and names `finish`. In merge-request mode the next `finish` publishes it without running the checks; in push mode it verifies it as `verify` does, and failing checks stop it in the step `catch_up`, unverified. Verification after a catch-up uses the install command and checks of the configuration at the catch-up merge. The catch-up merge drops a partial run, and a partial run on an unverified catch-up merge does not stop `finish` from verifying it. `finish` makes at most one catch-up merge per run, concluding one left by an earlier run included: when the branch moves again meanwhile, it pushes nothing and says to run it again.

A catch-up refuses, merging and pushing nothing, a tip that would change what the update means. It names `abort`, after which `start` plans the update from the new history; for `sync` it names the merge request, `abort --merge-request` with the warning that withdrawing declines its targets, and `start` and `finish` to publish them again. It refuses:

- a tip that does not contain the base: the integration branch was rewritten;
- a commit that carries a sync record or a copy of an upstream the update takes in, as when another clone took that upstream in; a sync record counts only on a merge whose second parent is its target;
- a change of the journal of a port upstream the update takes;
- a change of `.git-intake.toml` to the integration branch, the publication setting or an upstream, which the update fixed at `start`; the install command and checks may change.

`abort` works in a catch-up step as in any other.

### The environment of the checks

A check that compares the tree with a baseline, such as a ratchet or an audit since `merge-base origin/main HEAD`, would count every upstream change an update brings as the downstream's own. The right baseline is the upstream's target: `GIT_INTAKE_HEAD` against it measures the downstream's whole delta, and comparing that with the delta between `GIT_INTAKE_BASE` and `GIT_INTAKE_UPSTREAM_<NAME>_PREVIOUS` judges the update alone. `verify` gives `install` and every check these variables, with full commit ids:

| Variable | Value |
| --- | --- |
| `GIT_INTAKE_BASE` | The integration tip the update lands on: the tip it started from, or the tip of its last catch-up merge, so that comparing it with `GIT_INTAKE_HEAD` measures the update alone. A catch-up changes no upstream variable. |
| `GIT_INTAKE_HEAD` | The commit being verified, the head of the update branch. |
| `GIT_INTAKE_INTEGRATION` | The name of the integration branch. |
| `GIT_INTAKE_UPSTREAMS` | The names of the upstreams the update takes, separated by spaces, in the order they were merged or carried out. An upstream that was current at `start` is not among them. |
| `GIT_INTAKE_UPSTREAM_<NAME>_MODE` | `follow` or `port`. |
| `GIT_INTAKE_UPSTREAM_<NAME>_TARGET` | Follow mode: the target merged. Port mode: the newest upstream commit the update takes or decides on, which does not move the journal's base and need not be an ancestor of `GIT_INTAKE_HEAD`; absent when the update takes and decides none. |
| `GIT_INTAKE_UPSTREAM_<NAME>_PREVIOUS` | Follow mode only: the last synced commit before the update; absent on a first import or when the history names none. |

`<NAME>` is the upstream's name in upper case with every character but letters and digits replaced by `_`: `app-a` gives `GIT_INTAKE_UPSTREAM_APP_A_TARGET`. Every commit named stays in the repository while `verify` runs. `GIT_INTAKE_` variables the tool inherits are not passed on, and git commands the tool runs see none.

The target is a tree baseline only for a root upstream; a path upstream's target has the upstream's own layout, so compare the path's subtree.

A fork with one root upstream named `upstream` can give an audit the target as its baseline:

```toml
checks = [
    { name = "test", run = "pnpm test" },
    { name = "audit", run = 'FALLOW_AUDIT_BASE="$GIT_INTAKE_UPSTREAM_UPSTREAM_TARGET" fallow audit' },
]
```

A script the repository's CI also runs can fall back outside an update: `"${GIT_INTAKE_UPSTREAM_UPSTREAM_TARGET:-$(git merge-base origin/main HEAD)}"`. A merge request pipeline gets the same variables from `git intake env` (see *The update variables outside verify*).

### Choosing a target

By default a follow upstream is taken at its fetched tip, or with `tags` at its newest release (see *Release tags*). To take it at another point, name a revision:

```sh
git intake start upstream=v2.3              # a fork: take the upstream's tag v2.3
git intake start app-a=1a2b3c4d app-b       # app-a at a commit, app-b at its fetched tip (without `tags`)
```

The name ends at the first `=`; naming an upstream twice is refused. The revision resolves as follows:

- A tag of that name in the upstream's repository, as `git ls-remote --tags` lists it from the URL, names its commit, peeled through an annotated tag; `tags/` or `refs/tags/` in front is dropped. No tag is fetched. When the URL cannot be asked, `start` refuses, naming the upstream, its URL and git's error.
- Otherwise the revision resolves in the repository like any `git rev-parse` argument, after the fetch: a commit id or an expression such as `1a2b3c4d~2`. A revision that resolves through one of the repository's own tags, alone or at the start of an expression, is refused, even when a branch has the same name: the repository's tags are not the upstream's.

Only a plain tag name is looked up on the upstream, so an expression on a tag only the upstream has, such as `v2.3~1`, names nothing; name the commit instead.

The commit must be on the fetched upstream branch, the only one fetched. With `L` the upstream's last synced commit:

- a target older than `L` is refused, because an update never moves an upstream back;
- a target the integration branch already contains, such as `L`, is reported as current;
- a target that does not contain `L`, such as a side branch the upstream merged after `L`, is refused;
- otherwise the upstream is queued at that target.

The sync record names the target and an upstream tag pointing at it, if any; `status` and `report` show that tag, or else the revision unless it is the target's id. `status` then counts the upstream `behind` by the commits up to the fetched tip, or with `tags` up to the newest release.

A revision overrides `tags`: it takes any commit of the fetched branch, released or not.

### Release tags

A fork that must take only released versions sets `tags` on its follow upstream:

```toml
[[upstream]]
name = "upstream"
url = "https://example.com/team/app.git"
branch = "main"
tags = "v*"
```

Without a revision, `start` then takes the newest **release**: of the upstream's tags, as `git ls-remote --tags` lists them, those that

- match the pattern,
- are a semantic version once the pattern's literal prefix (before its first `*`, `?` or `[`) is removed, with one `v` or `V` after it unless the prefix itself ends in `v` or `V`: `v0.21.25` is 0.21.25 under `v*`, `release-v3.1.4` is 3.1.4 under `release-*`, and `vv1.2.3` is no version under `v*`,
- have no build metadata, and no prerelease part unless `prereleases = true`,
- and point at a commit of the fetched branch,

the highest by semantic versioning precedence; other tags are ignored. Name a prerelease or any other commit to take it: `start upstream=v1.0.0-rc.1`. No tag is fetched, so an update never changes the repository's tags.

The usual target rules apply (see *Choosing a target*). When the integration branch contains the release, the upstream is current, even if an earlier update took a later commit by name. Without any release, `start` refuses and says to name the target, `status` shows `no release`, and `status --check` fails.

`status` shows the newest release in the `TIP` column and counts `BEHIND` against it, with a line naming the fetched tip and how far it is past the release. In `status --json`, `tip` stays the fetched tip, and `tags`, `release`, `release_tag` and `commits_after_release` describe the release. `log` lists commits up to the release, and `status --check` predicts the conflicts of merging it.

An upstream that tags daily builds as prereleases is followed build by build with `prereleases = true` and a pattern that selects them:

```toml
[[upstream]]
name = "upstream"
url = "https://example.com/team/app.git"
branch = "main"
tags = "v*-nightly.*"
prereleases = true
max_conflicts = 150
```

`v1.2.3-nightly.20260102.8` is newer than `v1.2.3-nightly.20260101.7` and older than `v1.2.3`. The release commands never count a prerelease as the upstream's release (see *Release versions*).

#### Conflict limit

With `max_conflicts`, an update named without a revision takes, of the tags that count, contain the last synced commit and the integration branch lacks, the newest whose merge `git merge-tree` predicts to conflict in at most that many files, or else the oldest, so the downstream still moves forward. `start` and `sync` name the tag taken, the files predicted and the newer tags passed over:

```text
upstream: takes v1.2.3-nightly.20260102.8, predicted to conflict in 120 files, within max_conflicts = 150; passed over v1.2.3-nightly.20260103.9 (300 files) for the limit
```

A named revision overrides the limit. `status` counts `BEHIND` against the newest tag; `status --check` names and predicts the tag an update would take, which `status --json` writes as `next_tag`, `next_target` and `next_within_limit`.

### Port mode

The unit of port mode is a commit on the first-parent line of the upstream branch, where a hosted project lands each change; a merge commit is taken as its change against its first parent (`git cherry-pick -m 1`).

**The journal.** A port upstream's decisions live in `.git-intake/<name>.toml` on the integration branch, read at the integration tip like the configuration:

```toml
base = "<the upstream commit the port started from>"
retired = ["<upstream commit>"]   # optional: see *Recovering from a rewrite*

[[decision]]
commit = "<upstream commit>"
verdict = "skip"          # or "have": the product already has the change
note = "optional reason"
```

Commit ids are full ids, upper case read as lower case. The tool sets the base when it creates the journal and never moves it. A taken commit is recorded by the commit that took it, not in the journal. A journal with another verdict, a commit decided or retired twice, or ids that are not full is refused, naming the problem. The journal is an ordinary file, reviewed with the code; removing a skip makes its commit pending again.

**Pending commits** are the first-parent commits of the fetched branch after the base, minus those a commit on the integration branch names as taken in its trailers `Intake-Upstream: <name>` and `Intake-Taken: <upstream commit>` (a copy made by hand with both counts), minus those the journal decides. When the base, or a decided or taken commit not retired, is off that line, the upstream was rewritten: `status` reports it and `start` refuses it.

**Recovering from a rewrite.** `status` (in its text and in `off_line` of its JSON) and `doctor` name each commit taken or decided that left the line, with what it was recorded as, and the base if it is gone. List them under `retired` in an ordinary commit, and move a lost base to a commit of the current line. Retired commits count as neither taken nor decided, so their rewritten versions show as pending and the next plan can mark them `have`; a retired commit needs no object in the repository. While the base is off the line, the upstream stays rewritten.

**The first plan creates the journal.** Without one, the base is the last synced commit, from the newest sync record or else the history, so a followed or cloned upstream starts porting where it is; the first update that carries out a plan commits the journal. With neither a journal nor a last synced commit, `status` shows `no base`, `status --check` fails, and `start` refuses, each saying to write the journal by hand.

**The plan.** When there are pending commits, `start` creates the update worktree and branch, writes the plan to `<git-common-dir>/git-intake/plan`, and stops:

```
# git intake plan for example-app: 4 pending commits, oldest first.
# take  copy the commit onto the update branch
# skip  record that the product does not take it
# have  record that the product already has it
# later leave it pending
# Takes are copied in the order listed here. Text after the subject and ' # ' is the
# note of a skip or have decision. A commit left out of the plan stays pending.
later 1a2b3c4d5e6f fix(web): keep the composer focused after a paste
later 5d6e7f8a9b0c feat: add a settings page for keybindings
later 9a8b7c6d5e4f chore: update dependencies
later 0f1e2d3c4b5a fix(server): close the watcher on shutdown
```

Edit it and run `continue`. A line is a verb, a commit (any unique prefix of at least four hex digits of a listed commit), and optionally the subject and a note after ` # `; comments and empty lines are ignored. A plan that names an unlisted commit, names one twice, uses another verb, or decides nothing is refused, naming the line, and nothing changes. `start <name>=<revision>` limits the plan to the pending commits the revision contains, resolved as under *Choosing a target* without the last-synced rules.

**Carrying out the plan.** `continue` copies each `take` in the plan's order, keeping the upstream author and message and adding the trailers:

```
Intake-Upstream: example-app
Intake-Taken: <full id of the upstream commit>
```

A conflicted copy stops as a conflicted merge does, naming the commit, the files, and the commits copied and still to copy, with status 1; resolve, stage and `continue`. A copy committed by hand with both trailers is accepted. A copy that changes nothing is not committed but recorded as `have` with the note `taken, but its copy changed nothing`, which `report` names.

When the commit of a copy fails, as when a pre-commit hook rejects it, the update stops at the step `commit_failed` with git's output and status 1, the copy staged. Fix and stage; `continue` commits what is staged as the copy. `continue` never resets the worktree. A resumed copy whose changes are in the worktree but not staged stops the same way; a copy changes nothing only when neither the index nor the worktree differs.

A copy never changes `.git-intake.toml` or `.git-intake/`: those paths are restored from the update branch head before the commit, conflicts included, and named in `continue`'s output and in `report`; a copy that changed nothing else is recorded as `have` with the note `taken, but its copy changed nothing outside .git-intake.toml and .git-intake/`. Last, the `skip` and `have` decisions are committed to the journal in one commit, which creates it if needed. A plan without decisions on an existing journal adds no journal commit. `later` commits and those left out stay pending.

For each taken commit, `report` lists the earlier pending upstream commits that changed the same files, as likely dependencies to check.

### Contributing to an upstream

```sh
git intake contribute example-app fix-watcher        # a branch at the fetched tip
git intake contribute example-app=v1.4 fix-watcher   # a branch at the upstream's tag v1.4
```

`contribute` creates the local branch at the upstream's fetched tip, or at a revision on the fetched branch resolved as for `start` without the last-synced rules. It holds only upstream history: copy the change onto it and push it where pull requests to the upstream come from. The tool sets no tracking branch, changes no checkout and never pushes it, and refuses an existing branch or an invalid name.

### Publication through a merge request

With `via = "merge-request"`, nobody pushes to the integration branch, so it can stay protected. Once published, the merge request is the update: its pipeline verifies it, the forge merges it, and `git intake sync` keeps it current from any clone or CI job. The tool reaches GitLab or GitHub through its REST API and requires that the forge merges only a tree its required pipeline tested. `finish` needs no local verification in this mode; `verify` still runs the checks locally.

`finish` then, moving no local branch:

1. Reaches the forge (see *The forge*), and refuses, pushing nothing, when it cannot: an unrecognised forge, no token, or a rejected token or request, with the forge's status and message.
2. Fetches the integration tip. When it contains the update head, the update landed by other means: `finish` pushes nothing, removes the update and says so. When it moved past the base, `finish` catches up (see *Catching up with a moved integration branch*) without running the checks.
3. Reads the forge's merge settings and refuses, pushing nothing, unless they guarantee that only a tree the required pipeline tested is merged (see *Merge settings*).
4. Pushes the update head without force to the branch `git-intake/update` on each push URL where it is not there yet. The fetch URL decides the forge; other push URLs receive the branch as mirrors.
5. Opens the merge request from that branch into the integration branch, on GitLab with squashing off and the branch removed on merge, or rewrites the title and description of the open one, also one another run opened meanwhile.
6. Removes the update worktree, branch, plan, references and state, and names the merge request, which carries the update from then on.

A `finish` interrupted before the merge request exists keeps the update; run again, it pushes only where the branch is missing. Messages say merge request on GitLab and pull request on GitHub.

`finish` refuses, pushing nothing and keeping the update, when a merge request of the update into another branch is open, or when the update's open merge request has a head this update does not contain, as when another clone published another update. `git intake abort` and `git intake sync` then carry that merge request on; `git intake abort --merge-request` withdraws it, which also removes this update and declines its targets until `start` and `finish` publish them again. When `git-intake/update` is on the remote without an open merge request, `finish` pushes over it when the update head contains it, deletes it when it holds nothing but catch-up merges beyond the integration tip or beyond the head of a closed or merged merge request of the update, and otherwise refuses, naming the branch, the push URL and `abort --merge-request`. A deletion is pushed with a lease on the commit read on each push URL.

#### The forge

The forge is that of the publish remote's fetch URL after git's `insteadOf` rewriting, as `git remote get-url` reports it. The publication setting is read at the integration tip, never from an update's head, so an upstream cannot point the token at another host.

- **The kind of forge**: the setting `forge`; else the host (`gitlab.com` is GitLab; `github.com` and `ghe.com` subdomains are GitHub); else a GitLab CI or GitHub Actions job whose `CI_SERVER_URL` or `GITHUB_SERVER_URL` names the remote's host.
- **The API base**: the setting `api`, an `https` URL on the remote's host (for an SSH host alias, the host `ssh -G` resolves, an IP address being no alias; for GitHub also `api.<host>`); else, in a CI job of that host, its `CI_API_V4_URL` or `GITHUB_API_URL`, which must name the same host (or `api.<host>` for GitHub) and be `https` unless the job's server is plain HTTP; else `https://<host>/api/v4` for GitLab, `https://api.github.com`, `https://api.<host>` for `ghe.com`, and `https://<host>/api/v3` for GitHub Enterprise Server.
- **The project**: the URL's path without credentials and `.git`, less the path the API base has before `/api/v4` or `/api/v3`, for a GitLab under a relative URL root.

Outside its own CI jobs, a forge on its own domain needs `forge`, and nothing more when its API is at the usual address:

```toml
[publish]
remote = "origin"
via = "merge-request"
forge = "gitlab"
```

Add `api` only where that address is wrong, as for an SSH host alias or a GitLab under a relative URL root; `doctor` shows it by failing to reach the project:

```toml
api = "https://gitlab.example.com/gitlab/api/v4"
```

Without `forge`, a remote on an unrecognised host is refused by the commands that need the forge, naming `forge` and `api` where needed; so is an `api` on another host.

The API's certificate is checked by the operating system's verifier against the system store (on Linux the system bundle or `SSL_CERT_FILE`), so a private certificate authority works once trusted there. `ALL_PROXY`, `HTTPS_PROXY`, `HTTP_PROXY` and `NO_PROXY` apply, in either case. A redirect to another host is followed without the token. GitHub requests ask for API version `2022-11-28`.

#### Tokens

The token comes from the environment only:

| Forge | Variables, in order | Sent as |
| --- | --- | --- |
| GitLab | `GITLAB_TOKEN` | `PRIVATE-TOKEN` header |
| github.com and `ghe.com` hosts | `GH_TOKEN`, `GITHUB_TOKEN` | `Authorization: Bearer` |
| GitHub Enterprise Server | `GH_ENTERPRISE_TOKEN`, `GITHUB_ENTERPRISE_TOKEN`; in an Actions job of that server also `GH_TOKEN`, `GITHUB_TOKEN` | `Authorization: Bearer` |

A github.com token is never sent to an Enterprise Server, except the one an Actions job of that server is given. The release commands also take a GitLab job's `CI_JOB_TOKEN`, sent as `JOB-TOKEN`, and ask without a token when none is set (see *Publication*). Without a token, `start`, `finish` and `sync` refuse before pushing and name the variables. The token is never printed, stored or logged, and goes only to the API's host.

The token must be able to open merge requests:

- **GitLab**: a project, group or personal access token with the `api` scope and at least Developer, or Maintainer where the branch protection requires it for pushing, or for merging, which `land` does. With `replace_after`, `sync` also reads approvals, notes and award emoji. The CI job token cannot open merge requests, and its pushes start no pipeline.
- **GitHub**: a GitHub App installation token or a fine-grained personal access token with *Pull requests: write*, *Contents: write* (which also shows the merge settings and lets `land` merge) and *Metadata: read*; *Administration: read* only for classic branch protection. With `replace_after`, `sync` also reads reviews, comments and reactions, the last needing *Issues: read*, without which every pull request counts as touched. On a private repository `land` reads check runs, which need *Checks: read* that only an App's token gets, and commit statuses (*Commit statuses: read*): use an App token with both, or a classic token with `repo`. Not Actions' `GITHUB_TOKEN`: its pushes start no workflow, and its pull requests wait for someone to approve their runs.

Pushes go through git with the remote's own credentials; the tool never puts the API token into git's configuration. They must be credentials whose pushes start the forge's pipeline.

#### Merge settings

The guarantee needs a merge that cannot happen without a successful pipeline, of exactly the tree that lands; squash and rebase merges are excluded too, since they drop the merge commits that carry the sync records. `finish` and `sync` read the settings before every push of the update branch, and `doctor` reads them; a refusal names each setting to change.

On **GitLab** (every field is readable with read access):

- *Pipelines must succeed* is on (`only_allow_merge_if_pipeline_succeeds`), and *Skipped pipelines are considered successful* is off (`allow_merge_on_skipped_pipeline`).
- The merge method is *Merge commit with semi-linear history* (`rebase_merge`) or *Fast-forward merge* (`ff`), which merge only a branch that contains the target, so the merged tree is the one tested. Plain *Merge commit* is refused, merged results pipelines included, since they can be outdated at merge time.
- Merge trains are off.
- *Enable automatic rebase prior to merge* is off (`automatic_rebase_enabled`; absent counts as off); `sync` keeps the branch mergeable by catch-up merges instead.
- *Squash commits when merging* is not *Require* (`squash_option` `always`). With *Encourage* or *Allow*, the tool opens the merge request with squashing off and the description asks not to squash; `doctor` notes the setting without counting it as a problem. A per-branch squash setting of GitLab Premium is not read; a squash that happens shows as a merge request merged without its history.

On **GitHub**, for the integration branch:

- A required status check, naming at least one check, with *Require branches to be up to date before merging* (strict), through a ruleset or classic branch protection.
- No merge queue applies.
- No rule requires a linear history.
- Merge commits are allowed: `allow_merge_commit`, or a ruleset's pull request rule that allows `merge`.

The rules come from `GET /repos/{owner}/{repo}/rules/branches/{branch}`, readable by any token that reads the repository; classic protection needs administration access and the merge settings push access, and what the token cannot read is unknown, not false. A requirement that cannot be confirmed is refused, naming it and both ways out: a ruleset, or the access the token lacks. Selectable squash or rebase merges are `doctor` warnings.

The guarantee is the forge's, with its limit: whoever may bypass its rules can merge anything.

#### Title and description

The title names the updated upstreams. The description lists each follow upstream's merged range with the tag of its target, and each port upstream's taken, skipped and present commits, and asks to merge without squashing or rebasing and to keep the merge request current with `git intake sync`. When the update changes the forge's CI definition (the project's CI configuration file on GitLab, `.github/workflows/` on GitHub) or the install command or checks, `finish` and `sync` name the changes in their output and the description: each check added, removed or changed by name, or that only the order changed; a renamed check counts as removed and added. When `.git-intake.toml` differs between base and head and either side cannot be read, the install command and checks are named as changed, without names.

Every push of the update rewrites the title and description, and on GitLab asks again for squashing off. The description ends with a block that renders as nothing and records each follow target:

```
<!-- git-intake targets
upstream 0123456789abcdef0123456789abcdef01234567
-->
```

`sync` reads it back, to tell the targets a closed merge request declined and whether a merged one kept the update's history. A description without a readable block records nothing. An over-long description is shortened before the block.

A merge request `sync` replaces (see *Unattended synchronisation*) is closed with a description that says so and records no targets, so it declines nothing; its replacement's description names it and the releases that caused it.

#### Merging

Merge the merge request by a merge commit or fast-forward, by hand or with `git intake land` (see *Landing an update*). A fix someone pushes to its branch is checked by the pipeline and kept. After the merge there is nothing to do locally.

A merge request **merged without its history** is one whose final head the integration branch lacks, or whose recorded follow targets lack a sync record on the integration line: a squash, or a rebase before a fast-forward. `status` names the newest one, and `sync` proposes the same targets again with a warning, naming it in the new description; each retry waits for a person, so a project that squashes everything gets one open merge request, not a loop. A sync record counts only on a merge commit whose second parent is its target, so trailers a squash message copies do not count.

#### Updates left waiting by an earlier release

An update of state format 4 or 5 in the step `waiting_for_merge_request` is read only to end it. `finish` hands it over to its merge request: it finds the merge request by its recorded number or its branch (`git-intake/update-<commit>` in format 4), says whether it is open, merged or closed, records the resolutions of a catch-up step, and removes the update worktree, branch, references and state, leaving the merge request and its branch for `sync`. `abort` removes the update and keeps the merge request; `abort --merge-request` withdraws it too. Other commands name `finish`. Both work whatever form the configuration's checks have, using the publication setting the update recorded, and never write the state file.

### Unattended synchronisation

`git intake sync` takes the follow upstreams in without local state, from a scheduled CI job or from a person taking over where a job stopped. Each run derives everything from the publish remote, its forge and the upstreams, and carries the update as far as it can. It reads the integration tip as every command does (see *The integration tip*), so a CI clone with `HEAD` detached at a commit of the integration branch needs nothing more. It refuses while an update is in progress, naming its step and the next command.

In **merge-request mode** `sync` runs no install command and no check: the merge request's pipeline verifies the update.

- **The update's merge request is open.** When its head contains the integration tip, nothing changes: `sync` says the update waits for the merge request, names the upstreams that moved on since, and with `replace_after` why it is not replaced. An update's follow targets never change; a replacement ends one update and starts another. When the integration branch moved past the head, `sync` carries the update on locally, catches it up, and publishes it as `finish` does: the catch-up merge, with the head, any pushed fix included, as its second parent, is pushed without force and the title and description are rewritten. Just before the push it reads the merge request and the integration branch again: merged or closed meanwhile ends the run with status 0, any other change with status 2 and the advice to run again. Right after the push it reads the state again and, when the merge request is no longer open, deletes the branch the push recreated while it is at the pushed commit.
- **A rewritten merge request**, whose head lacks the sync record of a follow target its description records, as the forge's rebase button leaves it, or without readable targets carries no update, is refused and named, with the advice that `abort --merge-request` declines its targets and `start` and `finish` publish them again. A person force-pushing their own fix keeps the sync merges and rewrites nothing.
- **Replacing an untouched merge request.** With `replace_after`, `sync` plans an update from the integration tip as for a new update, `prereleases` and `max_conflicts` included. It replaces the open, unrewritten merge request when that plan takes a newer release (a target of an upstream with `tags` the head lacks), leaves no follow upstream out, takes every follow upstream the merge request takes, and the merge request is untouched and was opened at least `replace_after` ago. Releases published meanwhile collapse into one replacement; an upstream without `tags` never triggers one and is taken at its fetched tip.

  Untouched means: every commit on the head's update line is a sync merge of a follow upstream or a catch-up merge; every sync merge records in `Intake-Chosen` that the tool chose its target (see *The sync record*), and for an upstream with `tags` names a tag the newest-release rules admit; every merge's tree is the clean merge of its parents, as `git intake conflicts` recomputes it; and the forge shows no approval (GitLab) or review (GitHub), no requested reviewer or team, no comment but system notes, no reaction on the merge request, no assignee, no draft flag, no automatic merge and, on GitLab, no `merge_after`. Anything a person did keeps the merge request, a pushed fix, an update-button merge, a resolved or amended merge, a revision chosen with `start`, a merge by an earlier release, a review request or any comment, a pause note included, and so does whatever the token cannot read, a plan that would move an upstream of the merge request back, and a merge request GitLab reports `locked`. Checks that ask nothing of the forge come first, then the conflict predictions, then the activity reads; while a pause of the merge queue is recorded, or its record cannot be read, the merge request is kept without reading its activity.

  The replacement is merged locally without the install command or checks, and the merge settings are required as for `finish`. Just before closing, `sync` reads the activity, then the integration branch, the merge request's branch and the pause record in one listing, and last the merge request's state, head, draft flag, assignees, reviewers, automatic merge and schedule: merged, closed, being merged, touched or paused meanwhile ends the run with status 0, any other change with status 2. It closes the merge request with its new description in one request (see *Title and description*): one the forge reports merged, as a queue may do in the same instant, gets its description back and ends with 0; one still open gets it back and ends with 2; one closed with the old description gets the new one in a second request, retried in place, and ends with 2 naming the merge request when that fails. It then deletes the branch under a lease on the head read and publishes the new update from `git-intake/update` as a new merge request.

  When the replacement conflicts, `sync` names the releases and files, discards it, closes and pushes nothing, says that merging or withdrawing the merge request lets the next update take the newer release, waits or catches up as without the setting, and exits 1. A run killed after the close leaves a closed merge request that declines nothing and a branch the next `sync` handles as below; that next run's merge request names the replaced one.

- **No merge request is open.** `sync` plans, merges and publishes an update of every follow upstream whose target the integration branch lacks, as `start` with no upstream named and `finish` would; with every one current it starts nothing.
- **Declined targets.** When the most recently ended merge request was closed without merging and recorded every follow target `sync` would merge, `sync` starts nothing, names it and exits 0. The next update comes when an upstream moves on, or by hand with `start` and `finish`.
- **The update's branch without an open merge request**, as GitHub keeps after a close or a killed run or withdrawal leaves: a branch holding nothing but catch-up merges beyond the integration tip or beyond the head of a closed or merged merge request of the update is deleted under a lease, and the run goes on. A branch holding an update no merge request carries is carried on and gets its merge request, unless its targets were declined, which is refused. Anything else is refused, naming the branch, the push URL and `abort --merge-request`.

In **push mode**, `sync` starts the update, runs `verify`, and when the checks pass runs `finish`.

An upstream `start` would refuse on its own (unfetchable, without a release, rewritten, or whose path cannot take it) is left out and named, and the run exits 1. Port upstreams are never taken; those with pending commits are named.

A conflict, or failed checks in push mode, stops `sync` as it stops `start`, with status 1 and the update in progress. A person then runs `sync` in their own clone, meets the same stop, resolves it, and runs `continue` and `finish`, which for an open merge request pushes to it. Every other end, refusals included, removes the update `sync` created, so the next run meets the same situation.

An update is recreated from history (see *The update line*); `report` cannot show the logs and conflicts of its earlier steps and says so. It records the merge request it was carried on from, and `finish` refuses to publish it once that merge request is no longer open, naming `abort`.

In merge-request mode a run that fails on what may pass, a 5xx or network error of the forge or a failed `git fetch`, `git ls-remote` or `git push` of the publish remote other than a push another run's push rejected, is made again up to three times, five seconds apart; once the branch is pushed, only the request that opens or rewrites the merge request is retried, in place. In push mode only reading the publish remote before the update starts is retried.

Of two runs at once, the second push is rejected as not a fast-forward, which `sync` reports with status 2 and the advice to run again, without trying it again itself; it opens no merge request. `sync` exits 2 when it refused or failed, 1 when it stopped on a conflict or failed checks, found a replacement conflicting, or left out a follow upstream, and 0 otherwise.

### Landing an update

`git intake land` carries the update's open merge request to the integration branch, from any clone:

```sh
git intake land                  # wait, catch up, merge; give up after six hours
git intake land --timeout 90m    # give up sooner
git intake land --resume         # only end a pause of the merge queue that a killed run left
```

It refuses while an update is in progress, except `--resume`, and requires the merge settings as `finish` does. Then every `--poll` seconds (60 by default) it reads the merge request and the pipeline of its head, the one the forge's must-succeed rule reads:

- While the pipeline runs, or the head has none yet, it waits, even when the integration branch moves meanwhile.
- When it passed and the head contains the integration tip, it merges that head by its commit, so a head moved meanwhile is not merged, never squashed or rebased, with the source branch removed, and exits 0 once the forge reports the merge.
- When it passed and the integration branch moved past the head, it catches up as `sync` does and waits for the new pipeline; a move during the catch-up only makes it read again.
- When it failed or was canceled, it exits 1 naming the pipeline; push a fix to the branch and run `land` again.
- When the catch-up conflicts, it stops in `catch_up_conflict` with status 1; resolve, run `continue` and `finish`, then `land` again.

With no open merge request, or one a person merges or closes while it waits, it says so and exits 0. It never replaces the merge request; when `sync` replaces it meanwhile, `land` merges nothing, says so and exits 0. It exits 2 when refused, when the forge fails or refuses the merge, and after `--timeout` (six hours by default; seconds, or `s`, `m` or `h`).

An outage does not end the wait: a 5xx, a network error or any failed `git fetch`, `git ls-remote` or `git push` of the publish remote makes it read again at the next poll, and it gives up after ten failed rounds in a row. A push rejected by another run's push, and a lost swap of the pause record, are final. A merge the forge refuses while it reads the merge request as mergeable is retried up to five times; a refusal names the merge status and `merge_error`. SIGINT, SIGTERM and SIGHUP end the pause, if any, and exit 130, 143 and 129; a second SIGINT or SIGTERM ends the process at once, a second SIGHUP changes nothing, and under `nohup` SIGHUP stays ignored. Output that can no longer be written is lost, and the pause is still ended.

In CI, give `land` a `--timeout` shorter than the job's limit, or a killed job may leave a pause for `land --resume`. In a GitLab job, push with an access token (see *Running in CI*): `land` refuses a clone that would push with the job token.

#### The merge queue

A merge queue moves the integration branch one merge at a time, sending the update back to a new pipeline each time, so it can lose that race indefinitely. With `queue`, the account of a queue that takes the merge requests assigned to it, as [marge-bot](https://gitlab.com/marge-org/marge-bot) does, `land` holds the queue for the update's last pipeline:

```toml
[publish]
remote = "origin"
via = "merge-request"
queue = "merge-bot"
```

`land` pauses the queue only when the update's pipeline passed and the integration branch moved past its head, so a pause lasts one pipeline. When the merge status says the merge request cannot merge for a reason a catch-up does not change, such as a draft, unresolved discussions or missing approvals, it says so with status 2 and touches nothing; when the catch-up is predicted to conflict, it goes on to that stop without pausing.

The queue's merge requests are those open into the integration branch and assigned to the account, other than the update's, in the order of their latest assignment. One whose head contains the integration tip and whose pipeline runs, waits or passed is being landed: it keeps its assignment and `land` waits for it first, one at most; the others are held, and a passed one the queue does not take is held after a few polls.

Before taking any assignment, `land` records the pause in the reference `refs/git-intake/pause` on the publish remote: a parentless commit whose message holds the account, the merge request the pause is for, the held merge requests in return order with the assignees each kept, and those in flight:

```
git intake: a pause of the merge queue

{
  "account": { "id": 42, "username": "merge-bot" },
  "merge_request": { "number": 3, "url": "https://gitlab.example.com/team/app/-/merge_requests/3" },
  "held": [ { "number": 7, "others": [] }, { "number": 5, "others": [11] } ],
  "in_flight": [ 9 ]
}
```

Each write is an atomic swap, `git push --force-with-lease`, requiring the reference absent or at the value read, so two runs stay apart; a run that loses the swap touches no assignment and stops with status 2. Writing takes the right to push, and reading takes git alone. The record is read back from where git fetches the publish remote before any assignment is taken: when the remote pushes elsewhere, `land` removes what it pushed and stops with status 2. When the record cannot be read back, it stays, nothing is touched, `land` stops with status 2, and `land --resume` ends the pause. A run whose record a `--resume` removed goes on without pausing again. The reference is no branch or tag, so pushing it starts no pipeline.

Then `land` takes the account's assignment from each, keeping other assignees; one assigned during the pause is held too, after those held before it. The pause ends on every way out, a merge, a failed pipeline, a conflict, a refusal, a timeout, a signal or an error: each held merge request still open with the assignees the pause left gets the account back, in order and at least a second apart; one merged, closed, deleted or reassigned meanwhile is left alone. A 404 counts as deleted only while the project still answers the token; otherwise the pause stays for `land --resume`. Each step is retried a few times. When git cannot read the record but the forge answers, the merge requests go back and the record stays for `land --resume`. The reference goes last.

Notes on the update's merge request say when the queue is paused, for whom, and when it resumes; they decide nothing, and one that cannot be written is a warning.

A killed run leaves the reference. `status` and `doctor` name it and the merge requests it holds, `sync` and `start` warn of it, a plain `land` carries it on, and `land --resume` ends it from any clone, in any update state and whatever became of its merge request, doing nothing else; with no pause recorded it says so. An unreadable record is reported as such, never as no pause.

The queue is a GitLab setting: on GitHub every command that reaches the forge refuses `queue`, as does `doctor`, which also checks that the account exists. The token must be able to change assignees, which Developer allows.

### The update line

`env`, carrying an update on and telling a rewritten merge request need the commits that are an update's; the commits between head and base are too many, since a sync merge brings in the upstream's whole history. The **update line** walks from the head along first parents through the commits the base lacks, crossing to the second parent only at a catch-up merge (marked `Intake-Catch-Up`) and at a head that merges the update into a commit the base contains without a sync record, as a merged results pipeline tests. A merge by the forge's update button stays on the line. The update's own commits are the sync merges, copies and journal changes on that line; a line without any carries no update.

### The update variables outside verify

`git intake env` gives a merge request pipeline, on a fresh clone without update state, the variables `verify` gives the checks (see *The environment of the checks*), for `HEAD` or a named commit, read from history alone:

```sh
git intake env -- ./scripts/check.sh   # run a command with the variables
git intake env --dotenv > intake.env   # NAME=value lines, for a dotenv report or $GITHUB_ENV
git intake env --json                  # one JSON object
git intake env                         # export lines, quoted for the shell
```

The integration tip is fetched as *The integration tip* describes, from where the configuration at the commit, or else at `HEAD`, says; a failed fetch refuses with git's reason. For a commit H, the base is the merge base of H and that tip: the tip the update started from or last caught up with, or for a merged results pipeline the target it merges into. The upstreams and targets come from the update line and the previous synced commits from the integration line at the base, so the values are those `verify` gives.

With a command after `--`, `env` runs it with the variables, without inherited `GIT_INTAKE_` variables, and exits with its status; when it cannot read the variables, as in a shallow clone or without the publish remote, it exits 2 without running it. A commit the integration tip contains, or that carries no update, gets no variable: `env` says so on standard error and exits 0, `--json` prints `{}`, and a command after `--` runs without `GIT_INTAKE_` variables, so a check's fallback baseline keeps working.

Prefer `env -- <command>` or a `--dotenv` file: `eval "$(git intake env)"` loses `env`'s exit status.

### What an update changed in the delta

A downstream's **delta** over its upstream is large; what an update changes in it, its resolutions and the lines the upstream moved around the downstream's changes, is usually small. `git intake delta` shows that:

```sh
git intake delta                        # in the update worktree, or on a merge request's head
git intake delta git-intake/update      # a named commit
```

It compares with `git range-diff` two unreferenced one-commit series: the base's tree on the upstream's previous target, and the commit's tree on the target. Unchanged parts of the delta and cleanly merged upstream changes do not show; a resolution that dropped an upstream line shows it. Commit and base are read as `env` reads them, so `delta` works in an update, on a merge request's head and in its pipeline.

It refuses with status 2 anything but an update of exactly one root follow upstream with a previous target.

### Conflicts resolved by updates

`git intake conflicts` lists the files that conflicted in the updates the integration branch received, as the parts most worth a second look:

```sh
git intake conflicts                                   # the whole history of the integration branch
git intake conflicts --since v1.4.0 --path 'src/ui/**' # after a commit, under some paths
git intake conflicts --json                            # each merge with its paths
```

It recomputes the conflicts of each sync and catch-up merge on the integration line from its parents with `git merge-tree`, needing no record, and prints the files one per line, sorted and unique; an update landed with a catch-up merge counts with both. `--since COMMIT` keeps the merges after COMMIT; `--path GLOB`, repeatable, keeps the matching paths, `*` within a directory and `**` across. With nothing to list it prints nothing and exits 0. The integration tip is read as `env` reads it. `--json` writes `merges`, newest first, each with `commit`, `upstream` (or null), `catch_up` and `paths`, and `paths`, the list the text form prints.

### Abort

`abort` removes the worktree, the update branch, the plan, the update's references, its logs and the state at any step, a catch-up step included, and changes nothing on the publish remote: what `finish` published stays published. Resolutions made by hand in a stopped merge or copy are recorded first, so a retry reapplies them. Without an update in progress it removes what an interrupted `start` left, needing no publish remote. A worktree with `git-intake/update` checked out anywhere but the tool's place is the user's: `abort`, `start`, `sync` and `doctor` name it and leave it.

`abort` never touches the forge. `git intake abort --merge-request` removes the local update, if any, then closes the update's open merge request and deletes the update's branch on each push URL while it is at the commit read, whether a merge request is open or not, and says what remains or that there is nothing to withdraw. A branch someone moved meanwhile is kept and named; run again after an interruption, it deletes the branch a closed merge request left. Withdrawing declines the targets: `sync` proposes them again only once an upstream moves on, and `start` with `finish` publish them by hand. With no update in progress in merge-request mode, plain `abort` names an open merge request, when it can read the integration tip, or says it could not read the forge; on an update carried on from a merge request, the merge request stays open.

### The update state

The update state lives in `<git-common-dir>/git-intake/state.json`, shared by every worktree and carried by no commit. It records its format and the release that started the update. This release reads format 7, and formats 4 and 5 only for an update waiting for its merge request (see *Updates left waiting by an earlier release*). Any other format is refused, changing nothing; the refusal names both formats, the releases that keep the update's format and the release that started it when recorded, and advises finishing or aborting the update with one of them, then starting again with this release.

Another release of the same format carries the update on, and the first `continue`, `verify` or `finish` it runs warns once. Until the update is pushed to a merge request's branch or carried on from one, the warning advises `abort` and a new `start` to use one release throughout.

### Upgrading to the publish remote's integration tip

Upgrade in this order:

1. Finish or abort every update in progress with the release that started it, except an update waiting for its merge request: this release refuses an update state of format 6.
2. Commit a `[publish]` table naming the publish remote, if the configuration has none; a repository without a hosted remote adds a local bare repository and pushes its integration branch there.
3. Replace `finish --no-publish` in scripts with `finish`, which pushes; a protected integration branch publishes with `via = "merge-request"`.
4. Bring a local integration branch up to date yourself where it is still used: no command moves or creates it.

## The sync record

Every sync merge of a follow upstream looks like this; a catch-up merge carries `Intake-Catch-Up` instead:

```
sync(app-a): merge app-a/main at 1a2b3c4 (v2.3.0)

Intake-Upstream: app-a
Intake-Target: <full commit id>
Intake-Tag: v2.3.0
Intake-Chosen: newest-release
```

`Intake-Tag` names an upstream tag at the target, fixed by `start`, and is absent when none points there. It is the tag a revision named (whatever its name), the release `tags` selected, or else, of the tags at the target, the highest version among those matching `tags` and read as under *Release tags* (prereleases and build metadata allowed), or else the highest among names that are a semantic version with an optional `v` or `V`. Other names, such as `archive/2024`, and any name with a comma are not recorded.

`Intake-Chosen` names the rule by which the tool chose the target: `newest-release` (`tags`), `conflict-limit` (`max_conflicts`) or `branch-tip` (the fetched tip of an upstream without `tags`). A merge of a named revision carries none, even of the release the rules would take, and neither do merges made by earlier releases. Only a merge with it leaves a merge request replaceable by `replace_after`.

`status --json` gives each follow upstream's `last_synced_tag`, from the record of its last synced commit on the integration line, for release tooling. For an upstream named `upstream`:

```sh
set -o pipefail
tag=$(git intake status --json | jq -r '.upstreams[] | select(.name == "upstream") | .last_synced_tag // empty')
```

`tag` is empty when that record has no `Intake-Tag`, the upstream has no record yet, or no upstream has that name. `status` exits 1 when an upstream cannot be fetched, which `pipefail` passes on.

A sync record counts only on a merge commit whose second parent is its target, so trailers a squash commit copies do not count.

A port's journal commit is titled `sync(<name>): record <n> port decisions` or `sync(<name>): start the port journal at <commit>`.

The tool reads its history along the **integration line**: the first-parent line of the integration branch, where each merge is followed by the commits its second parent brought in along that parent's first-parent line (`git rev-list --first-parent <merge>^1..<merge>^2`). So the tool's merges count whether an update was pushed, merged by fast-forward, or merged with a merge commit. A catch-up merge among those commits is followed in turn along its second parent, at any depth, down to what the first-parent merge's first parent contains, so an update that caught up counts as if merged straight in. The `Intake-Catch-Up` trailer is as trustworthy as the sync records beside it: both are trailers the tool writes in history it does not police. Any other merge counts at that one level only, and a sync merge brings in nothing of its own.

A follow upstream's **last synced commit** is the target of its newest sync record on the integration line that no other of its records' targets contains, so an older record merged later never moves it back; it is stored nowhere else. Without a record it comes from the history, from the integration branch and that upstream's branch alone: for a root upstream the newest upstream commit the integration branch contains; for a path upstream the second parent of the newest merge on the integration line whose second parent belongs to the upstream's branch and which changes its path, as `git subtree add` or `pull` leaves, so a shared template does not count, and the import still counts after another upstream merges this one's history. `status` says so, and `status --json` marks the source `history` instead of `record`; the first update writes a record. Records inside a configured upstream's fetched branch, as a stacked branch inherits, are not this repository's and are skipped. When the upstream's branch no longer contains its last synced commit, it was rewritten: `status` reports it and `start` refuses it.

## Downstream releases

git intake versions the downstream's releases by one of two schemes, which `versioning` selects:

- `dated`, the default, for a downstream that ships builds: a dated prerelease of the next upstream patch, `0.11.1-custom.20261101.2`, which names the upstream it carries and gives a monotonic build number.
- `semver`, for a downstream others depend on as a package: its own semantic version, `1.3.0`, from the Conventional Commits since the previous release, which version ranges select; the upstream release goes into the notes and the build metadata.

### Release configuration

```toml
[release]
upstream = "app"         # the follow upstream whose version the releases carry
tags = "v*"              # optional: the glob of its release tags; default: the upstream's own `tags`
labels = ["custom"]      # optional, default ["custom"]: the label of new dated releases, then labels earlier releases took
versioning = "dated"     # optional, default "dated": "dated" or "semver"
```

`tags` under `[release]` is separate from the upstream's own `tags`, so updates may keep taking the branch tip. A release of the upstream is a tag of its repository that matches the pattern and is a semantic version without prerelease or build metadata once the literal prefix and one `v` or `V` are removed: `expo-device-hub@0.19.1` is 0.19.1 under `expo-device-hub@*`, and builds such as `v0.1.7-nightly.20261031.3991` never count. Tags are listed from the URL, never fetched.

### Release versions

Under `dated`, a release is the tag `v<base>-<label>.<YYYYMMDD>.<n>`, and its version is the name without the `v`:

- `<base>` is the **release base** of the tagged commit: the newest upstream release it contains, with the patch number raised by one. A branch that contains `v0.11.0` and later commits has the base `0.11.1`. A commit that contains no upstream release has no base, and the commands refuse it, naming the upstream, its URL and the pattern.
- `<label>` is the first of `labels`.
- `<YYYYMMDD>` is the UTC day the tag is cut.
- `<n>` is one past the highest number any release-form tag of the publish remote took that day, wherever it points, or else 1, up to 99.

Every release must sort after every other by semantic versioning, and `YYYYMMDD * 100 + n` must grow, as a monotonic build number. The tool refuses a release that breaks either order, naming the release it would not follow: an older base, a label that sorts before an earlier label for the same base, or a day's hundredth release.

Releases count only on the **downstream's line**: the first-parent commits of the publish remote's integration branch that no upstream's last synced commit contains (see *The sync record*). A release-form tag of the publish remote with one of `labels`, peeled, is a release only when it points at that line, so upstream tags and tags on second-parent history never count. List the labels of earlier releases after the current one. Under `dated`, `v<x.y.z>` is no release.

### Semver releases

Under `semver`, a release is the tag `v<x.y.z>`. Dated tags with one of `labels` stay releases, so they bound the first semver release and take part in the order.

The **previous release** of a commit is the newest release below it on the downstream's line. The next release follows it by the largest change among the commits since, merges excluded, the upstream commits updates brought in included, by semantic-release's default rules:

- a breaking change, `!` before the colon (`feat!:`, `fix(api)!:`) or a `BREAKING CHANGE:` or `BREAKING-CHANGE:` footer, whatever the type, raises the major version;
- `feat` raises the minor version;
- `fix`, `perf` and `revert` raise the patch version;
- any other type, such as `docs`, `test`, `ci`, `chore` or `refactor`, asks for no release.

A subject that is not a Conventional Commit raises the patch version, so no upstream change ships without a release. Types compare without regard to case; `BREAKING CHANGE:` counts only in the closing footers.

Each semver release records in its tag's trailers the **upstream release** it carries, the newest its commit contained:

```text
Release 1.3.0

Intake-Upstream: app
Intake-Target: <the commit of v0.11.0>
Intake-Tag: v0.11.0
```

The major version also rises when the commit's upstream release is outside the caret range `^<v>` of the one the previous release records, or lower: so `v0.11.0` to `v0.12.0` raises it whatever the commits say. An unmarked breaking change upstream ships under a minor version until someone marks it. An upstream release tagged on a commit an earlier release already contained changes nothing until the next release, which takes the major version. A previous release without a record (a dated one, or a semver tag cut by hand without trailers) compares with the newest upstream release its commit contains. With git before 2.47, a semver tag signed by hand counts as one without a record, since that git reads its trailers together with the signature; `release tag` signs nothing.

The first semver release is `v1.0.0`, or the next major version above earlier releases at 1.0.0 or above: after `v0.11.1-sakiori.20261103.1` it is `v1.0.0`, after `v1.4.1-custom.20261103.1` `v2.0.0`. When the commits since the previous release ask for none, `release tag` says so and creates nothing; the commit that sets `versioning = "semver"` counts like any other, so committed as `chore:` it asks for no release.

Every release must sort after every other by semantic versioning. A version depends only on the commit, its history, the releases below it and the upstream releases their tags record, so `release check`, `release version` and a rerun tag pipeline compute the same version later. For their tag they take its recorded upstream release, which must be a release of the release upstream whose commit the tagged commit contains, read locally; `release check` also reports a tag the upstream lists at another commit.

The upstream's own tags never count as the downstream's releases; a clone that fetches tags from both may see two tags of one name.

### Publication

A release is **published** when the publish remote's forge has a release for its tag, which the project's pipeline creates as its last step (GitLab's `release` keyword, `glab release create` or `gh release create`). git intake only reads forge releases.

The release commands find the forge as in *The forge*, in either publication mode. On GitLab they send `GITLAB_TOKEN`, or in a GitLab CI job of the remote's host `CI_JOB_TOKEN`; on GitHub the variables of *Tokens*; without a token they ask anonymously. A forge that hides the project's releases is refused, naming the token variables.

### Release commands

The release commands fetch the publish remote's integration branch and tags into references of their own, without the remote's configured refspecs, and remove them before they exit. They change no local branch or tag except the tag `release tag` creates.

| Command | Purpose |
| --- | --- |
| `release next [--json]` | For the integration tip: the newest upstream release it contains and, under `dated`, the release base; then the release at the tip and whether it is published, or the release a tag cut now would take (under `semver` with the change that sets it, or that none is due), and the newest release below when it is unpublished. Changes nothing; a forge that cannot be read leaves publication unknown. |
| `release tag` | Cuts a release on the integration tip: creates the next release as an annotated tag `Release <version>` (under `semver` with the trailers of the tip's upstream release) with git's identity, and pushes only that tag, only where the remote lacks it, whatever the push settings; it warns of an unpublished release below. With a published release at the tip it does nothing. With an unpublished one it creates nothing and exits 1: its pipeline is running or failed, and rerunning the failed jobs completes it. Under `semver` with no release due it creates nothing and exits 0. |
| `release check [TAG]` | Guards a tag pipeline: checks TAG, or else `CI_COMMIT_TAG`, against the publish remote and reports every problem: not of release form (under `dated` with the current label), not on the remote, not at the checked-out commit, that commit not on the downstream's line, under `dated` a base other than the commit's, under `semver` a recorded upstream release that does not hold, a version other than the one the commit takes, or not sorting after every other release (under `dated` also by day and number). Only then does it ask whether the release is published, which is a problem too. Under `semver` it checks the tag's own recorded upstream release, not that it is the newest the commit contains. Exits 1 on a problem. |
| `release version [--tag TAG] [--json \| --dotenv]` | Prints the version a build makes. For a release tag (`--tag` or `CI_COMMIT_TAG`), the tag's version, once it has the current label and base (`dated`) or the version the commit takes (`semver`); otherwise it refuses. Otherwise the checked-out commit's rehearsal version, with the UTC day of `CI_PIPELINE_CREATED_AT` or now: `<base>-<label>.<YYYYMMDD>.0` under `dated`, `<next>-rehearsal.<YYYYMMDD>` under `semver`, `<next>` being the next release or else a patch bump. `--dotenv` writes `RELEASE_VERSION`, for a tag `RELEASE_TAG`, and under `semver` `RELEASE_UPSTREAM=<upstream>-<x.y.z>` (characters other than ASCII letters, digits and `-` written `-`). `--json` writes `version`, `tag` (null for a rehearsal), `rehearsal` and under `semver` `upstream`, `previous` (the previous release's tag, or null) and `bump` (`major`, `minor`, `patch` or null) of the versioned commit. It lists the release upstream's tags from its URL, and under `semver` fetches the publish remote's branch and tags. Last synced commits come from the commit's own sync records, a record counting only when the configuration at its merge names that upstream at the same URL and branch; an upstream is fetched only without such a record, so a merge request's build needs no other upstream. |
| `release notes [TAG]` | Writes the Markdown notes of TAG, or `CI_COMMIT_TAG`: the newest release before the tag, then newest first the subjects of the downstream's own commits since it and each upstream update by name and tag, or else abbreviated target; the upstream's own commits are not listed. The first release, and one with nothing to list, say so. Under `semver` they start with the tag's upstream release and how many upstream commits it carries after it, then the breaking changes: the upstream's move when it raised the major version, and each breaking commit with its `BREAKING CHANGE` text. |

`release next`, `release tag` and `status` read the configuration at the integration tip; `release check`, `release version` and `release notes` read it at the checked-out commit, so a rerun tag pipeline behaves as it did.

Of two runs cutting the same release, the second push fails without effect, exits 2 and says to run again. A release tag is never pushed to an upstream, and needs a credential whose pushes start the forge's pipeline.

### Releasing from CI

A scheduled job cuts the release tag; the tag's pipeline checks it, builds with its version and creates the forge release last. On GitLab, protect the release tags (`v*`), so the tag pipeline gets its protected variables, and let the scheduled job's token create them:

```yaml
variables:
  GIT_DEPTH: "0"        # the full history; git intake refuses a shallow clone
  # The checkout, for an image that runs as another user than cloned it; see
  # Running in CI, GitLab.
  GIT_CONFIG_COUNT: "1"
  GIT_CONFIG_KEY_0: safe.directory
  GIT_CONFIG_VALUE_0: $CI_PROJECT_DIR

stages: [check, build, release]

release-tag:
  stage: release
  rules:
    - if: $CI_PIPELINE_SOURCE == "schedule" && $CI_COMMIT_BRANCH == $CI_DEFAULT_BRANCH
  script:
    # Pushes with the access token in GITLAB_TOKEN; see Running in CI, GitLab.
    - !reference [.push-with-token, script]
    - git intake release tag

release-check:
  stage: check
  rules:
    - if: $CI_COMMIT_TAG
  script:
    - git intake release check

# In a tag pipeline, nothing is versioned or built before the check passes.
version:
  stage: build
  needs:
    - job: release-check
      optional: true
  rules:
    - if: $CI_PIPELINE_SOURCE != "schedule"
  script:
    - git intake release version --dotenv > release.env
  artifacts:
    reports:
      dotenv: release.env

build:
  stage: build
  needs:
    - job: release-check
      optional: true
    - version
  rules:
    - if: $CI_PIPELINE_SOURCE != "schedule"
  script:
    - ./scripts/build.sh "$RELEASE_VERSION"

release-notes:
  stage: build
  rules:
    - if: $CI_COMMIT_TAG
  script:
    - git intake release notes > release-notes.md
  artifacts:
    paths: [release-notes.md]

# The last job: the release is published once the forge release exists.
release:
  stage: release
  image: registry.gitlab.com/gitlab-org/cli:<version>
  rules:
    - if: $CI_COMMIT_TAG
  needs: [release-check, build, release-notes]
  script:
    - echo "releasing $CI_COMMIT_TAG"
  release:
    tag_name: $CI_COMMIT_TAG
    description: release-notes.md
```

The tag pipeline needs only `CI_JOB_TOKEN` to read the forge; a branch pipeline gets a rehearsal version. `glab release create "$CI_COMMIT_TAG" --notes-file release-notes.md` can replace the `release` keyword. When `release-tag` fails on an unpublished release at the tip, rerun the failed jobs of its tag pipeline.

Under `semver` the same pipeline works unchanged; the build gets a plain `x.y.z` and can carry the upstream release as build metadata where its channel keeps it (npm drops it):

```yaml
build:
  script:
    - ./scripts/build.sh "$RELEASE_VERSION+$RELEASE_UPSTREAM"   # 1.2.0+app-0.11.0
```

A squashed merge request's title becomes its commit message, so check that titles are Conventional Commits.

On GitHub Actions, a scheduled workflow runs `git intake release tag` with a GitHub App installation token with *Contents: write*, passed to `actions/checkout` and as `GH_TOKEN` (a tag pushed with `GITHUB_TOKEN` starts no workflow). A workflow `on: push: tags: ["v*"]` with `fetch-depth: 0` then runs `git intake release check "$GITHUB_REF_NAME"`, `git intake release version --tag "$GITHUB_REF_NAME" --dotenv >> "$GITHUB_ENV"`, the build, `git intake release notes "$GITHUB_REF_NAME" > notes.md` and `gh release create "$GITHUB_REF_NAME" --verify-tag --notes-file notes.md`, with `GH_TOKEN: ${{ github.token }}` (without it a private repository's releases read as hidden) and `permissions: contents: write`.

With `[release]`, `status` shows the release state as `release next` does.

## Commands

| Command | Purpose |
| --- | --- |
| `start [UPSTREAM[=REV]...]` | Starts an update (see *The update cycle*), taking a follow upstream at REV (see *Choosing a target*) or limiting a port plan to the commits REV contains. |
| `continue` | Carries out an edited plan, or concludes a merge, copy or catch-up merge stopped on conflicts, and takes in the rest; one concluded by hand with its trailers is accepted. |
| `abort [--merge-request]` | Removes the update; with `--merge-request` also withdraws its merge request (see *Abort*). |
| `verify [--only NAME]... \| [--skip NAME]...` | Runs `install` and the checks and records the verified commit; `--only` and `--skip` make a partial run (see *Running some of the checks*). |
| `finish` | Catches up when the integration branch moved, then in push mode verifies a catch-up and pushes the verified update, or in merge-request mode pushes `git-intake/update` and opens or rewrites the merge request; hands over an update an earlier release left waiting. |
| `sync` | Takes the follow upstreams in without local state (see *Unattended synchronisation*). |
| `land [--resume] [--timeout DURATION] [--poll SECONDS]` | Carries the update's merge request to the integration branch; `--resume` only ends a pause a killed run left (see *Landing an update*). |
| `env [COMMIT] [--dotenv \| --json] [-- COMMAND...]` | Prints the variables `verify` gives the checks for a commit, read from history, or runs a command with them (see *The update variables outside verify*). |
| `conflicts [--since COMMIT] [--path GLOB]... [--json]` | Lists the files that conflicted in the sync and catch-up merges on the integration line, recomputed from their parents (see *Conflicts resolved by updates*). |
| `delta [COMMIT]` | Shows, as a `git range-diff`, how the update a commit carries changed the downstream's delta over its upstream (see *What an update changed in the delta*). |
| `status [--check] [--json]` | Shows the configuration, every upstream's state, the merge request and the update in progress (see [status](#status)). |
| `report [--json]` | Summarises the update in progress, or the last one (see [report](#report)). |
| `log [UPSTREAM...] [--json]` | Lists, oldest first, a follow upstream's commits the integration branch lacks (up to the newest release with `tags`), or a port upstream's pending commits. |
| `branches [UPSTREAM...] [--json]` | Lists every upstream branch with commits the integration branch lacks, and how many; a port upstream's taken commits do not count. |
| `doctor [--json]` | Checks the repository (see [doctor](#doctor)). |
| `contribute UPSTREAM[=REV] BRANCH` | Creates a local branch from upstream history for a contribution (see *Contributing to an upstream*). |
| `release next\|tag\|check\|version\|notes` | Shows, cuts, checks, versions and describes the releases of the integration branch (see *Release commands*). |

`status --check` also predicts with `git merge-tree` the files each behind follow upstream's merge would conflict in, for its tip, newest release, or with `max_conflicts` the tag an update would take, and fails unless every upstream is current, pending port commits included. It creates nothing, so it can gate a cutover or a port review on a schedule.

### status

`status` shows the configuration, with each check's name and command, then each upstream's path, mode, branch, fetched tip and state. A follow upstream also shows its last synced commit and how many commits the integration branch lacks, in the state `current`, `behind`, `not imported`, `no release`, `path removed`, `rewritten` or `unavailable`; with `tags`, the tip and count are those of the newest release.

A port upstream shows its journal's base and its taken, decided and pending counts, in the state `current`, `pending`, `no base`, `rewritten` (naming the commits off its line) or `unavailable`.

`unavailable`: the upstream or its branch could not be fetched, its tags could not be listed, or a port journal is invalid; an unfetchable upstream makes `status` exit 1. `path removed`: a synced path upstream's path is gone from the integration branch, which `start` refuses and `status --check` counts as drift. `unavailable` beats `rewritten`, which beats `path removed`. A path upstream is `not imported` until it has a sync record or a merge of it brought in its path; a root upstream while it shares no history with the integration branch.

In merge-request mode it shows the open merge request with its number, address and head and whether the head contains the integration tip (naming `sync` when not), the newest one merged without its history, and a pause of the merge queue the remote records, read with git alone. When it cannot read the forge or the pause record, it says why and keeps its exit status.

With `[release]` it shows what `release next` shows, or why it cannot, keeping its exit status.

During an update it also shows the step, the base (and the start base after a catch-up), the worktree, each queued upstream's target or plan and step, a catch-up in conflict or awaiting verification, the checks, the verified commit, the partial run with its outcomes and the checks it left out, and the next command.

### report

`report` summarises the update in progress, or the last one, per upstream, measured from the base it started from. A follow upstream: the merged range and count, the target's tag, each conflicted file and how it was resolved (by hand or by a recorded resolution), and the size of the repository's own changes against the upstream before and after. A port upstream: the taken commits in order with their conflicts, the paths their copies left out and the earlier pending commits that touch the same files; the commits recorded as present; the skip and have decisions with notes; and the count left pending. Each catch-up merge follows with its tip and conflicts.

### doctor

`doctor` reports each check as pass, fail or warn, changing nothing but its own fetch references, which it removes:

- The publish remote's integration branch can be fetched. When it cannot, the checks that read the integration tip, the configuration, the upstreams, the history and the forge, are skipped.
- The configuration at the integration tip is valid.
- The update in progress, if any, holds its references at the tips `start` fetched.
- No reference under `refs/git-intake/` is left without an update.
- Every upstream's URL can be fetched and its branch exists.
- Every imported path exists on the integration branch.
- Every follow upstream's last synced commit is still contained in its branch.
- Every port upstream's journal is valid and its base and its decided and taken commits that are not retired are still on its branch, naming each one that is not.
- No git remote names an upstream's repository, with a note when the remote is the one a partial clone fetches missing objects from.
- No journal belongs to an upstream that is missing or in follow mode.
- In merge-request mode, the forge is recognised, a token is set and accepted, and the merge settings qualify, warning when GitHub leaves squash or rebase merges selectable and noting when GitLab leaves squashing optional; with `queue`, the account exists.
- In merge-request mode, the publish remote records no pause of the merge queue, with a warning that names one it does, and a failure when its record cannot be read.
- The update state, if any, matches its worktree and branch, and without one, no interrupted update left the update branch or its worktree behind.
- No upstream with sync records was dropped from the configuration (a warning).

### JSON output

`status`, `report`, `log`, `branches`, `doctor`, `env`, `conflicts`, `release next` and `release version` accept `--json`: one JSON document on standard output with the same facts under stable snake_case names, and the same exit status. URLs carry no password, nor the user of an `http` or `https` URL, while an SSH URL keeps its user; no message shows a token the tool reads, even when a forge quotes it. Notes and errors go to standard error. `env --json` prints one object of the variables, `{}` without an update.

#### status --json

The document carries `integration` (the integration branch), `integration_tip` (the commit the publish remote's integration branch is at, as `status` fetched it), `publish` (the publish remote), `publish_via` (`push` or `merge_request`), `replace_after` (the setting as the configuration writes it, or null), `install` (the install command or null) and `checks`, the checks of the configuration at the integration tip, each with `name` and `run`. In merge-request mode it carries `merge_request`, the update's open merge request with `number`, `url`, `head` and `current` (whether the head contains the integration tip), `merged_without_history`, the newest merge request of the update merged without its history with `number` and `url`, each or null, `pause`, a pause of the merge queue the publish remote records, with the `number` and `url` of the merge request it was started for, the queue's `account`, the held `merge_requests` in their order and those `in_flight` when it began, or null, `pause_error`, why its record could not be read, or null, and `forge_error`, why the forge could not be read, or null. An update carried on from a merge request carries `carried_from` with `number`, `url` and `source_branch` in its `update`; an update an earlier release left waiting carries `step` `waiting_for_merge_request` and `merge_request` with `number`, `remote`, `source_branch`, `target_branch` and `commit`.

**Upstreams.** Every entry of `upstreams` carries `name`, `path`, `url`, `branch`, `mode` (`follow` or `port`), `tip`, `state` (`current`, `behind`, `pending`, `no_base`, `not_imported`, `no_release`, `path_removed`, `rewritten`, `unavailable`) and `detail`. A follow upstream fills `last_synced`, `last_synced_source` (`record`, `history` or null), `last_synced_tag` (the `Intake-Tag` of the newest sync record, or null), `tags` (the configured pattern or null) and `commits_behind`, with `tags` also `release`, `release_tag` and `commits_after_release`, and with `--check` `conflicts`, and, with `max_conflicts` when it is behind, `next_tag`, `next_target` and `next_within_limit`; a port upstream fills `base`, `journal` (whether the journal exists), `taken`, `decided` and `pending`, and when rewritten `off_line`, with `base` (the base when it is off the line, or null) and `commits` (each with `commit` and `recorded_as`: `taken`, `skip` or `have`). Fields that do not apply are null.

**The update in progress.** Each queued upstream under `update` carries `name`, `mode`, `fetched`, `rev`, `step` (`plan`, `pending`, `conflict`, `merged`) and `conflicts`; a follow upstream adds `target`, `tag` (the upstream tag that names the target, or null), `kind` and `merge_commit`, a port upstream `plan` (the plan file), `commits` (each with `commit`, `subject`, `verb`, `note`, `copy` — `none`, `conflict`, `commit_failed`, `copied` or `empty` — `copied`, `failure`, git's output when the commit of the copy failed, and `left_out`) and `journal_commit`.

The update's `step` is `plan` while a plan waits to be edited, `commit_failed` while a copy whose commit failed waits for `continue`, `catch_up_conflict` while a catch-up merge waits for its conflicts to be resolved and `continue`, and `catch_up` while a catch-up merge awaits its verification; in both, `catch_up` gives the catch-up's `tip`, `head_before`, `merge_commit` and `conflicts`, and is null otherwise.

`base` is the integration tip the update lands on and `start_base` the one it started from. `checks` lists the names of the update's checks, `verified_commit` is the commit a full run verified, or null, and `partial_run` is the update's partial run, or null: its `commit`, `install`, the install command's outcome or null when no install command is configured, `checks`, each check it ran with `name` and `outcome`, `left_out`, the names of the checks it left out, and `passed`. An outcome is `passed`, `failed`, or `skipped` when the install command failed; a check left out appears only in `left_out`.

With `[release]`, `status --json` carries `release`, the object `release next --json` writes, and `release_error`, why the release state could not be read, or null; without it both are null.

#### release next --json

One object: `tip`, the publish remote's integration tip; `versioning`, `dated` or `semver`; `base`, its release base under `dated`, or null under `semver`; `upstream_release`, the tag of the newest upstream release the tip contains; `release`, the release at the tip, or null; `published`, whether that release is published, or null without one or when the forge cannot be read; `bump`, under `semver` the change that sets the next release, `major`, `minor` or `patch`, or null under `dated`, with a release at the tip, or when no release is due; `next`, the release a tag cut now would take, or null when the tip carries one or no release is due; `unpublished_previous`, when the tip carries no release, the newest release below it if it is not published, or null; and `forge_error`, why the forge could not be read, or null.

#### report --json, log --json and branches --json

`report --json` carries `base`, the integration tip the update started from or was carried on at, `carried_from`, the address of the merge request an update was carried on from, or null, and `catch_ups`, each with `tip`, `merge_commit` and `conflicts`. Each entry of `report --json` `upstreams` carries `mode` and `rev`; a follow upstream fills `kind`, `range_start`, `target`, `tag`, `commits`, `merge_commit`, `conflicts` and the diff sizes, and a port upstream fills `port` with `taken` (each with `commit`, `subject`, `copy`, `copied`, `conflicts`, `left_out` and `earlier_pending`), `unchanged` (each with `commit`, `subject` and `left_out`), `decisions` (each with `commit`, `subject`, `verdict` and `note`), `left_pending` and `journal_commit`. Each entry of `log --json` and `branches --json` carries `mode`; a follow upstream's entry of `log --json` adds `release` and `release_tag`, null without `tags`.

## Exit status

| Status | Meaning |
| --- | --- |
| 0 | Success. |
| 1 | A negative result: `status --check` found drift or pending port commits; `status`, `log` or `branches` could not fetch an upstream; a `doctor` check failed; `verify` failed, partial runs included; `start` or `continue` stopped on conflicts or a copy's failed commit; `finish` stopped on a catch-up merge's conflicts or failed checks, or because the integration branch moved again while it made or verified one; `sync` stopped on a conflict or failed checks, found a replacement conflicting, or left out a follow upstream; `land` met a failed or canceled pipeline or a conflicted catch-up merge; `release tag` found an unpublished release at the tip; `release check` found a problem. An open merge request is not a negative result. |
| 2 | An error: invalid configuration or usage; a refused command (such as `start` with an unfetchable upstream, `continue` with an invalid plan, `verify` with an unknown check, `finish` on an unverified update in push mode or a head with only a partial run, a forge whose merge settings do not qualify or no token, `sync` on a rewritten merge request or a declined branch, `sync` in merge-request mode, `release tag` or `land` pushing with the job token, any command in a shallow clone); a forge that rejected the token or a request; a push of the update's branch or a release tag rejected because another run pushed first; a release that breaks the order or whose name exists; a forge whose releases `release tag` or `release check` cannot read, or that hides them from `release next`; a catch-up refused because the integration branch was rewritten or changes what the update means; `delta` on a commit it cannot compare; `land` refused, refused a merge, stopped by another run's pause, or given up after its timeout or failed reads; output that cannot be written; a git failure, an unfetchable publish remote and a rejected push included (run `finish` again after it). |
| 129, 130, 143 | `land` stopped by SIGHUP, SIGINT or SIGTERM, with its pause ended. |

`env -- <command>` exits with the command's status, or with status 2 without running it when it cannot read the variables.

## Running in CI

A scheduled job runs `git intake sync`; the merge request's pipeline runs the checks with the variables of `git intake env`. Every command refuses a shallow clone, whose cut history would hide sync records, so clone the full history. The job pushes with a credential whose pushes start the forge's pipeline, has a token that can open merge requests (see *Tokens*), and sets git's `user.name` and `user.email`. For releases see *Releasing from CI*.

### GitLab

```yaml
variables:
  GIT_DEPTH: "0"        # the full history; git intake refuses a shallow clone
  # The checkout, for an image that runs as another user than cloned it.
  GIT_CONFIG_COUNT: "1"
  GIT_CONFIG_KEY_0: safe.directory
  GIT_CONFIG_VALUE_0: $CI_PROJECT_DIR

# Pushes with the access token in GITLAB_TOKEN instead of the job token, whose
# pushes start no pipeline.
.push-with-token:
  script:
    # Drop the runner's git configuration, which carries the job token.
    - export GIT_CONFIG_GLOBAL="$(mktemp)"
    - git remote set-url origin "${CI_PROJECT_URL}.git"
    - git config --local --unset-all include.path || true
    - git config --unset-all credential.helper || true
    - git config user.name "Sakiori"
    - git config user.email "oss@sakiori.app"
    # Answer the server, and no other host, with the access token.
    - git config --add "credential.${CI_SERVER_URL}.helper" ''
    - git config --add "credential.${CI_SERVER_URL}.helper" '!f() { echo username=oauth2; echo "password=${GITLAB_TOKEN}"; }; f'
    - |
      push_url="$(git remote get-url --push origin)"
      case "$push_url" in
        *@*) echo "git rewrites the push URL to carry credentials; it must reach the server without them" >&2; exit 1 ;;
      esac

git-intake-sync:
  rules:
    - if: $CI_PIPELINE_SOURCE == "schedule" && $CI_COMMIT_BRANCH == $CI_DEFAULT_BRANCH
  script:
    - !reference [.push-with-token, script]
    - git intake sync

upstream-checks:
  rules:
    - if: $CI_PIPELINE_SOURCE == "merge_request_event"
  script:
    - git intake env -- ./scripts/check.sh
```

Each part of `.push-with-token` is needed:

- The runner's configuration, which the clone includes, either rewrites the server's URLs to carry the job token or, with `FF_GIT_URLS_WITHOUT_TOKENS`, answers with it through a helper. The job drops that configuration, any global configuration, and an unbound helper left in a reused checkout.
- The first, empty helper clears any system helper for the server.
- The helper is bound to the server's URL, or it would answer every host, a vanished upstream answering 401 included.
- The guard fails the job at once when something still rewrites the URL, or git cannot read it.
- `git remote set-url` comes before the `|| true` lines, so a refused checkout stops the job with git's message.

The `GIT_CONFIG_*` variables name the checkout in `safe.directory` for every job, for an image that runs as another user than cloned it; git reads that setting only outside the clone's configuration. Where `CI_PROJECT_DIR` is a symbolic link, an older git needs the path it leads to.

Behind a private certificate authority, which the dropped runner configuration may have named, add `export GIT_SSL_CAINFO="$CI_SERVER_TLS_CA_FILE"`.

In a GitLab CI job, `sync` in merge-request mode, `release tag` and `land` refuse with status 2, at the start of every run, a clone that would push with the job token, whose pushes start no pipeline; `sync` in push mode and `land --resume` only warn.

Store `GITLAB_TOKEN` (see *Tokens*) as a **masked and protected** CI/CD variable. It reaches the scheduled pipeline on the protected default branch, and must not reach the merge request pipeline that runs an update's code:

- Keep `git-intake/update` unprotected: no protected-branch rule may match it, wildcards such as `*` or `git-intake/*` included.
- Keep **Settings > CI/CD > Variables > Allow merge request pipelines to access protected variables and runners** off ([Control access to protected variables and runners](https://docs.gitlab.com/ci/pipelines/merge_request_pipelines/#control-access-to-protected-variables-and-runners)).

See also [protected CI/CD variables](https://docs.gitlab.com/ci/variables/#protect-a-cicd-variable). In a job of the remote's instance, the forge and API come from `CI_SERVER_URL` and `CI_API_V4_URL`, under a relative URL root too.

`upstream-checks` gets no variable outside an update, so a check falls back to its usual baseline. To hand the variables to later jobs, write a dotenv report:

```yaml
intake-env:
  rules:
    - if: $CI_PIPELINE_SOURCE == "merge_request_event"
  script:
    - git intake env --dotenv > intake.env
  artifacts:
    reports:
      dotenv: intake.env
```

### GitHub Actions

```yaml
# .github/workflows/git-intake-sync.yml
name: git intake sync
on:
  schedule:
    - cron: "17 4 * * *"
  workflow_dispatch:
permissions:
  contents: read
jobs:
  sync:
    runs-on: ubuntu-latest
    # The environment holds the App's client id (variable INTAKE_APP_CLIENT_ID)
    # and private key (secret INTAKE_APP_PRIVATE_KEY); its deployment branch
    # policy allows only the integration branch.
    environment: git-intake
    steps:
      - id: app
        uses: actions/create-github-app-token@v3
        with:
          client-id: ${{ vars.INTAKE_APP_CLIENT_ID }}
          private-key: ${{ secrets.INTAKE_APP_PRIVATE_KEY }}
          permission-contents: write
          permission-pull-requests: write
      - uses: actions/checkout@v7
        with:
          fetch-depth: 0          # the full history; git intake refuses a shallow clone
          token: ${{ steps.app.outputs.token }}
      - run: |
          git config user.name "Sakiori"
          git config user.email "oss@sakiori.app"
      - run: git intake sync
        env:
          GH_TOKEN: ${{ steps.app.outputs.token }}
```

```yaml
# .github/workflows/upstream-checks.yml
name: upstream checks
on:
  pull_request:
permissions:
  contents: read          # enough for env to fetch the publish remote
jobs:
  checks:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v7
        with:
          fetch-depth: 0      # keep the checkout's credential: env fetches the integration branch
      - run: git intake env -- ./scripts/check.sh
```

`env` fetches the integration branch, so in a private repository it needs the read-only credential `actions/checkout` leaves.

The scheduled job pushes and opens the pull request with a GitHub App installation token or a fine-grained personal access token (see *Tokens*). Keep the App's private key, or the token, as a secret of an [environment](https://docs.github.com/en/actions/how-tos/deploy/configure-and-manage-deployments/manage-environments) (`git-intake` above) whose deployment branches are only the integration branch, never as a repository secret, which a `pull_request` workflow of the same repository receives. With a personal access token, pass it to `actions/checkout` as `token` and to `sync` as `GH_TOKEN`, and drop the App step. Not `GITHUB_TOKEN` (see *Tokens*). A `pull_request` workflow checks out the merge into the base, so `env` gives the base's tip as `GIT_INTAKE_BASE`. To hand the variables to later steps: `git intake env --dotenv >> "$GITHUB_ENV"`.

### Code from upstreams in CI

An update's checks run upstream code: in `verify`, in `sync`'s push mode, or in the merge request's pipeline, which a scheduled `sync` triggers before anyone looked at the update. A merge request pipeline of a branch in the same repository gets the non-protected variables (GitLab) or secrets (GitHub), and an upstream that changes the CI definition changes the pipeline that tests it. In push mode the job that runs the checks also holds its push credential.

The tool does not hide its token variables from the checks, which would not protect the push credential. It makes the exposure visible: when an update changes the forge's CI definition or the install command or checks, `finish` and `sync` name it in their output and the merge request's description, before they run the checks themselves and before the push; `verify` run by hand names nothing. In push mode the CI definition counts when the forge is known without its API (from `forge`, the host or the CI job): `.github/workflows/` on GitHub, only the default `.gitlab-ci.yml` on GitLab. Contain the rest:

- Treat upstreams as code the repository chooses to run.
- Keep deploy secrets in protected variables (GitLab) or in environments with protection rules (GitHub) that an update's branch cannot reach, and keep the token `sync` uses there too.
- On GitHub, a token without *Workflows* cannot push an update that changes `.github/workflows`; such an update stops for a person to publish with `start` and `finish`.
- In push mode, give the job only the credential it pushes with.

## Monorepo migration

1. Commit a configuration that lists one `[[upstream]]` per source repository, each with its own `path` and `url`.
2. Run `git intake start`: each source is imported under its path with its full history and finished like any update. A path from `git subtree add` without `--squash` is adopted; a squashed import, unrelated content, or an import that would replace a file is refused.
3. Until cutover, keep taking the sources in. Conflicts with the new layout are resolved once and recorded; files moved within a path merge as renames; files moved out of a path may conflict or arrive at their old location.
4. Run `git intake status --check` on a schedule; zero drift is the cutover gate.
5. Before freezing the sources, `git intake branches` finds work left on their other branches, and says when no upstream holds any.
6. After cutover, remove the upstream from the configuration; `doctor` keeps warning while it has sync records.

## Safety properties

- The tool never rewrites history and never force-pushes. The integration branch only fast-forwards, and so does the update branch, through catch-up merges.
- The integration branch is read only from the publish remote. No local integration branch is read, created or moved; the only local branches the tool creates are the update branch and the branch `contribute` names.
- Updates never change the repository's tags. The release commands create release tags and never move, replace or delete a tag, locally or on the remote: `release tag` pushes only the tag it creates, and only where the remote lacks it.
- It never pushes to an upstream: the publish remote may not name an upstream's repository, and no remote is kept for an upstream.
- No file of the main checkout is changed. The tool removes only the update worktree git registers at its place, reached through directories alone, and leaves anything else there. The update lives in its own worktree, `.worktrees/git-intake/update` in the main worktree (in the git directory of a bare repository), on `git-intake/update`, with its state and plan in `<git-common-dir>/git-intake/`, which every worktree shares and no commit can carry.
- Each target is fixed at `start`, and the update holds references to its targets and planned commits until it ends. What is merged or copied, verified and published is the same commit. A replacement with `replace_after` never changes an open merge request's targets: it closes it unmerged and opens a new one.
- In push mode, `finish` pushes exactly the verified commit, never what a local branch points at, and only when it contains the integration tip just fetched, so the push is a fast-forward; a moved tip is first taken in by a catch-up merge, verified before it is pushed, and a rewritten one is refused. A rejected push publishes nothing, and a retry never publishes anything unverified.
- In push mode, only a commit `verify` passed reaches the integration branch. In merge-request mode, only a tree the forge's required pipeline tested can be merged, by anyone the forge's rules bind: the tool publishes only to a forge whose settings guarantee it, read before every push. The update's branch only moves forward, and a branch the tool did not leave is never deleted or overwritten; deletions go under a lease on the commit read.
- The forge's token is read from the environment only, sent only to the API of the publish remote's own host, and never printed or stored; the publication setting is read from the integration tip, never from an update's head. The release commands only read the forge.
- The tool never adds or changes a git remote, and never fetches an upstream through one.
- A rewritten upstream is detected from the sync record or the port journal and copies, and is not taken.
- The configuration and a port's journal are read at the publish remote's integration tip, never from a local branch, a working copy or the tree being taken in.

## License

git-intake is released under the MIT license; see [LICENSE](LICENSE).
