//! Finding the forge of the publish remote, its API and the token.
//!
//! The forge is that of the publish remote's fetch URL as git uses it, after
//! its `url.<base>.insteadOf` rewriting: the merge request is opened on the
//! forge the branch actually lands on. The kind of forge comes
//! from the setting `forge`, else from the URL's host, else from the CI job
//! when the job's server is that host. The API base comes from the setting
//! `api`, which must be on the remote's host, else from the CI job's
//! variables when the job's server is the remote's host, else from the
//! host. The token comes from the environment only.

use url::Url;

use crate::config::{ForgeKind, Publish};

use super::Kind;
use super::http::Secret;

/// The host, port and repository path of a remote's URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteUrl {
    /// `ssh` for the scp-like form.
    pub scheme: String,
    /// In lower case.
    pub host: String,
    pub port: Option<u16>,
    /// The repository path without a leading or trailing `/` and without
    /// `.git`.
    pub path: String,
}

/// Reads a remote URL that names a host: `scheme://[user@]host[:port]/path`
/// or the scp-like `[user@]host:path`. A local path or `file://` URL names
/// none.
pub fn parse_remote_url(url: &str) -> Option<RemoteUrl> {
    let url = url.trim();
    if url.contains("://") {
        let parsed = Url::parse(url).ok()?;
        if parsed.scheme() == "file" {
            return None;
        }
        let host = parsed
            .host_str()?
            .trim_matches(['[', ']'])
            .to_ascii_lowercase();
        if host.is_empty() {
            return None;
        }
        let scheme = match parsed.scheme() {
            "git+ssh" | "ssh+git" => "ssh".to_string(),
            s => s.to_string(),
        };
        return Some(RemoteUrl {
            scheme,
            host,
            port: parsed.port(),
            path: repository_path(&decode(parsed.path())),
        });
    }
    // The scp-like form: a colon before the first slash.
    let colon = url.find(':')?;
    if url.find('/').is_some_and(|slash| slash < colon) {
        return None;
    }
    let (authority, path) = url.split_at(colon);
    let host = authority
        .rsplit_once('@')
        .map_or(authority, |(_, h)| h)
        .trim_matches(['[', ']'])
        .to_ascii_lowercase();
    if host.is_empty() {
        return None;
    }
    Some(RemoteUrl {
        scheme: "ssh".to_string(),
        host,
        port: None,
        path: repository_path(&path[1..]),
    })
}

fn decode(path: &str) -> String {
    percent_encoding::percent_decode_str(path)
        .decode_utf8_lossy()
        .into_owned()
}

fn repository_path(path: &str) -> String {
    let path = path.trim_matches('/');
    path.strip_suffix(".git")
        .unwrap_or(path)
        .trim_end_matches('/')
        .to_string()
}

/// Where the forge of the publish remote is, and the token to reach it.
#[derive(Debug, Clone)]
pub struct Location {
    pub kind: Kind,
    /// The API base, without a trailing `/`.
    pub api: Url,
    /// The project's path on the forge, such as `group/sub/app`.
    pub project: String,
    /// The token requests carry; `None` asks without credentials.
    pub credential: Option<Credential>,
    /// The variables a token for this forge is read from, as messages name
    /// them.
    pub token_variables: String,
}

impl Location {
    /// The variable the token was read from, or `none`.
    pub fn token_variable(&self) -> &str {
        self.credential
            .as_ref()
            .map_or("none", |c| c.variable.as_str())
    }
}

/// A token read from the environment.
#[derive(Debug, Clone)]
pub struct Credential {
    /// The variable it was read from.
    pub variable: String,
    pub secret: Secret,
    /// Whether it is a GitLab CI job token, sent as one.
    pub job: bool,
}

/// What the tool reaches the forge for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// Opening, changing and reading merge requests: a token is required.
    MergeRequests,
    /// Reading the project's releases: a GitLab CI job token serves, and
    /// without any token the forge is asked without credentials.
    Releases,
    /// Creating a release tag: a token is required, and a GitLab CI job
    /// token does not serve.
    Tags,
}

/// What the location needs from outside: the environment, and the host an
/// SSH host alias stands for.
pub struct Surroundings<'a> {
    pub env: &'a dyn Fn(&str) -> Option<String>,
    pub ssh_host: &'a dyn Fn(&str) -> Option<String>,
}

