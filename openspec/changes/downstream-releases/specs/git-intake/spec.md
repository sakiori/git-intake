## ADDED Requirements

### Requirement: Release configuration
The configuration MAY carry a `[release]` table with exactly the keys `upstream`, `tags` and `labels`. `upstream` SHALL name a `follow` upstream. `tags` SHALL be a glob over that upstream's tag names, as for an upstream's `tags`, defaulting to the upstream's own `tags`. The tool SHALL refuse the table when neither sets a pattern, or when the configuration names no publish remote, naming the key with the configuration's other problems.

#### Scenario: Upstream that does not follow
- **WHEN** `[release]` names an upstream in `port` mode, or a name no upstream has
- **THEN** every command refuses the configuration and names `release.upstream`

#### Scenario: No release tags
- **WHEN** `[release]` sets no `tags` and the upstream it names has no `tags` pattern
- **THEN** every command refuses the configuration and names `release.tags`

#### Scenario: Release tags without following them
- **WHEN** `[release]` sets `tags = "v*"` and its upstream sets none
- **THEN** updates take that upstream at its branch tip as before, and releases take their base from its `v*` tags

### Requirement: Release labels
`labels` SHALL be a non-empty list of distinct labels, defaulting to `["custom"]`, each a SemVer prerelease identifier of ASCII letters, digits and `-` that is not only digits. The first SHALL be the label of new releases; the others are labels that earlier releases took.

#### Scenario: Numeric label
- **WHEN** `labels` is `["2024"]`
- **THEN** every command refuses the configuration and names `release.labels`

### Requirement: Upstream releases of a commit
An upstream release SHALL be a tag of the release upstream's repository that matches the release `tags` pattern and is a semantic version without a prerelease part or build metadata once the pattern's literal prefix, and one `v` or `V` after it unless the prefix ends in one, are removed. A commit SHALL contain it when its commit exists in the repository and is the commit or an ancestor of it. The tool SHALL list the upstream's tags from its URL and fetch none of them.

#### Scenario: Prefixed release tags
- **WHEN** the release `tags` pattern is `expo-device-hub@*` and the upstream has the tag `expo-device-hub@0.19.1`
- **THEN** that tag is an upstream release of version `0.19.1`

#### Scenario: Prerelease of the upstream
- **WHEN** the upstream has the tag `v0.1.7-nightly.20261031.3991` on a commit the integration tip contains
- **THEN** that tag is no upstream release

### Requirement: Release base
The release base of a commit SHALL be the version of the newest upstream release, by semantic versioning precedence, that the commit contains, with its patch number increased by one and nothing else changed. The tool SHALL refuse, naming the upstream, its URL and the pattern, when the commit contains no upstream release.

#### Scenario: Branch past a release
- **WHEN** the integration tip contains upstream `v0.11.0` and twelve upstream commits after it, and no later release
- **THEN** the release base is `0.11.1`

#### Scenario: Exactly at a release
- **WHEN** the integration tip took the upstream at `v0.11.0` and carries downstream commits on top
- **THEN** the release base is `0.11.1`

#### Scenario: Upstream release not taken
- **WHEN** the upstream has released `v0.12.0` and the integration tip does not contain it
- **THEN** the release base comes from the newest release the tip contains

### Requirement: Release tag names
A release tag SHALL be named `v<base>-<label>.<YYYYMMDD>.<n>`, where `<base>` is a version `x.y.z` without leading zeros, `<label>` one of the configured labels, `<YYYYMMDD>` a calendar day, and `<n>` a number from 1 to 99 without a leading zero. Its version SHALL be the name without the `v`.

#### Scenario: Day that does not exist
- **WHEN** a tag is named `v0.11.1-custom.20260230.1`
- **THEN** it is not a release tag

### Requirement: Downstream line
The downstream's line SHALL be the commits of the first-parent line of the publish remote's integration branch that the last synced commit of no upstream contains, the last synced commit being the one `status` reports. A release SHALL be a tag of release tag form of the publish remote that points, peeled through an annotated tag, at a commit of the downstream's line. Other tags SHALL be no releases.

#### Scenario: Release of an earlier label
- **WHEN** `labels` is `["nightly", "custom"]` and the downstream's line carries `v0.1.6-custom.20261012.3`
- **THEN** that tag is a release, and new releases take the label `nightly`

#### Scenario: Upstream tags of release form
- **WHEN** the repository carries the upstream's own tags such as `v0.0.21-nightly.20260419.73` on upstream commits of the first-parent line before the downstream's first commit
- **THEN** none of them is a release, and they take no part in the order of releases

