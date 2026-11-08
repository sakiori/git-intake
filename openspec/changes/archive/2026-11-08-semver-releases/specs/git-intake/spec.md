## ADDED Requirements

### Requirement: Release versioning
`[release].versioning` SHALL be `dated` or `semver`, and `dated` when it is not set. Under `dated`, releases SHALL take the dated versions of "Release tag names" and "Next release"; under `semver`, the semantic versions of "Next semver release", computed from the changes since the previous release. The tool SHALL refuse any other value, naming `release.versioning` with the configuration's other problems.

#### Scenario: Configuration without versioning
- **WHEN** `[release]` sets no `versioning`
- **THEN** releases take dated versions, as before the setting existed

#### Scenario: Unknown versioning
- **WHEN** `[release]` sets `versioning = "calendar"`
- **THEN** every command refuses the configuration and names `release.versioning`

### Requirement: Upstream release of a semver commit
Under `semver`, a commit the release commands version SHALL contain an upstream release, and the tool SHALL refuse one that contains none, as "Release base" says for the release base.

#### Scenario: Semver tip without an upstream release
- **WHEN** under `semver` the publish remote's integration tip contains no upstream release
- **THEN** `release next` exits with status 2 and names the upstream and the pattern

### Requirement: Semver release tags
Under `semver`, a release tag SHALL be named `v<x.y.z>`, a version without leading zeros, prerelease part or build metadata, and its version SHALL be the name without the `v`. A tag of the dated form of "Release tag names" whose label is one of `labels` SHALL stay a release tag, so that the releases cut before the change count as releases.

#### Scenario: Prerelease tag
- **WHEN** under `semver` the publish remote has the tag `v1.2.0-rc.1` on a commit of the downstream's line
- **THEN** that tag is no release

#### Scenario: Dated releases before the change
- **WHEN** `[release]` sets `versioning = "semver"` and `labels = ["sakiori"]`, and the downstream's line carries `v0.11.1-sakiori.20261103.1`
- **THEN** that tag is a release, and it takes part in the order of releases

### Requirement: Commits of a semver release
Under `semver`, the previous release of a commit SHALL be the newest release on the downstream's line below the commit: the highest release of the nearest earlier commit of the line that has one. The commits of the commit's next release SHALL be those the commit has and the previous release lacks, merges excluded, including the upstream commits that updates brought in; without a previous release, every commit the commit has, merges excluded.

#### Scenario: Update with upstream fixes
- **WHEN** since the previous release only an update landed, and it brought in the upstream commits `fix: a` and `docs: b`
- **THEN** the commits of the next release are those two upstream commits, and the update's merge is not one of them

### Requirement: Change of a commit
Under `semver`, the change of a commit whose message is a Conventional Commit SHALL be major for a breaking change, marked by `!` before the colon of the subject or by a `BREAKING CHANGE` or `BREAKING-CHANGE` footer, whatever the type; minor for the type `feat`; patch for the types `fix`, `perf` and `revert`; and no change for any other type. Types SHALL compare without regard to letter case.

#### Scenario: Chores only
- **WHEN** every commit since the previous release has the type `docs`, `test`, `ci` or `chore`, and none is breaking
- **THEN** the commits make no change

#### Scenario: Breaking footer
- **WHEN** a commit `refactor: rename the service` has the footer `BREAKING CHANGE: clients use the new name`
- **THEN** its change is major

### Requirement: Change of another commit
Under `semver`, the change of a commit whose subject is not a Conventional Commit SHALL be patch.

#### Scenario: Subject of git revert
- **WHEN** a commit since the previous release has the subject `Revert "feat: add the widget"`
- **THEN** its change is patch

#### Scenario: Plain upstream subject
- **WHEN** an update brought in an upstream commit with the subject `Add the widget`
- **THEN** its change is patch

### Requirement: Change of an upstream release
Under `semver`, the next release's change SHALL also be major when the commit's upstream release does not satisfy `^<v>`, `<v>` being the previous release's upstream release, both by "Upstream release of a semver release": it is lower than `<v>`, or has another major version, at major version 0 another minor version, at 0.0 another patch version. Without a previous release, or one without an upstream release, the upstream release SHALL change nothing.

