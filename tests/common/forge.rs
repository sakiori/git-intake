//! A fake GitLab or GitHub for the tests, served by a mockito server on the
//! loopback interface.
//!
//! The fake keeps merge requests in memory and answers the REST requests
//! the tool makes as the forge would, reading the head of an open merge
//! request from the branch in the publish remote's bare repository, as a
//! forge does after a push. Every request is recorded, with its headers, so
//! that tests can tell what the tool sent and where. The tool reaches the
//! fake as a forge reached from its own CI job: the publish remote's URL is
//! an SSH URL on the fake's host, which the test environment's SSH shim
//! serves from the bare repository, and the job's variables name the fake's
//! API.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

use mockito::{Matcher, Mock, Request, ServerGuard};
use serde_json::{Value, json};

/// The token the tests give the tool.
pub const TOKEN: &str = "test-token-never-printed";

/// The project on the fake forge.
pub const OWNER: &str = "team";
pub const REPO: &str = "app";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    GitLab,
    GitHub,
}

/// One merge request or pull request on the fake forge.
#[derive(Debug, Clone)]
pub struct FakeMr {
    pub number: u64,
    /// `open`, `merged` or `closed`.
    pub state: &'static str,
    pub source: String,
    pub target: String,
    pub title: String,
    pub description: String,
    /// Its head once it is merged or closed; an open one's head is its
    /// branch's tip.
    pub head: Option<String>,
    /// When it ended, as a count of the fake's clock.
    pub ended: Option<u64>,
    /// A head the forge reports while it is open instead of its branch's
    /// tip, as a forge that has not processed a push yet does.
    pub pinned_head: Option<String>,
    /// GitLab: what the request asked for.
    pub squash: Option<bool>,
    pub remove_source_branch: Option<bool>,
}

/// One request the tool made.
#[derive(Debug, Clone)]
pub struct Recorded {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl Recorded {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    pub fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or(Value::Null)
    }
}

/// What the fake forge holds.
pub struct World {
    /// The project (GitLab) or repository (GitHub) as `GET` returns it.
    pub project: Value,
    /// GitHub: the rules for the integration branch.
    pub rules: Value,
    /// GitHub: whether the integration branch has classic protection.
    pub protected: bool,
    /// GitHub: its classic protection, or the status reading it answers.
    pub protection: Result<Value, u16>,
    pub mrs: Vec<FakeMr>,
    pub requests: Vec<Recorded>,
    /// Requests answered with a failure: method, path suffix, status and
    /// message.
    pub failures: Vec<(String, String, u16, String)>,
    /// Requests answered with a redirect: method, path suffix and target.
    pub redirects: Vec<(String, String, String)>,
    /// How many lists of merge requests still leave out every merge
    /// request, as a list read just before another run opened one.
    pub stale_lists: usize,
    /// A merge request the forge merges once this file exists, as a
    /// person does while the tool runs.
    pub merge_when: Option<(u64, PathBuf)>,
    clock: u64,
}

pub struct FakeForge {
    pub kind: Kind,
    pub server: ServerGuard,
    pub world: Arc<Mutex<World>>,
    /// The bare repository the publish remote's URL is rewritten to.
    pub publish: PathBuf,
    _mocks: Vec<Mock>,
}

impl FakeForge {
    /// A GitLab project whose merge settings qualify: a successful pipeline
    /// required, skipped pipelines not counted, semi-linear history, no
    /// merge trains, squashing only on request.
    pub fn gitlab(publish: &Path) -> Self {
        Self::start(
            Kind::GitLab,
            publish,
            json!({
                "id": 1,
                "path_with_namespace": format!("{OWNER}/{REPO}"),
                "only_allow_merge_if_pipeline_succeeds": true,
                "allow_merge_on_skipped_pipeline": false,
                "merge_method": "rebase_merge",
                "squash_option": "default_off",
                "ci_config_path": null,
            }),
        )
    }

