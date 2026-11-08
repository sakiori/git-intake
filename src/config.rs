//! The repository's committed `.git-intake.toml`: parsing and validation.

use std::collections::HashMap;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

/// File name of the configuration at the repository root.
pub const FILE_NAME: &str = ".git-intake.toml";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    integration: String,
    /// Read as a value first so that the string form of earlier releases gets
    /// a message showing the table form.
    publish: Option<toml::Value>,
    install: Option<String>,
    /// Read as a value first so that the plain command strings of earlier
    /// releases get a message showing the table form.
    checks: Option<toml::Value>,
    #[serde(default, rename = "upstream")]
    upstreams: Vec<RawUpstream>,
    /// Read as a value first so that its problems are named with its keys.
    release: Option<toml::Value>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRelease {
    upstream: String,
    tags: Option<String>,
    labels: Option<Vec<String>>,
    /// Read as text so that an unknown scheme is named with its key.
    versioning: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawUpstream {
    name: String,
    url: Option<String>,
    branch: String,
    #[serde(default)]
    mode: Mode,
    path: Option<String>,
    tags: Option<String>,
    prereleases: Option<bool>,
    /// Read as a signed number so that zero and negative values get a
    /// message of their own.
    max_conflicts: Option<i64>,
    /// Settings of earlier releases, read only to name what replaces them.
    remote: Option<toml::Value>,
    mirror: Option<toml::Value>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPublish {
    remote: String,
    #[serde(default)]
    via: PublishVia,
    /// Read as text so that an unknown forge is named with the values
    /// accepted.
    forge: Option<String>,
    api: Option<String>,
    queue: Option<String>,
    replace_after: Option<String>,
}

/// How an upstream is taken.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    /// Every update merges the upstream branch up to a target.
    #[default]
    Follow,
    /// Every update takes the upstream commits a plan chooses, one by one.
    Port,
}

impl Mode {
    /// The value as written in the configuration.
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Follow => "follow",
            Mode::Port => "port",
        }
    }
}

/// How `finish` publishes the verified update.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PublishVia {
    /// Push the integration branch.
    #[default]
    Push,
    /// Push the verified commit to a branch of its own and open a merge
    /// request into the integration branch.
    MergeRequest,
}

impl PublishVia {
    /// The value as written in the configuration.
    pub fn as_str(self) -> &'static str {
        match self {
            PublishVia::Push => "push",
            PublishVia::MergeRequest => "merge-request",
        }
    }
}

/// The forge that hosts the publish remote, as the setting `forge` names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ForgeKind {
    Gitlab,
    Github,
}

/// Where and how `finish` publishes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Publish {
    pub remote: String,
    pub via: PublishVia,
    /// Merge-request mode: the forge of the publish remote, when its URL
    /// does not tell it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forge: Option<ForgeKind>,
    /// Merge-request mode: the base URL of the forge's REST API, an `https`
    /// URL on the publish remote's host.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api: Option<String>,
    /// Merge-request mode: the account of a merge queue that takes the
    /// merge requests assigned to it, which `land` pauses.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queue: Option<String>,
    /// Merge-request mode: how long the update's merge request stays open
    /// at least before `sync` may replace it with a newer release.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replace_after: Option<ReplaceAfter>,
}

/// The shortest time the update's merge request stays open before `sync`
/// may replace it: a positive duration of whole weeks, days, hours and
/// minutes, in which a day counts as 24 hours. It keeps the text it was
/// written as, which messages and the update's state show.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ReplaceAfter {
    text: String,
    duration: jiff::SignedDuration,
}

impl ReplaceAfter {
    /// Reads a duration such as `7d`, `36h`, `2w` or `1d 12h`, refusing
    /// years and months, which have no fixed length, seconds, and anything
    /// that is not positive.
    pub fn parse(text: &str) -> std::result::Result<Self, String> {
        let refuse = || {
            format!(
                "publish replace_after: '{text}' is not a positive duration of weeks, days, hours and minutes; \
                 write one such as `7d`, `36h`, `2w` or `1d 12h`, in which a day counts as 24 hours"
            )
        };
        let trimmed = text.trim();
        let span: jiff::Span = trimmed.parse().map_err(|_| refuse())?;
        let calendar = span.get_years() != 0 || span.get_months() != 0;
        let below_minutes = span.get_seconds() != 0
            || span.get_milliseconds() != 0
            || span.get_microseconds() != 0
            || span.get_nanoseconds() != 0;
        if calendar || below_minutes || !span.is_positive() {
            return Err(refuse());
        }
        let duration = span
            .to_duration(jiff::SpanRelativeTo::days_are_24_hours())
            .map_err(|_| refuse())?;
        if !duration.is_positive() {
            return Err(refuse());
        }
        Ok(Self {
            text: trimmed.to_string(),
            duration,
        })
    }

    /// The duration as it was written.
    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn duration(&self) -> jiff::SignedDuration {
        self.duration
    }
}

impl TryFrom<String> for ReplaceAfter {
    type Error = String;

    fn try_from(text: String) -> std::result::Result<Self, String> {
        Self::parse(&text)
    }
}

impl From<ReplaceAfter> for String {
    fn from(value: ReplaceAfter) -> String {
        value.text
    }
}

/// A validated configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    pub integration: String,
    pub publish: Option<Publish>,
    pub install: Option<String>,
    pub checks: Vec<Check>,
    pub upstreams: Vec<Upstream>,
    /// The releases of the integration branch, when it is releasable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release: Option<Release>,
}

/// The `[release]` table: the upstream whose version the releases carry, the
/// pattern of its release tags, and the labels of the downstream's releases.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Release {
    pub upstream: String,
    /// The pattern of `[release]`, or else the upstream's own `tags`.
    pub tags: String,
    /// The label of new releases first, then the labels earlier releases
    /// took.
    pub labels: Vec<String>,
    #[serde(default)]
    pub versioning: Versioning,
}

/// How releases are versioned.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Versioning {
    /// `v<base>-<label>.<YYYYMMDD>.<n>`, from the upstream's version.
    #[default]
    Dated,
    /// `v<x.y.z>`, from the changes since the previous release.
    Semver,
}

impl Release {
    /// The label of new releases.
    pub fn label(&self) -> &str {
        &self.labels[0]
    }
}

/// The label of releases when `[release]` sets no `labels`.
pub const DEFAULT_LABEL: &str = "custom";

/// One check: a name, unique ignoring letter case, and the command it runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Check {
    pub name: String,
    pub run: String,
}

/// One upstream: a source taken at the root or under its own path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Upstream {
    pub name: String,
    /// Where the upstream is fetched from; the tool keeps no remote for it.
    pub url: String,
    pub branch: String,
    pub mode: Mode,
    /// Normalised path inside the repository; `None` is the root.
    pub path: Option<String>,
    /// Follow mode: a glob over the upstream's tag names that makes `start`
    /// take the newest release tag on the branch instead of its tip.
    #[serde(default)]
    pub tags: Option<String>,
    /// Follow mode with `tags`: whether tags with a prerelease part count as
    /// targets.
    #[serde(default)]
    pub prereleases: bool,
    /// Follow mode with `tags`: the most files a target's merge may be
    /// predicted to conflict in, when an update names no revision.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_conflicts: Option<u32>,
}

impl Upstream {
    /// The upstream branch as `name/branch`, for messages and sync records.
    pub fn source(&self) -> String {
        format!("{}/{}", self.name, self.branch)
    }
}

/// The part of the environment variable names `verify` gives the checks,
/// `GIT_INTAKE_UPSTREAM_<NAME>_*`, that stands for the upstream `name`: the
/// name in upper case with every character other than a letter or a digit
/// replaced by `_`.
pub fn env_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect()
}

impl Config {
    pub fn upstream(&self, name: &str) -> Option<&Upstream> {
        self.upstreams.iter().find(|u| u.name == name)
    }

