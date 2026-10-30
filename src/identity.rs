//! Whether a git remote, or a URL pushed to, names the repository of an
//! upstream: the one comparison behind every guarantee that the tool never
//! pushes to an upstream, in the validation of the configuration, in
//! `doctor`, and before each push.
//!
//! Places are compared as git reaches them: a remote's URLs after git's own
//! `url.<base>.insteadOf` and `url.<base>.pushInsteadOf` rewriting, as
//! `git remote get-url` reports them, and an upstream's URL as
//! `git ls-remote --get-url` resolves it for a fetch. The URLs as configured
//! are compared as well. Each URL is then made comparable: a local path or
//! `file://` URL names the repository git finds there, made canonical; any
//! other URL, the scp-like `[user@]host:path` read as `ssh`, is compared
//! without its credentials, by scheme, host in lower case, port unless it is
//! the scheme's default, and path without its trailing `/` and `.git`.

use std::path::{Component, Path, PathBuf};

use anyhow::{Result, bail};
use percent_encoding::percent_decode_str;
use url::Url;

use crate::config::Upstream;
use crate::git::{Git, lines};
use crate::redact::redacted;

/// A URL git reaches for a remote: as configured, and as git resolves it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    pub configured: String,
    pub resolved: String,
}

impl Place {
    fn plain(url: &str) -> Self {
        Self {
            configured: url.to_string(),
            resolved: url.to_string(),
        }
    }

    /// The place for a message, without credentials.
    pub fn shown(&self) -> String {
        let configured = redacted(&self.configured);
        if self.configured == self.resolved {
            configured
        } else {
            format!(
                "{configured} (rewritten by git to {})",
                redacted(&self.resolved)
            )
        }
    }
}

/// An upstream by its name and URL, with the comparable forms of the URL
/// as configured and as git resolves it.
pub struct Known {
    pub name: String,
    pub url: String,
    ids: Vec<String>,
}

/// The upstreams of a configuration, each with the places its URL names.
pub struct Upstreams {
    base: PathBuf,
    known: Vec<Known>,
}

impl Upstreams {
    /// Resolves the URL of each of `upstreams` as `git` fetches it.
    pub fn new(git: &Git, upstreams: &[Upstream]) -> Result<Self> {
        Self::named(
            git,
            upstreams.iter().map(|u| (u.name.clone(), u.url.clone())),
        )
    }

    /// [`Self::new`] for upstreams given by their names and URLs.
    pub fn named(git: &Git, upstreams: impl IntoIterator<Item = (String, String)>) -> Result<Self> {
        let base = git.dir().to_path_buf();
        let mut known = Vec::new();
        for (name, url) in upstreams {
            let mut urls = vec![url.clone()];
            let out = git.raw(&["ls-remote", "--get-url", "--end-of-options", &url])?;
            if out.success() {
                urls.push(out.stdout.trim().to_string());
            }
            urls.dedup();
            let ids = urls.iter().map(|u| normalise(u, &base)).collect();
            known.push(Known { name, url, ids });
        }
        Ok(Self { base, known })
    }

    /// The upstream whose repository `place` names, if any.
    pub fn named_by(&self, place: &Place) -> Option<&Known> {
        let mut ids = vec![normalise(&place.configured, &self.base)];
        if place.resolved != place.configured {
            ids.push(normalise(&place.resolved, &self.base));
        }
        self.known
            .iter()
            .find(|known| known.ids.iter().any(|k| ids.contains(k)))
    }

    /// Each upstream a place of `remote`, fetched from or pushed to, names,
    /// with the first place that names it.
    pub fn named_by_remote(&self, git: &Git, remote: &str) -> Result<Vec<(Place, &Known)>> {
        let mut found: Vec<(Place, &Known)> = Vec::new();
        for place in remote_places(git, remote)? {
            if let Some(up) = self.named_by(&place)
                && !found.iter().any(|(_, u)| u.name == up.name)
            {
                found.push((place, up));
            }
        }
        Ok(found)
    }

    /// Refuses a push to `target`, a remote or a URL, when any place the
    /// push goes to names an upstream's repository.
    pub fn refuse_push(&self, git: &Git, target: &str) -> Result<()> {
        for place in push_places(git, target)? {
            if let Some(up) = self.named_by(&place) {
                bail!(
                    "'{}' pushes to {}, which names the repository of upstream '{}' ({}); the tool never pushes \
                     to an upstream. Nothing was pushed. Point the publish remote at the downstream's own \
                     repository, then run the command again",
                    redacted(target),
                    place.shown(),
                    up.name,
                    redacted(&up.url)
                );
            }
        }
        Ok(())
    }
}

