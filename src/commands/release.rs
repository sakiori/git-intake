//! `release`: cut, check, version and describe the releases of the
//! integration branch.
//!
//! The commands act on the publish remote, never on local branches: they
//! fetch its integration branch and tags into references of their own, as
//! every command reads the integration branch.

use std::collections::HashSet;

use anyhow::{Context, Result, bail};
use serde::Serialize;

use crate::Exit;
use crate::bump::Bump;
use crate::config::{self, Config, Publish, Versioning};
use crate::forge::{self, Forge, NewTag, ReleaseLookup, TagAccess, TagCreation};
use crate::git::{Git, abbrev};
use crate::integration_line;
use crate::output::{Render, emit};
use crate::output::{esay, say};
use crate::redact::redacted;
use crate::release::{self, Known, Release, ReleaseTag, Remote, SemverNext, UpstreamRelease};
use crate::repo::{Bootstrap, Repo};
use crate::sync_record;
use crate::tags::UpstreamTags;
use crate::upstreams::Fetches;
use crate::upstreams::Recorded;

/// The release state of the publish remote's integration tip, as
/// `release next` and `status` report it.
#[derive(Debug, Serialize)]
pub struct ReleaseState {
    pub tip: String,
    pub versioning: Versioning,
    /// The release base of the tip under `dated`, else `null`.
    pub base: Option<String>,
    /// The tag of the newest upstream release the tip contains.
    pub upstream_release: String,
    /// The release at the tip, or `null`.
    pub release: Option<String>,
    /// Whether the release at the tip is published; `null` without one, or
    /// when the forge cannot be read.
    pub published: Option<bool>,
    /// Under `semver`, the change that sets the next release; `null` when
    /// none is due or the tip carries a release.
    pub bump: Option<Bump>,
    /// The release a tag cut now would take, or `null` when the tip carries
    /// one or none is due.
    pub next: Option<String>,
    /// When the tip carries no release: the newest release on the
    /// downstream's line below it, when it is not published; else `null`.
    pub unpublished_previous: Option<String>,
    /// Why the forge could not be read, or `null`.
    pub forge_error: Option<String>,
    #[serde(skip)]
    pub upstream: String,
    /// The newest release below the tip, for the text of a release not due.
    #[serde(skip)]
    pub previous: Option<String>,
    #[serde(skip)]
    pub remote: String,
    #[serde(skip)]
    pub integration: String,
}

