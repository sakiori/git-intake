//! The repository the tool runs in: its common git directory, worktrees,
//! cleanliness, and the committed configuration.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::config::{self, Config};
use crate::git::{Git, branch_ref};
use crate::state::{self, Paths, UpdateState};

pub struct Repo {
    /// Runs every repository-wide git command. It is bound to the main
    /// worktree, or to the repository itself when it is bare, never to the
    /// directory the tool was started in: that directory may be the update
    /// worktree, which `finish` and `abort` remove, and relative remote URLs
    /// must mean what they mean in the main worktree.
    pub git: Git,
    pub paths: Paths,
    /// The top of the worktree the tool was started in.
    toplevel: Option<PathBuf>,
    /// The commit at `HEAD` of the worktree the tool was started in.
    head: Option<String>,
}

/// One entry of `git worktree list`.
#[derive(Debug, Clone)]
pub struct Worktree {
    pub path: PathBuf,
    pub branch: Option<String>,
    pub bare: bool,
}

/// The configuration and whether the working copy differs from it.
pub struct Loaded {
    pub config: Config,
    pub working_copy_differs: bool,
}

impl Repo {
    /// Opens the repository containing the current directory. Everything
    /// that depends on that directory is resolved here, once.
    pub fn open() -> Result<Self> {
        let cwd = std::env::current_dir().context("cannot determine the current directory")?;
        let here = Git::new(cwd);
        let common = common_dir(&here)?
            .map_err(|why| anyhow::anyhow!("not inside a git repository: {why}"))?;
        let top = here.raw(&["rev-parse", "--show-toplevel"])?;
        let toplevel = top
            .success()
            .then(|| canonical(Path::new(top.stdout.trim())));
        let head = here.commit("HEAD")?;
        let worktrees = here.run(&["worktree", "list", "--porcelain", "-z"])?;
        let main = parse_worktrees(&worktrees)
            .into_iter()
            .next()
            .map(|w| w.path)
            .filter(|main| matches!(common_dir(&Git::worktree(main)), Ok(Ok(c)) if c == common));
        Ok(Self {
            // A main worktree that is gone or no longer belongs to this
            // repository leaves the repository itself as the place to run in.
            git: Git::worktree(main.unwrap_or_else(|| common.clone())),
            paths: Paths::new(&common),
            toplevel,
            head,
        })
    }

    /// The update in progress, if any.
    pub fn state(&self) -> Result<Option<UpdateState>> {
        state::load(&self.paths.state)
    }

    /// The update in progress, or a refusal naming the situation.
    pub fn require_state(&self) -> Result<UpdateState> {
        match self.state()? {
            Some(s) => Ok(s),
            None => bail!("no update is in progress; run `git intake start` to begin one"),
        }
    }

    pub fn save_state(&self, state: &UpdateState) -> Result<()> {
        state::save(&self.paths.state, state)
    }

    /// Every worktree of the repository, the main one first.
    pub fn worktrees(&self) -> Result<Vec<Worktree>> {
        let out = self.git.run(&["worktree", "list", "--porcelain", "-z"])?;
        Ok(parse_worktrees(&out))
    }

    /// The main checkout, unless the repository is bare.
    pub fn main_checkout(&self) -> Result<Option<PathBuf>> {
        Ok(self
            .worktrees()?
            .into_iter()
            .next()
            .filter(|w| !w.bare)
            .map(|w| w.path))
    }

    /// Worktrees that have `branch` checked out.
    pub fn checkouts_of(&self, branch: &str) -> Result<Vec<PathBuf>> {
        Ok(self
            .worktrees()?
            .into_iter()
            .filter(|w| w.branch.as_deref() == Some(branch))
            .map(|w| w.path)
            .collect())
    }

    /// Whether the update worktree is registered with git.
    pub fn update_worktree_registered(&self) -> Result<bool> {
        let wt = canonical(&self.paths.worktree);
        Ok(self.worktrees()?.iter().any(|w| w.path == wt))
    }