impl Surroundings<'_> {
    fn var(&self, name: &str) -> Option<String> {
        (self.env)(name).filter(|v| !v.trim().is_empty())
    }

    /// The host of the CI job's server, when the tool runs in a job of that
    /// kind of forge.
    fn job_host(&self, kind: Kind) -> Option<String> {
        Some(self.job_server(kind)?.host_str()?.to_ascii_lowercase())
    }

    /// The server URL of the CI job the tool runs in, when it is a job of
    /// that kind of forge.
    fn job_server(&self, kind: Kind) -> Option<Url> {
        let (flag, server) = match kind {
            Kind::GitLab => ("GITLAB_CI", "CI_SERVER_URL"),
            Kind::GitHub => ("GITHUB_ACTIONS", "GITHUB_SERVER_URL"),
        };
        if self.var(flag).as_deref() != Some("true") {
            return None;
        }
        Url::parse(self.var(server)?.trim()).ok()
    }
}

/// Settings that tell an unknown forge, as refusals name them.
const SETTINGS_HINT: &str = "set `forge` (`gitlab` or `github`) in the [publish] table of .git-intake.toml, \
     and also `api` (the base URL of its REST API) when the API is not at the usual address on the remote's host, \
     as for an SSH host alias or a relative URL root";

/// The kind of forge of a publish remote on `host`: the setting `forge`,
/// else the host, else the CI job the tool runs in when the job's server is
/// that host.
fn kind_of(publish: &Publish, host: &str, around: &Surroundings) -> Option<Kind> {
    match publish.forge {
        Some(ForgeKind::Gitlab) => Some(Kind::GitLab),
        Some(ForgeKind::Github) => Some(Kind::GitHub),
        None if host == "gitlab.com" => Some(Kind::GitLab),
        None if host == "github.com" || host.ends_with(".ghe.com") => Some(Kind::GitHub),
        None if around.job_host(Kind::GitLab).as_deref() == Some(host) => Some(Kind::GitLab),
        None if around.job_host(Kind::GitHub).as_deref() == Some(host) => Some(Kind::GitHub),
        None => None,
    }
}

/// The kind of forge of the publish remote whose fetch URL is `url`, when
/// the tool can tell it without the forge's API; `None` otherwise.
pub fn kind(publish: &Publish, url: Option<&str>, around: &Surroundings) -> Option<Kind> {
    match publish.forge {
        Some(ForgeKind::Gitlab) => return Some(Kind::GitLab),
        Some(ForgeKind::Github) => return Some(Kind::GitHub),
        None => {}
    }
    let parsed = parse_remote_url(url?)?;
    kind_of(publish, &parsed.host, around)
}

