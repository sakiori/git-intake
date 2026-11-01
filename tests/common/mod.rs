//! Throwaway git repositories and a runner for the built binary, isolated
//! from any user or system git configuration.

#![allow(dead_code)]

pub mod forge;

use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use assert_cmd::assert::Assert;
use tempfile::TempDir;

const ISOLATION_CLEARED: &[&str] = &[
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_COMMON_DIR",
    "GIT_PREFIX",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_NAMESPACE",
];

/// Variables of the environment the tool would otherwise read from the
/// machine the tests run on: CI jobs, forge tokens, proxies.
const TOOL_CLEARED: &[&str] = &[
    "GITLAB_CI",
    "CI_SERVER_URL",
    "CI_API_V4_URL",
    "GITHUB_ACTIONS",
    "GITHUB_SERVER_URL",
    "GITHUB_API_URL",
    "GITLAB_TOKEN",
    "GH_TOKEN",
    "GITHUB_TOKEN",
    "GH_ENTERPRISE_TOKEN",
    "GITHUB_ENTERPRISE_TOKEN",
    "CI_JOB_TOKEN",
    "CI_COMMIT_TAG",
    "CI_PIPELINE_CREATED_AT",
    "ALL_PROXY",
    "HTTPS_PROXY",
    "HTTP_PROXY",
    "NO_PROXY",
    "all_proxy",
    "https_proxy",
    "http_proxy",
    "no_proxy",
];

pub struct Env {
    dir: TempDir,
    /// Variables every run of the tool gets, as a CI job gives them.
    vars: RefCell<Vec<(String, String)>>,
}

/// An upstream: a bare repository and a working clone that pushes to it.
pub struct Upstream {
    pub bare: PathBuf,
    pub work: PathBuf,
}

impl Upstream {
    pub fn url(&self) -> String {
        self.bare.to_string_lossy().into_owned()
    }
}

