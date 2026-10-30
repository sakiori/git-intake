# git-intake

`git intake` keeps a downstream repository current with one or more upstreams. It takes upstream changes by merge commits, never rewrites history and never force-pushes. The merges happen in a separate worktree. A conflict is resolved once, and the repository's own checks run on exactly the tree that gets published. The integration branch moves only by fast-forward, after verification.

It covers two cases:

- **A fork**: one upstream merged at the repository root. A branch stacked on another fork's integration branch is a fork too, with that branch as its upstream.
- **A monorepo assembled from several repositories**: each source repository is an upstream merged under its own path. They keep being taken in until cutover, and `status --check` gates the cutover.

The tool runs the installed `git` for every repository operation. Each repository describes itself in a committed `.git-intake.toml`.

## Installation

Requires Git 2.43 or later (`git merge-tree -X` is used for conflict prediction) and a POSIX `sh`. The tool is a single binary named `git-intake`; with it on the `PATH`, git runs it as `git intake`.

Releases provide prebuilt binaries for the targets `x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl` and `aarch64-apple-darwin`. The musl binaries are statically linked and run on any Linux distribution and in minimal CI images.

### With mise

Pin the version in the repository's tool configuration, so that an update of the tool is a reviewed change:

```sh
mise use gitlab:sakiori-oss/git-intake@0.1.0
```

mise's GitLab backend picks the release asset for the running platform by its target triple and finds the binary at the archive root without further options.

### Direct download

Each release lists its archives, `git-intake-vX.Y.Z-<target>.tar.gz`, with a `.sha256` checksum next to each. The binary sits at the archive root, and the manual pages under `man/man1/`:

```
git-intake
man/man1/git-intake.1
man/man1/git-intake-<command>.1
```

To download an archive and its checksum directly:

```sh
version=0.1.0
target=x86_64-unknown-linux-musl
file="git-intake-v$version-$target.tar.gz"
base="https://gitlab.com/sakiori-oss/git-intake/-/releases/v$version/downloads/binaries"
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

From a checkout, `cargo install --locked --path .` installs the working tree. `cargo run --locked -p xtask -- man <dir>` writes the manual pages into `<dir>`.

### Help and the manual page

`git intake -h`, and `-h` or `--help` after a command (`git intake finish --help`), print the built-in help. `git intake --help` is different: git turns it into `git help intake`, which opens the manual page `git-intake(1)`, so it works only where `man` finds that page.

`man` finds the pages of a release archive when its `man` directory is on the manual search path. On Linux, man-db derives that path from `PATH`: for every directory on `PATH` it also searches that directory's `man` subdirectory, so the pages are found when the directory holding `git-intake` is on `PATH`, as with `mise activate`. Otherwise, or when `MANPATH` is set, add the directory to `MANPATH`; with mise:

```sh
export MANPATH="$(mise where gitlab:sakiori-oss/git-intake)/man:$MANPATH"
```

After a direct download, extracting the pages into `/usr/local/share` puts them on the default search path.

## Releasing

Versions follow the Conventional Commits on the default branch. In `0.x`, a breaking change raises the minor version and every other change the patch version; from `1.0` on, the usual SemVer rules apply. `CHANGELOG.md` keeps one section per release in the Keep a Changelog format.

1. On an up-to-date `main` with a clean working tree, run `ci/prepare-release.sh`. It needs git-cliff, pinned in `mise.toml`; run `mise install` once. The script does the following:
   - computes the next version from the commits since the last release;
   - sets that version in `Cargo.toml` and `Cargo.lock`;
   - adds the release's section to `CHANGELOG.md`;
   - commits `chore(release): vX.Y.Z` on a new branch `release/vX.Y.Z`.

   It then prints the command that pushes the branch and opens the merge request. Run the script with `--push` to do that directly.
2. Review the release merge request, edit its changelog section if needed, and merge it. Its pipeline builds the binaries for every target.
3. The pipeline of the default branch finds that `vX.Y.Z` has no tag yet. It builds the binaries and their manual pages, uploads the archives to the generic package registry, and creates the release and its tag `vX.Y.Z` at the merged commit, with the changelog section as the release notes. A merge that leaves the version unchanged releases nothing.

Creating the tag requires the person who merges to be allowed to create `v*` tags.

## Configuration

`.git-intake.toml` lives at the repository root and is committed on the integration branch. The tool always reads the version committed at the integration branch tip. An uncommitted edit in the working copy is ignored, and the tool prints a note that the working copy differs. The tool finds the integration branch from the configuration committed at `HEAD`, or from the update in progress, so run it from a checkout of the integration branch or of a branch that carries the configuration.

```toml
integration = "main"                  # the branch that receives upstream merges
install = "pnpm install"              # optional: runs before the checks
checks = ["pnpm lint", "pnpm test"]   # run in order by verify