    /// Whether the update branch exists.
    pub fn update_branch_exists(&self) -> Result<bool> {
        Ok(self
            .git
            .commit(&branch_ref(state::UPDATE_BRANCH))?
            .is_some())
    }

    /// Refuses when the main checkout or a checkout of the integration branch
    /// has uncommitted changes to tracked files.
    pub fn require_clean(&self, integration: &str) -> Result<()> {
        let mut dirs: Vec<PathBuf> = self.main_checkout()?.into_iter().collect();
        for dir in self.checkouts_of(integration)? {
            if !dirs.contains(&dir) {
                dirs.push(dir);
            }
        }
        for dir in dirs {
            let changes = Git::worktree(&dir).tracked_changes()?;
            if !changes.is_empty() {
                bail!(
                    "{} has uncommitted changes; commit or stash them first:\n{}",
                    self.describe_checkout(&dir, integration)?,
                    bullet(&changes)
                );
            }
        }
        Ok(())
    }

    fn describe_checkout(&self, dir: &Path, integration: &str) -> Result<String> {
        let main = self.main_checkout()?;
        if main.as_deref() == Some(dir) {
            Ok("the main checkout".to_string())
        } else {
            Ok(format!(
                "the checkout of integration branch '{integration}' at {}",
                dir.display()
            ))
        }
    }

    /// Finds the integration branch: from the update in progress, else from
    /// the configuration committed at HEAD.
    fn integration_branch(&self) -> Result<String> {
        if let Some(state) = self.state()? {
            return Ok(state.config.integration);
        }
        let committed = match &self.head {
            Some(head) => self.git.show_file(head, config::FILE_NAME)?,
            None => None,
        };
        let Some(text) = committed else {
            bail!(
                "no {} is committed at HEAD; run the tool from a checkout of the integration branch",
                config::FILE_NAME
            );
        };
        let table: toml::Table = toml::from_str(&text).map_err(|e| {
            anyhow::anyhow!("invalid {} at HEAD: {}", config::FILE_NAME, e.message())
        })?;
        match table.get("integration").and_then(|v| v.as_str()) {
            Some(b) if !b.trim().is_empty() => Ok(b.trim().to_string()),
            _ => bail!(
                "invalid {} at HEAD: `integration` is missing",
                config::FILE_NAME
            ),
        }
    }

    /// Reads and validates the configuration committed on the integration
    /// branch tip. A working copy that differs is reported on stderr.
    pub fn load_config(&self) -> Result<Loaded> {
        let integration = self.integration_branch()?;
        let rev = branch_ref(&integration);
        if self.git.commit(&rev)?.is_none() {
            bail!("the integration branch '{integration}' does not exist");
        }
        let Some(text) = self.git.show_file(&rev, config::FILE_NAME)? else {
            bail!(
                "no {} is committed on the integration branch '{integration}'",
                config::FILE_NAME
            );
        };
        let config = config::parse(&text)?;
        if config.integration != integration {
            bail!(
                "the {} committed on '{integration}' names '{}' as the integration branch",
                config::FILE_NAME,
                config.integration
            );
        }
        let working_copy_differs = self.working_copy_differs(&text);
        if working_copy_differs {
            eprintln!(
                "note: the working copy of {} differs from the version committed on '{integration}'; using the committed version",
                config::FILE_NAME
            );
        }
        Ok(Loaded {
            config,
            working_copy_differs,
        })
    }

    fn working_copy_differs(&self, committed: &str) -> bool {
        let Some(top) = &self.toplevel else {
            return false;
        };
        if self.started_in_update_worktree() {
            return false;
        }
        match fs::read_to_string(top.join(config::FILE_NAME)) {
            Ok(text) => text != committed,
            Err(_) => false,
        }
    }

