# git-intake

`git intake` keeps a downstream repository current with one or more upstreams. It takes upstream changes by merge commits or by copying chosen upstream commits, never rewrites history and never force-pushes. The work happens in a separate worktree. A conflict is resolved once, and the repository's own checks run on exactly the tree that gets published. The integration branch moves only by fast-forward, after verification.

It covers three cases:

- **A fork**: one upstream followed at the repository root, merged as a whole. A branch stacked on another fork's integration branch is a fork too, with that branch as its upstream.
- **A monorepo assembled from several repositories**: each source repository is an upstream merged under its own path. They keep being taken in until cutover, and `status --check` gates the cutover.
- **A product built on an open-source project**: the product owns its history and takes only the upstream commits it wants. Each update lists the upstream commits nobody has decided on yet; the product takes some, skips others, and records which ones it already has.

The tool runs the installed `git` for every repository operation. Each repository describes itself in a committed `.git-intake.toml`. Upstreams are fetched by their URL; the repository keeps no remote, remote-tracking branch or other branch of an upstream.

## Installation

Requires Git 2.43 or later (`git merge-tree -X` is used for conflict prediction) and a POSIX `sh`. The tool is a single binary named `git-intake`; with it on the `PATH`, git runs it as `git intake`.

Releases provide prebuilt binaries for the targets `x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl` and `aarch64-apple-darwin`. The musl binaries are statically linked and run on any Linux distribution and in minimal CI images.

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
checks = ["pnpm lint", "pnpm test"]   # run in order by verify

[publish]                             # optional: where and how finish publishes
remote = "origin"                     # the publish remote, a git remote of the repository
via = "push"                          # optional: "push" (default) or "merge-request"

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
- `publish` is a table: `remote` is required, and `via` is `push` or `merge-request`. Write it as the `[publish]` section shown above, after the top-level keys (a section header ends the top level, so keys written below it belong to the section), or inline as `publish = { remote = "origin", via = "merge-request" }`. The string form `publish = "origin"` is rejected with the form to write instead.
- The publish remote may not be fetched from or pushed to (its `url` or any `pushurl`) a URL that names an upstream's repository, because the tool never pushes to an upstream. Two URLs name the same repository when a local path or `file://` URL resolves, against the main worktree and the way git finds a repository (`<path>/.git`, `<path>`, then `<path>.git`), to the same directory, so `../up`, `../up.git/` and the absolute path of that directory match; or when any other URL matches once the scp-like form `user@host:path` is read as `ssh://user@host/path`, scheme and host are put in lower case, a default port such as `:22` or `:443` is dropped, and a leading `/` and a trailing `/` and `.git` of the path are dropped. `doctor` compares the URLs of every remote with the upstreams' URLs the same way.

### Upstreams by URL

An upstream is fetched with `git fetch --no-tags <url> +refs/heads/<branch>:<reference>`, in the main worktree so that a relative URL means what it means there, into a reference under `refs/git-intake/`. That lives outside `refs/heads` and `refs/remotes`, so no branch listing, checkout, `git push --all` or hosting tool sees it, and no remote exists for a tool to take as the place to open pull requests in.

