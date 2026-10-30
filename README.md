# git-intake

`git intake` keeps a downstream repository current with one or more upstreams. It takes upstream changes by merge commits or by copying chosen upstream commits, never rewrites history and never force-pushes. The work happens in a separate worktree. A conflict is resolved once, and the repository's own checks run on exactly the tree that gets published: `verify` runs them before an update is pushed, and a merge request's pipeline runs them on an update published for review, on a forge whose settings merge only a tree that pipeline tested. The integration branch moves only by fast-forward. `git intake sync` carries the whole cycle out unattended, as a scheduled CI job.

It covers three cases:

- **A fork**: one upstream followed at the repository root, merged as a whole. A branch stacked on another fork's integration branch is a fork too, with that branch as its upstream.
- **A monorepo assembled from several repositories**: each source repository is an upstream merged under its own path. They keep being taken in until cutover, and `status --check` gates the cutover.
- **A product built on an open-source project**: the product owns its history and takes only the upstream commits it wants. Each update lists the upstream commits nobody has decided on yet; the product takes some, skips others, and records which ones it already has.

The tool runs the installed `git` for every repository operation. Each repository describes itself in a committed `.git-intake.toml`. Upstreams are fetched by their URL; the repository keeps no remote, remote-tracking branch or other branch of an upstream.

## Installation

Requires Git 2.43 or later (`git merge-tree -X` is used for conflict prediction) and a POSIX `sh`. The tool is a single binary named `git-intake`; with it on the `PATH`, git runs it as `git intake`.

Releases provide prebuilt binaries for the targets `x86_64-unknown-linux-musl` and `aarch64-unknown-linux-musl`. They are statically linked and run on any Linux distribution and in minimal CI images. On macOS, install from source (below): the tool verifies the forge's TLS certificate against the keychain through the system's Security framework, so its macOS binary is built on macOS.

### With mise

Pin the version in the repository's tool configuration, so that an update of the tool is a reviewed change:

```sh
mise use gitlab:sakiori-oss/git-intake@0.1.0
```

mise's GitLab backend picks the release asset for the running platform by its target triple and finds the binary at the archive root without further options. The manual pages come with it; *Help and the manual page* below says how `man` finds them.

### Direct download

Each release lists its archives, `git-intake-vX.Y.Z-<target>.tar.gz`, with a `.sha256` checksum next to each. The binary sits at the archive root, and the manual pages under `man/man1/`:

```
git-intake
man/man1/git-intake.1
man/man1/git-intake-<command>.1
```

To download an archive and its checksum directly:

```sh
tag=vX.Y.Z  # the release to install
target=x86_64-unknown-linux-musl
file="git-intake-$tag-$target.tar.gz"
base="https://gitlab.com/sakiori-oss/git-intake/-/releases/$tag/downloads/binaries"
curl --fail --location --remote-name "$base/$file"
curl --fail --location --remote-name "$base/$file.sha256"
sha256sum -c "$file.sha256"
tar -xzf "$file" -C /usr/local/bin git-intake
```

To install the manual pages as well, extract them into a manual directory, for example `tar -xzf "$file" -C /usr/local/share man`.

### From source

Contributors with a Rust toolchain can build any revision:

```sh
cargo install --locked --git https://gitlab.com/sakiori-oss/git-intake.git --tag v0.1.0
```

From a checkout, `cargo install --locked --path .` installs the working tree. `cargo run --locked -p xtask -- man <dir>` writes the manual pages into `<dir>`. A build from source reports the version `0.0.0`: only a release build carries its tag's version.

### Help and the manual page

`git intake -h`, and `-h` or `--help` after a command (`git intake finish --help`), print the built-in help. `git intake --help` is different: git turns it into `git help intake`, which opens the manual page `git-intake(1)`, so it works only where `man` finds that page.

Every release archive carries the manual pages of that release under `man/man1/`, and mise installs them with the binary. `man` finds them when that `man` directory is on the manual search path. Unless `MANPATH` is set, both man-db and the BSD `man` derive the search path from `PATH`: for every directory on `PATH` they also search its `man` subdirectory. The pages are therefore found when the directory holding `git-intake` itself is on `PATH`, as with `mise activate`, and they then belong to the binary that runs. They are not found through mise's shims, whose directory holds no pages, nor when `MANPATH` is set without an empty component (a leading or trailing `:`, or `::`), which replaces that search.

When the current pages are not on the search path, `man` opens any other `git-intake` pages it finds there, such as pages extracted from an earlier release's archive, and they describe that release. `man -w git-intake-verify` names the file `man` opens, and the footer of every page names its release. Remove such leftover pages, or put the current pages first:

```sh
export MANPATH="$(mise where gitlab:sakiori-oss/git-intake)/man:$MANPATH"
```

`mise where` answers for the directory it runs in, so in a shell startup file this line names the version mise uses there, not the version a repository pins. `git intake <command> -h` always describes the binary that runs.

After a direct download, extracting the pages into `/usr/local/share` puts them on the default search path; extract the pages of every new release there as well, or the old pages stay.

## Releasing

A release is the pipeline of a protected tag `vX.Y.Z` on a commit of `main`; nothing else publishes. The tag, the release guard, the version a pipeline builds and the GitLab Release come from shared release components, which `.gitlab-ci.yml` includes at a pinned commit.

- A daily pipeline schedule on `main` cuts the tag. Its pipeline holds only the tagging job, which tags the tip of `main` with the next version unless a release tag already points at it. git-cliff computes the version from the Conventional Commits since the last release: in `0.x` a breaking change raises the minor version and a feature, fix, performance change or revert the patch version; other commits, such as documentation, ask for no release. Nobody picks a version or pushes a release tag by hand.
- An off-cycle release is the same schedule run by hand; it cuts the next version by the same rules.
- The tag's pipeline starts with the guard, which every build waits for. It refuses a tag that is malformed, does not point at a commit of `main`, does not sort after every other release, or whose release is published completely. After it the `release-version` job hands the tag's version to the builds, which stamp it into `Cargo.toml` and `Cargo.lock` in the job's working tree only, so the binaries and their manual pages report it; the committed version stays `0.0.0`. The publish job uploads the archives and their `.sha256` files to the generic package registry, package `git-intake`, version `vX.Y.Z`, and last a `SHA256SUMS` over the archives, which marks the release complete. The GitLab Release follows, with notes git-cliff writes from the commits since the previous release and links to the archives.
- A failed release is finished, not cut again: retry its failed jobs, or run its tag's pipeline again while nothing of it is published completely and no newer release exists; after that the guard refuses a new pipeline, so only the failed jobs are retried. The schedule cuts no second tag for a commit that already has one. Rollback is forward, by a revert and an off-cycle release.
- A pipeline started by hand on `main` (`glab ci run --branch main`) is a rehearsal: it builds every target as a release does and publishes nothing. Its builds carry the version `<x.y.z>-rehearsal.<YYYYMMDD>`: the version the next release would take, or the latest release's when no commit since asks for one, and the pipeline's UTC day. A prerelease, it is a version no release tag can name. Rehearse after a change to the build before the next scheduled release; merge request pipelines also build when a change can affect the build.

## Configuration

`.git-intake.toml` lives at the repository root and is committed on the integration branch. The tool always reads the version committed at the integration branch tip. An uncommitted edit in the working copy is ignored, and the tool prints a note that the working copy differs. The tool finds the integration branch from the configuration committed at `HEAD`, or from the update in progress, so run it from a checkout of the integration branch or of a branch that carries the configuration.

```toml
integration = "main"                  # the branch that receives the upstream changes
install = "pnpm install"              # optional: runs before the checks
checks = [                            # run in order by verify, each named
  { name = "lint", run = "pnpm lint" },
  { name = "test", run = "pnpm test" },
]

[publish]                             # optional: where and how finish publishes
remote = "origin"                     # the publish remote, a git remote of the repository
via = "push"                          # optional: "push" (default) or "merge-request"
# forge = "gitlab"                    # optional, merge-request mode: "gitlab" or "github", when the URL does not tell
# api = "https://gitlab.example.com/api/v4"  # optional, merge-request mode: the forge's REST API on the remote's host

[[upstream]]
name = "app-a"                        # unique; used on the command line, in sync records, trailers and check variables
url = "git@example.com:team/app-a.git"  # required: where the upstream is fetched from
branch = "main"                       # the upstream branch to take from
mode = "follow"                       # optional: "follow" (default) or "port"
path = "apps/app-a"                   # optional, follow mode only: merge under this path; absent = repository root
tags = "v*"                           # optional, follow mode only: take the newest release tag matching this pattern instead of the branch tip
```

Validation rules. The tool refuses to run when any of them is broken, and it names the entries involved:

