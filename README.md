# fork-sync

`git fork-sync` keeps a downstream repository current with one or more upstreams. It takes upstream changes by merge commits, never rewrites history and never force-pushes. The merges happen in a separate worktree. A conflict is resolved once, and the repository's own checks run on exactly the tree that gets published. The integration branch moves only by fast-forward, after verification.

It covers two cases:

- **A fork**: one upstream merged at the repository root. A branch stacked on another fork's integration branch is a fork too, with that branch as its upstream.
- **A monorepo assembled from several repositories**: each source repository is an upstream merged under its own path. They keep being taken in until cutover, and `status --check` gates the cutover.

The tool runs the installed `git` for every repository operation. Each repository describes itself in a committed `.fork-sync.toml`.

## Install

Requires Git 2.43 or later (`git merge-tree -X` is used for conflict prediction) and a POSIX `sh`.

```sh
cargo install --path .
```

This installs the `git-fork-sync` binary. With it on the `PATH`, git runs it as `git fork-sync`.

## Configuration

`.fork-sync.toml` lives at the repository root and is committed on the integration branch. The tool always reads the version committed at the integration branch tip. An uncommitted edit in the working copy is ignored, and the tool prints a note that the working copy differs. The tool finds the integration branch from the configuration committed at `HEAD`, or from the update in progress, so run it from a checkout of the integration branch or of a branch that carries the configuration.

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
- A path is normalised: no leading or trailing slash, no empty, `.` or `..` segment, and it may not be empty. Omit `path` for the root.
- Paths may not nest; one upstream's path cannot lie inside another's.
- An upstream at the root must be the only upstream.
- `publish` may not be an upstream's remote, because the tool never pushes to an upstream.
- A mirror may be neither the integration branch nor `fork-sync/update`.

## The update cycle

```sh
git fork-sync start        # fetch, fix targets, merge each upstream with new commits
# on conflict: resolve in the update worktree, `git add`, then
git fork-sync continue     # conclude the merge, merge the remaining upstreams
git fork-sync verify       # run install and checks on the merged result
git fork-sync finish       # fast-forward, publish, clean up
```

`start` refuses to begin if any of these hold:

- an update is already in progress;
- the main checkout or a checkout of the integration branch has uncommitted changes to tracked files;
- the integration branch is behind its counterpart on the publish remote;
- a selected upstream is unknown, unavailable or rewritten;
- a path upstream's path already holds content that does not come from the upstream's history.

When `start` runs, it does the following:

1. Fetches the selected upstreams. With no names it selects all of them.
2. Records each fetched tip as that upstream's **target** and the integration tip as the **base**. Targets never change during the update, even if the upstream moves on.
3. Queues, in configuration order, the upstreams whose target the base does not contain yet. The others are reported as current. If nothing is queued, nothing is created.
4. Creates the worktree `<git-common-dir>/fork-sync/worktree` on the branch `fork-sync/update` from the base. The main checkout is never touched.
5. Enables `rerere.enabled` for the repository. A resolution recorded in one attempt is reapplied when the same conflict recurs, and the tool reports such files as *resolved by a recorded resolution*. Review those files and stage them like any other resolution.
6. Merges each queued target with its own merge commit:
   - a root upstream: `git merge --no-ff`;
   - a path upstream, first import (the path is absent and no history is shared): an `ours` merge followed by `git read-tree --prefix=<path>/`. This keeps the upstream's full history;
   - a path upstream, later updates: `git merge --no-ff -X subtree=<path>`.

On a conflict the tool stops with exit status 1. It names the upstream, lists the conflicted files, and reports which upstreams are merged and which are pending. Resolve the files in the worktree, stage them, and run `continue`. If one upstream cannot be resolved now, `abort` and run `start` again with only the other upstreams named. Recorded resolutions make the repeated merges cheap.

`verify` runs `install` and then every check through `sh -c` in the update worktree. Each command's output goes to its own log under `<git-common-dir>/fork-sync/logs/`. If `install` fails, the checks are skipped. Verification fails in any of these cases:

- a command fails or is skipped;
- the commands change tracked files;
- the update branch moves while the checks run.

When it passes, `verify` records the commit it verified.

`finish` checks the following before moving any ref:

- every upstream is merged;
- the worktree head is the verified commit, so any commit added after `verify` needs a new `verify`;
- the base is still the integration tip. If the integration branch moved, `abort` and `start` again so the update merges the new tip, then verify again.

It then fast-forwards the integration branch to the verified commit. A checkout of that branch is updated with `git merge --ff-only`, and other branches move with `git update-ref` against their old value. Each merged upstream's mirror is fast-forwarded to its target. When `publish` is set, `finish` pushes these branches with one atomic push and without force, so either every branch is published or none is. Finally it removes the worktree, the update branch and the state.

If the push is rejected because someone published first, the local branches stay advanced and the tool explains how to reconcile without force: fetch, merge the remote branch, verify, and push again.

`abort` removes the worktree, the update branch and the state at any step before `finish`. The integration and mirror branches stay exactly as they were. Resolutions already made by hand are recorded first, so a retry reapplies them.

## The sync record

Every merge commit the tool creates looks like this:

```
sync(app-a): merge app-a/main at 1a2b3c4

Fork-Sync-Upstream: app-a
Fork-Sync-Target: <full commit id>
```