    /// A GitHub repository whose merge settings qualify: merge commits
    /// allowed, squash and rebase merges off, and a ruleset requiring a
    /// status check with the branch up to date.
    pub fn github(publish: &Path) -> Self {
        let f = Self::start(
            Kind::GitHub,
            publish,
            json!({
                "full_name": format!("{OWNER}/{REPO}"),
                "allow_merge_commit": true,
                "allow_squash_merge": false,
                "allow_rebase_merge": false,
            }),
        );
        f.world.lock().unwrap().rules = json!([{
            "type": "required_status_checks",
            "parameters": {
                "required_status_checks": [{"context": "ci"}],
                "strict_required_status_checks_policy": true
            }
        }]);
        f
    }

    fn start(kind: Kind, publish: &Path, project: Value) -> Self {
        let mut server = mockito::Server::new();
        let world = Arc::new(Mutex::new(World {
            project,
            rules: json!([]),
            protected: false,
            protection: Err(404),
            mrs: Vec::new(),
            requests: Vec::new(),
            failures: Vec::new(),
            redirects: Vec::new(),
            stale_lists: 0,
            merge_when: None,
            clock: 0,
        }));
        let mut mocks = Vec::new();
        for method in ["GET", "POST", "PUT", "PATCH", "DELETE"] {
            let slot: Arc<Mutex<Option<Reply>>> = Arc::new(Mutex::new(None));
            let (w, s, bare) = (world.clone(), slot.clone(), publish.to_path_buf());
            let host = server.url();
            let status = move |req: &Request| {
                let reply = answer(kind, &mut w.lock().unwrap(), req, &bare, &host);
                let code = reply.0;
                *s.lock().unwrap() = Some(reply);
                code as usize
            };
            let s2 = slot.clone();
            let location = move |_: &Request| {
                s2.lock()
                    .unwrap()
                    .as_ref()
                    .and_then(|(_, h, _)| h.iter().find(|(n, _)| n == "location"))
                    .map(|(_, v)| v.clone())
                    .unwrap_or_default()
            };
            let s3 = slot.clone();
            let body = move |_: &Request| {
                s3.lock()
                    .unwrap()
                    .take()
                    .map(|(_, _, b)| b)
                    .unwrap_or_default()
            };
            mocks.push(
                server
                    .mock(method, Matcher::Any)
                    .with_status_code_from_request(status)
                    .with_header("content-type", "application/json")
                    .with_header_from_request("location", location)
                    .with_body_from_request(body)
                    .create(),
            );
        }
        Self {
            kind,
            server,
            world,
            publish: publish.to_path_buf(),
            _mocks: mocks,
        }
    }

    /// The address of the forge, as its CI jobs know it.
    pub fn url(&self) -> String {
        self.server.url()
    }

    /// The publish remote's URL on the forge's host, which git reaches over
    /// SSH through the test environment's shim.
    pub fn remote_url(&self) -> String {
        let host = self.server.host_with_port();
        let host = host.rsplit_once(':').map_or(host.as_str(), |(h, _)| h);
        format!("ssh://{host}/{OWNER}/{REPO}.git")
    }

    /// The API base.
    pub fn api(&self) -> String {
        match self.kind {
            Kind::GitLab => format!("{}/api/v4", self.url()),
            Kind::GitHub => format!("{}/api/v3", self.url()),
        }
    }

    /// The variables of a CI job of this forge, with the token.
    pub fn job_vars(&self) -> Vec<(String, String)> {
        let vars: Vec<(&str, String)> = match self.kind {
            Kind::GitLab => vec![
                ("GITLAB_CI", "true".into()),
                ("CI_SERVER_URL", self.url()),
                ("CI_API_V4_URL", self.api()),
                ("GITLAB_TOKEN", TOKEN.into()),
            ],
            Kind::GitHub => vec![
                ("GITHUB_ACTIONS", "true".into()),
                ("GITHUB_SERVER_URL", self.url()),
                ("GITHUB_API_URL", self.api()),
                ("GITHUB_TOKEN", TOKEN.into()),
            ],
        };
        vars.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
    }