#### Scenario: Upstream minor release at major version 0
- **WHEN** the previous release contains upstream `v0.11.0`, and since then only an update to upstream `v0.12.0` landed, whose upstream commits are all `fix` commits
- **THEN** the next release raises the major version

#### Scenario: Upstream patch release
- **WHEN** the previous release contains upstream `v0.11.0`, and since then only an update to upstream `v0.11.3` landed, whose upstream commits are all `fix` commits
- **THEN** the next release raises the patch version

#### Scenario: Upstream release withdrawn
- **WHEN** the previous release records upstream `v0.11.3`, the upstream deletes that tag, and the tip's newest upstream release is `v0.11.0`
- **THEN** the next release raises the major version

#### Scenario: Chores and an incompatible upstream release
- **WHEN** since the previous release, which records upstream `v0.11.0`, only an update to upstream `v0.12.0` landed, whose commits are all `docs` commits
- **THEN** the next release raises the major version

### Requirement: Upstream release of a semver release
Under `semver`, the upstream release of a semver release SHALL be the one its tag's annotation records in the trailers `Intake-Upstream`, `Intake-Target` and `Intake-Tag`, when they name the release upstream and a tag that the release `tags` pattern selects as an upstream release. For a dated release or a tag without all three, it SHALL be the newest upstream release its commit contains, and so SHALL the upstream release of a commit without a release.

#### Scenario: Upstream tag after the release
- **WHEN** a release `v1.0.1` records upstream `v0.11.0`, and the upstream later tags `v0.12.0` on a commit that `v1.0.1` contains
- **THEN** the upstream release of `v1.0.1` stays `v0.11.0`, `release check` and `release version` of `v1.0.1` pass as before, and the next release raises the major version

### Requirement: Recorded upstream release of a tag
When `release check`, `release version` or `release notes` act on a semver release tag whose annotation records an upstream release, it SHALL be a tag of the release upstream that the release `tags` pattern selects, and the tagged commit SHALL contain its recorded commit, which they read locally. `release check` SHALL report a record that does not hold as a problem, and `release version` and `release notes` SHALL refuse with exit status 2.

#### Scenario: Recorded release the commit lacks
- **WHEN** the annotation of `v1.0.0` records upstream `v0.12.0` at a commit that the tagged commit does not contain
- **THEN** `release check` exits with status 1 and names `v0.12.0` and the commit

#### Scenario: Upstream release withdrawn after the release
- **WHEN** the release `v1.0.0` records upstream `v0.11.3`, and the upstream deletes that tag
- **THEN** `release check`, `release version` and `release notes` of `v1.0.0` give what they gave before

### Requirement: Moved upstream release tag
`release check` SHALL also report as a problem a recorded upstream release whose tag the upstream still lists at another commit than the recorded one. A tag the upstream no longer lists SHALL be no problem.

#### Scenario: Upstream tag moved after the release
- **WHEN** the release `v1.0.0` records upstream `v0.11.3`, and the upstream moves that tag to another commit
- **THEN** `release check` exits with status 1 and names both commits, and `release version` and `release notes` give what they gave before

### Requirement: Network reads of release version
Under `semver`, `release version` SHALL read the publish remote's integration branch and tags, and list the release upstream's tags. It SHALL take the last synced commits from the sync records of the commit's history, leaving out the records below the synced commits they give and those whose merge's configuration names the upstream at another URL or branch, and SHALL fetch an upstream only when that history has no such record of it.

#### Scenario: Build of a merge request
- **WHEN** under `semver` a merge request pipeline runs `release version` where the upstream's branch cannot be fetched, and the history has a sync record of every upstream
- **THEN** it prints the rehearsal version

#### Scenario: Fork of a fork
- **WHEN** a downstream was cloned from a fork that follows its own upstream under the same name, and has no sync record of its own
- **THEN** `release version` fetches the upstream, and takes none of the fork's sync records for its own

### Requirement: Next semver release
Under `semver`, a commit's next change SHALL be the largest change of its commits and its upstream release; with none, it SHALL take no release. After a semver release `v<x>.<y>.<z>`, the next release SHALL be `v<x+1>.0.0` for a major change, `v<x>.<y+1>.0` for a minor one and `v<x>.<y>.<z+1>` for a patch. The tool SHALL refuse a next release that breaks "Release order", naming the release it does not come after, or whose name the remote already has.

