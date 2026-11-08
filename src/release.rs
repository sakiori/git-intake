//! Releases of the integration branch: their tag names and order, the base
//! version a commit takes from the release upstream, the downstream's own
//! line on which releases count, and the release tags of the publish remote.

use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result, bail};
use jiff::Timestamp;
use jiff::civil::Date;
use semver::{Prerelease, Version};

use crate::bump::{self, Bump};
use crate::config::{self, Config, Upstream, Versioning};
use crate::drift;
use crate::git::{Git, abbrev};
use crate::sync_record;
use crate::tags::{TagPattern, UpstreamTags};
use crate::upstreams::{Fetches, Recorded, TagRef};

/// The highest number a release of one day takes, so that
/// `YYYYMMDD * 100 + n` stays a build number.
pub const MAX_PER_DAY: u32 = 99;

/// A release tag: `v<base>-<label>.<YYYYMMDD>.<n>` or `v<x.y.z>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseTag {
    pub name: String,
    pub version: Version,
    /// The parts of a dated tag; `None` for `v<x.y.z>`.
    pub dated: Option<Dated>,
}

/// The parts of a tag `v<base>-<label>.<YYYYMMDD>.<n>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dated {
    pub base: Version,
    pub label: String,
    /// The day as `YYYYMMDD`.
    pub day: u32,
    pub number: u32,
}

impl ReleaseTag {
    /// Reads a release tag of the form `settings` versions with: under
    /// `semver`, `v<x.y.z>` or a dated tag of an earlier label.
    pub fn parse(name: &str, settings: &config::Release) -> Option<Self> {
        match settings.versioning {
            Versioning::Dated => Self::parse_dated(name, &settings.labels),
            Versioning::Semver => {
                Self::parse_semver(name).or_else(|| Self::parse_dated(name, &settings.labels))
            }
        }
    }

    /// Reads a tag name `v<base>-<label>.<YYYYMMDD>.<n>` whose label is one
    /// of `labels`.
    pub fn parse_dated(name: &str, labels: &[String]) -> Option<Self> {
        let version = Version::parse(name.strip_prefix('v')?).ok()?;
        if !version.build.is_empty() {
            return None;
        }
        let parts: Vec<&str> = version.pre.as_str().split('.').collect();
        let [label, day, number] = parts.as_slice() else {
            return None;
        };
        if !labels.iter().any(|l| l == label) {
            return None;
        }
        let day = parse_day(day)?;
        let number = number
            .parse::<u32>()
            .ok()
            .filter(|n| (1..=MAX_PER_DAY).contains(n) && !number.starts_with('0'))?;
        let base = Version::new(version.major, version.minor, version.patch);
        Some(Self {
            name: name.to_string(),
            dated: Some(Dated {
                base,
                label: label.to_string(),
                day,
                number,
            }),
            version,
        })
    }

    /// Reads a tag name `v<x.y.z>`.
    pub fn parse_semver(name: &str) -> Option<Self> {
        let version = Version::parse(name.strip_prefix('v')?).ok()?;
        (version.pre.is_empty() && version.build.is_empty()).then(|| Self::semver(version))
    }

    /// The dated tag of `base` with `label`, `day` and `number`.
    pub fn dated(base: &Version, label: &str, day: u32, number: u32) -> Self {
        let mut version = base.clone();
        version.pre = Prerelease::new(&format!("{label}.{day}.{number}"))
            .expect("a release label and two numbers form a prerelease");
        Self {
            name: format!("v{version}"),
            version,
            dated: Some(Dated {
                base: base.clone(),
                label: label.to_string(),
                day,
                number,
            }),
        }
    }

    /// The tag `v<x.y.z>` of `version`.
    pub fn semver(version: Version) -> Self {
        Self {
            name: format!("v{version}"),
            version,
            dated: None,
        }
    }

    /// The base of a dated tag.
    pub fn base(&self) -> Option<&Version> {
        self.dated.as_ref().map(|d| &d.base)
    }