impl Render for ReleaseState {
    fn render(&self) -> String {
        let mut out = format!(
            "integration tip  {} ({}/{})\n",
            abbrev(&self.tip),
            self.remote,
            self.integration
        );
        match &self.base {
            Some(base) => out.push_str(&format!(
                "release base     {base} (upstream '{}' at {})\n",
                self.upstream, self.upstream_release
            )),
            None => out.push_str(&format!(
                "upstream release {} (upstream '{}')\n",
                self.upstream_release, self.upstream
            )),
        }
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
        match (&self.next, self.bump) {
            (Some(next), Some(bump)) => out.push_str(&format!(
                "next release     {next}, a {} change\n",
                bump.as_str()
            )),
            (Some(next), None) => out.push_str(&format!("next release     {next}\n")),
            (None, _) if self.release.is_none() => out.push_str(&format!(
                "next release     none: {}\n",
                nothing_due(self.previous.as_deref())
            )),
            (None, _) => {}
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
    upstream: UpstreamRelease,
    tags: UpstreamTags,
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

    /// The release a tag cut now on the tip would take, with its change
    /// under `semver`; no tag when none is due.
    fn next(&self, git: &Git) -> Result<(Option<ReleaseTag>, Option<Bump>)> {
        if !self.on_line {
            bail!(
                "the integration tip {} of '{}' carries no commit of the downstream's own: every upstream's last \
                 synced commit contains it, so there is nothing to release",
                abbrev(&self.remote.tip),
                self.remote.name
            );
        }
        let (settings, up) = release::settings(&self.config, "the integration tip")?;
        let (next, bump) = match settings.versioning {
            Versioning::Dated => {
                let day = release::utc_day(release::now());
                let next = release::next(
                    &self.releases,
                    &self.release_tags,
                    &self.upstream.base(),
                    settings.label(),
                    day,
                )
                .map_err(|why| anyhow::anyhow!("{why}"))?;
                (next, None)
            }
            Versioning::Semver => {
                let source = release::Source {
                    settings,
                    up,
                    tags: &self.tags,
                };
                let planned = release::semver_next(
                    git,
                    &source,
                    &self.remote.tip,
                    &self.line[1..],
                    &self.releases,
                    &self.upstream,
                )?;
                let Some(version) = planned.version else {
                    return Ok((None, None));
                };
                let next = ReleaseTag::semver(version);
                if let Some(release) = release::not_after(&next, &self.releases) {
                    bail!(
                        "the next release would be {}, which breaks the order of releases: {}",
                        next.name,
                        release::order_problem(&next, release)
                    );
                }
                (next, planned.bump)
            }
        };
        if let Some(commit) = self.remote.tag(&next.name) {
            bail!(
                "the next release would be {}, but '{}' already has a tag of that name at {}, which is no release \
                 of the downstream's line; remove or rename that tag on the remote",
                next.name,
                self.remote.name,
                abbrev(commit)
            );
        }
        Ok((Some(next), bump))
    }

    /// Under `semver`, the tag the smallest next release would take, as a
    /// rehearsal names it when none is due.
    fn smallest_next(&self, git: &Git) -> Result<ReleaseTag> {
        let (settings, up) = release::settings(&self.config, "the integration tip")?;
        let source = release::Source {
            settings,
            up,
            tags: &self.tags,
        };
        let planned = release::semver_next(
            git,
            &source,
            &self.remote.tip,
            &self.line[1..],
            &self.releases,
            &self.upstream,
        )?;
        Ok(ReleaseTag::semver(
            planned.version.unwrap_or(planned.smallest),
        ))
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

/// Fetches the integration tip, where `from` says, with the publish
/// remote's tags, and reads the configuration committed there, refusing as
/// every command does one that names another integration branch or publish
/// remote.
fn at_tip(repo: &Repo, fetches: &mut Fetches, from: &Bootstrap, known: &Known) -> Result<AtTip> {
    let git = &repo.git;
    repo.require_fetchable(from)?;
    let remote = release::fetch_remote(fetches, &from.remote, &from.integration)?;
    let config = repo.config_at_tip(from, &remote.tip)?;
    let at = format!(
        "the tip of '{}' on '{}'",
        config.integration, config.publish.remote
    );
    let (settings, up) = release::settings(&config, &at)?;
    let synced = release::last_synced(git, fetches, &config, &remote.tip, known)?;
    let line = release::line(git, &remote.tip, &release::synced_commits(&synced))?;
    let releases = release::releases_on(&line, &remote.tags, settings);
    let release_tags = release::release_tags(&remote.tags, settings);
    let tags = release::upstream_tags(git, up, known)?;
    let upstream = release::contained(git, settings, up, &tags, &remote.tip)?;
    let on_line = line.first() == Some(&remote.tip);
    Ok(AtTip {
        config,
        remote,
        upstream,
        tags,
        line,
        releases,
        release_tags,
        on_line,
    })
}

/// Why a tip that carries no release takes none.
fn nothing_due(previous: Option<&str>) -> String {
    match previous {
        Some(p) => format!("the commits since the release {p} ask for no release"),
        None => "the commits of the downstream's line ask for no release".to_string(),
    }
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
    match forge::connect(git, publish, forge::Access::Releases) {
        Ok(forge) => lookup_on(forge.as_ref(), tag),
        Err(err) => Lookup::Failed(format!("{err:#}")),
    }
}

fn lookup_on(forge: &dyn Forge, tag: &str) -> Lookup {
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
    from: &Bootstrap,
    known: &Known,
    refuse_hidden: bool,
) -> Result<ReleaseState> {
    let mut fetches = Fetches::new(repo);
    let tip = at_tip(repo, &mut fetches, from, known)?;
    let publish = &tip.config.publish;
    let mut published = None;
    let mut forge_error = None;
    let mut next = None;
    let mut bump = None;
    let mut unpublished_previous = None;
    let release = tip.release().map(|r| r.tag.name.clone());
    match &release {
        Some(name) => match lookup(&repo.git, publish, name) {
            Lookup::Known(p) => published = Some(p),
            Lookup::Hidden(why) if refuse_hidden => bail!("{why}"),
            Lookup::Hidden(why) | Lookup::Failed(why) => forge_error = Some(why),
        },
        None => {
            let (tag, change) = tip.next(&repo.git)?;
            next = tag.map(|t| t.name);
            bump = change;
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
    let versioning = release::settings(&tip.config, "the integration tip")?
        .0
        .versioning;
    Ok(ReleaseState {
        tip: tip.remote.tip.clone(),
        versioning,
        base: (versioning == Versioning::Dated).then(|| tip.upstream.base().to_string()),
        upstream_release: tip.upstream.tag.clone(),
        release,
        published,
        bump,
        next,
        unpublished_previous,
        forge_error,
        upstream: tip.upstream.upstream.clone(),
        previous: tip.previous().map(|p| p.tag.name.clone()),
        remote: tip.remote.name.clone(),
        integration: tip.config.integration.clone(),
    })
}

/// `release next`.
pub fn next(repo: &Repo, json: bool) -> Result<Exit> {
    let state = state(repo, &repo.bootstrap()?, &Known::default(), true)?;
    emit(&state, json)?;
    Ok(Exit::Success)
}

/// `release tag`.
pub fn tag(repo: &Repo, dry_run: bool) -> Result<Exit> {
    let git = &repo.git;
    let mut fetches = Fetches::new(repo);
    let tip = at_tip(repo, &mut fetches, &repo.bootstrap()?, &Known::default())?;
    let publish = &tip.config.publish;
    let remote = &tip.remote;
    let forge = forge::connect(git, publish, forge::Access::Tags)?;
    let forge = forge.as_ref();
    if dry_run {
        return check_tag_access(git, forge, &tip);
    }
    if let Some(at) = tip.release() {
        return match lookup_on(forge, &at.tag.name) {
            Lookup::Known(true) => {
                say!(
                    "the integration tip {} carries the published release {}; nothing to release",
                    abbrev(&remote.tip),
                    at.tag.name
                );
                Ok(Exit::Success)
            }
            Lookup::Known(false) => {
                say!(
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
    let (next, _) = tip.next(git)?;
    if let Some(previous) = tip.previous() {
        match lookup_on(forge, &previous.tag.name) {
            Lookup::Known(true) => {}
            Lookup::Known(false) => esay!(
                "warning: the release {} below the tip is not published: {}",
                previous.tag.name,
                unpublished_advice()
            ),
            Lookup::Hidden(why) | Lookup::Failed(why) => esay!(
                "{}",
                redacted(&format!(
                    "warning: whether the release {} below the tip is published cannot be read: {why}",
                    previous.tag.name
                ))
            ),
        }
    }
    let Some(next) = next else {
        say!(
            "the integration tip {} carries no release, and {}; nothing to release",
            abbrev(&remote.tip),
            nothing_due(tip.previous().map(|p| p.tag.name.as_str()))
        );
        return Ok(Exit::Success);
    };
    let recorded = (next.dated.is_none()).then_some(&tip.upstream);
    let message = tag_message(&next, recorded);
    let identity = match forge.kind() {
        forge::Kind::GitHub => Some(tagger(git)?),
        forge::Kind::GitLab => None,
    };
    let new = NewTag {
        name: &next.name,
        commit: &remote.tip,
        message: &message,
        tagger: identity.as_ref().map(|(n, e)| (n.as_str(), e.as_str())),
    };
    let created = forge.create_tag(&new).with_context(|| {
        format!(
            "creating the release tag {} on the {} project '{}' failed; nothing was created",
            next.name,
            forge.kind().name(),
            forge.location().project
        )
    })?;
    if created == TagCreation::Taken {
        bail!(
            "another run created the tag {} on '{}' first; nothing was created. Run `git intake release tag` \
             again, which takes the release after it",
            next.name,
            publish.remote
        );
    }
    say!(
        "created the release tag {} at {} on the {} project '{}'",
        next.name,
        abbrev(&remote.tip),
        forge.kind().name(),
        forge.location().project
    );
    Ok(Exit::Success)
}

/// `release tag --dry-run`: whether the token may create the tag the next
/// release takes; while the tip carries a release, or under `semver` none
/// is due, a tag of the same form.
fn check_tag_access(git: &Git, forge: &dyn Forge, tip: &AtTip) -> Result<Exit> {
    let name = match tip.release() {
        Some(at) => at.tag.name.clone(),
        None => match tip.next(git)? {
            (Some(next), _) => next.name,
            (None, _) => tip.smallest_next(git)?.name,
        },
    };
    let l = forge.location();
    let place = format!("the {} project '{}'", forge.kind().name(), l.project);
    match forge.tag_access(&name).with_context(|| {
        format!(
            "cannot tell whether the token in {} may create the release tag {name} on {place}",
            l.token_variable()
        )
    })? {
        TagAccess::Allowed { account, unproven } => {
            let of = account.map(|a| format!(", of {a},")).unwrap_or_default();
            let unproven = unproven
                .map(|u| format!(" as far as {} tells: {u}", forge.kind().name()))
                .unwrap_or_default();
            say!(
                "the token in {}{of} may create the release tag {name} on {place}{unproven}; nothing was created",
                l.token_variable()
            );
            Ok(Exit::Success)
        }
        TagAccess::Denied(why) => {
            say!(
                "{}",
                redacted(&format!(
                    "the token in {} may not create the release tag {name} on {place}: {why}",
                    l.token_variable()
                ))
            );
            Ok(Exit::Negative)
        }
    }
}

/// The annotation of `tag`: `Release <version>`, and for a semver tag
/// `upstream` in trailers below it.
fn tag_message(tag: &ReleaseTag, upstream: Option<&UpstreamRelease>) -> String {
    let mut text = format!("Release {}\n", tag.version_text());
    if let Some(upstream) = upstream {
        text.push('\n');
        text.push_str(&upstream.trailers());
    }
    text
}

/// git's configured identity: name and email.
fn tagger(git: &Git) -> Result<(String, String)> {
    let name = git
        .line(&["var", "GIT_COMMITTER_IDENT"])
        .context("the release tag on GitHub needs git's identity: set user.name and user.email")?;
    // `Name <email> <time> <zone>`.
    let (name, rest) = name
        .split_once(" <")
        .context("git's identity has no email")?;
    let email = rest.split_once('>').map_or(rest, |(e, _)| e);
    Ok((name.to_string(), email.to_string()))
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
    let publish = &config.publish;
    let mut fetches = Fetches::new(repo);
    let remote = release::fetch_remote(&mut fetches, &publish.remote, &config.integration)?;
    let known = Known::default();
    let synced = release::last_synced(git, &mut fetches, &config, &remote.tip, &known)?;
    let line = release::line(git, &remote.tip, &release::synced_commits(&synced))?;
    let releases = release::releases_on(&line, &remote.tags, settings);

    let mut problems = Vec::new();
    let tag = new_release_tag(&name, settings);
    if tag.is_none() {
        problems.push(not_a_release_tag(&name, settings));
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
        let source = release::Source {
            settings,
            up,
            tags: &tags,
        };
        let recorded = remote
            .entry(&name)
            .and_then(|t| t.recorded.as_ref())
            .filter(|_| tag.dated.is_none());
        match tag_upstream(git, &source, &name, recorded, &head, true)? {
            Err(why) => problems.push(why),
            Ok(upstream) => match settings.versioning {
                Versioning::Dated => {
                    if let Some(problem) = base_problem(tag, abbrev(&head), &upstream) {
                        problems.push(problem);
                    }
                }
                Versioning::Semver => {
                    if let Some(at) = line.iter().position(|c| *c == head) {
                        let planned = release::semver_next(
                            git,
                            &source,
                            &head,
                            &line[at + 1..],
                            &releases,
                            &upstream,
                        )?;
                        if let Some(problem) = version_problem(tag, abbrev(&head), &planned) {
                            problems.push(problem);
                        }
                    }
                }
            },
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

/// A tag of the form new releases take: under `dated` with the current
/// label, under `semver` `v<x.y.z>`.
fn new_release_tag(name: &str, settings: &config::Release) -> Option<ReleaseTag> {
    match settings.versioning {
        Versioning::Dated => {
            ReleaseTag::parse_dated(name, std::slice::from_ref(&settings.labels[0]))
        }
        Versioning::Semver => ReleaseTag::parse_semver(name),
    }
}

fn not_a_release_tag(name: &str, settings: &config::Release) -> String {
    match settings.versioning {
        Versioning::Dated => format!(
            "'{name}' is not a release tag v<x.y.z>-{}.<YYYYMMDD>.<n> with the current label '{}'",
            settings.label(),
            settings.label()
        ),
        Versioning::Semver => format!("'{name}' is not a release tag v<x.y.z>"),
    }
}

/// The upstream release of the release tag `name` on `commit`: the one its
/// annotation records, which `commit` must contain, or else the newest one
/// `commit` contains.
fn tag_upstream(
    git: &Git,
    source: &release::Source,
    name: &str,
    recorded: Option<&Recorded>,
    commit: &str,
    listed: bool,
) -> Result<std::result::Result<UpstreamRelease, String>> {
    Ok(match recorded {
        Some(r) => release::recorded_release(git, source, r, commit, listed)?.map_err(|why| {
            format!("'{name}' records an upstream release that does not hold: {why}")
        }),
        None => release::contained(git, source.settings, source.up, source.tags, commit)
            .map_err(|err| format!("{err:#}")),
    })
}

/// The upstream release a release tag's annotation records: the publish
/// remote's tag, or else the repository's own.
fn recorded_of(git: &Git, remote: &Remote, name: &str) -> Result<Option<Recorded>> {
    if let Some(t) = remote.entry(name) {
        return Ok(t.recorded.clone());
    }
    Ok(crate::upstreams::tag_refs(git, "refs/tags/")?
        .into_iter()
        .find(|t| t.name == name)
        .and_then(|t| t.recorded))
}

/// Why a dated tag's base is not the release base of the commit `at`
/// names.
fn base_problem(tag: &ReleaseTag, at: &str, upstream: &UpstreamRelease) -> Option<String> {
    let base = upstream.base();
    let carried = tag.base()?;
    (*carried != base).then(|| {
        format!(
            "'{}' carries the base {carried}, but the release base of {at} is {base} (upstream '{}' at {})",
            tag.name, upstream.upstream, upstream.tag
        )
    })
}

/// Why a semver tag's version is not the one the commit `at` names takes.
fn version_problem(tag: &ReleaseTag, at: &str, planned: &SemverNext) -> Option<String> {
    match (&planned.version, planned.bump) {
        (Some(v), _) if *v == tag.version => None,
        (Some(v), Some(bump)) => Some(format!(
            "'{}' carries the version {}, but {at} takes {v}, a {} change since {}",
            tag.name,
            tag.version,
            bump.as_str(),
            planned.since()
        )),
        _ => Some(format!(
            "'{}' is on {at}, which takes no release: its commits since {} ask for none",
            tag.name,
            planned.since()
        )),
    }
}

/// The result of `release version`.
#[derive(Serialize)]
struct VersionReport {
    version: String,
    /// The release tag, or `null` for a rehearsal.
    tag: Option<String>,
    rehearsal: bool,
    /// Under `semver` only.
    #[serde(flatten)]
    semver: Option<SemverReport>,
}

/// What `release version` adds under `semver`.
#[derive(Serialize)]
struct SemverReport {
    /// `<upstream>-<x.y.z>` of the newest upstream release.
    upstream: String,
    /// The previous release of the versioned commit, or `null`.
    previous: Option<String>,
    /// The change that sets the versioned commit's next release, or `null`
    /// when it takes none.
    bump: Option<Bump>,
}

impl Render for VersionReport {
    fn render(&self) -> String {
        format!("{}\n", self.version)
    }
}

/// `release version`.
pub fn version(repo: &Repo, given: Option<&str>, json: bool, dotenv: bool) -> Result<Exit> {
    let config = config_at_head(repo)?;
    let head = repo.head().context("a checked-out commit")?.to_string();
    let (settings, _) = release::settings(&config, "the checked-out commit")?;
    let report = match settings.versioning {
        Versioning::Dated => dated_version(repo, &config, &head, given)?,
        Versioning::Semver => semver_version(repo, &config, &head, given)?,
    };
    if dotenv {
        let mut out = format!("RELEASE_VERSION={}\n", report.version);
        if let Some(tag) = &report.tag {
            out.push_str(&format!("RELEASE_TAG={tag}\n"));
        }
        if let Some(semver) = &report.semver {
            out.push_str(&format!("RELEASE_UPSTREAM={}\n", semver.upstream));
        }
        crate::output::print(&redacted(&out))?;
    } else {
        emit(&report, json)?;
    }
    Ok(Exit::Success)
}

fn dated_version(
    repo: &Repo,
    config: &Config,
    head: &str,
    given: Option<&str>,
) -> Result<VersionReport> {
    let git = &repo.git;
    let (settings, up) = release::settings(config, "the checked-out commit")?;
    let tags = release::upstream_tags(git, up, &Known::default())?;
    if let Some(name) = named_tag(given) {
        let Some(tag) = new_release_tag(&name, settings) else {
            bail!(
                "'{name}' is not a release tag v<x.y.z>-{}.<YYYYMMDD>.<n> with the current label",
                settings.label()
            );
        };
        let commit = tag_commit(git, &name)?;
        let upstream = release::contained(git, settings, up, &tags, &commit)?;
        if let Some(problem) = base_problem(
            &tag,
            &format!("the tagged commit {}", abbrev(&commit)),
            &upstream,
        ) {
            bail!("{problem}");
        }
        return Ok(VersionReport {
            version: tag.version_text(),
            tag: Some(name),
            rehearsal: false,
            semver: None,
        });
    }
    let upstream = release::contained(git, settings, up, &tags, head)?;
    let mut version = upstream.base();
    version.pre = semver::Prerelease::new(&format!("{}.{}.0", settings.label(), rehearsal_day()?))?;
    Ok(VersionReport {
        version: version.to_string(),
        tag: None,
        rehearsal: true,
        semver: None,
    })
}

fn semver_version(
    repo: &Repo,
    config: &Config,
    head: &str,
    given: Option<&str>,
) -> Result<VersionReport> {
    let git = &repo.git;
    let (settings, up) = release::settings(config, "the checked-out commit")?;
    let publish = &config.publish;
    let name = named_tag(given);
    let tag = match &name {
        Some(name) => Some(
            new_release_tag(name, settings)
                .with_context(|| format!("'{name}' is not a release tag v<x.y.z>"))?,
        ),
        None => None,
    };
    let commit = match &name {
        Some(name) => tag_commit(git, name)?,
        None => head.to_string(),
    };
    let tags = release::upstream_tags(git, up, &Known::default())?;
    let mut fetches = Fetches::new(repo);
    let remote = release::fetch_remote(&mut fetches, &publish.remote, &config.integration)?;
    let source = release::Source {
        settings,
        up,
        tags: &tags,
    };
    let upstream = match &name {
        Some(name) => {
            let recorded = recorded_of(git, &remote, name)?;
            tag_upstream(git, &source, name, recorded.as_ref(), &commit, false)?
                .map_err(|why| anyhow::anyhow!("{why}"))?
        }
        None => release::contained(git, settings, up, &tags, &commit)?,
    };
    let synced = release::last_synced_offline(git, &mut fetches, config, &commit)?;
    let line = release::line(git, &commit, &release::synced_commits(&synced))?;
    let below = match line.first() {
        Some(first) if *first == commit => &line[1..],
        _ => &line[..],
    };
    let releases = release::releases_on(&line, &remote.tags, settings);
    let planned = release::semver_next(git, &source, &commit, below, &releases, &upstream)?;
    let semver = Some(SemverReport {
        upstream: upstream.label(),
        previous: planned.previous.as_ref().map(|p| p.tag.name.clone()),
        bump: planned.bump,
    });
    let report = match tag {
        Some(tag) => {
            let at = format!("the tagged commit {}", abbrev(&commit));
            if let Some(problem) = version_problem(&tag, &at, &planned) {
                bail!("{problem}");
            }
            VersionReport {
                version: tag.version_text(),
                tag: name,
                rehearsal: false,
                semver,
            }
        }
        None => {
            let mut version = planned.version.clone().unwrap_or(planned.smallest.clone());
            version.pre = semver::Prerelease::new(&format!("rehearsal.{}", rehearsal_day()?))?;
            VersionReport {
                version: version.to_string(),
                tag: None,
                rehearsal: true,
                semver,
            }
        }
    };
    Ok(report)
}

/// The commit the repository's tag `name` points at.
fn tag_commit(git: &Git, name: &str) -> Result<String> {
    git.commit(&format!("refs/tags/{name}"))?
        .with_context(|| format!("the repository has no tag '{name}'"))
}

/// The UTC day of `CI_PIPELINE_CREATED_AT`, or else of now.
fn rehearsal_day() -> Result<u32> {
    let at = match std::env::var("CI_PIPELINE_CREATED_AT")
        .ok()
        .filter(|v| !v.trim().is_empty())
    {
        Some(text) => text
            .trim()
            .parse()
            .with_context(|| format!("CI_PIPELINE_CREATED_AT '{text}' is not an RFC 3339 time"))?,
        None => release::now(),
    };
    Ok(release::utc_day(at))
}

/// `release notes`.
pub fn notes(repo: &Repo, given: Option<&str>) -> Result<Exit> {
    let git = &repo.git;
    let name = named_tag(given).context(
        "name the release tag to write the notes of, or run in a tag pipeline that sets CI_COMMIT_TAG",
    )?;
    let config = config_at_head(repo)?;
    let (settings, up) = release::settings(&config, "the checked-out commit")?;
    let publish = &config.publish;
    let semver = settings.versioning == Versioning::Semver;
    let tag = ReleaseTag::parse(&name, settings).with_context(|| {
        if semver {
            format!("'{name}' is not a release tag v<x.y.z>")
        } else {
            format!("'{name}' is not a release tag v<x.y.z>-<label>.<YYYYMMDD>.<n>")
        }
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
    let known = Known::default();
    let synced = release::last_synced(git, &mut fetches, &config, &commit, &known)?;
    let synced_commits = release::synced_commits(&synced);
    let line = release::line(git, &commit, &synced_commits)?;
    let releases = release::releases_on(&line, &remote.tags, settings);
    let previous = previous_release(&line, &releases, &tag, &commit);
    let mut notes = String::new();
    let mut moved = None;
    if semver {
        let tags = release::upstream_tags(git, up, &known)?;
        let source = release::Source {
            settings,
            up,
            tags: &tags,
        };
        let recorded = recorded_of(git, &remote, &name)?.filter(|_| tag.dated.is_none());
        let upstream = tag_upstream(git, &source, &name, recorded.as_ref(), &commit, false)?
            .map_err(|why| anyhow::anyhow!("{why}"))?;
        if let Some(p) = previous
            && let Some((before, version)) = release::release_upstream(git, &source, p)?
            && !crate::bump::compatible(&version, &upstream.version)
        {
            moved = Some(format!(
                "- Major: upstream {} moved from {before} to {}",
                upstream.upstream, upstream.tag
            ));
        }
        let carried = synced.iter().find(|(name, _)| *name == up.name);
        notes = carries(git, &upstream, carried.map(|(_, c)| c.as_str()))? + "\n";
    }
    notes.push_str(&write_notes(
        git,
        &commit,
        previous,
        &synced_commits,
        semver,
        moved,
    )?);
    crate::output::print(&redacted(&notes))?;
    Ok(Exit::Success)
}

/// The first line of semver notes: the upstream release a release carries,
/// and how many upstream commits after it, up to `synced`.
fn carries(git: &Git, upstream: &UpstreamRelease, synced: Option<&str>) -> Result<String> {
    let after = match synced {
        Some(synced) => git
            .line(&[
                "rev-list",
                "--count",
                synced,
                &format!("^{}", upstream.commit),
                "--",
            ])?
            .parse::<u64>()
            .context("git rev-list --count prints a number")?,
        None => 0,
    };
    let name = format!("{} {}", upstream.upstream, upstream.tag);
    Ok(match after {
        0 => format!("Carries {name}.\n"),
        1 => format!("Carries {name} and 1 upstream commit after it.\n"),
        n => format!("Carries {name} and {n} upstream commits after it.\n"),
    })
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

/// The Markdown notes of the release at `commit` after `previous`, with
/// the breaking changes first when `breaking_first`, led by `moved`, an
/// incompatible upstream release.
fn write_notes(
    git: &Git,
    commit: &str,
    previous: Option<&Release>,
    synced: &[String],
    breaking_first: bool,
    moved: Option<String>,
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
    let messages = messages(git, &listed)?;
    let mut breaking: Vec<String> = moved.into_iter().collect();
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
            None => {
                let (subject, message) = messages.get(c).cloned().unwrap_or_default();
                match crate::bump::breaking(&message).filter(|_| breaking_first) {
                    Some(Some(text)) => breaking.push(format!(
                        "- {subject}\n\n  {}",
                        text.lines().map(str::trim).collect::<Vec<_>>().join("\n  ")
                    )),
                    Some(None) => breaking.push(format!("- {subject}")),
                    None => entries.push(format!("- {subject}")),
                }
            }
        }
    }
    let since = &previous.tag.name;
    Ok(match (breaking.is_empty(), entries.is_empty()) {
        (true, true) => format!("No changes since the previous release, {since}.\n"),
        (true, false) => format!("Changes since {since}:\n\n{}\n", entries.join("\n")),
        (false, true) => format!(
            "Breaking changes since {since}:\n\n{}\n",
            breaking.join("\n")
        ),
        (false, false) => format!(
            "Breaking changes since {since}:\n\n{}\n\nOther changes since {since}:\n\n{}\n",
            breaking.join("\n"),
            entries.join("\n")
        ),
    })
}

/// The subject and the message of each of `commits`.
fn messages(
    git: &Git,
    commits: &[&str],
) -> Result<std::collections::HashMap<String, (String, String)>> {
    if commits.is_empty() {
        return Ok(Default::default());
    }
    let input: String = commits.iter().map(|c| format!("{c}\n")).collect();
    let out = git.run_with_input(
        &[
            "log",
            "--no-walk=unsorted",
            "--stdin",
            "-z",
            "--format=%H%x1f%s%x1f%B",
            "--",
        ],
        input,
    )?;
    Ok(out
        .split('\0')
        .filter_map(|r| {
            let mut parts = r.trim_start_matches('\n').splitn(3, '\u{1f}');
            let (c, s, m) = (parts.next()?, parts.next()?, parts.next()?);
            Some((c.to_string(), (s.to_string(), m.to_string())))
        })
        .collect())
}