impl Env {
    pub fn new() -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        // git's own technique for SSH remotes in tests: `ssh://<host>/<path>`
        // reaches the repository `<path>` under `served/` through a shim
        // that drops the host and runs the requested git command locally.
        let shim = dir.path().join("ssh-shim");
        fs::write(
            &shim,
            format!(
                "#!/bin/sh\n# git runs: ssh-shim <host> \"<git-upload-pack|git-receive-pack> '<path>'\"\n\
                 shift\neval \"set -- $1\"\nexec git \"${{1#git-}}\" \"{}$2\"\n",
                dir.path().join("served").display()
            ),
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&shim, fs::Permissions::from_mode(0o755)).unwrap();
        fs::write(
            dir.path().join("gitconfig"),
            format!(
                "[core]\n\tsshCommand = {}\n[ssh]\n\tvariant = simple\n",
                shim.display()
            ),
        )
        .unwrap();
        // The `ssh` the tool finds first: `-G <host>` answers as a client
        // whose configuration holds only the aliases a test sets with
        // [`Env::ssh_alias`], so that no test reads the configuration of
        // the machine it runs on; anything else goes to the shim.
        let bin = dir.path().join("bin");
        fs::create_dir_all(&bin).unwrap();
        let ssh = bin.join("ssh");
        fs::write(
            &ssh,
            format!(
                "#!/bin/sh\nif [ \"$1\" = -G ]; then\n  shift\n  [ \"$1\" = -- ] && shift\n  host=$1\n  \
                 alias=$(awk -v a=\"$1\" '$1 == a {{ print $2 }}' {aliases} 2>/dev/null)\n  \
                 [ -n \"$alias\" ] && host=$alias\n  printf 'hostname %s\\nport 22\\n' \"$host\"\n  exit 0\nfi\n\
                 exec {shim} \"$@\"\n",
                aliases = dir.path().join("ssh-aliases").display(),
                shim = shim.display()
            ),
        )
        .unwrap();
        fs::set_permissions(&ssh, fs::Permissions::from_mode(0o755)).unwrap();
        fs::create_dir_all(dir.path().join("home")).unwrap();
        Self {
            dir,
            vars: RefCell::new(Vec::new()),
        }
    }

    /// Makes the SSH client's configuration, as the stub `ssh` answers it,
    /// let `alias` stand for `host`.
    pub fn ssh_alias(&self, alias: &str, host: &str) {
        use std::io::Write;
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.path("ssh-aliases"))
            .unwrap();
        writeln!(file, "{alias} {host}").unwrap();
    }

    /// Gives every later run of the tool the variable `name`.
    pub fn set_var(&self, name: &str, value: &str) {
        let mut vars = self.vars.borrow_mut();
        vars.retain(|(n, _)| n != name);
        vars.push((name.to_string(), value.to_string()));
    }

    /// Stops giving the tool the variable `name`.
    pub fn unset_var(&self, name: &str) {
        self.vars.borrow_mut().retain(|(n, _)| n != name);
    }

    fn tool_command(&self, cwd: &Path, args: &[&str]) -> Command {
        let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("git-intake"));
        cmd.current_dir(cwd).args(args);
        self.apply_env(&mut cmd);
        for var in TOOL_CLEARED {
            cmd.env_remove(var);
        }
        for (var, _) in std::env::vars() {
            if var.starts_with("GIT_INTAKE_") {
                cmd.env_remove(var);
            }
        }
        for (k, v) in self.vars.borrow().iter() {
            cmd.env(k, v);
        }
        cmd
    }

    pub fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    fn apply_env(&self, cmd: &mut Command) {
        let config = self.path("gitconfig");
        for var in ISOLATION_CLEARED {
            cmd.env_remove(var);
        }
        let path = std::env::var_os("PATH").unwrap_or_default();
        let mut dirs = vec![self.path("bin")];
        dirs.extend(std::env::split_paths(&path));
        cmd.env("PATH", std::env::join_paths(dirs).unwrap())
            .env("GIT_CONFIG_GLOBAL", &config)
            .env("GIT_CONFIG_SYSTEM", &config)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("HOME", self.path("home"))
            .env("XDG_CONFIG_HOME", self.path("home"))
            .env("GIT_AUTHOR_NAME", "Test Author")
            .env("GIT_AUTHOR_EMAIL", "author@example.com")
            .env("GIT_COMMITTER_NAME", "Test Committer")
            .env("GIT_COMMITTER_EMAIL", "committer@example.com")
            .env("GIT_EDITOR", "true")
            .env("EDITOR", "true")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("LC_ALL", "C");
    }

    /// Runs git and returns its output whatever the status.
    pub fn git_output(&self, cwd: &Path, args: &[&str]) -> Output {
        let mut cmd = Command::new("git");
        cmd.current_dir(cwd).args(args);
        self.apply_env(&mut cmd);
        cmd.output().expect("run git")
    }

    /// Runs git, panicking on failure, and returns trimmed stdout.
    pub fn git(&self, cwd: &Path, args: &[&str]) -> String {
        let out = self.git_output(cwd, args);
        assert!(
            out.status.success(),
            "git {:?} failed in {}: {}",
            args,
            cwd.display(),
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    /// Whether a revision resolves in a repository.
    pub fn has_rev(&self, cwd: &Path, rev: &str) -> bool {
        self.git_output(cwd, &["rev-parse", "-q", "--verify", rev])
            .status
            .success()
    }

    pub fn rev(&self, cwd: &Path, rev: &str) -> String {
        self.git(cwd, &["rev-parse", rev])
    }

    pub fn is_ancestor(&self, cwd: &Path, a: &str, b: &str) -> bool {
        self.git_output(cwd, &["merge-base", "--is-ancestor", a, b])
            .status
            .success()
    }

    /// A bare repository at `/<path>.git` of the SSH host the shim serves,
    /// as `ssh://127.0.0.1/<path>.git` reaches it.
    pub fn served(&self, path: &str) -> PathBuf {
        self.bare(&format!("served/{path}"))
    }

    /// A bare repository with `main` as its default branch.
    pub fn bare(&self, name: &str) -> PathBuf {
        let path = self.path(&format!("{name}.git"));
        self.git(
            self.dir.path(),
            &[
                "init",
                "-q",
                "--bare",
                "-b",
                "main",
                &path.to_string_lossy(),
            ],
        );
        path
    }

    /// A working repository on `main`.
    pub fn work(&self, name: &str) -> PathBuf {
        let path = self.path(name);
        self.git(
            self.dir.path(),
            &["init", "-q", "-b", "main", &path.to_string_lossy()],
        );
        path
    }

    /// An upstream with one initial commit pushed to its bare repository.
    pub fn upstream(&self, name: &str, files: &[(&str, &str)]) -> Upstream {
        let bare = self.bare(name);
        let work = self.work(&format!("{name}-src"));
        self.git(&work, &["remote", "add", "origin", &bare.to_string_lossy()]);
        self.commit(&work, files, &format!("{name}: initial"));
        self.git(&work, &["push", "-q", "origin", "main"]);
        Upstream { bare, work }
    }

    /// Writes files and commits them; returns the new commit id.
    pub fn commit(&self, repo: &Path, files: &[(&str, &str)], message: &str) -> String {
        for (name, content) in files {
            let p = repo.join(name);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, content).unwrap();
        }
        self.git(repo, &["add", "-A"]);
        self.git(repo, &["commit", "-q", "--allow-empty", "-m", message]);
        self.rev(repo, "HEAD")
    }

    /// Commits to an upstream's work clone and pushes; returns the commit id.
    pub fn push_upstream(&self, up: &Upstream, files: &[(&str, &str)], message: &str) -> String {
        let c = self.commit(&up.work, files, message);
        self.git(&up.work, &["push", "-q", "origin", "HEAD:main"]);
        c
    }

    /// Runs the tool in `cwd`.
    pub fn tool(&self, cwd: &Path, args: &[&str]) -> Assert {
        let output = self
            .tool_command(cwd, args)
            .output()
            .expect("run git-intake");
        Assert::new(output)
    }

    /// Runs the tool in `cwd` with extra environment variables.
    pub fn tool_env(&self, cwd: &Path, args: &[&str], vars: &[(&str, &str)]) -> Assert {
        let mut cmd = self.tool_command(cwd, args);
        for (k, v) in vars {
            cmd.env(k, v);
        }
        Assert::new(cmd.output().expect("run git-intake"))
    }

    /// Runs the tool in `cwd` without the variables `names`.
    pub fn tool_without(&self, cwd: &Path, args: &[&str], names: &[&str]) -> Assert {
        let mut cmd = self.tool_command(cwd, args);
        for name in names {
            cmd.env_remove(name);
        }
        Assert::new(cmd.output().expect("run git-intake"))
    }

    /// Runs the tool with `--json` style output and parses stdout.
    pub fn tool_json(&self, cwd: &Path, args: &[&str], code: i32) -> serde_json::Value {
        let assert = self.tool(cwd, args).code(code);
        let out = &assert.get_output().stdout;
        serde_json::from_slice(out).unwrap_or_else(|e| {
            panic!(
                "invalid JSON from {:?}: {e}\n{}",
                args,
                String::from_utf8_lossy(out)
            )
        })
    }

    /// The `<git-common-dir>/git-intake` directory of a repository.
    pub fn intake_dir(&self, repo: &Path) -> PathBuf {
        repo.join(".git").join("git-intake")
    }

    /// The update worktree of the repository whose main worktree is `repo`.
    pub fn update_worktree(&self, repo: &Path) -> PathBuf {
        repo.join(".worktrees").join("git-intake").join("update")
    }

    /// The `info/exclude` file of the repository whose main worktree is
    /// `repo`.
    pub fn info_exclude(&self, repo: &Path) -> PathBuf {
        PathBuf::from(self.git(
            repo,
            &[
                "rev-parse",
                "--path-format=absolute",
                "--git-path",
                "info/exclude",
            ],
        ))
    }

    /// The references under `refs/git-intake/`.
    pub fn intake_refs(&self, repo: &Path) -> Vec<String> {
        self.git(
            repo,
            &["for-each-ref", "--format=%(refname)", "refs/git-intake/"],
        )
        .lines()
        .map(str::to_string)
        .collect()
    }

    /// Asserts that no update worktree, branch, state, plan or reference
    /// under `refs/git-intake/` exists.
    pub fn assert_no_update(&self, repo: &Path) {
        assert_eq!(
            self.intake_refs(repo),
            Vec::<String>::new(),
            "references under refs/git-intake/ remain"
        );
        assert!(
            !self.intake_dir(repo).join("plan").exists(),
            "the plan remains"
        );
        let dir = self.intake_dir(repo);
        assert!(
            !self.update_worktree(repo).exists(),
            "the update worktree remains"
        );
        assert!(!dir.join("state.json").exists(), "update state remains");
        assert!(
            !self.has_rev(repo, "refs/heads/git-intake/update"),
            "update branch remains"
        );
        let list = self.git(repo, &["worktree", "list", "--porcelain"]);
        assert_eq!(
            list.lines().filter(|l| l.starts_with("worktree ")).count(),
            1,
            "extra worktrees: {list}"
        );
    }
}