    /// The build number a dated tag stands for, `YYYYMMDD * 100 + n`.
    pub fn build_number(&self) -> Option<u64> {
        self.dated
            .as_ref()
            .map(|d| u64::from(d.day) * 100 + u64::from(d.number))
    }

    /// The version without the `v`.
    pub fn version_text(&self) -> String {
        self.version.to_string()
    }
}

/// A calendar day written `YYYYMMDD`, as a number.
fn parse_day(text: &str) -> Option<u32> {
    if text.len() != 8 || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let year: i16 = text[..4].parse().ok()?;
    let month: i8 = text[4..6].parse().ok()?;
    let day: i8 = text[6..].parse().ok()?;
    Date::new(year, month, day).ok()?;
    text.parse().ok()
}

/// A day as `YYYYMMDD`.
pub fn day_number(date: Date) -> u32 {
    let n = i32::from(date.year()) * 10_000 + i32::from(date.month()) * 100 + i32::from(date.day());
    u32::try_from(n).unwrap_or_default()
}

/// The UTC day of an instant, as `YYYYMMDD`.
pub fn utc_day(at: Timestamp) -> u32 {
    day_number(jiff::tz::Offset::UTC.to_datetime(at).date())
}

/// The current instant.
pub fn now() -> Timestamp {
    // Lets the tests fix the day; builds without debug assertions ignore it.
    #[cfg(debug_assertions)]
    if let Some(at) = std::env::var("GIT_INTAKE_TEST_NOW")
        .ok()
        .and_then(|v| v.parse().ok())
    {
        return at;
    }
    Timestamp::now()
}

/// A release: a tag of release form on the downstream's line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub tag: ReleaseTag,
    pub commit: String,
    /// The upstream release its annotation records.
    pub recorded: Option<Recorded>,
}

/// The first release of `others` that `candidate` does not come after: one
/// with an equal or higher version, or, between dated tags, an equal or
/// higher build number.
pub fn not_after<'a>(candidate: &ReleaseTag, others: &'a [Release]) -> Option<&'a Release> {
    others
        .iter()
        .filter(|r| r.tag.name != candidate.name)
        .filter(|r| r.tag.version >= candidate.version || later_build(&r.tag, candidate))
        .max_by(|a, b| a.tag.version.cmp(&b.tag.version))
}

fn later_build(release: &ReleaseTag, candidate: &ReleaseTag) -> bool {
    matches!(
        (release.build_number(), candidate.build_number()),
        (Some(r), Some(c)) if r >= c
    )
}

/// Why `candidate` cannot come after `release`, for messages.
pub fn order_problem(candidate: &ReleaseTag, release: &Release) -> String {
    if release.tag.version >= candidate.version {
        format!(
            "{} does not sort after the release {} at {}",
            candidate.name,
            release.tag.name,
            abbrev(&release.commit)
        )
    } else {
        format!(
            "{} has the build number {}, not above {} of the release {} at {}",
            candidate.name,
            candidate.build_number().unwrap_or_default(),
            release.tag.build_number().unwrap_or_default(),
            release.tag.name,
            abbrev(&release.commit)
        )
    }
}

/// The next release of `base` with `label` on `day`, after `releases`. Its
/// number follows every tag of release form in `tags` of that day, on the
/// downstream's line or not, so that it never takes an existing tag's name.
pub fn next(
    releases: &[Release],
    tags: &[ReleaseTag],
    base: &Version,
    label: &str,
    day: u32,
) -> std::result::Result<ReleaseTag, String> {
    let highest = releases
        .iter()
        .map(|r| &r.tag)
        .chain(tags)
        .filter_map(|t| t.dated.as_ref())
        .filter(|d| d.day == day)
        .map(|d| d.number)
        .max();
    if let Some(n) = highest
        && n >= MAX_PER_DAY
    {
        return Err(format!(
            "the day {day} already has a release numbered {MAX_PER_DAY}, the most one day takes; \
             the next release can be cut on the next day (UTC)"
        ));
    }
    let candidate = ReleaseTag::dated(base, label, day, highest.map_or(1, |n| n + 1));
    match not_after(&candidate, releases) {
        Some(release) => Err(format!(
            "the next release would be {}, which breaks the order of releases: {}",
            candidate.name,
            order_problem(&candidate, release)
        )),
        None => Ok(candidate),
    }
}

