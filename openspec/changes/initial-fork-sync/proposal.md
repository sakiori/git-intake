## Why

Downstream forks of active upstream projects carry lasting features of their own. Keeping each fork current is the same procedure in every repository: pin an upstream target, merge it into the fork's integration branch away from the working checkout, resolve conflicts once, run the fork's own checks, and only then advance and publish the branches. Done by hand, the procedure is long and easy to get wrong: a target that moves during the update, checks run against a different tree than the one published, a push that overwrites concurrent work, a half-finished merge left in the main checkout.

No established tool covers this for source forks hosted outside GitHub: the large downstream projects each wrap plain git in scripts of their own, and the generic candidates either mirror code one way, merge without verification, or require a particular forge.

The same procedure applies to a monorepo assembled from several repositories that keep receiving changes while it takes shape. Each source repository is an upstream merged under its own path; until cutover, every one of them has to be taken in again and again, conflicts against the monorepo's new layout and tooling resolved, and the result checked. Before cutover, every source has to be shown fully taken in, including work left on its other branches.

## What changes

A single command-line tool, `git fork-sync`, drives the update of any repository that describes itself in a committed configuration file. The tool owns the procedure; each repository owns its facts: its integration branch, its upstreams (for each, the remote, branch, optional URL, optional path inside the repository and optional mirror branch), where to publish, and how to install and check the result. A fork has one upstream at the repository root; a monorepo in migration has one upstream per source repository, each under its own path.

The model is merge-based. The integration branch takes upstream by a merge commit, its history is never rewritten, and nothing is force-pushed. A downstream branch stacked on another fork's integration branch is updated by the same tool and model, with that branch as its upstream.

One update takes every selected upstream that has new commits, each by its own merge commit, stops at each conflict, verifies the combined result once and publishes it. The repository's history is the only record of what was taken in: every merge commit names its upstream and target. `status` reports drift per upstream and, with `--check`, gates a cutover on zero drift; `log`, `branches` and `doctor` show pending commits, unmerged upstream branches and configuration problems. Reporting commands have a JSON form for agents and scripts. Data flows only from upstreams into the repository; the tool never pushes to an upstream.

## Capabilities

### New capabilities

- `fork-sync`: configured, resumable updates of a repository from one or more upstreams, each at the root or under its own path, with fixed targets, isolated merges, recorded verification, guarded publication and per-upstream status.

### Modified capabilities

None.

## Impact

Each fork or monorepo adopts a configuration file and documents the tool as its update procedure. The tool shells out to the installed `git` and to the fork's own commands; it adds no runtime dependency to the forks. Conflict resolution stays with the person or agent running the update; the tool stops, explains the state, and resumes.
