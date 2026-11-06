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
    /// Whether it comes from a fork, whose branch is not the project's.
    pub fork: bool,
    /// How many more reads report `pinned_head`, as a forge that has not
    /// processed a push yet does; `None` for all of them.
    pub lagging_reads: Option<usize>,
    /// GitLab: what the request asked for.
    pub squash: Option<bool>,
    pub remove_source_branch: Option<bool>,
    /// GitLab: what keeps it from merging, whatever the branches say, as a
    /// `detailed_merge_status`.
    pub blocked: Option<&'static str>,
    /// GitLab: why the last merge failed.
    pub merge_error: Option<&'static str>,
    /// The ids of its assignees.
    pub assignees: Vec<u64>,
}

/// One note on a merge request of the fake forge.
#[derive(Debug, Clone)]
pub struct FakeNote {
    pub number: u64,
    pub body: String,
    /// Whether the forge wrote it, as it records an assignment.
    pub system: bool,
    /// When, as a count of the fake's clock.
    pub at: u64,
}

/// Something that happens on the fake forge at a step of a pipeline
/// script, as people and a merge queue act while the tool waits.
#[derive(Debug, Clone)]
pub enum Action {
    /// Points a reference of the publish remote at a commit.
    SetRef(String, String),
    /// Removes a reference of the publish remote.
    DeleteRef(String),
    /// Merges a merge request, by fast-forward when its head contains its
    /// target, as a queue does.
    Merge(u64),
    /// Assigns a merge request to an account, keeping its assignees.
    Assign(u64, u64),
    /// Answers the next requests whose path ends with a suffix with a
    /// failure: method, path suffix, status and how many.
    FailTimes(String, String, u16, usize),
    /// Creates a file, as the test environment's SSH shim reads one.
    Touch(PathBuf),
    /// Writes a file, as the test environment's SSH shim reads one.
    Write(PathBuf, String),
    /// Removes that file.
    Remove(PathBuf),
    /// Hides the project from the token, which GitLab and GitHub then answer
    /// with 404 on every request of it, as they do when it loses access.
    Hide,
    /// Sends a signal, such as `TERM`, to a process the test started.
    Signal(u32, &'static str),
    /// Makes actions happen once the next request with a method and a path
    /// suffix has been answered, as [`FakeForge::on_request`].
    OnRequest(String, String, Vec<Action>),
}

/// One read of a pipeline: its state, or `None` before it exists, and what
/// happens when it is read.
#[derive(Debug, Clone)]
pub struct Step {
    pub state: Option<&'static str>,
    pub then: Vec<Action>,
    /// What happens once the read has been answered, as a person acts just
    /// after the tool read.
    pub after: Vec<Action>,
}

impl Step {
    pub fn is(state: &'static str) -> Self {
        Self {
            state: Some(state),
            then: Vec::new(),
            after: Vec::new(),
        }
    }

    pub fn none() -> Self {
        Self {
            state: None,
            then: Vec::new(),
            after: Vec::new(),
        }
    }

    pub fn then(mut self, action: Action) -> Self {
        self.then.push(action);
        self
    }

