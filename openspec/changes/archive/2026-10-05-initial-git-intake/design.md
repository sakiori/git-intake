## Merge, not rebase

Forks that carry many lasting features over a fast-moving upstream take upstream by merge in the projects that maintain such forks (Android vendor trees, Apple's LLVM, LineageOS); rebasing a patch stack is the practice where the downstream changes are few and expected to disappear upstream (OpenShift carry commits, distribution kernels). The forks this tool serves carry dozens of features that upstream will not take, and most of them edit code upstream changes daily. Under rebase, one upstream change conflicts again in every feature commit that touches the same code; under merge, it is resolved once, against the combined result. Merging also removes force-pushes and lets a downstream branch stacked on a fork follow it by the same operation.

What merge gives up is a branch whose commits are exactly the features. Features still land as one squash commit each, so `git log --no-merges <mirror>..<integration>` lists them, and a feature is dropped by a revert that names it. The cost of conflicts is reduced further inside each fork by keeping feature code in fork-owned modules behind small seams in upstream files; that is the fork's concern, not the tool's.

## Plain git, driven as a subprocess

The tool runs the installed `git` for every repository operation instead of linking a git library. Worktrees, merges with recorded resolution, and conflict state are what the procedure needs, and the git command line is the complete, documented implementation of them; libraries either lack parts of this (merge with rerere, worktree management) or reimplement it with subtle differences. Running `git` also leaves the repository in states a user recognises and can finish by hand.

## Rust, one binary

The tool serves forks in any language, on developer machines and in scheduled CI jobs. A single static binary needs no runtime on the machine and none in the forks; installing it as `git-intake` on the path makes it available as `git intake`.

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

## Distribution

The tool is delivered as prebuilt binaries attached to releases of its own repository, so neither developer machines nor CI jobs of the repositories that use it need a Rust toolchain.

Releases follow the release merge request model of release-plz. Preparing a release raises the crate version from the Conventional Commits since the last release and adds the matching CHANGELOG.md section, both computed by git-cliff from `cliff.toml`, and proposes them in a merge request `chore(release): vX.Y.Z`. Merging it is the decision to release. The default-branch pipeline then finds a crate version without its tag `vX.Y.Z`, builds static binaries for `x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl` and `aarch64-apple-darwin`, packs each as `git-intake-vX.Y.Z-<target>.tar.gz` with a SHA-256 checksum, uploads them to the project's generic package registry, and creates the tag and a release whose notes are that CHANGELOG.md section and whose asset links point at the archives. A merge that does not raise the version releases nothing. The version in the repository is therefore always the latest released or proposed one, and the tag is derived from it rather than the other way round.

Release preparation runs as `ci/prepare-release.sh` by hand, because the hosting tier offers no project-scoped token for a bot that pushes branches and opens merge requests. The script performs what `release-plz release-pr` performs, from the same git-cliff configuration, so release-plz replaces it without changing the release pipeline once such a token is available.

Merge request pipelines run the checks and, when a change touches the build configuration, the cross builds, so a release never meets a broken build for the first time. The default-branch pipeline does not repeat the checks: merges are semi-linear and require a successful pipeline, so the default branch always equals a checked merge request head. Musl targets give binaries that run on any Linux distribution and in minimal CI images.

Developers and CI jobs install a pinned version with mise through its GitLab backend, which picks the asset for the running platform by its target triple. On developer machines mise authenticates to the private project with the user's GitLab credentials. In other projects' CI, mise needs an access token with `read_api` scope, because it sends tokens as bearer credentials, which GitLab does not accept for CI job tokens; a job may instead download the release asset directly with its job token once the project's job token allowlist admits it. The pinned version sits in the consuming repository's tool configuration, so an update of the tool is a reviewed change there. `cargo install --git` remains available for contributors who build from source.

The release asset names follow the Rust target triples because both mise's asset selection and human readers recognise them; since the tag is created from the crate version of the commit being released, a binary never reports a version other than the one it was released as.