#### Scenario: Feature since a release
- **WHEN** the previous release is `v1.2.3` and since then `fix: a` and `feat: b` landed
- **THEN** the next release is `v1.3.0`, and its change is minor

### Requirement: First semver release
Under `semver`, when there is no previous release or the previous release is of the dated form, a commit that takes a release SHALL take the first semver release, with a major change: `v1.0.0`, or, when a release on the downstream's line below the commit has a version of 1.0.0 or above by precedence, `v<x+1>.0.0`, `<x>` being the major version of the highest of them.

#### Scenario: First release after dated releases
- **WHEN** `[release]` changes to `versioning = "semver"` with `labels = ["sakiori"]`, the previous release is `v0.11.1-sakiori.20261103.1`, and since then `fix: a` landed
- **THEN** the next release is `v1.0.0`, and its change is major

#### Scenario: Dated releases at 1.0.0 or above
- **WHEN** the previous release is `v1.4.1-custom.20261103.1` and the configuration changes to `versioning = "semver"`
- **THEN** the first semver release is `v2.0.0`

#### Scenario: First release of a line
- **WHEN** under `semver` the downstream's line carries no release and its commits make a change
- **THEN** the next release is `v1.0.0`

### Requirement: Nothing to release
Under `semver`, when the integration tip carries no release and takes none, `release tag` SHALL create nothing, say that the commits since the previous release ask for no release, naming that release, and exit with status 0, warning as "Unpublished release below the tip" says when the previous release is not published. `release next` SHALL report that no release is due.

#### Scenario: Documentation only
- **WHEN** under `semver` only `docs:` and `ci:` commits landed since the previous release
- **THEN** `release tag` pushes nothing and exits with status 0, and `release next --json` carries `bump` null and `next` null

### Requirement: Upstream release in the notes
Under `semver`, the notes SHALL start with a line that names the release upstream and the tag's upstream release by "Upstream release of a semver release", and says how many commits of that upstream the tagged commit carries after it: those the upstream's last synced commit at the tagged commit has and the upstream release lacks, as `status` reports the last synced commit. Without a last synced commit of that upstream, the count SHALL be zero.

#### Scenario: Branch past an upstream release
- **WHEN** a `semver` release contains upstream `v0.11.0` of the upstream `fff` and twelve commits of `fff` after it
- **THEN** the first line of its notes names `fff`, `v0.11.0` and twelve upstream commits after it

### Requirement: Versioning in the release state
`release next --json` SHALL also carry `versioning`, `dated` or `semver`, and `bump`: under `semver` `major`, `minor` or `patch`, the change that sets the next release, and null under `dated`, with a release at the tip, or when no release is due. Under `semver`, `base` SHALL be null, and `next` SHALL be null when no release is due.

#### Scenario: Semver tip
- **WHEN** under `semver` the integration tip carries no release and a `fix:` commit landed since the previous release `v1.2.3`
- **THEN** `versioning` is `semver`, `base` is null, `bump` is `patch` and `next` is `v1.2.4`

### Requirement: Upstream release in a semver tag
Under `semver`, the annotation of the tag `release tag` creates SHALL end with the trailers `Intake-Upstream: <upstream>`, `Intake-Target: <commit>` and `Intake-Tag: <tag>` of the newest upstream release the tip contains, after the message.

#### Scenario: Semver release
- **WHEN** under `semver` the previous release is `v1.2.3` and a `feat:` commit landed since
- **THEN** `release tag` pushes the annotated tag `v1.3.0` with the message `Release 1.3.0` and the trailers of the tip's upstream release

### Requirement: Checking a semver release tag
Under `semver`, `release check` SHALL compute again the version the tagged commit takes, from the releases below it and the upstream releases that they and the tag record.

#### Scenario: Semver pipeline run again after an upstream release
- **WHEN** under `semver` the upstream tagged a release on a commit that the release already contained, and the release's pipeline runs again
- **THEN** `release check` passes, and `release version` prints the same version