    pub fn after(mut self, action: Action) -> Self {
        self.after.push(action);
        self
    }
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
    /// GitHub: its classic protection, or the status and message reading it
    /// answers.
    pub protection: Result<Value, (u16, &'static str)>,
    pub mrs: Vec<FakeMr>,
    pub requests: Vec<Recorded>,
    /// Requests answered with a failure: method, path suffix, status and
    /// message.
    pub failures: Vec<(String, String, u16, String)>,
    /// Requests answered with a failure a number of times: method, path
    /// suffix, status and how many times more.
    pub flaky: Vec<(String, String, u16, usize)>,
    /// What happens once the current request has been answered.
    pub after: Vec<Action>,
    /// What happens once the next request with a method and a path suffix
    /// has been answered.
    pub on_request: Vec<(String, String, Vec<Action>)>,
    /// Whether the project is hidden from the token.
    pub hidden: bool,
    /// Requests answered with a redirect: method, path suffix and target.
    pub redirects: Vec<(String, String, String)>,
    /// How many lists of merge requests still leave out every merge
    /// request, as a list read just before another run opened one.
    pub stale_lists: usize,
    /// A merge request the forge merges once this file exists, as a
    /// person does while the tool runs.
    pub merge_when: Option<(u64, PathBuf)>,
    /// GitHub: the repository's name as GitHub reports it, which differs
    /// from the remote's path once the repository was renamed.
    pub full_name: String,
    /// The tags that have a release on the forge.
    pub releases: Vec<String>,
    /// Whether the project is shown only to a request with a credential.
    pub private: bool,
    /// The forge's accounts: id and name.
    pub users: Vec<(u64, String)>,
    pub notes: Vec<FakeNote>,
    /// The pipeline of each commit, read by read: each read takes the next
    /// step, and the last one stays.
    pub pipelines: std::collections::HashMap<String, std::collections::VecDeque<Step>>,
    /// The pipelines of the heads of the merge requests from a branch that
    /// have no script of their own, read by read, as for a head pushed
    /// while the tool runs.
    pub branch_pipelines: std::collections::HashMap<String, std::collections::VecDeque<Step>>,
    /// The pipeline state of a commit without a script; `None` for no
    /// pipeline.
    pub default_pipeline: Option<&'static str>,
    /// GitHub: check runs of every commit besides the scripted one: name,
    /// status and conclusion.
    pub check_runs: Vec<(String, &'static str, Option<&'static str>)>,
    /// GitHub: the check `ci` of each commit as last read.
    pub read_checks: std::collections::HashMap<String, Option<&'static str>>,
    /// GitLab: how the next requests to merge are answered: status, message
    /// and the `detailed_merge_status` reads show afterwards.
    pub merge_answers: std::collections::VecDeque<(u16, &'static str, &'static str)>,
    /// A status reads show since the last of `merge_answers`.
    pub merge_status: Option<&'static str>,
    /// What happens when a request to merge arrives, before it is judged.
    pub on_merge: Vec<Action>,
    /// Each change of a merge request's assignees: its number, and the
    /// message of the commit `refs/git-intake/pause` pointed at then.
    pub assignments: Vec<(u64, Option<String>)>,
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
            protection: Err((404, "Branch not protected")),
            mrs: Vec::new(),
            requests: Vec::new(),
            failures: Vec::new(),
            flaky: Vec::new(),
            after: Vec::new(),
            on_request: Vec::new(),
            hidden: false,
            redirects: Vec::new(),
            stale_lists: 0,
            merge_when: None,
            full_name: format!("{OWNER}/{REPO}"),
            releases: Vec::new(),
            private: false,
            users: Vec::new(),
            notes: Vec::new(),
            pipelines: std::collections::HashMap::new(),
            branch_pipelines: std::collections::HashMap::new(),
            default_pipeline: Some("success"),
            check_runs: Vec::new(),
            read_checks: std::collections::HashMap::new(),
            merge_answers: std::collections::VecDeque::new(),
            merge_status: None,
            on_merge: Vec::new(),
            assignments: Vec::new(),
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

    /// Hides the project from the token, as for [`Action::Hide`].
    pub fn hide(&self) {
        self.world().hidden = true;
    }

    /// Makes `actions` happen once the next request with `method` whose path
    /// ends with `suffix` has been answered.
    pub fn on_request(&self, method: &str, suffix: &str, actions: Vec<Action>) {
        self.world()
            .on_request
            .push((method.into(), suffix.into(), actions));
    }

    /// Answers the next `times` requests whose path ends with `suffix` with
    /// `status`.
    pub fn fail_times(&self, method: &str, suffix: &str, status: u16, times: usize) {
        self.world()
            .flaky
            .push((method.into(), suffix.into(), status, times));
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
            lagging_reads: None,
            fork: false,
            squash: None,
            remove_source_branch: None,
            blocked: None,
            merge_error: None,
            assignees: Vec::new(),
        });
        number
    }

    /// Adds a merge request from a fork's branch of the name `source`, as
    /// anyone may open; the fork's head is `head`.
    pub fn add_from_fork(&self, source: &str, target: &str, description: &str, head: &str) -> u64 {
        let number = self.add(source, target, description);
        let mut w = self.world();
        let mr = w.mrs.iter_mut().find(|m| m.number == number).unwrap();
        mr.fork = true;
        mr.pinned_head = Some(head.into());
        number
    }

    /// Makes the forge report `head` as an open merge request's head.
    pub fn pin_head(&self, number: u64, head: &str) {
        let mut w = self.world();
        let mr = w.mrs.iter_mut().find(|m| m.number == number).unwrap();
        mr.pinned_head = Some(head.into());
    }

