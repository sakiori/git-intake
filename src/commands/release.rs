//! `release`: cut, check, version and describe the releases of the
//! integration branch.
//!
//! The commands act on the publish remote, never on local branches: they
//! fetch its integration branch and tags into references of their own, so
//! that a CI clone without a local integration branch serves as well.

use std::collections::HashSet;

use anyhow::{Context, Result, bail};
use serde::Serialize;

use crate::Exit;
use crate::config::{self, Config, Publish};
use crate::forge::{self, ReleaseLookup};
use crate::git::{Git, abbrev};
use crate::identity;
use crate::integration_line;
use crate::output::{Render, emit};
use crate::redact::redacted;
use crate::release::{self, Base, Known, Release, ReleaseTag, Remote};
use crate::repo::Repo;
use crate::sync_record;
use crate::upstreams::Fetches;

/// The release state of the publish remote's integration tip, as
/// `release next` and `status` report it.
#[derive(Debug, Serialize)]
pub struct ReleaseState {
    pub tip: String,
    /// The release base of the tip.
    pub base: String,
    /// The tag of the upstream release the base comes from.
    pub upstream_release: String,
    /// The release at the tip, or `null`.
    pub release: Option<String>,
    /// Whether the release at the tip is published; `null` without one, or
    /// when the forge cannot be read.
    pub published: Option<bool>,
    /// The release a tag cut now would take, or `null` when the tip carries
    /// one.
    pub next: Option<String>,
    /// When the tip carries no release: the newest release on the
    /// downstream's line below it, when it is not published; else `null`.
    pub unpublished_previous: Option<String>,
    /// Why the forge could not be read, or `null`.
    pub forge_error: Option<String>,
    #[serde(skip)]
    pub upstream: String,
    #[serde(skip)]
    pub remote: String,
    #[serde(skip)]
    pub integration: String,
}

impl Render for ReleaseState {
    fn render(&self) -> String {
        let mut out = format!(
            "integration tip  {} ({}/{})\nrelease base     {} (upstream '{}' at {})\n",
            abbrev(&self.tip),
            self.remote,
            self.integration,
            self.base,
            self.upstream,
            self.upstream_release
        );
        if let Some(release) = &self.release {
            let state = match (self.published, &self.forge_error) {
                (Some(true), _) => "published".to_string(),
                (Some(false), _) => "not published".to_string(),
                (None, Some(why)) => format!("publication unknown: {why}"),
                (None, None) => "publication unknown".to_string(),
            };
            out.push_str(&format!("release at tip   {release}, {state}\n"));
        }
        if let Some(previous) = &self.unpublished_previous {
            out.push_str(&format!(
                "unpublished      {previous}, below the tip: {}\n",
                unpublished_advice()
            ));
        }
        if let Some(next) = &self.next {
            out.push_str(&format!("next release     {next}\n"));
        }
        if let (None, Some(why)) = (&self.release, &self.forge_error) {
            out.push_str(&format!("cannot read the forge: {why}\n"));
        }
        out
    }
}

/// The publish remote's integration tip and what it holds of releases.
struct AtTip {
    config: Config,
    remote: Remote,
    base: Base,
    /// The downstream's line ending at the tip, newest first.
    line: Vec<String>,
    releases: Vec<Release>,
    /// Every tag of the remote of release form, on the line or not.
    release_tags: Vec<ReleaseTag>,
    /// Whether the tip is a commit of the downstream's line.
    on_line: bool,
}

impl AtTip {
    /// The release at the tip, the highest when several point at it.
    fn release(&self) -> Option<&Release> {
        self.releases
            .iter()
            .filter(|r| r.commit == self.remote.tip)
            .max_by(|a, b| a.tag.version.cmp(&b.tag.version))
    }

    /// The release a tag cut now on the tip would take.
    fn next(&self) -> Result<ReleaseTag> {
        if !self.on_line {
            bail!(
                "the integration tip {} of '{}' carries no commit of the downstream's own: every upstream's last \
                 synced commit contains it, so there is nothing to release",
                abbrev(&self.remote.tip),
                self.remote.name
            );
        }
        let settings = self.config.release.as_ref().context("a release table")?;
        let day = release::utc_day(release::now());
        let next = release::next(
            &self.releases,
            &self.release_tags,
            &self.base.version,
            settings.label(),
            day,
        )
        .map_err(|why| anyhow::anyhow!("{why}"))?;
        if let Some(commit) = self.remote.tag(&next.name) {
            bail!(
                "the next release would be {}, but '{}' already has a tag of that name at {}, which is no release \
                 of the downstream's line; remove or rename that tag on the remote",
                next.name,
                self.remote.name,
                abbrev(commit)
            );
        }
        Ok(next)
    }

