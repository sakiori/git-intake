## Where repository-wide git commands run

Before this change every git command that is not bound to a specific worktree ran in the directory the tool was started in. That directory is not stable: `finish`, `abort` and the opening of a merge request remove the update worktree, and the user may be standing in it.

The commands in question (fetch, push, ref updates, object and history queries, configuration, remotes, and worktree management) need a repository, not a working tree. Three places can run them:

- The starting directory, as before. It fails as soon as the update worktree it lies in is removed, and it gives a relative remote URL a different meaning in each worktree.
- The common git directory. It always exists and needs no working tree, but git treats it like a bare repository: a relative remote URL is resolved against the git directory itself, so `../upstream.git`, which works from the main checkout, would stop working everywhere.
- The main worktree, or the repository itself when it is bare. It exists for as long as the repository does, it is where a plain `git fetch` in the main checkout runs, and for a bare repository it is the common git directory. This is the choice.

The main worktree is taken from `git worktree list`, whose first entry is the main worktree or the bare repository. It is used only after git confirms that it belongs to the same repository; if the main worktree is gone or has been replaced, the commands run in the common git directory, which is correct for everything except a relative remote URL.

The handle runs with the variables that would redirect git elsewhere (`GIT_DIR`, `GIT_WORK_TREE` and the like) cleared. The repository is found once, from the starting directory with the caller's environment honoured; after that, a relative `GIT_DIR` would be read against the wrong directory.

## What still depends on the starting directory

Two things do, by design: the integration branch is found from the configuration committed at `HEAD` of the worktree the tool runs in when no update is in progress, and the note about an uncommitted configuration edit compares against that worktree's copy. Both are read when the command starts, so the commit at `HEAD` is resolved then and not looked up again later.

## Refusing to run from the update worktree

Refusing `finish` and `abort` in the update worktree would avoid the removed directory, but it would forbid the place where a user resolves conflicts and runs `continue` and `verify`, and it would leave the relative-URL inconsistency. A command line tool cannot change its caller's directory, so the command completes and prints a note instead.
