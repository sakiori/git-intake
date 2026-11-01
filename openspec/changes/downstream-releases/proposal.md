## Why

A downstream repository that git-intake keeps current also has to be released, and its version has to say which upstream it carries. No established release tool does that: semantic-release, release-please, release-plz and changesets compute the next version from the repository's own commits, and git-cliff only bumps along Conventional Commits. Downstreams therefore end up with release scripts of their own that re-derive what git-intake already knows: the integration line, the sync records and the upstream's release tags.

Those scripts also get the version wrong where it matters most. A downstream usually follows its upstream's branch, not its releases, so the code it ships is newer than the upstream's last release. Taking the version from a manifest the upstream commits gives a number that may lag several releases behind, and calling the result `<release>-<label>` makes it sort before the release it already contains.

And a scheduled tagger that skips whenever a release tag sits on the tip hides a failed release: the tag stays, nothing is published, and every later scheduled run reports success.

## What changes

- An optional `[release]` section in `.git-intake.toml` makes the integration branch releasable. It names the upstream whose version the releases carry, the glob that selects that upstream's release tags (by default the upstream's own `tags` pattern), and the prerelease label of the downstream's releases, with labels used earlier.
- A release version is `<base>-<label>.<YYYYMMDD>.<n>`, where `<base>` is the next patch version after the newest upstream release the integration tip contains, `<YYYYMMDD>` the UTC day and `<n>` one past the highest number any release of that day took. Releases sort after every earlier release, and by day and number as well, so that `YYYYMMDD * 100 + n` grows with every release.
- New commands under `git intake release`:
  - `next` shows the release a tag cut now would take, or that the tip is released already;
  - `tag` cuts the next release tag on the publish remote's integration tip when the tip carries none, does nothing when the tip carries a release that is published, and fails when the tip carries a release tag whose release was never published;
  - `check` guards a tag pipeline: the tag is well formed, points at the pipeline's commit on the first-parent integration line, carries the base that commit has, sorts after every other release, and is not published yet;
  - `version` gives builds the version, in a tag pipeline from its tag and otherwise as a rehearsal version no release can take, as text, JSON or a dotenv report;
  - `notes` writes the release notes: the downstream's own commits and the upstream updates since the previous release.
- A release is published when the forge has a release for its tag. The project's pipeline creates that forge release as its last step, with GitLab's `release` keyword, `glab release create` or `gh release create`, from the notes `git intake release notes` writes.
- `status` reports the release state.
- The tool's description widens from keeping a repository current to keeping it current and releasing it. The statements that the repository's tags never change are scoped to updates: release commands create tags and never move or delete one.

Out of scope: creating the forge release, building and uploading artifacts, and the scheduling itself, which stay in the project's CI; releases of a project's own product versioned by its own commits, which release-plz and its peers serve.

## Capabilities

### New capabilities

None.

### Modified capabilities

- `git-intake`: the fork configuration gains the release section; new requirements for release versions, cutting, checking and versioning release tags, release notes, release state in `status`, and the forge requests they need; the requirement on release tags as follow targets scopes "repository tags untouched" to updates. The capability's Purpose gains releasing when the change is archived.

## Impact

- Configuration: `[release]` is a new table. Releases before this one refuse a configuration that contains it, as an unknown key; every place that runs git intake on the repository, CI jobs included, upgrades before the section is committed.
- Forge: the tool reads forge releases by tag, on GitLab with `GITLAB_TOKEN` or, in a GitLab CI job of the same instance, `CI_JOB_TOKEN`, and on GitHub with the existing token variables. Pushing a release tag needs a credential whose pushes start pipelines, as `sync` does.
- Downstream projects that release through a separate set of CI components replace them: the scheduled job runs `git intake release tag`, the tag pipeline starts with `git intake release check`, builds take `git intake release version --dotenv`, and the last job creates the forge release from `git intake release notes`. Existing release tags of the form `v<x.y.z>-<label>.<YYYYMMDD>.<n>` remain valid releases; their upstream part is not re-checked.
- `status --json` gains `release`; other fields keep their meaning.
