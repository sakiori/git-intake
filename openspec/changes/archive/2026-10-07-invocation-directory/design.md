## Where repository-wide git commands run

Before this change every git command that is not bound to a specific worktree ran in the directory the tool was started in. That directory is not stable: `finish`, `abort` and the opening of a merge request remove the update worktree, and the user may be standing in it.

The commands in question (fetch, push, ref updates, object and history queries, configuration, remotes, and worktree management) need a repository, not a working tree. Three places can run them:

- The starting directory, as before. It fails as soon as the update worktree it lies in is removed, and it gives a relative remote URL a different meaning in each worktree.
- The git directory. It always exists and needs no working tree, but git treats it like a bare repository: a relative remote URL is resolved against the git directory itself, so `../upstream.git`, which works from the main checkout, would stop working everywhere.
- The main worktree, or the git directory when the repository is bare. It exists for as long as the repository does, and it is where a plain `git fetch` in the main checkout runs. This is the choice.

The handle runs with the variables that would redirect git elsewhere (`GIT_DIR`, `GIT_WORK_TREE` and the like) cleared. The repository is found once, from the starting directory with the caller's environment honoured; after that, a relative `GIT_DIR` would be read against the wrong directory.

## Locating the main worktree

`git worktree list` names the main worktree only in the usual layout. When the git directory lies elsewhere, as with `--separate-git-dir` or a submodule's `.git/modules/<name>`, it lists the git directory instead, which is not a working tree. The tool therefore locates the main worktree itself, in this order, when a command starts:

1. A bare repository has none; commands run in the git directory.
2. A git directory that names its working tree with `core.worktree`, as a submodule's does: `git rev-parse --show-toplevel` run in the git directory returns it.
3. A git directory named `.git`: its parent, once git confirms that the parent's git directory is this one and that the parent is the top of that worktree.
4. The tool was started in the main worktree, which git recognises by its git directory being the common one: the top of the starting worktree.
5. The update in progress recorded it: `start` writes the main worktree into the update state, and it is used once git confirms it as in step 3. `start` needs the main checkout to check that it is clean, so in this layout it runs there or refuses, and every later command of the update finds the main worktree in the state.

Only a repository whose git directory lies outside its checkout without naming it, with no update in progress, run from a linked worktree, is left without a main worktree. Git itself cannot locate it from there. Commands that need the main checkout (`start`, which checks that it is clean, a `finish` that would move a branch checked out there, and `doctor`'s checkout checks) refuse, or fail the check, and say to run the command in the main worktree. The other commands run in the git directory, where a relative remote URL is resolved against the git directory.

`git worktree list` is still read for the linked worktrees and the branches checked out; its first entry is replaced by the located main worktree.

## What still depends on the starting directory

Three things do, by design, and all are read where the tool starts, which `start` never removes:

- The integration branch is found from the configuration committed at `HEAD` of the worktree the tool runs in when no update is in progress. The commit at `HEAD` is resolved when the command starts.
- The note about an uncommitted configuration edit compares against that worktree's copy.
- A revision named to `start` is resolved there, so `HEAD~1`, `@{-1}` or `ORIG_HEAD` typed in a linked worktree mean that worktree's. Tags, ancestry and the remote are still checked in the repository as a whole.

## Refusing to run from the update worktree

Refusing `finish` and `abort` in the update worktree would avoid the removed directory, but it would forbid the place where a user resolves conflicts and runs `continue` and `verify`, and it would leave the relative-URL inconsistency. A command line tool cannot change its caller's directory, so the command completes and, after its own output, prints a note instead.