/// Every git remote of the repository: those of its configuration, and
/// those of the `remotes` and `branches` directories of its git directory,
/// which `git remote` does not list.
pub fn remotes(git: &Git) -> Result<Vec<String>> {
    let mut all = lines(&git.run(&["remote"])?);
    for kind in ["remotes", "branches"] {
        let dir = git.line(&["rev-parse", "--path-format=absolute", "--git-path", kind])?;
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if entry.path().is_file() && !all.contains(&name) {
                all.push(name);
            }
        }
    }
    Ok(all)
}

/// The places git fetches `remote` from and pushes it to.
pub fn remote_places(git: &Git, remote: &str) -> Result<Vec<Place>> {
    let urls = configured(git, remote, "url")?;
    let mut places = paired(&urls, resolved(git, remote, false)?);
    places.extend(pushed(git, remote)?);
    let mut unique: Vec<Place> = Vec::new();
    for place in places {
        if !unique.contains(&place) {
            unique.push(place);
        }
    }
    Ok(unique)
}

/// The places a push to `target` goes to: a remote's push URLs, or else its
/// URLs, as `git push` resolves them; a URL as git rewrites it for a push.
pub fn push_places(git: &Git, target: &str) -> Result<Vec<Place>> {
    // A remote of the configuration or of the `remotes` directory, which
    // `git remote` does not list.
    let remote = git.raw(&["remote", "get-url", "--end-of-options", target])?;
    if remote.success() {
        return pushed(git, target);
    }
    Ok(vec![Place {
        configured: target.to_string(),
        resolved: rewritten_for_push(git, target)?,
    }])
}

/// The places a push to the remote `remote` goes to.
fn pushed(git: &Git, remote: &str) -> Result<Vec<Place>> {
    let mut from = configured(git, remote, "pushurl")?;
    if from.is_empty() {
        from = configured(git, remote, "url")?;
    }
    Ok(paired(&from, resolved(git, remote, true)?))
}

/// The values of `remote.<remote>.<key>`, as configured.
fn configured(git: &Git, remote: &str, key: &str) -> Result<Vec<String>> {
    let out = git.raw(&["config", "--get-all", &format!("remote.{remote}.{key}")])?;
    match out.code {
        Some(0) => Ok(lines(&out.stdout)),
        Some(1) => Ok(Vec::new()),
        _ => Err(out.into_error().into()),
    }
}

/// The URLs git fetches `remote` from, or with `push` pushes it to, after
/// its rewriting; `None` when git cannot tell, as for a remote without one.
fn resolved(git: &Git, remote: &str, push: bool) -> Result<Option<Vec<String>>> {
    let mut args = vec!["remote", "get-url"];
    if push {
        args.push("--push");
    }
    args.extend(["--all", "--end-of-options", remote]);
    let out = git.raw(&args)?;
    Ok(out.success().then(|| lines(&out.stdout)))
}

/// The configured URLs with what git resolves them to, one for one when git
/// gives as many; otherwise each URL on either side stands for itself.
fn paired(configured: &[String], resolved: Option<Vec<String>>) -> Vec<Place> {
    match resolved {
        Some(resolved) if resolved.len() == configured.len() => configured
            .iter()
            .zip(resolved)
            .map(|(c, r)| Place {
                configured: c.clone(),
                resolved: r,
            })
            .collect(),
        resolved => configured
            .iter()
            .chain(resolved.iter().flatten())
            .map(|u| Place::plain(u))
            .collect(),
    }
}

/// `url` as `git push <url>` rewrites it: by the longest matching
/// `url.<base>.pushInsteadOf`, else by the longest matching
/// `url.<base>.insteadOf`, else unchanged. Git reports this only for a
/// configured remote; a URL given to `git push` is rewritten by the same
/// rule.
fn rewritten_for_push(git: &Git, url: &str) -> Result<String> {
    let out = git.raw(&[
        "config",
        "-z",
        "--get-regexp",
        r"^url\..*\.(push)?insteadof$",
    ])?;
    let entries = match out.code {
        Some(0) => out.stdout,
        Some(1) => return Ok(url.to_string()),
        _ => return Err(out.into_error().into()),
    };
    let mut push: Option<(&str, &str)> = None;
    let mut fetch: Option<(&str, &str)> = None;
    for entry in entries.split('\0').filter(|e| !e.is_empty()) {
        let Some((key, prefix)) = entry.split_once('\n') else {
            continue;
        };
        let Some(key) = key.strip_prefix("url.") else {
            continue;
        };
        let (base, best) = if let Some(base) = key.strip_suffix(".pushinsteadof") {
            (base, &mut push)
        } else if let Some(base) = key.strip_suffix(".insteadof") {
            (base, &mut fetch)
        } else {
            continue;
        };
        if url.starts_with(prefix) && best.is_none_or(|(_, p)| prefix.len() > p.len()) {
            *best = Some((base, prefix));
        }
    }
    Ok(match push.or(fetch) {
        Some((base, prefix)) => format!("{base}{}", &url[prefix.len()..]),
        None => url.to_string(),
    })
}