    pub fn world(&self) -> std::sync::MutexGuard<'_, World> {
        self.world.lock().unwrap()
    }

    /// Changes the project's settings.
    pub fn set(&self, key: &str, value: Value) {
        self.world().project[key] = value;
    }

    /// Removes a project setting, as GitHub hides the merge settings from a
    /// token without push access.
    pub fn unset(&self, key: &str) {
        if let Some(o) = self.world().project.as_object_mut() {
            o.remove(key);
        }
    }

    /// Answers requests whose path ends with `suffix` with `status`.
    pub fn fail(&self, method: &str, suffix: &str, status: u16, message: &str) {
        self.world()
            .failures
            .push((method.into(), suffix.into(), status, message.into()));
    }

    /// Answers requests whose path ends with `suffix` with a redirect to
    /// `target`.
    pub fn redirect(&self, method: &str, suffix: &str, target: &str) {
        self.world()
            .redirects
            .push((method.into(), suffix.into(), target.into()));
    }

    pub fn requests(&self) -> Vec<Recorded> {
        self.world().requests.clone()
    }

    /// The requests that change something on the forge.
    pub fn changes(&self) -> Vec<Recorded> {
        self.requests()
            .into_iter()
            .filter(|r| r.method != "GET")
            .collect()
    }

    pub fn clear_requests(&self) {
        self.world().requests.clear();
    }

    pub fn mrs(&self) -> Vec<FakeMr> {
        self.world().mrs.clone()
    }

    pub fn mr(&self, number: u64) -> FakeMr {
        self.mrs()
            .into_iter()
            .find(|m| m.number == number)
            .expect("merge request")
    }

    pub fn open_mrs(&self) -> Vec<FakeMr> {
        self.mrs()
            .into_iter()
            .filter(|m| m.state == "open")
            .collect()
    }

    /// Adds a merge request, as a person or another run opens it.
    pub fn add(&self, source: &str, target: &str, description: &str) -> u64 {
        let mut w = self.world();
        let number = w.mrs.iter().map(|m| m.number).max().unwrap_or(0) + 1;
        w.mrs.push(FakeMr {
            number,
            state: "open",
            source: source.into(),
            target: target.into(),
            title: "Merge upstream update".into(),
            description: description.into(),
            head: None,
            ended: None,
            pinned_head: None,
            squash: None,
            remove_source_branch: None,
        });
        number
    }

    /// Makes the forge report `head` as an open merge request's head.
    pub fn pin_head(&self, number: u64, head: &str) {
        let mut w = self.world();
        let mr = w.mrs.iter_mut().find(|m| m.number == number).unwrap();
        mr.pinned_head = Some(head.into());
    }

    /// Ends a merge request as `merged` or `closed`, keeping its head.
    pub fn end(&self, number: u64, state: &'static str) {
        let head = branch_tip(&self.publish, &self.mr(number).source);
        let mut w = self.world();
        w.clock += 1;
        let clock = w.clock;
        let mr = w.mrs.iter_mut().find(|m| m.number == number).unwrap();
        mr.state = state;
        mr.head = mr.head.clone().or(head);
        mr.ended = Some(clock);
    }

    /// Sets the description of a merge request, as a person edits it.
    pub fn describe(&self, number: u64, description: &str) {
        let mut w = self.world();
        let mr = w.mrs.iter_mut().find(|m| m.number == number).unwrap();
        mr.description = description.into();
    }
}

/// The tip of `branch` in the bare repository `bare`.
pub fn branch_tip(bare: &Path, branch: &str) -> Option<String> {
    let mut cmd = Command::new("git");
    for var in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_COMMON_DIR",
    ] {
        cmd.env_remove(var);
    }
    let out = cmd
        .arg("--git-dir")
        .arg(bare)
        .args([
            "rev-parse",
            "-q",
            "--verify",
            &format!("refs/heads/{branch}"),
        ])
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn time(clock: u64) -> String {
    format!(
        "2026-01-01T{:02}:{:02}:{:02}Z",
        clock / 3600,
        (clock / 60) % 60,
        clock % 60
    )
}

