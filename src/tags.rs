//! The tags of an upstream's repository: the release a follow upstream with
//! a `tags` pattern is taken at, and the tag that names a merged target.
//!
//! Tags are read from the upstream's URL with `git ls-remote` and never
//! fetched, so the repository's own tags never clash with them. A tag the
//! tool uses must point at a commit of the fetched upstream branch, which
//! the fetch already brought into the repository.

use std::cmp::Ordering;
use std::collections::BTreeMap;

use anyhow::Result;
use semver::Version;

use crate::config::Upstream;
use crate::git::Git;

/// A `tags` pattern: a glob over tag names, such as `v*`.
#[derive(Debug, Clone)]
pub struct TagPattern {
    pattern: glob::Pattern,
    /// The literal text before the first wildcard, which a version follows.
    prefix: String,
}

impl TagPattern {
    /// Parses a pattern as the configuration writes it, or says why it is
    /// not one.
    pub fn parse(text: &str) -> std::result::Result<Self, String> {
        if text.is_empty() {
            return Err("the pattern is empty".to_string());
        }
        if text.starts_with("refs/") || text.starts_with("tags/") {
            return Err(
                "the pattern matches tag names; write it without `refs/tags/` or `tags/`"
                    .to_string(),
            );
        }
        if text.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err("a tag name cannot contain whitespace or control characters".to_string());
        }
        let pattern = glob::Pattern::new(text).map_err(|e| format!("not a valid glob: {e}"))?;
        let prefix = text
            .split(['*', '?', '['])
            .next()
            .unwrap_or_default()
            .to_string();
        Ok(Self { pattern, prefix })
    }

    pub fn matches(&self, tag: &str) -> bool {
        self.pattern.matches(tag)
    }

    /// The version a tag name stands for: the name without the pattern's
    /// literal prefix, and without a `v` or `V` after it unless the prefix
    /// ends in one, read as a semantic version.
    fn version(&self, tag: &str) -> Option<Version> {
        version_after(tag, &self.prefix)
    }

    /// The version of a tag that names a release: a version without a
    /// prerelease part or build metadata.
    fn release_version(&self, tag: &str) -> Option<Version> {
        self.version(tag)
            .filter(|v| v.pre.is_empty() && v.build.is_empty())
    }
}

fn version_after(tag: &str, prefix: &str) -> Option<Version> {
    let rest = tag.strip_prefix(prefix)?;
    let rest = if prefix.ends_with(['v', 'V']) {
        rest
    } else {
        rest.strip_prefix(['v', 'V']).unwrap_or(rest)
    };
    Version::parse(rest).ok()
}

/// Whether a tag can be recorded in an `Intake-Tag` trailer, whose readers
/// split values at commas.
pub fn recordable(tag: &str) -> bool {
    !tag.contains(',')
}

/// Orders tag names by version, then by name; a name that is no version
/// comes before every version.
fn by_version(a: &(Option<Version>, &str), b: &(Option<Version>, &str)) -> Ordering {
    a.0.cmp(&b.0).then_with(|| a.1.cmp(b.1))
}

/// A release an upstream is taken at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub tag: String,
    pub commit: String,
}

/// The tags of an upstream's repository, each with the object it names,
/// peeled through annotated tags.
#[derive(Debug, Clone, Default)]
pub struct UpstreamTags {
    tags: BTreeMap<String, String>,
}

impl UpstreamTags {
    /// Asks the upstream's URL for its tags. Returns why when it cannot be
    /// asked.
    pub fn list(git: &Git, up: &Upstream) -> Result<std::result::Result<Self, String>> {
        let out = git.raw(&["ls-remote", "--tags", "--end-of-options", &up.url])?;
        if !out.success() {
            return Ok(Err(format!(
                "upstream '{}': cannot list the tags of {}: {}",
                up.name,
                crate::git::redacted(&up.url),
                crate::git::redacted(out.stderr.trim())
            )));
        }
        Ok(Ok(Self::parse(&out.stdout)))
    }