- Unknown keys are rejected. The settings `remote` and `mirror` of earlier releases are refused with what replaces them: upstreams are fetched by their `url`, and mirror branches are no longer kept.
- At least one `[[upstream]]` is required, and every upstream needs a `url`.
- Upstream names are unique, and no two upstreams have the same URL and branch. A name may contain only letters, digits, `-`, `_` and `.`, may not start with `-` or `.`, and, since it is part of a git reference name, may not contain `..` or end with `.` or `.lock`. Since a name is also part of the variables the checks receive, no two names may differ only in letter case or in which of `-`, `_` and `.` they use (see [The environment of the checks](#the-environment-of-the-checks)).
- A path is normalised: no leading or trailing slash, no empty, `.` or `..` segment, no `.git` segment in any letter case, and it may not be empty. Omit `path` for the root.
- Paths may not nest; one upstream's path cannot lie inside another's.
- An upstream at the root must be the only upstream.
- An upstream in `port` mode is taken at the repository root and may not have a path.
- `tags` belongs to an upstream in `follow` mode. It is a glob over tag names (`*`, `?` and `[...]`, matched against the whole name), written without `refs/tags/`, such as `v*` or `release-[0-9]*`.
- Each check is a table with exactly the keys `name` and `run`, the command, which may not be empty. A check's name may contain only ASCII letters, digits, `-`, `_` and `.`, and may not start with `-` or `.`; no two checks may have names that differ only in letter case. The name selects the check for `verify --only` and `--skip`, names it in the output and the merge request's description, and names its log. The list of plain command strings of earlier releases, `checks = ["pnpm lint"]`, is rejected with the table form to write instead.
- `publish` is a table: `remote` is required, `via` is `push` or `merge-request`, `forge` is `gitlab` or `github`, and `api` is an `https` URL with a host and without credentials, query or fragment (see *The forge*). Write it as the `[publish]` section shown above, after the top-level keys (a section header ends the top level, so keys written below it belong to the section), or inline as `publish = { remote = "origin", via = "merge-request" }`. The string form `publish = "origin"` is rejected with the form to write instead.
- The publish remote may not be fetched from or pushed to (its `url` or any `pushurl`) a URL that names an upstream's repository, because the tool never pushes to an upstream. Two URLs name the same repository when a local path or `file://` URL resolves, against the main worktree and the way git finds a repository (`<path>/.git`, `<path>`, then `<path>.git`), to the same directory, so `../up`, `../up.git/` and the absolute path of that directory match; or when any other URL matches once the scp-like form `user@host:path` is read as `ssh://user@host/path`, scheme and host are put in lower case, a default port such as `:22` or `:443` is dropped, and a leading `/` and a trailing `/` and `.git` of the path are dropped. `doctor` compares the URLs of every remote with the upstreams' URLs the same way.

### Upstreams by URL

An upstream is fetched with `git fetch --no-tags <url> +refs/heads/<branch>:<reference>`, in the main worktree so that a relative URL means what it means there, into a reference under `refs/git-intake/`. That lives outside `refs/heads` and `refs/remotes`, so no branch listing, checkout, `git push --all` or hosting tool sees it, and no remote exists for a tool to take as the place to open pull requests in.

The references exist only while something needs them. Every command that fetches, `status`, `log`, `branches`, `doctor`, `contribute`, `start` and `sync`, fetches into a namespace of its own, `refs/git-intake/read/<token>/`, and removes it before it exits, whatever its outcome; every run contacts the upstreams. `branches` fetches every branch of an upstream there. An update holds `refs/git-intake/upstreams/<name>` for each queued upstream, at the tip `start` fetched, so that its targets and planned commits stay in the repository even if the upstream moves or becomes unreachable; `continue`, `verify` and `finish` never fetch an upstream. Only `start` and `sync` write those references, and only `finish`, `abort` and `sync` remove them; an update `sync` carries on from a merge request holds each follow upstream's reference at its target: a command that reads an upstream during the update fetches into its own namespace and never touches them. `start` saves the update before it writes them; if it is killed in between, `status`, `continue`, `verify` and `finish` restore the missing references from the update state, at the tips `start` fetched, and say so. `doctor` reports an update reference that is missing or elsewhere, and leaves alone, with a warning to keep it, a reference that still holds the commit of a missing one. It reports any other reference under `refs/git-intake/` that no update accounts for, such as the namespace of a command that was killed; nothing uses it, and `git update-ref -d <reference>` removes it.

### Modes

Each upstream has a mode, which describes the relationship to that source:

- `follow`, the default: an update merges the upstream branch up to a target, at the repository root or under the upstream's path. A fork that carries a few patches follows its upstream. With `tags`, the target is the upstream's newest release tag (see *Release tags*).
- `port`: an update takes chosen upstream commits, one by one, at the repository root. A product built on an upstream ports from it: it decides for every upstream commit whether to take it, skip it, or record that it already has it.

Switching an upstream from `follow` to `port` changes one setting. The first port update starts the journal at the last synced commit (see *Port mode*). Switching back to `follow` is allowed: the next update merges the upstream up to its target, git finds the merge base from the last merge, and changes taken in port mode come back as the same changes, which usually merge cleanly or conflict where the product changed them. The journal of an upstream in follow mode means nothing; `doctor` reports it as stale.

## The update cycle

```sh
git intake start        # fetch, fix targets and merge each follow upstream, or plan a port
# port mode: edit the plan, then
git intake continue     # carry out the plan
# on conflict: resolve in the update worktree, `git add`, then
git intake continue     # conclude the merge or copy, take in the rest
git intake verify       # run install and checks on the result
git intake finish       # catch up if the integration branch moved, fast-forward, publish, clean up
# with via = "merge-request": verify is optional; finish pushes the update, opens its merge
# request and ends the local update; the merge request carries it from then on

git intake sync         # all of it, unattended, as a scheduled CI job runs it
```

Every command works the same from any directory of the repository: the main checkout, a linked worktree, or the update worktree itself. This holds for a submodule and for a repository whose git directory lies outside its checkout as well. Git operations on the repository as a whole, such as fetching, pushing and moving branches, run in the main worktree, so a relative URL means what it means there. A revision named to `start` or `contribute` is resolved in the worktree the command runs in. When `finish` or `abort` removes the update worktree it was run from, the command still completes and notes that the current directory is gone; change to another directory before the next command.

Git cannot locate the main worktree of a repository whose git directory lies outside it, without naming it, from a linked worktree. The tool then finds it in the update in progress, which `start` records. Without an update, run `start` in the main worktree; from elsewhere it refuses, `doctor` warns that it did not check the main checkout, and a relative URL is resolved against the git directory.

`start` refuses to begin if any of these hold:

- an update is already in progress;
- `.worktrees` or `.worktrees/git-intake` in the main worktree is a symbolic link or not a directory;
- the repository is bare and the path of its update worktree contains `/.git`, as when the bare repository is itself a directory named `.git`; run the tool in a clone with a checkout instead;
- the main checkout or a checkout of the integration branch has uncommitted changes to tracked files;
- the publish remote cannot be fetched, or the integration branch and its counterpart on the publish remote each hold commits the other lacks; `start` names how many on each side, and the local commits must be published or reconciled first;
- the repository is a shallow clone (every command but `abort` refuses one, naming `git fetch --unshallow`);
- in merge-request mode, the forge cannot be reached as `finish` needs it, for example without a token; the local integration branch holds commits the publish remote's lacks, which would ride along in the merge request; or the update's merge request into the integration branch is open, which `sync` carries on and `abort --merge-request` withdraws;
- a selected upstream is unknown, named more than once, cannot be fetched, or was rewritten;
- a revision named for an upstream is refused, as described under *Choosing a target* below, or the upstream's URL cannot be asked for its tags;
- an upstream with `tags`, named without a revision, has no release tag on its fetched branch (see *Release tags*);
- a path upstream that has no sync record yet finds its path holding content unrelated to the upstream's history, or finds a file where a directory of its path would go;
- a path upstream that was imported before finds its path removed on the integration branch;
- a port upstream has an invalid journal, or neither a journal nor a last synced commit to start one from.

When the integration branch is strictly behind its counterpart on the publish remote, `start` first brings it up to date by fast-forward and says so: a checkout of the branch moves with `git merge --ff-only`, refused when it would lose changes, and the branch otherwise with `git update-ref` against its old value. It then reads the configuration again at the new tip. When that configuration names another integration branch or publish remote, `start` refuses and keeps the fast-forward; run it again to start with the new configuration.

When `start` runs, it does the following:

1. Fetches the selected upstreams. With no names it selects all of them.
2. For a follow upstream, records the fetched tip, the commit a revision names (see *Choosing a target*), or with `tags` the newest release tag (see *Release tags*), as that upstream's **target**, together with the upstream tag that names it, if any. For every upstream it records the integration tip as the **base**. Targets never change during the update, even if the upstream or a tag moves on.
3. Queues, in configuration order, the follow upstreams whose target the base does not contain yet, and a port upstream that has pending commits. The others are reported as current. If nothing is queued, nothing is created and no reference is kept.
4. Creates the worktree `.worktrees/git-intake/update` in the main worktree on the branch `git-intake/update` from the base. It is never placed inside the git directory, because compilers, bundlers and watchers ignore paths that contain `/.git`. Unless that path is ignored already, `start` adds the line `/.worktrees/` to the repository's `info/exclude` once, so the main checkout does not show the update worktree; a committed `.gitignore` is never changed. Beyond the fast-forward of a checkout of the integration branch, by `start` to the publish remote and by `finish` to the update, the files of the main checkout are never touched. While an update is in progress, its tree sits under `.worktrees/` in the checkout, so a tool that crawls dot-directories of the main checkout may see it. A bare repository has no main worktree, so its update worktree is `<git-common-dir>/git-intake/worktree`.
5. Enables `rerere.enabled` for the repository. A resolution recorded in one attempt is reapplied when the same conflict recurs, in a merge or in a copy, and the tool reports such files as *resolved by a recorded resolution*. Review those files and stage them like any other resolution.
6. For a port upstream, writes the plan and stops (see *Port mode*). For follow upstreams, merges each queued target with its own merge commit:
   - a root upstream: `git merge --no-ff`;
   - a path upstream, first import (no sync record for it yet, and the path is absent): an `ours` merge followed by `git read-tree --prefix=<path>/`. This keeps the upstream's full history. History shared through a common ancestor, such as a template both repositories started from, does not count as imported. The import is all or nothing: if any step fails, the merge is undone and the upstream stays pending;
   - a path upstream, later updates (it has a sync record): `git merge --no-ff -X subtree=<path>`;
   - a path upstream imported by other means, such as `git subtree add` (no sync record, but a merge on the integration line, described below, brought the upstream's history into its path): the same subtree merge, which writes the first sync record. A path that holds content no such merge brought in is refused, even when it shares a common template with the upstream.

   Merges run with `--no-log` so that git configuration cannot push the trailers out of the last paragraph. Before an upstream counts as merged, the tool checks that the commit carries its sync record for the target, that the target is merged, and that a path upstream's path exists. If a hook rewrote the message, the tool stops and says so. Each step can be repeated: after an interruption, `continue` recognises a merge that is already committed and carries its record.

On a conflict the tool stops with exit status 1. It names the upstream, lists the conflicted files, and reports which upstreams are merged and which are pending. A file is labelled *resolved by a recorded resolution* only when git's rerere reports that it resolved it (including files `rerere.autoupdate` already staged); every other conflicted file waits for a resolution by hand. Resolve the files in the worktree, stage them, and run `continue`. If one upstream cannot be resolved now, `abort` and run `start` again with only the other upstreams named. Recorded resolutions make the repeated merges cheap.

`verify` runs `install` and then every check, in order, through `sh -c` in the update worktree, once every queued upstream is merged or its plan carried out. The update's checks are those of the configuration it recorded: the one at the integration tip at `start`, or at its last catch-up merge. Each command receives variables that describe the update (see [The environment of the checks](#the-environment-of-the-checks)), and the output names each check by its name. Each command's output goes to its own log in `<git-common-dir>/git-intake/logs/verify/`: `install.log` for the install command and `check-<name>.log` for each check, so a check named `install` has a log of its own. If `install` fails, the checks are skipped. Verification fails in any of these cases:

- the `.git-intake.toml` of the update branch is missing or invalid, or names a different integration branch (the commands are then skipped);
- a command fails or is skipped;
- the commands change tracked files;
- the update branch moves while the checks run.

When it passes, `verify` records the commit it verified.

### Running some of the checks

`verify --only <name>` runs only the named check, and `verify --skip <name>` every check but the named one; each can be repeated, and a name given twice counts once. The two cannot be combined. The install command always runs, the selected checks run in the order of the update's checks, and the preconditions and the environment are those of a full run, so a single failing check can be run again in the update worktree, with the update's variables, until it passes. A name that is not one of the update's checks is refused with exit status 2, naming the checks the update has, and nothing runs. A check that a catch-up merge brought is one of the update's checks from then on.

A run that leaves out a check is a **partial run**. It is recorded beside the verification, with its commit, the checks it ran and their outcomes, and its own logs in `<git-common-dir>/git-intake/logs/partial/`; the logs of the verification stay as they are. A partial run never verifies the update, and never changes or completes its verification: after it, the update is verified exactly when it was before. Partial runs do not add up either: `--only lint` and then `--only test` on the same head leave the update unverified. A later partial run replaces the earlier one and its logs; a full run, and a catch-up merge, drop it with its logs. A selection that leaves no check out, such as every check named with `--only`, is a full run, and one that skips every check runs only the install command.

After a partial run, `verify` exits as a full run with the same outcomes would, 0 when everything it ran passed and 1 otherwise, and says that this run did not verify the update: when a full run verified the head, that the update stays verified at that commit, and otherwise to run `verify` without `--only` or `--skip`. `status` shows the partial run apart from the verified commit, with the checks it ran, their outcomes and the checks it left out.

In push mode, and with `--no-publish`, `finish` checks the following before moving any ref (in merge-request mode see *Publication through a merge request*):

- every upstream is merged or its plan carried out;
- the worktree head is the verified commit, so any commit added after `verify` needs a new `verify`, or a catch-up merge the tool made and has not verified yet, which `finish` verifies itself by running every check; a head with only a partial run is refused, naming the checks the run left out and `verify`;
- the base is still the integration tip; if the integration branch moved, `finish` catches up with it first (see *Catching up with a moved integration branch*);
- the verified commit contains the base, every merged target, and every copy and journal commit of a port, so the integration branch only fast-forwards;
- every checkout of the integration branch has no uncommitted changes and no untracked file the fast-forward would overwrite.

In push mode, `finish` first fetches the publish remote, and refuses with git's reason when it cannot. A local integration branch strictly behind the remote's is brought up to date as `start` does; that is not a move of `finish`, so `abort` still works afterwards. When the two each hold commits the other lacks, `finish` refuses before moving anything: the local commits must be published or reconciled first. When the integration branch already contains the verified commit, because it was moved there or past it by other means, `finish` makes no catch-up, counts the move as done, pushes the verified commit unless the remote holds it already, and completes the update.

It then fast-forwards the integration branch to the verified commit. A checkout of that branch is updated with `git merge --ff-only`; otherwise the branch moves with `git update-ref` against its old value. In push mode (`via = "push"`, the default), `finish` pushes the integration branch to the publish remote without force. Finally it removes the worktree, the update branch, the plan, the update's upstream references, its logs and the state.

`finish` saves its planned move in the update state before it moves anything, and `status` shows the step as `finishing`. Running `finish` again after an interruption completes the move; a branch already at its planned value, or at a commit containing it, counts as done. It then publishes and cleans up. Once the branch has moved, `abort` refuses and points to `finish` and `finish --no-publish`.

The push sends exactly the planned commit (`<commit>:refs/heads/<branch>`), never whatever the local branch points to by then. A commit added to the integration branch while the update is finishing stays local.

If the push is rejected, nothing is published, the local integration branch stays advanced, and the update stays in the `finishing` step. Running `finish` again fetches the publish remote. When the remote's integration branch did not move past the planned commit, `finish` retries the same push, unless the remote no longer contains its tip as the update last took it: it was rewritten, and a push would publish again what it dropped, so `finish` refuses and names `finish --no-publish`. When it moved past the base, as when other work landed between the fetch and the push, `finish` catches up with the remote's branch from the finishing step: the catch-up merge contains the planned commit, to which the local branch already moved, so the local branch fast-forwards again to the verified catch-up merge, which is then pushed; nothing was published in between. If a commit was added to the local integration branch after `finish` moved it, `finish` makes no catch-up and names `finish --no-publish`. If the remote never accepts the push (for example a protected branch), run `finish --no-publish`. It closes the update locally without fetching. A verified catch-up made from the finishing step is landed locally instead of the planned commit. One that is not verified, or still in conflict, is dropped and the update closes at the planned commit: the resolutions made so far are recorded first, and the commits it drops are named, so that they can still be found until git prunes unreachable commits. It completes the local move with the same checks, removes the worktree, the update branch, the plan, the references, the logs and the state, and names the branch it left unpublished (also in `report`). Then merge the remote's work and publish the result through a new verified update or the repository's usual review. `--no-publish` also works on a verified update before any push attempt; it then catches up only with the local integration branch.

### Catching up with a moved integration branch

Other work often lands on the integration branch while an update waits for a conflict resolution, a long verification or a review. The update branch belongs to the tool, so instead of starting over, `finish`, or `sync` for an open merge request, takes the new integration tip in with a **catch-up merge**: a merge commit whose first parent is the new integration tip and whose second parent is the update branch head, marked with a trailer:

```
catch-up: merge main at 1a2b3c4 into the update

Intake-Catch-Up: main
```

Every resolution and fix already on the update branch stays exactly as it is, including edits made beyond the conflicting hunks while resolving and commits added on top of the merges; only real conflicts between the new integration commits and the update stop it. With the integration tip as the first parent, the first-parent line of the integration branch stays the line of commits the branch itself received, and the update arrives on the second-parent side, as when a merge request is merged with a merge commit. The base becomes the tip caught up with, and the update counts as unverified; upstream targets, last synced commits and port plans stay as they were. A second move of the integration branch adds a second catch-up merge on top, whose second parent is the first, so the update branch only ever moves forward.

The tool records the tip it catches up with in the update state, checks it out detached in the update worktree, merges the update head into it, and then moves the update branch from the head it recorded to the merge with `git update-ref` against that head, refusing and naming both commits when the branch moved meanwhile; the worktree is then on the update branch again. After an interruption, `status`, `continue` and `finish` recognise a catch-up merge in progress or already committed from the recorded tip, and never make a committed one again. A catch-up merge committed by hand while the worktree is detached, with or without commits made on top of it, is kept: the update branch moves to the worktree's commit and the worktree is on the branch again, and commits on top of the catch-up merge need `verify`. A catch-up given up by hand, with the merge aborted and the update branch checked out again in the update worktree, is no longer recorded once `continue` or `finish` sees it: a commit made there needs `verify`, and `finish` catches up afresh when the integration branch is still ahead.

A conflicted catch-up stops `finish` or `sync` with exit status 1 in the step `catch_up_conflict`, naming the conflicted files and which side is which: the integration branch is *ours*, the update *theirs*. Resolve the files, stage them, and run `continue`, which commits the catch-up merge with its trailer and names `finish`. In merge-request mode the next `finish` publishes the catch-up merge without running the checks, which the merge request's pipeline runs. In push mode it verifies the catch-up merge as `verify` does and publishes it; failing checks stop it as `verify` does, with the update unverified at the catch-up merge in the step `catch_up`, and a commit added on top of the catch-up merge needs `verify` as usual. After a catch-up, verification runs the install command and the checks of the configuration at the catch-up merge, so a check added on the integration branch meanwhile verifies the result. The catch-up merge drops a partial run with its logs, and a partial run on an unverified catch-up merge does not stop `finish` from verifying the merge itself. `finish` makes at most one catch-up merge per run, and concluding a catch-up merge left by an earlier run counts as that one: when the integration branch moves again meanwhile, it moves no branch to the update and says to run it again, which catches up once more. In merge-request mode, a remote tip that the base already contains is not a move, as when the publish remote lacks commits the local integration branch holds: the merge request carries them.

A catch-up takes the integration branch's own work in. In push mode the integration tip is the local branch, whose tip at `start` is the base, so a local branch rewound below the base counts as rewritten and is refused. Some commits would change what the update means rather than add to it, and a catch-up refuses them, merges nothing and names `abort`, after which `start` plans the update from the new history; once `finish` has moved the integration branch, it names `finish --no-publish` instead, and for `sync` it names the merge request, `abort --merge-request` with the warning that withdrawing declines its targets, and `start` and `finish` to publish them again:

- a publish remote's tip that moved forward from the tip the update last took but lacks commits of the local integration branch that the base holds and that were never published, as in merge-request mode with unpublished local commits: the refusal names those commits; publish them or reconcile the integration branch with the remote, then `start` again;
- any other tip that does not contain the base: the integration branch was rewritten;
- a commit that carries a sync record or a copy of an upstream the update takes in, as when another clone took that upstream in; a sync record counts only on a merge whose second parent is its target;
- a change of the journal of a port upstream the update takes;
- a change of `.git-intake.toml` other than its install command and checks, such as an upstream or the publication setting, which the update fixed at `start`.

`abort` works in a catch-up step as in any other, except once `finish` has moved the integration branch, where it refuses and names `finish` and `finish --no-publish`.

### The environment of the checks

A check that compares the tree with a baseline, such as a ratchet that may not grow or an audit of the changes since `merge-base origin/main HEAD`, would count every change an update brings in as the downstream's own. The upstream already checked its changes; what an update adds on top of them is the downstream's delta, so the baseline to compare an update with is the upstream's target. Comparing `GIT_INTAKE_HEAD` with that target measures everything the downstream carries on top of the upstream, not only what this update changed; a ratchet that should judge the update alone compares that difference with the one before the update, between `GIT_INTAKE_BASE` and `GIT_INTAKE_UPSTREAM_<NAME>_PREVIOUS`. `verify` runs `install` and every check with these variables, which name commits by their full ids:

| Variable | Value |
| --- | --- |
| `GIT_INTAKE_BASE` | The integration tip the update lands on: the tip it started from, or after a catch-up merge the tip it caught up with, so that a ratchet comparing it with `GIT_INTAKE_HEAD` measures the update and not the commits that landed meanwhile. A catch-up changes none of the upstream variables. |
| `GIT_INTAKE_HEAD` | The commit being verified, the head of the update branch. |
| `GIT_INTAKE_INTEGRATION` | The name of the integration branch. |
| `GIT_INTAKE_UPSTREAMS` | The names of the upstreams the update takes, separated by spaces, in the order they were merged or carried out. An upstream that was current at `start` is not among them. |
| `GIT_INTAKE_UPSTREAM_<NAME>_MODE` | `follow` or `port`. |
| `GIT_INTAKE_UPSTREAM_<NAME>_TARGET` | Follow mode: the target the update merges. Port mode: the newest upstream commit the update takes or decides on, which is what it moves the upstream to in the journal; commits the plan leaves `later` stay pending, and an update that takes and decides none has no target. A port target is not merged, so it is not an ancestor of `GIT_INTAKE_HEAD`. |
| `GIT_INTAKE_UPSTREAM_<NAME>_PREVIOUS` | Follow mode only: the last synced commit before the update; absent on a first import or when the history names none. A port upstream has no such commit; its journal names the commit the port started from. |

`<NAME>` is the upstream's name in upper case, with every character other than a letter or a digit replaced by `_`: `app-a` gives `GIT_INTAKE_UPSTREAM_APP_A_TARGET`. Two upstream names that give the same `<NAME>`, such as `app-a` and `app.a`, are refused when the configuration is read. Every commit named stays in the repository while `verify` runs: the base and the head are on the update branch, and an upstream's commits are in the history of `refs/git-intake/upstreams/<name>`, which the update holds. Variables with the `GIT_INTAKE_` prefix that `git intake` itself inherits are not passed on, so a check sees only those of the update it verifies; git commands the tool runs itself never see them.

The target is a baseline for the tree only for a follow upstream at the repository root. A path upstream's target has the upstream's own layout, with its files at its root rather than under the path, so a check that compares trees with it has to compare the path's subtree, or measure the upstream on its own.

A fork that follows one upstream at the root, named `upstream`, can give an audit the upstream's target as the baseline it takes from `FALLOW_AUDIT_BASE`:

```toml
checks = [
    { name = "test", run = "pnpm test" },
    { name = "audit", run = 'FALLOW_AUDIT_BASE="$GIT_INTAKE_UPSTREAM_UPSTREAM_TARGET" fallow audit' },
]
```

A script that the repository's own CI also runs can fall back to its usual baseline outside an update: `"${GIT_INTAKE_UPSTREAM_UPSTREAM_TARGET:-$(git merge-base origin/main HEAD)}"`. A merge request pipeline gets the same variables from `git intake env` (see *The update variables outside verify*).

### Choosing a target

By default an update takes each follow upstream at the tip it just fetched. To take an upstream at a reviewed point instead, such as a release tag, name it with a revision:

```sh
git intake start upstream=v2.3              # a fork: take the upstream's tag v2.3
git intake start app-a=1a2b3c4d app-b       # app-a at a commit, app-b at its fetched tip
```

The name ends at the first `=`, since an upstream name cannot contain one, and the rest is the revision. Upstreams named without a revision take their fetched tip, and naming an upstream twice is refused. The revision is resolved as follows:

- If the upstream's repository has a tag of that name, the tag's commit is the target, peeled through an annotated tag. A leading `refs/tags/` or `tags/` is dropped first, so `v2.3`, `tags/v2.3` and `refs/tags/v2.3` name the same tag. Looking the tag up contacts the upstream: after the fetch, `start` lists the upstream's tags with `git ls-remote --tags`, as it does for every follow upstream to name the target's tag in the sync record. No tag is fetched, so the repository's own tags never clash with the upstream's. If the URL cannot be asked, `start` refuses and names the upstream, its URL and git's error.
- Otherwise the revision is resolved in the repository, like any argument to `git rev-parse`: a full or abbreviated commit id, or an expression such as `1a2b3c4d~2`. `start` fetches before it resolves the revision, so commits of the fetched branch resolve even when the repository did not have them before. A revision that resolves through one of the repository's own tags, by itself or at the start of an expression such as `v2.3~1`, is refused: the repository's tags are not the upstream's, so a tag `v2.3` in the repository never stands in for an upstream tag `v2.3`, even when the upstream has none. The tool looks for that tag by name, so it is found even when a branch has the same name.

Only a plain tag name is looked up on the upstream. An expression is always resolved in the repository, where upstream tags never are, so an expression on a tag only the upstream has, such as `v2.3~1`, names no commit and is refused; name the commit, or an expression on the fetched branch, instead.

The commit, however it was named, must be on the fetched upstream branch: the tool takes from a branch, not from arbitrary commits. Only that branch is fetched, so a commit of another upstream branch is not in the repository and names no commit. With `L` the upstream's last synced commit, `start` then decides:

- a target older than `L` is refused, because an update never moves an upstream back;
- a target the integration branch already contains, such as `L` itself, is reported as current;
- a target that does not contain `L`, such as a commit of a side branch that the upstream merged after `L`, is refused, because the last synced commit only moves forward;
- otherwise the upstream is queued at that target.

The rest of the update does not change: the sync record names the target and an upstream tag that points at it, if any, and `status` and `report` show the upstream tag next to the target, or otherwise the revision unless it is the target's own commit id; `report` also names the revision. Afterwards `status` shows the upstream as `behind`, by the commits between the target and the fetched tip.

A revision overrides `tags`: `start <name>=<rev>` takes any commit of the fetched branch, released or not.

### Release tags

Some upstreams keep a development version on their branch between releases, for example `0.21.26-dev` in a manifest, and set the plain version only at the release tag. A fork that must take only released versions sets `tags` on its follow upstream:

```toml
[[upstream]]
name = "upstream"
url = "https://example.com/team/app.git"
branch = "main"
tags = "v*"
```

`start` named without a revision then takes the upstream at its newest **release**: of the upstream's tags, listed with `git ls-remote --tags` from its URL, those that

- match the pattern,
- are a semantic version once the pattern's literal prefix (the text before its first `*`, `?` or `[`) is removed, together with one `v` or `V` after it unless the prefix itself ends in `v` or `V`: `v0.21.25` matches `v*` as version 0.21.25, `release-v3.1.4` matches `release-*` as 3.1.4, and `vv1.2.3` is no version under `v*`,
- have neither a prerelease part nor build metadata, so `v1.0.0-rc.1` and `v1.0.0+build.5` never count, whatever the pattern,
- and point at a commit of the fetched upstream branch, so a release made on another branch, such as a maintenance branch the upstream never merged, never counts;

the one with the highest version by semantic versioning precedence: `v0.10.0` is newer than `v0.9.0`, whatever their order on the branch. Tags whose names are not versions in this sense are ignored. To take a prerelease, or any other commit, name it: `start upstream=v1.0.0-rc.1`. No tag is fetched; a release's commit is on the fetched branch, which `refs/git-intake/upstreams/<name>` holds while the update runs, so the repository's `refs/tags` never change.

The release is the target, and the usual rules apply to it (see *Choosing a target*). When the integration branch already contains it, `start` reports the upstream as current, even when an earlier update took a later, unreleased commit by naming it. When the branch has no release, `start` refuses and says to name the target; `status` shows the state `no release`, and `status --check` fails.

Releases before the one that introduced `tags` refuse a configuration that contains it, as an unknown key. Upgrade every place that runs git intake on the repository, including CI jobs that pin a release, before committing `tags`. An update started by this release and carried on by an earlier one of the same state format loses the tag it recorded for each target when that release saves the state, so its sync merges carry no `Intake-Tag`; finish an update with the release that started it.

`status` shows the newest release in the `TIP` column and counts `BEHIND` against it, so the development commits after the release are not drift. A line below the table names the fetched branch tip and how far it is past the release. In `status --json`, `tip` stays the fetched branch tip, and `tags`, `release`, `release_tag` and `commits_after_release` describe the release. `log` lists the commits up to the release, and `status --check` predicts the conflicts of merging the release.

### Port mode

An upstream in port mode takes chosen upstream commits. The unit is a commit on the first-parent line of the upstream branch: a hosted project lands its pull requests on that line, by squash, merge commit or rebase, so each first-parent commit is one landed change or one commit of a rebased series. A merge commit is taken as its change against its first parent (`git cherry-pick -m 1`), so a pull request merged with a merge commit is one decision.

**The journal.** A port upstream keeps its decisions in `.git-intake/<name>.toml`, committed on the integration branch and read from its tip like the configuration:

```toml
base = "<the upstream commit the port started from>"
retired = ["<upstream commit>"]   # optional: see *Recovering from a rewrite*

[[decision]]
commit = "<upstream commit>"
verdict = "skip"          # or "have": the product already has the change
note = "optional reason"
```

Commit ids are full ids; upper case is read as lower case. The tool sets the base when it creates the journal and never moves it; only the owner moves it, after a rewrite. A taken commit is recorded by the commit that took it, not in the journal. The tool refuses a journal with another verdict, a commit decided or retired twice, or ids that are not full commit ids, and names the journal and the problem. The journal is an ordinary file: a decision is reviewed in the same merge request as the code, and an ordinary commit can change it, for instance remove a skip so that its commit is pending again. When the tool records decisions, it writes the journal in the form shown above.

**Pending commits** are computed every time: the first-parent commits of the fetched upstream branch after the base, minus those that a commit on the integration branch names as taken in its trailers `Intake-Upstream: <name>` and `Intake-Taken: <upstream commit>`, minus those the journal decides. A commit copied by hand counts as taken when it carries both trailers, wherever it sits in the integration branch's history. When the base, or a decided or taken commit the journal does not retire, is no longer on the first-parent line of the fetched branch, the upstream was rewritten: `status` reports it and `start` refuses to plan it.

**Recovering from a rewrite.** When the upstream force-pushes commits the product took or decided on off its first-parent line, `status` (in its text, and in `off_line` of its JSON) and `doctor` name each such commit and what it was recorded as, and the base when it is gone too. The tool cannot tell a commit replaced by an equivalent one from a change the upstream withdrew, so it waits for the owner to acknowledge them: list them under `retired` in the journal, in an ordinary commit on the integration branch, and if the base is gone, move it to a commit of the upstream's current first-parent line. Retired commits count as neither taken nor decided, so the rewritten versions of their changes show as pending, and the next plan can mark them `have`. A retired commit needs no object in the repository; one that garbage collection pruned is retired the same way. While the base is off the line, the upstream stays rewritten.

**The first plan creates the journal.** Without a journal, the base is the upstream's last synced commit, as the newest sync record names it or, without one, as the history gives it (the newest upstream commit the integration branch contains), so an upstream followed until now, or a product created by cloning its upstream, starts porting where it is. The first update that carries out a plan commits the journal with that base. When there is neither a journal nor a last synced commit, because the product shares no history with the upstream, `status` shows the state `no base`, `status --check` fails, and `start` refuses; each says to write the journal with its base by hand.

**The plan.** `start` fetches the upstream, computes its pending commits and, when there are any, creates the update worktree and branch and writes the plan to `<git-common-dir>/git-intake/plan`, then stops and names it:

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

Edit it, as a person or an agent, and run `continue`. A line is a verb, the commit (any unique prefix of at least four hex digits of a listed commit), and optionally the subject and a note after ` # `; comment and empty lines are ignored. `continue` refuses a plan that names a commit it does not list, names one twice, uses another verb, or decides nothing (every commit `later`), names the line, and changes nothing. `start <name>=<revision>` limits the plan to the pending commits that the revision contains, so that a review can stop at an upstream release; the revision is resolved as under *Choosing a target*, without the rules about the last synced commit.

**Carrying out the plan.** `continue` copies each commit marked `take` onto the update branch in the order the plan lists them, so a later fix can be taken before an earlier feature. A copy keeps the upstream commit's author and message and adds the trailers:

```
Intake-Upstream: example-app
Intake-Taken: <full id of the upstream commit>
```

A copy that stops on conflicts stops the update as a conflicted merge does: the tool names the upstream commit and the conflicted files, lists the commits copied and still to copy, and exits with status 1. Resolve the files, stage them, and run `continue`, which commits the copy with its trailers and goes on with the remaining takes; a copy committed by hand with both trailers is accepted too. A copy that changes nothing, because the product already has the change, is not committed; it is recorded as `have` with the note `taken, but its copy changed nothing`, and `report` names it.

When the commit of a copy fails, for example because a pre-commit hook of the repository rejects it, the update stops at the step `commit_failed` with git's output and exit status 1, and the copy stays staged in the update worktree. Fix the cause there and stage the result; `continue` commits what is staged as the copy, with the upstream author, message and trailers. `continue` never resets the worktree, so a fix made by hand, staged or not, is never thrown away. Only what is staged is committed: when a copy resumed after an interruption has its changes in the worktree but nothing staged, the update stops the same way and asks to stage what belongs to the copy, and a copy counts as changing nothing only when neither the index nor the worktree differs.

A copy never changes `.git-intake.toml` or anything under `.git-intake/`: an upstream built with git intake carries its own configuration and journals, which are not the product's. The tool restores those paths from the update branch head before it commits a copy, including paths in conflict, names the paths it left out in the output of `continue` and in `report`, and records a copy that changed nothing else as `have`, with the note `taken, but its copy changed nothing outside .git-intake.toml and .git-intake/`. Last, the `skip` and `have` decisions with their notes are committed to the journal in one commit, which also creates the journal; a plan without decisions on an existing journal adds no journal commit. Commits marked `later`, or left out of the plan, stay pending. The update then goes on to `verify` and `finish` like any other.

An upstream commit often builds on earlier ones. The tool cannot know such a dependency, but it points at a likely one: for each taken commit, `report` lists the earlier upstream commits still pending that changed any of the same files. A copy can apply cleanly and still miss what an earlier commit introduced; the checks and the review catch that, and the report says where to look.

### Contributing to an upstream

```sh
git intake contribute example-app fix-watcher        # a branch at the fetched tip
git intake contribute example-app=v1.4 fix-watcher   # a branch at the upstream's tag v1.4
```

`contribute` fetches the upstream and creates the local branch at its fetched tip, or at the commit a revision names on the fetched branch, resolved as for `start` but without the rules about the last synced commit. The branch holds only upstream history: copy the change onto it (for example with `git cherry-pick`) and push it to where pull requests to the upstream come from, such as your fork on the upstream's host. The tool sets no tracking branch, changes no checkout, and never pushes the branch. It refuses a branch that exists or a name that is not a valid branch name.

### Publication through a merge request

With `via = "merge-request"`, nobody needs to push to the integration branch, so it can stay protected. Once published, the merge request is the update: its pipeline verifies the update, the forge merges it, and `git intake sync` keeps it current, from any clone or from a CI job. The tool reaches the forge, GitLab or GitHub, through its REST API, and requires that the forge merges only a tree its required pipeline tested. `finish` requires no local verification in this mode; `verify` still runs the checks locally for whoever wants them before the review.

`finish` then does the following, moving no local branch:

1. It reaches the forge of the publish remote (see *The forge* below), and refuses, pushing nothing, when it cannot: the forge is not recognised, no token is set, or the forge rejects the token or a request, with the forge's status and message.
2. It fetches the publish remote's integration branch. When that already contains the update head, the update landed by other means: `finish` pushes nothing, opens no merge request, removes the update and says so. When it moved past the base, `finish` catches up with it (see *Catching up with a moved integration branch*), without running the checks.
3. It reads the forge's merge settings and refuses, pushing nothing, unless they guarantee that only a tree the required pipeline tested is merged (see *Merge settings* below).
4. It pushes the update head, without force, to the branch `git-intake/update` on the publish remote: on each push URL where the branch is not at the update head already. The fetch URL decides the forge; further push URLs receive the branch as mirrors.
5. Through the API it opens the merge request from that branch into the integration branch, on GitLab with squashing off and the branch removed on merge, or rewrites the title and description of the update's open merge request. Both forges refuse a second merge request from the same branch; when another run opened one meanwhile, `finish` rewrites that one.
6. It removes the update worktree, the update branch, the plan, the update's references and the state, and names the merge request with its address. The merge request carries the update from then on.

A `finish` interrupted before the merge request exists keeps the update; run again, it pushes only where the branch is missing and opens the merge request. Messages speak of a merge request on GitLab and of a pull request on GitHub.

`finish` refuses, pushing nothing and keeping the update, when a merge request of the update into another branch is open, or when the update's merge request is open with a head this update does not contain, as when another clone published another update. `git intake abort` followed by `git intake sync` then carries that merge request on; `git intake abort --merge-request` withdraws it, which also removes this update and declines its targets until `start` and `finish` publish them again. When `git-intake/update` is on the remote without an open merge request, `finish` pushes over a branch the update head contains, deletes one that holds nothing but catch-up merges beyond the integration tip or beyond the head of a closed or merged merge request of the update, and refuses any other, naming the branch, the push URL and `abort --merge-request`, since it may be someone's work. A deletion is pushed with a lease on the commit read on each push URL, so a commit pushed meanwhile is never lost.

`finish --no-publish` keeps its meaning: it requires a verified update, makes only the local move, and closes the update without pushing.

#### The forge

The forge is that of the publish remote's fetch URL as git uses it, after its `url.<base>.insteadOf` rewriting (as `git remote get-url` reports it): the merge request is opened on the forge where the branch actually lands, and a shorthand that `insteadOf` expands, such as `gh:team/app`, names the forge it expands to. The publication setting is always read from the configuration at the integration branch tip, never from an update's head, which carries content merged from upstreams: otherwise an upstream could point the token at a host of its choosing.

- **The kind of forge** comes from the setting `forge` (`gitlab` or `github`); else from the URL's host: `gitlab.com` is GitLab, `github.com` and a `ghe.com` subdomain are GitHub; else from the CI job the tool runs in, when the job's server is the remote's host: a GitLab CI job whose `CI_SERVER_URL`, or a GitHub Actions job whose `GITHUB_SERVER_URL`, names that host.
- **The API base** is the setting `api`, an `https` URL on the remote's own host: for an SSH host alias, the host the SSH configuration names for it (as `ssh -G` resolves it; an IP address is no alias), and for GitHub also `api.<host>`. Without the setting, in a CI job of the remote's host, it is the job's `CI_API_V4_URL` or `GITHUB_API_URL`, which must name the same host (or `api.<host>` for GitHub) and be `https` unless the job's server itself is reached by plain HTTP; otherwise `https://<host>/api/v4` for GitLab, `https://api.github.com` for github.com, `https://api.<host>` for a `ghe.com` host, and `https://<host>/api/v3` for GitHub Enterprise Server.
- **The project** is the URL's path without credentials and `.git`, less the path the API base carries before `/api/v4` or `/api/v3`, which covers a GitLab installed under a relative URL root.

An SSH host alias, or an SSH URL of a GitLab under a relative URL root, names neither the forge nor its API; set both:

```toml
[publish]
remote = "origin"
via = "merge-request"
forge = "gitlab"
api = "https://gitlab.example.com/gitlab/api/v4"
```

A remote on a host the tool does not recognise, without these settings, is refused by the commands that need the forge, naming `forge` and `api`; so is an `api` on another host than the remote's.

An `https` API is reached with rustls, and its certificate is checked by the operating system's verifier, which trusts the system's certificate store (on Linux the system bundle, or the file `SSL_CERT_FILE` names), so a self-managed forge behind a private certificate authority works once its root is trusted there. The proxy of `ALL_PROXY`, `HTTPS_PROXY` or `HTTP_PROXY` is used, with `NO_PROXY`, in upper or lower case. A redirect to another host is followed without the token. GitHub requests ask for the API version `2022-11-28`, which every supported GitHub Enterprise Server release knows.

#### Tokens

The token comes from the environment only, under the names the forges' own command-line tools use:

| Forge | Variables, in order | Sent as |
| --- | --- | --- |
| GitLab | `GITLAB_TOKEN` | `PRIVATE-TOKEN` header |
| github.com and `ghe.com` hosts | `GH_TOKEN`, `GITHUB_TOKEN` | `Authorization: Bearer` |
| GitHub Enterprise Server | `GH_ENTERPRISE_TOKEN`, `GITHUB_ENTERPRISE_TOKEN`; in an Actions job of that server also `GH_TOKEN`, `GITHUB_TOKEN` | `Authorization: Bearer` |

A github.com token is never sent to an Enterprise Server, except the token an Actions job of that server is given. Without a token, `start`, `finish` and `sync` refuse before they push anything and name the variables. The tool never prints, stores or logs the token, and sends it only to the API's host.

The token must be able to open merge requests:

- **GitLab**: a project or group access token, or a personal access token, with the `api` scope and at least the Developer role; Maintainer where the integration branch's protection requires it for pushing. Reading the merge settings needs only read access to the project. The CI job token cannot open merge requests, and the pushes it may make where a project allows them start no pipeline.
- **GitHub**: a GitHub App installation token, or a fine-grained personal access token, with *Pull requests: write* to open, rewrite and close pull requests, *Contents: write*, which also lets the token see the repository's merge settings, and *Metadata: read*, which the rulesets need. Add *Administration: read* only when the integration branch's requirements live in classic branch protection. Not Actions' `GITHUB_TOKEN`: pushes made with it start no workflow, and pull requests it opens start their runs only after someone approves them, so the pull request would sit untested.

Pushing goes through git with the remote's own configuration, an SSH key or a credential helper, as before: the tool never puts the API token into git's configuration. The credential that pushes must be one whose pushes start the forge's pipeline.

#### Merge settings

The guarantee needs two things: the merge cannot happen without a successful pipeline, and the tree that pipeline tested is the tree that lands. Squash and rebase merges are excluded besides, because they drop the merge commits whose trailers are the sync records. `finish` and `sync` read the settings before every push of the update branch, and `doctor` reads them too; a refusal names each setting to change.

On **GitLab**, every field the tool reads is visible to any token that can read the project:

- *Pipelines must succeed* is on (`only_allow_merge_if_pipeline_succeeds`), and *Skipped pipelines are considered successful* is off (`allow_merge_on_skipped_pipeline`): a skipped pipeline tests nothing.
- The merge method is *Merge commit with semi-linear history* (`rebase_merge`) or *Fast-forward merge* (`ff`). GitLab then merges only a branch that contains the target branch, so the merged tree is the tree of the head the pipeline tested. The plain *Merge commit* method is refused, even with merged results pipelines: GitLab requires the pipeline to match the merge request's head, not the current target branch, so a merged results pipeline can be outdated when the merge happens.
- Merge trains are off: whether a train lands the commit it tested could not be confirmed, and with fast-forward or semi-linear history its refs are rebased.
- *Enable automatic rebase prior to merge* is off (`automatic_rebase_enabled`; an instance without the setting counts as off): a rebase drops the update's merge commits. `sync` keeps the branch mergeable by catch-up merges instead.
- *Squash commits when merging* is not *Require* (`squash_option` `always`). *Encourage* and *Allow* let the person merging tick squash, which the tool cannot prevent: the description asks not to, and `doctor` warns. GitLab Premium's per-branch squash setting is readable only through an experimental GraphQL field, so the tool does not read it; a squash that happens anyway shows as a merge request merged without its history (below).

On **GitHub**, for the integration branch:

- A status check is required, naming at least one check, with *Require branches to be up to date before merging* (strict), through a ruleset or classic branch protection. GitHub then merges a pull request only when its head contains the base branch and its checks passed on that head. Without strict, a `pull_request` workflow tests the merge as it was when it started, which can be outdated.
- No merge queue applies: GitHub does not say whether a queue lands the commit it tested or makes a new one.
- No rule requires a linear history, which forbids merge commits.
- Merge commits are allowed: the repository's `allow_merge_commit`, or a ruleset's pull request rule that allows `merge`.

The rules come from `GET /repos/{owner}/{repo}/rules/branches/{branch}`, which returns the active rules of repository and organisation rulesets to any token that can read the repository. Classic branch protection is readable only with administration access, and the repository's merge settings only with push access; absent, they are unknown, not false. Whatever the token cannot read and the rules do not settle cannot be confirmed: the tool refuses, says which requirement it could not confirm, and names both ways out, a ruleset or the access the token lacks. Squash or rebase merges that stay selectable are warnings of `doctor`.

The guarantee is the forge's, and so is its limit: people the forge lets bypass its rules, administrators on GitHub without enforcement for them or bypass actors of a ruleset, can merge anything, as they can push anything.

#### Title and description

The title names the updated upstreams. The description lists each follow upstream's merged range with the upstream tag of its target, and each port upstream's taken, skipped and already present commits. It says to merge the merge request without squashing it and without rebasing its branch, including with the forge's rebase or update-with-rebase buttons, because either drops the update's history, and to keep it current with `git intake sync`. When the update changes its own checks, the forge's CI definition (the project's CI configuration file on GitLab, `.gitlab-ci.yml` unless the project names another, and `.github/workflows/` on GitHub) or the install command or checks of `.git-intake.toml`, `finish` and `sync` name those changes in their output and in the description, since the pipeline that is supposed to verify the update then runs what the update brings. They name each check added, removed or given another command by its name, and say when only the order of the checks changed; a renamed check counts as one removed and one added. When `.git-intake.toml` differs between the base and the head and either side cannot be read, as a base whose checks are still plain commands, its install command and checks are named as changed, without names.

The title and description belong to the tool: every push of the update rewrites them, and on GitLab asks again for squashing off. The description ends with a block that renders as nothing and records each follow upstream's target:

```
<!-- git-intake targets
upstream 0123456789abcdef0123456789abcdef01234567
-->
```

`sync` reads it back: from a merge request closed without merging, to tell the targets a person declined, and from a merged one, to tell whether the merge kept the update's history. A description without a readable block, as one an earlier release wrote or a person edited, records nothing. A description longer than the forge takes is shortened before the block.

#### Merging

Merge the merge request with a merge commit or by fast-forward, as the settings above allow. A commit someone adds to the merge request branch, a fix for instance, is welcome: the pipeline checks it and the update keeps it. After the merge there is nothing to do locally. The local integration branch does not move when the merge request is merged; the next `start` or `sync` brings it up to date by fast-forward, and the merged sync records count on the integration line however the merge request was merged.

A merge request **merged without its history** is one whose final head, as the forge reports it, the integration branch does not contain, or whose recorded follow targets lack a sync record on the integration line: a squash, or a rebase before a fast-forward merge. `status` names the newest one, and `sync` proposes the same targets again, with a warning that names it, and names it in the new merge request's description: the new merge request carries little more than the sync records, and merging it records them. Every retry waits for a person to merge it, so a project that squashes everything gets one open merge request, not a loop. A sync record counts only on a merge commit whose second parent is its target, so the trailers a squash commit's message copies, as GitHub's squash merge writes them, do not count.

#### Updates left waiting by an earlier release

Releases before this one kept an update alive on the machine that opened its merge request (state formats 4 and 5, in the step `waiting_for_merge_request`) and completed it with a second `finish`. The first `finish` of this release hands such an update over: it finds the merge request through the API, by the number the earlier release recorded or else by its branch, which may be `git-intake/update-<commit>` in format 4, says whether it is open, merged or closed, records the resolutions of a catch-up step it stopped in, and removes the update worktree, branch, references and state. The merge request and its branch stay as they are, and `sync` carries the merge request on. `abort` removes such an update, keeps the merge request and says what became of it; `abort --merge-request` withdraws the merge request as well. Every other command shows the update as waiting, left by an earlier release, and names `finish`.

Of such a state this release reads only what the hand-over and `abort` need, and neither the checks nor the verification it records, whose checks are plain commands. `finish` hands the update over, and `abort` ends it, even while the configuration committed on the integration branch still lists its checks as plain commands; they then use the publication setting the update recorded. The state file is never written: it stays as the earlier release left it until it is removed.

### Unattended synchronisation

`git intake sync` takes the repository's follow upstreams in without any local state: it is what a scheduled CI job runs, and what a person runs to take over where a job stopped. Each run derives everything from the publish remote, its forge and the upstreams, and carries the update as far as it can.

Like `start`, it reads the configuration at `HEAD`, which needs a `[publish]` setting, fetches the publish remote and fast-forwards a local integration branch strictly behind the remote's, refusing when the two diverged. In a CI clone, with `HEAD` detached at a commit of the integration branch and no local integration branch, it creates that branch at the remote's. It reads the configuration again at the new tip. In merge-request mode it refuses a local integration branch with commits the remote lacks, since they would ride along in the merge request unannounced. It refuses while an update is in progress, naming its step and the next command.

In **merge-request mode** `sync` runs no install command and no check: the merge request's pipeline verifies the update.

- **The update's merge request is open.** `sync` fetches its head. When the head contains the integration tip, nothing changes: it says that the update waits for the merge request and names the upstreams that moved on since, which the next update takes; a follow target stays fixed for the life of an update. When the integration branch moved past the head, `sync` carries the merge request's update on locally, catches it up with the integration tip by a catch-up merge, and publishes it as `finish` does: the merge request's head becomes the catch-up merge, pushed without force, with the merge request's head as its second parent, including any fix someone pushed onto it, and the title and description are rewritten. Just before the push it reads the merge request's state and head and the remote's integration branch again: a merge request merged or closed meanwhile ends the run with status 0, any other change with status 2 and the advice to run again. Right after the push it reads the state once more and, when the merge request is no longer open, deletes the branch the push recreated while it is still at the pushed commit.
- **A rewritten merge request** is left alone: one whose head lacks the sync record of a follow target its description records, as the forge's rebase button leaves it, or, without readable targets, whose head carries no update. A person amending their own fix by force keeps the sync merges and is not rewritten. `sync` refuses, names the merge request, and says that withdrawing it with `abort --merge-request` declines its targets, and that `start` and `finish` publish them again by hand.
- **No merge request is open.** `sync` plans an update of every follow upstream the integration branch lacks the target of, the fetched tip or, with `tags`, the newest release, as `start` would with no upstream named, merges them, and publishes the update as `finish` does. It starts nothing when every follow upstream is current.
- **Declined targets.** When the merge request closed or merged most recently was closed without merging and its description recorded every follow target `sync` would merge, a person declined those targets: `sync` starts nothing, names the closed merge request and exits with status 0. The next update comes when an upstream moves on, or by hand with `start` and `finish`.
- **The update's branch without an open merge request.** GitHub keeps the branch of a closed pull request, a job can be killed between the push and the merge request, and a withdrawal can stop between closing and deleting. A branch that holds nothing but catch-up merges beyond the integration tip or beyond the head of a closed or merged merge request of the update is deleted, under a lease on the commit read, and the run goes on. A branch that holds an update no merge request carries is carried on and gets its merge request, unless its targets were declined, which `sync` refuses so that the branch, perhaps with a person's fix, is not silently ignored. Anything else is refused, naming the branch, the push URL and `abort --merge-request`, because the branch may be someone's work.

In **push mode** there is no merge request: `sync` starts the update, runs `verify`, and when the checks pass runs `finish`, which pushes the integration branch.

Upstreams that `start` would refuse were they the only one selected, because they cannot be fetched, have no release or were rewritten, or because their path cannot take them, are left out and named, so that one broken source does not stop the others; the run then exits with status 1 for the job to show. Port upstreams are never taken by `sync`, since their plans are reviewed by a person; those with pending commits are named.

A conflict, in an upstream merge or a catch-up merge, stops `sync` exactly as it stops `start`, with exit status 1 and the update in progress in the update worktree; so do failed checks in push mode. In CI the job fails and its clone is discarded. A person then runs `sync` in their own clone, meets the same stop while nothing moved, resolves it, and runs `continue` and `finish`; for a catch-up of an open merge request, `finish` pushes the result to the same merge request. Every other end, a refusal such as a catch-up precondition included, removes the update `sync` created, so the next scheduled run meets the same situation rather than an update in progress.

Recreating an update from history is possible because an update describes itself (see *The update line* below). What history does not carry, the logs and the conflicts of earlier steps, `report` cannot show for an update carried on from a merge request, and says so. Such an update records the merge request it was carried on from, and `finish` refuses to publish it once that merge request is no longer open, keeping the update and naming `abort`.

Two runs at once share one branch: the first push wins, and the second is rejected as not a fast-forward, which `sync` reports with status 2 and the advice to run it again; it opens no merge request. `sync` exits with status 2 when it refused or failed, 1 when it stopped on a conflict or failed checks or left out a follow upstream, and 0 otherwise.

### The update line

Several rules need to know which commits are an update's: `env`, carrying an update on, and telling a rewritten merge request. Every commit between the head and the base is the wrong answer, because a sync merge brings in the upstream's whole history, which may carry sync records of its own. The **update line** walks from the head along first parents, as the update branch grows, through the commits the base lacks, and crosses to the second parent only at a catch-up merge, which carries `Intake-Catch-Up`, and at a head that merges the update into a commit the base contains without a sync record, as a merged results pipeline tests. A merge the forge's update button makes has the branch's previous head as its first parent and stays on the line. The update's own commits are the sync merges, copies and changes to a port upstream's journal on that line; a line without any carries no update.

### The update variables outside verify

A merge request pipeline runs its own jobs, on a fresh clone without an update state. `git intake env` gives them the variables `verify` gives the checks (see *The environment of the checks*), for `HEAD` or a named commit, read from history alone:

```sh
git intake env -- ./scripts/check.sh   # run a command with the variables
git intake env --dotenv > intake.env   # NAME=value lines, for a dotenv report or $GITHUB_ENV
git intake env --json                  # one JSON object
git intake env                         # export lines, quoted for the shell
```

The integration tip is the publish remote's integration branch, fetched for the command, or the local integration branch when no publish remote is configured; a failed fetch refuses with git's reason. For a commit H, the base is the merge base of H and that tip: the tip the update started from, or the one it last caught up with, and for a merged results pipeline, whose commit merges the update into the target, that target, which is the base of the tree being tested. The upstreams and their targets come from the update line, their previous synced commits from the integration line at the base, so the values are those `verify` gives for the same commit.

With a command after `--`, `env` runs it with the variables, without the `GIT_INTAKE_` variables it inherited, and exits with the command's status; when it cannot read the variables, as in a shallow clone or when the publish remote cannot be fetched, it exits with status 2 without running the command. A commit the integration tip contains, or whose update line carries no update, gets no variable: `env` says so on standard error and exits with status 0, `--json` prints `{}`, and a command after `--` runs without any `GIT_INTAKE_` variable, so a check that falls back to its usual baseline outside an update keeps working in a pipeline that fetches the full history.

Prefer `env -- <command>` or a `--dotenv` file. `eval "$(git intake env)"` loses `env`'s exit status: when it fails, the shell evaluates nothing and the check runs without its variables.

### Abort

`abort` removes the worktree, the update branch, the plan, the update's upstream references, its logs and the state at any step before `finish` moves the integration branch, including a catch-up step. The integration branch stays where `start`, or the last fast-forward to the publish remote by `start` or `finish`, left it; a port's journal is unchanged. Resolutions already made by hand in a stopped merge or copy are recorded first, so a retry reapplies them. A worktree that has `git-intake/update` checked out anywhere other than where `start` places the update worktree belongs to the user: `abort`, `start`, `sync` and `doctor` name it and leave it for the user to remove or switch.

`abort` never touches the forge. Closing a merge request that a CI job opened, perhaps while someone reviews it, is a different act, with its own spelling: `git intake abort --merge-request` removes the local update in progress, if any, then closes the update's open merge request through the API and deletes the update's branch on each push URL while it points at the commit read, open merge request or not; it says what remains, and that there is nothing to withdraw when there is neither. A branch someone moved meanwhile is kept and named. Run again after an interruption between closing and deleting, it deletes the branch the closed merge request left. Withdrawing declines the merge request's targets: `sync` proposes them again only once an upstream moves on, and `start` with `finish` publishes them by hand. Running `abort` twice never escalates from the local update to the forge. With no update in progress in merge-request mode, plain `abort` names `abort --merge-request` when the update's merge request is open, or says that it could not read the forge; on an update carried on from a merge request, it removes the local update, and the merge request stays open.

### The update state

The update state lives in `<git-common-dir>/git-intake/state.json`, which every worktree shares and no commit can carry. Its format has a version, and the state records the release that started the update, `start` or `sync`. Format 6 records the configuration with named checks and a partial run beside the verification. This release reads an update of format 4 or 5 only when it waits for its merge request, to hand it over or abort it (see *Updates left waiting by an earlier release*). Any other state in a format other than 6, older or newer, is refused by every command that reads the update, which changes nothing: the refusal names both formats and the release that started the update when the state records it, and advises finishing or aborting the update with a release that keeps that format, then starting it again with this release. For format 3 it also names carrying the update over with `migrate` of v0.3.6 and finishing it with that release. The formats belong to these releases:

| Format | Releases |
| --- | --- |
| 1 | v0.1.0 |
| 2 | v0.2.0 to v0.2.2 |
| 3 | v0.3.0 to v0.3.1 (the update worktree in the git directory) |
| 4 | v0.3.2 to v0.3.5, also carried on by v0.3.6 (no catch-up; the merge request branch named after the verified commit) |
| 5 | v0.3.6 |
| 6 | v0.3.7 and later (named checks and partial runs) |

A newer format is refused with the advice to use the release that started the update, or a later one that reads its format. Another release with the same format carries the update on; the first of `continue`, `verify` and `finish` that it runs warns once that a different release started it. Before `finish`, the warning advises `abort` and a new `start` to use one release throughout; once `finish` has moved a branch or pushed the update, it gives no such advice, because `abort` would leave what is already published behind.

### Upgrading to named checks

Named checks change the form of `checks`, and no release reads both forms: a release before v0.3.7 refuses the table form, and v0.3.7 and later refuse the list of plain commands. Every command refuses a `.git-intake.toml` with the old form and shows the form to write. Upgrade in this order:

1. Finish or abort every update in progress with the release that started it, except an update waiting for its merge request. This release cannot carry an update of an earlier state format on.
2. Convert the checks on the integration branch to the table form and upgrade every runner, CI jobs included, together.
3. Hand a waiting update over with `finish` of this release.

Scripts that read the output of `verify` or its log files see the checks' names instead of their command lines, and `install.log` and `check-<name>.log` in place of numbered files.

## The sync record

Every merge commit the tool creates looks like this:

```
sync(app-a): merge app-a/main at 1a2b3c4 (v2.3.0)

Intake-Upstream: app-a
Intake-Target: <full commit id>
Intake-Tag: v2.3.0
```

`Intake-Tag` names the upstream tag that points at the target, and is left out when no tag does. It is the tag a revision named, the release that `tags` selected, or otherwise a tag of the target commit: of the tags that point at it, the highest version among those that match the upstream's `tags` pattern and are a version as described under *Release tags* (prerelease and build metadata allowed), or without such a tag the highest version among the tags whose name is a semantic version with an optional `v` or `V`. A tag whose name is no version, such as `archive/2024` or `vnext`, is not recorded this way, even when it matches the pattern; a tag named on the command line is recorded whatever its name. A tag whose name contains a comma is never recorded, because trailer readers split values at commas. The tag is fixed by `start` with the target, so a tag added or moved later does not change the record.

The trailer is how release tooling learns which upstream release the repository holds, for example a downstream whose version follows the upstream release it merged. The newest sync merge of an upstream named `upstream` names it:

```sh
git log -1 --format='%(trailers:key=Intake-Tag,valueonly)' --grep='^Intake-Upstream: upstream$' main
```

The output is empty when the newest merge took a commit that no tag names. `status --json` gives the same fact for the last synced commit as `last_synced_tag`, read along the integration line as described below.

A sync record counts only on a merge commit whose second parent is its target, as every sync merge the tool makes is: the trailers a squash commit's message copies from the merges it replaced do not count.

Every commit it copies in port mode keeps the upstream commit's message and adds `Intake-Upstream: <name>` and `Intake-Taken: <upstream commit>`; the journal commit of a port is titled `sync(<name>): record <n> port decisions` or `sync(<name>): start the port journal at <commit>`.

The tool reads its own history along the **integration line**: the first-parent line of the integration branch, where each merge on it is followed by the commits its second parent brought in along that parent's own first-parent line, as `git rev-list --first-parent <merge>^1..<merge>^2` lists them. An update published by push, or a merge request merged by fast-forward, puts the tool's merges on the first-parent line itself; a merge request merged with a merge commit puts them right after that merge commit. A catch-up merge among those commits, marked by its `Intake-Catch-Up` trailer, is followed in turn by the first-parent line of its second parent, which holds the update it caught up, down to the first commit that the first parent of the merge on the first-parent line contains, at any depth; so an update that caught up twice, or caught up and was merged with a merge commit, counts as if it had been merged straight in. Any other merge counts only at that one level: a branch merged into another branch, by a merge without the trailer, before that one was merged brings in nothing of its own. The trailer is as trustworthy as the sync records beside it: both are trailers the tool writes, and the tool reads history it does not police. A sync merge brings in nothing either: what it merged is the upstream's history.

A follow upstream's **last synced commit** is the target of its newest sync record on the integration line whose target no other of its records' targets contains, so a branch carrying an older record that is merged after a newer one never moves the upstream back. It is not stored anywhere else. An upstream without a record has its last synced commit taken from the history instead, and the decision depends only on the integration branch and that upstream's own branch, never on other upstreams or on which of them are fetched. For a root upstream, such as a fork created by cloning, it is the newest upstream commit the integration branch contains. For a path upstream, it is the second parent of the newest merge on the integration line whose second parent belongs to the upstream's branch and which changes the upstream's path. That is the merge `git subtree add` or `git subtree pull` leaves, so ancestry shared through a template does not count, and the import still counts when another upstream later merges this one's history. `status` says so in text, and `status --json` marks the source as `history` instead of `record`. The first update writes a record. Only this repository's own records count. Records inside a configured upstream's fetched branch are skipped; this happens, for example, when a stacked branch starts from another fork's integration branch that carries that fork's records. If a follow upstream's branch no longer contains its last synced commit, the upstream was rewritten: `status` reports it and `start` refuses to update it.

## Commands

| Command | Purpose |
| --- | --- |
| `start [UPSTREAM[=REV]...]` | Starts an update, as described above. A follow upstream named as `UPSTREAM=REV` is taken at the commit REV names instead of its fetched tip (see *Choosing a target*), or without REV, with `tags`, at its newest release (see *Release tags*); a port upstream's plan is limited to the pending commits REV contains. |
| `continue` | Carries out the edited plan of a port upstream, or concludes the merge, copy or catch-up merge stopped on conflicts, and takes in the rest. It also accepts a merge concluded by hand when the commit keeps the sync record, a copy committed by hand when it keeps both copy trailers, and a catch-up merge committed by hand with its trailer. |
| `abort [--merge-request]` | Removes the update worktree, branch, plan, upstream references, logs and state, and never touches the forge. With `--merge-request`, then closes the update's open merge request through the forge's API and deletes the update's branch on each push URL while it points at the commit read (see *Abort*). |
| `verify [--only NAME... \| --skip NAME...]` | Runs `install` and the checks and records the verified commit. With `--only` or `--skip`, runs `install` and only the checks selected by name, as a partial run that never verifies the update (see *Running some of the checks*). |
| `finish [--no-publish]` | Catches the update up when the integration branch moved. In push mode it verifies the catch-up merge, then fast-forwards and publishes the verified update. In merge-request mode it requires the forge's merge settings, pushes the update to `git-intake/update`, opens its merge request or rewrites the open one, and ends the local update. With `--no-publish` it fetches nothing, makes only the local move of a verified update, closes the update without pushing, and names the unpublished branch. On an update an earlier release left waiting for its merge request, it hands the update over to the merge request. |
| `sync` | Takes the follow upstreams in without local state: catches the update's open merge request up, or starts an update of every follow upstream that is behind and publishes it, as merge request or by push after `verify` (see *Unattended synchronisation*). |
| `env [COMMIT] [--dotenv \| --json] [-- COMMAND...]` | Prints the variables `verify` gives the checks for a commit, read from history, or runs a command with them (see *The update variables outside verify*). |
| `status [--check] [--json]` | Fetches every upstream and shows the configuration, with each check by its name and command, then for every upstream its path, mode, branch, fetched tip and one state. A follow upstream also shows its last synced commit and the number of commits the integration branch lacks, with the state `current`, `behind`, `not imported`, `no release`, `path removed`, `rewritten` or `unavailable`; with `tags`, the tip shown and the commits counted are those of the newest release, and `no release` means the branch has none. A port upstream shows its journal's base and its numbers of taken, decided and pending commits, with the state `current`, `pending`, `no base`, `rewritten` or `unavailable`; `no base` means it has no journal and nothing to start one from, and a rewritten one names the commits off its line. `unavailable` means the upstream could not be fetched, its branch is missing, the tags of an upstream with `tags` could not be listed, or, for a port upstream, its journal is invalid; any upstream that cannot be fetched makes `status` exit with status 1. `path removed` means a synced path upstream, from a record or from history, whose path no longer exists on the integration branch; `start` refuses it and `status --check` counts it as drift. When several states apply, `unavailable` comes first, then `rewritten`, then `path removed`. A path upstream is `not imported` until a sync record for it exists or, without one, while its path is absent or holds content that no merge of the upstream brought in; a root upstream is `not imported` while it shares no history with the integration branch. In merge-request mode it shows the update's open merge request with its number, address and head and whether the head contains the integration tip, naming `sync` when it does not, and the newest merge request of the update merged without its history; when it cannot read the forge, it says why and keeps its exit status. While an update is in progress it also shows the step reached, the base and the base the update started from when a catch-up moved it, the worktree, each queued upstream's target or plan and step, a catch-up merge in conflict or awaiting verification with the tip it catches up with, the update's checks by name, the verified commit, the partial run with its commit, the outcome of each command it ran and the checks it left out, and the next command. |
| `report [--json]` | For the update in progress, or the one just finished, per upstream. For a follow upstream: the merged range and commit count, the upstream tag of the target, each conflicted file with its resolution source (by hand or by a recorded resolution), and the size of the repository's own changes against that upstream before and after the merge. For a port upstream: the taken commits in the order copied, each with its conflicted files, the git-intake paths its copy left out and, where there are any, the earlier pending commits that change the same files; the commits recorded as present because their copy changed nothing; the skip and have decisions with their notes; and the number of commits left pending. Ranges and sizes are measured from the base the update started from. Each catch-up merge follows with the integration tip it took in and its conflicted files with their resolution source. |
| `log [UPSTREAM...] [--json]` | Fetches the selected upstreams and lists, for a follow upstream, the commits on its branch that the integration branch does not contain, up to the newest release with `tags`, and for a port upstream its pending commits, oldest first. |
| `branches [UPSTREAM...] [--json]` | Fetches every branch of the selected upstreams for the command and lists every branch that holds commits the integration branch does not contain, with their number; for a port upstream, commits a copy names as taken do not count. |
| `doctor [--json]` | Checks the repository and reports each check as pass, fail or warn. It changes nothing but the references of its own fetches, which it removes. Checks: the configuration is valid; the update in progress, if any, holds its references at the tips `start` fetched; no reference under `refs/git-intake/` is left without an update; every upstream's URL can be fetched and its branch exists; every imported path exists on the integration branch; every follow upstream's last synced commit is still contained in its branch; every port upstream's journal is valid and its base and its decided and taken commits that are not retired are still on its branch, naming each one that is not; no git remote names an upstream's repository, with a note when the remote is the one a partial clone fetches missing objects from; no journal belongs to an upstream that is missing or in follow mode; in merge-request mode, the forge is recognised, a token is set and accepted, and the merge settings qualify, with a warning when squash or rebase merges stay selectable; the main checkout is clean; no merge is in progress there; the update state, if any, matches its worktree and branch. It warns about upstreams that still have sync records but were dropped from the configuration. |
| `contribute UPSTREAM[=REV] BRANCH` | Creates the local branch BRANCH at the upstream's fetched tip, or at the commit REV names on its branch, for a contribution to the upstream (see *Contributing to an upstream*). Never pushes. |

`status --check` additionally predicts, with `git merge-tree`, the files a merge of each behind follow upstream's tip, or newest release with `tags`, would conflict in. It creates no worktree, branch or state, and fails unless every upstream is current, so a port upstream with pending commits fails it too. It can run on a schedule and gate a cutover or a port review.

### JSON output

`status`, `report`, `log`, `branches`, `doctor` and `env` accept `--json`. They then write one JSON document to standard output, with the same facts as the text form under stable snake_case field names, and exit with the same status. A URL in the output, as in every message, carries no credentials: a password is never shown, nor the user of an `http` or `https` URL, where a token is usually written; the user of an SSH URL stays. Notes and errors go to standard error. For example, `status --json` carries `publish` (the publish remote or null), `publish_via` (`push`, `merge_request` or null), `install` (the install command or null) and `checks`, the checks of the configuration at the integration tip, each with `name` and `run`. In merge-request mode it carries `merge_request`, the update's open merge request with `number`, `url`, `head` and `current` (whether the head contains the integration tip), `merged_without_history`, the newest merge request of the update merged without its history with `number` and `url`, each or null, and `forge_error`, why the forge could not be read, or null. An update carried on from a merge request carries `carried_from` with `number`, `url` and `source_branch` in its `update`; an update an earlier release left waiting carries `step` `waiting_for_merge_request` and `merge_request` with `number`, `remote`, `source_branch`, `target_branch` and `commit`. `env --json` prints one object whose keys are the variable names, and `{}` for a commit that carries no update.

Every entry of `status --json` `upstreams` carries `name`, `path`, `url`, `branch`, `mode` (`follow` or `port`), `tip`, `state` (`current`, `behind`, `pending`, `no_base`, `not_imported`, `no_release`, `path_removed`, `rewritten`, `unavailable`) and `detail`. A follow upstream fills `last_synced`, `last_synced_source` (`record`, `history` or null), `last_synced_tag` (the `Intake-Tag` of the newest sync record, or null), `tags` (the configured pattern or null) and `commits_behind`, with `tags` also `release`, `release_tag` and `commits_after_release`, and with `--check` `conflicts`; a port upstream fills `base`, `journal` (whether the journal exists), `taken`, `decided` and `pending`, and when rewritten `off_line`, with `base` (the base when it is off the line, or null) and `commits` (each with `commit` and `recorded_as`: `taken`, `skip` or `have`). Fields that do not apply are null.

Each queued upstream under `update` in `status --json` carries `name`, `mode`, `fetched`, `rev`, `step` (`plan`, `pending`, `conflict`, `merged`) and `conflicts`; a follow upstream adds `target`, `tag` (the upstream tag that names the target, or null), `kind` and `merge_commit`, a port upstream `plan` (the plan file), `commits` (each with `commit`, `subject`, `verb`, `note`, `copy` — `none`, `conflict`, `commit_failed`, `copied` or `empty` — `copied`, `failure`, git's output when the commit of the copy failed, and `left_out`) and `journal_commit`. The update's `step` is `plan` while a plan waits to be edited, `commit_failed` while a copy whose commit failed waits for `continue`, `catch_up_conflict` while a catch-up merge waits for its conflicts to be resolved and `continue`, and `catch_up` while a catch-up merge awaits its verification; in both, `catch_up` gives the catch-up's `tip`, `head_before`, `merge_commit` and `conflicts`, and is null otherwise. `base` is the integration tip the update lands on and `start_base` the one it started from. `checks` lists the names of the update's checks, `verified_commit` is the commit a full run verified, or null, and `partial_run` is the update's partial run, or null: its `commit`, `install`, the install command's outcome or null when no install command is configured, `checks`, each check it ran with `name` and `outcome`, `left_out`, the names of the checks it left out, and `passed`. An outcome is `passed`, `failed`, or `skipped` when the install command failed; a check left out appears only in `left_out`. `report --json` carries `base`, the integration tip the update started from or was carried on at, `carried_from`, the address of the merge request an update was carried on from, or null, and `catch_ups`, each with `tip`, `merge_commit` and `conflicts`. Each entry of `report --json` `upstreams` carries `mode` and `rev`; a follow upstream fills `kind`, `range_start`, `target`, `tag`, `commits`, `merge_commit`, `conflicts` and the diff sizes, and a port upstream fills `port` with `taken` (each with `commit`, `subject`, `copy`, `copied`, `conflicts`, `left_out` and `earlier_pending`), `unchanged` (each with `commit`, `subject` and `left_out`), `decisions` (each with `commit`, `subject`, `verdict` and `note`), `left_pending` and `journal_commit`. Each entry of `log --json` and `branches --json` carries `mode`; a follow upstream's entry of `log --json` adds `release` and `release_tag`, null without `tags`.

## Exit status

| Status | Meaning |
| --- | --- |
| 0 | Success. |
| 1 | A negative result: `status --check` found drift or pending port commits, an upstream could not be fetched by `status`, `log` or `branches`, a `doctor` check failed, `verify` failed, a partial run included, or `start`/`continue` stopped on conflicts or on the failed commit of a copy. `finish` stopped on the conflicts or the failed checks of a catch-up merge, or because the integration branch moved again while it made or verified one. `sync` stopped on a conflict or on failed checks, or left out a follow upstream it could not take. An open merge request is not a negative result. |
| 2 | An error: invalid configuration, a refused command (for example `start` with an upstream that cannot be fetched, `continue` with an invalid plan, `verify` with a check name the update does not have, `finish` on an unverified update in push mode, a head with only a partial run included, on a forge whose merge settings do not qualify or without a token, `sync` on a rewritten merge request or a declined branch, or any command in a shallow clone), a forge that rejected the token or a request, a push of the update's branch rejected because another run pushed it first, a catch-up refused because the integration branch was rewritten or its new commits change what the update means, a git failure (including a push rejected by the publish remote, after which `finish` retries the push, catching up first when the remote moved, and `finish --no-publish` closes the update locally), or invalid usage. |

`env -- <command>` exits with the command's status, or with status 2 without running it when it cannot read the variables.

## Running in CI

A scheduled job runs `git intake sync`; the merge request's own pipeline runs the repository's checks on the update, with the variables `git intake env` gives them. Every command refuses a shallow clone, because a shallow history cuts the integration line off: an upstream's last sync record may lie below the cut, and the tool would plan from the wrong commit without noticing. CI systems clone shallow by default, so set the job to fetch the full history. Install the binary in the job first, as under *Installation*.

The job's clone must push with a credential whose pushes start the forge's pipeline, and the token in the environment must be able to open merge requests (see *Tokens*). The tool commits merges, so the job also sets git's `user.name` and `user.email`.

### GitLab

```yaml
variables:
  GIT_DEPTH: "0"        # the full history; git intake refuses a shallow clone

git-intake-sync:
  rules:
    - if: $CI_PIPELINE_SOURCE == "schedule" && $CI_COMMIT_BRANCH == $CI_DEFAULT_BRANCH
  script:
    - git config user.name "git intake"
    - git config user.email "git-intake@example.com"
    # Push with the access token in GITLAB_TOKEN instead of the job token,
    # whose pushes start no pipeline.
    - git remote set-url origin "$CI_SERVER_URL/$CI_PROJECT_PATH.git"
    - git config credential.helper '!f() { echo username=oauth2; echo "password=$GITLAB_TOKEN"; }; f'
    - git intake sync

upstream-checks:
  rules:
    - if: $CI_PIPELINE_SOURCE == "merge_request_event"
  script:
    - git intake env -- ./scripts/check.sh
```

`GITLAB_TOKEN` is a project or group access token with the `api` scope and the Developer role, or Maintainer where the integration branch's protection requires it; a personal access token works as well. Store it as a **masked and protected** CI/CD variable: protected variables reach only pipelines of protected branches and tags, such as the scheduled pipeline on the default branch, and never the merge request pipeline that runs an update's code. `CI_JOB_TOKEN` cannot open merge requests, and pushes made with it start no pipeline. In a job of the instance that hosts the remote, the tool finds the forge and its API from `CI_SERVER_URL` and `CI_API_V4_URL`, including on an instance installed under a relative URL root.

`upstream-checks` runs on every merge request pipeline and gets no variable outside an update, so a check can fall back to its usual baseline. A job that hands the variables to later jobs writes them as a dotenv report:

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
    steps:
      - id: app
        uses: actions/create-github-app-token@v2
        with:
          app-id: ${{ vars.INTAKE_APP_ID }}
          private-key: ${{ secrets.INTAKE_APP_PRIVATE_KEY }}
      - uses: actions/checkout@v5
        with:
          fetch-depth: 0          # the full history; git intake refuses a shallow clone
          token: ${{ steps.app.outputs.token }}
      - run: |
          git config user.name "git intake"
          git config user.email "git-intake@users.noreply.github.com"
      - run: git intake sync
        env:
          GH_TOKEN: ${{ steps.app.outputs.token }}
```

```yaml
# .github/workflows/upstream-checks.yml
name: upstream checks
on:
  pull_request:
jobs:
  checks:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v5
        with:
          fetch-depth: 0
      - run: git intake env -- ./scripts/check.sh
```

The scheduled job pushes and opens the pull request with a GitHub App installation token, or a fine-grained personal access token with *Contents: write*, *Pull requests: write* and *Metadata: read*, stored as a secret. Not the workflow's `GITHUB_TOKEN`: pushes made with it start no workflow, and pull requests it opens start their runs only after someone with write access approves them, so the pull request would wait untested until a person approves its runs. `actions/checkout` keeps the token it was given for the job's later pushes. A `pull_request` workflow checks out the merge of the pull request into its base by default; `env` then gives the base branch's tip as `GIT_INTAKE_BASE`, which is the base of the tree being tested. To hand the variables to later steps, append them to the environment file: `git intake env --dotenv >> "$GITHUB_ENV"`.

### Code from upstreams in CI

An update brings upstream code into the repository, and its checks run that code: locally in `verify`, in CI in `sync`'s push mode or in the merge request's pipeline. That was always so; what a scheduled `sync` changes is that it happens before a person has looked at the update. A pipeline of a merge request from a branch of the same repository gets the repository's non-protected CI/CD variables on GitLab and its secrets on GitHub, and an upstream that changes the CI definition changes the pipeline that tests it. In push mode, the job that runs the checks also holds the credential it pushes with, typically in the clone's git configuration.

The tool does not pretend to fence this off: hiding its own token variables from the checks would not protect the push credential beside them, and would break checks that use those variables for their own purposes. It makes the exposure visible instead: when an update changes the forge's CI definition, or the install command or checks of `.git-intake.toml`, `finish` and `sync` name the change in their output and in the merge request's description, since the pipeline that is supposed to verify the update then runs what the update brings. Contain the rest by configuration:

- Treat upstreams as code the repository chooses to run.
- Keep deploy secrets in protected variables (GitLab) or in environments with protection rules (GitHub) that an update's branch cannot reach, and keep the token `sync` uses there too.
- On GitHub, a token without the *Workflows* permission cannot push an update that changes `.github/workflows`: such an update stops, for a person to look at and publish by hand with `start` and `finish`.
- In push mode, give the job only the credential it pushes with.

## Moving from upstream remotes and mirror branches

Earlier releases kept a git remote per upstream, its remote-tracking branches, and optionally a mirror branch, and had a `fetch` command. This release refuses such a configuration and names what replaces each setting. To move a repository:

1. Finish or abort any update in progress with the release that started it; this release refuses its state.
2. In `.git-intake.toml`, give every upstream a `url` (the URL of its remote, `git remote get-url <remote>`), remove `remote` and `mirror`, and commit the change on the integration branch. The mode stays `follow` unless set.
3. Remove the upstream remotes with `git remote remove <remote>`, which also removes their remote-tracking branches. `doctor` names every remote left at an upstream's URL. In a partial clone, the remote that missing objects are fetched from must first provide the objects the history still lacks, or that history can no longer be read; `doctor` says so for such a remote.
4. A mirror branch stays an ordinary branch; delete it locally and on the publish remote if nothing else uses it.
5. Replace scripts that ran `git intake fetch`: `status`, `log`, `branches`, `doctor` and `start` fetch what they need.

## Monorepo migration

1. Commit a configuration that lists one `[[upstream]]` per source repository, each with its own `path` and `url`.
2. Run `git intake start`. Each source is imported under its path with its full history, then verified and finished like any update. A path imported earlier with `git subtree add` is adopted by its first update. A path that holds content unrelated to the source's history is refused, and so is an import that would replace a file.
3. Until cutover, keep taking the sources in with the same cycle. Conflicts against the monorepo's new layout are resolved once and recorded. Files moved within an upstream's path merge as renames. Files moved out of the path may conflict or arrive at their old location; resolve them like any other conflict.
4. Run `git intake status --check` on a schedule. It fails whenever a source has moved on and lists the files that would conflict. Zero drift, meaning every upstream is current, is the cutover gate.
5. Before freezing the sources, run `git intake branches` to find work left on other branches of each source. It reports when no upstream holds unmerged work.
6. After cutover, remove an upstream from the configuration. `doctor` keeps warning about upstreams that are dropped but still have sync records, so the removal is visible.

## Safety properties

- The tool never rewrites history and never force-pushes. Local branches only fast-forward, and so does the update branch, through catch-up merges.
- It never pushes to an upstream. The publish remote may not be fetched from or pushed to an upstream's URL, and the tool keeps no remote for an upstream: a reference to an upstream exists only while a command or an update needs it.
- No file of the main checkout is changed, beyond the fast-forward of a checkout of the integration branch. The update lives in its own worktree, `.worktrees/git-intake/update` in the main worktree (in the git directory of a bare repository), on `git-intake/update`, with its state and plan in `<git-common-dir>/git-intake/`, which every worktree shares and no commit can carry.
- Each target is fixed when the update starts, and an update keeps the references that hold its targets and planned commits until it ends. What is merged or copied, verified and published is the same commit.
- In push mode, `finish` moves no ref unless the verified commit is the worktree head, the base is still the integration tip, and the move is a fast-forward. When the integration branch moved, the update first takes the new tip in by a catch-up merge, which is verified before anything moves to it. It validates the move before making it, and it can be resumed.
- Publication by push pushes exactly the planned commit. Nothing unverified is ever published by a retry, and `finish --no-publish` closes an update that cannot be published.
- In push mode, only a commit `verify` passed reaches the integration branch. In merge-request mode, only a tree the forge's required pipeline tested can be merged, by anyone the forge's rules bind: the tool publishes only to a forge whose settings guarantee it, read before every push. The update's branch only moves forward, and a branch the tool did not leave is never deleted or overwritten; deletions go under a lease on the commit read.
- The forge's token is read from the environment only, sent only to the API of the publish remote's own host, and never printed or stored; the publication setting is read from the integration tip, never from an update's head.
- The tool never adds or changes a git remote, and never fetches an upstream through one.
- A rewritten upstream is detected from the sync record or the port journal and copies, and is not taken.
- The configuration and a port's journal are read from the committed integration tip, never from a working copy or from the tree being taken in.