/// Locates the forge of the publish remote whose fetch URL is `url`.
pub fn locate(
    publish: &Publish,
    url: Option<&str>,
    around: &Surroundings,
    access: Access,
) -> std::result::Result<Location, String> {
    let remote = &publish.remote;
    let Some(url) = url else {
        return Err(format!(
            "the publish remote '{remote}' does not exist; add it with `git remote add {remote} <url>`"
        ));
    };
    let Some(parsed) = parse_remote_url(url) else {
        return Err(format!(
            "the URL of the publish remote '{remote}' names no host, so the tool cannot tell its forge; \
             {} needs a remote on GitLab or GitHub",
            match access {
                Access::MergeRequests => "publication through a merge request",
                Access::Releases => "reading the releases of the forge",
                Access::Tags => "creating a release tag through the forge's API",
            }
        ));
    };
    let host = parsed.host.clone();
    let Some(kind) = kind_of(publish, &host, around) else {
        return Err(format!(
            "the publish remote '{remote}' is on '{host}', which the tool does not recognise as GitLab or GitHub; \
             {SETTINGS_HINT}"
        ));
    };
    let in_job = around.job_host(kind).as_deref() == Some(host.as_str());
    let api = match &publish.api {
        Some(api) => {
            let base =
                Url::parse(api).map_err(|e| format!("the setting `api` is not a URL: {e}"))?;
            let api_host = base.host_str().unwrap_or_default().to_ascii_lowercase();
            if !own_host(kind, &parsed, &api_host, around) {
                return Err(format!(
                    "the setting `api` names the host '{api_host}', but the publish remote '{remote}' is on '{host}'; \
                     the token is sent only to the API of the remote's own host, so `api` must name that host"
                ));
            }
            base
        }
        None => {
            let from_job = match kind {
                Kind::GitLab => "CI_API_V4_URL",
                Kind::GitHub => "GITHUB_API_URL",
            };
            match around.var(from_job).filter(|_| in_job) {
                Some(api) => {
                    let base = Url::parse(api.trim().trim_end_matches('/'))
                        .map_err(|e| format!("{from_job} is not a URL: {e}"))?;
                    let api_host = base.host_str().unwrap_or_default().to_ascii_lowercase();
                    let same = |h: &str| {
                        api_host == h || (kind == Kind::GitHub && api_host == format!("api.{h}"))
                    };
                    if !same(&host) {
                        return Err(format!(
                            "{from_job} names the host '{api_host}', but the publish remote '{remote}' is on '{host}'; \
                             the token is sent only to the API of the remote's own host"
                        ));
                    }
                    // `https`, or the scheme of the job's own server, which
                    // a self-managed instance may serve without TLS.
                    let server = around.job_server(kind).map(|u| u.scheme().to_string());
                    if base.scheme() != "https" && server.as_deref() != Some(base.scheme()) {
                        return Err(format!(
                            "{from_job} is not an `https` URL, and the job's server is not reached by `{}` either; \
                             the token is not sent over it",
                            base.scheme()
                        ));
                    }
                    base
                }
                None => default_api(kind, &parsed)?,
            }
        }
    };
    let project = project_path(&parsed.path, &api);
    if project.is_empty() {
        return Err(format!(
            "the URL of the publish remote '{remote}' names no project on '{host}'"
        ));
    }
    if kind == Kind::GitHub && project.split('/').count() != 2 {
        return Err(format!(
            "the URL of the publish remote '{remote}' names '{project}', which is not a GitHub repository \
             `<owner>/<repository>`; {SETTINGS_HINT}"
        ));
    }
    let names = token_names(kind, &host, &api, in_job);
    let mut token_variables = listed(&names);
    let mut credential = names.iter().find_map(|name| {
        around.var(name).map(|value| Credential {
            variable: name.to_string(),
            secret: Secret::new(value.trim().to_string()),
            job: false,
        })
    });
    if access == Access::Releases && kind == Kind::GitLab {
        token_variables.push_str(&format!(
            ", or {JOB_TOKEN} in a GitLab CI job of the remote's own host"
        ));
        if credential.is_none() && in_job {
            credential = around.var(JOB_TOKEN).map(|value| Credential {
                variable: JOB_TOKEN.to_string(),
                secret: Secret::new(value.trim().to_string()),
                job: true,
            });
        }
    }
    if credential.is_none() && access != Access::Releases {
        let job = if access == Access::Tags && kind == Kind::GitLab {
            "; a CI job token cannot create a tag that starts a pipeline"
        } else {
            ""
        };
        return Err(format!(
            "no token for the {} API at {api} is set; set {token_variables} in the environment{job}",
            kind.name(),
        ));
    }
    Ok(Location {
        kind,
        api,
        project,
        credential,
        token_variables,
    })
}

/// Whether the API host `api_host` is the publish remote's own host: the
/// remote's host, or the host an SSH host alias stands for; for GitHub also
/// the API host of github.com and of `ghe.com` hosts, `api.<host>`.
fn own_host(kind: Kind, remote: &RemoteUrl, api_host: &str, around: &Surroundings) -> bool {
    let matches = |host: &str| {
        api_host == host || (kind == Kind::GitHub && api_host == format!("api.{host}"))
    };
    if matches(&remote.host) {
        return true;
    }
    // An address names no SSH host alias, so there is nothing to resolve.
    remote.scheme == "ssh"
        && remote.host.parse::<std::net::IpAddr>().is_err()
        && (around.ssh_host)(&remote.host)
            .map(|h| h.to_ascii_lowercase())
            .is_some_and(|h| matches(&h))
}

/// The API base a forge has on the remote's host.
fn default_api(kind: Kind, remote: &RemoteUrl) -> std::result::Result<Url, String> {
    let host = &remote.host;
    // The port of an `https` remote is the web server's; any other is git's.
    let authority = match (remote.scheme.as_str(), remote.port) {
        ("https", Some(port)) => format!("{host}:{port}"),
        _ => host.clone(),
    };
    let text = match kind {
        Kind::GitLab => format!("https://{authority}/api/v4"),
        Kind::GitHub if host == "github.com" => "https://api.github.com".to_string(),
        Kind::GitHub if host.ends_with(".ghe.com") => format!("https://api.{host}"),
        Kind::GitHub => format!("https://{authority}/api/v3"),
    };
    Url::parse(&text).map_err(|e| format!("cannot form the API address {text}: {e}"))
}