    /// Makes the forge report `head` as an open merge request's head for the
    /// next `reads` reads of it, and its branch's tip from then on.
    pub fn lag_head(&self, number: u64, head: &str, reads: usize) {
        let mut w = self.world();
        let mr = w.mrs.iter_mut().find(|m| m.number == number).unwrap();
        mr.pinned_head = Some(head.into());
        mr.lagging_reads = Some(reads);
    }

    /// Answers the next requests to merge with these: status, message and
    /// the `detailed_merge_status` reads show afterwards.
    pub fn answer_merges(&self, answers: &[(u16, &'static str, &'static str)]) {
        self.world().merge_answers.extend(answers.iter().copied());
    }

    /// Runs `action` when the next request to merge arrives.
    pub fn on_merge(&self, action: Action) {
        self.world().on_merge.push(action);
    }

    /// Changes a merge request on the forge.
    pub fn edit(&self, number: u64, change: impl FnOnce(&mut FakeMr)) {
        let mut w = self.world();
        change(w.mrs.iter_mut().find(|m| m.number == number).unwrap());
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

    /// Creates the forge release of `tag`, as a release pipeline's last job
    /// does.
    pub fn publish_release(&self, tag: &str) {
        self.world().releases.push(tag.to_string());
    }

    /// Adds an account to the forge.
    pub fn add_user(&self, id: u64, username: &str) {
        self.world().users.push((id, username.to_string()));
    }

    /// Assigns merge request `number` to the account `id` besides its
    /// assignees, as a person queues it, with the note the forge writes.
    pub fn assign(&self, number: u64, id: u64) {
        let bare = self.publish.clone();
        act(&mut self.world(), &bare, &Action::Assign(number, id));
    }

    /// The notes of merge request `number`, oldest first.
    pub fn notes(&self, number: u64) -> Vec<FakeNote> {
        self.world()
            .notes
            .iter()
            .filter(|n| n.number == number)
            .cloned()
            .collect()
    }

    /// Adds a note to merge request `number`, as a person or another run
    /// writes it.
    pub fn add_note(&self, number: u64, body: &str) {
        let mut w = self.world();
        w.clock += 1;
        let at = w.clock;
        w.notes.push(FakeNote {
            number,
            body: body.into(),
            system: false,
            at,
        });
    }

    /// Scripts the pipeline of `commit`, read by read.
    pub fn script(&self, commit: &str, steps: Vec<Step>) {
        self.world()
            .pipelines
            .insert(commit.to_string(), steps.into_iter().collect());
    }

    /// Scripts the pipelines of the heads of the merge requests from
    /// `branch` that have no script of their own, read by read.
    pub fn script_branch(&self, branch: &str, steps: Vec<Step>) {
        self.world()
            .branch_pipelines
            .insert(branch.to_string(), steps.into_iter().collect());
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
    bare_git(
        bare,
        &[
            "rev-parse",
            "-q",
            "--verify",
            &format!("refs/heads/{branch}"),
        ],
    )
}

/// Runs git on the bare repository `bare`; its output, trimmed, or `None`
/// when it fails.
fn bare_git(bare: &Path, args: &[&str]) -> Option<String> {
    let mut cmd = Command::new("git");
    for var in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_COMMON_DIR",
    ] {
        cmd.env_remove(var);
    }
    // The forge commits merges as itself, whatever the machine running the tests has configured.
    cmd.env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "Fake Forge")
        .env("GIT_AUTHOR_EMAIL", "forge@example.com")
        .env("GIT_COMMITTER_NAME", "Fake Forge")
        .env("GIT_COMMITTER_EMAIL", "forge@example.com");
    let out = cmd.arg("--git-dir").arg(bare).args(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Whether `a` is an ancestor of `b` in the bare repository.
fn contains(bare: &Path, b: &str, a: &str) -> bool {
    bare_git(bare, &["merge-base", "--is-ancestor", a, b]).is_some()
}

fn username(w: &World, id: u64) -> String {
    w.users
        .iter()
        .find(|(i, _)| *i == id)
        .map(|(_, n)| n.clone())
        .unwrap_or_else(|| format!("user{id}"))
}

/// Sets a merge request's assignees, with the notes the forge writes.
fn set_assignees(w: &mut World, number: u64, assignees: Vec<u64>) {
    let Some(before) = w
        .mrs
        .iter()
        .find(|m| m.number == number)
        .map(|m| m.assignees.clone())
    else {
        return;
    };
    let added: Vec<String> = assignees
        .iter()
        .filter(|a| !before.contains(a))
        .map(|a| format!("@{}", username(w, *a)))
        .collect();
    let removed: Vec<String> = before
        .iter()
        .filter(|a| !assignees.contains(a))
        .map(|a| format!("@{}", username(w, *a)))
        .collect();
    let mut parts = Vec::new();
    if !added.is_empty() {
        parts.push(format!("assigned to {}", added.join(" and ")));
    }
    if !removed.is_empty() {
        parts.push(format!("unassigned {}", removed.join(" and ")));
    }
    if let Some(mr) = w.mrs.iter_mut().find(|m| m.number == number) {
        mr.assignees = assignees;
    }
    if !parts.is_empty() {
        w.clock += 1;
        let at = w.clock;
        w.notes.push(FakeNote {
            number,
            body: parts.join(" and "),
            system: true,
            at,
        });
    }
}

/// Merges an open merge request into its target as the project's merge
/// method does: by fast-forward with `ff`, and otherwise with a merge commit,
/// or with one squashed commit when the merge request asks for squashing and
/// the project lets it. Returns whether it merged.
fn merge_mr(w: &mut World, bare: &Path, number: u64, remove_source: bool) -> bool {
    let Some(mr) = w
        .mrs
        .iter()
        .find(|m| m.number == number && m.state == "open")
        .cloned()
    else {
        return false;
    };
    let head = head_of(&mr, bare);
    let target_ref = format!("refs/heads/{}", mr.target);
    let Some(target) = bare_git(bare, &["rev-parse", &target_ref]) else {
        return false;
    };
    let squashed = mr.squash == Some(true) && w.project["squash_option"] != "never";
    let fast_forward = w.project["merge_method"] == "ff" && contains(bare, &head, &target);
    let merged = if squashed {
        bare_git(
            bare,
            &[
                "commit-tree",
                &format!("{head}^{{tree}}"),
                "-p",
                &target,
                "-m",
                &format!("Squashed '{}'", mr.source),
            ],
        )
    } else if fast_forward {
        Some(head.clone())
    } else {
        bare_git(
            bare,
            &[
                "commit-tree",
                &format!("{head}^{{tree}}"),
                "-p",
                &target,
                "-p",
                &head,
                "-m",
                &format!("Merge branch '{}'", mr.source),
            ],
        )
    };
    let Some(merged) = merged else {
        return false;
    };
    bare_git(bare, &["update-ref", &target_ref, &merged]);
    if remove_source {
        bare_git(
            bare,
            &["update-ref", "-d", &format!("refs/heads/{}", mr.source)],
        );
    }
    w.clock += 1;
    let clock = w.clock;
    if let Some(m) = w.mrs.iter_mut().find(|m| m.number == number) {
        m.head = Some(head);
        m.state = "merged";
        m.ended = Some(clock);
    }
    true
}

fn act(w: &mut World, bare: &Path, action: &Action) {
    match action {
        Action::SetRef(name, commit) => {
            bare_git(bare, &["update-ref", name, commit]);
        }
        Action::DeleteRef(name) => {
            bare_git(bare, &["update-ref", "-d", name]);
        }
        Action::FailTimes(method, suffix, status, times) => {
            w.flaky
                .push((method.clone(), suffix.clone(), *status, *times));
        }
        Action::Touch(path) => {
            std::fs::write(path, "").unwrap();
        }
        Action::Write(path, text) => {
            std::fs::write(path, text).unwrap();
        }
        Action::Remove(path) => {
            std::fs::remove_file(path).unwrap();
        }
        Action::Hide => w.hidden = true,
        Action::OnRequest(method, suffix, actions) => {
            w.on_request
                .push((method.clone(), suffix.clone(), actions.clone()));
        }
        Action::Signal(pid, signal) => {
            let sent = std::process::Command::new("kill")
                .args([format!("-{signal}"), pid.to_string()])
                .status()
                .unwrap();
            assert!(sent.success());
        }
        Action::Merge(number) => {
            merge_mr(w, bare, *number, true);
        }
        Action::Assign(number, id) => {
            let mut assignees = w
                .mrs
                .iter()
                .find(|m| m.number == *number)
                .map(|m| m.assignees.clone())
                .unwrap_or_default();
            if !assignees.contains(id) {
                assignees.push(*id);
            }
            set_assignees(w, *number, assignees);
        }
    }
}

/// Reads the pipeline of `commit` once: the state of the next step of its
/// script, after the step's actions.
fn read_pipeline(
    w: &mut World,
    bare: &Path,
    commit: &str,
    branch: Option<&str>,
) -> Option<&'static str> {
    let script = match w.pipelines.contains_key(commit) {
        true => w.pipelines.get_mut(commit),
        false => branch.and_then(|b| w.branch_pipelines.get_mut(b)),
    };
    let step = match script {
        None => return w.default_pipeline,
        Some(steps) if steps.len() > 1 => steps.pop_front(),
        Some(steps) => steps.front_mut().map(|s| Step {
            state: s.state,
            then: std::mem::take(&mut s.then),
            after: std::mem::take(&mut s.after),
        }),
    }?;
    for action in &step.then {
        act(w, bare, action);
    }
    w.after.extend(step.after);
    step.state
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

/// The page of a list a request asks for, as both forges cut it.
fn page<T: Clone>(list: &[T], q: &Query) -> Vec<T> {
    let per_page: usize = q.get("per_page").and_then(|p| p.parse().ok()).unwrap_or(20);
    let page: usize = q.get("page").and_then(|p| p.parse().ok()).unwrap_or(1);
    list.iter()
        .skip(per_page * (page - 1))
        .take(per_page)
        .cloned()
        .collect()
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
    let hidden = [
        format!("/api/v4/projects/{OWNER}%2F{REPO}"),
        format!("/api/v3/repos/{OWNER}/{REPO}"),
    ];
    if w.hidden && hidden.iter().any(|p| path.starts_with(p.as_str())) {
        return reply(404, json!({"message": "404 Project Not Found"}));
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
    if let Some(flaky) = w
        .flaky
        .iter_mut()
        .find(|(m, s, _, times)| *m == method && path.ends_with(s.as_str()) && *times > 0)
    {
        flaky.3 -= 1;
        let status = flaky.2;
        return reply(
            status,
            json!({ "message": format!("{status} from the forge") }),
        );
    }
    let body: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
    let credential = req
        .headers()
        .iter()
        .any(|(n, _)| ["private-token", "job-token", "authorization"].contains(&n.as_str()));
    if w.private && !credential {
        return reply(404, json!({"message": "404 Project Not Found"}));
    }
    if let Some(at) = w
        .on_request
        .iter()
        .position(|(m, s, _)| *m == method && path.ends_with(s.as_str()))
    {
        let (_, _, actions) = w.on_request.remove(at);
        w.after.extend(actions);
    }
    let answered = match kind {
        Kind::GitLab => gitlab(w, &method, path, &q, &body, bare, host),
        Kind::GitHub => github(w, &method, path, &q, &body, bare, host),
    };
    for action in std::mem::take(&mut w.after) {
        act(w, bare, &action);
    }
    answered
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
        "source_project_id": if mr.fork { 2 } else { 1 },
        "target_project_id": 1,
        "target_branch": mr.target,
        "title": mr.title,
        "description": mr.description,
        "merged_at": (mr.state == "merged").then(|| time(mr.ended.unwrap_or(0))),
        "closed_at": (mr.state == "closed").then(|| time(mr.ended.unwrap_or(0))),
        "assignees": mr.assignees.iter().map(|a| json!({"id": a})).collect::<Vec<_>>(),
    })
}

/// A merge request as GitLab answers a request for it alone: with its
/// assignees, the pipeline of its head, which counts as a read of it, and
/// how far its head is behind its target.
fn gitlab_single(w: &mut World, number: u64, bare: &Path, host: &str) -> Option<Value> {
    let (head, source) = w
        .mrs
        .iter()
        .find(|m| m.number == number)
        .map(|m| (head_of(m, bare), m.source.clone()))?;
    let pipeline = if head.is_empty() {
        None
    } else {
        read_pipeline(w, bare, &head, Some(&source))
    };
    // What the read set off shows in the same answer.
    let mr = w.mrs.iter().find(|m| m.number == number)?.clone();
    if let Some(m) = w.mrs.iter_mut().find(|m| m.number == number)
        && let Some(reads) = &mut m.lagging_reads
    {
        *reads = reads.saturating_sub(1);
        if *reads == 0 {
            m.lagging_reads = None;
            m.pinned_head = None;
        }
    }
    let mut v = gitlab_mr(&mr, bare, host);
    v["head_pipeline"] = match pipeline {
        Some(status) => json!({
            "id": 100 + mr.number,
            "sha": head,
            "status": status,
            "web_url": format!("{host}/{OWNER}/{REPO}/-/pipelines/{}", 100 + mr.number),
        }),
        None => Value::Null,
    };
    v["assignees"] = Value::Array(
        mr.assignees
            .iter()
            .map(|a| json!({"id": a, "username": username(w, *a)}))
            .collect(),
    );
    let current = bare_git(bare, &["rev-parse", &format!("refs/heads/{}", mr.target)])
        .is_some_and(|target| contains(bare, &head, &target));
    v["diverged_commits_count"] = json!(if current { 0 } else { 1 });
    v["detailed_merge_status"] = json!(match (w.merge_status, mr.blocked, current) {
        (Some(status), _, _) => status,
        (None, Some(blocked), _) => blocked,
        (None, None, true) => "mergeable",
        (None, None, false) => "need_rebase",
    });
    v["squash"] = json!(mr.squash == Some(true));
    v["merge_error"] = json!(mr.merge_error);
    Some(v)
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
    if (method, path) == ("GET", "/api/v4/users") {
        let name = q.get("username").unwrap_or_default();
        return reply(
            200,
            Value::Array(
                w.users
                    .iter()
                    .filter(|(_, n)| n == name)
                    .map(|(id, n)| json!({"id": id, "username": n}))
                    .collect(),
            ),
        );
    }
    let project = format!("/api/v4/projects/{OWNER}%2F{REPO}");
    let Some(rest) = path.strip_prefix(&project) else {
        return reply(404, json!({"message": "404 Project Not Found"}));
    };
    let parts: Vec<&str> = rest.split('/').filter(|p| !p.is_empty()).collect();
    match (method, parts.as_slice()) {
        ("GET", []) => reply(200, w.project.clone()),
        ("GET", ["releases"]) => reply(
            200,
            Value::Array(w.releases.iter().map(|t| json!({"tag_name": t})).collect()),
        ),
        ("GET", ["releases", tag]) => match w.releases.iter().find(|t| t.as_str() == *tag) {
            Some(t) => reply(200, json!({"tag_name": t})),
            None => reply(404, json!({"message": "404 Not found"})),
        },
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
                .filter(|m| {
                    q.get("assignee_id")
                        .is_none_or(|a| m.assignees.iter().any(|i| i.to_string() == a))
                })
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
                Value::Array(
                    page(&list, q)
                        .iter()
                        .map(|m| gitlab_mr(m, bare, host))
                        .collect(),
                ),
            )
        }
        ("GET", ["merge_requests", n]) => {
            match gitlab_single(w, n.parse().unwrap_or_default(), bare, host) {
                Some(v) => reply(200, v),
                None => reply(404, json!({"message": "404 Not found"})),
            }
        }
        ("GET", ["merge_requests", n, "notes"]) => {
            let mut notes: Vec<&FakeNote> = w
                .notes
                .iter()
                .filter(|x| x.number.to_string() == *n)
                .collect();
            if q.get("sort") != Some("asc") {
                notes.reverse();
            }
            reply(
                200,
                Value::Array(
                    page(&notes, q)
                        .iter()
                        .map(|x| json!({"body": x.body, "system": x.system, "created_at": time(x.at)}))
                        .collect(),
                ),
            )
        }
        ("POST", ["merge_requests", n, "notes"]) => {
            let Ok(number) = n.parse::<u64>() else {
                return reply(404, json!({"message": "404 Not found"}));
            };
            w.clock += 1;
            let at = w.clock;
            w.notes.push(FakeNote {
                number,
                body: body["body"].as_str().unwrap_or_default().into(),
                system: false,
                at,
            });
            reply(201, json!({"body": body["body"], "system": false}))
        }
        // As `lib/api/merge_requests.rb` answers: the head first, then a
        // squash asked for, then whether it can merge.
        ("PUT", ["merge_requests", n, "merge"]) => {
            let number: u64 = n.parse().unwrap_or_default();
            for action in std::mem::take(&mut w.on_merge) {
                act(w, bare, &action);
            }
            let Some(mr) = w.mrs.iter().find(|m| m.number == number).cloned() else {
                return reply(404, json!({"message": "404 Not found"}));
            };
            if mr.state == "open"
                && body["sha"]
                    .as_str()
                    .is_some_and(|sha| sha != head_of(&mr, bare))
            {
                return reply(
                    409,
                    json!({"message": "SHA does not match HEAD of source branch"}),
                );
            }
            if body["squash"].as_bool() == Some(true)
                && let Some(m) = w.mrs.iter_mut().find(|m| m.number == number)
            {
                m.squash = Some(true);
            }
            w.merge_status = None;
            if let Some((status, message, then)) = w.merge_answers.pop_front() {
                w.merge_status = Some(then);
                return reply(status, json!({ "message": message }));
            }
            let target = bare_git(bare, &["rev-parse", &format!("refs/heads/{}", mr.target)])
                .unwrap_or_default();
            if mr.state != "open"
                || mr.blocked.is_some()
                || !contains(bare, &head_of(&mr, bare), &target)
            {
                return reply(405, json!({"message": "405 Method Not Allowed"}));
            }
            let remove = body["should_remove_source_branch"].as_bool() == Some(true);
            if !merge_mr(w, bare, number, remove) {
                return reply(422, json!({"message": "Branch cannot be merged"}));
            }
            let m = w.mrs.iter().find(|m| m.number == number).cloned().unwrap();
            reply(200, gitlab_mr(&m, bare, host))
        }
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
                .find(|m| m.state == "open" && !m.fork && m.source == source && m.target == target)
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
                lagging_reads: None,
                fork: false,
                squash: body["squash"].as_bool(),
                remove_source_branch: body["remove_source_branch"].as_bool(),
                blocked: None,
                merge_error: None,
                assignees: Vec::new(),
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
            let number = mr.number;
            if let Some(ids) = body["assignee_ids"].as_array() {
                let ids = ids.iter().filter_map(Value::as_u64).collect();
                set_assignees(w, number, ids);
                let record = bare_git(bare, &["log", "-1", "--format=%B", "refs/git-intake/pause"]);
                w.assignments.push((number, record));
            }
            let m = w.mrs.iter().find(|m| m.number == number).cloned().unwrap();
            reply(200, gitlab_mr(&m, bare, host))
        }
        _ => reply(404, json!({"message": "404 Not Found"})),
    }
}

