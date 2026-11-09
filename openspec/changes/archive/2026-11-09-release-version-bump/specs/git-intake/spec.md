## ADDED Requirements

### Requirement: Change of a semver build
Under `semver`, `release version --json` SHALL also carry `previous`, the tag of the previous release of the versioned commit or null when it has none, and `bump`, the change that sets that commit's next release by "Next semver release", `"major"`, `"minor"` or `"patch"`, or null when the commit takes no release. The versioned commit SHALL be the tagged commit for a release tag and the checked-out commit for a rehearsal.

#### Scenario: Change of a semver rehearsal
- **WHEN** under `semver` the previous release of the checked-out commit is `v1.2.3`, `feat: b` landed since, and a pipeline without a tag created on 2026-11-01 runs `release version --json`
- **THEN** the object has `version` `1.3.0-rehearsal.20261101`, `previous` `v1.2.3` and `bump` `"minor"`

#### Scenario: Rehearsal without a release due
- **WHEN** under `semver` only `docs: a` landed since the previous release `v1.2.3`, and a pipeline without a tag created on 2026-11-01 runs `release version --json`
- **THEN** the object has `version` `1.2.4-rehearsal.20261101`, `previous` `v1.2.3` and `bump` null

#### Scenario: Change of an incompatible upstream release
- **WHEN** under `semver` the previous release `v1.0.0` records upstream `v0.11.0`, and an update since then took upstream `v0.12.0` with only `docs:` commits
- **THEN** the object has `previous` `v1.0.0` and `bump` `"major"`

#### Scenario: Change of a semver release tag
- **WHEN** under `semver` the pipeline of `v2.0.0`, whose previous release is `v1.4.1`, runs `release version --json`
- **THEN** the object has `previous` `v1.4.1` and `bump` `"major"`

#### Scenario: Change of a line's first release
- **WHEN** under `semver` the checked-out commit has no release below it on the downstream's line and `fix: a` landed
- **THEN** `previous` is null and `bump` is `"major"`, the change of "First semver release"

#### Scenario: Change after dated releases
- **WHEN** `[release]` changes to `versioning = "semver"`, the previous release of the checked-out commit is `v0.11.1-custom.20261101.1`, and since then `fix: a` landed
- **THEN** `previous` is `v0.11.1-custom.20261101.1` and `bump` is `"major"`

#### Scenario: Dated object
- **WHEN** under `dated` a pipeline runs `release version --json`
- **THEN** the object has neither `previous` nor `bump`