### Requirement: Problems of a semver release tag
Under `semver`, the problems of a release tag SHALL also be: it is not of the form `v<x.y.z>`; the upstream release its annotation records does not hold by "Recorded upstream release of a tag"; its commit takes no release; its version is not the version of its commit's next release by "Next semver release".

#### Scenario: Semver tag with the wrong version
- **WHEN** under `semver` the tag `v2.0.0` points at a commit whose next release is `v1.3.0`
- **THEN** `release check` exits with status 1 and names both versions

### Requirement: Upstream of a semver build
Under `semver`, `release version --json` SHALL carry `upstream`, and `--dotenv` the line `RELEASE_UPSTREAM=<upstream>`. `<upstream>` SHALL be the release upstream's name, with every character but ASCII letters, digits and `-` written `-`, and the version of the commit's upstream release by "Upstream release of a semver release", joined by `-`, so that it is valid build metadata.

#### Scenario: Upstream of a semver release
- **WHEN** under `semver` the pipeline of `v1.2.0`, whose commit contains upstream `v0.11.0` of the upstream `fff`, runs `release version --dotenv`
- **THEN** it writes `RELEASE_VERSION=1.2.0`, `RELEASE_TAG=v1.2.0` and `RELEASE_UPSTREAM=fff-0.11.0`

#### Scenario: Upstream name outside build metadata
- **WHEN** under `semver` the release upstream is named `app_core` and the commit's upstream release is `v0.4.0`
- **THEN** `RELEASE_UPSTREAM` is `app-core-0.4.0`

### Requirement: Breaking changes in the notes
Under `semver`, the notes SHALL list the breaking changes first: a line naming the previous and the new upstream release when "Change of an upstream release" made the change major, then the breaking changes among the listed commits, newest first, each with the text of its `BREAKING CHANGE` footer when it has one. The other entries SHALL come after them.

#### Scenario: Breaking change in a semver release
- **WHEN** under `semver` since the previous release `fix: a` and then `feat!: speak protocol 3` with the footer `BREAKING CHANGE: clients need protocol 3` landed
- **THEN** the notes list `feat!: speak protocol 3` with the text `clients need protocol 3` first, and `fix: a` after it

#### Scenario: Incompatible upstream release in the notes
- **WHEN** under `semver` the previous release records upstream `v0.11.0`, and the release records `v0.12.0`
- **THEN** the breaking changes start with a line saying that the upstream moved from `v0.11.0` to `v0.12.0`

## MODIFIED Requirements

### Requirement: Release configuration
The configuration MAY carry a `[release]` table with exactly the keys `upstream`, `tags`, `labels` and `versioning`. `upstream` SHALL name a `follow` upstream. `tags` SHALL be a glob over that upstream's tag names, as for an upstream's `tags`, defaulting to the upstream's own `tags`. `versioning` SHALL follow "Release versioning". The tool SHALL refuse the table when neither sets a pattern, or when the configuration names no publish remote, naming the key with the configuration's other problems.

#### Scenario: Upstream that does not follow
- **WHEN** `[release]` names an upstream in `port` mode, or a name no upstream has
- **THEN** every command refuses the configuration and names `release.upstream`

#### Scenario: No release tags
- **WHEN** `[release]` sets no `tags` and the upstream it names has no `tags` pattern
- **THEN** every command refuses the configuration and names `release.tags`

#### Scenario: Release tags without following them
- **WHEN** `[release]` sets `tags = "v*"` and its upstream sets none
- **THEN** updates take that upstream at its branch tip as before, and releases take their version from its `v*` tags

### Requirement: Release labels
`labels` SHALL be a non-empty list of distinct labels, defaulting to `["custom"]`, each a SemVer prerelease identifier of ASCII letters, digits and `-` that is not only digits. Under `dated`, the first SHALL be the label of new releases, and the others are labels that earlier releases took. Under `semver`, every label SHALL be one that earlier dated releases took.

#### Scenario: Numeric label
- **WHEN** `labels` is `["2024"]`
- **THEN** every command refuses the configuration and names `release.labels`

