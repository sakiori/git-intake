## ADDED Requirements

### Requirement: Same behaviour from every directory
Every command SHALL behave the same whichever directory of the repository it runs in: the main checkout, a subdirectory, a linked worktree, or the update worktree, whether the git directory is `.git` in the main worktree, lies outside it, or belongs to a submodule. Only finding the integration branch from the configuration committed at `HEAD` when no update is in progress, the note about an uncommitted configuration edit, and resolving the revisions named to `start` SHALL depend on the worktree the command runs in. The tool SHALL run git operations on the repository as a whole in the main worktree, or in the git directory when the repository is bare, so that a relative remote URL means what it means in the main worktree. `start` SHALL record the main worktree with the update so that the update's later commands find it from any worktree. When the main worktree cannot be located, because the git directory lies outside it without naming it, no update is in progress and the command runs elsewhere, a command that needs the main checkout SHALL refuse and say to run it in the main worktree, `doctor` SHALL fail its checkout check with that reason, and a relative remote URL SHALL resolve against the git directory. A command that removes the update worktree it runs in SHALL complete and, after its own output, SHALL note that the current directory was removed.

#### Scenario: Finish from the update worktree
- **WHEN** `finish` runs in the update worktree, with or without a publish remote, with `--no-publish`, or opening a merge request
- **THEN** it moves, publishes and cleans up exactly as from the main checkout, and notes that the current directory was removed

#### Scenario: Abort from the update worktree
- **WHEN** `abort` runs in a directory of the update worktree
- **THEN** no worktree, update branch, log or state of the update remains, and the tool notes that the current directory was removed

#### Scenario: Relative remote URL
- **WHEN** an upstream's URL is a relative path and `start` runs in a linked worktree, or a command fetches it from the update worktree
- **THEN** the URL is resolved against the main worktree, as when the command runs in the main checkout

#### Scenario: Git directory outside the checkout
- **WHEN** the downstream is a submodule, or its git directory was placed elsewhere, and an update is started in its checkout and finished from the update worktree
- **THEN** relative remote URLs resolve against the checkout, and `finish` fast-forwards the checkout

#### Scenario: Main worktree that cannot be located
- **WHEN** the git directory lies outside the checkout without naming it, no update is in progress, and `start` runs in a linked worktree
- **THEN** `start` refuses and says to run it in the main worktree

#### Scenario: Revision named in a linked worktree
- **WHEN** `start` runs in a linked worktree with a revision relative to that worktree, such as `HEAD^2`
- **THEN** the revision is resolved in that worktree

#### Scenario: Linked checkout of the integration branch
- **WHEN** a linked worktree has the integration branch checked out and `finish` runs in the update worktree
- **THEN** that checkout follows the fast-forward and stays clean