/// Turns `state`, an update state as this release writes it, into the shape
/// a release of state format `version` wrote, started by `release`: checks as
/// plain commands, a verification as that release recorded it, and no
/// partial run.
pub fn as_earlier_release(state: &mut serde_json::Value, version: u64, release: &str) {
    state["version"] = version.into();
    state["release"] = release.into();
    let checks: Vec<serde_json::Value> = state["config"]["checks"]
        .as_array()
        .map(|c| c.iter().map(|c| c["run"].clone()).collect())
        .unwrap_or_default();
    state["config"]["checks"] = checks.into();
    if let Some(v) = state.get("verification").filter(|v| !v.is_null()) {
        let results: Vec<serde_json::Value> = v["checks"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|c| {
                serde_json::json!({
                    "kind": "check",
                    "command": c["name"],
                    "outcome": c["outcome"],
                    "exit_code": c["exit_code"],
                    "log": c["log"],
                })
            })
            .collect();
        state["verification"] = serde_json::json!({
            "commit": v["commit"],
            "passed": v["passed"],
            "results": results,
            "problems": [],
        });
    }
    if let Some(object) = state.as_object_mut() {
        object.remove("partial_run");
    }
}

/// Output text of an assertion.
pub fn stdout(a: &Assert) -> String {
    String::from_utf8_lossy(&a.get_output().stdout).into_owned()
}