    /// The newest release on the line below the tip, when the tip carries
    /// none.
    fn previous(&self) -> Option<&Release> {
        if self.release().is_some() {
            return None;
        }
        release::previous(&self.line, &self.releases)
    }
}

/// The configuration committed at the commit checked out where the tool
/// was started.
fn config_at_head(repo: &Repo) -> Result<Config> {
    let head = repo
        .head()
        .context("no commit is checked out; run the command in a checkout of the repository")?;
    repo.config_at(head)?.with_context(|| {
        format!(
            "no {} is committed at the checked-out commit {}",
            config::FILE_NAME,
            abbrev(head)
        )
    })
}

fn publish_of(config: &Config) -> Result<&Publish> {
    config
        .publish
        .as_ref()
        .context("releases are cut on the publish remote; configure it in a [publish] table")
}

/// Fetches the publish remote that `start` names and reads its integration
/// tip, with the configuration committed there.
fn at_tip(repo: &Repo, fetches: &mut Fetches, start: &Config, known: &Known) -> Result<AtTip> {
    let git = &repo.git;
    let publish = publish_of(start)?;
    let remote = release::fetch_remote(fetches, &publish.remote, &start.integration)?;
    let config = repo.config_at(&remote.tip)?.with_context(|| {
        format!(
            "no {} is committed at the tip of '{}' on '{}'",
            config::FILE_NAME,
            start.integration,
            publish.remote
        )
    })?;
    if config.integration != start.integration {
        bail!(
            "the {} at the tip of '{}' on '{}' names '{}' as the integration branch",
            config::FILE_NAME,
            start.integration,
            publish.remote,
            config.integration
        );
    }
    let at = format!(
        "the tip of '{}' on '{}'",
        config.integration, publish.remote
    );
    let (settings, up) = release::settings(&config, &at)?;
    let synced = release::last_synced(git, fetches, &config, &remote.tip, known)?;
    let line = release::line(git, &remote.tip, &synced)?;
    let releases = release::releases_on(&line, &remote.tags, &settings.labels);
    let release_tags = release::release_tags(&remote.tags, &settings.labels);
    let tags = release::upstream_tags(git, up, known)?;
    let base = release::base_of(git, settings, up, &tags, &remote.tip)?;
    let on_line = line.first() == Some(&remote.tip);
    Ok(AtTip {
        config,
        remote,
        base,
        line,
        releases,
        release_tags,
        on_line,
    })
}

/// What an unpublished release means, and what completes it.
fn unpublished_advice() -> &'static str {
    "its pipeline is still running or failed, and a failed release is completed by running its pipeline's \
     failed jobs again"
}

/// What the forge says of a release.
enum Lookup {
    Known(bool),
    /// The project is hidden from the release commands.
    Hidden(String),
    /// The forge could not be read.
    Failed(String),
}

fn lookup(git: &Git, publish: &Publish, tag: &str) -> Lookup {
    let forge = match forge::connect(git, publish, forge::Access::Releases) {
        Ok(forge) => forge,
        Err(err) => return Lookup::Failed(format!("{err:#}")),
    };
    match forge.release(tag) {
        Ok(ReleaseLookup::Published) => Lookup::Known(true),
        Ok(ReleaseLookup::Unpublished) => Lookup::Known(false),
        Ok(ReleaseLookup::Hidden) => {
            let l = forge.location();
            let how = match &l.credential {
                Some(c) => format!("with the token in {}", c.variable),
                None => "without a token".to_string(),
            };
            Lookup::Hidden(format!(
                "the {} project '{}' at {} does not show its releases {how}, so whether a release is published \
                 cannot be read; set {}",
                forge.kind().name(),
                l.project,
                l.api,
                l.token_variables
            ))
        }
        Err(err) => Lookup::Failed(format!("{err:#}")),
    }
}

