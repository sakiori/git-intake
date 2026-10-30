//! The only place that runs `git`. Every repository operation goes through
//! [`Git`], which captures output and turns failures into [`GitError`]s that
//! name the command and carry git's own message.

use std::fmt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result};

/// Environment variables that would redirect git away from the directory a
/// [`Git`] handle is bound to. They are cleared for worktree handles.
const REDIRECTING_ENV: &[&str] = &[
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_PREFIX",
    "GIT_COMMON_DIR",
];

/// A failed git invocation.
#[derive(Debug, Clone)]
pub struct GitError {
    pub args: Vec<String>,
    pub code: Option<i32>,
    pub stderr: String,
}

impl fmt::Display for GitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let code = match self.code {
            Some(code) => format!("exit status {code}"),
            None => "terminated by a signal".to_string(),
        };
        write!(f, "`git {}` failed ({code})", self.args.join(" "))?;
        let stderr = self.stderr.trim();
        if !stderr.is_empty() {
            write!(f, ": {stderr}")?;
        }
        Ok(())
    }
}

impl std::error::Error for GitError {}

/// The captured result of a git invocation, successful or not.
#[derive(Debug, Clone)]
pub struct Output {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    args: Vec<String>,
}

impl Output {
    pub fn success(&self) -> bool {
        self.code == Some(0)
    }

    /// Converts a failed invocation into an error.
    pub fn into_error(self) -> GitError {
        GitError {
            args: self.args,
            code: self.code,
            stderr: self.stderr,
        }
    }

    /// Returns stdout on success, the error otherwise.
    pub fn checked(self) -> Result<String, GitError> {
        if self.success() {
            Ok(self.stdout)
        } else {
            Err(self.into_error())
        }
    }
}

/// A git handle bound to one directory.
#[derive(Debug, Clone)]
pub struct Git {
    dir: PathBuf,
    isolated: bool,
}