type Reply = (u16, Vec<(String, String)>, Vec<u8>);

/// The query of a request.
struct Query(Vec<(String, String)>);

impl Query {
    fn get(&self, key: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }
}

fn reply(status: u16, body: Value) -> Reply {
    (status, Vec::new(), body.to_string().into_bytes())
}

/// Answers one request as the fake forge.
fn answer(kind: Kind, w: &mut World, req: &Request, bare: &Path, host: &str) -> Reply {
    let full = req.path_and_query().to_string();
    let (path, query) = full.split_once('?').unwrap_or((full.as_str(), ""));
    let q = Query(
        url::form_urlencoded::parse(query.as_bytes())
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect(),
    );
    let body = req
        .utf8_lossy_body()
        .map(|b| b.into_owned())
        .unwrap_or_default();
    w.requests.push(Recorded {
        method: req.method().to_string(),
        path: full.clone(),
        headers: req
            .headers()
            .iter()
            .map(|(n, v)| (n.to_string(), v.to_str().unwrap_or_default().to_string()))
            .collect(),
        body: body.clone(),
    });
    let method = req.method().to_string();
    if let Some((number, file)) = w.merge_when.clone()
        && file.exists()
    {
        w.merge_when = None;
        w.clock += 1;
        let clock = w.clock;
        if let Some(mr) = w
            .mrs
            .iter_mut()
            .find(|m| m.number == number && m.state == "open")
        {
            mr.head = branch_tip(bare, &mr.source);
            mr.state = "merged";
            mr.ended = Some(clock);
        }
    }
    if let Some((_, _, target)) = w
        .redirects
        .iter()
        .find(|(m, s, _)| *m == method && path.ends_with(s.as_str()))
    {
        return (302, vec![("location".into(), target.clone())], Vec::new());
    }
    if let Some((_, _, status, message)) = w
        .failures
        .iter()
        .find(|(m, s, _, _)| *m == method && path.ends_with(s.as_str()))
    {
        return reply(*status, json!({ "message": message }));
    }
    let body: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
    match kind {
        Kind::GitLab => gitlab(w, &method, path, &q, &body, bare, host),
        Kind::GitHub => github(w, &method, path, &q, &body, bare, host),
    }
}

fn head_of(mr: &FakeMr, bare: &Path) -> String {
    match (&mr.head, mr.state) {
        (Some(h), s) if s != "open" => h.clone(),
        (_, "open") if mr.pinned_head.is_some() => mr.pinned_head.clone().unwrap_or_default(),
        _ => branch_tip(bare, &mr.source)
            .or_else(|| mr.head.clone())
            .unwrap_or_default(),
    }
}

fn gitlab_mr(mr: &FakeMr, bare: &Path, host: &str) -> Value {
    json!({
        "iid": mr.number,
        "web_url": format!("{host}/{OWNER}/{REPO}/-/merge_requests/{}", mr.number),
        "state": if mr.state == "open" { "opened" } else { mr.state },
        "sha": head_of(mr, bare),
        "source_branch": mr.source,
        "target_branch": mr.target,
        "title": mr.title,
        "description": mr.description,
        "merged_at": (mr.state == "merged").then(|| time(mr.ended.unwrap_or(0))),
        "closed_at": (mr.state == "closed").then(|| time(mr.ended.unwrap_or(0))),
    })
}