/// Whether two URLs name the same repository, local paths resolved
/// against `base`.
#[cfg(test)]
fn same_repository(a: &str, b: &str, base: &Path) -> bool {
    normalise(a, base) == normalise(b, base)
}

/// The comparable form of a URL; see the module's documentation.
fn normalise(url: &str, base: &Path) -> String {
    let url = address(url.trim());
    if let Some(path) = local_path(url) {
        let path = Path::new(&path);
        let full = if path.is_absolute() {
            path.to_path_buf()
        } else {
            base.join(path)
        };
        return format!("file:{}", local_repository(&full).display());
    }
    let written = if url.contains("://") {
        url.to_string()
    } else {
        scp_as_ssh(url)
    };
    match Url::parse(&written) {
        Ok(parsed) => comparable(&parsed),
        Err(_) => format!("unparsed:{url}"),
    }
}

/// Hosts whose repository paths ignore case.
const CASELESS_HOSTS: &[&str] = &["github.com", "gitlab.com"];

/// The address of git's `<transport>::<address>` form, which a remote
/// helper reaches; any other URL itself.
fn address(url: &str) -> &str {
    match url.split_once("::") {
        Some((transport, address))
            if transport.starts_with(|c: char| c.is_ascii_alphabetic())
                && transport
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "+.-".contains(c)) =>
        {
            address
        }
        _ => url,
    }
}

/// A parsed URL by host, port and path: one repository is served over
/// several schemes, so the scheme does not tell repositories apart, nor do
/// credentials or the scheme's default port. The path loses doubled and
/// trailing `/`, a `.git` suffix in any case, and a leading `~/`, by which an
/// `ssh` URL names the home directory that an scp-like path starts from; on
/// hosts that ignore the case of paths, its case.
fn comparable(url: &Url) -> String {
    let default_port = match url.scheme() {
        "ssh" | "git+ssh" | "ssh+git" => Some(22),
        "https" => Some(443),
        "http" => Some(80),
        "git" => Some(9418),
        _ => None,
    };
    let host = url
        .host_str()
        .unwrap_or_default()
        .trim_end_matches('.')
        .to_ascii_lowercase();
    let port = match url.port() {
        Some(port) if Some(port) != default_port => format!(":{port}"),
        _ => String::new(),
    };
    let decoded = percent_decode_str(url.path()).decode_utf8_lossy();
    let mut path = decoded
        .split('/')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("/");
    if let Some(home) = path.strip_prefix("~/") {
        path = home.to_string();
    }
    if path.len() >= 4 && path[path.len() - 4..].eq_ignore_ascii_case(".git") {
        path.truncate(path.len() - 4);
    }
    let path = path.trim_end_matches('/');
    let path = if CASELESS_HOSTS.contains(&host.as_str()) {
        path.to_lowercase()
    } else {
        path.to_string()
    };
    format!("{host}{port}/{path}")
}

/// The scp-like `[user@]host:path` as the `ssh://` URL it stands for.
fn scp_as_ssh(url: &str) -> String {
    // A bracketed host, as `[::1]:path`, holds colons of its own.
    let split = match url.find("]:") {
        Some(end) => Some((&url[..=end], &url[end + 2..])),
        None => url.split_once(':'),
    };
    let (authority, path) = split.unwrap_or((url, ""));
    format!("ssh://{authority}/{path}")
}