    /// Reads `git ls-remote --tags` output: the peeled entry (`<name>^{}`)
    /// of an annotated tag wins over the tag object itself.
    fn parse(listing: &str) -> Self {
        let mut tags = BTreeMap::new();
        let mut peeled = BTreeMap::new();
        for line in listing.lines() {
            let Some((oid, refname)) = line.split_once('\t') else {
                continue;
            };
            let Some(name) = refname.strip_prefix("refs/tags/") else {
                continue;
            };
            match name.strip_suffix("^{}") {
                Some(name) => peeled.insert(name.to_string(), oid.to_string()),
                None => tags.insert(name.to_string(), oid.to_string()),
            };
        }
        tags.extend(peeled);
        Self { tags }
    }

    /// The object the tag `name` names, peeled.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.tags.get(name).map(String::as_str)
    }

    /// The newest release that `pattern` selects on the branch fetched at
    /// `tip`: of the tags matching the pattern whose name is a semantic
    /// version without a prerelease part or build metadata, the one with the
    /// highest version whose commit the branch contains.
    ///
    /// One `cat-file` finds which candidates the repository has as commits;
    /// a tag outside the fetched branch usually names an object it lacks.
    /// Ancestry is checked only for those, newest first, stopping at the
    /// first on the branch.
    pub fn release(&self, git: &Git, pattern: &TagPattern, tip: &str) -> Result<Option<Release>> {
        let mut candidates: Vec<(Option<Version>, &str)> = self
            .tags
            .keys()
            .filter(|name| pattern.matches(name))
            .filter_map(|name| Some((Some(pattern.release_version(name)?), name.as_str())))
            .collect();
        if candidates.is_empty() {
            return Ok(None);
        }
        candidates.sort_by(|a, b| by_version(b, a));
        let input: String = candidates
            .iter()
            .map(|(_, name)| format!("{}\n", self.tags[*name]))
            .collect();
        let out = git.run_with_input(
            &["cat-file", "--batch-check=%(objectname) %(objecttype)"],
            input,
        )?;
        let kinds: Vec<&str> = out.lines().collect();
        anyhow::ensure!(
            kinds.len() == candidates.len(),
            "unexpected output from git cat-file --batch-check"
        );
        for ((_, name), kind) in candidates.iter().zip(kinds) {
            let Some(commit) = kind.strip_suffix(" commit") else {
                continue;
            };
            if git.is_ancestor(commit, tip)? {
                return Ok(Some(Release {
                    tag: name.to_string(),
                    commit: commit.to_string(),
                }));
            }
        }
        Ok(None)
    }

    /// The tag that names `commit` in its sync record, when any of the
    /// upstream's tags points at it: of those matching `pattern` whose name
    /// is a version after the pattern's literal prefix, the highest; without
    /// such a tag, of those whose name is a version (with an optional `v` or
    /// `V`), the highest. A tag whose name is no version is never chosen. A tag whose name
    /// holds a comma is never chosen.
    pub fn naming(&self, pattern: Option<&TagPattern>, commit: &str) -> Option<String> {
        let at: Vec<&str> = self
            .tags
            .iter()
            .filter(|(name, oid)| oid.eq_ignore_ascii_case(commit) && recordable(name))
            .map(|(name, _)| name.as_str())
            .collect();
        let highest = |prefix: &str, names: &mut dyn Iterator<Item = &str>| {
            names
                .filter_map(|t| Some((Some(version_after(t, prefix)?), t)))
                .max_by(by_version)
                .map(|(_, t)| t.to_string())
        };
        if let Some(p) = pattern
            && let Some(tag) = highest(&p.prefix, &mut at.iter().copied().filter(|t| p.matches(t)))
        {
            return Some(tag);
        }
        highest("", &mut at.into_iter())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags(entries: &[(&str, &str)]) -> UpstreamTags {
        let listing: String = entries
            .iter()
            .map(|(oid, name)| format!("{oid}\trefs/tags/{name}\n"))
            .collect();
        UpstreamTags::parse(&listing)
    }

    #[test]
    fn patterns_are_validated() {
        assert!(TagPattern::parse("v*").is_ok());
        assert!(TagPattern::parse("release-[0-9]*").is_ok());
        for bad in ["", "refs/tags/v*", "tags/v*", "v *", "v[", "v\t*"] {
            assert!(TagPattern::parse(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn patterns_match_whole_tag_names() {
        let p = TagPattern::parse("v*").unwrap();
        assert!(p.matches("v1.0.0"));
        assert!(!p.matches("release-v1.0.0"));
        let p = TagPattern::parse("pkg@*").unwrap();
        assert!(p.matches("pkg@1.2.3"));
        assert_eq!(p.version("pkg@1.2.3"), Version::parse("1.2.3").ok());
    }

    #[test]
    fn versions_follow_the_literal_prefix() {
        let p = TagPattern::parse("v*").unwrap();
        assert_eq!(p.version("v0.10.0"), Version::parse("0.10.0").ok());
        assert_eq!(p.version("v1.2"), None);
        assert_eq!(p.version("vnext"), None);
        // A prefix ending in `v` takes the `v` already: no second one.
        assert_eq!(p.version("vv1.2.3"), None);
        assert_eq!(p.version("vV1.2.3"), None);
        let p = TagPattern::parse("V*").unwrap();
        assert_eq!(p.version("V1.2.3"), Version::parse("1.2.3").ok());
        let p = TagPattern::parse("*").unwrap();
        assert_eq!(p.version("v2.0.0"), Version::parse("2.0.0").ok());
        assert_eq!(p.version("V2.0.0"), Version::parse("2.0.0").ok());
        assert_eq!(p.version("2.0.0"), Version::parse("2.0.0").ok());
        let p = TagPattern::parse("release-[0-9]*").unwrap();
        assert_eq!(p.version("release-3.1.4"), Version::parse("3.1.4").ok());
        let p = TagPattern::parse("release-*").unwrap();
        assert_eq!(p.version("release-v3.1.4"), Version::parse("3.1.4").ok());
    }

    #[test]
    fn releases_are_plain_versions() {
        let p = TagPattern::parse("v*").unwrap();
        assert!(p.release_version("v1.2.3").is_some());
        assert!(p.release_version("v1.2.3-rc.1").is_none());
        assert!(p.release_version("v1.2.3+build.5").is_none());
        assert!(p.release_version("vnext").is_none());
    }

    #[test]
    fn annotated_tags_are_peeled() {
        let t = UpstreamTags::parse(
            "aaaa\trefs/tags/v1\nbbbb\trefs/tags/v1^{}\ncccc\trefs/tags/v2\ndddd\trefs/heads/main\n",
        );
        assert_eq!(t.get("v1"), Some("bbbb"));
        assert_eq!(t.get("v2"), Some("cccc"));
        assert_eq!(t.get("main"), None);
    }

    #[test]
    fn the_naming_tag_prefers_the_pattern_and_the_highest_version() {
        let t = tags(&[
            ("c1", "latest"),
            ("c1", "v1.9.0"),
            ("c1", "v1.10.0"),
            ("c2", "stable"),
            ("c3", "archive/2024"),
            ("c3", "2.0.0"),
            ("c3", "V1.0.0"),
            ("c4", "archive/old"),
            ("c5", "v9.0.0,v8.0.0"),
            ("c6", "v1.0.0-rc.1"),
            ("c8", "vnext"),
            ("c8", "3.0.0"),
            ("c9", "vnext"),
        ]);
        let p = TagPattern::parse("v*").unwrap();
        assert_eq!(t.naming(Some(&p), "c1").as_deref(), Some("v1.10.0"));
        assert_eq!(t.naming(None, "c1").as_deref(), Some("v1.10.0"));
        // Without a matching tag, only a version names the target.
        assert_eq!(t.naming(Some(&p), "c2"), None);
        assert_eq!(t.naming(None, "c3").as_deref(), Some("2.0.0"));
        assert_eq!(t.naming(Some(&p), "c3").as_deref(), Some("2.0.0"));
        assert_eq!(t.naming(None, "c4"), None);
        // A trailer value cannot hold a comma.
        assert_eq!(t.naming(Some(&p), "c5"), None);
        assert!(!recordable("v9.0.0,v8.0.0"));
        assert!(recordable("v9.0.0"));
        assert_eq!(t.naming(None, "c6").as_deref(), Some("v1.0.0-rc.1"));
        assert_eq!(t.naming(None, "c7"), None);
        // A matching tag that is no version falls through to the versions.
        assert_eq!(t.naming(Some(&p), "c8").as_deref(), Some("3.0.0"));
        assert_eq!(t.naming(Some(&p), "c9"), None);
    }
}