An upstream's **last synced commit** is the target in the newest such commit on the integration branch's first-parent line. It is not stored anywhere else. Only this repository's own records count. Records inside a configured upstream's fetched branch are skipped; this happens, for example, when a stacked branch starts from another fork's integration line that carries that fork's records. The tool's merges always stay on the first-parent line because `finish` only fast-forwards to the update branch, where each merge takes the previous head as its first parent. If an upstream's branch no longer contains its last synced commit, the upstream was rewritten: `status` reports it and `start` refuses to update it.

## Commands

| Command | Purpose |
| --- | --- |
| `fetch [UPSTREAM...]` | Adds missing remotes from configured URLs and fetches the selected upstreams, reporting each one. An existing remote whose URL differs from the configuration, or a missing remote without a URL, is refused and nothing is fetched. The fetch uses `--prune --no-tags`. |
| `start [UPSTREAM...]` | Starts an update, as described above. |
| `continue` | Concludes the merge stopped on conflicts and merges the remaining queued upstreams. It also accepts a merge concluded by hand when the commit keeps the sync record. |
| `abort` | Removes the update worktree, branch and state. |
| `verify` | Runs `install` and the checks and records the verified commit. |
| `finish` | Fast-forwards and publishes the verified update. |
| `status [--check] [--json]` | Shows the configuration, then for every upstream its path, branch, last synced commit, fetched tip, the number of commits the integration branch lacks, and one state: `current`, `behind`, `not imported`, `rewritten` or `unavailable`. While an update is in progress it also shows the step reached, the base, the worktree, each queued upstream's target and step, and the next command. Without `--check` it uses the remote-tracking refs as last fetched. |
| `report [--json]` | For the update in progress, or the one just finished: per upstream, the merged range and commit count, each conflicted file with its resolution source (by hand or by a recorded resolution), and the size of the repository's own changes against that upstream before and after the merge. |
| `log [UPSTREAM...] [--json]` | Lists the commits on each fetched upstream branch that the integration branch does not contain. |
| `branches [UPSTREAM...] [--json]` | Lists every branch of each upstream remote that holds commits the integration branch does not contain, with the commit count. |
| `doctor [--json]` | Checks the repository without changing it, and reports each check as pass, fail or warn. Checks: the configuration is valid; every remote exists with the configured URL; every upstream branch exists; every imported path exists on the integration branch; every last synced commit is still contained in its branch; the main checkout is clean; no merge is in progress there; the update state, if any, matches its worktree and branch. It warns about upstreams that still have sync records but were dropped from the configuration. |

`status --check` first fetches every upstream, adding missing remotes. It then predicts, with `git merge-tree`, the files a merge of each behind upstream would conflict in. It creates no worktree, branch or state, and fails unless every upstream is current.

### JSON output

`status`, `report`, `log`, `branches` and `doctor` accept `--json`. They then write one JSON document to standard output, with the same facts as the text form under stable snake_case field names, and exit with the same status. Notes and errors go to standard error. For example, every entry of `status --json` `upstreams` carries `name`, `path`, `remote`, `branch`, `url`, `mirror`, `last_synced`, `tip`, `commits_behind`, `state` (`current`, `behind`, `not_imported`, `rewritten`, `unavailable`), `detail`, and, with `--check`, `conflicts`.

## Exit status

| Status | Meaning |
| --- | --- |
| 0 | Success. |
| 1 | A negative result: `status --check` found drift, a `doctor` check failed, `verify` failed, an upstream could not be fetched, or `start`/`continue` stopped on conflicts. |
| 2 | An error: invalid configuration, a refused command (for example `finish` on an unverified update), a git failure (including a push rejected by the publish remote), or invalid usage. |

## Monorepo migration

1. Commit a configuration that lists one `[[upstream]]` per source repository, each with its own `path` and `url`.
2. Run `git fork-sync start`. Each source is imported under its path with its full history, then verified and finished like any update. A path that already holds unrelated content is refused.
3. Until cutover, keep taking the sources in with the same cycle. Conflicts against the monorepo's new layout are resolved once and recorded. Files moved within an upstream's path merge as renames. Files moved out of the path may conflict or arrive at their old location; resolve them like any other conflict.
4. Run `git fork-sync status --check` on a schedule. It fails whenever a source has moved on and lists the files that would conflict. Zero drift, meaning every upstream is current, is the cutover gate.
5. Before freezing the sources, run `git fork-sync branches` to find work left on other branches of each source. It reports when no upstream holds unmerged work.
6. After cutover, remove an upstream from the configuration. `doctor` keeps warning about upstreams that are dropped but still have sync records, so the removal is visible.

## Safety properties

- The tool never rewrites history and never force-pushes. Local branches only fast-forward.
- It never pushes to an upstream remote. `publish` may not be an upstream's remote.
- Nothing is written to the main working tree. The update lives in its own worktree, on `fork-sync/update`, with its state in `<git-common-dir>/fork-sync/`, which every worktree shares and no commit can carry.
- Each target is fixed when the update starts. What is merged, verified and published is the same commit.
- `finish` moves no ref unless the verified commit is the worktree head and the base is still the integration tip.
- Publication is a single atomic push. If any branch is rejected, none moves on the remote, and the tool explains how to reconcile without force.
- An existing remote is never retargeted.
- A rewritten upstream is detected from the sync record and is not merged.
- The configuration is read from the committed integration tip, never from a working copy or from the tree being merged.
