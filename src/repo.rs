//! The repository the tool runs in: its common git directory, worktrees,
//! cleanliness, and the committed configuration.

use std::cell::Cell;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::config::{self, Config};
use crate::git::{Git, branch_ref};
use crate::state::{self, Paths, UpdateState};

pub struct Repo {
    /// Runs every repository-wide git command. It is bound to the main
    /// worktree, or to the git directory when the repository is bare or its
    /// main worktree cannot be located, never to the directory the tool was
    /// started in: that directory may be the update worktree, which `finish`
    /// and `abort` remove, and relative remote URLs must mean what they mean
    /// in the main worktree.
    pub git: Git,
    /// Bound to the directory the tool was started in. Only for what the user
    /// names there, such as the revisions given to `start`, which removes no
    /// worktree.
    pub here: Git,
    pub paths: Paths,
    /// The repository's common git directory.
    common: PathBuf,
    main: MainWorktree,
    /// Where `start` places the update worktree: [`state::UPDATE_WORKTREE`]
    /// in the main worktree, [`state::BARE_UPDATE_WORKTREE`] in the git
    /// directory of a bare repository, or `None` when the main worktree
    /// cannot be located.
    placed_worktree: Option<PathBuf>,
    /// The top of the worktree the tool was started in.
    toplevel: Option<PathBuf>,
    /// The commit at `HEAD` of the worktree the tool was started in.
    head: Option<String>,
    /// Whether this command removed the update worktree it was started in.
    left_removed_directory: Cell<bool>,
}

/// Where the repository's main worktree is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MainWorktree {
    /// The repository is bare and has none.
    Bare,
    At(PathBuf),
    /// The git directory lies outside the main worktree and does not name it,
    /// the tool was not started in it, and no update in progress recorded it.
    Unknown,
}

