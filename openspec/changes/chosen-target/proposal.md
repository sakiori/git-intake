## Why

`start` takes each selected upstream at the tip it just fetched. A fork that syncs only reviewed upstream states, such as a release tag or a commit that passed the fork's own review, cannot use the tool for that: the tip may already hold newer, unreviewed commits, and the only way around is to hold the remote-tracking branch back by hand. The same applies to a monorepo whose sources are taken in at agreed points rather than at whatever they hold at the moment.

## What changes

`start` accepts a revision for each upstream it names, written as `<upstream>=<revision>`. The revision is a commit, a tag of the upstream's remote, or anything git resolves, and its commit becomes that upstream's target instead of the fetched tip. Upstreams named without a revision keep the fetched tip, so one update can take several upstreams, each at its own chosen point.

The target must lie on the fetched upstream branch: the tool syncs a branch, not arbitrary commits. An update never moves an upstream backwards or sideways: a target older than the last synced commit, or one that does not contain it, is refused. A target the integration branch already contains is reported as current. Everything after `start` is unchanged: the sync record names the chosen target, the mirror branch fast-forwards to it, and `status` shows the upstream as behind its branch tip by the commits the update left out. `status` and `report` also show the revision an update was asked for.

## Capabilities

### New capabilities

None.

### Modified capabilities

- `git-intake`: `start` takes an optional revision per named upstream as its target, and `status` reports an upstream synced to a chosen target.

## Impact

The command line of `start` gains the `<upstream>=<revision>` form; existing invocations keep their meaning. The update state and the JSON output of `status` and `report` gain an optional field with the requested revision; a state written by the previous release is still read. Resolving a tag asks the upstream's remote for that tag, and no tag is fetched into the repository.