### Requirement: Release tag names
Under `dated`, a release tag SHALL be named `v<base>-<label>.<YYYYMMDD>.<n>`, where `<base>` is a version `x.y.z` without leading zeros, `<label>` one of the configured labels, `<YYYYMMDD>` a calendar day, and `<n>` a number from 1 to 99 without a leading zero. Its version SHALL be the name without the `v`. Under `semver`, release tags SHALL be named as "Semver release tags" says.

#### Scenario: Day that does not exist
- **WHEN** a tag is named `v0.11.1-custom.20260230.1`
- **THEN** it is not a release tag

#### Scenario: Plain version under dated
- **WHEN** under `dated` the downstream's line carries the tag `v1.0.0`
- **THEN** that tag is no release

### Requirement: Release order
A new release SHALL have a higher precedence by semantic versioning than every other release. Under `dated`, its day and number, compared as `YYYYMMDD * 100 + n`, SHALL also be greater than those of every other release.

#### Scenario: Label that sorts first
- **WHEN** `labels` changes from `["nightly"]` to `["custom", "nightly"]` while a release `v0.1.7-nightly.20261031.1` exists, and a release with base `0.1.7` is cut
- **THEN** the tool refuses, since `v0.1.7-custom.<day>.1` sorts before that release

#### Scenario: Semver release after dated releases
- **WHEN** under `semver` the downstream's line carries `v0.11.1-sakiori.20261103.1` and the next release is `v1.0.0`
- **THEN** `v1.0.0` sorts after that release, and no day or number is compared

### Requirement: Next release
Under `dated`, the next release of a commit SHALL take the commit's release base, the current label, the current UTC day, and the number one past the highest number that any tag of release form of the publish remote took on that day, on the downstream's line or not, whatever its base or label, or else 1. The tool SHALL refuse a next release that breaks "Release order", naming the release it does not come after, on a day that has a tag numbered 99, and whose name the remote already has.

#### Scenario: Second release of a day
- **WHEN** the line carries `v0.11.1-custom.20261101.1` and a release is cut on the same UTC day with the same base
- **THEN** the next release is `v0.11.1-custom.20261101.2`

#### Scenario: Tag of release form off the line
- **WHEN** a tag `v0.11.1-custom.20261101.1` of the publish remote points at a commit off the downstream's line, and a release is cut on 2026-11-01 with the base `0.11.1`
- **THEN** that tag is no release, and the next release is `v0.11.1-custom.20261101.2`

#### Scenario: Base older than a release
- **WHEN** the line carries a release with base `0.12.1` and the tip's release base is `0.11.1`
- **THEN** the tool refuses to name a next release and names the release with base `0.12.1`

### Requirement: Release state of the tip
`release next` SHALL report, for the publish remote's integration tip, the newest upstream release the tip contains and, under `dated`, the release base that comes from it, and either the release at the tip and whether it is published, or the next release, under `semver` with the change that sets it, or under `semver` that no release is due. It SHALL change nothing, and exit with status 0 when it can report and 2 otherwise.

#### Scenario: Tip changed since the last release
- **WHEN** the integration tip carries no release
- **THEN** `release next` names the next release and creates no tag

#### Scenario: Semver tip with a feature
- **WHEN** under `semver` the previous release is `v1.2.3` and a `feat:` commit landed since
- **THEN** `release next` names `v1.3.0` and the minor change, and no release base

### Requirement: Cutting a release
`release tag` SHALL act on the publish remote's integration tip, never on the local checkout. When no release points at the tip and the tip takes a next release, it SHALL create that release as an annotated tag on the tip with the message `Release <version>` and git's configured identity, push only that tag to the publish remote, and exit with status 0. It SHALL never move, replace or delete a tag, locally or on the remote.

#### Scenario: Tip changed
- **WHEN** the integration tip carries no release
- **THEN** `release tag` pushes the next release tag at the tip, and the remote's other tags are unchanged

#### Scenario: Local branch behind
- **WHEN** the local integration branch is behind the publish remote's
- **THEN** the tag points at the remote's tip, and the local branch does not move

### Requirement: Release tag problems
The problems of a release tag SHALL be: it is not of release tag form, under `dated` with the current label; the remote has no such tag; it does not point at the checked-out commit; that commit is not on the downstream's line; under `dated`, its base is not the release base of that commit; it breaks "Release order" against the other releases, among which it does not count itself; it is published; and under `semver` the problems of "Problems of a semver release tag".