/// The newest upstream release a commit contains.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpstreamRelease {
    pub upstream: String,
    pub tag: String,
    pub version: Version,
    pub commit: String,
}

impl UpstreamRelease {
    /// The release base it gives: its version with the patch number raised
    /// by one.
    pub fn base(&self) -> Version {
        let v = &self.version;
        Version::new(v.major, v.minor, v.patch + 1)
    }

    /// The upstream's name and version, `<upstream>-<x.y.z>`, with every
    /// character of the name but ASCII letters, digits and `-` written `-`,
    /// so that it is valid build metadata.
    pub fn label(&self) -> String {
        let name: String = self
            .upstream
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' {
                    c
                } else {
                    '-'
                }
            })
            .collect();
        format!("{name}-{}", self.version)
    }

    /// The trailers that record it in a release tag's annotation.
    pub fn trailers(&self) -> String {
        format!(
            "{}: {}\n{}: {}\n{}: {}\n",
            sync_record::UPSTREAM_TRAILER,
            self.upstream,
            sync_record::TARGET_TRAILER,
            self.commit,
            sync_record::TAG_TRAILER,
            self.tag
        )
    }
}

/// The newest upstream release `commit` contains, if any.
pub fn upstream_release(
    git: &Git,
    settings: &config::Release,
    up: &Upstream,
    tags: &UpstreamTags,
    commit: &str,
) -> Result<Option<UpstreamRelease>> {
    let pattern = TagPattern::parse(&settings.tags)
        .map_err(|why| anyhow::anyhow!("release.tags '{}': {why}", settings.tags))?;
    let Some(found) = tags.release(git, &pattern, false, commit)? else {
        return Ok(None);
    };
    let version = pattern
        .release_version(&found.tag)
        .context("an upstream release has a version")?;
    Ok(Some(UpstreamRelease {
        upstream: up.name.clone(),
        tag: found.tag,
        version,
        commit: found.commit,
    }))
}

/// The newest upstream release `commit` contains, which every commit the
/// release commands version must have.
pub fn contained(
    git: &Git,
    settings: &config::Release,
    up: &Upstream,
    tags: &UpstreamTags,
    commit: &str,
) -> Result<UpstreamRelease> {
    if let Some(found) = upstream_release(git, settings, up, tags, commit)? {
        return Ok(found);
    }
    let why = match settings.versioning {
        Versioning::Dated => {
            "the release base comes from the newest upstream release a commit contains"
        }
        Versioning::Semver => {
            "a semver release names the newest upstream release its commit contains"
        }
    };
    bail!(
        "{} contains no release of upstream '{}' ({}) whose tag matches '{}'; {why}",
        abbrev(commit),
        up.name,
        crate::redact::redacted(&up.url),
        settings.tags
    )
}

/// The upstream release `recorded` names, when it is a release of the
/// release upstream whose recorded commit `commit` contains; else why not.
/// With `listed`, a tag the upstream still lists must point at that commit.
pub fn recorded_release(
    git: &Git,
    source: &Source,
    recorded: &Recorded,
    commit: &str,
    listed: bool,
) -> Result<std::result::Result<UpstreamRelease, String>> {
    let (settings, up) = (source.settings, source.up);
    let named = format!(
        "the upstream release {} {}",
        recorded.upstream, recorded.tag
    );
    if recorded.upstream != up.name {
        return Ok(Err(format!(
            "{named} is not of the release upstream '{}'",
            up.name
        )));
    }
    let Some(version) = recorded_version(source, &recorded.tag)? else {
        return Ok(Err(format!(
            "{named} is no release whose tag matches '{}'",
            settings.tags
        )));
    };
    let released = recorded.commit.as_str();
    if !git.object_exists(released)? || !git.is_ancestor(released, commit)? {
        return Ok(Err(format!(
            "{} does not contain {named} at {}",
            abbrev(commit),
            abbrev(released)
        )));
    }
    if listed
        && let Some(now) = source.tags.commit(&recorded.tag)
        && !now.eq_ignore_ascii_case(released)
    {
        return Ok(Err(format!(
            "upstream '{}' now has {} at {}, not at the recorded {}",
            up.name,
            recorded.tag,
            abbrev(now),
            abbrev(released)
        )));
    }
    Ok(Ok(UpstreamRelease {
        upstream: up.name.clone(),
        tag: recorded.tag.clone(),
        version,
        commit: released.to_string(),
    }))
}