fn gitlab(
    w: &mut World,
    method: &str,
    path: &str,
    q: &Query,
    body: &Value,
    bare: &Path,
    host: &str,
) -> Reply {
    let project = format!("/api/v4/projects/{OWNER}%2F{REPO}");
    let Some(rest) = path.strip_prefix(&project) else {
        return reply(404, json!({"message": "404 Project Not Found"}));
    };
    let parts: Vec<&str> = rest.split('/').filter(|p| !p.is_empty()).collect();
    match (method, parts.as_slice()) {
        ("GET", []) => reply(200, w.project.clone()),
        ("GET", ["merge_requests"]) => {
            if w.stale_lists > 0 {
                w.stale_lists -= 1;
                return reply(200, json!([]));
            }
            let mut list: Vec<&FakeMr> = w
                .mrs
                .iter()
                .filter(|m| q.get("source_branch").is_none_or(|s| m.source == s))
                .filter(|m| q.get("target_branch").is_none_or(|t| m.target == t))
                .filter(|m| match q.get("state") {
                    Some("opened") => m.state == "open",
                    Some("merged") => m.state == "merged",
                    Some("closed") => m.state == "closed",
                    _ => true,
                })
                .collect();
            list.sort_by_key(|m| std::cmp::Reverse(m.number));
            reply(
                200,
                Value::Array(list.iter().map(|m| gitlab_mr(m, bare, host)).collect()),
            )
        }
        ("GET", ["merge_requests", n]) => match w.mrs.iter().find(|m| m.number.to_string() == *n) {
            Some(m) => reply(200, gitlab_mr(m, bare, host)),
            None => reply(404, json!({"message": "404 Not found"})),
        },
        ("POST", ["merge_requests"]) => {
            let source = body["source_branch"]
                .as_str()
                .unwrap_or_default()
                .to_string();
            let target = body["target_branch"]
                .as_str()
                .unwrap_or_default()
                .to_string();
            if let Some(open) = w
                .mrs
                .iter()
                .find(|m| m.state == "open" && m.source == source && m.target == target)
            {
                return reply(
                    409,
                    json!({"message": [format!(
                        "Another open merge request already exists for this source branch: !{}",
                        open.number
                    )]}),
                );
            }
            let number = w.mrs.iter().map(|m| m.number).max().unwrap_or(0) + 1;
            let mr = FakeMr {
                number,
                state: "open",
                source,
                target,
                title: body["title"].as_str().unwrap_or_default().into(),
                description: body["description"].as_str().unwrap_or_default().into(),
                head: None,
                ended: None,
                pinned_head: None,
                squash: body["squash"].as_bool(),
                remove_source_branch: body["remove_source_branch"].as_bool(),
            };
            let json = gitlab_mr(&mr, bare, host);
            w.mrs.push(mr);
            reply(201, json)
        }
        ("PUT", ["merge_requests", n]) => {
            w.clock += 1;
            let clock = w.clock;
            let Some(mr) = w.mrs.iter_mut().find(|m| m.number.to_string() == *n) else {
                return reply(404, json!({"message": "404 Not found"}));
            };
            if let Some(t) = body["title"].as_str() {
                mr.title = t.into();
            }
            if let Some(d) = body["description"].as_str() {
                mr.description = d.into();
            }
            if let Some(s) = body["squash"].as_bool() {
                mr.squash = Some(s);
            }
            if body["state_event"] == "close" && mr.state == "open" {
                mr.head = branch_tip(bare, &mr.source);
                mr.state = "closed";
                mr.ended = Some(clock);
            }
            reply(200, gitlab_mr(mr, bare, host))
        }
        _ => reply(404, json!({"message": "404 Not Found"})),
    }
}

fn github_pr(mr: &FakeMr, bare: &Path, host: &str) -> Value {
    json!({
        "number": mr.number,
        "html_url": format!("{host}/{OWNER}/{REPO}/pull/{}", mr.number),
        "state": if mr.state == "open" { "open" } else { "closed" },
        "merged_at": (mr.state == "merged").then(|| time(mr.ended.unwrap_or(0))),
        "closed_at": (mr.state != "open").then(|| time(mr.ended.unwrap_or(0))),
        "head": {"sha": head_of(mr, bare), "ref": mr.source},
        "base": {"ref": mr.target},
        "title": mr.title,
        "body": mr.description,
    })
}