/// The release state of the publish remote's integration tip. A forge that
/// cannot be read leaves `published` unknown; a project hidden from the
/// release commands is refused only when `refuse_hidden`.
pub fn state(
    repo: &Repo,
    start: &Config,
    known: &Known,
    refuse_hidden: bool,
) -> Result<ReleaseState> {
    let mut fetches = Fetches::new(repo);
    let tip = at_tip(repo, &mut fetches, start, known)?;
    let publish = publish_of(&tip.config)?;
    let mut published = None;
    let mut forge_error = None;
    let mut next = None;
    let mut unpublished_previous = None;
    let release = tip.release().map(|r| r.tag.name.clone());
    match &release {
        Some(name) => match lookup(&repo.git, publish, name) {
            Lookup::Known(p) => published = Some(p),
            Lookup::Hidden(why) if refuse_hidden => bail!("{why}"),
            Lookup::Hidden(why) | Lookup::Failed(why) => forge_error = Some(why),
        },
        None => {
            next = Some(tip.next()?.name);
            if let Some(previous) = tip.previous() {
                match lookup(&repo.git, publish, &previous.tag.name) {
                    Lookup::Known(false) => unpublished_previous = Some(previous.tag.name.clone()),
                    Lookup::Known(true) => {}
                    Lookup::Hidden(why) if refuse_hidden => bail!("{why}"),
                    Lookup::Hidden(why) | Lookup::Failed(why) => forge_error = Some(why),
                }
            }
        }
    }
    Ok(ReleaseState {
        tip: tip.remote.tip.clone(),
        base: tip.base.version.to_string(),
        upstream_release: tip.base.tag.clone(),
        release,
        published,
        next,
        unpublished_previous,
        forge_error,
        upstream: tip.base.upstream.clone(),
        remote: tip.remote.name.clone(),
        integration: tip.config.integration.clone(),
    })
}

/// `release next`.
pub fn next(repo: &Repo, json: bool) -> Result<Exit> {
    let start = config_at_head(repo)?;
    let state = state(repo, &start, &Known::default(), true)?;
    emit(&state, json)?;
    Ok(Exit::Success)
}

/// `release tag`.
pub fn tag(repo: &Repo) -> Result<Exit> {
    let git = &repo.git;
    let start = config_at_head(repo)?;
    let mut fetches = Fetches::new(repo);
    let tip = at_tip(repo, &mut fetches, &start, &Known::default())?;
    let publish = publish_of(&tip.config)?;
    let remote = &tip.remote;
    crate::forge::refuse_job_token_push(git, &publish.remote)?;
    if let Some(at) = tip.release() {
        return match lookup(git, publish, &at.tag.name) {
            Lookup::Known(true) => {
                println!(
                    "the integration tip {} carries the published release {}; nothing to release",
                    abbrev(&remote.tip),
                    at.tag.name
                );
                Ok(Exit::Success)
            }
            Lookup::Known(false) => {
                println!(
                    "the integration tip {} carries the release {}, which is not published: {}; no new tag \
                     was created",
                    abbrev(&remote.tip),
                    at.tag.name,
                    unpublished_advice()
                );
                Ok(Exit::Negative)
            }
            Lookup::Hidden(why) | Lookup::Failed(why) => bail!(
                "the integration tip {} carries the release {}, and whether it is published cannot be read: \
                 {why}\nNo tag was created",
                abbrev(&remote.tip),
                at.tag.name
            ),
        };
    }
    let next = tip.next()?;
    if let Some(previous) = tip.previous() {
        match lookup(git, publish, &previous.tag.name) {
            Lookup::Known(true) => {}
            Lookup::Known(false) => eprintln!(
                "warning: the release {} below the tip is not published: {}",
                previous.tag.name,
                unpublished_advice()
            ),
            Lookup::Hidden(why) | Lookup::Failed(why) => eprintln!(
                "{}",
                redacted(&format!(
                    "warning: whether the release {} below the tip is published cannot be read: {why}",
                    previous.tag.name
                ))
            ),
        }
    }
    identity::Upstreams::new(git, &tip.config.upstreams)?.refuse_push(git, &publish.remote)?;
    let object = tag_object(git, &remote.tip, &next)?;
    let full = format!("refs/tags/{}", next.name);
    let out = git.raw(&[
        "push",
        "--quiet",
        // Exactly the one tag, whatever push.followTags or
        // push.recurseSubmodules say.
        "--no-follow-tags",
        "--no-recurse-submodules",
        &format!("--force-with-lease={full}:"),
        "--end-of-options",
        &publish.remote,
        &format!("{object}:{full}"),
    ])?;
    if !out.success() {
        let taken = crate::commands::merge_request::remote_refs(
            git,
            &publish.remote,
            std::slice::from_ref(&full),
        )
        .map(|r| r.contains_key(&full))
        .unwrap_or(false);
        if taken {
            bail!(
                "another run created the tag {} on '{}' first; nothing was pushed. Run `git intake release tag` \
                 again, which takes the release after it",
                next.name,
                publish.remote
            );
        }
        bail!(
            "pushing the tag {} to '{}' was rejected: {}\nNothing was pushed",
            next.name,
            publish.remote,
            out.into_error()
        );
    }
    let local = git.raw(&["update-ref", &full, &object, ""])?;
    println!(
        "pushed the release tag {} at {} to '{}'",
        next.name,
        abbrev(&remote.tip),
        publish.remote
    );
    if !local.success() {
        eprintln!(
            "note: the repository already has a tag {}, which was left as it is",
            next.name
        );
    }
    Ok(Exit::Success)
}

