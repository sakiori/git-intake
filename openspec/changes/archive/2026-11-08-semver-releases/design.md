## Context

git intake releases a downstream by cutting a tag on its integration tip when the tip carries no release, guarding the tag pipeline, giving builds the version and writing the notes. The version is `<base>-<label>.<YYYYMMDD>.<n>`: the next patch after the newest upstream release the tip contains, a label, the day and a number. Two kinds of downstream use it: an application with a nightly line, whose Android version code is `YYYYMMDD * 100 + n`, and forks published as packages and executables that other projects depend on. The second kind needs versions that ranges can select and that say what changed.

## Decisions

### Practice where an upstream exists

Two practices exist, and they answer different consumers:

- **Upstream version and a downstream revision.** Distributions version a package as the upstream's version with a revision of their own (Debian's `upstream_version-debian_revision`, restarted at 1 for each upstream version; Fedora's Release, Homebrew's revision), and so do soft forks of applications that ship as builds, such as Chromium and Firefox derivatives. It works because their version comparison sorts `1.2.3-2` after `1.2.3`. Under SemVer, `-2` is a prerelease: it sorts before `1.2.3`, and ranges exclude it. Build metadata, `1.2.3+2`, does not help either: SemVer ignores it in precedence, and npm drops it and refuses two versions that differ only there.
- **A version of the fork's own, with the upstream as metadata.** A fork that others depend on through SemVer ranges versions its own contract. Forgejo did exactly this move: its versions followed Gitea's with a revision (`1.21.x-y`) until v7.0.0, which adopted SemVer 2.0.0 for Forgejo itself and carries the compatible Gitea version as build metadata, `7.0.0+gitea-1.22.0`.

The forks this change serves are depended on as packages, and each carries its own interface beyond the upstream's. They take the second practice; the platform's nightly line, which ships builds, keeps the first in its dated form.

### Two schemes, chosen per downstream

`[release].versioning` selects `dated` or `semver`. Each is a complete scheme with its own consumers: a nightly application line needs a monotonic build number and a version a person reads as a date, and a package needs a compatibility contract. Neither is a transition to the other, and a configuration without the key keeps `dated`, which is what it means today.

Everything else in releasing stays shared: the downstream's line, publication by forge release, the scheduled `release tag` that fails on an unpublished release at the tip, the push race through the remote, and where the configuration is read.

Alternatives:
- semantic-release or release-please in each downstream. Both compute the next version from Conventional Commits, but they know nothing of the downstream's line or sync records, and they would replace the guard, the version handed to builds and the notes, which the downstreams already take from git intake. Each downstream would carry a second release mechanism beside git intake, which still has to keep updates and the platform's dated line.
- release-plz, which git intake itself uses. It releases Rust crates from their manifests, and none of the three forks publishes its crates.

### The next version comes from every commit since the previous release

The commits that set the next version are those the integration tip has and the previous release lacks, merges excluded. These are the downstream's own squashed commits and the upstream commits that updates brought in. All three upstreams the forks follow write Conventional Commits, so an update carrying only upstream fixes is a patch, and an upstream `feat!` is a major version.

For Conventional Commits the rules are semantic-release's defaults, the most widely used mapping: a breaking change gives major, `feat` minor, `fix`, `perf` and `revert` patch, and any other type no release. Commits are parsed with `git-conventional`, which reads a `BREAKING CHANGE:` note only in the footers that close the message, as the Conventional Commits specification places it; conventional-commits-parser, which semantic-release uses, also finds one at the start of any line of the body.

A subject that is not a Conventional Commit gives a patch, where semantic-release gives nothing. The downstream's own commits are Conventional Commits, so this applies to upstream commits, which are not written under the downstream's rules: of the last 200 upstream commits of each fork, one of fff's (git's `Revert "…"`) and seven of expo-device-hub's are plain subjects, and all of them change code. Releasing nothing for them would ship an upstream change without a release.

Alternatives:
- Own commits only, with an update counted as a fixed minor change. It loses what the upstream's own commits say, and a fixed minor change would overstate a fix-only update.
- Any commit releases a patch, as release-plz does. The scheduled job would then publish packages for a change to tests or CI alone.

