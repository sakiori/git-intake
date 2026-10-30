## Why

Downstream forks of active upstream projects carry lasting features of their own. Keeping each fork current is the same procedure in every repository: pin an upstream target, merge it into the fork's integration branch away from the working checkout, resolve conflicts once, run the fork's own checks, and only then advance and publish the branches. Done by hand, the procedure is long and easy to get wrong: a target that moves during the update, checks run against a different tree than the one published, a push that overwrites concurrent work, a half-finished merge left in the main checkout.

No established tool covers this for source forks hosted outside GitHub: the large downstream projects each wrap plain git in scripts of their own, and the generic candidates either mirror code one way, merge without verification, or require a particular forge.

## What changes

A single command-line tool, `git fork-sync`, drives the update of any fork that describes itself in a committed configuration file. The tool owns the procedure; each fork owns its facts: which remote and branch are upstream, which branch is the mirror and which the integration branch, where to publish, and how to install and check the result.

The model is merge-based. The integration branch takes upstream by a merge commit, its history is never rewritten, and nothing is force-pushed. A downstream branch stacked on another fork's integration branch is updated by the same tool and model, with that branch as its upstream.

## Capabilities

### New capabilities

- `fork-sync`: configured, resumable upstream updates for downstream forks, with a fixed target, isolated merge, recorded verification and guarded publication.

### Modified capabilities

None.

## Impact

Each fork adopts a configuration file and documents the tool as its update procedure. The tool shells out to the installed `git` and to the fork's own commands; it adds no runtime dependency to the forks. Conflict resolution stays with the person or agent running the update; the tool stops, explains the state, and resumes.
