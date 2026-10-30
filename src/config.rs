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
    #[serde(default)]
    checks: Vec<String>,
    #[serde(default, rename = "upstream")]
    upstreams: Vec<RawUpstream>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawUpstream {
    name: String,
    remote: String,
    branch: String,
    url: Option<String>,
    path: Option<String>,
    mirror: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPublish {
    remote: String,
    #[serde(default)]
    via: PublishVia,
}

/// How `finish` publishes the verified update.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PublishVia {
    /// Push the integration and mirror branches.
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

/// Where and how `finish` publishes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Publish {
    pub remote: String,
    pub via: PublishVia,
}

/// A validated configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    pub integration: String,
    pub publish: Option<Publish>,
    pub install: Option<String>,
    pub checks: Vec<String>,
    pub upstreams: Vec<Upstream>,
}

/// One upstream: a source merged at the root or under its own path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Upstream {
    pub name: String,
    pub remote: String,
    pub branch: String,
    pub url: Option<String>,
    /// Normalised path inside the repository; `None` is the root.
    pub path: Option<String>,
    pub mirror: Option<String>,
}

impl Upstream {
    /// The upstream branch as `remote/branch`.
    pub fn source(&self) -> String {
        format!("{}/{}", self.remote, self.branch)
    }
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
    for (i, check) in raw.checks.iter().enumerate() {
        if check.trim().is_empty() {
            problems.push(format!("checks[{i}]: the command is empty"));
        }
    }
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
        check_remote(&format!("{label} remote"), &up.remote, &mut problems);
        check_branch(&format!("{label} branch"), &up.branch, &mut problems);
        if let Some(mirror) = &up.mirror {
            check_branch(&format!("{label} mirror"), mirror, &mut problems);
        }
        if let Some(url) = &up.url
            && url.trim().is_empty()
        {
            problems.push(format!("{label} url: the URL is empty"));
        }
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
        upstreams.push(Upstream {
            name: up.name,
            remote: up.remote,
            branch: up.branch,
            url: up.url,
            path,
            mirror: up.mirror,
        });
    }

    check_layout(
        &integration,
        publish.as_ref().map(|p| p.remote.as_str()),
        &upstreams,
        &mut problems,
    );

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
        checks: raw.checks,
        upstreams,
    })
}

/// Reads the `[publish]` table.
fn parse_publish(value: toml::Value) -> std::result::Result<Publish, String> {
    if let toml::Value::String(remote) = &value {
        return Err(format!(
            "publish: expected a table, not a string; write\n      [publish]\n      remote = {remote:?}\n    and add `via = \"merge-request\"` to publish through a merge request"
        ));
    }
    let raw: RawPublish = value
        .try_into()
        .map_err(|e: toml::de::Error| format!("publish: {}", e.message()))?;
    Ok(Publish {
        remote: raw.remote,
        via: raw.via,
    })
}

/// Cross-entry rules: unique names, remotes and mirrors, non-nesting paths,
/// a root upstream alone, no publishing to an upstream remote.
fn check_layout(
    integration: &str,
    publish: Option<&str>,
    upstreams: &[Upstream],
    problems: &mut Vec<String>,
) {
    duplicates(upstreams.iter().map(|u| u.name.as_str()), "name", problems);
    duplicates(
        upstreams.iter().map(|u| u.remote.as_str()),
        "remote",
        problems,
    );
    duplicates(
        upstreams.iter().filter_map(|u| u.mirror.as_deref()),
        "mirror branch",
        problems,
    );

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

    for up in upstreams {
        if let Some(mirror) = &up.mirror {
            if mirror == integration {
                problems.push(format!(
                    "upstream '{}' mirror '{mirror}' is the integration branch",
                    up.name
                ));
            }
            if mirror == crate::state::UPDATE_BRANCH {
                problems.push(format!(
                    "upstream '{}' mirror '{mirror}' is reserved for updates",
                    up.name
                ));
            }
        }
        if Some(up.remote.as_str()) == publish {
            problems.push(format!(
                "publish remote '{}' is the remote of upstream '{}'; the tool never pushes to an upstream",
                up.remote, up.name
            ));
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

    const UP_A: &str =
        "[[upstream]]\nname = \"a\"\nremote = \"ra\"\nbranch = \"main\"\npath = \"apps/a\"\n";
    const UP_B: &str =
        "[[upstream]]\nname = \"b\"\nremote = \"rb\"\nbranch = \"main\"\npath = \"apps/b\"\n";

    #[test]
    fn parses_full_configuration() {
        let cfg = parse(
            r#"
integration = "main"
install = "make deps"
checks = ["make lint", "make test"]

[[upstream]]
name = "app-a"
remote = "app-a"
branch = "main"
url = "git@example.com:team/app-a.git"
path = "/apps/app-a/"
mirror = "upstream-app-a"

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
                via: PublishVia::MergeRequest
            })
        );
        assert_eq!(cfg.checks.len(), 2);
        let up = &cfg.upstreams[0];
        assert_eq!(up.path.as_deref(), Some("apps/app-a"));
        assert_eq!(up.mirror.as_deref(), Some("upstream-app-a"));
        assert_eq!(up.source(), "app-a/main");
    }

    #[test]
    fn root_upstream_alone_is_valid() {
        let cfg = parse(
            "integration = \"main\"\n[[upstream]]\nname = \"up\"\nremote = \"upstream\"\nbranch = \"main\"\n",
        )
        .unwrap();
        assert_eq!(cfg.upstreams[0].path, None);
        assert!(cfg.checks.is_empty());
    }

    #[test]
    fn rejects_unknown_keys() {
        let e = err("integration = \"main\"\nfoo = 1\n");
        assert!(e.contains("unknown field `foo`"), "{e}");
        let e = err(
            "integration = \"main\"\n[[upstream]]\nname = \"a\"\nremote = \"a\"\nbranch = \"main\"\nsquash = true\n",
        );
        assert!(e.contains("unknown field `squash`"), "{e}");
    }

    #[test]
    fn rejects_missing_upstreams() {
        assert!(err("integration = \"main\"\n").contains("no [[upstream]]"));
    }

    #[test]
    fn rejects_duplicate_names_and_remotes() {
        let text = format!(
            "integration = \"main\"\n{UP_A}{}",
            UP_B.replace("\"b\"", "\"a\"").replace("\"rb\"", "\"ra\"")
        );
        let e = err(&text);
        assert!(e.contains("2 upstreams share the name 'a'"), "{e}");
        assert!(e.contains("2 upstreams share the remote 'ra'"), "{e}");
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
    fn rejects_publishing_to_an_upstream_remote() {
        let text = format!("integration = \"main\"\n{UP_A}[publish]\nremote = \"ra\"\n");
        assert!(err(&text).contains("never pushes to an upstream"));
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
    fn rejects_the_string_form_of_publish() {
        let e = err(&format!(
            "integration = \"main\"\npublish = \"origin\"\n{UP_A}"
        ));
        assert!(e.contains("[publish]"), "{e}");
        assert!(e.contains("remote = \"origin\""), "{e}");
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
            "integration = \"ma in\"\n[[upstream]]\nname = \"a b\"\nremote = \"r\"\nbranch = \"x..y\"\n",
        );
        assert!(
            e.contains("integration: 'ma in' is not a valid branch name"),
            "{e}"
        );
        assert!(e.contains("the name may contain only"), "{e}");
        assert!(e.contains("'x..y' is not a valid branch name"), "{e}");
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