/// One entry of `git worktree list`.
#[derive(Debug, Clone)]
pub struct Worktree {
    pub path: PathBuf,
    pub branch: Option<String>,
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
        let git_dir = here.raw(&["rev-parse", "--path-format=absolute", "--git-dir"])?;
        let started_in_main =
            git_dir.success() && canonical(Path::new(git_dir.stdout.trim())) == common;
        let paths = Paths::new(&common);
        let main = locate_main_worktree(
            &common,
            toplevel.as_deref().filter(|_| started_in_main),
            &paths,
        )?;
        let anchor = match &main {
            MainWorktree::At(dir) => dir.clone(),
            MainWorktree::Bare | MainWorktree::Unknown => common.clone(),
        };
        let placed_worktree = match &main {
            MainWorktree::At(dir) => Some(dir.join(state::UPDATE_WORKTREE)),
            MainWorktree::Bare => Some(common.join(state::BARE_UPDATE_WORKTREE)),
            MainWorktree::Unknown => None,
        };
        Ok(Self {
            git: Git::worktree(anchor),
            here,
            paths,
            common,
            main,
            placed_worktree,
            toplevel,
            head,
            left_removed_directory: Cell::new(false),
        })
    }

    /// The update in progress, if any.
    pub fn state(&self) -> Result<Option<UpdateState>> {
        state::load(&self.paths.state)
    }

    /// The update in progress, or a refusal naming the situation. The
    /// references the update holds are restored first where they are
    /// missing.
    pub fn require_state(&self) -> Result<UpdateState> {
        match self.state()? {
            Some(s) => {
                self.restore_held_references(&s)?;
                Ok(s)
            }
            None => bail!("no update is in progress; run `git intake start` to begin one"),
        }
    }

    /// Restores the references an update holds where they are missing, as
    /// after an interrupted `start`, and says so.
    pub fn restore_held_references(&self, state: &UpdateState) -> Result<()> {
        for name in crate::upstreams::restore_held(&self.git, state)? {
            eprintln!("note: restored the update's reference {name} at the tip `start` fetched");
        }
        Ok(())
    }

    pub fn save_state(&self, state: &UpdateState) -> Result<()> {
        state::save(&self.paths.state, state)
    }

    /// Every worktree of the repository, the main one first. Git lists the
    /// git directory for a main worktree that lies elsewhere; it is replaced
    /// by the main worktree when that is known.
    pub fn worktrees(&self) -> Result<Vec<Worktree>> {
        let out = self.git.run(&["worktree", "list", "--porcelain", "-z"])?;
        let mut list = parse_worktrees(&out);
        if let (Some(first), MainWorktree::At(dir)) = (list.first_mut(), &self.main) {
            first.path = dir.clone();
        }
        Ok(list)
    }

    /// Where the main worktree is.
    pub fn main_worktree(&self) -> &MainWorktree {
        &self.main
    }

    /// The main checkout, unless the repository is bare. Refuses when it
    /// cannot be located.
    pub fn main_checkout(&self) -> Result<Option<PathBuf>> {
        match &self.main {
            MainWorktree::Bare => Ok(None),
            MainWorktree::At(dir) => Ok(Some(dir.clone())),
            MainWorktree::Unknown => bail!("{}", self.unknown_main_worktree()),
        }
    }

    /// Why the main worktree cannot be located.
    pub fn unknown_main_worktree(&self) -> String {
        format!(
            "the main worktree cannot be located from here: the git directory {} lies outside it and does not name it; \
             run the command in the main worktree",
            self.common.display()
        )
    }

    /// Worktrees that have `branch` checked out. Refuses when the main
    /// worktree has it checked out but cannot be located.
    pub fn checkouts_of(&self, branch: &str) -> Result<Vec<PathBuf>> {
        let list = self.worktrees()?;
        if self.main == MainWorktree::Unknown
            && list.first().and_then(|w| w.branch.as_deref()) == Some(branch)
        {
            bail!(
                "'{branch}' is checked out in the main worktree, and {}",
                self.unknown_main_worktree()
            );
        }
        Ok(list
            .into_iter()
            .filter(|w| w.branch.as_deref() == Some(branch))
            .map(|w| w.path)
            .collect())
    }

    /// The update worktree: the one the update in progress records, else the
    /// linked worktree that has the update branch checked out, as an
    /// interrupted `start` leaves it, else where `start` places it. `None`
    /// when there is none of these.
    pub fn update_worktree(&self) -> Result<Option<PathBuf>> {
        if let Some(state) = state::load(&self.paths.state).ok().flatten() {
            return Ok(Some(state.worktree));
        }
        // The first entry is the main worktree, which is never the update's.
        if let Some(w) = self
            .worktrees()?
            .into_iter()
            .skip(1)
            .find(|w| w.branch.as_deref() == Some(state::UPDATE_BRANCH))
        {
            return Ok(Some(w.path));
        }
        Ok(self.placed_worktree.clone())
    }

    /// Where `start` places the update worktree. Refuses where no correct
    /// place exists, before anything changes: the main worktree cannot be
    /// located; a directory on the way to [`state::UPDATE_WORKTREE`] is a
    /// symbolic link or not a directory, which would put the update worktree
    /// outside the checkout; or a bare repository's update worktree would lie
    /// at a path containing `/.git`, which compilers, bundlers and watchers
    /// ignore, so checks run there would not see the tree as it is.
    pub fn place_update_worktree(&self) -> Result<PathBuf> {
        let Some(wt) = &self.placed_worktree else {
            bail!("{}", self.unknown_main_worktree());
        };
        match &self.main {
            MainWorktree::At(main) => {
                for rel in [".worktrees", ".worktrees/git-intake"] {
                    let Ok(meta) = fs::symlink_metadata(main.join(rel)) else {
                        continue;
                    };
                    if meta.file_type().is_symlink() {
                        bail!(
                            "{rel} in the main worktree is a symbolic link; the update worktree has to lie in the checkout, \
                             at {}. Replace the link with a directory",
                            state::UPDATE_WORKTREE
                        );
                    }
                    if !meta.is_dir() {
                        bail!(
                            "{rel} in the main worktree is not a directory; the update worktree is placed at {}",
                            state::UPDATE_WORKTREE
                        );
                    }
                }
            }
            MainWorktree::Bare if wt.to_string_lossy().contains("/.git") => bail!(
                "the update worktree of this bare repository would be {}, a path containing `/.git`, \
                 which compilers, bundlers and watchers ignore, so checks run there would not see the tree as it is; \
                 run the tool in a clone with a checkout",
                wt.display()
            ),
            MainWorktree::Bare | MainWorktree::Unknown => {}
        }
        Ok(wt.clone())
    }

    /// Keeps the update worktree out of the main checkout's status: unless it
    /// is ignored already, adds [`state::WORKTREES_EXCLUDE`] to the
    /// repository's `info/exclude`, once. A bare repository has no checkout.
    pub fn exclude_update_worktree(&self) -> Result<()> {
        if !matches!(self.main, MainWorktree::At(_)) {
            return Ok(());
        }
        let ignored = self
            .git
            .raw(&["check-ignore", "-q", state::UPDATE_WORKTREE])?;
        match ignored.code {
            Some(0) => return Ok(()),
            Some(1) => {}
            _ => return Err(ignored.into_error().into()),
        }
        let path = PathBuf::from(self.git.line(&[
            "rev-parse",
            "--path-format=absolute",
            "--git-path",
            "info/exclude",
        ])?);
        let mut text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(e).with_context(|| format!("cannot read {}", path.display())),
        };
        if text.lines().any(|l| l == state::WORKTREES_EXCLUDE) {
            return Ok(());
        }
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(state::WORKTREES_EXCLUDE);
        text.push('\n');
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
        }
        fs::write(&path, text).with_context(|| format!("cannot write {}", path.display()))
    }

    /// Whether the update worktree is registered with git.
    pub fn update_worktree_registered(&self) -> Result<bool> {
        let Some(wt) = self.update_worktree()? else {
            return Ok(false);
        };
        let wt = canonical(&wt);
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
        self.refuse_publishing_to_an_upstream(&config)?;
        if config.integration != integration {
            bail!(
                "the {} committed on '{integration}' names '{}' as the integration branch",
                config::FILE_NAME,
                config.integration
            );
        }
        let working_copy_differs = self.working_copy_differs(&text)?;
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

    /// The tool never pushes to an upstream: a publish remote fetched from
    /// or pushed to a URL that names an upstream's repository is refused.
    fn refuse_publishing_to_an_upstream(&self, config: &Config) -> Result<()> {
        let Some(publish) = &config.publish else {
            return Ok(());
        };
        let urls = crate::upstreams::remote_urls(&self.git, &publish.remote)?;
        let base = self.git.dir();
        let mut problems = Vec::new();
        for up in &config.upstreams {
            if let Some(url) = urls
                .iter()
                .find(|u| crate::upstreams::same_repository(u, &up.url, base))
            {
                problems.push(format!(
                    "publish remote '{}' has the URL {url}, which names the repository of upstream '{}' ({}); \
                     the tool never pushes to an upstream",
                    publish.remote, up.name, up.url
                ));
            }
        }
        if !problems.is_empty() {
            bail!(
                "invalid {} on '{}':\n{}",
                config::FILE_NAME,
                config.integration,
                problems
                    .iter()
                    .map(|p| format!("  - {p}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            );
        }
        Ok(())
    }

    fn working_copy_differs(&self, committed: &str) -> Result<bool> {
        let Some(top) = &self.toplevel else {
            return Ok(false);
        };
        if self.started_in_update_worktree()? {
            return Ok(false);
        }
        Ok(match fs::read_to_string(top.join(config::FILE_NAME)) {
            Ok(text) => text != committed,
            Err(_) => false,
        })
    }

    /// Removes the update worktree and branch if they exist. Resolutions made
    /// in an unfinished merge or copy are recorded first so a retry reapplies
    /// them.
    pub fn remove_update_artifacts(&self) -> Result<Vec<String>> {
        let mut removed = self.remove_update_worktree()?;
        if self.update_branch_exists()? {
            self.git
                .run(&["branch", "-D", "--quiet", state::UPDATE_BRANCH])?;
            removed.push(format!("branch {}", state::UPDATE_BRANCH));
        }
        Ok(removed)
    }

    /// Removes the update worktree if it exists, keeping the update branch,
    /// and the directories that held it once they are empty.
    pub fn remove_update_worktree(&self) -> Result<Vec<String>> {
        let started_inside = self.started_in_update_worktree()?;
        let mut removed = Vec::new();
        let Some(wt) = self.update_worktree()? else {
            self.git.run(&["worktree", "prune"])?;
            return Ok(removed);
        };
        if self.update_worktree_registered()? {
            if wt.exists() {
                // Best effort: record resolutions already made by hand in a
                // stopped merge or copy; with nothing pending, rerere does
                // nothing.
                let _ = Git::worktree(&wt).raw(&["rerere"]);
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
            fs::remove_dir_all(&wt).with_context(|| format!("cannot remove {}", wt.display()))?;
            if removed.is_empty() {
                removed.push(format!("worktree {}", wt.display()));
            }
        }
        // `.worktrees/git-intake`, then `.worktrees` in the main worktree;
        // either may hold other worktrees or files, which keep it.
        if let MainWorktree::At(main) = &self.main
            && wt == main.join(state::UPDATE_WORKTREE)
        {
            for dir in wt.ancestors().skip(1).take(2) {
                let _ = fs::remove_dir(dir);
            }
        }
        self.git.run(&["worktree", "prune"])?;
        if started_inside && !removed.is_empty() {
            self.left_removed_directory.set(true);
        }
        Ok(removed)
    }

    /// Notes that the command removed the update worktree it was started in.
    /// Printed once the command's own output is complete.
    pub fn note_removed_directory(&self) {
        if self.left_removed_directory.get() {
            eprintln!(
                "note: the current directory was in the update worktree, which is now removed"
            );
        }
    }

    /// Whether the tool was started in the update worktree.
    fn started_in_update_worktree(&self) -> Result<bool> {
        let Some(top) = &self.toplevel else {
            return Ok(false);
        };
        Ok(self
            .update_worktree()?
            .is_some_and(|wt| *top == canonical(&wt)))
    }

    /// Removes the plan of a port update, if any.
    pub fn remove_plan(&self) -> Result<Vec<String>> {
        let plan = &self.paths.plan;
        if !plan.exists() {
            return Ok(Vec::new());
        }
        fs::remove_file(plan).with_context(|| format!("cannot remove {}", plan.display()))?;
        Ok(vec![format!("plan {}", plan.display())])
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

/// Locates the main worktree of the repository whose common git directory is
/// `common`. `started_in` is the top of the worktree the tool was started in
/// when that is the main worktree.
fn locate_main_worktree(
    common: &Path,
    started_in: Option<&Path>,
    paths: &Paths,
) -> Result<MainWorktree> {
    let repo = Git::worktree(common);
    if repo.line(&["rev-parse", "--is-bare-repository"])? == "true" {
        return Ok(MainWorktree::Bare);
    }
    // `core.worktree`, as a submodule's git directory sets it.
    let top = repo.raw(&["rev-parse", "--show-toplevel"])?;
    if top.success() {
        return Ok(MainWorktree::At(canonical(Path::new(top.stdout.trim()))));
    }
    // A git directory that does not name its main worktree: the tool was
    // started in it, or the update in progress recorded it. Both come before
    // the guess below, which a separate git directory that is itself named
    // `.git` would mislead.
    if let Some(top) = started_in {
        return Ok(MainWorktree::At(top.to_path_buf()));
    }
    // An unreadable state, or one of another format, gives no record and is
    // not reported here. `continue`, `verify`, `finish` and `abort` read the
    // state before they need the main worktree and stop on its error; `start`
    // refuses on it as well. Read-only commands carry on: run outside the
    // main worktree of such a layout, they find it from the guess below or
    // say that it cannot be located, and `doctor` also reports the state.
    let recorded = state::load(&paths.state)
        .ok()
        .flatten()
        .and_then(|s| s.main_worktree);
    if let Some(dir) = recorded
        && is_main_worktree(&dir, common)?
    {
        return Ok(MainWorktree::At(dir));
    }
    // The usual layout: the git directory is `.git` at the top of the main
    // worktree.
    if common.file_name().is_some_and(|n| n == ".git")
        && let Some(parent) = common.parent()
        && is_main_worktree(parent, common)?
    {
        return Ok(MainWorktree::At(parent.to_path_buf()));
    }
    Ok(MainWorktree::Unknown)
}

/// Whether `dir` is the top of the main worktree of the repository whose
/// common git directory is `common`.
fn is_main_worktree(dir: &Path, common: &Path) -> Result<bool> {
    if !dir.is_dir() {
        return Ok(false);
    }
    let out = Git::worktree(dir).raw(&[
        "rev-parse",
        "--path-format=absolute",
        "--git-dir",
        "--show-toplevel",
    ])?;
    let mut lines = out.stdout.lines();
    Ok(out.success()
        && lines.next().map(|l| canonical(Path::new(l))).as_deref() == Some(common)
        && lines.next().map(|l| canonical(Path::new(l))).as_deref() == Some(dir))
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
            });
        } else if let Some(w) = current.as_mut()
            && let Some(b) = line.strip_prefix("branch ")
        {
            w.branch = Some(b.strip_prefix("refs/heads/").unwrap_or(b).to_string());
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