The references exist only while something needs them. Every command that fetches, `status`, `log`, `branches`, `doctor`, `contribute` and `start`, fetches into a namespace of its own, `refs/git-intake/read/<token>/`, and removes it before it exits, whatever its outcome; every run contacts the upstreams. `branches` fetches every branch of an upstream there. An update holds `refs/git-intake/upstreams/<name>` for each queued upstream, at the tip `start` fetched, so that its targets and planned commits stay in the repository even if the upstream moves or becomes unreachable; `continue`, `verify` and `finish` never fetch an upstream. Only `start` writes those references and only `finish` or `abort` removes them: a command that reads an upstream during the update fetches into its own namespace and never touches them. `start` saves the update before it writes them; if it is killed in between, `status`, `continue`, `verify` and `finish` restore the missing references from the update state, at the tips `start` fetched, and say so. `doctor` reports an update reference that is missing or elsewhere, and leaves alone, with a warning to keep it, a reference that still holds the commit of a missing one. It reports any other reference under `refs/git-intake/` that no update accounts for, such as the namespace of a command that was killed; nothing uses it, and `git update-ref -d <reference>` removes it.

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
# with via = "merge-request": finish opens a merge request and the update waits;
# once the merge request is merged, run finish again to complete the update
```

Every command works the same from any directory of the repository: the main checkout, a linked worktree, or the update worktree itself. This holds for a submodule and for a repository whose git directory lies outside its checkout as well. Git operations on the repository as a whole, such as fetching, pushing and moving branches, run in the main worktree, so a relative URL means what it means there. A revision named to `start` or `contribute` is resolved in the worktree the command runs in. When `finish` or `abort` removes the update worktree it was run from, the command still completes and notes that the current directory is gone; change to another directory before the next command.

Git cannot locate the main worktree of a repository whose git directory lies outside it, without naming it, from a linked worktree. The tool then finds it in the update in progress, which `start` records. Without an update, run `start` in the main worktree; from elsewhere it refuses, `doctor` warns that it did not check the main checkout, and a relative URL is resolved against the git directory.

`start` refuses to begin if any of these hold:

- an update is already in progress;
- `.worktrees` or `.worktrees/git-intake` in the main worktree is a symbolic link or not a directory;
- the repository is bare and the path of its update worktree contains `/.git`, as when the bare repository is itself a directory named `.git`; run the tool in a clone with a checkout instead;
- the main checkout or a checkout of the integration branch has uncommitted changes to tracked files;
- the publish remote cannot be fetched, or the integration branch and its counterpart on the publish remote each hold commits the other lacks; `start` names how many on each side, and the local commits must be published or reconciled first;
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

`verify` runs `install` and then every check through `sh -c` in the update worktree, once every queued upstream is merged or its plan carried out. Each command receives variables that describe the update (see [The environment of the checks](#the-environment-of-the-checks)), and its output goes to its own log under `<git-common-dir>/git-intake/logs/`. If `install` fails, the checks are skipped. Verification fails in any of these cases:

- the `.git-intake.toml` of the update branch is missing or invalid, or names a different integration branch (the commands are then skipped);
- a command fails or is skipped;
- the commands change tracked files;
- the update branch moves while the checks run.

When it passes, `verify` records the commit it verified.

`finish` checks the following before moving any ref:

- every upstream is merged or its plan carried out;
- the worktree head is the verified commit, so any commit added after `verify` needs a new `verify`, or a catch-up merge the tool made and has not verified yet, which `finish` verifies itself;
- the base is still the integration tip; if the integration branch moved, `finish` catches up with it first (see *Catching up with a moved integration branch*);
- the verified commit contains the base, every merged target, and every copy and journal commit of a port, so the integration branch only fast-forwards;
- every checkout of the integration branch has no uncommitted changes and no untracked file the fast-forward would overwrite.

In push mode, `finish` first fetches the publish remote, and refuses with git's reason when it cannot. A local integration branch strictly behind the remote's is brought up to date as `start` does; that is not a move of `finish`, so `abort` still works afterwards. When the two each hold commits the other lacks, `finish` refuses before moving anything: the local commits must be published or reconciled first. When the integration branch already contains the verified commit, because it was moved there or past it by other means, `finish` makes no catch-up, counts the move as done, pushes the verified commit unless the remote holds it already, and completes the update.

It then fast-forwards the integration branch to the verified commit. A checkout of that branch is updated with `git merge --ff-only`; otherwise the branch moves with `git update-ref` against its old value. In push mode (`via = "push"`, the default), `finish` pushes the integration branch to the publish remote without force. Finally it removes the worktree, the update branch, the plan, the update's upstream references and the state.

`finish` saves its planned move in the update state before it moves anything, and `status` shows the step as `finishing`. Running `finish` again after an interruption completes the move; a branch already at its planned value, or at a commit containing it, counts as done. It then publishes and cleans up. Once the branch has moved, `abort` refuses and points to `finish` and `finish --no-publish`.

The push sends exactly the planned commit (`<commit>:refs/heads/<branch>`), never whatever the local branch points to by then. A commit added to the integration branch while the update is finishing stays local.

If the push is rejected, nothing is published, the local integration branch stays advanced, and the update stays in the `finishing` step. Running `finish` again fetches the publish remote. When the remote's integration branch did not move past the planned commit, `finish` retries the same push, unless the remote no longer contains its tip as the update last took it: it was rewritten, and a push would publish again what it dropped, so `finish` refuses and names `finish --no-publish`. When it moved past the base, as when other work landed between the fetch and the push, `finish` catches up with the remote's branch from the finishing step: the catch-up merge contains the planned commit, to which the local branch already moved, so the local branch fast-forwards again to the verified catch-up merge, which is then pushed; nothing was published in between. If a commit was added to the local integration branch after `finish` moved it, `finish` makes no catch-up and names `finish --no-publish`. If the remote never accepts the push (for example a protected branch), run `finish --no-publish`. It closes the update locally without fetching. A verified catch-up made from the finishing step is landed locally instead of the planned commit. One that is not verified, or still in conflict, is dropped and the update closes at the planned commit: the resolutions made so far are recorded first, and the commits it drops are named, so that they can still be found until git prunes unreachable commits. It completes the local move with the same checks, removes the worktree, the update branch, the plan, the references and the state, and names the branch it left unpublished (also in `report`). Then merge the remote's work and publish the result through a new verified update or the repository's usual review. `--no-publish` also works on a verified update before any push attempt; it then catches up only with the local integration branch.

### Catching up with a moved integration branch

Other work often lands on the integration branch while an update waits for a conflict resolution, a long verification or a review. The update branch belongs to the tool, so instead of starting over, `finish` takes the new integration tip in with a **catch-up merge**: a merge commit whose first parent is the new integration tip and whose second parent is the update branch head, marked with a trailer:

```
catch-up: merge main at 1a2b3c4 into the update