#### Scenario: Tag off the line
- **WHEN** a tag of release form points at a commit that a merge brought into the integration branch through its second parent
- **THEN** that tag is no release

### Requirement: Release order
A new release SHALL have a higher precedence by semantic versioning than every other release, and its day and number, compared as `YYYYMMDD * 100 + n`, SHALL be greater than those of every other release.

#### Scenario: Label that sorts first
- **WHEN** `labels` changes from `["nightly"]` to `["custom", "nightly"]` while a release `v0.1.7-nightly.20261031.1` exists, and a release with base `0.1.7` is cut
- **THEN** the tool refuses, since `v0.1.7-custom.<day>.1` sorts before that release

### Requirement: Next release
The next release of a commit SHALL take the commit's release base, the current label, the current UTC day, and the number one past the highest number that any tag of release form of the publish remote took on that day, on the downstream's line or not, whatever its base or label, or else 1. The tool SHALL refuse a next release that breaks "Release order", naming the release it does not come after, on a day that has a tag numbered 99, and whose name the remote already has.

#### Scenario: Second release of a day
- **WHEN** the line carries `v0.11.1-custom.20261101.1` and a release is cut on the same UTC day with the same base
- **THEN** the next release is `v0.11.1-custom.20261101.2`

#### Scenario: Tag of release form off the line
- **WHEN** a tag `v0.11.1-custom.20261101.1` of the publish remote points at a commit off the downstream's line, and a release is cut on 2026-11-01 with the base `0.11.1`
- **THEN** that tag is no release, and the next release is `v0.11.1-custom.20261101.2`

#### Scenario: Base older than a release
- **WHEN** the line carries a release with base `0.12.1` and the tip's release base is `0.11.1`
- **THEN** the tool refuses to name a next release and names the release with base `0.12.1`

### Requirement: Published release
A release SHALL count as published when the forge of the publish remote has a release whose tag is the release tag. A project is expected to create that forge release as the last step of its release, after everything it publishes.

#### Scenario: Tag without a forge release
- **WHEN** a release tag exists and the forge has no release for it
- **THEN** the release is not published

### Requirement: Release state of the tip
`release next` SHALL report, for the publish remote's integration tip, the release base and the upstream release it comes from, and either the release at the tip and whether it is published, or the next release. It SHALL change nothing, and exit with status 0 when it can report and 2 otherwise.

#### Scenario: Tip changed since the last release
- **WHEN** the integration tip carries no release
- **THEN** `release next` names the next release and creates no tag

### Requirement: Release state in JSON
`release next --json` SHALL write one object with `tip`, `base`, `upstream_release` (the tag of the upstream release the base comes from), `release` (the release at the tip, or null), `published` (a boolean, or null without a release at the tip or a readable forge), `next` (the next release, or null), `unpublished_previous` (see "Unpublished release below the tip", or null) and `forge_error` (why the forge could not be read, or null).

#### Scenario: Released tip
- **WHEN** the integration tip carries a published release
- **THEN** `release` names it, `published` is true, and `next` is null

### Requirement: Unpublished release below the tip
When the integration tip carries no release, `release next` and `status` SHALL ask the forge whether the newest release on the downstream's line below the tip is published, and name it as `unpublished_previous` when it is not. `release tag` SHALL cut the next release all the same, and warn, naming that release and how a failed release is completed.

#### Scenario: Superseded failure
- **WHEN** the release below the tip was never published and the tip has moved on
- **THEN** `release next` names it as `unpublished_previous`, and `release tag` pushes the next release, warns naming it, and exits with status 0

### Requirement: Cutting a release
`release tag` SHALL act on the publish remote's integration tip, never on the local checkout. When no release points at the tip, it SHALL create the next release as an annotated tag on the tip with the message `Release <version>` and git's configured identity, push only that tag to the publish remote, and exit with status 0. It SHALL never move, replace or delete a tag, locally or on the remote.

#### Scenario: Tip changed
- **WHEN** the integration tip carries no release
- **THEN** `release tag` pushes the next release tag at the tip, and the remote's other tags are unchanged

#### Scenario: Local branch behind
- **WHEN** the local integration branch is behind the publish remote's
- **THEN** the tag points at the remote's tip, and the local branch does not move

### Requirement: Released tip
When a published release points at the integration tip, `release tag` SHALL create nothing, say so and exit with status 0.

#### Scenario: Nothing new
- **WHEN** the integration tip carries a published release
- **THEN** `release tag` pushes nothing and exits with status 0

