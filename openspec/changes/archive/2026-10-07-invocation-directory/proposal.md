## Why

`finish` run with the current directory inside the update worktree advances the integration branch and then fails while cleaning up: once the worktree is removed, the next git command still runs in the removed directory and cannot start. The update is left in its finishing step, with its branch and state behind, and `doctor` reports a missing worktree until `finish` is run again from elsewhere. `abort` and the opening of a merge request fail the same way. The update worktree is where conflicts are resolved and where a user naturally stands when the update is ready, so the tool must not depend on being started somewhere else.

The same cause makes a relative remote URL mean different things depending on where the tool runs: git resolves it against the worktree it runs in, so from the update worktree it points at a path inside the git directory.

## What changes

Every command behaves the same whichever directory of the repository it runs in: the main checkout, a subdirectory, a linked worktree, or the update worktree. What depends on the starting directory, the commit at its `HEAD` and the top of its worktree, is resolved once when the command starts. Git operations on the repository as a whole run in the main worktree, or in the repository itself when it is bare, so they never run in a directory the command removes, and a relative remote URL means what it means in the main worktree. A command that removes the update worktree it was started in completes and notes that the current directory is gone.

## Capabilities

### New capabilities

None.

### Modified capabilities

- `git-intake`: commands behave the same from every directory of the repository, including the update worktree.

## Impact

No command-line, configuration or state format change. A relative remote URL is now always resolved against the main worktree; before, it was resolved against whichever worktree the tool ran in, which agreed with the main worktree only for a worktree placed beside it.
