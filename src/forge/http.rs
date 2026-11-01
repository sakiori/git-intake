//! Requests to a forge's REST API over `ureq`: TLS through rustls with the
//! platform's certificate verifier, the proxies of the environment, and the
//! token attached only to requests for the API's own host.
//!
//! Redirects are followed here rather than by `ureq`, so that the token,
//! whichever header carries it, is never sent to another host: a redirect to
//! the API's host keeps it, a redirect elsewhere goes without it.

use std::fmt;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::Value;
use url::Url;

use crate::state::RELEASE;

/// How many redirects one request follows.
const MAX_REDIRECTS: usize = 5;

/// The most of a response body an error message quotes.
const QUOTED: usize = 400;

/// A token read from the environment. It is never shown: neither its
/// `Debug` nor any message of the tool prints it.
#[derive(Clone)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: String) -> Self {
        Self(value)
    }

    fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(..)")
    }
}

/// How a forge takes the token.
#[derive(Debug, Clone, Copy)]
pub enum Auth {
    /// `PRIVATE-TOKEN: <token>`, as GitLab takes a personal, project or
    /// group access token.
    PrivateToken,
    /// `Authorization: Bearer <token>`.
    Bearer,
    /// `JOB-TOKEN: <token>`, as GitLab takes a CI job's token.
    JobToken,
}

/// A request method the forge clients use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
    Put,
    Patch,
}

impl Method {
    fn as_str(self) -> &'static str {
        match self {
            Method::Get => "GET",
            Method::Post => "POST",
            Method::Put => "PUT",
            Method::Patch => "PATCH",
        }
    }
}

/// A response: its status and its body as JSON, `Null` when it is empty or
/// not JSON.
#[derive(Debug, Clone)]
pub struct Reply {
    pub status: u16,
    pub json: Value,
    text: String,
}

impl Reply {
    pub fn success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// What the forge said, for an error message: its `message` or `error`
    /// field, and the details GitHub lists under `errors`, or the start of
    /// the body.
    pub fn message(&self) -> String {
        let mut parts = Vec::new();
        for key in ["message", "error", "error_description"] {
            if let Some(v) = self.json.get(key) {
                parts.push(flatten(v));
            }
        }
        if let Some(Value::Array(errors)) = self.json.get("errors") {
            for e in errors {
                parts.push(match e.get("message") {
                    Some(m) => flatten(m),
                    None => flatten(e),
                });
            }
        }
        parts.retain(|p| !p.is_empty());
        if parts.is_empty() {
            let text = self.text.trim();
            let mut quoted: String = text.chars().take(QUOTED).collect();
            if quoted.len() < text.len() {
                quoted.push_str("...");
            }
            return quoted;
        }
        parts.join("; ")
    }
}

