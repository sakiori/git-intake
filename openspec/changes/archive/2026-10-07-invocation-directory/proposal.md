## Why

`finish` run with the current directory inside the update worktree advances the integration branch and then fails while cleaning up: once the worktree is removed, the next git command still runs in the removed directory and cannot start. The update is left in its finishing step, with its branch and state behind, and `doctor` reports a missing worktree until `finish` is run again from elsewhere. `abort` and the opening of a merge request fail the same way. The update worktree is where conflicts are resolved and where a user naturally stands when the update is ready, so the tool must not depend on being started somewhere else.

The same cause makes a relative remote URL mean different things depending on where the tool runs: git resolves it against the worktree it runs in, so from the update worktree it points at a path inside the git directory.

## What changes

Every command behaves the same whichever directory of the repository it runs in: the main checkout, a subdirectory, a linked worktree, or the update worktree, and whether the git directory is `.git` in the main worktree, lies elsewhere (`--separate-git-dir`), or belongs to a submodule. What depends on the starting directory, the commit at its `HEAD`, the top of its worktree and the revisions named to `start`, is read there. Git operations on the repository as a whole run in the main worktree, or in the git directory when the repository is bare, so they never run in a directory the command removes, and a relative remote URL means what it means in the main worktree. A command that removes the update worktree it was started in completes and, after its own output, notes that the current directory is gone.

A repository whose git directory lies outside its checkout was not usable before: `start` looked for the main checkout in the git directory. The tool now locates the main worktree in every layout git can describe, and refuses, naming the reason, what needs the main checkout when it cannot be located.

## Capabilities

### New capabilities

None.

### Modified capabilities

- `git-intake`: commands behave the same from every directory of the repository, including the update worktree.

## Impact

No command-line or configuration change. The update state gains an optional field with the main worktree; a state written by the previous release is still read. A relative remote URL is now always resolved against the main worktree; before, it was resolved against whichever worktree the tool ran in, which agreed with the main worktree only for a worktree placed beside it.