/// The repository a local path names, as git finds it: the first of
/// `<path>/.git`, `<path>`, `<path>.git/.git` and `<path>.git` that is a git
/// directory of its own; else, as for a linked worktree, whose `.git` is a
/// file, or a worktree's private git directory, the common git directory
/// git reports for `<path>` or `<path>.git`; made canonical. The path
/// itself, made canonical or cleaned, when none is found.
fn local_repository(path: &Path) -> PathBuf {
    let plain = path.as_os_str().to_string_lossy();
    let plain = plain.trim_end_matches('/');
    for candidate in [
        format!("{plain}/.git"),
        plain.to_string(),
        format!("{plain}.git/.git"),
        format!("{plain}.git"),
    ] {
        let dir = Path::new(&candidate);
        if dir.join("HEAD").is_file()
            && !dir.join("commondir").exists()
            && let Ok(found) = std::fs::canonicalize(dir)
        {
            return found;
        }
    }
    for candidate in [plain.to_string(), format!("{plain}.git")] {
        let dir = Path::new(&candidate);
        if !dir.is_dir() {
            continue;
        }
        let found =
            Git::exactly_at(dir).raw(&["rev-parse", "--path-format=absolute", "--git-common-dir"]);
        if let Ok(out) = found
            && out.success()
            && let Ok(common) = std::fs::canonicalize(out.stdout.trim())
        {
            return common;
        }
    }
    std::fs::canonicalize(path).unwrap_or_else(|_| clean(path))
}

/// The path of a URL that names a local repository, as git tells them
/// apart: a `file://` URL, whose host git ignores and whose path it
/// percent-decodes, or no scheme and no colon before the first slash.
fn local_path(url: &str) -> Option<String> {
    if let Some(rest) = url
        .get(..7)
        .filter(|s| s.eq_ignore_ascii_case("file://"))
        .map(|_| &url[7..])
    {
        let path = rest.find('/').map_or("", |at| &rest[at..]);
        return Some(percent_decode_str(path).decode_utf8_lossy().into_owned());
    }
    if url.contains("://") {
        return None;
    }
    match (url.find(':'), url.find('/')) {
        (Some(colon), Some(slash)) if colon < slash => None,
        (Some(_), None) => None,
        _ => Some(url.to_string()),
    }
}