    /// Selects upstreams by name in configuration order; no names selects all.
    pub fn select(&self, names: &[String]) -> Result<Vec<&Upstream>> {
        let unknown: Vec<&str> = names
            .iter()
            .filter(|n| self.upstream(n).is_none())
            .map(String::as_str)
            .collect();
        if !unknown.is_empty() {
            let known: Vec<&str> = self.upstreams.iter().map(|u| u.name.as_str()).collect();
            bail!(
                "unknown upstream {}; configured upstreams: {}",
                quote_list(&unknown),
                known.join(", ")
            );
        }
        Ok(self
            .upstreams
            .iter()
            .filter(|u| names.is_empty() || names.contains(&u.name))
            .collect())
    }
}

fn quote_list(items: &[&str]) -> String {
    items
        .iter()
        .map(|i| format!("'{i}'"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Parses and validates configuration text. All problems are reported at once.
pub fn parse(text: &str) -> Result<Config> {
    let raw: RawConfig = match toml::from_str(text) {
        Ok(raw) => raw,
        Err(err) => bail!("invalid {FILE_NAME}: {}", err.message()),
    };
    let mut problems = Vec::new();

    let integration = raw.integration.trim().to_string();
    check_branch("integration", &integration, &mut problems);
    let publish = match raw.publish {
        None => None,
        Some(value) => match parse_publish(value) {
            Ok(publish) => {
                check_remote("publish remote", &publish.remote, &mut problems);
                Some(publish)
            }
            Err(why) => {
                problems.push(why);
                None
            }
        },
    };
    if let Some(install) = &raw.install
        && install.trim().is_empty()
    {
        problems.push("install: the command is empty".to_string());
    }
    let checks = match raw.checks {
        None => Vec::new(),
        Some(value) => parse_checks(value, &mut problems),
    };
    if raw.upstreams.is_empty() {
        problems.push("no [[upstream]] is configured".to_string());
    }

    let mut upstreams = Vec::new();
    for (i, up) in raw.upstreams.into_iter().enumerate() {
        let label = if up.name.trim().is_empty() {
            format!("upstream[{i}]")
        } else {
            format!("upstream '{}'", up.name)
        };
        check_name(&label, &up.name, &mut problems);
        check_branch(&format!("{label} branch"), &up.branch, &mut problems);
        if up.remote.is_some() {
            problems.push(format!(
                "{label} remote: upstreams are fetched by their URL and the tool keeps no remote for them; \
                 replace `remote` with `url` set to that remote's URL"
            ));
        }
        if up.mirror.is_some() {
            problems.push(format!(
                "{label} mirror: mirror branches are no longer kept; remove `mirror` \
                 (an existing mirror branch stays an ordinary branch)"
            ));
        }
        let url = match up.url {
            Some(url) if url.trim().is_empty() => {
                problems.push(format!("{label} url: the URL is empty"));
                String::new()
            }
            Some(url) if crate::redact::has_credentials(&url) => {
                problems.push(format!(
                    "{label} url: '{}' carries credentials, which belong in the environment; give the URL \
                     without them and let git's credential helper or SSH key supply them",
                    crate::redact::redacted(&url)
                ));
                url
            }
            Some(url) => url,
            None => {
                if up.remote.is_none() {
                    problems.push(format!("{label}: `url` is required"));
                }
                String::new()
            }
        };
        let path = match &up.path {
            None => None,
            Some(p) => match normalise_path(p) {
                Ok(p) => Some(p),
                Err(why) => {
                    problems.push(format!("{label} path '{p}': {why}"));
                    None
                }
            },
        };
        if up.mode == Mode::Port && up.path.is_some() {
            problems.push(format!(
                "{label}: an upstream in `port` mode is taken at the repository root and cannot have a path"
            ));
        }
        if let Some(tags) = &up.tags {
            if up.mode == Mode::Port {
                problems.push(format!(
                    "{label}: `tags` selects the target of a follow upstream; an upstream in `port` mode cannot have it"
                ));
            } else if let Err(why) = crate::tags::TagPattern::parse(tags) {
                problems.push(format!("{label} tags '{tags}': {why}"));
            }
        }
        for (key, set) in [
            ("prereleases", up.prereleases.is_some()),
            ("max_conflicts", up.max_conflicts.is_some()),
        ] {
            if set && up.tags.is_none() {
                problems.push(format!(
                    "{label}: `{key}` applies to the tags a follow upstream is taken at; set `tags` to their pattern, \
                     or remove `{key}`"
                ));
            }
        }
        let max_conflicts = match up.max_conflicts {
            None => None,
            Some(n) => match u32::try_from(n) {
                Ok(n) if n > 0 => Some(n),
                _ => {
                    problems.push(format!(
                        "{label} max_conflicts: {n} is not a positive number of files"
                    ));
                    None
                }
            },
        };
        upstreams.push(Upstream {
            name: up.name,
            url,
            branch: up.branch,
            mode: up.mode,
            path,
            tags: up.tags,
            prereleases: up.prereleases.unwrap_or(false),
            max_conflicts,
        });
    }

    check_layout(&integration, &upstreams, &mut problems);
    if let Some(publish) = &publish {
        check_replacement(publish, &upstreams, &mut problems);
    }
    let release = match raw.release {
        None => None,
        Some(value) => parse_release(value, &upstreams, publish.is_some(), &mut problems),
    };

    if !problems.is_empty() {
        bail!(
            "invalid {FILE_NAME}:\n{}",
            problems
                .iter()
                .map(|p| format!("  - {p}"))
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
    Ok(Config {
        integration,
        publish,
        install: raw.install,
        checks,
        upstreams,
        release,
    })
}

/// Reads the `[release]` table against the upstreams it may name.
fn parse_release(
    value: toml::Value,
    upstreams: &[Upstream],
    publish: bool,
    problems: &mut Vec<String>,
) -> Option<Release> {
    let raw: RawRelease = match value.try_into() {
        Ok(raw) => raw,
        Err(e) => {
            let e: toml::de::Error = e;
            problems.push(format!("release: {}", e.message()));
            return None;
        }
    };
    let mut valid = true;
    if !publish {
        problems.push(
            "release: releases are cut on the publish remote's integration branch; configure the publish \
             remote in a [publish] table"
                .to_string(),
        );
        valid = false;
    }
    let upstream = upstreams.iter().find(|u| u.name == raw.upstream);
    match upstream {
        None => {
            let known: Vec<&str> = upstreams.iter().map(|u| u.name.as_str()).collect();
            problems.push(format!(
                "release.upstream: no upstream is named '{}'; configured upstreams: {}",
                raw.upstream,
                known.join(", ")
            ));
            valid = false;
        }
        Some(up) if up.mode != Mode::Follow => {
            problems.push(format!(
                "release.upstream: upstream '{}' is in `{}` mode; releases take their version from an upstream \
                 in `follow` mode",
                up.name,
                up.mode.as_str()
            ));
            valid = false;
        }
        Some(_) => {}
    }
    let tags = match (raw.tags, upstream) {
        (Some(tags), _) => match crate::tags::TagPattern::parse(&tags) {
            Ok(_) => Some(tags),
            Err(why) => {
                problems.push(format!("release.tags '{tags}': {why}"));
                None
            }
        },
        (None, Some(up)) if up.tags.is_some() => up.tags.clone(),
        (None, Some(up)) => {
            problems.push(format!(
                "release.tags: upstream '{}' has no `tags` pattern; set `tags` in [release] to the glob of its \
                 release tags, such as \"v*\"",
                up.name
            ));
            None
        }
        (None, None) => None,
    };
    let labels = raw
        .labels
        .unwrap_or_else(|| vec![DEFAULT_LABEL.to_string()]);
    let mut labels_valid = true;
    if labels.is_empty() {
        problems.push(
            "release.labels: the list is empty; name at least the label of new releases"
                .to_string(),
        );
        labels_valid = false;
    }
    for label in &labels {
        if let Err(why) = check_label(label) {
            problems.push(format!("release.labels: {why}"));
            labels_valid = false;
        }
    }
    for (i, label) in labels.iter().enumerate() {
        if labels[..i].contains(label) {
            problems.push(format!(
                "release.labels: '{label}' is listed more than once"
            ));
            labels_valid = false;
        }
    }
    let versioning = match raw.versioning.as_deref() {
        None | Some("dated") => Some(Versioning::Dated),
        Some("semver") => Some(Versioning::Semver),
        Some(other) => {
            problems.push(format!(
                "release.versioning: '{other}' is neither \"dated\" nor \"semver\""
            ));
            None
        }
    };
    if !valid || !labels_valid {
        return None;
    }
    Some(Release {
        upstream: raw.upstream,
        tags: tags?,
        labels,
        versioning: versioning?,
    })
}

/// The rule for a release label, a SemVer prerelease identifier that is not
/// a number.
fn check_label(label: &str) -> std::result::Result<(), String> {
    if label.is_empty() {
        Err("a label is empty".to_string())
    } else if !label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        Err(format!(
            "'{label}' is not a valid label: it may contain only ASCII letters, digits and '-'"
        ))
    } else if label.chars().all(|c| c.is_ascii_digit()) {
        Err(format!(
            "'{label}' is not a valid label: a label of digits alone would read as a number in the version"
        ))
    } else {
        Ok(())
    }
}

/// Reads the list of checks, each a table with exactly the keys `name` and
/// `run`. A list of plain command strings, the form of earlier releases, is
/// refused with the table form written out for it.
fn parse_checks(value: toml::Value, problems: &mut Vec<String>) -> Vec<Check> {
    let toml::Value::Array(entries) = value else {
        problems.push(
            "checks: expected a list of tables such as `checks = [{ name = \"test\", run = \"make test\" }]`"
                .to_string(),
        );
        return Vec::new();
    };
    let plain: Vec<&str> = entries.iter().filter_map(toml::Value::as_str).collect();
    if !entries.is_empty() && plain.len() == entries.len() {
        problems.push(format!(
            "checks: each check is a table with a name and the command it runs, not a plain command; write `checks = [{}]`, \
             naming each check",
            plain
                .iter()
                .map(|run| table_form(run))
                .collect::<Vec<_>>()
                .join(", ")
        ));
        return Vec::new();
    }
    let mut checks = Vec::new();
    // Every name given, valid or not, so that a repeated name is reported
    // with the other problems of its checks.
    let mut names: Vec<&str> = Vec::new();
    for (i, entry) in entries.iter().enumerate() {
        let table = match entry {
            toml::Value::Table(table) => table,
            toml::Value::String(run) => {
                problems.push(format!(
                    "checks[{i}]: a check is a table with a name and the command it runs, not a plain command; \
                     write `{}`, naming the check",
                    table_form(run)
                ));
                continue;
            }
            _ => {
                problems.push(format!(
                    "checks[{i}]: expected a table with the keys `name` and `run`"
                ));
                continue;
            }
        };
        let name = table.get("name").and_then(toml::Value::as_str);
        names.extend(name.filter(|n| !n.is_empty()));
        let label = match name {
            Some(name) if !name.is_empty() => format!("check '{name}'"),
            _ => format!("checks[{i}]"),
        };
        let mut valid = true;
        for key in table
            .keys()
            .filter(|k| !matches!(k.as_str(), "name" | "run"))
        {
            problems.push(format!(
                "{label}: unknown key `{key}`; a check has exactly the keys `name` and `run`"
            ));
            valid = false;
        }
        match (table.get("name"), name) {
            (None, _) => {
                problems.push(format!("{label}: `name` is required"));
                valid = false;
            }
            (Some(_), None) => {
                problems.push(format!("{label}: `name` must be a string"));
                valid = false;
            }
            (Some(_), Some(name)) => {
                if let Err(why) = check_check_name(name) {
                    problems.push(format!("{label}: {why}"));
                    valid = false;
                }
            }
        }
        let run = match table.get("run") {
            None => {
                problems.push(format!("{label}: `run` is required"));
                None
            }
            Some(toml::Value::String(run)) if run.trim().is_empty() => {
                problems.push(format!("{label}: the command is empty"));
                None
            }
            Some(toml::Value::String(run)) => Some(run.clone()),
            Some(_) => {
                problems.push(format!("{label}: `run` must be a string"));
                None
            }
        };
        if let (true, Some(name), Some(run)) = (valid, name, run) {
            checks.push(Check {
                name: name.to_string(),
                run,
            });
        }
    }
    let mut groups: Vec<(String, Vec<&str>)> = Vec::new();
    for name in names {
        let key = name.to_ascii_lowercase();
        match groups.iter_mut().find(|(k, _)| *k == key) {
            Some((_, same)) => same.push(name),
            None => groups.push((key, vec![name])),
        }
    }
    for (_, names) in groups.into_iter().filter(|(_, n)| n.len() > 1) {
        let (last, first) = names.split_last().unwrap_or((&"", &[]));
        problems.push(format!(
            "checks {} and '{last}' share a name; check names are unique ignoring letter case",
            quote_list(first)
        ));
    }
    checks
}

/// A check in the table form, with the command `run` and a name to fill in.
fn table_form(run: &str) -> String {
    format!(
        "{{ name = \"...\", run = {} }}",
        toml::Value::String(run.to_string())
    )
}

/// The rule for a check's name, which names its log file and selects it
/// for `verify --only` and `--skip`.
fn check_check_name(name: &str) -> std::result::Result<(), String> {
    if name.is_empty() {
        Err("the name is empty".to_string())
    } else if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        || name.starts_with(['-', '.'])
    {
        Err(format!(
            "'{name}' is not a valid check name: it may contain only ASCII letters, digits, '-', '_' and '.', \
             and may not start with '-' or '.'"
        ))
    } else {
        Ok(())
    }
}

/// Reads the `[publish]` table.
fn parse_publish(value: toml::Value) -> std::result::Result<Publish, String> {
    if let toml::Value::String(remote) = &value {
        return Err(format!(
            "publish: expected a table, not a string; write `publish = {{ remote = {remote:?} }}`, \
             adding `via = \"merge-request\"` to publish through a merge request, or a `[publish]` table \
             with the same keys placed after the top-level keys"
        ));
    }
    let raw: RawPublish = value
        .try_into()
        .map_err(|e: toml::de::Error| format!("publish: {}", e.message()))?;
    let forge = match raw.forge.as_deref().map(str::trim) {
        None => None,
        Some("gitlab") => Some(ForgeKind::Gitlab),
        Some("github") => Some(ForgeKind::Github),
        Some(other) => {
            return Err(format!(
                "publish forge: '{other}' is not a forge the tool reaches; use `gitlab` or `github`"
            ));
        }
    };
    let api = match raw.api.as_deref().map(str::trim) {
        None => None,
        Some(api) => Some(check_api(api)?),
    };
    let queue = match raw.queue.as_deref().map(str::trim) {
        None => None,
        Some(_) if raw.via == PublishVia::Push => {
            return Err(
                "publish queue: a merge queue takes merge requests, and `via = \"push\"` opens none; remove \
                 `queue`, or publish with `via = \"merge-request\"`"
                    .to_string(),
            );
        }
        Some(account) => Some(check_account(account)?),
    };
    let replace_after = match raw.replace_after.as_deref() {
        None => None,
        Some(text) => Some(ReplaceAfter::parse(text)?),
    };
    Ok(Publish {
        remote: raw.remote,
        via: raw.via,
        forge,
        api,
        queue,
        replace_after,
    })
}

/// Validates the setting `queue`: an account name as a forge writes it,
/// without the `@` of a mention.
fn check_account(account: &str) -> std::result::Result<String, String> {
    let account = account.strip_prefix('@').unwrap_or(account);
    if account.is_empty()
        || !account
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        return Err(format!(
            "publish queue: '{account}' is not an account name; give the name of the merge queue's account, \
             such as \"merge-bot\""
        ));
    }
    Ok(account.to_string())
}

/// Refuses `replace_after` where it would do nothing: in push mode, which
/// opens no merge request, and without a follow upstream with `tags`, whose
/// releases alone trigger a replacement.
fn check_replacement(publish: &Publish, upstreams: &[Upstream], problems: &mut Vec<String>) {
    let Some(after) = &publish.replace_after else {
        return;
    };
    if publish.via != PublishVia::MergeRequest {
        problems.push(format!(
            "publish replace_after: '{}' replaces the update's merge request, which needs merge-request mode; \
             set `via = \"merge-request\"` or remove `replace_after`",
            after.text()
        ));
    }
    if !upstreams
        .iter()
        .any(|u| u.mode == Mode::Follow && u.tags.is_some())
    {
        problems.push(format!(
            "publish replace_after: '{}' does nothing here: only a newer release of a follow upstream with `tags` \
             triggers a replacement, and no follow upstream has `tags`",
            after.text()
        ));
    }
}

/// Validates the setting `api`: an `https` URL with a host, without
/// credentials, query or fragment. Returns it without a trailing `/`.
fn check_api(api: &str) -> std::result::Result<String, String> {
    let refuse = |why: &str| {
        format!(
            "publish api: '{}' {why}; give the base URL of the forge's REST API as an `https` URL, \
             such as `https://gitlab.example.com/api/v4` or `https://github.example.com/api/v3`",
            crate::redact::redacted(api)
        )
    };
    let url = url::Url::parse(api).map_err(|e| refuse(&format!("is not a URL ({e})")))?;
    if url.scheme() != "https" {
        return Err(refuse("is not an `https` URL"));
    }
    if url.host_str().is_none_or(str::is_empty) {
        return Err(refuse("names no host"));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(refuse(
            "carries credentials, which belong in the environment",
        ));
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(refuse("has a query or a fragment"));
    }
    Ok(api.trim_end_matches('/').to_string())
}

/// Cross-entry rules: unique names and sources, distinct environment
/// variables, non-nesting paths, a root upstream alone.
fn check_layout(integration: &str, upstreams: &[Upstream], problems: &mut Vec<String>) {
    duplicates(upstreams.iter().map(|u| u.name.as_str()), "name", problems);
    let mut by_variable: Vec<(String, Vec<&str>)> = Vec::new();
    for up in upstreams.iter().filter(|u| !u.name.is_empty()) {
        let variable = env_name(&up.name);
        match by_variable.iter_mut().find(|(v, _)| *v == variable) {
            Some((_, names)) if !names.contains(&up.name.as_str()) => names.push(&up.name),
            Some(_) => {}
            None => by_variable.push((variable, vec![&up.name])),
        }
    }
    for (variable, names) in by_variable.into_iter().filter(|(_, n)| n.len() > 1) {
        let (last, first) = names.split_last().unwrap_or((&"", &[]));
        problems.push(format!(
            "upstreams {} and '{last}' share the environment variables GIT_INTAKE_UPSTREAM_{variable}_* \
             that verify gives the checks; rename them so that they differ in more than case and punctuation",
            quote_list(first)
        ));
    }
    let mut sources: Vec<(&str, &str, usize)> = Vec::new();
    for up in upstreams.iter().filter(|u| !u.url.is_empty()) {
        match sources
            .iter_mut()
            .find(|(url, branch, _)| *url == up.url && *branch == up.branch)
        {
            Some(seen) => seen.2 += 1,
            None => sources.push((&up.url, &up.branch, 1)),
        }
    }
    for (url, branch, n) in sources.into_iter().filter(|s| s.2 > 1) {
        problems.push(format!(
            "{n} upstreams share the URL '{}' and the branch '{branch}'",
            crate::redact::redacted(url)
        ));
    }

    if upstreams.len() > 1 {
        for (i, root) in upstreams.iter().enumerate() {
            if root.path.is_some() {
                continue;
            }
            let others: Vec<&str> = upstreams
                .iter()
                .enumerate()
                .filter(|(j, _)| *j != i)
                .map(|(_, u)| u.name.as_str())
                .collect();
            problems.push(format!(
                "upstream '{}' has no path (the repository root) and cannot share the repository with {}",
                root.name,
                quote_list(&others)
            ));
        }
    }

    for (i, a) in upstreams.iter().enumerate() {
        for b in &upstreams[i + 1..] {
            if let (Some(pa), Some(pb)) = (&a.path, &b.path)
                && let Some((outer, inner)) = nesting(a, pa, b, pb)
            {
                problems.push(format!(
                    "upstream '{}' path '{}' lies inside upstream '{}' path '{}'",
                    inner.0, inner.1, outer.0, outer.1
                ));
            }
        }
    }

    if integration == crate::state::UPDATE_BRANCH {
        problems.push(format!(
            "integration: '{integration}' is reserved for updates"
        ));
    }
}

type Named<'a> = (&'a str, &'a str);

/// Returns `(outer, inner)` when one path contains the other.
fn nesting<'a>(
    a: &'a Upstream,
    pa: &'a str,
    b: &'a Upstream,
    pb: &'a str,
) -> Option<(Named<'a>, Named<'a>)> {
    let inside =
        |inner: &str, outer: &str| inner == outer || inner.starts_with(&format!("{outer}/"));
    if inside(pb, pa) {
        Some(((&a.name, pa), (&b.name, pb)))
    } else if inside(pa, pb) {
        Some(((&b.name, pb), (&a.name, pa)))
    } else {
        None
    }
}

fn duplicates<'a>(values: impl Iterator<Item = &'a str>, what: &str, problems: &mut Vec<String>) {
    let mut seen: HashMap<&str, usize> = HashMap::new();
    let mut order = Vec::new();
    for v in values {
        let n = seen.entry(v).or_insert(0);
        *n += 1;
        if *n == 2 {
            order.push(v);
        }
    }
    for v in order {
        problems.push(format!("{} upstreams share the {what} '{v}'", seen[v]));
    }
}

/// Normalises an upstream path: no leading or trailing slash, no empty, `.`
/// or `..` segment, not empty.
pub fn normalise_path(path: &str) -> std::result::Result<String, &'static str> {
    let trimmed = path.trim().trim_matches('/');
    if trimmed.is_empty() {
        return Err("the path is empty; omit `path` for the repository root");
    }
    if trimmed.contains('\\') {
        return Err("use '/' to separate path segments");
    }
    for segment in trimmed.split('/') {
        match segment {
            "" => return Err("the path has an empty segment"),
            "." | ".." => return Err("the path may not contain '.' or '..' segments"),
            s if s.eq_ignore_ascii_case(".git") => {
                return Err("the path may not contain a '.git' segment");
            }
            s if s.chars().any(char::is_control) => {
                return Err("the path contains a control character");
            }
            _ => {}
        }
    }
    Ok(trimmed.to_string())
}

