## ADDED Requirements

### Requirement: Same behaviour from every directory
Every command SHALL behave the same whichever directory of the repository it runs in: the main checkout, a subdirectory, a linked worktree, or the update worktree. Only finding the integration branch from the configuration committed at `HEAD` when no update is in progress, and the note about an uncommitted configuration edit, SHALL depend on the worktree the command runs in. The tool SHALL run git operations on the repository as a whole in the main worktree, or in the repository itself when it is bare, so that a relative remote URL means what it means in the main worktree. A command that removes the update worktree it runs in SHALL complete and SHALL note that the current directory was removed.

#### Scenario: Finish from the update worktree
- **WHEN** `finish` runs in the update worktree, with or without a publish remote, with `--no-publish`, or opening a merge request
- **THEN** it moves, publishes and cleans up exactly as from the main checkout, and notes that the current directory was removed

#### Scenario: Abort from the update worktree
- **WHEN** `abort` runs in a directory of the update worktree
- **THEN** no worktree, update branch, log or state of the update remains, and the tool notes that the current directory was removed

#### Scenario: Relative remote URL
- **WHEN** an upstream's URL is a relative path and a command fetches it from the update worktree
- **THEN** the URL is resolved against the main worktree, as when the command runs in the main checkout