impl Git {
    /// A handle for the repository the tool was started in. The caller's git
    /// environment is honoured.
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            isolated: false,
        }
    }

    /// A handle for a specific worktree: variables that would redirect git to
    /// another repository or index are cleared.
    pub fn worktree(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            isolated: true,
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn command<S: AsRef<str>>(&self, args: &[S]) -> Command {
        let mut cmd = Command::new("git");
        cmd.arg("-C").arg(&self.dir);
        cmd.args(args.iter().map(|a| a.as_ref()));
        cmd.env("LC_ALL", "C")
            .env("GIT_TERMINAL_PROMPT", "0")
            .stdin(Stdio::null());
        if self.isolated {
            for var in REDIRECTING_ENV {
                cmd.env_remove(var);
            }
        }
        cmd
    }

    /// Runs git and captures the result whatever the exit status.
    pub fn raw<S: AsRef<str>>(&self, args: &[S]) -> Result<Output> {
        let owned: Vec<String> = args.iter().map(|a| a.as_ref().to_string()).collect();
        let out = self
            .command(args)
            .output()
            .with_context(|| format!("cannot run `git {}`", owned.join(" ")))?;
        Ok(Output {
            code: out.status.code(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
            args: owned,
        })
    }

    /// Runs git and returns stdout, failing on a non-zero exit status.
    pub fn run<S: AsRef<str>>(&self, args: &[S]) -> Result<String> {
        Ok(self.raw(args)?.checked()?)
    }

    /// Runs git and returns trimmed stdout.
    pub fn line<S: AsRef<str>>(&self, args: &[S]) -> Result<String> {
        Ok(self.run(args)?.trim().to_string())
    }

    /// Runs a git predicate: exit status 0 is true, 1 is false, anything else
    /// is an error.
    pub fn test<S: AsRef<str>>(&self, args: &[S]) -> Result<bool> {
        let out = self.raw(args)?;
        match out.code {
            Some(0) => Ok(true),
            Some(1) => Ok(false),
            _ => Err(out.into_error().into()),
        }
    }

    /// Resolves a revision to a full commit id, or `None` when it does not
    /// name a commit.
    pub fn commit(&self, rev: &str) -> Result<Option<String>> {
        let out = self.raw(&[
            "rev-parse",
            "-q",
            "--verify",
            "--end-of-options",
            &format!("{rev}^{{commit}}"),
        ])?;
        if out.success() {
            Ok(Some(out.stdout.trim().to_string()))
        } else {
            Ok(None)
        }
    }

    /// Whether `ancestor` is contained in `descendant`.
    pub fn is_ancestor(&self, ancestor: &str, descendant: &str) -> Result<bool> {
        self.test(&["merge-base", "--is-ancestor", ancestor, descendant])
    }

    /// The best common ancestor of two commits, if they share history.
    pub fn merge_base(&self, a: &str, b: &str) -> Result<Option<String>> {
        let out = self.raw(&["merge-base", a, b])?;
        match out.code {
            Some(0) => Ok(Some(out.stdout.trim().to_string())),
            Some(1) => Ok(None),
            _ => Err(out.into_error().into()),
        }
    }

    /// Number of commits reachable from `include` but not from `exclude`.
    pub fn count(&self, exclude: &str, include: &str) -> Result<u64> {
        let text = self.line(&["rev-list", "--count", &format!("{exclude}..{include}")])?;
        text.parse()
            .with_context(|| format!("unexpected output from git rev-list: {text}"))
    }

    /// Whether `path` exists in the tree of `commit`.
    pub fn path_exists(&self, commit: &str, path: &str) -> Result<bool> {
        let out = self.raw(&["cat-file", "-e", &format!("{commit}:{path}")])?;
        Ok(out.success())
    }

    /// The object type at `rev:path` (`blob`, `tree`, `commit`), or `None`
    /// when nothing is there.
    pub fn object_type(&self, rev: &str, path: &str) -> Result<Option<String>> {
        let out = self.raw(&["cat-file", "-t", &format!("{rev}:{path}")])?;
        Ok(out.success().then(|| out.stdout.trim().to_string()))
    }

    /// Whether an object exists in the repository.
    pub fn object_exists(&self, oid: &str) -> Result<bool> {
        Ok(self.raw(&["cat-file", "-e", oid])?.success())
    }

    /// Abbreviated commit id as git would print it.
    pub fn short(&self, oid: &str) -> Result<String> {
        self.line(&["rev-parse", "--short", oid])
    }

    /// Tracked files with uncommitted changes in this worktree.
    pub fn tracked_changes(&self) -> Result<Vec<String>> {
        let out = self.run(&["status", "--porcelain", "--untracked-files=no"])?;
        Ok(out
            .lines()
            .filter(|l| l.len() > 3)
            .map(|l| l[3..].to_string())
            .collect())
    }

    /// Files with unmerged index entries in this worktree.
    pub fn unmerged_files(&self) -> Result<Vec<String>> {
        let out = self.run(&["diff", "--name-only", "--diff-filter=U"])?;
        Ok(lines(&out))
    }

    /// Whether a merge is in progress in this worktree.
    pub fn merge_in_progress(&self) -> Result<bool> {
        Ok(self.commit("MERGE_HEAD")?.is_some())
    }

    /// The configured URL of a remote, or `None` when the remote is missing.
    pub fn remote_url(&self, remote: &str) -> Result<Option<String>> {
        let out = self.raw(&["config", "--get", &format!("remote.{remote}.url")])?;
        match out.code {
            Some(0) => Ok(Some(out.stdout.trim().to_string())),
            Some(1) => Ok(None),
            _ => Err(out.into_error().into()),
        }
    }

    /// Reads a blob at `rev:path`, or `None` when it does not exist.
    pub fn show_file(&self, rev: &str, path: &str) -> Result<Option<String>> {
        let spec = format!("{rev}:{path}");
        if !self.raw(&["cat-file", "-e", &spec])?.success() {
            return Ok(None);
        }
        Ok(Some(self.run(&["show", &spec])?))
    }
}

/// Splits git output into non-empty lines.
pub fn lines(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim_end)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect()
}

/// A full ref name for a local branch.
pub fn branch_ref(branch: &str) -> String {
    format!("refs/heads/{branch}")
}

/// A full ref name for a remote-tracking branch.
pub fn tracking_ref(remote: &str, branch: &str) -> String {
    format!("refs/remotes/{remote}/{branch}")
}

/// The first twelve characters of a commit id, for display.
pub fn abbrev(oid: &str) -> &str {
    &oid[..oid.len().min(12)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_skips_blank_lines() {
        assert_eq!(lines("a\n\nb  \n"), vec!["a", "b"]);
    }

    #[test]
    fn abbrev_truncates() {
        assert_eq!(abbrev("0123456789abcdef"), "0123456789ab");
        assert_eq!(abbrev("abc"), "abc");
    }

    #[test]
    fn error_names_command_and_stderr() {
        let err = GitError {
            args: vec!["fetch".into(), "origin".into()],
            code: Some(128),
            stderr: "fatal: no route\n".into(),
        };
        assert_eq!(
            err.to_string(),
            "`git fetch origin` failed (exit status 128): fatal: no route"
        );
    }
}