/// The tag and version of the upstream release of `release`: the one its
/// annotation records, which `release check` checked when it was released,
/// or, without a record of the release upstream, the newest one its commit
/// contains now.
pub fn release_upstream(
    git: &Git,
    source: &Source,
    release: &Release,
) -> Result<Option<(String, Version)>> {
    if let Some(recorded) = &release.recorded
        && recorded.upstream == source.up.name
        && let Some(version) = recorded_version(source, &recorded.tag)?
    {
        return Ok(Some((recorded.tag.clone(), version)));
    }
    let found = upstream_release(
        git,
        source.settings,
        source.up,
        source.tags,
        &release.commit,
    )?;
    Ok(found.map(|r| (r.tag, r.version)))
}

/// The version of the upstream release tag `tag`, if the release `tags`
/// pattern selects it.
fn recorded_version(source: &Source, tag: &str) -> Result<Option<Version>> {
    let settings = source.settings;
    let pattern = TagPattern::parse(&settings.tags)
        .map_err(|why| anyhow::anyhow!("release.tags '{}': {why}", settings.tags))?;
    Ok(pattern
        .matches(tag)
        .then(|| pattern.release_version(tag))
        .flatten())
}

/// The next semver release of a commit, from the releases below it.
#[derive(Debug, Clone)]
pub struct SemverNext {
    pub previous: Option<Release>,
    /// The change that sets the next release, or `None` when none is due.
    pub bump: Option<Bump>,
    pub version: Option<Version>,
    /// The version the smallest change would give.
    pub smallest: Version,
}

impl SemverNext {
    /// What the previous release is, for messages.
    pub fn since(&self) -> String {
        match &self.previous {
            Some(p) => format!("the release {}", p.tag.name),
            None => "the start of the downstream's line".to_string(),
        }
    }
}

/// The release settings, the upstream they name and its tags.
pub struct Source<'a> {
    pub settings: &'a config::Release,
    pub up: &'a Upstream,
    pub tags: &'a UpstreamTags,
}

/// The next semver release of `commit`, whose upstream release is
/// `upstream`, after the releases on `below`, the downstream's line below
/// it, newest first.
pub fn semver_next(
    git: &Git,
    source: &Source,
    commit: &str,
    below: &[String],
    releases: &[Release],
    upstream: &UpstreamRelease,
) -> Result<SemverNext> {
    let previous = highest_on(below, releases).cloned();
    let mut bump = commits_bump(git, commit, previous.as_ref().map(|p| p.commit.as_str()))?;
    if let Some(p) = &previous
        && let Some((_, before)) = release_upstream(git, source, p)?
        && !bump::compatible(&before, &upstream.version)
    {
        bump = Some(Bump::Major);
    }
    let after_semver = previous.as_ref().filter(|p| p.tag.dated.is_none());
    let Some(previous_semver) = after_semver else {
        let on: HashSet<&str> = below.iter().map(String::as_str).collect();
        let first = releases
            .iter()
            .filter(|r| on.contains(r.commit.as_str()))
            .map(|r| &r.tag.version)
            .max()
            .filter(|v| **v >= Version::new(1, 0, 0))
            .map_or(Version::new(1, 0, 0), |v| Version::new(v.major + 1, 0, 0));
        return Ok(SemverNext {
            bump: bump.map(|_| Bump::Major),
            version: bump.map(|_| first.clone()),
            smallest: first,
            previous,
        });
    };
    let v = previous_semver.tag.version.clone();
    Ok(SemverNext {
        bump,
        version: bump.map(|b| b.apply(&v)),
        smallest: Bump::Patch.apply(&v),
        previous,
    })
}