#### Scenario: Release already published
- **WHEN** a pipeline of a published release's tag starts again
- **THEN** `release check` exits with status 1 and says the release is published and is never written again

#### Scenario: Tag on a merge request branch
- **WHEN** a release tag points at a commit that is not on the downstream's line
- **THEN** `release check` exits with status 1 and names the commit and the integration branch

### Requirement: Release version for builds
`release version` SHALL print the version a build makes. For a release tag, named by `--tag` or `CI_COMMIT_TAG`, it SHALL be the tag's version once the tag is of release tag form, under `dated` with the current label and a base that is the release base of the tagged commit, and under `semver` with the version of the tagged commit's next release, computed with the tag's upstream release by "Upstream release of a semver release"; otherwise it SHALL refuse with exit status 2.

#### Scenario: Tag pipeline
- **WHEN** the pipeline of `v0.11.1-custom.20261101.2` runs `release version`
- **THEN** it prints `0.11.1-custom.20261101.2`

#### Scenario: Semver tag pipeline
- **WHEN** under `semver` the pipeline of `v1.3.0` runs `release version`
- **THEN** it prints `1.3.0`

### Requirement: Rehearsal version
Without a release tag, `release version` SHALL print the rehearsal version of the checked-out commit, with the UTC day of `CI_PIPELINE_CREATED_AT`, or else of now: under `dated`, `<base>-<label>.<YYYYMMDD>.0` with the current label; under `semver`, `<next>-rehearsal.<YYYYMMDD>`, where `<next>` is the version of the commit's next release, or, when the commit takes no release, the version a patch change would give it. No release SHALL take a rehearsal version.

#### Scenario: Rehearsal
- **WHEN** a pipeline of the integration branch without a tag, created on 2026-11-01, runs `release version` on a commit whose release base is `0.11.1`
- **THEN** it prints `0.11.1-custom.20261101.0`

#### Scenario: Semver rehearsal
- **WHEN** under `semver` a pipeline without a tag, created on 2026-11-01, runs `release version` on a commit whose next release is `v1.3.0`
- **THEN** it prints `1.3.0-rehearsal.20261101`

### Requirement: Release version formats
`release version --json` SHALL write an object with `version`, `tag` (or null) and `rehearsal`. `release version --dotenv` SHALL write the line `RELEASE_VERSION=<version>` and, for a release tag, `RELEASE_TAG=<tag>`. Under `semver`, both SHALL also carry the upstream of "Upstream of a semver build".

#### Scenario: Dotenv in a tag pipeline
- **WHEN** the pipeline of `v0.11.1-custom.20261101.2` runs `release version --dotenv`
- **THEN** it writes `RELEASE_VERSION=0.11.1-custom.20261101.2` and `RELEASE_TAG=v0.11.1-custom.20261101.2`

### Requirement: Release notes entries
The notes SHALL list, newest first, the subject of each commit of that range that carries no sync record and is neither a catch-up merge nor a merge that brought sync records in, and for each sync record there the upstream's name and its recorded tag, or else the target's abbreviated commit. Under `semver`, they SHALL be ordered as "Breaking changes in the notes" says.

#### Scenario: Feature and update
- **WHEN** since the previous release one squashed feature and one update of the upstream `fff` to its branch tip landed
- **THEN** the notes list the feature's subject and the update of `fff` with the target's abbreviated commit, and none of the upstream's own commits

### Requirement: Release state in status
When the configuration carries `[release]`, `status` SHALL show what `release next` reports, and `status --json` SHALL carry it as `release`, the object of `release next --json`; without `[release]`, `release` SHALL be null. A forge that cannot be read, or that hides the project, SHALL leave `published` null and not change the exit status of `status`.

#### Scenario: Release state without the forge
- **WHEN** `status --json` runs where the forge cannot be reached
- **THEN** `release.published` is null, `release.forge_error` says why, and the exit status is the one the upstreams give

#### Scenario: Semver release state in status
- **WHEN** under `semver` only `docs:` commits landed since the previous release
- **THEN** `status` says that no release is due, and `status --json` carries `release.next` null and `release.bump` null
