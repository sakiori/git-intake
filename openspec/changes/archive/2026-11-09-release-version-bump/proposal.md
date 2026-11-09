## Why

A downstream under `semver` may need to know why a build gets the version it gets, not only the version. A downstream that guards a public contract, such as the API report of a package it publishes, has to check that a change of that contract since the previous release is marked by a large enough change: an added declaration by at least a minor change, a removed or changed one by a major change. That check needs the previous release, to read the contract it shipped, and the change git intake computed for the commit, to compare with it.

`release version` already computes both and prints neither. A version alone does not tell them apart: a rehearsal of a commit that takes no release prints the version a patch change would give, and the previous release cannot be read back from the next one. Recomputing them outside the tool would repeat its rules for the downstream's line, sync records and recorded upstream releases, and drift from them.

## What changes

- Under `semver`, `release version --json` also writes `previous`, the tag of the previous release of the commit it versions, or null when there is none, and `bump`, the change that sets that commit's next release, `"major"`, `"minor"` or `"patch"`, or null when the commit takes no release.
- For a release tag, they describe the tagged commit: the release before the tag and the change that gave the tag its version.
- Under `dated`, the object is unchanged.
- `--dotenv` and the text output stay as they are.

## Capabilities

### New capabilities

None.

### Modified capabilities

- `git-intake`: the JSON object of `release version`.

## Impact

- Output: two new keys in `release version --json` under `semver`. A consumer that reads the object by key is unaffected.
- Network: none beyond what `release version` already reads.