/// Writes the annotated tag object of `tag` on `commit` with git's
/// configured identity, without a reference to it.
fn tag_object(git: &Git, commit: &str, tag: &ReleaseTag) -> Result<String> {
    let tagger = git
        .line(&["var", "GIT_COMMITTER_IDENT"])
        .context("the release tag needs git's identity: set user.name and user.email")?;
    let text = format!(
        "object {commit}\ntype commit\ntag {}\ntagger {tagger}\n\nRelease {}\n",
        tag.name,
        tag.version_text()
    );
    Ok(git.run_with_input(&["mktag"], text)?.trim().to_string())
}

/// The tag a pipeline command works on: the one named, or else
/// `CI_COMMIT_TAG`.
fn named_tag(given: Option<&str>) -> Option<String> {
    given
        .map(str::to_string)
        .or_else(|| std::env::var("CI_COMMIT_TAG").ok())
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
}

/// The result of `release check`.
#[derive(Serialize)]
struct CheckReport {
    tag: String,
    problems: Vec<String>,
}

/// `release check`.
pub fn check(repo: &Repo, given: Option<&str>) -> Result<Exit> {
    let git = &repo.git;
    let name = named_tag(given).context(
        "name the release tag to check, or run in a tag pipeline that sets CI_COMMIT_TAG",
    )?;
    let config = config_at_head(repo)?;
    let head = repo.head().context("a checked-out commit")?.to_string();
    let (settings, up) = release::settings(&config, "the checked-out commit")?;
    let publish = publish_of(&config)?;
    let mut fetches = Fetches::new(repo);
    let remote = release::fetch_remote(&mut fetches, &publish.remote, &config.integration)?;
    let known = Known::default();
    let synced = release::last_synced(git, &mut fetches, &config, &remote.tip, &known)?;
    let line = release::line(git, &remote.tip, &synced)?;
    let releases = release::releases_on(&line, &remote.tags, &settings.labels);

    let mut problems = Vec::new();
    let tag = ReleaseTag::parse(&name, std::slice::from_ref(&settings.labels[0]));
    if tag.is_none() {
        problems.push(format!(
            "'{name}' is not a release tag v<x.y.z>-{}.<YYYYMMDD>.<n> with the current label '{}'",
            settings.label(),
            settings.label()
        ));
    }
    match remote.tag(&name) {
        None => problems.push(format!(
            "the publish remote '{}' has no tag '{name}'",
            publish.remote
        )),
        Some(commit) if commit != head => problems.push(format!(
            "'{name}' points at {}, not at the checked-out commit {}",
            abbrev(commit),
            abbrev(&head)
        )),
        Some(_) => {}
    }
    if !line.contains(&head) {
        problems.push(format!(
            "the checked-out commit {} is not on the downstream's line of '{}' on '{}'; releases are cut \
             only from the integration branch's own first-parent commits",
            abbrev(&head),
            config.integration,
            publish.remote
        ));
    }
    if let Some(tag) = &tag {
        let tags = release::upstream_tags(git, up, &known)?;
        match release::base_of(git, settings, up, &tags, &head) {
            Ok(base) if base.version != tag.base => problems.push(format!(
                "'{name}' carries the base {}, but the release base of {} is {} (upstream '{}' at {})",
                tag.base,
                abbrev(&head),
                base.version,
                base.upstream,
                base.tag
            )),
            Ok(_) => {}
            Err(err) => problems.push(format!("{err:#}")),
        }
        if let Some(other) = release::not_after(tag, &releases) {
            problems.push(release::order_problem(tag, other));
        }
    }
    if problems.is_empty() {
        match lookup(git, publish, &name) {
            Lookup::Known(false) => {}
            Lookup::Known(true) => problems.push(format!(
                "{name} is published already, and a published release is never written again"
            )),
            Lookup::Hidden(why) | Lookup::Failed(why) => {
                bail!("whether {name} is published cannot be read: {why}")
            }
        }
    }
    let report = CheckReport {
        tag: name,
        problems,
    };
    emit(&report, false)?;
    Ok(if report.problems.is_empty() {
        Exit::Success
    } else {
        Exit::Negative
    })
}