/// The largest change of the commits `commit` has and `previous` lacks,
/// merges excluded.
fn commits_bump(git: &Git, commit: &str, previous: Option<&str>) -> Result<Option<Bump>> {
    let not = previous.map(|p| format!("^{p}"));
    let mut args = vec!["log", "--no-merges", "-z", "--format=%H%n%B", commit];
    args.extend(not.as_deref());
    args.push("--");
    let out = git.run(&args)?;
    Ok(out
        .split('\0')
        .filter(|r| !r.is_empty())
        .filter_map(|r| Bump::of_message(r.split_once('\n').map_or("", |(_, m)| m)))
        .max())
}

/// The release settings of a configuration, and the upstream they name.
pub fn settings<'c>(config: &'c Config, at: &str) -> Result<(&'c config::Release, &'c Upstream)> {
    let Some(settings) = &config.release else {
        bail!(
            "the {} at {at} has no [release] table; add one to make the integration branch releasable",
            config::FILE_NAME
        );
    };
    let up = config
        .upstream(&settings.upstream)
        .context("the release upstream is configured")?;
    Ok((settings, up))
}

/// What a command already fetched or listed, so that it is not asked for
/// twice.
#[derive(Default)]
pub struct Known {
    /// Fetched upstream tips by URL and branch.
    pub tips: HashMap<(String, String), String>,
    /// Upstream tag listings by URL.
    pub tags: HashMap<String, UpstreamTags>,
}

/// The tags of the release upstream, listed from its URL.
pub fn upstream_tags(git: &Git, up: &Upstream, known: &Known) -> Result<UpstreamTags> {
    if let Some(tags) = known.tags.get(&up.url) {
        return Ok(tags.clone());
    }
    match UpstreamTags::list(git, up)? {
        Ok(tags) => Ok(tags),
        Err(why) => bail!("{why}"),
    }
}

/// The last synced commit of each upstream at `tip`, by the upstream's
/// name, as `status` reports it: the target of its last sync record, or
/// else the one the history shows. An upstream that has neither has none.
pub fn last_synced(
    git: &Git,
    fetches: &mut Fetches,
    config: &Config,
    tip: &str,
    known: &Known,
) -> Result<Vec<(String, String)>> {
    let mut tips = Vec::new();
    for up in &config.upstreams {
        let key = (up.url.clone(), up.branch.clone());
        let fetched = match known.tips.get(&key) {
            Some(t) => t.clone(),
            None => match fetches.upstream(up)? {
                Ok(t) => t,
                Err(why) => bail!(
                    "cannot fetch upstream '{}' to find the downstream's own commits: {why}",
                    up.name
                ),
            },
        };
        tips.push(fetched);
    }
    let latest = sync_record::latest(git, &sync_record::records(git, tip, &tips)?)?;
    let mut synced = Vec::new();
    for (up, fetched) in config.upstreams.iter().zip(&tips) {
        let commit = match latest.get(&up.name) {
            Some(r) if git.object_exists(&r.target)? => Some(r.target.clone()),
            Some(_) => None,
            None => drift::adopted_base(git, tip, up, fetched)?,
        };
        synced.extend(commit.map(|c| (up.name.clone(), c)));
    }
    Ok(synced)
}