fn flatten(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(items) => items.iter().map(flatten).collect::<Vec<_>>().join("; "),
        Value::Object(map) => map
            .iter()
            .map(|(k, v)| format!("{k} {}", flatten(v)))
            .collect::<Vec<_>>()
            .join("; "),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// A client of one forge's API.
pub struct Client {
    agent: ureq::Agent,
    /// The API base; requests for its host carry the token.
    base: Url,
    /// `None` asks without credentials.
    token: Option<Secret>,
    auth: Auth,
    /// Headers every request carries, such as GitHub's API version.
    headers: Vec<(&'static str, &'static str)>,
    /// The forge's name for messages, such as `GitLab`.
    forge: &'static str,
}

impl Client {
    pub fn new(
        base: Url,
        token: Option<Secret>,
        auth: Auth,
        headers: Vec<(&'static str, &'static str)>,
        forge: &'static str,
    ) -> Self {
        let tls = ureq::tls::TlsConfig::builder()
            .root_certs(ureq::tls::RootCerts::PlatformVerifier)
            .build();
        // The default configuration reads the proxies of the environment.
        let agent = ureq::Agent::config_builder()
            .tls_config(tls)
            .http_status_as_error(false)
            .max_redirects(0)
            .user_agent(format!("git-intake/{RELEASE}"))
            .timeout_global(Some(Duration::from_secs(120)))
            .build()
            .new_agent();
        Self {
            agent,
            base,
            token,
            auth,
            headers,
            forge,
        }
    }

    pub fn base(&self) -> &Url {
        &self.base
    }

    /// Sends a request and returns the reply, whatever its status.
    pub fn send(&self, method: Method, url: &Url, body: Option<&Value>) -> Result<Reply> {
        let mut url = url.clone();
        let mut method = method;
        let mut body = body.cloned();
        for _ in 0..=MAX_REDIRECTS {
            let response = self.once(method, &url, body.as_ref())?;
            let status = response.status().as_u16();
            if !matches!(status, 301 | 302 | 303 | 307 | 308) {
                return read(response);
            }
            let location = response
                .headers()
                .get("location")
                .and_then(|v| v.to_str().ok())
                .map(str::to_string);
            let Some(location) = location else {
                return read(response);
            };
            let next = url.join(&location).with_context(|| {
                format!(
                    "the {} API redirected {} {} to an invalid address",
                    self.forge,
                    method.as_str(),
                    shown(&url)
                )
            })?;
            match (status, method) {
                (307 | 308, _) | (_, Method::Get) => {}
                (303, _) => {
                    method = Method::Get;
                    body = None;
                }
                _ => bail!(
                    "the {} API answered {} {} with {status}, a redirect that would turn it into another request; \
                     check the setting `api` of the publish remote",
                    self.forge,
                    method.as_str(),
                    shown(&url)
                ),
            }
            url = next;
        }
        bail!(
            "the {} API redirected {} more than {MAX_REDIRECTS} times",
            self.forge,
            shown(&url)
        )
    }

    /// Sends a request and returns its JSON body, refusing with the forge's
    /// status and message when it is not a success.
    pub fn json(&self, method: Method, url: &Url, body: Option<&Value>) -> Result<Value> {
        let reply = self.send(method, url, body)?;
        if !reply.success() {
            bail!("{}", self.refusal(method, url, &reply));
        }
        Ok(reply.json)
    }

    /// The message of a request the forge rejected: its status and what it
    /// said.
    pub fn refusal(&self, method: Method, url: &Url, reply: &Reply) -> String {
        let hint = match reply.status {
            401 => "; the token was not accepted",
            403 => "; the token lacks the access this needs",
            404 => {
                "; the project was not found, or the token cannot see it: check the publish remote's URL, \
                   the setting `api`, and the token's access to the project"
            }
            _ => "",
        };
        // The forge's message may quote what it was sent, the token as well.
        crate::redact::redacted(&format!(
            "the {} API answered {} {} with {} {}{hint}",
            self.forge,
            method.as_str(),
            shown(url),
            reply.status,
            reply.message()
        ))
    }

    /// Whether `url` may carry the token: it is on the API's host, by the
    /// same scheme or a stronger one.
    fn trusted(&self, url: &Url) -> bool {
        url.host_str() == self.base.host_str()
            && url.port_or_known_default() == self.base.port_or_known_default()
            && (url.scheme() == self.base.scheme() || url.scheme() == "https")
    }

    fn once(
        &self,
        method: Method,
        url: &Url,
        body: Option<&Value>,
    ) -> Result<ureq::http::Response<ureq::Body>> {
        let mut builder = ureq::http::Request::builder()
            .method(method.as_str())
            .uri(url.as_str());
        for (name, value) in &self.headers {
            builder = builder.header(*name, *value);
        }
        if let Some(token) = self.token.as_ref().filter(|_| self.trusted(url)) {
            builder = match self.auth {
                Auth::PrivateToken => builder.header("PRIVATE-TOKEN", token.expose()),
                Auth::JobToken => builder.header("JOB-TOKEN", token.expose()),
                Auth::Bearer => {
                    builder.header("Authorization", format!("Bearer {}", token.expose()))
                }
            };
        }
        let unreachable = |e: ureq::Error| {
            anyhow::anyhow!("cannot reach the {} API at {}: {e}", self.forge, shown(url))
        };
        let response = match body {
            Some(body) => {
                let request = builder
                    .header("Content-Type", "application/json")
                    .body(serde_json::to_vec(body)?)
                    .context("cannot build the request")?;
                self.agent.run(request).map_err(unreachable)?
            }
            None => {
                let request = builder.body(()).context("cannot build the request")?;
                self.agent.run(request).map_err(unreachable)?
            }
        };
        Ok(response)
    }
}

fn read(mut response: ureq::http::Response<ureq::Body>) -> Result<Reply> {
    let status = response.status().as_u16();
    let text = response
        .body_mut()
        .with_config()
        .limit(64 * 1024 * 1024)
        .read_to_string()
        .context("cannot read the forge's answer")?;
    let json = serde_json::from_str(&text).unwrap_or(Value::Null);
    Ok(Reply { status, json, text })
}

/// A URL as messages show it: without credentials, and without its query,
/// which may be long.
fn shown(url: &Url) -> String {
    let mut url = url.clone();
    url.set_query(None);
    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(json: &str) -> Reply {
        Reply {
            status: 422,
            json: serde_json::from_str(json).unwrap_or(Value::Null),
            text: json.to_string(),
        }
    }

    #[test]
    fn messages_come_from_the_forge_s_fields() {
        assert_eq!(
            reply(r#"{"message": ["Another open merge request already exists for this source branch: !7"]}"#)
                .message(),
            "Another open merge request already exists for this source branch: !7"
        );
        assert_eq!(
            reply(r#"{"message": "Validation Failed", "errors": [{"message": "A pull request already exists for team:git-intake/update."}]}"#)
                .message(),
            "Validation Failed; A pull request already exists for team:git-intake/update."
        );
        assert_eq!(
            reply(r#"{"message": {"title": ["is too long"]}}"#).message(),
            "title is too long"
        );
        assert_eq!(
            reply("<html>gateway</html>").message(),
            "<html>gateway</html>"
        );
    }

    #[test]
    fn a_secret_is_never_shown() {
        let s = Secret::new("glpat-very-secret".into());
        assert_eq!(format!("{s:?}"), "Secret(..)");
    }

    #[test]
    fn only_the_api_host_is_trusted_with_the_token() {
        let client = Client::new(
            Url::parse("https://gitlab.example.com/api/v4").unwrap(),
            Some(Secret::new("t".into())),
            Auth::PrivateToken,
            Vec::new(),
            "GitLab",
        );
        let trusted = |u: &str| client.trusted(&Url::parse(u).unwrap());
        assert!(trusted("https://gitlab.example.com/api/v4/projects/1"));
        assert!(trusted("https://gitlab.example.com:443/other"));
        assert!(!trusted("https://cdn.example.com/api/v4/projects/1"));
        assert!(!trusted("http://gitlab.example.com/api/v4/projects/1"));
        assert!(!trusted("https://gitlab.example.com:8443/api/v4"));
    }
}