impl Render for CheckReport {
    fn render(&self) -> String {
        if self.problems.is_empty() {
            return format!("{} may be released\n", self.tag);
        }
        let mut out = format!("{} may not be released:\n", self.tag);
        for p in &self.problems {
            out.push_str(&format!("  - {p}\n"));
        }
        out
    }
}

/// The result of `release version`.
#[derive(Serialize)]
struct VersionReport {
    version: String,
    /// The release tag, or `null` for a rehearsal.
    tag: Option<String>,
    rehearsal: bool,
}

impl Render for VersionReport {
    fn render(&self) -> String {
        format!("{}\n", self.version)
    }
}

/// `release version`.
pub fn version(repo: &Repo, given: Option<&str>, json: bool, dotenv: bool) -> Result<Exit> {
    let git = &repo.git;
    let config = config_at_head(repo)?;
    let head = repo.head().context("a checked-out commit")?.to_string();
    let (settings, up) = release::settings(&config, "the checked-out commit")?;
    let tags = release::upstream_tags(git, up, &Known::default())?;
    let report = match named_tag(given) {
        Some(name) => {
            let Some(tag) = ReleaseTag::parse(&name, std::slice::from_ref(&settings.labels[0]))
            else {
                bail!(
                    "'{name}' is not a release tag v<x.y.z>-{}.<YYYYMMDD>.<n> with the current label",
                    settings.label()
                );
            };
            let commit = git
                .commit(&format!("refs/tags/{name}"))?
                .with_context(|| format!("the repository has no tag '{name}'"))?;
            let base = release::base_of(git, settings, up, &tags, &commit)?;
            if base.version != tag.base {
                bail!(
                    "'{name}' carries the base {}, but the release base of the tagged commit {} is {} \
                     (upstream '{}' at {})",
                    tag.base,
                    abbrev(&commit),
                    base.version,
                    base.upstream,
                    base.tag
                );
            }
            VersionReport {
                version: tag.version_text(),
                tag: Some(name),
                rehearsal: false,
            }
        }
        None => {
            let base = release::base_of(git, settings, up, &tags, &head)?;
            let at = match std::env::var("CI_PIPELINE_CREATED_AT")
                .ok()
                .filter(|v| !v.trim().is_empty())
            {
                Some(text) => text.trim().parse().with_context(|| {
                    format!("CI_PIPELINE_CREATED_AT '{text}' is not an RFC 3339 time")
                })?,
                None => release::now(),
            };
            let mut version = base.version.clone();
            version.pre = semver::Prerelease::new(&format!(
                "{}.{}.0",
                settings.label(),
                release::utc_day(at)
            ))?;
            VersionReport {
                version: version.to_string(),
                tag: None,
                rehearsal: true,
            }
        }
    };
    if dotenv {
        let mut out = format!("RELEASE_VERSION={}\n", report.version);
        if let Some(tag) = &report.tag {
            out.push_str(&format!("RELEASE_TAG={tag}\n"));
        }
        print!("{}", redacted(&out));
    } else {
        emit(&report, json)?;
    }
    Ok(Exit::Success)
}