fn github_pr(mr: &FakeMr, bare: &Path, host: &str, full_name: &str) -> Value {
    let repo = json!({"id": 1, "full_name": full_name});
    json!({
        "number": mr.number,
        "html_url": format!("{host}/{OWNER}/{REPO}/pull/{}", mr.number),
        "state": if mr.state == "open" { "open" } else { "closed" },
        "merged_at": (mr.state == "merged").then(|| time(mr.ended.unwrap_or(0))),
        "closed_at": (mr.state != "open").then(|| time(mr.ended.unwrap_or(0))),
        "head": {
            "sha": head_of(mr, bare),
            "ref": mr.source,
            "repo": if mr.fork {
                json!({"id": 2, "full_name": format!("someone/{REPO}")})
            } else {
                repo.clone()
            },
        },
        "base": {"ref": mr.target, "repo": repo},
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
        ("GET", ["releases"]) => reply(
            200,
            Value::Array(w.releases.iter().map(|t| json!({"tag_name": t})).collect()),
        ),
        ("GET", ["releases", "tags", tag]) => {
            match w.releases.iter().find(|t| t.as_str() == *tag) {
                Some(t) => reply(200, json!({"tag_name": t})),
                None => reply(404, json!({"message": "Not Found"})),
            }
        }
        ("GET", ["rules", "branches", _]) => reply(200, w.rules.clone()),
        ("GET", ["branches", b]) => reply(200, json!({"name": b, "protected": w.protected})),
        ("GET", ["branches", _, "protection"]) => match &w.protection {
            Ok(p) => reply(200, p.clone()),
            Err((status, message)) => reply(*status, json!({ "message": message })),
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
                Value::Array(
                    page(&list, q)
                        .iter()
                        .map(|m| github_pr(m, bare, host, &w.full_name))
                        .collect(),
                ),
            )
        }
        ("GET", ["pulls", n]) => match w.mrs.iter().find(|m| m.number.to_string() == *n) {
            Some(m) => reply(200, github_pr(m, bare, host, &w.full_name)),
            None => reply(404, json!({"message": "Not Found"})),
        },
        ("GET", ["commits", sha, "check-runs"]) => {
            let source = w
                .mrs
                .iter()
                .find(|m| m.state == "open" && head_of(m, bare) == *sha)
                .map(|m| m.source.clone());
            let scripted = if q.get("page").is_none_or(|p| p == "1") {
                let state = read_pipeline(w, bare, sha, source.as_deref());
                w.read_checks.insert(sha.to_string(), state);
                state
            } else {
                w.read_checks.get(*sha).copied().flatten()
            };
            let mut runs: Vec<Value> = Vec::new();
            if let Some(state) = scripted {
                let (status, conclusion) = match state {
                    "success" => ("completed", Some("success")),
                    "failed" => ("completed", Some("failure")),
                    "canceled" => ("completed", Some("cancelled")),
                    _ => ("in_progress", None),
                };
                runs.push(json!({"name": "ci", "status": status, "conclusion": conclusion}));
            }
            runs.extend(w.check_runs.iter().map(|(name, status, conclusion)| {
                json!({"name": name, "status": status, "conclusion": conclusion})
            }));
            reply(
                200,
                json!({"total_count": runs.len(), "check_runs": page(&runs, q)}),
            )
        }
        ("GET", ["commits", _, "status"]) => {
            reply(200, json!({"state": "pending", "statuses": []}))
        }
        ("PUT", ["pulls", n, "merge"]) => {
            let number: u64 = n.parse().unwrap_or_default();
            for action in std::mem::take(&mut w.on_merge) {
                act(w, bare, &action);
            }
            let Some(mr) = w.mrs.iter().find(|m| m.number == number).cloned() else {
                return reply(404, json!({"message": "Not Found"}));
            };
            if mr.state != "open" {
                return reply(405, json!({"message": "Pull Request is not mergeable"}));
            }
            if body["sha"].as_str() != Some(head_of(&mr, bare).as_str()) {
                return reply(
                    409,
                    json!({"message": "Head branch was modified. Review and try the merge again."}),
                );
            }
            if body["merge_method"] != "merge" {
                return reply(405, json!({"message": "Merge method not allowed"}));
            }
            let required = w
                .rules
                .as_array()
                .is_some_and(|rules| rules.iter().any(|r| r["type"] == "required_status_checks"));
            let head = head_of(&mr, bare);
            if required && w.read_checks.get(&head).copied().flatten().is_none() {
                return reply(
                    405,
                    json!({"message": "Required status check \"ci\" is expected."}),
                );
            }
            let target = bare_git(bare, &["rev-parse", &format!("refs/heads/{}", mr.target)])
                .unwrap_or_default();
            let head = head_of(&mr, bare);
            let merged = bare_git(
                bare,
                &[
                    "commit-tree",
                    &format!("{head}^{{tree}}"),
                    "-p",
                    &target,
                    "-p",
                    &head,
                    "-m",
                    &format!("Merge pull request #{number}"),
                ],
            )
            .unwrap_or_default();
            bare_git(
                bare,
                &["update-ref", &format!("refs/heads/{}", mr.target), &merged],
            );
            w.clock += 1;
            let clock = w.clock;
            if let Some(m) = w.mrs.iter_mut().find(|m| m.number == number) {
                m.head = Some(head);
                m.state = "merged";
                m.ended = Some(clock);
            }
            reply(200, json!({"merged": true, "sha": merged}))
        }
        ("GET", ["issues", n, "comments"]) => reply(
            200,
            Value::Array(
                w.notes
                    .iter()
                    .filter(|x| x.number.to_string() == *n)
                    .map(|x| json!({"body": x.body, "created_at": time(x.at)}))
                    .collect(),
            ),
        ),
        ("POST", ["issues", n, "comments"]) => {
            let Ok(number) = n.parse::<u64>() else {
                return reply(404, json!({"message": "Not Found"}));
            };
            w.clock += 1;
            let at = w.clock;
            w.notes.push(FakeNote {
                number,
                body: body["body"].as_str().unwrap_or_default().into(),
                system: false,
                at,
            });
            reply(201, json!({"body": body["body"]}))
        }
        ("POST", ["pulls"]) => {
            let source = body["head"].as_str().unwrap_or_default().to_string();
            let target = body["base"].as_str().unwrap_or_default().to_string();
            if w.mrs
                .iter()
                .any(|m| m.state == "open" && !m.fork && m.source == source && m.target == target)
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
                lagging_reads: None,
                fork: false,
                squash: None,
                remove_source_branch: None,
                blocked: None,
                merge_error: None,
                assignees: Vec::new(),
            };
            let json = github_pr(&mr, bare, host, &w.full_name);
            w.mrs.push(mr);
            reply(201, json)
        }
        ("PATCH", ["pulls", n]) => {
            w.clock += 1;
            let clock = w.clock;
            let full_name = w.full_name.clone();
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
            reply(200, github_pr(mr, bare, host, &full_name))
        }
        _ => reply(404, json!({"message": "Not Found"})),
    }
}
