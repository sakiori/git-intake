//! Throwaway git repositories and a runner for the built binary, isolated
//! from any user or system git configuration.

#![allow(dead_code)]

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

pub struct Env {
    dir: TempDir,
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
        fs::write(dir.path().join("gitconfig"), "").unwrap();
        fs::create_dir_all(dir.path().join("home")).unwrap();
        Self { dir }
    }

    pub fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    fn apply_env(&self, cmd: &mut Command) {
        let config = self.path("gitconfig");
        for var in ISOLATION_CLEARED {
            cmd.env_remove(var);
        }
        cmd.env("GIT_CONFIG_GLOBAL", &config)
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
        let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("git-fork-sync"));
        cmd.current_dir(cwd).args(args);
        self.apply_env(&mut cmd);
        let output = cmd.output().expect("run git-fork-sync");
        Assert::new(output)
    }

    /// Runs the tool in `cwd` with extra environment variables.
    pub fn tool_env(&self, cwd: &Path, args: &[&str], vars: &[(&str, &str)]) -> Assert {
        let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("git-fork-sync"));
        cmd.current_dir(cwd).args(args);
        self.apply_env(&mut cmd);
        for (k, v) in vars {
            cmd.env(k, v);
        }
        Assert::new(cmd.output().expect("run git-fork-sync"))
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

    /// The `<git-common-dir>/fork-sync` directory of a repository.
    pub fn fork_sync_dir(&self, repo: &Path) -> PathBuf {
        repo.join(".git").join("fork-sync")
    }

    pub fn update_worktree(&self, repo: &Path) -> PathBuf {
        self.fork_sync_dir(repo).join("worktree")
    }

    /// Asserts that no update worktree, branch or state exists.
    pub fn assert_no_update(&self, repo: &Path) {
        let dir = self.fork_sync_dir(repo);
        assert!(!dir.join("worktree").exists(), "update worktree remains");
        assert!(!dir.join("state.json").exists(), "update state remains");
        assert!(
            !self.has_rev(repo, "refs/heads/fork-sync/update"),
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

/// Output text of an assertion.
pub fn stdout(a: &Assert) -> String {
    String::from_utf8_lossy(&a.get_output().stdout).into_owned()
}

pub fn stderr(a: &Assert) -> String {
    String::from_utf8_lossy(&a.get_output().stderr).into_owned()
}

/// A fork: a clone of one upstream with the upstream at the root, a publish
/// remote, and a mirror branch.
pub struct Fork {
    pub env: Env,
    pub up: Upstream,
    pub publish: PathBuf,
    pub down: PathBuf,
}

pub const FORK_FILE: &str = "app.txt";
pub const FORK_CONTENT: &str = "one\ntwo\nthree\n";

impl Fork {
    pub fn new() -> Self {
        Self::with_checks(&["test -f app.txt"])
    }

    pub fn with_checks(checks: &[&str]) -> Self {
        let env = Env::new();
        let up = env.upstream("upstream", &[(FORK_FILE, FORK_CONTENT)]);
        let publish = env.bare("publish");
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
        env.git(
            &down,
            &["remote", "add", "origin", &publish.to_string_lossy()],
        );
        let checks = checks
            .iter()
            .map(|c| format!("{c:?}"))
            .collect::<Vec<_>>()
            .join(", ");
        let config = format!(
            "integration = \"main\"\npublish = \"origin\"\ninstall = \"true\"\nchecks = [{checks}]\n\n\
             [[upstream]]\nname = \"upstream\"\nremote = \"upstream\"\nbranch = \"main\"\nurl = {:?}\nmirror = \"upstream-main\"\n",
            up.url()
        );
        env.commit(
            &down,
            &[(".fork-sync.toml", &config)],
            "fork: add fork-sync configuration",
        );
        env.git(&down, &["push", "-q", "origin", "main"]);
        Self {
            env,
            up,
            publish,
            down,
        }
    }

    pub fn tool(&self, args: &[&str]) -> Assert {
        self.env.tool(&self.down, args)
    }
}

/// A monorepo with path upstreams `app-a` at `apps/a` (mirror `mirror-a`) and
/// `app-b` at `apps/b`, not yet imported, and no publish remote.
pub struct Mono {
    pub env: Env,
    pub a: Upstream,
    pub b: Upstream,
    pub down: PathBuf,
}

impl Mono {
    pub fn new() -> Self {
        let env = Env::new();
        let a = env.upstream("app-a", &[("a.txt", "a1\na2\na3\n")]);
        let b = env.upstream("app-b", &[("b.txt", "b1\nb2\nb3\n")]);
        let down = env.work("mono");
        let config = mono_config(&a, &b, "");
        env.commit(
            &down,
            &[("README.md", "monorepo\n"), (".fork-sync.toml", &config)],
            "mono: initial layout",
        );
        Self { env, a, b, down }
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
        "integration = \"main\"\nchecks = [\"test -f apps/a/a.txt\", \"test -f apps/b/b.txt\"]\n\n\
         [[upstream]]\nname = \"app-a\"\nremote = \"app-a\"\nbranch = \"main\"\nurl = {:?}\npath = \"apps/a\"\nmirror = \"mirror-a\"\n\n\
         [[upstream]]\nname = \"app-b\"\nremote = \"app-b\"\nbranch = \"main\"\nurl = {:?}\npath = \"apps/b\"\n{extra}",
        a.url(),
        b.url()
    )
}