Intake-Catch-Up: main
```

Every resolution and fix already on the update branch stays exactly as it is, including edits made beyond the conflicting hunks while resolving and commits added on top of the merges; only real conflicts between the new integration commits and the update stop it. With the integration tip as the first parent, the first-parent line of the integration branch stays the line of commits the branch itself received, and the update arrives on the second-parent side, as when a merge request is merged with a merge commit. The base becomes the tip caught up with, and the update counts as unverified; upstream targets, last synced commits and port plans stay as they were. A second move of the integration branch adds a second catch-up merge on top, whose second parent is the first, so the update branch only ever moves forward.

The tool records the tip it catches up with in the update state, checks it out detached in the update worktree, merges the update head into it, and then moves the update branch from the head it recorded to the merge with `git update-ref` against that head, refusing and naming both commits when the branch moved meanwhile; the worktree is then on the update branch again. After an interruption, `status`, `continue` and `finish` recognise a catch-up merge in progress or already committed from the recorded tip, and never make a committed one again. A catch-up merge committed by hand while the worktree is detached, with or without commits made on top of it, is kept: the update branch moves to the worktree's commit and the worktree is on the branch again, and commits on top of the catch-up merge need `verify`. A catch-up given up by hand, with the merge aborted and the update branch checked out again in the update worktree, is no longer recorded once `continue` or `finish` sees it: a commit made there needs `verify`, and `finish` catches up afresh when the integration branch is still ahead.

A conflicted catch-up stops `finish` with exit status 1 in the step `catch_up_conflict`, naming the conflicted files and which side is which: the integration branch is *ours*, the update *theirs*. Resolve the files, stage them, and run `continue`, which commits the catch-up merge with its trailer and names `finish`. The next `finish` verifies the catch-up merge as `verify` does and publishes it; failing checks stop it as `verify` does, with the update unverified at the catch-up merge in the step `catch_up`, and a commit added on top of the catch-up merge needs `verify` as usual. After a catch-up, verification runs the install command and the checks of the configuration at the catch-up merge, so a check added on the integration branch meanwhile verifies the result. `finish` makes at most one catch-up merge per run, and concluding a catch-up merge left by an earlier run counts as that one: when the integration branch moves again meanwhile, it moves no branch to the update and says to run it again, which catches up once more. In merge-request mode, a remote tip that the base already contains is not a move, as when the publish remote lacks commits the local integration branch holds: the merge request carries them.

A catch-up takes the integration branch's own work in. In push mode the integration tip is the local branch, whose tip at `start` is the base, so a local branch rewound below the base counts as rewritten and is refused. Some commits would change what the update means rather than add to it, and a catch-up refuses them, merges nothing and names `abort`, after which `start` plans the update from the new history; once `finish` has moved the integration branch, it names `finish --no-publish` instead:

- a publish remote's tip that moved forward from the tip the update last took but lacks commits of the local integration branch that the base holds and that were never published, as in merge-request mode with unpublished local commits: the refusal names those commits; publish them or reconcile the integration branch with the remote, then `start` again;
- any other tip that does not contain the base: the integration branch was rewritten;
- a commit that carries a sync record or a copy of an upstream the update takes in, as when another clone took that upstream in;
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
| `GIT_INTAKE_UPSTREAM_<NAME>_TARGET` | Follow mode: the target the update merges. Port mode: the newest commit of the plan; after the update, every upstream commit up to it is taken, decided, or left pending by the plan. A port target is not merged, so it is not an ancestor of `GIT_INTAKE_HEAD`. |
| `GIT_INTAKE_UPSTREAM_<NAME>_PREVIOUS` | Follow mode only: the last synced commit before the update; absent on a first import or when the history names none. A port upstream has no such commit; its journal names the commit the port started from. |

`<NAME>` is the upstream's name in upper case, with every character other than a letter or a digit replaced by `_`: `app-a` gives `GIT_INTAKE_UPSTREAM_APP_A_TARGET`. Two upstream names that give the same `<NAME>`, such as `app-a` and `app.a`, are refused when the configuration is read. Every commit named stays in the repository while `verify` runs: the base and the head are on the update branch, and an upstream's commits are in the history of `refs/git-intake/upstreams/<name>`, which the update holds. Variables with the `GIT_INTAKE_` prefix that `git intake` itself inherits are not passed on, so a check sees only those of the update it verifies; git commands the tool runs itself never see them.

The target is a baseline for the tree only for a follow upstream at the repository root. A path upstream's target has the upstream's own layout, with its files at its root rather than under the path, so a check that compares trees with it has to compare the path's subtree, or measure the upstream on its own.

A fork that follows one upstream at the root, named `upstream`, can give an audit the upstream's target as the baseline it takes from `FALLOW_AUDIT_BASE`:

```toml
checks = [
    "pnpm test",
    'FALLOW_AUDIT_BASE="$GIT_INTAKE_UPSTREAM_UPSTREAM_TARGET" fallow audit',
]
```

A script that the repository's own CI also runs can fall back to its usual baseline outside an update: `"${GIT_INTAKE_UPSTREAM_UPSTREAM_TARGET:-$(git merge-base origin/main HEAD)}"`.

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

With `via = "merge-request"`, nobody needs to push to the integration branch, so it can stay protected. `finish` applies the same checks as above, with the publish remote's integration branch as the integration tip: it fetches it, and when it moved past the base, catches up with it and verifies the result first. It then publishes the verified commit itself, without moving any local branch:

1. It pushes the verified commit, without force, to the branch `git-intake/update` on the publish remote, the name of the local update branch. Before the first push of an update, that branch must be absent where each push URL points, or at a commit the base contains, as one left by a merge request merged with its branch kept, or at a commit the update head contains, as one left by this update's own interrupted first push; this is checked before any catch-up or verification; otherwise `finish` pushes nothing, refuses, and names the branch and the push URL, since the branch may carry a merge request the tool knows nothing of. Two clones that update the same repository at the same time share the branch: the first push wins, and the other is rejected as not a fast-forward. When the remote's integration branch already contains the verified commit, `finish` completes the update as after a merge, and deletes a `git-intake/update` that an interrupted first push left at that commit.
2. With the same push it asks the remote to open a merge request from that branch into the integration branch, through git push options (`merge_request.create`, `merge_request.target`, `merge_request.title`, `merge_request.description`, `merge_request.remove_source_branch`, `merge_request.squash=false`), which GitLab applies while it receives the push. The merge request is opened with squashing off whatever the project's default; GitLab before 17.2 ignores that option and applies its default. The title names the updated upstreams. The description lists each follow upstream's merged range with the upstream tag of its target, each port upstream's taken commits, skipped commits and commits already present, and the verified commit. The tool shows what the remote replies, such as the address of the new merge request, and records the merge request's number from the address GitLab gives under `View merge request for <branch>:` for the pushed branch (`.../-/merge_requests/<number>`); `status` shows it as `!<number>`.
3. It removes the update worktree, keeps the update in the step `waiting_for_merge_request`, and exits with status 0. `status` shows the merge request branch and the next command; `start` refuses while the update waits, and `continue` and `verify` work only on a catch-up the update makes meanwhile.

A remote applies push options only to a push that updates the branch. When the branch is already on the remote with the verified commit, as after an interrupted `finish`, the tool finds it there (looking where pushes go, which a remote's `pushurl` may set apart from where it fetches), pushes nothing and asks for no merge request; it then says so and names the merge request to open unless one is open. A remote that does not support push options refuses them before anything is written (`the receiving end does not support push options`); the tool then pushes the branch without options and names the merge request to open: from `git-intake/update` into the integration branch, with squashing off.

An update that waits for a merge request opened by a release of state format 4 keeps its branch `git-intake/update-<commit>`, named after the first twelve hex digits of its verified commit; `finish` and `abort` act on that branch, and a catch-up pushes to it.

Merge the merge request **with a merge commit or by fast-forward**, and leave its branch as it is. What lands must be one of the update's verified commits pushed to the branch itself, or a merge of it whose tree is that commit's tree. A squash or a rebase replaces the verified commit with commits that were never verified, and a rebase also rewrites the upstream merges and copies. A commit added to the merge request branch, or a merge commit made after other work landed on the integration branch, gives a tree that was never verified. In GitLab, set the project's merge method to *Merge commit with semi-linear history* or *Fast-forward merge*, which merge only a branch that contains the current target, and leave squashing off for these merge requests. With the plain *Merge commit* method, merge only while the integration branch is still at the update's base; when it moved, run `finish` first, which catches the merge request up. Where the project's or the target branch rule's squash setting is *Require*, GitLab squashes every merge request, whatever the merge request asks for, so such a project cannot take updates through merge requests: any other squash setting works, or publish by push. The tool cannot read that setting through git; it refuses the squash once it has landed, and says why.

Then run `finish` again. It looks up the merge request's head and branch on the publish remote, fetches the remote's integration branch, finds on its first-parent line the commit that brought a pushed commit in, and acts on the first of the outcomes below that holds. The update keeps every commit it pushed to the branch: the verified commit of the first push and each catch-up merge, so a merge request merged before the integration branch moved is recognised whichever of them it carried.

The merge request's head decides what was merged. GitLab keeps it as `refs/merge-requests/<number>/head`, which follows every push to the merge request branch and stays after the branch is removed. A merge request whose head is not the verified commit did not land the verified tree, whichever merge method was used: a commit added to the branch and merged by fast-forward leaves the verified commit on the integration branch, but with an unverified commit on top. The merge request branch counts as well, as listed now or, once it is gone, as the tool last saw it (when it pushed the branch, or at a later `finish`): if the integration branch took it in at another commit than the verified one, the update is refused even when the merge request's head matches. Without a number, or on a remote that keeps no such reference, the branch is all the tool can judge by. That fallback cannot tell a commit added to the branch after it was last seen and merged by fast-forward from work that landed on the integration branch afterwards: when it completes such a fast-forward, `finish` lists the commits above the verified commit that it could not attribute, so that they can be checked.

- **The verified tree landed.** The integration branch received one of the pushed commits, by fast-forward or through a merge whose tree is that commit's tree and whose parents it contains, and the merge request's head is that commit. `finish` fast-forwards the local integration branch to the remote one, with the same care for its checkouts as in push mode, deletes the merge request branch if it still exists, and removes the update.
- **Another head landed.** The integration branch received one of the pushed commits, but the merge request's head was another commit. `finish` refuses and moves no branch; that merge was never verified. `abort` then ends the update (below).
- **Merged by squash or rebase.** The integration branch carries, without any of the pushed commits, a commit with a pushed commit's tree, or the update's sync records or copy trailers. `finish` refuses, moves no branch, makes no catch-up and pushes nothing, even while the branch and the head still hold the verified commit. Run `abort`, then `start`, which brings the local integration branch up to date: the upstreams are taken in again on the new tip and recorded properly.
- **What landed differs from the verified tree.** A pushed commit came in through a merge with another tree, as when the merge request was merged with a merge commit after other work landed. `finish` refuses and moves no branch, and `abort` ends the update.
- **Unpublished local commits.** The integration branch moved forward from the tip the update last took, but lacks commits of the local integration branch that the base holds and that were never published. `finish` refuses, names those commits, and names `abort`; publish them or reconcile the local integration branch with the remote, then `start` again.
- **The integration branch was rewritten.** It no longer contains the base otherwise; `finish` refuses and names `abort`.
- **The merge request branch is gone and nothing landed.** The merge request was closed or its branch deleted; `finish` refuses, and `abort` ends the update.
- **The merge request's head or branch moved.** It no longer points at the verified commit: `finish` says so and exits with status 1, because merging it would not land the verified tree, so the update cannot complete until someone restores the branch or runs `abort`.
- **The integration branch moved past the base.** Other work landed before the merge. `finish` checks out the update branch in the update worktree again, catches up with the remote's integration branch (see *Catching up with a moved integration branch*), and verifies the catch-up merge. Just before pushing, it checks again that the merge request's head and branch and the remote's integration branch are as it fetched them. It then pushes the catch-up merge, without force, to the same branch, with push options that bring the merge request's title and description up to date and ask for no new merge request (no `merge_request.create`), so a push that races with the merge cannot open a second one. The open merge request then carries the new verified commit, the update keeps waiting for it, and `finish` exits with status 0. A conflict or failed checks stop `finish` with status 1 and push nothing: the merge request keeps its head, and the update keeps waiting for it in the catch-up step, where `continue`, `verify` and `abort` work. When the merge request is merged during the catch-up, before the push, `finish` pushes nothing, and since that merge could not land a verified tree on the moved integration branch, it refuses as above.
- **Nothing landed yet.** `finish` reports that the update waits for the merge request and what to do next, changes nothing, and exits with status 0: waiting is the expected step, not a failure. A script that needs to know whether work is pending runs `status --check`, which fails until the update completes because its upstreams are not current yet, or reads `update.step` from `status --json`.

Without the forge's API, a merge request closed with its branch kept looks open: `finish` keeps waiting, and `abort` deletes the branch and names the merge request to close.

`finish --no-publish` keeps its meaning in this mode: it makes the local move and pushes nothing. Before the merge request is opened, it closes the update locally as in push mode. While the update waits, it refuses with status 2: the verified commit is already published on the merge request branch, and before the merge there is no local move to make, so it would close nothing. Merge the merge request and run `finish --no-publish` again, or withdraw it with `abort`. After the merge, it fast-forwards the local integration branch, leaves the merge request branch in place, and removes the update.

### Abort

`abort` removes the worktree, the update branch, the plan, the update's upstream references, its logs and the state at any step before `finish` moves the integration branch, including a catch-up step. The integration branch stays where `start`, or the last fast-forward to the publish remote by `start` or `finish`, left it; a port's journal is unchanged. Resolutions already made by hand in a stopped merge or copy are recorded first, so a retry reapplies them. A worktree that has `git-intake/update` checked out anywhere other than where `start` places the update worktree belongs to the user: `abort`, `start` and `doctor` name it and leave it for the user to remove or switch.

While the update waits for its merge request, `abort` first checks the publish remote's integration branch as `finish` does:

- If the verified tree landed, `abort` refuses and names `finish`, which completes the update.
- If a merge that did not land the verified tree is there, `abort` removes the update and says that the integration branch holds an unverified merge.
- Otherwise it removes the update and names the merge request that is left to close on the forge.

In each case where it proceeds, it deletes the merge request branch from the publish remote only while that branch still points at one of the commits the update pushed. A branch that is already gone counts as deleted; a branch that received other commits is left alone.

### The update state

The update state lives in `<git-common-dir>/git-intake/state.json`, which every worktree shares and no commit can carry. Its format has a version, and the state records the release that started the update. This release reads a state of format 4 in place, including an update that waits for its merge request; the first command that changes such an update writes it in format 5 and says that releases before v0.3.6 can no longer carry it on, and the first of `continue`, `verify` and `finish` warns once that an earlier release started the update. A state in any other format, older or newer, is refused by every command that reads the update: it names both formats and the release that started the update when the state records it, and advises finishing or aborting the update with a release that keeps that format. For format 3 it names `migrate` first (see *Migrating an update of format 3*). The formats belong to these releases:

| Format | Releases |
| --- | --- |
| 1 | v0.1.0 |
| 2 | v0.2.0 to v0.2.2 |
| 3 | v0.3.0 to v0.3.1 (the update worktree in the git directory) |
| 4 | v0.3.2 to v0.3.5 (no catch-up; the merge request branch named after the verified commit) |
| 5 | v0.3.6 and later |

A newer format is refused with the advice to use the release that started the update, or a later one that reads its format. Another release with the same format carries the update on; the first of `continue`, `verify` and `finish` that it runs warns once that a different release started it. Before `finish`, the warning advises `abort` and a new `start` to use one release throughout; once `finish` has moved a branch or opened the merge request, it gives no such advice, because `abort` would withdraw what is already published.

### Migrating an update of format 3

`migrate` carries an update in progress over from state format 3 to the current format, in place. An update of format 4 needs no migration; `migrate` refuses it and says that this release carries it on. The update keeps its branch, commits, merge in progress, index, uncommitted and untracked files, plan, logs and references, so a conflict resolution or edit made in the update worktree is not lost. Releases of format 3 kept the update worktree at `<git-common-dir>/git-intake/worktree`, where compilers, bundlers and watchers ignore every path, so checks run there may not see the tree as it is. `migrate` moves it with `git worktree move` to `.worktrees/git-intake/update` in the main worktree, applying the placement rules of `start`, including the `info/exclude` line, and then writes the state. The directory moves, so stop the processes, shells and editors that use the update worktree before running `migrate`, and open the new path afterwards.

A verification recorded before the move is dropped, because its checks ran in the old location; run `verify` again. Once `finish` has begun, by planning its moves or pushing the merge request branch, the verification is kept, because it is the record of the commit being published. A bare repository keeps its update worktree in the git directory, so only its state is rewritten. Its verification is kept, unless the path of its update worktree contains `/.git`, the placement `start` refuses.

`migrate` refuses, before it changes anything, when no update is in progress, when the state is in a format other than 3, when the main worktree cannot be located, when `.worktrees` or `.worktrees/git-intake` is a symbolic link or not a directory, when `.worktrees/git-intake/update` already exists, or when git refuses the move, for example for a locked worktree or one that contains submodules; it then gives git's reason. If `migrate` is interrupted after the move, running it again finds the worktree at its new place and completes the state. An update worktree moved to `.worktrees/git-intake/update` without git, while git still records the old path, is recognised by its `.git` file: `migrate` runs `git worktree repair` on it and goes on. Anything else at that path is refused; `migrate` never removes it.

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

Every commit it copies in port mode keeps the upstream commit's message and adds `Intake-Upstream: <name>` and `Intake-Taken: <upstream commit>`; the journal commit of a port is titled `sync(<name>): record <n> port decisions` or `sync(<name>): start the port journal at <commit>`.

The tool reads its own history along the **integration line**: the first-parent line of the integration branch, where each merge on it is followed by the commits its second parent brought in along that parent's own first-parent line, as `git rev-list --first-parent <merge>^1..<merge>^2` lists them. An update published by push, or a merge request merged by fast-forward, puts the tool's merges on the first-parent line itself; a merge request merged with a merge commit puts them right after that merge commit. A catch-up merge among those commits, marked by its `Intake-Catch-Up` trailer, is followed in turn by the first-parent line of its second parent, which holds the update it caught up, down to the first commit that the first parent of the merge on the first-parent line contains, at any depth; so an update that caught up twice, or caught up and was merged with a merge commit, counts as if it had been merged straight in. Any other merge counts only at that one level: a branch merged into another branch, by a merge without the trailer, before that one was merged brings in nothing of its own. The trailer is as trustworthy as the sync records beside it: both are trailers the tool writes, and the tool reads history it does not police. A sync merge brings in nothing either: what it merged is the upstream's history.

A follow upstream's **last synced commit** is the target of its newest sync record on the integration line whose target no other of its records' targets contains, so a branch carrying an older record that is merged after a newer one never moves the upstream back. It is not stored anywhere else. An upstream without a record has its last synced commit taken from the history instead, and the decision depends only on the integration branch and that upstream's own branch, never on other upstreams or on which of them are fetched. For a root upstream, such as a fork created by cloning, it is the newest upstream commit the integration branch contains. For a path upstream, it is the second parent of the newest merge on the integration line whose second parent belongs to the upstream's branch and which changes the upstream's path. That is the merge `git subtree add` or `git subtree pull` leaves, so ancestry shared through a template does not count, and the import still counts when another upstream later merges this one's history. `status` says so in text, and `status --json` marks the source as `history` instead of `record`. The first update writes a record. Only this repository's own records count. Records inside a configured upstream's fetched branch are skipped; this happens, for example, when a stacked branch starts from another fork's integration branch that carries that fork's records. If a follow upstream's branch no longer contains its last synced commit, the upstream was rewritten: `status` reports it and `start` refuses to update it.

## Commands

| Command | Purpose |
| --- | --- |
| `start [UPSTREAM[=REV]...]` | Starts an update, as described above. A follow upstream named as `UPSTREAM=REV` is taken at the commit REV names instead of its fetched tip (see *Choosing a target*), or without REV, with `tags`, at its newest release (see *Release tags*); a port upstream's plan is limited to the pending commits REV contains. |
| `continue` | Carries out the edited plan of a port upstream, or concludes the merge, copy or catch-up merge stopped on conflicts, and takes in the rest. It also accepts a merge concluded by hand when the commit keeps the sync record, a copy committed by hand when it keeps both copy trailers, and a catch-up merge committed by hand with its trailer. |
| `abort` | Removes the update worktree, branch, plan, upstream references, logs and state. While the update waits for its merge request, also deletes the merge request branch from the publish remote. |
| `verify` | Runs `install` and the checks and records the verified commit. |
| `finish [--no-publish]` | Catches the update up when the integration branch moved, verifying the catch-up merge, then fast-forwards and publishes the verified update by push, or opens its merge request and, run again, catches it up while it waits or completes the update once it is merged. With `--no-publish` it fetches nothing, makes only the local move, closes the update without pushing, and names the unpublished branch. |
| `migrate` | Carries an update in progress of state format 3 over to this release's state format, in place, moving its worktree out of the git directory (see *Migrating an update of format 3*). Stop the processes and editors that use the update worktree first. |
| `status [--check] [--json]` | Fetches every upstream and shows the configuration, then for every upstream its path, mode, branch, fetched tip and one state. A follow upstream also shows its last synced commit and the number of commits the integration branch lacks, with the state `current`, `behind`, `not imported`, `no release`, `path removed`, `rewritten` or `unavailable`; with `tags`, the tip shown and the commits counted are those of the newest release, and `no release` means the branch has none. A port upstream shows its journal's base and its numbers of taken, decided and pending commits, with the state `current`, `pending`, `no base`, `rewritten` or `unavailable`; `no base` means it has no journal and nothing to start one from, and a rewritten one names the commits off its line. `unavailable` means the upstream could not be fetched, its branch is missing, the tags of an upstream with `tags` could not be listed, or, for a port upstream, its journal is invalid; any upstream that cannot be fetched makes `status` exit with status 1. `path removed` means a synced path upstream, from a record or from history, whose path no longer exists on the integration branch; `start` refuses it and `status --check` counts it as drift. When several states apply, `unavailable` comes first, then `rewritten`, then `path removed`. A path upstream is `not imported` until a sync record for it exists or, without one, while its path is absent or holds content that no merge of the upstream brought in; a root upstream is `not imported` while it shares no history with the integration branch. While an update is in progress it also shows the step reached, the base and the base the update started from when a catch-up moved it, the worktree, each queued upstream's target or plan and step, a catch-up merge in conflict or awaiting verification with the tip it catches up with, and the next command. |
| `report [--json]` | For the update in progress, or the one just finished, per upstream. For a follow upstream: the merged range and commit count, the upstream tag of the target, each conflicted file with its resolution source (by hand or by a recorded resolution), and the size of the repository's own changes against that upstream before and after the merge. For a port upstream: the taken commits in the order copied, each with its conflicted files, the git-intake paths its copy left out and, where there are any, the earlier pending commits that change the same files; the commits recorded as present because their copy changed nothing; the skip and have decisions with their notes; and the number of commits left pending. Ranges and sizes are measured from the base the update started from. Each catch-up merge follows with the integration tip it took in and its conflicted files with their resolution source. |
| `log [UPSTREAM...] [--json]` | Fetches the selected upstreams and lists, for a follow upstream, the commits on its branch that the integration branch does not contain, up to the newest release with `tags`, and for a port upstream its pending commits, oldest first. |
| `branches [UPSTREAM...] [--json]` | Fetches every branch of the selected upstreams for the command and lists every branch that holds commits the integration branch does not contain, with their number; for a port upstream, commits a copy names as taken do not count. |
| `doctor [--json]` | Checks the repository and reports each check as pass, fail or warn. It changes nothing but the references of its own fetches, which it removes. Checks: the configuration is valid; the update in progress, if any, holds its references at the tips `start` fetched; no reference under `refs/git-intake/` is left without an update; every upstream's URL can be fetched and its branch exists; every imported path exists on the integration branch; every follow upstream's last synced commit is still contained in its branch; every port upstream's journal is valid and its base and its decided and taken commits that are not retired are still on its branch, naming each one that is not; no git remote names an upstream's repository, with a note when the remote is the one a partial clone fetches missing objects from; no journal belongs to an upstream that is missing or in follow mode; the main checkout is clean; no merge is in progress there; the update state, if any, matches its worktree and branch. It warns about upstreams that still have sync records but were dropped from the configuration. |
| `contribute UPSTREAM[=REV] BRANCH` | Creates the local branch BRANCH at the upstream's fetched tip, or at the commit REV names on its branch, for a contribution to the upstream (see *Contributing to an upstream*). Never pushes. |

`status --check` additionally predicts, with `git merge-tree`, the files a merge of each behind follow upstream's tip, or newest release with `tags`, would conflict in. It creates no worktree, branch or state, and fails unless every upstream is current, so a port upstream with pending commits fails it too. It can run on a schedule and gate a cutover or a port review.

### JSON output

`status`, `report`, `log`, `branches` and `doctor` accept `--json`. They then write one JSON document to standard output, with the same facts as the text form under stable snake_case field names, and exit with the same status. Notes and errors go to standard error. For example, `status --json` carries `publish` (the publish remote or null) and `publish_via` (`push`, `merge_request` or null); while an update waits for its merge request, its `update` carries `step` `waiting_for_merge_request` and `merge_request` with `remote`, `source_branch`, `target_branch`, `commit`, `requested` (whether the push that asked the remote to open the merge request transferred the branch; when false, the merge request is opened by hand) and `number` (the merge request's number from the remote's reply, or null).

Every entry of `status --json` `upstreams` carries `name`, `path`, `url`, `branch`, `mode` (`follow` or `port`), `tip`, `state` (`current`, `behind`, `pending`, `no_base`, `not_imported`, `no_release`, `path_removed`, `rewritten`, `unavailable`) and `detail`. A follow upstream fills `last_synced`, `last_synced_source` (`record`, `history` or null), `last_synced_tag` (the `Intake-Tag` of the newest sync record, or null), `tags` (the configured pattern or null) and `commits_behind`, with `tags` also `release`, `release_tag` and `commits_after_release`, and with `--check` `conflicts`; a port upstream fills `base`, `journal` (whether the journal exists), `taken`, `decided` and `pending`, and when rewritten `off_line`, with `base` (the base when it is off the line, or null) and `commits` (each with `commit` and `recorded_as`: `taken`, `skip` or `have`). Fields that do not apply are null.

Each queued upstream under `update` in `status --json` carries `name`, `mode`, `fetched`, `rev`, `step` (`plan`, `pending`, `conflict`, `merged`) and `conflicts`; a follow upstream adds `target`, `tag` (the upstream tag that names the target, or null), `kind` and `merge_commit`, a port upstream `plan` (the plan file), `commits` (each with `commit`, `subject`, `verb`, `note`, `copy` — `none`, `conflict`, `commit_failed`, `copied` or `empty` — `copied`, `failure`, git's output when the commit of the copy failed, and `left_out`) and `journal_commit`. The update's `step` is `plan` while a plan waits to be edited, `commit_failed` while a copy whose commit failed waits for `continue`, `catch_up_conflict` while a catch-up merge waits for its conflicts to be resolved and `continue`, and `catch_up` while a catch-up merge awaits its verification; in both, `catch_up` gives the catch-up's `tip`, `head_before`, `merge_commit` and `conflicts`, and is null otherwise. `base` is the integration tip the update lands on and `start_base` the one it started from. `report --json` carries `base`, the integration tip the update started from, and `catch_ups`, each with `tip`, `merge_commit` and `conflicts`. Each entry of `report --json` `upstreams` carries `mode` and `rev`; a follow upstream fills `kind`, `range_start`, `target`, `tag`, `commits`, `merge_commit`, `conflicts` and the diff sizes, and a port upstream fills `port` with `taken` (each with `commit`, `subject`, `copy`, `copied`, `conflicts`, `left_out` and `earlier_pending`), `unchanged` (each with `commit`, `subject` and `left_out`), `decisions` (each with `commit`, `subject`, `verdict` and `note`), `left_pending` and `journal_commit`. Each entry of `log --json` and `branches --json` carries `mode`; a follow upstream's entry of `log --json` adds `release` and `release_tag`, null without `tags`.

## Exit status

| Status | Meaning |
| --- | --- |
| 0 | Success. |
| 1 | A negative result: `status --check` found drift or pending port commits, an upstream could not be fetched by `status`, `log` or `branches`, a `doctor` check failed, `verify` failed, or `start`/`continue` stopped on conflicts or on the failed commit of a copy. `finish` stopped on the conflicts or the failed checks of a catch-up merge, or because the integration branch moved again while it verified one, or found that the head or branch of the merge request it waits for no longer points at the verified commit. A pending update is not a negative result: `finish` exits with status 0 when it opens a merge request and while the update waits for it, and `status --check` is what reports the upstreams as not current. |
| 2 | An error: invalid configuration, a refused command (for example `start` with an upstream that cannot be fetched, `continue` with an invalid plan, `finish` on an unverified update, on a merge request merged by squash or without the verified tree, or `finish --no-publish` while the update waits for its merge request), a catch-up refused because the integration branch was rewritten or its new commits change what the update means, a git failure (including a push rejected by the publish remote, after which `finish` retries the push, catching up first when the remote moved, and `finish --no-publish` closes the update locally), or invalid usage. |

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
- `finish` moves no ref unless the verified commit is the worktree head, the base is still the integration tip, and the move is a fast-forward. When the integration branch moved, the update first takes the new tip in by a catch-up merge, which is verified before anything moves to it. It validates the move before making it, and it can be resumed.
- Publication by push pushes exactly the planned commit. Nothing unverified is ever published by a retry, and `finish --no-publish` closes an update that cannot be published.
- Publication through a merge request pushes only verified commits, without force, to the branch `git-intake/update`, which only moves forward. The local integration branch moves only after the publish remote's integration branch received one of them, by fast-forward or by a merge whose tree is that commit's tree. A merge by squash or rebase is refused, and so is a merge request whose head, as the remote keeps it or as the tool last saw its branch, is not the verified commit, or whose merge brought in another tree. Without the merge request's head, commits above a fast-forwarded verified commit cannot be attributed; `finish` lists them.
- The tool never adds or changes a git remote, and never fetches an upstream through one.
- A rewritten upstream is detected from the sync record or the port journal and copies, and is not taken.
- The configuration and a port's journal are read from the committed integration tip, never from a working copy or from the tree being taken in.