[publish]                             # optional: where and how finish publishes
remote = "origin"                     # the publish remote
via = "push"                          # optional: "push" (default) or "merge-request"

[[upstream]]
name = "app-a"                        # unique; used on the command line and in sync records
remote = "app-a"                      # unique git remote name
branch = "main"                       # upstream branch to follow
url = "git@example.com:team/app-a.git"  # optional: lets fetch add the remote
path = "apps/app-a"                   # optional: merge under this path; absent = repository root
mirror = "upstream-app-a"             # optional: local branch fast-forwarded to each merged target
```

Validation rules. The tool refuses to run when any of them is broken, and it names the entries involved:

- Unknown keys are rejected.
- At least one `[[upstream]]` is required.
- Upstream names, remotes and mirror branches are unique.
- A path is normalised: no leading or trailing slash, no empty, `.` or `..` segment, no `.git` segment in any letter case, and it may not be empty. Omit `path` for the root.
- Paths may not nest; one upstream's path cannot lie inside another's.
- An upstream at the root must be the only upstream.
- `publish` is a table: `remote` is required, and `via` is `push` or `merge-request`. Write it as the `[publish]` section shown above, after the top-level keys (a section header ends the top level, so keys written below it belong to the section), or inline as `publish = { remote = "origin", via = "merge-request" }`. The string form `publish = "origin"` is rejected with the form to write instead.
- The publish remote may not be an upstream's remote, because the tool never pushes to an upstream.
- A mirror may be neither the integration branch nor `git-intake/update`.

## The update cycle

```sh
git intake start        # fetch, fix targets, merge each upstream with new commits
# on conflict: resolve in the update worktree, `git add`, then
git intake continue     # conclude the merge, merge the remaining upstreams
git intake verify       # run install and checks on the merged result
git intake finish       # fast-forward, publish, clean up
# with via = "merge-request": finish opens a merge request and the update waits;
# once the merge request is merged, run finish again to complete the update
```

Every command works the same from any directory of the repository: the main checkout, a linked worktree, or the update worktree itself. This holds for a submodule and for a repository whose git directory lies outside its checkout as well. Git operations on the repository as a whole, such as fetching, pushing and moving branches, run in the main worktree, so a relative remote URL means what it means there. A revision named to `start` is resolved in the worktree `start` runs in. When `finish` or `abort` removes the update worktree it was run from, the command still completes and notes that the current directory is gone; change to another directory before the next command.

Git cannot locate the main worktree of a repository whose git directory lies outside it, without naming it, from a linked worktree. The tool then finds it in the update in progress, which `start` records. Without an update, run `start` in the main worktree; from elsewhere it refuses, `doctor` warns that it did not check the main checkout, and a relative remote URL is resolved against the git directory.

`start` refuses to begin if any of these hold:

- an update is already in progress;
- the main checkout or a checkout of the integration branch has uncommitted changes to tracked files;
- the integration branch is behind its counterpart on the publish remote;
- a selected upstream is unknown, named more than once, unavailable or rewritten;
- a revision named for an upstream is refused, as described under *Choosing a target* below, or the upstream's remote cannot be asked for its tag;
- a queued upstream's mirror branch holds a commit its target does not contain, so `finish` could not fast-forward the mirror;
- a path upstream that has no sync record yet finds its path holding content unrelated to the upstream's history, or finds a file where a directory of its path would go;
- a path upstream that was imported before finds its path removed on the integration branch.

When `start` runs, it does the following:

1. Fetches the selected upstreams. With no names it selects all of them.
2. Records each fetched tip, or the commit a revision names (see *Choosing a target*), as that upstream's **target**, and the integration tip as the **base**. Targets never change during the update, even if the upstream or a tag moves on.
3. Queues, in configuration order, the upstreams whose target the base does not contain yet. The others are reported as current. If nothing is queued, nothing is created.
4. Creates the worktree `<git-common-dir>/git-intake/worktree` on the branch `git-intake/update` from the base. The main checkout is never touched.
5. Enables `rerere.enabled` for the repository. A resolution recorded in one attempt is reapplied when the same conflict recurs, and the tool reports such files as *resolved by a recorded resolution*. Review those files and stage them like any other resolution.
6. Merges each queued target with its own merge commit:
   - a root upstream: `git merge --no-ff`;
   - a path upstream, first import (no sync record for it yet, and the path is absent): an `ours` merge followed by `git read-tree --prefix=<path>/`. This keeps the upstream's full history. History shared through a common ancestor, such as a template both repositories started from, does not count as imported. The import is all or nothing: if any step fails, the merge is undone and the upstream stays pending;
   - a path upstream, later updates (it has a sync record): `git merge --no-ff -X subtree=<path>`;
   - a path upstream imported by other means, such as `git subtree add` (no sync record, but a merge on the integration branch's first-parent line brought the upstream's history into its path): the same subtree merge, which writes the first sync record. A path that holds content no such merge brought in is refused, even when it shares a common template with the upstream.

   Merges run with `--no-log` so that git configuration cannot push the trailers out of the last paragraph. Before an upstream counts as merged, the tool checks that the commit carries its sync record for the target, that the target is merged, and that a path upstream's path exists. If a hook rewrote the message, the tool stops and says so. Each step can be repeated: after an interruption, `continue` recognises a merge that is already committed and carries its record.

On a conflict the tool stops with exit status 1. It names the upstream, lists the conflicted files, and reports which upstreams are merged and which are pending. A file is labelled *resolved by a recorded resolution* only when git's rerere reports that it resolved it (including files `rerere.autoupdate` already staged); every other conflicted file waits for a resolution by hand. Resolve the files in the worktree, stage them, and run `continue`. If one upstream cannot be resolved now, `abort` and run `start` again with only the other upstreams named. Recorded resolutions make the repeated merges cheap.

`verify` runs `install` and then every check through `sh -c` in the update worktree. Each command's output goes to its own log under `<git-common-dir>/git-intake/logs/`. If `install` fails, the checks are skipped. Verification fails in any of these cases:

- the merged `.git-intake.toml` is missing or invalid, or names a different integration branch (the commands are then skipped);

- a command fails or is skipped;
- the commands change tracked files;
- the update branch moves while the checks run.

When it passes, `verify` records the commit it verified.

`finish` checks the following before moving any ref:

- every upstream is merged;
- the worktree head is the verified commit, so any commit added after `verify` needs a new `verify`;
- the base is still the integration tip. If the integration branch moved, `abort` and `start` again so the update merges the new tip, then verify again;
- the verified commit contains the base and every merged target, and each mirror's current value is contained in its target, so every branch only fast-forwards;
- every checkout of a branch about to move has no uncommitted changes and no untracked file the fast-forward would overwrite.

It then fast-forwards the integration branch to the verified commit. A checkout of that branch is updated with `git merge --ff-only`, and other branches move with `git update-ref` against their old value. Each merged upstream's mirror is fast-forwarded to its target. In push mode (`via = "push"`, the default), `finish` pushes these branches to the publish remote with one atomic push and without force, so either every branch is published or none is. Finally it removes the worktree, the update branch and the state.

`finish` saves its planned branch moves in the update state before it moves anything, and `status` shows the step as `finishing`. Running `finish` again after an interruption completes the remaining moves; a branch already at its planned value, or at a commit containing it, counts as done. It then publishes and cleans up. Once a branch has moved, `abort` refuses and points to `finish` and `finish --no-publish`.

The push sends exactly the planned commits (`<commit>:refs/heads/<branch>`), never whatever a local branch points to by then. A commit added to the integration branch while the update is finishing stays local.

If the push is rejected, nothing is published, the local branches stay advanced, and the update stays in the `finishing` step. Running `finish` again retries the same push. If the remote holds work the local branches lack, or never accepts the push (for example a protected branch), run `finish --no-publish`. It closes the update locally: it completes any remaining local moves with the same checks, removes the worktree, the update branch and the state, and lists the branches it left unpublished (also in `report`). Then merge the remote's work and publish the result through a new verified update or the repository's usual review. `--no-publish` also works on a verified update before any push attempt.

### Choosing a target

By default an update takes each upstream at the tip it just fetched. To take an upstream at a reviewed point instead, such as a release tag, name it with a revision:

```sh
git intake start upstream=v2.3              # a fork: take the upstream's tag v2.3
git intake start app-a=1a2b3c4d app-b       # app-a at a commit, app-b at its fetched tip
```

The name ends at the first `=`, since an upstream name cannot contain one, and the rest is the revision. Upstreams named without a revision take their fetched tip, and naming an upstream twice is refused. The revision is resolved as follows:

- If the upstream's remote has a tag of that name, the tag's commit is the target, peeled through an annotated tag. A leading `refs/tags/` or `tags/` is dropped first, so `v2.3`, `tags/v2.3` and `refs/tags/v2.3` name the same tag. Looking the tag up contacts the upstream's remote: the tool asks it for that one tag with `git ls-remote`, after the fetch, whenever the revision is a valid tag name, including a commit id. No tag is fetched, so the repository's own tags never clash with the upstream's. If the remote cannot be asked, `start` refuses and names the upstream, the remote and git's error.
- Otherwise the revision is resolved in the repository, like any argument to `git rev-parse`: a full or abbreviated commit id, or an expression such as `upstream/main~3`. A revision that resolves through one of the repository's own tags, by itself or at the start of an expression such as `v2.3~1`, is refused: the repository's tags are not the upstream's, so a tag `v2.3` in the repository never stands in for an upstream tag `v2.3`, even when the upstream has none. The tool looks for that tag by name, so it is found even when a branch has the same name.

Only a plain tag name is looked up on the remote. An expression is always resolved in the repository, where upstream tags never are, so an expression on a tag only the upstream has, such as `v2.3~1`, names no commit and is refused; name the commit, or an expression on the fetched branch such as `upstream/main~3`, instead.

The commit, however it was named, must be on the fetched upstream branch: the tool syncs a branch, not arbitrary commits. With `L` the upstream's last synced commit, `start` then decides:

- a target older than `L` is refused, because an update never moves an upstream back;
- a target the integration branch already contains, such as `L` itself, is reported as current;
- a target that does not contain `L`, such as a commit of a side branch that the upstream merged after `L`, is refused, because the last synced commit and the mirror branch only move forward;
- a target that does not contain the upstream's mirror branch, where one exists, is refused for the same reason: `finish` could only fast-forward the mirror to a target that contains it. This also holds for an update without a revision;
- otherwise the upstream is queued at that target.

The rest of the update does not change: the sync record names the target, `finish` fast-forwards the mirror branch to it, and `status` and `report` show the revision next to the target. Afterwards `status` shows the upstream as `behind`, by the commits between the target and the fetched tip.

### Publication through a merge request

With `via = "merge-request"`, nobody needs to push to the integration branch, so it can stay protected. `finish` applies the same checks as above and then publishes the verified commit itself, without moving any local branch:

1. It pushes the verified commit, without force, to a new branch `git-intake/update-<commit>` on the publish remote, where `<commit>` is the first twelve hex digits of the verified commit.
2. With the same push it asks the remote to open a merge request from that branch into the integration branch, through git push options (`merge_request.create`, `merge_request.target`, `merge_request.title`, `merge_request.description`, `merge_request.remove_source_branch`), which GitLab applies while it receives the push. The title names the merged upstreams, and the description lists each upstream's merged range and the verified commit. The tool shows what the remote replies, such as the address of the new merge request, and records the merge request's number from the address GitLab gives under `View merge request for <branch>:` for the pushed branch (`.../-/merge_requests/<number>`); `status` shows it as `!<number>`.
3. It removes the update worktree and keeps the update in the step `waiting_for_merge_request`. `status` shows the merge request branch and the next command; `start`, `continue` and `verify` refuse while the update waits.

A remote applies push options only to a push that updates the branch. When the branch is already on the remote with the verified commit, as after an interrupted `finish`, the push changes nothing and asks for no merge request; the tool then says so and names the merge request to open unless one is open. A remote that does not support push options refuses them before anything is written (`the receiving end does not support push options`); the tool then pushes the branch without options and names the merge request to open: from `git-intake/update-<commit>` into the integration branch.

Merge the merge request **with a merge commit or by fast-forward**, and leave its branch as it is. What lands must be the verified commit itself, or a merge of it whose tree is the verified tree. A squash or a rebase replaces the verified commit with commits that were never verified, and a rebase also rewrites the upstream merges. A commit added to the merge request branch, or a merge commit made after other work landed on the integration branch, gives a tree that was never verified. In GitLab, set the project's merge method to *Merge commit with semi-linear history* or *Fast-forward merge*, which merge only a branch that contains the current target, and leave squashing off for these merge requests. With the plain *Merge commit* method, merge only while the integration branch is still where the update started.

Then run `finish` again. It looks up the merge request's head and branch on the publish remote, fetches the remote's integration branch, finds on its first-parent line the commit that brought the verified commit in, and acts on what it finds.

The merge request's head decides what was merged. GitLab keeps it as `refs/merge-requests/<number>/head`, which follows every push to the merge request branch and stays after the branch is removed. A merge request whose head is not the verified commit did not land the verified tree, whichever merge method was used: a commit added to the branch and merged by fast-forward leaves the verified commit on the integration branch, but with an unverified commit on top. The merge request branch counts as well, as listed now or, once it is gone, as the tool last saw it (when it pushed the branch, or at a later `finish`): if the integration branch took it in at another commit than the verified one, the update is refused even when the merge request's head matches. Without a number, or on a remote that keeps no such reference, the branch is all the tool can judge by. That fallback cannot tell a commit added to the branch after it was last seen and merged by fast-forward from work that landed on the integration branch afterwards: when it completes such a fast-forward, `finish` lists the commits above the verified commit that it could not attribute, so that they can be checked.

- **The verified tree landed.** The merge request's head is the verified commit, and the commit that brought it in is the verified commit itself (a fast-forward) or a merge whose tree is the verified tree and whose parents the verified commit contains. `finish` fast-forwards the local integration branch to the remote one, with the same care for its checkouts as in push mode, fast-forwards each merged upstream's mirror branch to its target and pushes the mirrors without force in one atomic push, deletes the merge request branch if it still exists, and removes the update. If the mirror push is rejected, the update stays in its `finishing` step: run `finish` again to retry, or `finish --no-publish` to close the update locally and publish the mirrors by hand.
- **Nothing landed yet, and the merge request branch exists.** `finish` reports that the update waits for the merge request, changes nothing, and exits with status 1. If the merge request's head or branch no longer points at the verified commit, it says so: merging it would not land the verified tree.
- **What landed differs from the verified tree.** The integration branch contains the verified commit, but the merge request's head was another commit, or the verified commit came in through a merge with another tree. `finish` refuses and moves no branch; that merge was never verified. `abort` then ends the update (below).
- **The integration branch changed without receiving the verified commit.** When a commit with the verified tree landed instead, the merge request was merged by squash or rebase; otherwise another change landed first, or the merge request was merged by squash or rebase after other changes. `finish` refuses and moves no branch. Run `abort`, bring the local integration branch up to date with the remote one, and run `start`: the upstreams are merged again on the new tip and recorded properly.
- **The merge request branch is gone and nothing landed.** The merge request was closed or its branch deleted; `finish` refuses, and `abort` ends the update.

`finish --no-publish` keeps its meaning in this mode: it makes the local moves and pushes nothing. Before the merge request is opened, it closes the update locally as in push mode. While the update waits, it moves no branch before the merge request is merged, and reports that the update waits. After the merge, it fast-forwards the local integration and mirror branches, leaves the mirrors unpublished and the merge request branch in place, and removes the update.

### Abort

`abort` removes the worktree, the update branch, the update's logs and the state at any step before `finish` moves a branch. The integration and mirror branches stay exactly as they were, and the message names the branches the configuration has. Resolutions already made by hand are recorded first, so a retry reapplies them.

While the update waits for its merge request, `abort` first checks the publish remote's integration branch as `finish` does:

- If the verified tree landed, `abort` refuses and names `finish`, which completes the update.
- If a merge that did not land the verified tree is there, `abort` removes the update, leaves the mirror branches unchanged, and says that the integration branch holds an unverified merge.
- Otherwise it removes the update and names the merge request that is left to close on the forge.

In each case where it proceeds, it deletes the merge request branch from the publish remote only while that branch still points at the verified commit. A branch that is already gone counts as deleted; a branch that received other commits is left alone.

## The sync record

Every merge commit the tool creates looks like this:

```
sync(app-a): merge app-a/main at 1a2b3c4