### An incompatible upstream release raises the major version

Upstreams at major version 0 mark breaking changes by their version, not by `!`: a 0.11 to 0.12 release is breaking under SemVer whatever its commits say. An update that moves the newest contained upstream release out of the previous one's `^` range therefore raises the major version, independently of the commits' types.

Before an upstream release, a downstream that follows the upstream's branch can ship an unmarked breaking change as a minor version. It is reported as a risk, not prevented: no signal exists until the upstream releases or a maintainer marks it.

### The first semver release is 1.0.0

Under `^`, a 0.x version pins its minor version, so 0.x releases would give consumers patch updates only. The first `semver` release on a line is therefore `1.0.0`, or the next major version after every earlier release when one is at 1.0.0 or above, so that it sorts after the dated releases it follows. Dated releases with a label in `labels` stay releases: they bound the first range of commits and take part in the order.

### Tags `v<x.y.z>`

`v<x.y.z>` is the form mise's GitLab and GitHub backends, Renovate's release datasources and the forges read without configuration. The forks' upstreams also tag `v*`, and a fork's version can equal one of the upstream's. git intake never confuses them: it lists the upstream's tags from the upstream's URL without fetching them, and counts as releases only tags of the publish remote on the downstream's line. A clone that fetches tags from both repositories can still see two tags of one name; projects document which remote releases what.

Alternative: a prefixed tag such as `sakiori-v1.0.0`. It needs a version prefix setting in every consumer, mise, Renovate and the forge links, for a collision that git intake already excludes.

### A version is a function of the commit and the publish remote's tags

The version a commit takes depends only on its history, the releases below it and the upstream releases their tags record. `release check` therefore recomputes the version of the tagged commit and compares it with the tag, which also covers the order of releases. A tag pipeline run again later gets the same answer.

The upstream release of a commit, the newest one it contains, is read from the upstream's tags, which the upstream can add at any time: a tag on a commit the release already contains would change the release's upstream release after the fact, fail its `release check` and its pipeline run again, and leave the major version an incompatible upstream release asks for to no release at all. `release tag` therefore records the upstream release it computed, with its commit, in the annotation of a semver tag, as the trailers `Intake-Upstream`, `Intake-Target` and `Intake-Tag` that sync records use for an upstream's target and tag. The rule for an incompatible upstream release compares with the previous release's record. The tag being checked, versioned or described takes its own record, which must name a release of the release upstream whose recorded commit the tagged commit contains; `release version` and `release notes` check that locally, so an upstream that later deletes or moves its tag changes neither, and `release check` also reports a tag the upstream still lists at another commit. A dated tag's trailers are never read, and tags without a record fall back to the newest upstream release their commit contains, as before.

`release version` runs in every build, including merge request pipelines that may not reach every upstream, so it reads the last synced commits from the sync records in the local history. A downstream cloned from another downstream carries that one's records; a record counts as the repository's own only when the configuration committed at its merge names that upstream at the same URL and branch, and an upstream without an own record is fetched, as the other commands do.

Alternative: recomputing from the upstream's tags every time. It needs no record, but a version would then depend on when it is computed.

### No override beyond Conventional Commits

A maintainer who knows of a breaking change the commits do not show, such as an unmarked one in an update, lands a commit marked breaking. A footer that sets a version, as in release-please's `Release-As`, would let a release skip numbers or break the order, which the tool would then have to check. It can be added when a case needs it.

## Risks

- A commit with a wrong type gives a wrong version. Squashed merge requests make the request title the commit subject, so the type is reviewed with the request.
- An unmarked breaking change in an update ships under a minor version until the upstream releases.
- An upstream that tags a release on a commit an earlier release already contained gives that earlier release no major version: it keeps its version and record, and the next release takes the major version.
- A `BREAKING CHANGE:` note in the middle of a body, rather than in the closing footers, is not read as breaking.
- An upstream commit that is not a Conventional Commit counts as a patch, even when it adds a feature or breaks something.
- A fork whose tarball has the upstream's package name, such as `simlock` or `expo-device-hub`, may share a name and version with an upstream package on npm. The forks publish these tarballs only as forge assets, and consumers pin them by integrity.