fn check_name(label: &str, name: &str, problems: &mut Vec<String>) {
    if name.is_empty() {
        problems.push(format!("{label}: the name is empty"));
    } else if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        || name.starts_with(['-', '.'])
    {
        problems.push(format!(
            "{label}: the name may contain only letters, digits, '-', '_' and '.', and may not start with '-' or '.'"
        ));
    } else if name.contains("..") || name.ends_with('.') || name.ends_with(".lock") {
        // The name is a component of the references that hold the upstream,
        // `refs/git-intake/upstreams/<name>`, which git refuses with these.
        problems.push(format!(
            "{label}: '{name}' cannot name a git reference: it may not contain '..' or end with '.' or '.lock'"
        ));
    }
}

fn check_remote(label: &str, remote: &str, problems: &mut Vec<String>) {
    if remote.is_empty() {
        problems.push(format!("{label}: the remote name is empty"));
    } else if remote.starts_with('-')
        || remote.chars().any(|c| {
            c.is_whitespace()
                || c.is_control()
                || matches!(c, ':' | '\\' | '*' | '?' | '[' | '~' | '^')
        })
        || remote.contains("..")
    {
        problems.push(format!("{label}: '{remote}' is not a valid remote name"));
    }
}

/// An approximation of `git check-ref-format --branch` that needs no git.
fn check_branch(label: &str, branch: &str, problems: &mut Vec<String>) {
    let invalid = branch.is_empty()
        || branch.starts_with(['-', '/'])
        || branch.ends_with(['/', '.'])
        || branch.ends_with(".lock")
        || branch.contains("..")
        || branch.contains("//")
        || branch.contains("@{")
        || branch == "@"
        || branch.chars().any(|c| {
            c.is_whitespace()
                || c.is_control()
                || matches!(c, '~' | '^' | ':' | '?' | '*' | '[' | '\\')
        })
        || branch.split('/').any(|s| s.starts_with('.'));
    if invalid {
        problems.push(format!("{label}: '{branch}' is not a valid branch name"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn err(text: &str) -> String {
        format!("{:#}", parse(text).unwrap_err())
    }

    const UP_A: &str = "[[upstream]]\nname = \"a\"\nurl = \"https://example.com/a.git\"\nbranch = \"main\"\npath = \"apps/a\"\n";
    const UP_B: &str = "[[upstream]]\nname = \"b\"\nurl = \"https://example.com/b.git\"\nbranch = \"main\"\npath = \"apps/b\"\n";

    #[test]
    fn parses_full_configuration() {
        let cfg = parse(
            r#"
integration = "main"
install = "make deps"
checks = [{ name = "lint", run = "make lint" }, { name = "test", run = "make test" }]

[[upstream]]
name = "app-a"
url = "git@example.com:team/app-a.git"
branch = "main"
path = "/apps/app-a/"

[publish]
remote = "origin"
via = "merge-request"
"#,
        )
        .unwrap();
        assert_eq!(cfg.integration, "main");
        assert_eq!(
            cfg.publish,
            Some(Publish {
                remote: "origin".into(),
                via: PublishVia::MergeRequest,
                forge: None,
                api: None,
                queue: None,
                replace_after: None,
            })
        );
        assert_eq!(
            cfg.checks,
            [
                Check {
                    name: "lint".into(),
                    run: "make lint".into()
                },
                Check {
                    name: "test".into(),
                    run: "make test".into()
                }
            ]
        );
        let up = &cfg.upstreams[0];
        assert_eq!(up.path.as_deref(), Some("apps/app-a"));
        assert_eq!(up.url, "git@example.com:team/app-a.git");
        assert_eq!(up.mode, Mode::Follow);
        assert_eq!(up.source(), "app-a/main");
    }

    #[test]
    fn root_upstream_alone_is_valid() {
        let cfg = parse(
            "integration = \"main\"\n[[upstream]]\nname = \"up\"\nurl = \"../up.git\"\nbranch = \"main\"\nmode = \"port\"\n",
        )
        .unwrap();
        assert_eq!(cfg.upstreams[0].path, None);
        assert_eq!(cfg.upstreams[0].mode, Mode::Port);
        assert!(cfg.checks.is_empty());
    }

    #[test]
    fn rejects_unknown_keys_and_modes() {
        let e = err("integration = \"main\"\nfoo = 1\n");
        assert!(e.contains("unknown field `foo`"), "{e}");
        let e = err(
            "integration = \"main\"\n[[upstream]]\nname = \"a\"\nurl = \"u\"\nbranch = \"main\"\nsquash = true\n",
        );
        assert!(e.contains("unknown field `squash`"), "{e}");
        let e = err(
            "integration = \"main\"\n[[upstream]]\nname = \"a\"\nurl = \"u\"\nbranch = \"main\"\nmode = \"pick\"\n",
        );
        assert!(e.contains("unknown variant `pick`"), "{e}");
    }

    #[test]
    fn rejects_missing_upstreams_and_urls() {
        assert!(err("integration = \"main\"\n").contains("no [[upstream]]"));
        let e = err("integration = \"main\"\n[[upstream]]\nname = \"a\"\nbranch = \"main\"\n");
        assert!(e.contains("upstream 'a': `url` is required"), "{e}");
        let e = err(
            "integration = \"main\"\n[[upstream]]\nname = \"a\"\nurl = \" \"\nbranch = \"main\"\n",
        );
        assert!(e.contains("upstream 'a' url: the URL is empty"), "{e}");
    }

    #[test]
    fn removed_settings_name_their_replacement() {
        let e = err(
            "integration = \"main\"\n[[upstream]]\nname = \"a\"\nremote = \"a\"\nbranch = \"main\"\nmirror = \"upstream-a\"\n",
        );
        assert!(
            e.contains("upstream 'a' remote: upstreams are fetched by their URL"),
            "{e}"
        );
        assert!(e.contains("replace `remote` with `url`"), "{e}");
        assert!(
            e.contains("upstream 'a' mirror: mirror branches are no longer kept"),
            "{e}"
        );
        assert!(!e.contains("`url` is required"), "{e}");
    }

    #[test]
    fn rejects_a_port_upstream_with_a_path() {
        let e = err(&format!(
            "integration = \"main\"\n{}",
            UP_A.replace("branch", "mode = \"port\"\nbranch")
        ));
        assert!(
            e.contains("upstream 'a': an upstream in `port` mode is taken at the repository root"),
            "{e}"
        );
    }

    #[test]
    fn release_tags_are_a_valid_pattern_of_a_follow_upstream() {
        let cfg = parse(&format!(
            "integration = \"main\"\n{}",
            UP_A.replace("branch", "tags = \"v*\"\nbranch")
        ))
        .unwrap();
        assert_eq!(cfg.upstreams[0].tags.as_deref(), Some("v*"));
        let e = err(&format!(
            "integration = \"main\"\n{}",
            UP_A.replace("branch", "tags = \"refs/tags/v*\"\nbranch")
        ));
        assert!(
            e.contains("upstream 'a' tags 'refs/tags/v*': the pattern matches tag names"),
            "{e}"
        );
        let e = err(&format!(
            "integration = \"main\"\n{}",
            UP_A.replace("branch", "tags = \"v[\"\nbranch")
        ));
        assert!(
            e.contains("upstream 'a' tags 'v[': not a valid glob"),
            "{e}"
        );
        let e = err(
            "integration = \"main\"\n[[upstream]]\nname = \"a\"\nurl = \"u\"\nbranch = \"main\"\nmode = \"port\"\ntags = \"v*\"\n",
        );
        assert!(
            e.contains("upstream 'a': `tags` selects the target of a follow upstream"),
            "{e}"
        );
    }

    #[test]
    fn prereleases_and_the_conflict_limit_need_tags() {
        let cfg = parse(&format!(
            "integration = \"main\"\n{}",
            UP_A.replace(
                "branch",
                "tags = \"v*\"\nprereleases = true\nmax_conflicts = 150\nbranch"
            )
        ))
        .unwrap();
        assert!(cfg.upstreams[0].prereleases);
        assert_eq!(cfg.upstreams[0].max_conflicts, Some(150));
        let cfg = parse(&format!("integration = \"main\"\n{UP_A}")).unwrap();
        assert!(!cfg.upstreams[0].prereleases);
        assert_eq!(cfg.upstreams[0].max_conflicts, None);
        let e = err(&format!(
            "integration = \"main\"\n{}",
            UP_A.replace("branch", "prereleases = true\nmax_conflicts = 5\nbranch")
        ));
        assert!(
            e.contains("upstream 'a': `prereleases` applies to the tags"),
            "{e}"
        );
        assert!(
            e.contains("upstream 'a': `max_conflicts` applies to the tags"),
            "{e}"
        );
        for bad in ["0", "-3"] {
            let e = err(&format!(
                "integration = \"main\"\n{}",
                UP_A.replace(
                    "branch",
                    &format!("tags = \"v*\"\nmax_conflicts = {bad}\nbranch")
                )
            ));
            assert!(
                e.contains(&format!(
                    "upstream 'a' max_conflicts: {bad} is not a positive number of files"
                )),
                "{e}"
            );
        }
    }

    #[test]
    fn the_release_table_takes_its_defaults_from_the_upstream() {
        let up = UP_A.replace("branch", "tags = \"v*\"\nbranch");
        let cfg = parse(&format!(
            "integration = \"main\"\n{up}[publish]\nremote = \"origin\"\n[release]\nupstream = \"a\"\n"
        ))
        .unwrap();
        assert_eq!(
            cfg.release,
            Some(Release {
                upstream: "a".into(),
                tags: "v*".into(),
                labels: vec!["custom".into()],
                versioning: Versioning::Dated,
            })
        );
        let cfg = parse(&format!(
            "integration = \"main\"\n{UP_A}[publish]\nremote = \"origin\"\n[release]\nupstream = \"a\"\n\
             tags = \"expo@*\"\nlabels = [\"nightly\", \"custom-2\"]\n"
        ))
        .unwrap();
        let release = cfg.release.unwrap();
        assert_eq!(release.tags, "expo@*");
        assert_eq!(release.label(), "nightly");
        // The upstream's own `tags` stays unset: updates take its branch tip.
        assert_eq!(cfg.upstreams[0].tags, None);
        let cfg = parse(&format!(
            "integration = \"main\"\n{up}[publish]\nremote = \"origin\"\n[release]\nupstream = \"a\"\n\
             versioning = \"semver\"\n"
        ))
        .unwrap();
        assert_eq!(cfg.release.unwrap().versioning, Versioning::Semver);
    }

    #[test]
    fn rejects_invalid_release_tables() {
        let publish = "[publish]\nremote = \"origin\"\n";
        let port = "[[upstream]]\nname = \"p\"\nurl = \"u\"\nbranch = \"main\"\nmode = \"port\"\n";
        let e = err(&format!(
            "integration = \"main\"\n{port}{publish}[release]\nupstream = \"p\"\ntags = \"v*\"\n"
        ));
        assert!(
            e.contains("release.upstream: upstream 'p' is in `port` mode"),
            "{e}"
        );
        let e = err(&format!(
            "integration = \"main\"\n{UP_A}{publish}[release]\nupstream = \"x\"\ntags = \"v*\"\n"
        ));
        assert!(
            e.contains("release.upstream: no upstream is named 'x'"),
            "{e}"
        );
        let e = err(&format!(
            "integration = \"main\"\n{UP_A}{publish}[release]\nupstream = \"a\"\n"
        ));
        assert!(
            e.contains("release.tags: upstream 'a' has no `tags` pattern"),
            "{e}"
        );
        let e = err(&format!(
            "integration = \"main\"\n{UP_A}{publish}[release]\nupstream = \"a\"\ntags = \"refs/tags/v*\"\n"
        ));
        assert!(e.contains("release.tags 'refs/tags/v*'"), "{e}");
        for (labels, why) in [
            ("[\"2024\"]", "'2024' is not a valid label"),
            ("[]", "the list is empty"),
            ("[\"a.b\"]", "'a.b' is not a valid label"),
            ("[\"x\", \"x\"]", "'x' is listed more than once"),
        ] {
            let e = err(&format!(
                "integration = \"main\"\n{UP_A}{publish}[release]\nupstream = \"a\"\ntags = \"v*\"\nlabels = {labels}\n"
            ));
            assert!(e.contains(&format!("release.labels: {why}")), "{e}");
        }
        let e = err(&format!(
            "integration = \"main\"\n{UP_A}[release]\nupstream = \"a\"\ntags = \"v*\"\n"
        ));
        assert!(
            e.contains("release: releases are cut on the publish remote"),
            "{e}"
        );
        let e = err(&format!(
            "integration = \"main\"\n{UP_A}{publish}[release]\nupstream = \"a\"\nlabel = \"x\"\n"
        ));
        assert!(e.contains("release: unknown field `label`"), "{e}");
        let e = err(&format!(
            "integration = \"main\"\n{UP_A}{publish}[release]\nupstream = \"a\"\ntags = \"v*\"\nversioning = \"calendar\"\n"
        ));
        assert!(
            e.contains("release.versioning: 'calendar' is neither \"dated\" nor \"semver\""),
            "{e}"
        );
    }

    #[test]
    fn rejects_duplicate_names_and_sources() {
        let text = format!(
            "integration = \"main\"\n{UP_A}{}",
            UP_B.replace("\"b\"", "\"a\"").replace("b.git", "a.git")
        );
        let e = err(&text);
        assert!(e.contains("2 upstreams share the name 'a'"), "{e}");
        assert!(
            e.contains(
                "2 upstreams share the URL 'https://example.com/a.git' and the branch 'main'"
            ),
            "{e}"
        );
        let other_branch = format!(
            "integration = \"main\"\n{UP_A}{}",
            UP_B.replace("b.git", "a.git")
                .replace("branch = \"main\"", "branch = \"next\"")
        );
        assert!(parse(&other_branch).is_ok());
    }

    #[test]
    fn rejects_nested_paths() {
        let text = format!(
            "integration = \"main\"\n{UP_A}{}",
            UP_B.replace("apps/b", "apps/a/sub")
        );
        let e = err(&text);
        assert!(
            e.contains("upstream 'b' path 'apps/a/sub' lies inside upstream 'a' path 'apps/a'"),
            "{e}"
        );
        let same = format!(
            "integration = \"main\"\n{UP_A}{}",
            UP_B.replace("apps/b", "apps/a")
        );
        assert!(err(&same).contains("lies inside"));
    }

    #[test]
    fn sibling_prefix_is_not_nesting() {
        let text = format!(
            "integration = \"main\"\n{UP_A}{}",
            UP_B.replace("apps/b", "apps/ab")
        );
        assert!(parse(&text).is_ok());
    }

    #[test]
    fn rejects_root_upstream_with_others() {
        let text = format!(
            "integration = \"main\"\n{UP_A}{}",
            UP_B.replace("path = \"apps/b\"\n", "")
        );
        let e = err(&text);
        assert!(
            e.contains("upstream 'b' has no path (the repository root) and cannot share the repository with 'a'"),
            "{e}"
        );
    }

    #[test]
    fn publish_defaults_to_push() {
        let cfg = parse(&format!(
            "integration = \"main\"\n{UP_A}[publish]\nremote = \"origin\"\n"
        ))
        .unwrap();
        assert_eq!(cfg.publish.unwrap().via, PublishVia::Push);
        let cfg = parse(&format!("integration = \"main\"\n{UP_A}")).unwrap();
        assert_eq!(cfg.publish, None);
    }

    #[test]
    fn checks_are_tables_in_order() {
        let cfg = parse(&format!(
            "integration = \"main\"\n{UP_A}[[checks]]\nname = \"Unit.tests_2\"\nrun = \"make test\"\n\
             [[checks]]\nname = \"lint\"\nrun = \"make lint\"\n"
        ))
        .unwrap();
        let names: Vec<&str> = cfg.checks.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["Unit.tests_2", "lint"]);
    }

    #[test]
    fn rejects_checks_as_plain_commands() {
        let e = err(&format!(
            "integration = \"main\"\nchecks = [\"make lint\", \"make test\"]\n{UP_A}"
        ));
        assert!(
            e.contains(
                "write `checks = [{ name = \"...\", run = \"make lint\" }, { name = \"...\", run = \"make test\" }]`"
            ),
            "{e}"
        );
        // A command with quotes is written as TOML writes it.
        let e = err(&format!(
            "integration = \"main\"\nchecks = ['test -f \"a b\"']\n{UP_A}"
        ));
        assert!(e.contains(r#"run = 'test -f "a b"' }"#), "{e}");
        // A plain command among tables is named by its place.
        let e = err(&format!(
            "integration = \"main\"\nchecks = [{{ name = \"lint\", run = \"make lint\" }}, \"make test\"]\n{UP_A}"
        ));
        assert!(
            e.contains("checks[1]: a check is a table with a name and the command it runs"),
            "{e}"
        );
        assert!(
            e.contains("write `{ name = \"...\", run = \"make test\" }`"),
            "{e}"
        );
        let e = err(&format!(
            "integration = \"main\"\nchecks = \"make\"\n{UP_A}"
        ));
        assert!(e.contains("checks: expected a list of tables"), "{e}");
    }

    #[test]
    fn rejects_invalid_checks_naming_them() {
        let config = |checks: &str| format!("integration = \"main\"\nchecks = [{checks}]\n{UP_A}");
        let e = err(&config(r#"{ name = "lint", cmd = "make lint" }"#));
        assert!(
            e.contains(
                "check 'lint': unknown key `cmd`; a check has exactly the keys `name` and `run`"
            ),
            "{e}"
        );
        assert!(e.contains("check 'lint': `run` is required"), "{e}");
        let e = err(&config(r#"{ run = "make lint" }"#));
        assert!(e.contains("checks[0]: `name` is required"), "{e}");
        let e = err(&config(r#"{ name = "lint", run = " " }"#));
        assert!(e.contains("check 'lint': the command is empty"), "{e}");
        let e = err(&config(r#"{ name = 1, run = "make" }"#));
        assert!(e.contains("checks[0]: `name` must be a string"), "{e}");
        let e = err(&config(r#"{ name = "lint", run = ["make"] }"#));
        assert!(e.contains("check 'lint': `run` must be a string"), "{e}");
        let e = err(&config(r#"{ name = "", run = "make" }"#));
        assert!(e.contains("checks[0]: the name is empty"), "{e}");
        let e = err(&config("42"));
        assert!(
            e.contains("checks[0]: expected a table with the keys `name` and `run`"),
            "{e}"
        );
        for name in ["a b", "-x", ".x", "läuft", "a/b"] {
            let e = err(&config(&format!("{{ name = {name:?}, run = \"make\" }}")));
            assert!(
                e.contains(&format!(
                    "check '{name}': '{name}' is not a valid check name"
                )),
                "{e}"
            );
        }
        for name in ["a", "A-1", "x_y.z", "install", "9"] {
            assert!(
                parse(&config(&format!("{{ name = {name:?}, run = \"make\" }}"))).is_ok(),
                "{name}"
            );
        }
    }

    #[test]
    fn rejects_check_names_that_differ_only_in_case() {
        let e = err(&format!(
            "integration = \"main\"\nchecks = [{{ name = \"test\", run = \"a\" }}, {{ name = \"lint\", run = \"b\" }}, \
             {{ name = \"Test\", run = \"c\" }}]\n{UP_A}"
        ));
        assert!(
            e.contains(
                "checks 'test' and 'Test' share a name; check names are unique ignoring letter case"
            ),
            "{e}"
        );
        assert!(!e.contains("'lint'"), "{e}");
        // A repeated name is reported together with the other problems of
        // the checks that carry it.
        let e = err(&format!(
            "integration = \"main\"\nchecks = [{{ name = \"test\", run = \" \" }}, {{ name = \"Test\", cmd = \"c\" }}]\n{UP_A}"
        ));
        assert!(e.contains("check 'test': the command is empty"), "{e}");
        assert!(e.contains("check 'Test': unknown key `cmd`"), "{e}");
        assert!(e.contains("checks 'test' and 'Test' share a name"), "{e}");
    }

    #[test]
    fn rejects_the_string_form_of_publish() {
        let e = err(&format!(
            "integration = \"main\"\npublish = \"origin\"\n{UP_A}"
        ));
        assert!(e.contains("publish = { remote = \"origin\" }"), "{e}");
        assert!(e.contains("placed after the top-level keys"), "{e}");
    }

    #[test]
    fn rejects_invalid_publish_tables() {
        let e = err(&format!(
            "integration = \"main\"\n{UP_A}[publish]\nremote = \"origin\"\nvia = \"pull-request\"\n"
        ));
        assert!(e.contains("unknown variant `pull-request`"), "{e}");
        let e = err(&format!(
            "integration = \"main\"\n{UP_A}[publish]\nremote = \"origin\"\nforce = true\n"
        ));
        assert!(e.contains("unknown field `force`"), "{e}");
        let e = err(&format!(
            "integration = \"main\"\n{UP_A}[publish]\nvia = \"push\"\n"
        ));
        assert!(e.contains("missing field `remote`"), "{e}");
        let e = err(&format!(
            "integration = \"main\"\n{UP_A}[publish]\nremote = \"bad name\"\n"
        ));
        assert!(
            e.contains("publish remote: 'bad name' is not a valid remote name"),
            "{e}"
        );
    }

    #[test]
    fn forge_and_api_settings_are_validated() {
        let cfg = parse(&format!(
            "integration = \"main\"\n{UP_A}[publish]\nremote = \"origin\"\nvia = \"merge-request\"\n\
             forge = \"gitlab\"\napi = \"https://gitlab.example.com/gitlab/api/v4/\"\n"
        ))
        .unwrap();
        let publish = cfg.publish.unwrap();
        assert_eq!(publish.forge, Some(ForgeKind::Gitlab));
        assert_eq!(
            publish.api.as_deref(),
            Some("https://gitlab.example.com/gitlab/api/v4")
        );
        let e = err(&format!(
            "integration = \"main\"\n{UP_A}[publish]\nremote = \"origin\"\nforge = \"gitea\"\n"
        ));
        assert!(
            e.contains(
                "publish forge: 'gitea' is not a forge the tool reaches; use `gitlab` or `github`"
            ),
            "{e}"
        );
        for (api, why) in [
            ("http://forge.example/api/v4", "is not an `https` URL"),
            ("forge.example/api/v4", "is not a URL"),
            (
                "https://forge.example/api/v4?x=1",
                "has a query or a fragment",
            ),
        ] {
            let e = err(&format!(
                "integration = \"main\"\n{UP_A}[publish]\nremote = \"origin\"\napi = \"{api}\"\n"
            ));
            assert!(e.contains(&format!("publish api: '{api}' {why}")), "{e}");
            assert!(e.contains("as an `https` URL"), "{e}");
        }
        // The refusal of credentials does not show them.
        let e = err(&format!(
            "integration = \"main\"\n{UP_A}[publish]\nremote = \"origin\"\n\
             api = \"https://user:secret@forge.example/api/v4\"\n"
        ));
        assert!(
            e.contains("publish api: 'https://forge.example/api/v4' carries credentials"),
            "{e}"
        );
        assert!(!e.contains("secret") && !e.contains("user"), "{e}");
    }

    #[test]
    fn the_queue_is_an_account_of_merge_request_mode() {
        let cfg = parse(&format!(
            "integration = \"main\"\n{UP_A}[publish]\nremote = \"origin\"\nvia = \"merge-request\"\n\
             queue = \"@merge-bot\"\n"
        ))
        .unwrap();
        assert_eq!(cfg.publish.unwrap().queue.as_deref(), Some("merge-bot"));
        let e = err(&format!(
            "integration = \"main\"\n{UP_A}[publish]\nremote = \"origin\"\nqueue = \"merge-bot\"\n"
        ));
        assert!(
            e.contains(
                "publish queue: a merge queue takes merge requests, and `via = \"push\"` opens none"
            ),
            "{e}"
        );
        let e = err(&format!(
            "integration = \"main\"\n{UP_A}[publish]\nremote = \"origin\"\nvia = \"merge-request\"\n\
             queue = \"merge bot\"\n"
        ));
        assert!(
            e.contains("publish queue: 'merge bot' is not an account name"),
            "{e}"
        );
    }

    #[test]
    fn replace_after_reads_weeks_days_hours_and_minutes() {
        for (text, hours) in [
            ("7d", 168),
            ("36h", 36),
            ("2w", 336),
            ("1d 12h", 36),
            (" 3 days ", 72),
            ("1w 1d", 192),
        ] {
            let after = ReplaceAfter::parse(text).unwrap();
            assert_eq!(after.text(), text.trim());
            assert_eq!(
                after.duration(),
                jiff::SignedDuration::from_hours(hours),
                "{text}"
            );
        }
        assert_eq!(
            ReplaceAfter::parse("90m").unwrap().duration(),
            jiff::SignedDuration::from_mins(90)
        );
        for text in [
            "0d", "-7d", "1 month", "1y", "soon", "", "7d ago", "30s", "1h 30s",
        ] {
            let e = ReplaceAfter::parse(text).unwrap_err();
            assert!(
                e.contains(&format!("publish replace_after: '{text}'")),
                "{e}"
            );
            assert!(e.contains("`7d`, `36h`, `2w` or `1d 12h`"), "{e}");
        }
    }

    #[test]
    fn replace_after_needs_merge_requests_and_a_release_upstream() {
        let tagged = "[[upstream]]\nname = \"a\"\nurl = \"https://example.com/a.git\"\nbranch = \"main\"\n\
                      path = \"apps/a\"\ntags = \"v*\"\n";
        let cfg = parse(&format!(
            "integration = \"main\"\n{tagged}{UP_B}[publish]\nremote = \"origin\"\nvia = \"merge-request\"\n\
             replace_after = \"7d\"\n"
        ))
        .unwrap();
        let after = cfg.publish.unwrap().replace_after.unwrap();
        assert_eq!(after.duration(), jiff::SignedDuration::from_hours(168));
        // The update's state keeps it as written.
        assert_eq!(serde_json::to_value(&after).unwrap(), "7d");
        assert_eq!(
            serde_json::from_value::<ReplaceAfter>("7d".into()).unwrap(),
            after
        );

        let e = err(&format!(
            "integration = \"main\"\n{tagged}[publish]\nremote = \"origin\"\nvia = \"push\"\n\
             replace_after = \"7d\"\n"
        ));
        assert!(e.contains("publish replace_after: '7d'"), "{e}");
        assert!(e.contains("needs merge-request mode"), "{e}");
        let e = err(&format!(
            "integration = \"main\"\n{UP_A}[publish]\nremote = \"origin\"\n\
             replace_after = \"7d\"\n"
        ));
        assert!(e.contains("needs merge-request mode"), "{e}");
        let e = err(&format!(
            "integration = \"main\"\n{UP_A}{UP_B}[publish]\nremote = \"origin\"\nvia = \"merge-request\"\n\
             replace_after = \"7d\"\n"
        ));
        assert!(e.contains("publish replace_after: '7d'"), "{e}");
        assert!(
            e.contains("only a newer release of a follow upstream with `tags`"),
            "{e}"
        );
        for bad in ["0d", "-7d", "1 month", "soon"] {
            let e = err(&format!(
                "integration = \"main\"\n{tagged}[publish]\nremote = \"origin\"\nvia = \"merge-request\"\n\
                 replace_after = \"{bad}\"\n"
            ));
            assert!(
                e.contains(&format!("publish replace_after: '{bad}'")),
                "{e}"
            );
            assert!(e.contains("`7d`, `36h`, `2w` or `1d 12h`"), "{e}");
        }
    }

    #[test]
    fn normalises_paths() {
        assert_eq!(normalise_path("/a/b/").unwrap(), "a/b");
        assert_eq!(normalise_path("a").unwrap(), "a");
        assert!(normalise_path("").is_err());
        assert!(normalise_path("/").is_err());
        assert!(normalise_path("a/../b").is_err());
        assert!(normalise_path("./a").is_err());
        assert!(normalise_path("a//b").is_err());
        assert!(normalise_path("vendor/.git").is_err());
        assert!(normalise_path(".GIT/x").is_err());
        assert!(normalise_path("a/.gitlab").is_ok());
    }

    #[test]
    fn rejects_invalid_branch_and_name() {
        let e = err(
            "integration = \"ma in\"\n[[upstream]]\nname = \"a b\"\nurl = \"u\"\nbranch = \"x..y\"\n",
        );
        assert!(
            e.contains("integration: 'ma in' is not a valid branch name"),
            "{e}"
        );
        assert!(e.contains("the name may contain only"), "{e}");
        assert!(e.contains("'x..y' is not a valid branch name"), "{e}");
    }

    #[test]
    fn names_that_cannot_name_a_reference_are_refused() {
        for name in ["a..b", "x.lock", "a."] {
            let e = err(&format!(
                "integration = \"main\"\n[[upstream]]\nname = \"{name}\"\nurl = \"u\"\nbranch = \"main\"\n"
            ));
            assert!(
                e.contains(&format!(
                    "upstream '{name}': '{name}' cannot name a git reference"
                )),
                "{e}"
            );
        }
        assert!(
            parse("integration = \"main\"\n[[upstream]]\nname = \"a.b_c-1\"\nurl = \"u\"\nbranch = \"main\"\n")
                .is_ok()
        );
    }

    #[test]
    fn env_names_are_upper_case_with_underscores() {
        assert_eq!(env_name("app-a"), "APP_A");
        assert_eq!(env_name("Web.v2_x"), "WEB_V2_X");
        assert_eq!(env_name("up"), "UP");
    }

    #[test]
    fn rejects_names_that_share_environment_variables() {
        let up = |name: &str, path: &str| {
            format!(
                "[[upstream]]\nname = \"{name}\"\nurl = \"https://example.com/{path}.git\"\nbranch = \"main\"\npath = \"{path}\"\n"
            )
        };
        let e = err(&format!(
            "integration = \"main\"\n{}{}{}{}",
            up("app-a", "a"),
            up("app_a", "b"),
            up("APP.A", "c"),
            up("other", "d")
        ));
        assert!(
            e.contains(
                "upstreams 'app-a', 'app_a' and 'APP.A' share the environment variables GIT_INTAKE_UPSTREAM_APP_A_*"
            ),
            "{e}"
        );
        assert!(!e.contains("OTHER"), "{e}");
        // A repeated name is reported once, as a shared name.
        let e = err(&format!(
            "integration = \"main\"\n{}{}",
            up("a", "a"),
            up("a", "b")
        ));
        assert!(e.contains("2 upstreams share the name 'a'"), "{e}");
        assert!(!e.contains("environment variables"), "{e}");
    }

    #[test]
    fn selects_in_configuration_order() {
        let cfg = parse(&format!("integration = \"main\"\n{UP_A}{UP_B}")).unwrap();
        let picked = cfg.select(&["b".into(), "a".into()]).unwrap();
        assert_eq!(
            picked.iter().map(|u| &u.name).collect::<Vec<_>>(),
            ["a", "b"]
        );
        assert_eq!(cfg.select(&[]).unwrap().len(), 2);
        let e = format!("{}", cfg.select(&["zz".into()]).unwrap_err());
        assert!(e.contains("unknown upstream 'zz'"), "{e}");
    }
}
