//! Releases of the integration branch: their tag names and order, the base
//! version a commit takes from the release upstream, the downstream's own
//! line on which releases count, and the release tags of the publish remote.

use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result, bail};
use jiff::Timestamp;
use jiff::civil::Date;
use semver::{Prerelease, Version};

use crate::config::{self, Config, Upstream};
use crate::drift;
use crate::git::{Git, abbrev};
use crate::sync_record;
use crate::tags::{TagPattern, UpstreamTags};
use crate::upstreams::Fetches;

/// The highest number a release of one day takes, so that
/// `YYYYMMDD * 100 + n` stays a build number.
pub const MAX_PER_DAY: u32 = 99;

/// A tag name of release form, `v<base>-<label>.<YYYYMMDD>.<n>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseTag {
    pub name: String,
    pub version: Version,
    pub base: Version,
    pub label: String,
    /// The day as `YYYYMMDD`.
    pub day: u32,
    pub number: u32,
}

impl ReleaseTag {
    /// Reads a tag name of release form whose label is one of `labels`.
    pub fn parse(name: &str, labels: &[String]) -> Option<Self> {
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
            label: label.to_string(),
            version,
            base,
            day,
            number,
        })
    }

    /// The tag of `base` with `label`, `day` and `number`.
    pub fn new(base: &Version, label: &str, day: u32, number: u32) -> Self {
        let mut version = base.clone();
        version.pre = Prerelease::new(&format!("{label}.{day}.{number}"))
            .expect("a release label and two numbers form a prerelease");
        Self {
            name: format!("v{version}"),
            version,
            base: base.clone(),
            label: label.to_string(),
            day,
            number,
        }
    }

    /// The build number the tag stands for, `YYYYMMDD * 100 + n`.
    pub fn build_number(&self) -> u64 {
        u64::from(self.day) * 100 + u64::from(self.number)
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
}

/// The first release of `others` that `candidate` does not come after: one
/// with an equal or higher version, or an equal or higher build number.
pub fn not_after<'a>(candidate: &ReleaseTag, others: &'a [Release]) -> Option<&'a Release> {
    others
        .iter()
        .filter(|r| r.tag.name != candidate.name)
        .filter(|r| {
            r.tag.version >= candidate.version || r.tag.build_number() >= candidate.build_number()
        })
        .max_by(|a, b| a.tag.version.cmp(&b.tag.version))
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
            candidate.build_number(),
            release.tag.build_number(),
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
        .filter(|t| t.day == day)
        .map(|t| t.number)
        .max();
    if let Some(n) = highest
        && n >= MAX_PER_DAY
    {
        return Err(format!(
            "the day {day} already has a release numbered {MAX_PER_DAY}, the most one day takes; \
             the next release can be cut on the next day (UTC)"
        ));
    }
    let candidate = ReleaseTag::new(base, label, day, highest.map_or(1, |n| n + 1));
    match not_after(&candidate, releases) {
        Some(release) => Err(format!(
            "the next release would be {}, which breaks the order of releases: {}",
            candidate.name,
            order_problem(&candidate, release)
        )),
        None => Ok(candidate),
    }
}

/// The release base of a commit and the upstream release it comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Base {
    pub version: Version,
    pub upstream: String,
    /// The upstream release's tag.
    pub tag: String,
}

/// The release base of `commit`: the version of the newest upstream release
/// it contains, with the patch number raised by one.
pub fn base_of(
    git: &Git,
    settings: &config::Release,
    up: &Upstream,
    tags: &UpstreamTags,
    commit: &str,
) -> Result<Base> {
    let pattern = TagPattern::parse(&settings.tags)
        .map_err(|why| anyhow::anyhow!("release.tags '{}': {why}", settings.tags))?;
    let Some(found) = tags.release(git, &pattern, commit)? else {
        bail!(
            "{} contains no release of upstream '{}' ({}) whose tag matches '{}'; the release base comes \
             from the newest upstream release a commit contains",
            abbrev(commit),
            up.name,
            crate::redact::redacted(&up.url),
            settings.tags
        );
    };
    let released = pattern
        .release_version(&found.tag)
        .context("an upstream release has a version")?;
    Ok(Base {
        version: Version::new(released.major, released.minor, released.patch + 1),
        upstream: up.name.clone(),
        tag: found.tag,
    })
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

/// The last synced commit of each upstream at `tip`, as `status` reports
/// it: the target of its last sync record, or else the one the history
/// shows. An upstream that has neither has none.
pub fn last_synced(
    git: &Git,
    fetches: &mut Fetches,
    config: &Config,
    tip: &str,
    known: &Known,
) -> Result<Vec<String>> {
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
        synced.extend(commit);
    }
    Ok(synced)
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

/// The tags of release form among `tags`, each with its commit, that point
/// at a commit of `line`.
pub fn releases_on(line: &[String], tags: &[(String, String)], labels: &[String]) -> Vec<Release> {
    let on: HashSet<&str> = line.iter().map(String::as_str).collect();
    tags.iter()
        .filter(|(_, commit)| on.contains(commit.as_str()))
        .filter_map(|(name, commit)| {
            Some(Release {
                tag: ReleaseTag::parse(name, labels)?,
                commit: commit.clone(),
            })
        })
        .collect()
}

/// The tags of release form among `tags`, wherever they point.
pub fn release_tags(tags: &[(String, String)], labels: &[String]) -> Vec<ReleaseTag> {
    tags.iter()
        .filter_map(|(name, _)| ReleaseTag::parse(name, labels))
        .collect()
}

/// The newest release on `line` below its first commit: the highest release
/// of the nearest commit after the first that has one.
pub fn previous<'a>(line: &[String], releases: &'a [Release]) -> Option<&'a Release> {
    line.iter().skip(1).find_map(|c| {
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
    /// Each tag whose name starts with `v`, with the commit it points at.
    pub tags: Vec<(String, String)>,
}

impl Remote {
    /// The commit the remote's tag `name` points at.
    pub fn tag(&self, name: &str) -> Option<&str> {
        self.tags
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, c)| c.as_str())
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
            tag: ReleaseTag::parse(name, &labels(&["custom", "nightly"])).unwrap(),
            commit: "c".repeat(40),
        }
    }

    #[test]
    fn release_tags_are_read_strictly() {
        let l = labels(&["custom"]);
        let t = ReleaseTag::parse("v0.11.1-custom.20261101.2", &l).unwrap();
        assert_eq!(t.base, Version::new(0, 11, 1));
        assert_eq!((t.day, t.number), (20261101, 2));
        assert_eq!(t.version_text(), "0.11.1-custom.20261101.2");
        assert_eq!(t.build_number(), 2026110102);
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
            assert!(ReleaseTag::parse(bad, &l).is_none(), "{bad}");
        }
        assert_eq!(
            ReleaseTag::new(&Version::new(0, 11, 1), "custom", 20261101, 2),
            t
        );
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
        let off = [ReleaseTag::parse("v0.11.1-custom.20261101.7", &labels(&["custom"])).unwrap()];
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