/// The last synced commit of each upstream at `tip`, as [`last_synced`]
/// gives it, from the sync records of the local history alone. The records
/// below the synced commits of a first reading are inherited from an
/// upstream and left out, and a record counts as this repository's own only
/// when the configuration committed at its merge names that upstream at the
/// same URL. An upstream without an own record is fetched, as
/// [`last_synced`] does, to find the commit its history was adopted from.
pub fn last_synced_offline(
    git: &Git,
    fetches: &mut Fetches,
    config: &Config,
    tip: &str,
) -> Result<Vec<(String, String)>> {
    let first = sync_record::latest(git, &sync_record::records(git, tip, &[])?)?;
    let mut targets = Vec::new();
    for up in &config.upstreams {
        if let Some(r) = first.get(&up.name)
            && git.object_exists(&r.target)?
        {
            targets.push(r.target.clone());
        }
    }
    let latest = if targets.is_empty() {
        first
    } else {
        sync_record::latest(git, &sync_record::records(git, tip, &targets)?)?
    };
    let mut synced = Vec::new();
    for up in &config.upstreams {
        let own = match latest.get(&up.name) {
            Some(r) => made_here(git, &r.commit, up)?.then_some(r),
            None => None,
        };
        let commit = match own {
            Some(r) => git.object_exists(&r.target)?.then(|| r.target.clone()),
            None => match fetches.upstream(up)? {
                Ok(fetched) => drift::adopted_base(git, tip, up, &fetched)?,
                Err(why) => bail!(
                    "upstream '{}' has no sync record at {}, and fetching it to find the commit the \
                     downstream was adopted from failed: {why}",
                    up.name,
                    abbrev(tip)
                ),
            },
        };
        synced.extend(commit.map(|c| (up.name.clone(), c)));
    }
    Ok(synced)
}

/// Whether the configuration committed at `merge` names `up` at its URL,
/// so that a sync record there is this repository's, not one an upstream's
/// history brought in.
fn made_here(git: &Git, merge: &str, up: &Upstream) -> Result<bool> {
    let Some(text) = git.show_file(merge, config::FILE_NAME)? else {
        return Ok(false);
    };
    Ok(config::parse(&text).is_ok_and(|c| {
        c.upstream(&up.name)
            .is_some_and(|u| u.url == up.url && u.branch == up.branch)
    }))
}

/// The commits of a [`last_synced`] list.
pub fn synced_commits(synced: &[(String, String)]) -> Vec<String> {
    synced.iter().map(|(_, c)| c.clone()).collect()
}

/// The downstream's line ending at `tip`: its first-parent commits that no
/// commit of `synced` contains, newest first.
pub fn line(git: &Git, tip: &str, synced: &[String]) -> Result<Vec<String>> {
    let mut args: Vec<&str> = vec!["rev-list", "--first-parent", tip];
    if !synced.is_empty() {
        args.push("--not");
        args.extend(synced.iter().map(String::as_str));
    }
    args.push("--");
    Ok(crate::git::lines(&git.run(&args)?))
}

/// The release tags among `tags`, each with its commit, that point at a
/// commit of `line`.
pub fn releases_on(line: &[String], tags: &[TagRef], settings: &config::Release) -> Vec<Release> {
    let on: HashSet<&str> = line.iter().map(String::as_str).collect();
    tags.iter()
        .filter(|t| on.contains(t.commit.as_str()))
        .filter_map(|t| {
            let tag = ReleaseTag::parse(&t.name, settings)?;
            Some(Release {
                // Only a semver tag records its upstream release.
                recorded: t.recorded.clone().filter(|_| tag.dated.is_none()),
                tag,
                commit: t.commit.clone(),
            })
        })
        .collect()
}

/// The release tags among `tags`, wherever they point.
pub fn release_tags(tags: &[TagRef], settings: &config::Release) -> Vec<ReleaseTag> {
    tags.iter()
        .filter_map(|t| ReleaseTag::parse(&t.name, settings))
        .collect()
}

/// The newest release on `line` below its first commit: the highest release
/// of the nearest commit after the first that has one.
pub fn previous<'a>(line: &[String], releases: &'a [Release]) -> Option<&'a Release> {
    highest_on(line.get(1..).unwrap_or_default(), releases)
}

/// The highest release of the first commit of `commits` that has one.
fn highest_on<'a>(commits: &[String], releases: &'a [Release]) -> Option<&'a Release> {
    commits.iter().find_map(|c| {
        releases
            .iter()
            .filter(|r| r.commit == *c)
            .max_by(|a, b| a.tag.version.cmp(&b.tag.version))
    })
}

/// The publish remote's integration branch and release tags, fetched into
/// the command's own references.
pub struct Remote {
    pub name: String,
    pub tip: String,
    /// Each tag whose name starts with `v`.
    pub tags: Vec<TagRef>,
}