/// A path without `.` and `..` components, for one that does not exist.
fn clean(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_naming_the_same_repository_match() {
        let base = Path::new("/srv/product");
        let same = |a: &str, b: &str| same_repository(a, b, base);
        assert!(same("../up.git", "/srv/up.git"));
        assert!(same("file:///srv/up.git", "/srv/./up.git/"));
        assert!(!same("../up.git", "/srv/up"));
        assert!(same(
            "https://Example.COM/team/up.git",
            "https://example.com:443/team/up/"
        ));
        assert!(same(
            "git@Example.com:team/up.git",
            "git@example.com:team/up"
        ));
        assert!(same(
            "git@example.com:team/up.git",
            "ssh://git@EXAMPLE.com:22/team/up"
        ));
        assert!(same(
            "example.com:/srv/up.git",
            "git+ssh://example.com/srv/up"
        ));
        assert!(!same(
            "git@example.com:team/up",
            "git@example.com:team/other"
        ));
        assert!(!same(
            "git@example.com:team/up",
            "ssh://git@example.com:2222/team/up"
        ));
        assert!(!same(
            "https://example.com/Team/up",
            "https://example.com/team/up"
        ));
    }

    #[test]
    fn credentials_do_not_tell_repositories_apart() {
        let base = Path::new("/srv/product");
        let same = |a: &str, b: &str| same_repository(a, b, base);
        assert!(same(
            "https://user:s3cret@example.com/team/up.git",
            "https://example.com/team/up"
        ));
        assert!(same(
            "https://oauth2:token@example.com:443/team/up",
            "https://other@EXAMPLE.com/team/up.git/"
        ));
        assert!(same(
            "deploy@example.com:team/up",
            "ssh://git@example.com/team/up.git"
        ));
        assert!(same(
            "git://example.com:9418/team/up.git",
            "git://example.com/team/up"
        ));
        assert!(!same(
            "https://user@example.com/team/up",
            "https://user@example.com/team/other"
        ));
        assert!(same("ssh://[::1]/team/up", "[::1]:team/up.git"));
    }

    #[test]
    fn file_urls_are_local_paths_as_git_reads_them() {
        let base = Path::new("/srv/product");
        let same = |a: &str, b: &str| same_repository(a, b, base);
        assert!(same("file://localhost/srv/up.git", "/srv/up.git"));
        assert!(same("file://localhost/srv/up.git", "file:///srv/up.git/"));
        assert!(same("FILE:///srv/up.git", "../up.git"));
        assert!(same("file:///srv/%75p.git", "/srv/up.git"));
        assert!(!same("file://localhost/srv/up.git", "/srv/other.git"));
    }

    #[test]
    fn local_paths_resolve_as_git_finds_repositories() {
        let dir = tempfile::tempdir().unwrap();
        let bare = dir.path().join("up.git");
        std::fs::create_dir_all(&bare).unwrap();
        std::fs::write(bare.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        let work = dir.path().join("work");
        std::fs::create_dir_all(work.join(".git")).unwrap();
        std::fs::write(work.join(".git").join("HEAD"), "ref: refs/heads/main\n").unwrap();
        let base = dir.path().join("product");
        std::fs::create_dir_all(&base).unwrap();
        let same = |a: &str, b: &str| same_repository(a, b, &base);
        let bare_path = bare.to_string_lossy();
        assert!(same("../up", &bare_path));
        assert!(same("../up/", &bare_path));
        assert!(same("../work", &format!("{}/.git", work.to_string_lossy())));
        assert!(!same("../work", &bare_path));
        // A link to the repository names it as well.
        let link = dir.path().join("link.git");
        std::os::unix::fs::symlink(&bare, &link).unwrap();
        assert!(same(
            &format!("file://localhost{}", link.display()),
            &bare_path
        ));
    }

    #[test]
    fn network_urls_are_compared_by_host_and_path() {
        let base = Path::new("/srv/product");
        let same = |a: &str, b: &str| same_repository(a, b, base);
        // The scheme does not tell repositories apart.
        for other in [
            "git@example.com:team/up.git",
            "ssh://example.com/team/up",
            "git+ssh://git@example.com:22/team/up.git",
            "git://example.com/team/up",
            "http://example.com:80/team/up.git",
            "https::https://example.com/team/up",
        ] {
            assert!(same("https://example.com/team/up.git", other), "{other}");
        }
        // A port other than the default still does.
        assert!(!same(
            "https://example.com/team/up",
            "https://example.com:8443/team/up"
        ));
        // The home directory, written either way.
        assert!(same("example.com:team/up", "ssh://example.com/~/team/up"));
        assert!(same("example.com:~ann/up", "ssh://example.com/~ann/up.git"));
        // A host's trailing dot, the case of `.git`, and doubled slashes.
        assert!(same(
            "https://example.com./team/up.GIT",
            "https://example.com/team//up"
        ));
        // Hosts that ignore the case of paths.
        assert!(same(
            "https://GitHub.com/Team/Up",
            "git@github.com:team/up.git"
        ));
        assert!(same(
            "https://gitlab.com/Team/Up",
            "https://gitlab.com/team/up"
        ));
        assert!(!same(
            "https://example.com/Team/up",
            "https://example.com/team/up"
        ));
    }

    #[test]
    fn a_linked_worktree_names_its_repository() {
        let dir = tempfile::tempdir().unwrap();
        let bare = dir.path().join("up.git");
        let git = |args: &[&str]| {
            let out = std::process::Command::new("git")
                .args(args)
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
        };
        let b = bare.to_string_lossy().into_owned();
        git(&["init", "-q", "--bare", "-b", "main", &b]);
        git(&[
            "-C",
            &b,
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.com",
            "commit-tree",
            "-m",
            "x",
            "4b825dc642cb6eb9a060e54bf8d69288fbee4904",
        ]);
        let linked = dir.path().join("linked");
        let l = linked.to_string_lossy().into_owned();
        git(&[
            "-C", &b, "worktree", "add", "-q", "--orphan", "-b", "side", &l,
        ]);
        let base = dir.path().join("product");
        std::fs::create_dir_all(&base).unwrap();
        assert!(same_repository(&l, &b, &base));
        // A directory inside the repository is not the repository.
        std::fs::create_dir_all(linked.join("sub")).unwrap();
        assert!(!same_repository(&format!("{l}/sub"), &b, &base));
    }

    #[test]
    fn a_remote_of_the_remotes_directory_is_pushed_to_as_a_remote() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        let out = std::process::Command::new("git")
            .args(["init", "-q", &repo.to_string_lossy()])
            .output()
            .unwrap();
        assert!(out.status.success());
        std::fs::create_dir_all(repo.join(".git").join("remotes")).unwrap();
        std::fs::write(
            repo.join(".git").join("remotes").join("elsewhere"),
            "URL: /srv/up.git\nPush: main\n",
        )
        .unwrap();
        let git = Git::worktree(&repo);
        assert!(remotes(&git).unwrap().contains(&"elsewhere".to_string()));
        let places = push_places(&git, "elsewhere").unwrap();
        assert_eq!(
            places
                .iter()
                .map(|p| p.resolved.as_str())
                .collect::<Vec<_>>(),
            ["/srv/up.git"]
        );
    }
}