### Requirement: Unpublished release on the tip
When a release points at the integration tip and is not published, `release tag` SHALL create nothing and exit with status 1, naming the tag and saying that its pipeline is still running or failed, and that a failed release is completed by running its pipeline's failed jobs again.

#### Scenario: Failed release
- **WHEN** the tag pipeline of the release at the tip failed before it created the forge release, and the scheduled job runs `release tag` the next day
- **THEN** the job fails, naming the release that is not published

### Requirement: Release tag push
Pushing a release tag SHALL obey "Pushes never reach an upstream". The push SHALL carry that tag alone, whatever git's push configuration says, and create it only if the remote lacks it; when another run created a tag of that name first, `release tag` SHALL report that with exit status 2 and the advice to run it again, and push nothing.

#### Scenario: Two runs at once
- **WHEN** two runs of `release tag` name the same next release and the other run pushes first
- **THEN** this run exits with status 2, and the remote's tag is the other run's

#### Scenario: Tags that follow a push
- **WHEN** git's configuration sets `push.followTags` and the repository has an annotated tag the remote lacks
- **THEN** `release tag` pushes the release tag alone

#### Scenario: Publish remote that names the upstream
- **WHEN** the publish remote's push URL names the release upstream's repository
- **THEN** `release tag` refuses, and no tag reaches the upstream's repository

### Requirement: Checking a release tag
`release check [<tag>]` SHALL check the tag named, or else `CI_COMMIT_TAG`, against the publish remote's integration branch and release tags. It SHALL report every problem that "Release tag problems" lists, and exit with status 1 when it found one and 0 otherwise. It SHALL ask whether the release is published only when it found no other problem.

#### Scenario: Pipeline run again
- **WHEN** a release's pipeline failed before it created the forge release, and its failed jobs run again
- **THEN** `release check` passes

### Requirement: Release tag problems
The problems of a release tag SHALL be: it is not of release tag form with the current label; the remote has no such tag; it does not point at the checked-out commit; that commit is not on the downstream's line; its base is not the release base of that commit; it breaks "Release order" against the other releases, among which it does not count itself; it is published.

#### Scenario: Release already published
- **WHEN** a pipeline of a published release's tag starts again
- **THEN** `release check` exits with status 1 and says the release is published and is never written again

#### Scenario: Tag on a merge request branch
- **WHEN** a release tag points at a commit that is not on the downstream's line
- **THEN** `release check` exits with status 1 and names the commit and the integration branch

### Requirement: Release version for builds
`release version` SHALL print the version a build makes. For a release tag, named by `--tag` or `CI_COMMIT_TAG`, it SHALL be the tag's version once the tag is of release tag form with the current label and its base is the release base of the tagged commit; otherwise it SHALL refuse with exit status 2.

#### Scenario: Tag pipeline
- **WHEN** the pipeline of `v0.11.1-custom.20261101.2` runs `release version`
- **THEN** it prints `0.11.1-custom.20261101.2`

### Requirement: Rehearsal version
Without a release tag, `release version` SHALL print the rehearsal version `<base>-<label>.<YYYYMMDD>.0` of the checked-out commit, with the current label and the UTC day of `CI_PIPELINE_CREATED_AT`, or else of now. No release SHALL take a rehearsal version.

#### Scenario: Rehearsal
- **WHEN** a pipeline of the integration branch without a tag, created on 2026-11-01, runs `release version` on a commit whose release base is `0.11.1`
- **THEN** it prints `0.11.1-custom.20261101.0`

### Requirement: Release version formats
`release version --json` SHALL write an object with `version`, `tag` (or null) and `rehearsal`. `release version --dotenv` SHALL write the line `RELEASE_VERSION=<version>` and, for a release tag, `RELEASE_TAG=<tag>`.

#### Scenario: Dotenv in a tag pipeline
- **WHEN** the pipeline of `v0.11.1-custom.20261101.2` runs `release version --dotenv`
- **THEN** it writes `RELEASE_VERSION=0.11.1-custom.20261101.2` and `RELEASE_TAG=v0.11.1-custom.20261101.2`

### Requirement: Release notes
`release notes [<tag>]`, for the tag named or else `CI_COMMIT_TAG`, SHALL write Markdown notes for the integration line after the previous release up to the tag's commit. The previous release SHALL be the newest release before the tag on the downstream's line ending at the tag's commit.

#### Scenario: Notes in a tag pipeline
- **WHEN** the pipeline of a release tag runs `release notes` without an argument
- **THEN** it writes the notes of the tag `CI_COMMIT_TAG` names

