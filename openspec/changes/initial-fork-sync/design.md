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
