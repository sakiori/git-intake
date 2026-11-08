## Why

Downstream releases are dated prereleases of the next upstream patch, `v<base>-<label>.<YYYYMMDD>.<n>`. For a downstream that others depend on as a package, that version is the wrong contract:

- Package managers treat it as a prerelease. Under node-semver, `0.11.1-sakiori.20261103.1` satisfies neither `^0.11.0`, `~0.11.0`, `>=0.11.0` nor `*`, and a range anchored on one dated release admits no release of the next base. Consumers can only pin a version exactly or take `latest`.
- The version says nothing about the downstream's own compatibility. It moves with the upstream's releases and the calendar, so a downstream change that breaks its consumers, such as a new service protocol, ships under the same kind of number as a typo fix.
- Every release is a new version on the day it is cut. The number tells a consumer when, not what.

A downstream released as a product of its own needs an ordinary semantic version computed from what changed, while the upstream it carries moves into the release's description. The dated scheme stays right for a downstream that ships builds, such as an application whose nightly line needs a monotonic build number.

## What changes

- `[release]` gains `versioning`, either `dated` (the current scheme, and the default) or `semver`.
- Under `semver`, a release is the tag `v<major>.<minor>.<patch>`, without a prerelease part or build metadata.
- The next version follows the previous release by the largest change among the commits the integration tip has and the previous release lacks, merges excluded and the upstream commits that updates brought in included:
  - a breaking change (`!` after the type, or a `BREAKING CHANGE` footer) raises the major version;
  - `feat` raises the minor version;
  - `fix`, `perf` and `revert` raise the patch version, and so does a subject that is not a Conventional Commit, such as git's `Revert "…"` or an upstream's plain subject;
  - any other type releases nothing.
  For Conventional Commits these are the default release rules of semantic-release's commit analyzer.
- An update also raises the major version when the newest upstream release the tip contains is incompatible with the one the previous release contained, by SemVer's rule for `^` ranges: a different major version, at major 0 a different minor version, and at 0.0 a different patch version.
- The first `semver` release on a downstream's line is `1.0.0`, or the next major version after every earlier release when one is at 1.0.0 or above. Earlier dated releases stay releases, which the existing `labels` names, so that the first `semver` release sorts after them.
- `release tag` cuts nothing when the commits since the previous release release nothing, says so and exits with status 0. `release next` reports the change that sets the next version, or that nothing is due.
- `release check` checks that a `semver` tag has the version its commit takes.
- `release version` gives a rehearsal version `<next>-rehearsal.<YYYYMMDD>`, which no release takes.
- `release version` also reports the newest upstream release the commit contains: `upstream` in JSON and `RELEASE_UPSTREAM=<upstream>-<x.y.z>` in dotenv, so that a build can show it as build metadata, `1.2.0+fff-0.11.0`, where its channel keeps build metadata (npm does not).
- `release notes` states the upstream release the tagged commit contains, and how many upstream commits it carries after it, in its first line.
- Under `semver`, `release notes` puts the downstream's breaking changes first, each with the text of its `BREAKING CHANGE` footer, where the commit says what a consumer has to change.
- Under `dated`, everything stays as it is.

Out of scope: changes to the downstreams' pipelines and settings, listed under Impact; a manual override of the computed version beyond Conventional Commits; changelog files committed to the repository.

## Capabilities

### New capabilities

None.

### Modified capabilities

- `git-intake`: the release configuration gains `versioning`; tag names, the next release, the release order, tag checks, rehearsal versions, `release next --json` and release notes gain their `semver` behaviour. The requirements of the dated scheme keep their text, scoped to `dated`.

## Impact

- Configuration: `versioning` is a new key of `[release]`. Releases before this one refuse it, as an unknown key; every place that runs git intake on the repository upgrades first. A configuration without it keeps the dated scheme.
- `release next --json`: `base` is null under `semver`; new fields `versioning` and `bump` (`major`, `minor`, `patch`, or null when nothing is due). `next` is null when nothing is due.
- `release version`: the same fields and dotenv lines. The version a tag pipeline gets is plain `x.y.z`.
- Dependencies: the Conventional Commits parser `git-conventional`, which git-cliff, release-plz and cargo-release use.
- Downstreams that move to `semver`: fff, simlock and expo-device-hub. None of them needs a build number. In each:
  - set `versioning = "semver"` and keep `labels = ["sakiori"]`, so that the dated releases count;
  - the tag rules of `.gitlab-ci.yml` accept `v<x.y.z>`;
  - the protected tag pattern becomes `v*`;
  - version stamping accepts a plain version (fff's `stamp-version.sh` refuses one now);
  - CUSTOM.md and README describe the new versions, and a change that breaks the downstream's consumers is committed as breaking: fff's library API or service protocol, simlock's CLI, daemon API or `deviceLifecycleProtocol`, expo-device-hub's package interface.
  - merge request pipelines check that the title, which becomes the squashed commit's message, is a Conventional Commit;
  - fff publishes its npm packages at the release's version, and its check of the Node package's API report asks a merge request that changes the report for a `feat` title, or a breaking one when a declaration is dropped or changed.
- Consumers: platform pins each downstream exactly and parses only its own nightly versions; its Renovate configuration no longer needs `ignoreUnstable: false` for the downstreams. The handbook's release and fork practices describe both schemes. platform's own line stays `dated`.
