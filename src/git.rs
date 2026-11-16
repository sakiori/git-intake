//! The only place that runs `git`. Every repository operation goes through
//! [`Git`], which captures output and turns failures into [`GitError`]s that
//! name the command and carry git's own message.

use std::fmt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result};

use crate::redact::redacted;

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
        // The arguments and git's message may name a URL with credentials,
        // and git's message may quote a token.
        write!(
            f,
            "`git {}` failed ({code})",
            redacted(&self.args.join(" "))
        )?;
        let stderr = self.stderr.trim();
        if !stderr.is_empty() {
            write!(f, ": {}", redacted(stderr))?;
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
    /// Whether git may find a repository only at `dir` itself, never in a
    /// directory above it.
    exact: bool,
}

impl Git {
    /// A handle for the directory the tool was started in, which finds the
    /// repository. The caller's git environment is honoured.
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            isolated: false,
            exact: false,
        }
    }

    /// A handle for a specific worktree: variables that would redirect git to
    /// another repository or index are cleared.
    pub fn worktree(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            isolated: true,
            exact: false,
        }
    }

    /// A handle for the repository at `dir` itself, as for a local URL:
    /// like [`Self::worktree`], and git never looks for a repository in a
    /// directory above `dir`.
    pub fn exactly_at(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            isolated: true,
            exact: true,
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn command<S: AsRef<str>>(&self, args: &[S]) -> Command {
        let mut cmd = Command::new("git");
        cmd.arg("-C").arg(&self.dir);
        // Signature checks would print into the formats the tool parses.
        cmd.args(["-c", "log.showSignature=false"]);
        cmd.args(args.iter().map(|a| a.as_ref()));
        cmd.env("LC_ALL", "C")
            .env("GIT_TERMINAL_PROMPT", "0")
            .stdin(Stdio::null());
        if self.isolated {
            for var in REDIRECTING_ENV {
                cmd.env_remove(var);
            }
        }
        if self.exact
            && let Some(parent) = self.dir.parent()
        {
            cmd.env("GIT_CEILING_DIRECTORIES", parent);
        }
        cmd
    }

    /// Runs git and captures the result whatever the exit status.
    pub fn raw<S: AsRef<str>>(&self, args: &[S]) -> Result<Output> {
        let owned: Vec<String> = args.iter().map(|a| a.as_ref().to_string()).collect();
        let out = self
            .command(args)
            .output()
            .with_context(|| format!("cannot run `git {}`", redacted(&owned.join(" "))))?;
        Ok(Output {
            code: out.status.code(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
            args: owned,
        })
    }

    /// Runs git with `input` on its standard input and returns stdout,
    /// failing on a non-zero exit status. The input is written from another
    /// thread so that large outputs cannot block it.
    pub fn run_with_input<S: AsRef<str>>(&self, args: &[S], input: String) -> Result<String> {
        Ok(self.feed(self.command(args), args, input)?.checked()?)
    }

    /// The password git's credential helpers answer `url` with, or `None` when
    /// they give none. Git asks nobody else: neither a terminal nor an askpass
    /// program, which an empty `GIT_ASKPASS` takes the place of.
    pub fn credential_password(&self, url: &str) -> Result<Option<String>> {
        let args = ["credential", "fill"];
        let mut cmd = self.command(&args);
        cmd.env("GIT_ASKPASS", "");
        let out = self.feed(cmd, &args, format!("url={url}\n\n"))?;
        Ok(out
            .success()
            .then(|| {
                out.stdout
                    .lines()
                    .find_map(|line| line.strip_prefix("password="))
                    .map(str::to_string)
            })
            .flatten())
    }

    /// Runs `cmd`, the git command `args`, with `input` on its standard input.
    fn feed<S: AsRef<str>>(&self, mut cmd: Command, args: &[S], input: String) -> Result<Output> {
        use std::io::Write;
        let owned: Vec<String> = args.iter().map(|a| a.as_ref().to_string()).collect();
        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("cannot run `git {}`", redacted(&owned.join(" "))))?;
        let mut stdin = child.stdin.take().context("no stdin for git")?;
        let writer = std::thread::spawn(move || stdin.write_all(input.as_bytes()));
        let out = child
            .wait_with_output()
            .with_context(|| format!("cannot run `git {}`", redacted(&owned.join(" "))))?;
        writer
            .join()
            .map_err(|_| {
                anyhow::anyhow!("writing to `git {}` panicked", redacted(&owned.join(" ")))
            })?
            .with_context(|| format!("cannot write to `git {}`", redacted(&owned.join(" "))))?;
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
        let out = self.run(&["status", "--porcelain", "-z", "--untracked-files=no"])?;
        Ok(parse_porcelain_z(&out))
    }

    /// Files in this worktree that differ from `HEAD` or the index, untracked
    /// files included and ignored files left out.
    pub fn worktree_changes(&self) -> Result<Vec<String>> {
        let out = self.run(&["status", "--porcelain", "-z", "--untracked-files=all"])?;
        Ok(parse_porcelain_z(&out))
    }

    /// Files with unmerged index entries in this worktree.
    pub fn unmerged_files(&self) -> Result<Vec<String>> {
        let out = self.run(&["diff", "--name-only", "-z", "--diff-filter=U"])?;
        Ok(split_z(&out))
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

/// Splits NUL-terminated git output (`-z`) into raw, unquoted paths.
pub fn split_z(text: &str) -> Vec<String> {
    text.split('\0')
        .filter(|p| !p.is_empty())
        .map(str::to_string)
        .collect()
}

/// Paths from `git status --porcelain -z`. A rename or copy entry is followed
/// by its source path, which is skipped.
fn parse_porcelain_z(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut entries = text.split('\0').filter(|e| !e.is_empty());
    while let Some(entry) = entries.next() {
        if entry.len() < 4 {
            continue;
        }
        let (status, path) = entry.split_at(3);
        out.push(path.to_string());
        if status.starts_with(['R', 'C']) {
            entries.next();
        }
    }
    out
}

/// A full ref name for a local branch.
pub fn branch_ref(branch: &str) -> String {
    format!("refs/heads/{branch}")
}

/// The first twelve characters of a commit id, for display. A revision a
/// user named may be shown the same way, so the cut falls between characters.
pub fn abbrev(oid: &str) -> &str {
    oid.char_indices().nth(12).map_or(oid, |(at, _)| &oid[..at])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_skips_blank_lines() {
        assert_eq!(lines("a\n\nb  \n"), vec!["a", "b"]);
    }

    #[test]
    fn nul_separated_paths_stay_raw() {
        assert_eq!(
            split_z("caf\u{e9}.txt\0q\"uote.txt\0"),
            vec!["caf\u{e9}.txt", "q\"uote.txt"]
        );
        assert_eq!(
            parse_porcelain_z(" M caf\u{e9}.txt\0R  new name\0old name\0M  q\"uote.txt\0"),
            vec!["caf\u{e9}.txt", "new name", "q\"uote.txt"]
        );
    }

    #[test]
    fn abbrev_truncates() {
        assert_eq!(abbrev("0123456789abcdef"), "0123456789ab");
        assert_eq!(abbrev("ééééééééééééé"), "éééééééééééé");
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