/// The project's path: the remote's path without the part the API base
/// carries before `/api/v4` or `/api/v3`, as a GitLab installed under a
/// relative URL root has it.
fn project_path(path: &str, api: &Url) -> String {
    let base = api.path().trim_end_matches('/');
    let root = ["/api/v4", "/api/v3"]
        .iter()
        .find_map(|suffix| base.strip_suffix(suffix))
        .unwrap_or("")
        .trim_matches('/');
    if root.is_empty() {
        return path.to_string();
    }
    match path
        .strip_prefix(root)
        .and_then(|rest| rest.strip_prefix('/'))
    {
        Some(rest) => rest.to_string(),
        None => path.to_string(),
    }
}

/// The token of a GitLab CI job.
pub(super) const JOB_TOKEN: &str = "CI_JOB_TOKEN";

/// Every variable a token is read from.
pub const TOKEN_VARIABLES: &[&str] = &[
    "GITLAB_TOKEN",
    "GH_TOKEN",
    "GITHUB_TOKEN",
    "GH_ENTERPRISE_TOKEN",
    "GITHUB_ENTERPRISE_TOKEN",
    JOB_TOKEN,
];

/// The variables the token for the forge is read from, in order: those its
/// own command-line tools read; each is one of [`TOKEN_VARIABLES`].
fn token_names(kind: Kind, host: &str, api: &Url, in_job: bool) -> Vec<&'static str> {
    let api_host = api.host_str().unwrap_or_default();
    match kind {
        Kind::GitLab => vec!["GITLAB_TOKEN"],
        Kind::GitHub
            if host == "github.com"
                || host.ends_with(".ghe.com")
                || api_host == "api.github.com"
                || (api_host.starts_with("api.") && api_host.ends_with(".ghe.com")) =>
        {
            vec!["GH_TOKEN", "GITHUB_TOKEN"]
        }
        // A GitHub Enterprise Server: never a github.com token, except the
        // token an Actions job of that server is given.
        Kind::GitHub if in_job => vec![
            "GH_ENTERPRISE_TOKEN",
            "GITHUB_ENTERPRISE_TOKEN",
            "GH_TOKEN",
            "GITHUB_TOKEN",
        ],
        Kind::GitHub => vec!["GH_ENTERPRISE_TOKEN", "GITHUB_ENTERPRISE_TOKEN"],
    }
}