fn github(
    w: &mut World,
    method: &str,
    path: &str,
    q: &Query,
    body: &Value,
    bare: &Path,
    host: &str,
) -> Reply {
    let repository = format!("/api/v3/repos/{OWNER}/{REPO}");
    let Some(rest) = path.strip_prefix(&repository) else {
        return reply(404, json!({"message": "Not Found"}));
    };
    let parts: Vec<&str> = rest.split('/').filter(|p| !p.is_empty()).collect();
    match (method, parts.as_slice()) {
        ("GET", []) => reply(200, w.project.clone()),
        ("GET", ["rules", "branches", _]) => reply(200, w.rules.clone()),
        ("GET", ["branches", b]) => reply(200, json!({"name": b, "protected": w.protected})),
        ("GET", ["branches", _, "protection"]) => match &w.protection {
            Ok(p) => reply(200, p.clone()),
            Err(status) => reply(
                *status,
                json!({"message": "Resource not accessible by integration"}),
            ),
        },
        ("GET", ["pulls"]) => {
            if w.stale_lists > 0 {
                w.stale_lists -= 1;
                return reply(200, json!([]));
            }
            let head = q
                .get("head")
                .map(|h| h.split_once(':').map_or(h, |(_, b)| b).to_string());
            let mut list: Vec<&FakeMr> = w
                .mrs
                .iter()
                .filter(|m| head.as_deref().is_none_or(|h| m.source == h))
                .filter(|m| q.get("base").is_none_or(|b| m.target == b))
                .filter(|m| match q.get("state") {
                    Some("closed") => m.state != "open",
                    Some("all") => true,
                    _ => m.state == "open",
                })
                .collect();
            list.sort_by_key(|m| std::cmp::Reverse(m.number));
            reply(
                200,
                Value::Array(list.iter().map(|m| github_pr(m, bare, host)).collect()),
            )
        }
        ("GET", ["pulls", n]) => match w.mrs.iter().find(|m| m.number.to_string() == *n) {
            Some(m) => reply(200, github_pr(m, bare, host)),
            None => reply(404, json!({"message": "Not Found"})),
        },
        ("POST", ["pulls"]) => {
            let source = body["head"].as_str().unwrap_or_default().to_string();
            let target = body["base"].as_str().unwrap_or_default().to_string();
            if w.mrs
                .iter()
                .any(|m| m.state == "open" && m.source == source && m.target == target)
            {
                return reply(
                    422,
                    json!({"message": "Validation Failed", "errors": [{
                        "resource": "PullRequest", "code": "custom",
                        "message": format!("A pull request already exists for {OWNER}:{source}.")
                    }]}),
                );
            }
            let number = w.mrs.iter().map(|m| m.number).max().unwrap_or(0) + 1;
            let mr = FakeMr {
                number,
                state: "open",
                source,
                target,
                title: body["title"].as_str().unwrap_or_default().into(),
                description: body["body"].as_str().unwrap_or_default().into(),
                head: None,
                ended: None,
                pinned_head: None,
                squash: None,
                remove_source_branch: None,
            };
            let json = github_pr(&mr, bare, host);
            w.mrs.push(mr);
            reply(201, json)
        }
        ("PATCH", ["pulls", n]) => {
            w.clock += 1;
            let clock = w.clock;
            let Some(mr) = w.mrs.iter_mut().find(|m| m.number.to_string() == *n) else {
                return reply(404, json!({"message": "Not Found"}));
            };
            if let Some(t) = body["title"].as_str() {
                mr.title = t.into();
            }
            if let Some(d) = body["body"].as_str() {
                mr.description = d.into();
            }
            if body["state"] == "closed" && mr.state == "open" {
                mr.head = branch_tip(bare, &mr.source);
                mr.state = "closed";
                mr.ended = Some(clock);
            }
            reply(200, github_pr(mr, bare, host))
        }
        _ => reply(404, json!({"message": "Not Found"})),
    }
}
