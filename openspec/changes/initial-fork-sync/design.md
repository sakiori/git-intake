## Merge, not rebase

Forks that carry many lasting features over a fast-moving upstream take upstream by merge in the projects that maintain such forks (Android vendor trees, Apple's LLVM, LineageOS); rebasing a patch stack is the practice where the downstream changes are few and expected to disappear upstream (OpenShift carry commits, distribution kernels). The forks this tool serves carry dozens of features that upstream will not take, and most of them edit code upstream changes daily. Under rebase, one upstream change conflicts again in every feature commit that touches the same code; under merge, it is resolved once, against the combined result. Merging also removes force-pushes and lets a downstream branch stacked on a fork follow it by the same operation.

What merge gives up is a branch whose commits are exactly the features. Features still land as one squash commit each, so `git log --no-merges <mirror>..<integration>` lists them, and a feature is dropped by a revert that names it. The cost of conflicts is reduced further inside each fork by keeping feature code in fork-owned modules behind small seams in upstream files; that is the fork's concern, not the tool's.

## Plain git, driven as a subprocess

The tool runs the installed `git` for every repository operation instead of linking a git library. Worktrees, merges with recorded resolution, and conflict state are what the procedure needs, and the git command line is the complete, documented implementation of them; libraries either lack parts of this (merge with rerere, worktree management) or reimplement it with subtle differences. Running `git` also leaves the repository in states a user recognises and can finish by hand.

## Rust, one binary

The tool serves forks in any language, on developer machines and in scheduled CI jobs. A single static binary needs no runtime on the machine and none in the forks; installing it as `git-fork-sync` on the path makes it available as `git fork-sync`.

## Configuration on the integration branch

The configuration describes facts about the fork that change with it: its check commands follow the fork's tooling, and its upstream is part of what the fork is. Committing it on the integration branch versions it with the code it checks. The tool reads the committed blob from the integration tip so that an update is never steered by an uncommitted edit, and so that the worktree being merged, whose copy may be in conflict, is never the source of its own instructions.

## State in the common git directory

An update spans several invocations and possibly several shells. Its state (target, base, worktree, step, verified commit, logs) lives in a file under the repository's common git directory, which every worktree shares and which no commit can carry. Nothing about an update is written to the working tree or to refs other than the update branch.

## Publication checks before any ref moves

`finish` compares the recorded base with the integration tip and the verified commit with the worktree head before moving anything. Branches only fast-forward; a push without force is rejected by the remote when someone else published first, and the tool then stops with the local branches advanced and an explanation, because overwriting a remote branch is never the fork's intent.

## Upstreams at a path

A monorepo assembled from repositories that keep moving is a downstream repository with several upstreams, each owning one path. The configuration is therefore a list of upstreams, and a fork is the case of one upstream at the root. Every other part of the procedure (fixed targets, an isolated worktree, recorded resolutions, one verification of the published tree, guarded publication) applies unchanged, so the monorepo case is the same tool rather than a separate migration script.

A path-scoped upstream is merged with git's subtree merge: the first import merges the upstream with the `ours` strategy and reads its tree under the path, and later updates merge with the `subtree=<path>` strategy option. This is the merge `git subtree add` and `git subtree pull` perform without `--squash`; the tool issues it directly because the merge has to run inside its own worktree and state, and because `git subtree` is a contributed script not every git installation carries. History is kept in full: squashed imports would lose the commit identities that drift counts and the sync record rely on, so the tool does not offer them.

The subtree strategy maps paths by prefix and then relies on git's rename detection. Files the monorepo moves within an upstream's path merge as renames; files moved out of the path, or split and joined during restructuring, may conflict or arrive at their old location, and are resolved by hand like any other conflict.

Paths may not nest, and an upstream at the root cannot share the repository with others, so that every file has at most one upstream and conflicts are always attributable.

## The history is the sync record

What each upstream last delivered is a fact about the integration branch, so it lives there: every merge commit the tool creates carries trailers naming the upstream and the target. `status` reads the newest such trailer per upstream; `doctor` uses the same trailers to notice an upstream dropped from the configuration. A last synced commit in the configuration would be a second copy that drifts from the history at the first manual merge or reverted update. The trailers also detect a rewritten upstream: if its branch no longer contains the recorded target, a merge would silently reintroduce or duplicate history, so the tool stops.

## One update, many upstreams

One `start` queues every selected upstream with new commits in configuration order and merges each with its own commit, so upstream changes stay separable from the repository's own work and from one another in history. Verification runs once on the combined tree: checks across a monorepo are expensive, and an intermediate tree that is never published needs no proof. An upstream whose conflict cannot be resolved now is handled by aborting and starting again with the others named; recorded resolutions make the repeated merges cheap, so the tool needs no partial-skip state.

## Remotes from the configuration

Upstream URLs are facts about the repository and are committed with it, so a new clone becomes operable by its first `fetch`, which adds missing remotes. An existing remote with a different URL is never rewritten: it may be a deliberate local mirror, and silently retargeting it would fetch from the wrong source.

## Output for agents

Reporting commands produce a JSON document with stable field names alongside the text, and the same exit status. Agents and scheduled jobs act on drift, conflicts and doctor failures without parsing prose, and `status --check` failing on any drift gives a cutover a single gate.