### Requirement: Release notes entries
The notes SHALL list, newest first, the subject of each commit of that range that carries no sync record and is neither a catch-up merge nor a merge that brought sync records in, and for each sync record there the upstream's name and its recorded tag, or else the target's abbreviated commit.

#### Scenario: Feature and update
- **WHEN** since the previous release one squashed feature and one update of the upstream `fff` to its branch tip landed
- **THEN** the notes list the feature's subject and the update of `fff` with the target's abbreviated commit, and none of the upstream's own commits

### Requirement: Notes without changes
Without a previous release, the notes SHALL say that this is the first release. With nothing to list, they SHALL say that there are no changes since the previous release, naming it.

#### Scenario: First release
- **WHEN** `release notes` runs for the first release on the downstream's line
- **THEN** the notes say that this is the first release

### Requirement: Release state in status
When the configuration carries `[release]`, `status` SHALL show what `release next` reports, and `status --json` SHALL carry it as `release`, the object of `release next --json`; without `[release]`, `release` SHALL be null. A forge that cannot be read, or that hides the project, SHALL leave `published` null and not change the exit status of `status`.

#### Scenario: Release state without the forge
- **WHEN** `status --json` runs where the forge cannot be reached
- **THEN** `release.published` is null, `release.forge_error` says why, and the exit status is the one the upstreams give

### Requirement: Release state errors in status
When `status` cannot read the release state, as when the publish remote cannot be fetched or its tip contains no upstream release, `status --json` SHALL carry `release` null and `release_error` saying why, and the exit status SHALL be the one the upstreams give. Otherwise `release_error` SHALL be null.

#### Scenario: Tip without an upstream release
- **WHEN** `status --json` runs with `[release]` and the publish remote's tip contains no upstream release
- **THEN** `release` is null, `release_error` names the upstream and the pattern, and the exit status is the one the upstreams give

### Requirement: Forge access for releases
The release commands SHALL reach the forge of the publish remote, in either publication mode, only to read the project and its releases. For GitLab they SHALL send `GITLAB_TOKEN`, or else, in a GitLab CI job whose server has the remote's host, `CI_JOB_TOKEN` as a job token; for GitHub the variables of "Forge token". Without a token they SHALL ask without credentials.

#### Scenario: Job token in a tag pipeline
- **WHEN** `release check` runs in a GitLab tag pipeline where only `CI_JOB_TOKEN` is set
- **THEN** it reads the project's release for the tag with the job token

### Requirement: Project hidden from the release commands
When the forge does not show the release commands the project, they SHALL refuse and name the token variables, so that a project hidden from them never reads as a project without releases.

#### Scenario: Private project without a token
- **WHEN** `release next` runs without a token against a private project
- **THEN** it refuses, names the token variables, and reports no release as unpublished

### Requirement: Configuration of the release commands
`release next`, `release tag` and `status` SHALL read the configuration at the publish remote's integration tip, and `release check`, `release version` and `release notes` at the checked-out commit.

#### Scenario: Tag pipeline run again later
- **WHEN** the integration branch changed `[release]` after a release, and that release's pipeline runs again
- **THEN** `release check` and `release version` use the configuration of the tagged commit

### Requirement: Release commands in CI
The release commands SHALL need no local integration branch. They SHALL fetch the publish remote's integration branch and its release tags into references of their own, applying none of the remote's configured refspecs, and leave the repository's branches and tags as they were, except for the tag `release tag` creates.

#### Scenario: Refspecs onto local refs
- **WHEN** the publish remote's configured fetch refspecs map its tags onto the repository's tags and its branches onto local branches
- **THEN** the release commands change no local tag or branch

#### Scenario: Detached tag checkout
- **WHEN** `release check` runs in a CI clone with `HEAD` detached at a tag and no local integration branch
- **THEN** it checks the tag against the publish remote's integration branch and creates no local branch

## MODIFIED Requirements

### Requirement: Fork configuration
The tool SHALL read a repository's settings from a `.git-intake.toml` file committed on the integration branch: the integration branch, an optional publication setting with the publish remote, the mode `push` or `merge-request`, and, for merge-request mode or a release table, an optional forge `gitlab` or `github` and an optional API base URL, an install command, an ordered list of named checks as "Named checks" describes them, an ordered list of upstreams, and an optional release table as "Release configuration" describes it.

#### Scenario: Valid configuration
- **WHEN** the integration branch carries a valid configuration
- **THEN** every command uses its values and `status` shows them