/// Names as a message lists them: `A`, `A, or B`, `A, B, or C`.
fn listed(names: &[&str]) -> String {
    match names {
        [one] => one.to_string(),
        [rest @ .., last] => format!("{}, or {last}", rest.join(", ")),
        [] => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::config::PublishVia;

    fn publish(forge: Option<ForgeKind>, api: Option<&str>) -> Publish {
        Publish {
            remote: "origin".into(),
            via: PublishVia::MergeRequest,
            forge,
            api: api.map(str::to_string),
            queue: None,
            replace_after: None,
        }
    }

    fn located(
        publish: &Publish,
        url: &str,
        vars: &[(&str, &str)],
    ) -> std::result::Result<Location, String> {
        let vars: HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        let env = move |name: &str| vars.get(name).cloned();
        let ssh = |alias: &str| (alias == "work").then(|| "gitlab.example.com".to_string());
        locate(
            publish,
            Some(url),
            &Surroundings {
                env: &env,
                ssh_host: &ssh,
            },
            Access::MergeRequests,
        )
    }

    #[test]
    fn remote_urls_are_read_in_every_form() {
        let p = parse_remote_url("git@github.com:team/app.git").unwrap();
        assert_eq!(
            (p.scheme.as_str(), p.host.as_str(), p.path.as_str()),
            ("ssh", "github.com", "team/app")
        );
        let p =
            parse_remote_url("https://user:pw@GitLab.Example.com:8443/group/sub/app.git/").unwrap();
        assert_eq!(p.host, "gitlab.example.com");
        assert_eq!(p.port, Some(8443));
        assert_eq!(p.path, "group/sub/app");
        let p = parse_remote_url("ssh://git@gitlab.com:22/group/app").unwrap();
        assert_eq!(
            (p.host.as_str(), p.path.as_str()),
            ("gitlab.com", "group/app")
        );
        assert!(parse_remote_url("../publish.git").is_none());
        assert!(parse_remote_url("/srv/publish.git").is_none());
        assert!(parse_remote_url("file:///srv/publish.git").is_none());
        assert!(parse_remote_url("./a:b").is_none());
    }

    #[test]
    fn the_forge_comes_from_the_host() {
        let l = located(
            &publish(None, None),
            "git@github.com:team/app.git",
            &[("GH_TOKEN", "t")],
        )
        .unwrap();
        assert_eq!(l.kind, Kind::GitHub);
        assert_eq!(l.project, "team/app");
        assert_eq!(l.api.as_str(), "https://api.github.com/");
        assert_eq!(l.token_variable(), "GH_TOKEN");
        let l = located(
            &publish(None, None),
            "https://gitlab.com/group/sub/app.git",
            &[("GITLAB_TOKEN", "t")],
        )
        .unwrap();
        assert_eq!(l.kind, Kind::GitLab);
        assert_eq!(l.api.as_str(), "https://gitlab.com/api/v4");
        assert_eq!(l.project, "group/sub/app");
        let l = located(
            &publish(None, None),
            "git@acme.ghe.com:team/app.git",
            &[("GITHUB_TOKEN", "t")],
        )
        .unwrap();
        assert_eq!(l.api.as_str(), "https://api.acme.ghe.com/");
    }

    #[test]
    fn an_unknown_host_names_the_settings() {
        let e = located(
            &publish(None, None),
            "git@work:group/app.git",
            &[("GITLAB_TOKEN", "t")],
        )
        .unwrap_err();
        assert!(
            e.contains("'work', which the tool does not recognise"),
            "{e}"
        );
        assert!(e.contains("`forge`") && e.contains("`api`"), "{e}");
    }

    #[test]
    fn a_self_managed_gitlab_in_its_own_ci_job() {
        let vars = [
            ("GITLAB_CI", "true"),
            ("CI_SERVER_URL", "https://gitlab.example.com/gitlab"),
            ("CI_API_V4_URL", "https://gitlab.example.com/gitlab/api/v4"),
            ("GITLAB_TOKEN", "t"),
        ];
        let l = located(
            &publish(None, None),
            "https://gitlab.example.com/gitlab/group/app.git",
            &vars,
        )
        .unwrap();
        assert_eq!(l.kind, Kind::GitLab);
        assert_eq!(l.api.as_str(), "https://gitlab.example.com/gitlab/api/v4");
        assert_eq!(l.project, "group/app");
        // A job of another server leaves the host unknown.
        let e = located(
            &publish(None, None),
            "https://other.example.com/group/app.git",
            &vars,
        )
        .unwrap_err();
        assert!(e.contains("does not recognise"), "{e}");
    }

    #[test]
    fn the_job_s_api_must_be_on_the_remote_s_host() {
        let vars = [
            ("GITLAB_CI", "true"),
            ("CI_SERVER_URL", "https://gitlab.example.com"),
            ("CI_API_V4_URL", "https://collector.example.net/api/v4"),
            ("GITLAB_TOKEN", "t"),
        ];
        let e = located(
            &publish(None, None),
            "https://gitlab.example.com/g/app.git",
            &vars,
        )
        .unwrap_err();
        assert!(
            e.contains("CI_API_V4_URL names the host 'collector.example.net'"),
            "{e}"
        );
        // Plain HTTP only where the job's own server uses it.
        let vars = [
            ("GITLAB_CI", "true"),
            ("CI_SERVER_URL", "https://gitlab.example.com"),
            ("CI_API_V4_URL", "http://gitlab.example.com/api/v4"),
            ("GITLAB_TOKEN", "t"),
        ];
        let e = located(
            &publish(None, None),
            "https://gitlab.example.com/g/app.git",
            &vars,
        )
        .unwrap_err();
        assert!(e.contains("is not an `https` URL"), "{e}");
        let vars = [
            ("GITLAB_CI", "true"),
            ("CI_SERVER_URL", "http://gitlab.example.com"),
            ("CI_API_V4_URL", "http://gitlab.example.com/api/v4"),
            ("GITLAB_TOKEN", "t"),
        ];
        assert!(
            located(
                &publish(None, None),
                "https://gitlab.example.com/g/app.git",
                &vars
            )
            .is_ok()
        );
        // github.com's job API lives on `api.github.com`.
        let vars = [
            ("GITHUB_ACTIONS", "true"),
            ("GITHUB_SERVER_URL", "https://github.com"),
            ("GITHUB_API_URL", "https://api.github.com"),
            ("GITHUB_TOKEN", "t"),
        ];
        assert!(
            located(
                &publish(None, None),
                "https://github.com/team/app.git",
                &vars
            )
            .is_ok()
        );
    }

    #[test]
    fn the_api_setting_must_name_the_remote_s_host() {
        let p = publish(
            Some(ForgeKind::Gitlab),
            Some("https://evil.example.com/api/v4"),
        );
        let e = located(
            &p,
            "https://gitlab.example.com/group/app.git",
            &[("GITLAB_TOKEN", "t")],
        )
        .unwrap_err();
        assert!(
            e.contains("the setting `api` names the host 'evil.example.com'"),
            "{e}"
        );
        // An SSH host alias stands for the host its SSH configuration names.
        let p = publish(
            Some(ForgeKind::Gitlab),
            Some("https://gitlab.example.com/gitlab/api/v4"),
        );
        let l = located(&p, "work:gitlab/group/app.git", &[("GITLAB_TOKEN", "t")]).unwrap();
        assert_eq!(l.project, "group/app");
        // GitHub's API lives on `api.<host>`.
        let p = publish(None, Some("https://api.github.com"));
        assert!(located(&p, "git@github.com:team/app.git", &[("GH_TOKEN", "t")]).is_ok());
    }

    fn located_for_releases(url: &str, vars: &[(&str, &str)]) -> Location {
        let vars: HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        let env = move |name: &str| vars.get(name).cloned();
        let ssh = |_: &str| None;
        locate(
            &publish(None, None),
            Some(url),
            &Surroundings {
                env: &env,
                ssh_host: &ssh,
            },
            Access::Releases,
        )
        .unwrap()
    }

    #[test]
    fn releases_are_read_with_a_job_token_or_without_one() {
        let job = [
            ("GITLAB_CI", "true"),
            ("CI_SERVER_URL", "https://gitlab.example.com"),
            ("CI_JOB_TOKEN", "job-token"),
        ];
        let l = located_for_releases("https://gitlab.example.com/g/app.git", &job);
        let c = l.credential.unwrap();
        assert_eq!((c.variable.as_str(), c.job), ("CI_JOB_TOKEN", true));
        assert!(l.token_variables.contains("GITLAB_TOKEN, or CI_JOB_TOKEN"));
        // GITLAB_TOKEN comes first.
        let mut vars = job.to_vec();
        vars.push(("GITLAB_TOKEN", "t"));
        let l = located_for_releases("https://gitlab.example.com/g/app.git", &vars);
        assert_eq!(l.token_variable(), "GITLAB_TOKEN");
        // A job of another instance gives no token.
        let l = located_for_releases("https://gitlab.com/g/app.git", &job);
        assert!(l.credential.is_none());
        let l = located_for_releases("git@github.com:team/app.git", &[]);
        assert!(l.credential.is_none());
        assert_eq!(l.token_variables, "GH_TOKEN, or GITHUB_TOKEN");
    }

    #[test]
    fn tokens_come_from_the_forge_s_variables() {
        let e = located(
            &publish(None, None),
            "git@gitlab.com:g/app.git",
            &[("GH_TOKEN", "t")],
        )
        .unwrap_err();
        assert!(e.contains("set GITLAB_TOKEN in the environment"), "{e}");
        let l = located(
            &publish(None, None),
            "git@github.com:team/app.git",
            &[("GITHUB_TOKEN", "b"), ("GH_TOKEN", "")],
        )
        .unwrap();
        assert_eq!(l.token_variable(), "GITHUB_TOKEN");
        // A GitHub Enterprise Server gets no github.com token outside its
        // own Actions jobs.
        let p = publish(Some(ForgeKind::Github), None);
        let e = located(
            &p,
            "git@github.example.com:team/app.git",
            &[("GH_TOKEN", "t")],
        )
        .unwrap_err();
        assert!(
            e.contains("set GH_ENTERPRISE_TOKEN, or GITHUB_ENTERPRISE_TOKEN"),
            "{e}"
        );
        let vars = [
            ("GITHUB_ACTIONS", "true"),
            ("GITHUB_SERVER_URL", "https://github.example.com"),
            ("GITHUB_API_URL", "https://github.example.com/api/v3"),
            ("GITHUB_TOKEN", "t"),
        ];
        let l = located(
            &publish(None, None),
            "https://github.example.com/team/app.git",
            &vars,
        )
        .unwrap();
        assert_eq!(l.kind, Kind::GitHub);
        assert_eq!(l.token_variable(), "GITHUB_TOKEN");
        assert_eq!(l.api.as_str(), "https://github.example.com/api/v3");
    }
}
