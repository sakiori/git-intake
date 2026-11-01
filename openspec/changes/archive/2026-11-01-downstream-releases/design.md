## Context

Downstream repositories kept current with git-intake were released by a separate set of GitLab CI components: a scheduled job tagged the default branch's tip as `v<upstream>-<label>.<YYYYMMDD>.<n>` when no release tag pointed at it, a guard checked the tag pipeline, a job handed builds the version, and a last job created the GitLab Release with git-cliff notes. Half of that code re-derived what git-intake owns: the integration line, the sync records and the upstream's version. The rest wrapped forge features that exist on their own.

## Decisions

### Releases belong to git-intake

The version of a downstream release is a statement about its upstream, and git-intake is the tool that knows which upstream commits the integration line contains. A separate tool would read the sync records and the upstream's tags a second time or depend on git-intake's JSON as a contract between two programs released apart.

Debian's git-buildpackage sets the precedent: one tool imports upstream sources and tags downstream releases whose versions combine the upstream version with a downstream revision.

Alternatives: a separate CLI beside git-intake, which duplicates the integration line and the forge layer; keeping CI components, which tie releases to one forge and to a container image of their own.

### The base is the next patch after the newest contained upstream release

Downstreams usually follow their upstream's branch. The newest upstream release the integration tip contains is the last version the downstream is known to be at or past. Its next patch version, with a prerelease label, sorts after that release and before the next patch release, which is what the shipped code is: the same rule upstreams apply to their own builds between releases (`0.11.1-nightly.<commit>` after `0.11.0`).

A manifest the upstream commits is not a reliable source. An upstream may never update the version it commits between releases, so a downstream reading it can report a version several releases old. The `Intake-Tag` of the last sync record is not enough either: it is empty whenever an update took the branch tip.

`[release].tags` is separate from the upstream's `tags`, because the upstream's `tags` also makes updates take only releases. A downstream that follows the branch sets the pattern under `[release]` only.

When the upstream's branch already prepares a minor or major release, the base still names the next patch. It stays in order with every upstream release: after the one contained, before the next one of any kind.

### Two orders, both enforced

A release must sort after every earlier release by semantic versioning, so that tools that pick the newest version, Renovate and package managers among them, see each release as newer. Its day and number must also grow, so that `YYYYMMDD * 100 + n` serves as a monotonic build number such as an Android version code. A changed label therefore has to sort after the labels released before it under the same base.

### A release is published when the forge has a release for its tag

Earlier, completeness was the presence of one file a project published last, in its package registry or at an origin URL, configured per project in one of two forms. A forge release created as the last step of the pipeline expresses the same thing in one form for every project, and both forges answer it by tag. It also makes the forge's release page the single place a consumer looks.

The cost: a project must create its forge release last. A project that creates it earlier would mark a release published before its artifacts exist.

### A release left unpublished fails the scheduled run

A release tag on the tip without a published release means its pipeline is running or failed. Skipping silently, as the predecessor did, hid a failed release for as long as the tip did not move. `release tag` now fails and names the tag. It does not run the pipeline again itself: a failed release is completed by running its failed jobs again, which is a decision for the project's maintainers.

### Notes from the integration line, not git-cliff

The downstream's own changes are the commits on the first-parent line between releases: squashed features and fixes, and update merges that carry sync records. git-intake reads both directly and describes updates by upstream and tag. git-cliff would see every upstream commit in the range and needed an ignore list built from the same knowledge.

The notes do not group commits by Conventional Commit type. Downstream subjects describe the resulting behaviour and read as a list.

### Configuration read where its commit is

Cutting a release acts on the publish remote's tip, so `release tag`, `release next` and `status` read the configuration there. A tag pipeline checks, versions and describes one commit, so `release check`, `release version` and `release notes` read the configuration of that commit. A tag pipeline run again later behaves as it did, whatever changed on the branch since.

### Concurrency through the remote

Two runs that compute the same next tag race on the push. The tool pushes the tag only where the remote lacks it, so the loser fails without effect. No CI-side lock is needed for correctness.

## Not carried over

- **Creating the forge release.** GitLab's `release` keyword, `glab release create` and `gh release create` do it from the notes file; the tool only reads releases.
- **Version from the downstream's own Conventional Commits.** release-plz and its peers release a project's own product.
- **Version from a committed manifest**, including the line reader for Cargo manifests. The base comes from upstream tags.
- **Completeness by package-registry file or origin URL.** A forge release replaces both.
- **A configurable daily maximum.** Releases of a day are capped at 99, the limit of the build number.
- **A history start**, a commit configured by hand before which release tags are ignored. See below.

### Releases only on the downstream's own line

A downstream created from its upstream's history carries the upstream's commits on its first-parent line below its own first commit, and often the upstream's tags with them. An upstream that tags its own builds as `v<x.y.z>-nightly.<YYYYMMDD>.<n>` produces tags of release form there. The predecessor excluded them by a commit configured by hand. git-intake knows the boundary: every upstream commit on the first-parent line is contained in that upstream's last synced commit, and no downstream commit is. Releases count only on the commits no last synced commit contains, so the boundary needs no setting and moves with nothing.

## Risks

- A forge reachable only with a token the tag pipeline lacks makes `release check` refuse instead of passing. The refusal names the token variables.
- Upstream tag listing contacts the upstream on every release command. An unreachable upstream stops the scheduled run with git's error, as it stops `sync`.
- A release whose commit later becomes contained in an upstream's last synced commit, as when an upstream merges the downstream's commits unchanged and the downstream takes that upstream again, leaves the downstream's line: it stops counting as a release and takes no part in the order check, so a later release need not sort after it.