#### Scenario: Publication setting
- **WHEN** the configuration names a publish remote without a mode
- **THEN** the tool publishes by `push`

### Requirement: Forge of the publish remote
In merge-request mode, and for the release commands in either mode, the tool SHALL reach the forge of the publish remote's fetch URL, GitLab or GitHub, through its REST API, reading the publication setting from the configuration at the integration tip, never at an update's head. It SHALL take the forge from the setting `forge`; otherwise from the URL's host, `gitlab.com` as GitLab and `github.com` or a `ghe.com` subdomain as GitHub; otherwise from a GitLab CI job or GitHub Actions job whose server URL has that host.

#### Scenario: Forge recognised from the URL
- **WHEN** the publish remote's URL is `git@github.com:team/app.git` and merge-request mode is configured
- **THEN** the tool treats the publish remote as the GitHub repository `team/app`

#### Scenario: Unknown host
- **WHEN** the publish remote is on a host the tool cannot recognise, such as an SSH host alias, and the configuration sets no `forge`
- **THEN** the commands that need the forge refuse, push nothing, and name the settings `forge` and `api`

#### Scenario: Setting changed by an update
- **WHEN** an update's head changes the publication setting's `api` and the integration tip does not
- **THEN** the tool uses the setting at the integration tip

### Requirement: Release tags as follow targets
For a `follow` upstream with a `tags` pattern named without a revision, `start` SHALL list the tags of the upstream's repository from its URL and take as the target the newest release among them. An update SHALL fetch no upstream tag into the repository. The release commands SHALL fetch the publish remote's release tags only into references of their own, and create no tag but the release tag.

#### Scenario: Repository tags untouched
- **WHEN** an update takes an upstream at a release tag
- **THEN** the repository's tags are the same before and after the update

### Requirement: Revision of a follow target
A revision named to `start` for a `follow` upstream, without a leading `refs/tags/` or `tags/`, SHALL name a tag of the upstream's repository when it has a tag of that name, and otherwise resolve as a revision of the repository. `start` SHALL fetch no upstream tag into the repository, and SHALL refuse a revision that resolves through one of the repository's own tags. `start` SHALL list the tags of every selected `follow` upstream from its URL, and refuse when it cannot.

#### Scenario: Chosen tag
- **WHEN** the revision is a tag of the upstream's repository on the upstream's branch, and the repository holds a tag of the same name on another commit
- **THEN** the commit of the upstream's tag is the target, and no tag is added to the repository

#### Scenario: Tag named with its prefix
- **WHEN** the revision is `refs/tags/<tag>` or `tags/<tag>` and the upstream's repository has the tag
- **THEN** the commit of the upstream's tag is the target

#### Scenario: Own tag instead of an upstream tag
- **WHEN** the upstream's repository has no tag of the revision's name, and the revision resolves through a tag of the repository, by itself or in an expression, even when a branch has the same name
- **THEN** `start` refuses and names the repository's tag, and creates nothing

#### Scenario: Expression on an upstream tag
- **WHEN** the revision is an expression on a tag that only the upstream's repository has, such as `<tag>~1`
- **THEN** `start` refuses because the revision names no commit in the repository, and creates nothing

#### Scenario: Upstream unreachable for the tag lookup
- **WHEN** the upstream's URL cannot be asked for its tags after the fetch
- **THEN** `start` refuses, names the upstream and its URL with git's error, and creates nothing

### Requirement: Forge token
Forge tokens SHALL come from the environment only: `GITLAB_TOKEN` for GitLab; `GH_TOKEN`, then `GITHUB_TOKEN`, for `github.com` and `ghe.com` hosts; `GH_ENTERPRISE_TOKEN`, then `GITHUB_ENTERPRISE_TOKEN`, for another GitHub host, then in an Actions job on it `GH_TOKEN` and `GITHUB_TOKEN`. Without one, merge requests SHALL be refused before any push, naming the variables; release reads follow "Forge access for releases". A token SHALL never be printed, stored, or sent to any host but the API's.

#### Scenario: No token
- **WHEN** merge-request mode is configured and none of the token variables of the forge is set
- **THEN** `finish` and `sync` refuse before pushing anything and name the variables

#### Scenario: Enterprise host outside Actions
- **WHEN** the publish remote is on a GitHub Enterprise Server host, only `GH_TOKEN` is set, and the tool runs outside GitHub Actions
- **THEN** the tool refuses and names `GH_ENTERPRISE_TOKEN` and `GITHUB_ENTERPRISE_TOKEN`