Intake-Upstream: app-a
Intake-Target: <full commit id>
```

An upstream's **last synced commit** is the target in the newest such commit on the integration branch's first-parent line. It is not stored anywhere else. An upstream without a record has its last synced commit taken from the history instead, and the decision depends only on the integration branch and that upstream's own branch, never on other upstreams or on which of them are fetched. For a root upstream, such as a fork created by cloning, it is the newest upstream commit the integration branch contains. For a path upstream, it is the second parent of the newest merge on the integration branch's first-parent line whose second parent belongs to the upstream's branch and which changes the upstream's path. That is the merge `git subtree add` or `git subtree pull` leaves, so ancestry shared through a template does not count, and the import still counts when another upstream later merges this one's history. `status` says so in text, and `status --json` marks the source as `history` instead of `record`. The first update writes a record. Only this repository's own records count. Records inside a configured upstream's fetched branch are skipped; this happens, for example, when a stacked branch starts from another fork's integration line that carries that fork's records. The tool's merges always stay on the first-parent line because `finish` only fast-forwards to the update branch, where each merge takes the previous head as its first parent. If an upstream's branch no longer contains its last synced commit, the upstream was rewritten: `status` reports it and `start` refuses to update it.

## Commands

| Command | Purpose |
| --- | --- |
| `fetch [UPSTREAM...]` | Adds missing remotes from configured URLs and fetches the selected upstreams, reporting each one. An upstream whose existing remote has a different URL, or whose remote is missing with no URL configured, is refused: nothing is fetched from it, and the others are still fetched. Any refusal or failed fetch exits with status 1. The fetch uses `--prune --no-tags`. `start` refuses outright if a selected upstream cannot be fetched. |
| `start [UPSTREAM[=REV]...]` | Starts an update, as described above. An upstream named as `UPSTREAM=REV` is taken at the commit REV names instead of its fetched tip (see *Choosing a target*). |
| `continue` | Concludes the merge stopped on conflicts and merges the remaining queued upstreams. It also accepts a merge concluded by hand when the commit keeps the sync record. |
| `abort` | Removes the update worktree, branch, logs and state. While the update waits for its merge request, also deletes the merge request branch from the publish remote. |
| `verify` | Runs `install` and the checks and records the verified commit. |
| `finish [--no-publish]` | Fast-forwards and publishes the verified update by push, or opens its merge request and, run again once it is merged, completes the update. With `--no-publish` it makes only the local moves, closes the update without pushing, and lists the unpublished branches. |
| `status [--check] [--json]` | Shows the configuration, then for every upstream its path, branch, last synced commit, fetched tip, the number of commits the integration branch lacks, and one state: `current`, `behind`, `not imported`, `path removed`, `rewritten` or `unavailable`. `path removed` means a synced path upstream, from a record or from history, whose path no longer exists on the integration branch; `start` refuses it and `status --check` counts it as drift. When several states apply, `unavailable` comes first, then `rewritten`, then `path removed`. A path upstream is `not imported` until a sync record for it exists or, without one, while its path is absent or holds content that no merge of the upstream brought in; a root upstream is `not imported` while it shares no history with the integration branch. While an update is in progress it also shows the step reached, the base, the worktree, each queued upstream's target and step, and the next command. Without `--check` it uses the remote-tracking refs as last fetched. |
| `report [--json]` | For the update in progress, or the one just finished: per upstream, the merged range and commit count, each conflicted file with its resolution source (by hand or by a recorded resolution), and the size of the repository's own changes against that upstream before and after the merge. |
| `log [UPSTREAM...] [--json]` | Lists the commits on each fetched upstream branch that the integration branch does not contain. |
| `branches [UPSTREAM...] [--json]` | Lists every branch of each upstream remote that holds commits the integration branch does not contain, with the commit count. |
| `doctor [--json]` | Checks the repository without changing it, and reports each check as pass, fail or warn. Checks: the configuration is valid; every remote exists with the configured URL; every upstream branch exists; every imported path exists on the integration branch; every last synced commit is still contained in its branch; the main checkout is clean; no merge is in progress there; the update state, if any, matches its worktree and branch. It warns about upstreams that still have sync records but were dropped from the configuration. |

`status --check` first fetches every upstream, adding missing remotes. It then predicts, with `git merge-tree`, the files a merge of each behind upstream would conflict in. It creates no worktree, branch or state, and fails unless every upstream is current.

### JSON output

`status`, `report`, `log`, `branches` and `doctor` accept `--json`. They then write one JSON document to standard output, with the same facts as the text form under stable snake_case field names, and exit with the same status. Notes and errors go to standard error. For example, `status --json` carries `publish` (the publish remote or null) and `publish_via` (`push`, `merge_request` or null); while an update waits for its merge request, its `update` carries `step` `waiting_for_merge_request` and `merge_request` with `remote`, `source_branch`, `target_branch`, `commit`, `requested` (whether the push that asked the remote to open the merge request transferred the branch; when false, the merge request is opened by hand) and `number` (the merge request's number from the remote's reply, or null). Every entry of `status --json` `upstreams` carries `name`, `path`, `remote`, `branch`, `url`, `mirror`, `last_synced`, `last_synced_source` (`record`, `history` or null), `tip`, `commits_behind`, `state` (`current`, `behind`, `not_imported`, `path_removed`, `rewritten`, `unavailable`), `detail`, and, with `--check`, `conflicts`. Each queued upstream under `update` in `status --json`, and each entry of `report --json` `upstreams`, carries `target` and `rev`: the revision `start` was given for the target, or null for the fetched tip.

## Exit status

| Status | Meaning |
| --- | --- |
| 0 | Success. |
| 1 | A negative result: `status --check` found drift, a `doctor` check failed, `verify` failed, an upstream could not be fetched or its remote was refused, `start`/`continue` stopped on conflicts, or `finish` found that the update still waits for its merge request. |
| 2 | An error: invalid configuration, a refused command (for example `finish` on an unverified update, or on a merge request merged by squash or without the verified tree), a git failure (including a push rejected by the publish remote, after which `finish` retries the same push and `finish --no-publish` closes the update locally), or invalid usage. |

## Monorepo migration

1. Commit a configuration that lists one `[[upstream]]` per source repository, each with its own `path` and `url`.
2. Run `git intake start`. Each source is imported under its path with its full history, then verified and finished like any update. A path imported earlier with `git subtree add` is adopted by its first update. A path that holds content unrelated to the source's history is refused, and so is an import that would replace a file.
3. Until cutover, keep taking the sources in with the same cycle. Conflicts against the monorepo's new layout are resolved once and recorded. Files moved within an upstream's path merge as renames. Files moved out of the path may conflict or arrive at their old location; resolve them like any other conflict.
4. Run `git intake status --check` on a schedule. It fails whenever a source has moved on and lists the files that would conflict. Zero drift, meaning every upstream is current, is the cutover gate.
5. Before freezing the sources, run `git intake branches` to find work left on other branches of each source. It reports when no upstream holds unmerged work.
6. After cutover, remove an upstream from the configuration. `doctor` keeps warning about upstreams that are dropped but still have sync records, so the removal is visible.

## Safety properties

- The tool never rewrites history and never force-pushes. Local branches only fast-forward.
- It never pushes to an upstream remote. The publish remote may not be an upstream's remote.
- Nothing is written to the main working tree. The update lives in its own worktree, on `git-intake/update`, with its state in `<git-common-dir>/git-intake/`, which every worktree shares and no commit can carry.
- Each target is fixed when the update starts. What is merged, verified and published is the same commit.
- `finish` moves no ref unless the verified commit is the worktree head, the base is still the integration tip, and every move is a fast-forward. It validates every move before making any, and it can be resumed.
- Publication by push is a single atomic push of exactly the planned commits. If any branch is rejected, none moves on the remote; nothing unverified is ever published by a retry, and `finish --no-publish` closes an update that cannot be published.
- Publication through a merge request pushes only the verified commit, to a branch of its own. The local integration branch moves only after the publish remote's integration branch received the verified commit, by fast-forward or by a merge whose tree is the verified tree, and the mirror branches follow only then. A merge by squash or rebase is refused, and so is a merge request whose head, as the remote keeps it or as the tool last saw its branch, is not the verified commit, or whose merge brought in another tree. Without the merge request's head, commits above a fast-forwarded verified commit cannot be attributed; `finish` lists them.
- An existing remote is never retargeted.
- A rewritten upstream is detected from the sync record and is not merged.
- The configuration is read from the committed integration tip, never from a working copy or from the tree being merged.
