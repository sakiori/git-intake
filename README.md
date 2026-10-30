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

mise's GitLab backend picks the release asset for the running platform by its target triple.

### Direct download

Each release lists its archives, `git-intake-vX.Y.Z-<target>.tar.gz`, with a `.sha256` checksum next to each. The binary sits at the archive root. They can be downloaded directly:

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

### From source

Contributors with a Rust toolchain can build any revision:

```sh
cargo install --locked --git https://gitlab.com/sakiori-oss/git-intake.git --tag v0.1.0
```

From a checkout, `cargo install --locked --path .` installs the working tree.

## Releasing

1. Set the new version in `Cargo.toml`, run `cargo build` so that `Cargo.lock` follows, and merge the change.
2. Tag that commit `vX.Y.Z` with the same version and push the tag.

Every pipeline builds the release archives for all targets. On a tag, the pipeline also does the following:

1. It refuses a tag that differs from `v` plus the crate version.
2. It uploads the archives and checksums to the project's generic package registry.
3. It creates the release with links to them.

`git intake --version` reports the crate version, so a released binary always names its release.

## Configuration

`.git-intake.toml` lives at the repository root and is committed on the integration branch. The tool always reads the version committed at the integration branch tip. An uncommitted edit in the working copy is ignored, and the tool prints a note that the working copy differs. The tool finds the integration branch from the configuration committed at `HEAD`, or from the update in progress, so run it from a checkout of the integration branch or of a branch that carries the configuration.

