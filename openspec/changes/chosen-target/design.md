## Naming a target on the command line

A target is bound to its upstream inside the operand: `git intake start app-a=v2.3 app-b`. An upstream name may contain only letters, digits, `-`, `_` and `.`, so the first `=` always ends the name and the rest is the revision, even when the revision itself contains `=`.

The alternative, an option such as `--to <revision>` allowed only when exactly one upstream is selected, reads well for a fork but cannot express a monorepo update that takes several sources at reviewed points in one verified update. It would also need an arity rule that changes meaning with the configuration (one upstream configured versus one named). The operand form needs no such rule, extends the existing upstream operands instead of adding a second way to select, and keeps every update a single `start`. Naming an upstream twice, or giving an empty name or revision, is refused.

## Resolving a revision

A tag is meant in the upstream's namespace. The fetch keeps `--no-tags`, because upstream tags would clash with the repository's own tags of the same name (a fork's `v1.0` is not its upstream's `v1.0`). Instead, when the revision is a valid tag name, the tool asks the upstream's remote for `refs/tags/<revision>` with `git ls-remote` and takes the commit the tag points at, peeled through an annotated tag. No tag object or tag ref is fetched: a commit of the upstream branch is already present after the branch fetch, and a tagged commit that is absent cannot be on the branch. A revision that is not a tag of the upstream's remote is resolved by `git rev-parse` in the repository, which covers full and abbreviated commit ids and expressions such as `upstream/main~3`.

Whatever named it, the commit must be contained in the fetched upstream branch. This is the guard that makes every name safe: a revision that resolves to a downstream commit, a commit of another upstream branch, or a tag on a side branch of the upstream is refused with the upstream, the revision and the branch named. The update records the commit id, never the revision, so the target cannot move during the update even if the tag does.

## Order of the decisions

With the last synced commit `L` and the target `T`:

1. `T` is an ancestor of `L` and not `L` itself: refused as older than the last synced commit. The integration branch contains `T` as well, but reporting it as current would hide that the request asked for an older state than the one already taken in.
2. The integration branch contains `T`: the upstream is current, as today.
3. `T` does not contain `L`, as for a commit on a side branch merged into the upstream after `L`: refused. Taking it would make the last synced commit go sideways, `status` would compare against a commit unrelated to the previous sync, and the mirror branch at `L` could not fast-forward to `T`, so `finish` would refuse at the end of the update anyway.
4. Otherwise the upstream is queued with `T` as its target, and the commit count is the number of commits of `T` the base lacks.

Without a sync record and without history that shows one, there is no `L`; the target only has to be on the branch, and the import or merge proceeds as for a tip.

## The record and the state

Nothing downstream of `start` changes. The sync record carries the chosen target, so the last synced commit is that target, and `status` counts the commits between it and the fetched tip as the upstream being behind. The mirror branch fast-forwards to the target. `continue`, `abort`, `finish` and resumption read the target from the state as before.

The queued upstream gains an optional field with the revision as given, shown by `status` and `report`. It is optional with a default, so a state written by the previous release, which has no such field, is still read, and the state format keeps its version.