impl Remote {
    /// The commit the remote's tag `name` points at.
    pub fn tag(&self, name: &str) -> Option<&str> {
        self.entry(name).map(|t| t.commit.as_str())
    }

    pub fn entry(&self, name: &str) -> Option<&TagRef> {
        self.tags.iter().find(|t| t.name == name)
    }
}

/// Fetches the integration branch and the tags of the publish remote.
pub fn fetch_remote(fetches: &mut Fetches, remote: &str, integration: &str) -> Result<Remote> {
    match fetches.release_refs(remote, integration)? {
        Ok((tip, tags)) => Ok(Remote {
            name: remote.to_string(),
            tip,
            tags,
        }),
        Err(why) => bail!(
            "cannot fetch '{integration}' and the release tags from the publish remote '{remote}': {why}"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(l: &[&str]) -> Vec<String> {
        l.iter().map(|s| s.to_string()).collect()
    }

    fn release(name: &str) -> Release {
        Release {
            tag: ReleaseTag::parse_dated(name, &labels(&["custom", "nightly"])).unwrap(),
            commit: "c".repeat(40),
            recorded: None,
        }
    }

    #[test]
    fn release_tags_are_read_strictly() {
        let l = labels(&["custom"]);
        let t = ReleaseTag::parse_dated("v0.11.1-custom.20261101.2", &l).unwrap();
        assert_eq!(t.base(), Some(&Version::new(0, 11, 1)));
        let d = t.dated.as_ref().unwrap();
        assert_eq!((d.day, d.number), (20261101, 2));
        assert_eq!(t.version_text(), "0.11.1-custom.20261101.2");
        assert_eq!(t.build_number(), Some(2026110102));
        for bad in [
            "0.11.1-custom.20261101.2",
            "v0.11.1-custom.20260230.1",
            "v0.11.1-custom.2026111.1",
            "v0.11.1-custom.20261101.0",
            "v0.11.1-custom.20261101.100",
            "v0.11.1-custom.20261101.01",
            "v0.11.1-other.20261101.1",
            "v0.11.1-custom.20261101",
            "v0.11.1-custom.20261101.1.1",
            "v0.11.1-custom.20261101.1+build",
            "v00.11.1-custom.20261101.1",
            "v0.11-custom.20261101.1",
        ] {
            assert!(ReleaseTag::parse_dated(bad, &l).is_none(), "{bad}");
        }
        assert_eq!(
            ReleaseTag::dated(&Version::new(0, 11, 1), "custom", 20261101, 2),
            t
        );
    }

    #[test]
    fn semver_tags_are_plain_versions() {
        let t = ReleaseTag::parse_semver("v1.2.3").unwrap();
        assert_eq!((t.version_text(), t.build_number()), ("1.2.3".into(), None));
        assert_eq!(t.dated, None);
        for bad in [
            "1.2.3",
            "v1.2",
            "v01.2.3",
            "v1.2.3-rc.1",
            "v1.2.3+b",
            "v0.11.1-custom.20261101.2",
        ] {
            assert!(ReleaseTag::parse_semver(bad).is_none(), "{bad}");
        }
        let mut settings = config::Release {
            upstream: "a".into(),
            tags: "v*".into(),
            labels: labels(&["custom"]),
            versioning: Versioning::Dated,
        };
        assert!(ReleaseTag::parse("v1.2.3", &settings).is_none());
        settings.versioning = Versioning::Semver;
        assert!(ReleaseTag::parse("v1.2.3", &settings).is_some());
        assert!(ReleaseTag::parse("v0.11.1-custom.20261101.2", &settings).is_some());
        assert!(ReleaseTag::parse("v0.11.1-other.20261101.2", &settings).is_none());
    }

    #[test]
    fn a_semver_release_sorts_by_version_alone() {
        let semver = |v: &str| Release {
            tag: ReleaseTag::parse_semver(v).unwrap(),
            commit: "c".repeat(40),
            recorded: None,
        };
        let v1 = ReleaseTag::parse_semver("v1.0.0").unwrap();
        // A dated release of a later day and a lower version does not block.
        assert!(not_after(&v1, &[release("v0.11.1-custom.20991231.99")]).is_none());
        let blocking = [release("v1.0.1-custom.20261101.1")];
        assert_eq!(not_after(&v1, &blocking), Some(&blocking[0]));
        let r = [semver("v1.2.0")];
        assert_eq!(
            not_after(&ReleaseTag::parse_semver("v1.1.9").unwrap(), &r),
            Some(&r[0])
        );
        assert!(not_after(&ReleaseTag::parse_semver("v1.2.1").unwrap(), &r).is_none());
    }

    #[test]
    fn upstream_labels_are_valid_build_metadata() {
        let up = |name: &str| UpstreamRelease {
            upstream: name.to_string(),
            tag: "v0.11.0".into(),
            version: Version::new(0, 11, 0),
            commit: "c".repeat(40),
        };
        assert_eq!(up("fff").label(), "fff-0.11.0");
        assert_eq!(up("app_b.core").label(), "app-b-core-0.11.0");
        for name in ["fff", "app_b.core", "A-1"] {
            let mut v = Version::new(1, 2, 0);
            v.build = semver::BuildMetadata::new(&up(name).label()).unwrap();
            assert_eq!(Version::parse(&v.to_string()).unwrap(), v);
        }
    }

    #[test]
    fn days_are_utc() {
        let at: Timestamp = "2026-10-31T23:30:00-02:00".parse().unwrap();
        assert_eq!(utc_day(at), 20261101);
        assert_eq!(day_number(Date::new(2026, 1, 9).unwrap()), 20260109);
    }

    #[test]
    fn the_next_release_numbers_the_day() {
        let base = Version::new(0, 11, 1);
        assert_eq!(
            next(&[], &[], &base, "custom", 20261101).unwrap().name,
            "v0.11.1-custom.20261101.1"
        );
        let r = [release("v0.11.1-custom.20261101.1")];
        assert_eq!(
            next(&r, &[], &base, "custom", 20261101).unwrap().name,
            "v0.11.1-custom.20261101.2"
        );
        assert_eq!(
            next(&r, &[], &base, "custom", 20261102).unwrap().name,
            "v0.11.1-custom.20261102.1"
        );
        // Any base and label of the day counts.
        let r = [release("v0.11.0-nightly.20261101.4")];
        assert_eq!(
            next(&r, &[], &base, "custom", 20261101).unwrap().name,
            "v0.11.1-custom.20261101.5"
        );
        // A tag of the day off the downstream's line counts as well.
        let off =
            [ReleaseTag::parse_dated("v0.11.1-custom.20261101.7", &labels(&["custom"])).unwrap()];
        assert_eq!(
            next(&[], &off, &base, "custom", 20261101).unwrap().name,
            "v0.11.1-custom.20261101.8"
        );
        let r = [release("v0.11.1-custom.20261101.99")];
        assert!(
            next(&r, &[], &base, "custom", 20261101)
                .unwrap_err()
                .contains("already has a release numbered 99")
        );
    }

    #[test]
    fn the_next_release_keeps_both_orders() {
        let r = [release("v0.12.1-custom.20261031.1")];
        let e = next(&r, &[], &Version::new(0, 11, 1), "custom", 20261101).unwrap_err();
        assert!(e.contains("v0.12.1-custom.20261031.1"), "{e}");
        // A label that sorts first.
        let r = [release("v0.1.7-nightly.20261031.1")];
        let e = next(&r, &[], &Version::new(0, 1, 7), "custom", 20261101).unwrap_err();
        assert!(
            e.contains(
                "v0.1.7-custom.20261101.1 does not sort after the release v0.1.7-nightly.20261031.1"
            ),
            "{e}"
        );
        // A release dated later breaks the build number.
        let r = [release("v0.1.6-custom.20261105.1")];
        let e = next(&r, &[], &Version::new(0, 1, 7), "custom", 20261101).unwrap_err();
        assert!(e.contains("build number"), "{e}");
    }
}