```toml
integration = "main"                  # the branch that receives upstream merges
publish = "origin"                    # optional: remote to push to after finish
install = "pnpm install"              # optional: runs before the checks
checks = ["pnpm lint", "pnpm test"]   # run in order by verify

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
- `publish` may not be an upstream's remote, because the tool never pushes to an upstream.
- A mirror may be neither the integration branch nor `git-intake/update`.

## The update cycle

```sh
git intake start        # fetch, fix targets, merge each upstream with new commits
# on conflict: resolve in the update worktree, `git add`, then
git intake continue     # conclude the merge, merge the remaining upstreams
git intake verify       # run install and checks on the merged result
git intake finish       # fast-forward, publish, clean up
```

`start` refuses to begin if any of these hold:

- an update is already in progress;
- the main checkout or a checkout of the integration branch has uncommitted changes to tracked files;
- the integration branch is behind its counterpart on the publish remote;
- a selected upstream is unknown, unavailable or rewritten;
- a path upstream that has no sync record yet finds its path holding content unrelated to the upstream's history, or finds a file where a directory of its path would go;
- a path upstream that was imported before finds its path removed on the integration branch.

When `start` runs, it does the following:

1. Fetches the selected upstreams. With no names it selects all of them.
2. Records each fetched tip as that upstream's **target** and the integration tip as the **base**. Targets never change during the update, even if the upstream moves on.
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

It then fast-forwards the integration branch to the verified commit. A checkout of that branch is updated with `git merge --ff-only`, and other branches move with `git update-ref` against their old value. Each merged upstream's mirror is fast-forwarded to its target. When `publish` is set, `finish` pushes these branches with one atomic push and without force, so either every branch is published or none is. Finally it removes the worktree, the update branch and the state.

`finish` saves its planned branch moves in the update state before it moves anything, and `status` shows the step as `finishing`. Running `finish` again after an interruption completes the remaining moves; a branch already at its planned value, or at a commit containing it, counts as done. It then publishes and cleans up. Once a branch has moved, `abort` refuses and points to `finish` and `finish --no-publish`.

The push sends exactly the planned commits (`<commit>:refs/heads/<branch>`), never whatever a local branch points to by then. A commit added to the integration branch while the update is finishing stays local.

If the push is rejected, nothing is published, the local branches stay advanced, and the update stays in the `finishing` step. Running `finish` again retries the same push. If the remote holds work the local branches lack, or never accepts the push (for example a protected branch), run `finish --no-publish`. It closes the update locally: it completes any remaining local moves with the same checks, removes the worktree, the update branch and the state, and lists the branches it left unpublished (also in `report`). Then merge the remote's work and publish the result through a new verified update or the repository's usual review. `--no-publish` also works on a verified update before any push attempt.

`abort` removes the worktree, the update branch and the state at any step before `finish` moves a branch. The integration and mirror branches stay exactly as they were. Resolutions already made by hand are recorded first, so a retry reapplies them.

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
| `start [UPSTREAM...]` | Starts an update, as described above. |
| `continue` | Concludes the merge stopped on conflicts and merges the remaining queued upstreams. It also accepts a merge concluded by hand when the commit keeps the sync record. |
| `abort` | Removes the update worktree, branch and state. |
| `verify` | Runs `install` and the checks and records the verified commit. |
| `finish [--no-publish]` | Fast-forwards and publishes the verified update. With `--no-publish` it completes the local moves, closes the update without pushing, and lists the unpublished branches. |
| `status [--check] [--json]` | Shows the configuration, then for every upstream its path, branch, last synced commit, fetched tip, the number of commits the integration branch lacks, and one state: `current`, `behind`, `not imported`, `path removed`, `rewritten` or `unavailable`. `path removed` means a synced path upstream, from a record or from history, whose path no longer exists on the integration branch; `start` refuses it and `status --check` counts it as drift. When several states apply, `unavailable` comes first, then `rewritten`, then `path removed`. A path upstream is `not imported` until a sync record for it exists or, without one, while its path is absent or holds content that no merge of the upstream brought in; a root upstream is `not imported` while it shares no history with the integration branch. While an update is in progress it also shows the step reached, the base, the worktree, each queued upstream's target and step, and the next command. Without `--check` it uses the remote-tracking refs as last fetched. |
| `report [--json]` | For the update in progress, or the one just finished: per upstream, the merged range and commit count, each conflicted file with its resolution source (by hand or by a recorded resolution), and the size of the repository's own changes against that upstream before and after the merge. |
| `log [UPSTREAM...] [--json]` | Lists the commits on each fetched upstream branch that the integration branch does not contain. |
| `branches [UPSTREAM...] [--json]` | Lists every branch of each upstream remote that holds commits the integration branch does not contain, with the commit count. |
| `doctor [--json]` | Checks the repository without changing it, and reports each check as pass, fail or warn. Checks: the configuration is valid; every remote exists with the configured URL; every upstream branch exists; every imported path exists on the integration branch; every last synced commit is still contained in its branch; the main checkout is clean; no merge is in progress there; the update state, if any, matches its worktree and branch. It warns about upstreams that still have sync records but were dropped from the configuration. |

`status --check` first fetches every upstream, adding missing remotes. It then predicts, with `git merge-tree`, the files a merge of each behind upstream would conflict in. It creates no worktree, branch or state, and fails unless every upstream is current.

### JSON output

`status`, `report`, `log`, `branches` and `doctor` accept `--json`. They then write one JSON document to standard output, with the same facts as the text form under stable snake_case field names, and exit with the same status. Notes and errors go to standard error. For example, every entry of `status --json` `upstreams` carries `name`, `path`, `remote`, `branch`, `url`, `mirror`, `last_synced`, `last_synced_source` (`record`, `history` or null), `tip`, `commits_behind`, `state` (`current`, `behind`, `not_imported`, `path_removed`, `rewritten`, `unavailable`), `detail`, and, with `--check`, `conflicts`.

## Exit status

| Status | Meaning |
| --- | --- |
| 0 | Success. |
| 1 | A negative result: `status --check` found drift, a `doctor` check failed, `verify` failed, an upstream could not be fetched or its remote was refused, or `start`/`continue` stopped on conflicts. |
| 2 | An error: invalid configuration, a refused command (for example `finish` on an unverified update), a git failure (including a push rejected by the publish remote, after which `finish` retries the same push and `finish --no-publish` closes the update locally), or invalid usage. |

## Monorepo migration

1. Commit a configuration that lists one `[[upstream]]` per source repository, each with its own `path` and `url`.
2. Run `git intake start`. Each source is imported under its path with its full history, then verified and finished like any update. A path imported earlier with `git subtree add` is adopted by its first update. A path that holds content unrelated to the source's history is refused, and so is an import that would replace a file.
3. Until cutover, keep taking the sources in with the same cycle. Conflicts against the monorepo's new layout are resolved once and recorded. Files moved within an upstream's path merge as renames. Files moved out of the path may conflict or arrive at their old location; resolve them like any other conflict.
4. Run `git intake status --check` on a schedule. It fails whenever a source has moved on and lists the files that would conflict. Zero drift, meaning every upstream is current, is the cutover gate.
5. Before freezing the sources, run `git intake branches` to find work left on other branches of each source. It reports when no upstream holds unmerged work.
6. After cutover, remove an upstream from the configuration. `doctor` keeps warning about upstreams that are dropped but still have sync records, so the removal is visible.

## Safety properties

- The tool never rewrites history and never force-pushes. Local branches only fast-forward.
- It never pushes to an upstream remote. `publish` may not be an upstream's remote.
- Nothing is written to the main working tree. The update lives in its own worktree, on `git-intake/update`, with its state in `<git-common-dir>/git-intake/`, which every worktree shares and no commit can carry.
- Each target is fixed when the update starts. What is merged, verified and published is the same commit.
- `finish` moves no ref unless the verified commit is the worktree head, the base is still the integration tip, and every move is a fast-forward. It validates every move before making any, and it can be resumed.
- Publication is a single atomic push of exactly the planned commits. If any branch is rejected, none moves on the remote; nothing unverified is ever published by a retry, and `finish --no-publish` closes an update that cannot be published.
- An existing remote is never retargeted.
- A rewritten upstream is detected from the sync record and is not merged.
- The configuration is read from the committed integration tip, never from a working copy or from the tree being merged.