    /// Removes the update worktree and branch if they exist. Resolutions made
    /// in an unfinished merge are recorded first so a retry reapplies them.
    pub fn remove_update_artifacts(&self) -> Result<Vec<String>> {
        let mut removed = self.remove_update_worktree()?;
        if self.update_branch_exists()? {
            self.git
                .run(&["branch", "-D", "--quiet", state::UPDATE_BRANCH])?;
            removed.push(format!("branch {}", state::UPDATE_BRANCH));
        }
        Ok(removed)
    }

    /// Removes the update worktree if it exists, keeping the update branch.
    pub fn remove_update_worktree(&self) -> Result<Vec<String>> {
        let started_inside = self.started_in_update_worktree();
        let mut removed = Vec::new();
        let wt = &self.paths.worktree;
        if self.update_worktree_registered()? {
            let wt_git = Git::worktree(wt);
            if wt.exists() && wt_git.merge_in_progress().unwrap_or(false) {
                // Best effort: record resolutions already made by hand.
                let _ = wt_git.raw(&["rerere"]);
            }
            self.git.run(&[
                "worktree",
                "remove",
                "--force",
                "--force",
                &wt.to_string_lossy(),
            ])?;
            removed.push(format!("worktree {}", wt.display()));
        }
        if wt.exists() {
            fs::remove_dir_all(wt).with_context(|| format!("cannot remove {}", wt.display()))?;
            if removed.is_empty() {
                removed.push(format!("worktree {}", wt.display()));
            }
        }
        self.git.run(&["worktree", "prune"])?;
        if started_inside && !removed.is_empty() {
            eprintln!(
                "note: the current directory was in the update worktree, which is now removed"
            );
        }
        Ok(removed)
    }

    /// Whether the tool was started in the update worktree.
    fn started_in_update_worktree(&self) -> bool {
        self.toplevel.as_deref() == Some(canonical(&self.paths.worktree).as_path())
    }

    /// Removes the update's logs: the verification logs and saved merge output.
    pub fn remove_logs(&self) -> Result<Vec<String>> {
        let logs = &self.paths.logs;
        if !logs.exists() {
            return Ok(Vec::new());
        }
        fs::remove_dir_all(logs).with_context(|| format!("cannot remove {}", logs.display()))?;
        Ok(vec![format!("logs {}", logs.display())])
    }
}

/// The canonical common git directory of the repository `git` runs in, or
/// git's message when it runs in none.
fn common_dir(git: &Git) -> Result<std::result::Result<PathBuf, String>> {
    let out = git.raw(&["rev-parse", "--path-format=absolute", "--git-common-dir"])?;
    Ok(if out.success() {
        Ok(canonical(Path::new(out.stdout.trim())))
    } else {
        Err(out.stderr.trim().to_string())
    })
}

/// The entries of `git worktree list --porcelain -z`, the main one first.
fn parse_worktrees(out: &str) -> Vec<Worktree> {
    let mut list = Vec::new();
    let mut current: Option<Worktree> = None;
    for line in out.split('\0') {
        if let Some(path) = line.strip_prefix("worktree ") {
            if let Some(w) = current.take() {
                list.push(w);
            }
            current = Some(Worktree {
                path: canonical(Path::new(path)),
                branch: None,
                bare: false,
            });
        } else if let Some(w) = current.as_mut() {
            if let Some(b) = line.strip_prefix("branch ") {
                w.branch = Some(b.strip_prefix("refs/heads/").unwrap_or(b).to_string());
            } else if line == "bare" {
                w.bare = true;
            }
        }
    }
    if let Some(w) = current {
        list.push(w);
    }
    list
}

/// Canonical form of a path when it exists, the path itself otherwise.
pub fn canonical(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Indented list for messages.
pub fn bullet<S: AsRef<str>>(items: &[S]) -> String {
    items
        .iter()
        .map(|i| format!("  {}", i.as_ref()))
        .collect::<Vec<_>>()
        .join("\n")
}