/// `release notes`.
pub fn notes(repo: &Repo, given: Option<&str>) -> Result<Exit> {
    let git = &repo.git;
    let name = named_tag(given).context(
        "name the release tag to write the notes of, or run in a tag pipeline that sets CI_COMMIT_TAG",
    )?;
    let config = config_at_head(repo)?;
    let (settings, _) = release::settings(&config, "the checked-out commit")?;
    let publish = publish_of(&config)?;
    let tag = ReleaseTag::parse(&name, &settings.labels).with_context(|| {
        format!("'{name}' is not a release tag v<x.y.z>-<label>.<YYYYMMDD>.<n>")
    })?;
    let mut fetches = Fetches::new(repo);
    let remote = release::fetch_remote(&mut fetches, &publish.remote, &config.integration)?;
    let commit = match git.commit(&format!("refs/tags/{name}"))? {
        Some(c) => c,
        None => remote.tag(&name).map(str::to_string).with_context(|| {
            format!(
                "neither the repository nor '{}' has the tag '{name}'",
                publish.remote
            )
        })?,
    };
    let synced = release::last_synced(git, &mut fetches, &config, &commit, &Known::default())?;
    let line = release::line(git, &commit, &synced)?;
    let releases = release::releases_on(&line, &remote.tags, &settings.labels);
    let previous = previous_release(&line, &releases, &tag, &commit);
    print!(
        "{}",
        redacted(&write_notes(git, &commit, previous, &synced)?)
    );
    Ok(Exit::Success)
}

/// The newest release before `tag` on `line`, which ends at `commit`: a
/// release of a lower version at the same commit, or else the highest
/// release of the nearest earlier commit that has one.
fn previous_release<'a>(
    line: &[String],
    releases: &'a [Release],
    tag: &ReleaseTag,
    commit: &str,
) -> Option<&'a Release> {
    let highest = |at: &str, below: Option<&semver::Version>| {
        releases
            .iter()
            .filter(|r| r.commit == at && r.tag.name != tag.name)
            .filter(|r| below.is_none_or(|v| r.tag.version < *v))
            .max_by(|a, b| a.tag.version.cmp(&b.tag.version))
    };
    highest(commit, Some(&tag.version)).or_else(|| {
        line.iter()
            .skip_while(|c| *c != commit)
            .skip(1)
            .find_map(|c| highest(c, None))
    })
}

/// The Markdown notes of the release at `commit` after `previous`.
fn write_notes(
    git: &Git,
    commit: &str,
    previous: Option<&Release>,
    synced: &[String],
) -> Result<String> {
    let Some(previous) = previous else {
        return Ok("This is the first release.\n".to_string());
    };
    let mut exclude = synced.to_vec();
    exclude.push(previous.commit.clone());
    let walked = integration_line::walk(git, commit, &exclude)?;
    let records: std::collections::HashMap<&str, &sync_record::Record> = walked
        .records
        .iter()
        .map(|r| (r.commit.as_str(), r))
        .collect();
    let updates: HashSet<&str> = walked
        .commits
        .iter()
        .filter(|c| records.contains_key(c.commit.as_str()))
        .filter_map(|c| c.merged_by.as_deref())
        .collect();
    let catch_ups = sync_record::catch_ups(git, walked.commits.iter().map(|c| c.commit.as_str()))?;
    let mut listed: Vec<&str> = Vec::new();
    for c in &walked.commits {
        let own = match &c.merged_by {
            None => !updates.contains(c.commit.as_str()),
            Some(by) => catch_ups.contains(by),
        };
        if records.contains_key(c.commit.as_str()) || (own && !catch_ups.contains(&c.commit)) {
            listed.push(&c.commit);
        }
    }
    let subjects = subjects(git, &listed)?;
    let mut entries = Vec::new();
    for c in listed {
        match records.get(c) {
            Some(r) => entries.push(format!(
                "- Update {} to {}",
                r.upstream,
                r.tag
                    .clone()
                    .unwrap_or_else(|| format!("`{}`", abbrev(&r.target)))
            )),
            None => entries.push(format!(
                "- {}",
                subjects.get(c).map(String::as_str).unwrap_or_default()
            )),
        }
    }
    if entries.is_empty() {
        return Ok(format!(
            "No changes since the previous release, {}.\n",
            previous.tag.name
        ));
    }
    Ok(format!(
        "Changes since {}:\n\n{}\n",
        previous.tag.name,
        entries.join("\n")
    ))
}

/// The subject of each of `commits`.
fn subjects(git: &Git, commits: &[&str]) -> Result<std::collections::HashMap<String, String>> {
    if commits.is_empty() {
        return Ok(Default::default());
    }
    let input: String = commits.iter().map(|c| format!("{c}\n")).collect();
    let out = git.run_with_input(
        &[
            "log",
            "--no-walk=unsorted",
            "--stdin",
            "--format=%H%x1f%s",
            "--",
        ],
        input,
    )?;
    Ok(out
        .lines()
        .filter_map(|l| {
            let (c, s) = l.split_once('\u{1f}')?;
            Some((c.to_string(), s.to_string()))
        })
        .collect())
}