pub fn stderr(a: &Assert) -> String {
    String::from_utf8_lossy(&a.get_output().stderr).into_owned()
}

/// A fork: a clone of one upstream with the upstream at the root, taken by
/// URL with no remote kept for it, and a publish remote.
pub struct Fork {
    pub env: Env,
    pub up: Upstream,
    pub publish: PathBuf,
    pub down: PathBuf,
    /// The forge of the publish remote, in merge-request mode.
    pub forge: Option<forge::FakeForge>,
}

pub const FORK_FILE: &str = "app.txt";
pub const FORK_CONTENT: &str = "one\ntwo\nthree\n";

/// The check of a fork, by its name and command.
pub const FORK_CHECK: (&str, &str) = ("app", "test -f app.txt");

/// The `checks` line of a configuration with `checks`, each a name and the
/// command it runs.
pub fn checks_line(checks: &[(&str, &str)]) -> String {
    format!(
        "checks = [{}]",
        checks
            .iter()
            .map(|(name, run)| format!("{{ name = {name:?}, run = {run:?} }}"))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

impl Fork {
    pub fn new() -> Self {
        Self::with_checks(&[FORK_CHECK])
    }

    /// A fork with `checks`, each a name and the command it runs.
    pub fn with_checks(checks: &[(&str, &str)]) -> Self {
        Self::build(checks, "push", "follow", None)
    }

    /// A fork that takes its upstream in port mode, publishing by push.
    pub fn port() -> Self {
        Self::build(&[FORK_CHECK], "push", "port", None)
    }

    /// A fork that takes its upstream in port mode and publishes through
    /// merge requests, like [`Fork::merge_request`].
    pub fn port_merge_request() -> Self {
        Self::build(
            &[FORK_CHECK],
            "merge-request",
            "port",
            Some(forge::Kind::GitLab),
        )
    }

    /// A fork that publishes through merge requests on a GitLab whose
    /// settings qualify, reached as from one of its CI jobs.
    pub fn merge_request() -> Self {
        Self::build(
            &[FORK_CHECK],
            "merge-request",
            "follow",
            Some(forge::Kind::GitLab),
        )
    }

    /// A fork that publishes by push to a GitLab project, reached as from
    /// one of its CI jobs.
    pub fn push_gitlab() -> Self {
        Self::build(&[FORK_CHECK], "push", "follow", Some(forge::Kind::GitLab))
    }

    /// A fork that publishes through pull requests on a GitHub whose
    /// settings qualify, reached as from one of its Actions jobs.
    pub fn github() -> Self {
        Self::build(
            &[FORK_CHECK],
            "merge-request",
            "follow",
            Some(forge::Kind::GitHub),
        )
    }

    fn build(checks: &[(&str, &str)], via: &str, mode: &str, kind: Option<forge::Kind>) -> Self {
        let env = Env::new();
        let up = env.upstream("upstream", &[(FORK_FILE, FORK_CONTENT)]);
        let publish = match kind {
            Some(_) => env.served(&format!("{}/{}", forge::OWNER, forge::REPO)),
            None => env.bare("publish"),
        };
        let down = env.path("down");
        env.git(
            &env.path(""),
            &[
                "clone",
                "-q",
                "-o",
                "upstream",
                &up.url(),
                &down.to_string_lossy(),
            ],
        );
        env.git(&down, &["remote", "remove", "upstream"]);
        let fake = kind.map(|kind| match kind {
            forge::Kind::GitLab => forge::FakeForge::gitlab(&publish),
            forge::Kind::GitHub => forge::FakeForge::github(&publish),
        });
        match &fake {
            Some(fake) => {
                // The remote is on the forge's host, reached over SSH.
                env.git(&down, &["remote", "add", "origin", &fake.remote_url()]);
                for (k, v) in fake.job_vars() {
                    env.set_var(&k, &v);
                }
            }
            None => {
                env.git(
                    &down,
                    &["remote", "add", "origin", &publish.to_string_lossy()],
                );
            }
        }
        let config = format!(
            "integration = \"main\"\ninstall = \"true\"\n{}\n\n\
             [publish]\nremote = \"origin\"\nvia = \"{via}\"\n\n\
             [[upstream]]\nname = \"upstream\"\nurl = {:?}\nbranch = \"main\"\nmode = \"{mode}\"\n",
            checks_line(checks),
            up.url()
        );
        env.commit(
            &down,
            &[(".git-intake.toml", &config)],
            "fork: add git-intake configuration",
        );
        env.git(&down, &["push", "-q", "origin", "main"]);
        Self {
            env,
            up,
            publish,
            down,
            forge: fake,
        }
    }

    pub fn tool(&self, args: &[&str]) -> Assert {
        self.env.tool(&self.down, args)
    }

    /// The fake forge of a fork in merge-request mode.
    pub fn forge(&self) -> &forge::FakeForge {
        self.forge.as_ref().expect("a fork in merge-request mode")
    }

    /// Commits and publishes on the integration branch the configuration
    /// with its checks as plain commands, the form of earlier releases.
    pub fn plain_checks(&self) {
        self.reconfigure(
            &checks_line(&[FORK_CHECK]),
            &format!("checks = [{:?}]", FORK_CHECK.1),
        );
    }

    /// The committed configuration with `from` replaced by `to`, committed
    /// and published on the integration branch.
    pub fn reconfigure(&self, from: &str, to: &str) {
        let path = self.down.join(".git-intake.toml");
        let cfg = fs::read_to_string(&path).unwrap();
        assert!(cfg.contains(from), "{cfg}");
        let cfg = cfg.replace(from, to);
        self.env.commit(
            &self.down,
            &[(".git-intake.toml", &cfg)],
            "fork: reconfigure",
        );
        self.env.git(&self.down, &["push", "-q", "origin", "main"]);
    }
}

/// The branch the update's merge request comes from.
pub const MERGE_REQUEST_BRANCH: &str = "git-intake/update";

/// A clone of the publish remote `publish` that plays the forge merging
/// requests, at the remote's integration branch.
pub fn forge(env: &Env, publish: &Path) -> PathBuf {
    let dir = env.path("forge");
    if !dir.exists() {
        env.git(
            &env.path(""),
            &[
                "clone",
                "-q",
                &publish.to_string_lossy(),
                &dir.to_string_lossy(),
            ],
        );
    } else {
        env.git(&dir, &["fetch", "-q", "origin"]);
        env.git(&dir, &["reset", "-q", "--hard", "origin/main"]);
    }
    dir
}

/// The forge merges `branch` of the publish remote `publish` into its
/// integration branch with a merge commit, as GitLab's merge commit methods
/// do; returns the new tip of the integration branch.
pub fn merge_on_forge(env: &Env, publish: &Path, branch: &str) -> String {
    let forge = forge(env, publish);
    env.git(
        &forge,
        &[
            "merge",
            "-q",
            "--no-ff",
            "-m",
            "Merge branch into main",
            &format!("origin/{branch}"),
        ],
    );
    env.git(&forge, &["push", "-q", "origin", "HEAD:main"]);
    env.rev(publish, "main")
}

/// How the forge merges a merge request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum How {
    /// A merge commit, as GitLab's merge commit methods and GitHub's
    /// `Create a merge commit` make it.
    MergeCommit,
    /// The integration branch moves to the merge request's head.
    FastForward,
    /// One commit with the merge request's changes, whose message joins the
    /// messages of its commits, as GitHub's squash merge writes it.
    Squash,
}

/// The fake forge merges merge request `number` the way `how` says, marks
/// it merged and deletes its branch, as a forge set to remove the source
/// branch does. Returns the new tip of the integration branch.
pub fn merge_on_fake(env: &Env, fake: &forge::FakeForge, number: u64, how: How) -> String {
    let mr = fake.mr(number);
    let dir = forge(env, &fake.publish);
    let source = format!("origin/{}", mr.source);
    match how {
        How::MergeCommit => {
            env.git(
                &dir,
                &[
                    "merge",
                    "-q",
                    "--no-ff",
                    "-m",
                    &format!("Merge branch '{}' into '{}'", mr.source, mr.target),
                    &source,
                ],
            );
        }
        How::FastForward => {
            env.git(&dir, &["merge", "-q", "--ff-only", &source]);
        }
        How::Squash => {
            let messages = env.git(&dir, &["log", "--format=%B", &format!("HEAD..{source}")]);
            env.git(&dir, &["merge", "-q", "--squash", &source]);
            env.git(
                &dir,
                &[
                    "commit",
                    "-q",
                    "-m",
                    &format!("{} (#{number})\n\n{messages}", mr.title),
                ],
            );
        }
    }
    env.git(
        &dir,
        &["push", "-q", "origin", &format!("HEAD:{}", mr.target)],
    );
    fake.end(number, "merged");
    env.git(
        &fake.publish,
        &["update-ref", "-d", &format!("refs/heads/{}", mr.source)],
    );
    env.rev(&fake.publish, &mr.target)
}

/// The number of the only open merge request on the fake forge.
pub fn open_number(fake: &forge::FakeForge) -> u64 {
    let open = fake.open_mrs();
    assert_eq!(open.len(), 1, "open merge requests: {open:?}");
    open[0].number
}

/// A monorepo with path upstreams `app-a` at `apps/a` and `app-b` at
/// `apps/b`, not yet imported, and no publish remote unless built by
/// [`Mono::merge_request`].
pub struct Mono {
    pub env: Env,
    pub a: Upstream,
    pub b: Upstream,
    pub down: PathBuf,
    pub publish: Option<PathBuf>,
    pub forge: Option<forge::FakeForge>,
}

impl Mono {
    pub fn new() -> Self {
        Self::build("")
    }

    /// A monorepo that publishes through merge requests to the remote
    /// `origin`, on a GitLab as in [`Fork::merge_request`].
    pub fn merge_request() -> Self {
        let mut m = Self::build("\n[publish]\nremote = \"origin\"\nvia = \"merge-request\"\n");
        let publish = m.env.served(&format!("{}/{}", forge::OWNER, forge::REPO));
        let fake = forge::FakeForge::gitlab(&publish);
        m.env
            .git(&m.down, &["remote", "add", "origin", &fake.remote_url()]);
        for (k, v) in fake.job_vars() {
            m.env.set_var(&k, &v);
        }
        m.env.git(&m.down, &["push", "-q", "origin", "main"]);
        m.publish = Some(publish);
        m.forge = Some(fake);
        m
    }

    fn build(extra: &str) -> Self {
        let env = Env::new();
        let a = env.upstream("app-a", &[("a.txt", "a1\na2\na3\n")]);
        let b = env.upstream("app-b", &[("b.txt", "b1\nb2\nb3\n")]);
        let down = env.work("mono");
        let config = mono_config(&a, &b, extra);
        env.commit(
            &down,
            &[("README.md", "monorepo\n"), (".git-intake.toml", &config)],
            "mono: initial layout",
        );
        Self {
            env,
            a,
            b,
            down,
            publish: None,
            forge: None,
        }
    }

    pub fn tool(&self, args: &[&str]) -> Assert {
        self.env.tool(&self.down, args)
    }

    /// Imports both upstreams through a full update cycle.
    pub fn import_all(&self) {
        self.tool(&["start"]).success();
        self.tool(&["verify"]).success();
        self.tool(&["finish"]).success();
    }
}

pub fn mono_config(a: &Upstream, b: &Upstream, extra: &str) -> String {
    format!(
        "integration = \"main\"\nchecks = [{{ name = \"a\", run = \"test -f apps/a/a.txt\" }}, \
         {{ name = \"b\", run = \"test -f apps/b/b.txt\" }}]\n\n\
         [[upstream]]\nname = \"app-a\"\nurl = {:?}\nbranch = \"main\"\npath = \"apps/a\"\n\n\
         [[upstream]]\nname = \"app-b\"\nurl = {:?}\nbranch = \"main\"\npath = \"apps/b\"\n{extra}",
        a.url(),
        b.url()
    )
}
