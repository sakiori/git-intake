## ADDED Requirements

### Requirement: Place of the update worktree in a tree
Before any branch moves, `finish` SHALL refuse an update whose head, after a merge, copy or catch-up, has anything but a directory at `.worktrees` or `.worktrees/git-intake`, or anything at `.worktrees/git-intake/update`, naming the path and why. A checkout of a branch in the main worktree SHALL never follow a move to such a tree. Names SHALL be compared without case where the checkout ignores case.

#### Scenario: Upstream brings a link
- **WHEN** an upstream's tree has `.worktrees` as a symbolic link, and `finish` runs on the verified update
- **THEN** it refuses, names `.worktrees` and says it is a symbolic link where the update worktree is kept, and neither the integration branch nor the publish remote moves

#### Scenario: Upstream brings a file at the update worktree
- **WHEN** an upstream's tree has a file at `.worktrees/git-intake/update`
- **THEN** `finish` refuses and names `.worktrees/git-intake/update`, and no branch moves

### Requirement: Removing only what the tool created
Before it removes the update worktree, the tool SHALL check that its path is the tool's placement, reached from the main worktree or git directory through directories alone, and that git registers it there for the repository. It SHALL refuse to remove a registered worktree that fails this check, removing nothing, and SHALL leave and name a directory git does not register, unless it is empty.

#### Scenario: Registered worktree reached through a link
- **WHEN** `.worktrees` in the main worktree has become a symbolic link to the directory holding the update worktree, and `abort` runs
- **THEN** `abort` refuses, names `.worktrees` as a symbolic link, and the worktree and the update branch are kept

#### Scenario: Unregistered directory reached through a link
- **WHEN** git no longer registers the update worktree, `.worktrees` is a symbolic link to a directory outside the repository, and `finish --no-publish` closes the update
- **THEN** the update closes, the directory outside the repository keeps its files, and the tool warns that it left the path, naming it and the link

#### Scenario: Leftover directory at the placement
- **WHEN** a directory git does not register lies at `.worktrees/git-intake/update`, and `start` runs
- **THEN** `start` refuses, naming the directory and saying to remove it

### Requirement: Removing the tool's files
The tool SHALL remove its state, plan and logs only inside its directory in the repository's common git directory, reached through directories alone, and SHALL refuse, removing nothing, when a directory on the way is a symbolic link or not a directory. A symbolic link inside it SHALL be removed itself, never what it points to.

#### Scenario: Tool directory that is a link
- **WHEN** `<git-common-dir>/git-intake` is a symbolic link to another directory, and the update's logs are removed
- **THEN** the tool refuses, and the other directory keeps its files

### Requirement: Pushes never reach an upstream
Before every push and every deletion of a branch on the publish remote, the tool SHALL resolve where git pushes, the remote's push URLs or else its URLs after git's rewriting, or a URL by the rule `git push` applies to it, and SHALL refuse, pushing nothing and moving no branch, when one names an upstream's repository, naming the remote, the URL and the upstream.

#### Scenario: Remote changed during an update
- **WHEN** after `start` the publish remote is given a push URL, or a `url.<base>.pushInsteadOf` rewriting, that names the upstream's repository, and `finish` runs
- **THEN** `finish` refuses, the integration branch stays at the base, and nothing reaches the upstream's repository

#### Scenario: Alias in sync
- **WHEN** the publish remote's push URL is an `insteadOf` alias, an `https` URL with credentials or a `file://localhost/` URL of the upstream's repository, and `sync` runs
- **THEN** `sync` refuses before it starts an update, and nothing reaches the upstream's repository

### Requirement: Messages without credentials
Every message the tool shows, in text or JSON, on standard output or standard error, SHALL pass one sanitising step that removes the password of every URL and the user of an `http` or `https` URL, and replaces every value of a token variable the tool reads, wherever it appears. Git command lines in messages, including those of git commands that cannot be started, SHALL pass the same step.

#### Scenario: Forge quotes the token
- **WHEN** the forge answers with a message that contains the token, and `doctor`, `doctor --json` or `finish` reports it
- **THEN** the message is shown with `[hidden]` in place of the token

#### Scenario: API URL with credentials
- **WHEN** the setting `api` is an `https` URL with a user and password
- **THEN** the tool refuses it, shows the URL without them, and neither `status`, `doctor` nor their `--json` forms show them

#### Scenario: Git cannot be started
- **WHEN** a git command whose arguments hold a URL with credentials cannot be started
- **THEN** the message names the command without the credentials

## MODIFIED Requirements

### Requirement: Same repository by URL
Where the tool compares a remote or URL with an upstream's URL, it SHALL compare each as configured and as git resolves it with its URL rewriting, and URLs naming the same repository SHALL count as equal: local paths and `file://` URLs, whatever their host, resolved against the main worktree as git finds a repository and made canonical; other URLs, the scp-like form read as `ssh`, by scheme, host in lower case, port unless default and path without end `/` and `.git`, credentials ignored.

#### Scenario: Relative and absolute paths
- **WHEN** the publish remote's URL is a relative path to an upstream's repository, configured by its absolute path
- **THEN** the tool refuses to run and names the remote and the upstream

#### Scenario: Path without its `.git`
- **WHEN** the publish remote's URL is `../upstream` and an upstream's repository is the directory `../upstream.git`
- **THEN** the tool refuses to run and names the remote and the upstream

#### Scenario: Scp-like and ssh URLs
- **WHEN** an upstream's URL is `git@example.com:team/up.git` and the publish remote's is `ssh://git@example.com:22/team/up`
- **THEN** the tool refuses to run and names the remote and the upstream

#### Scenario: Hosted URL written differently
- **WHEN** a git remote's URL differs from an upstream's URL only by a trailing `/` or `.git`, or by the case of the host
- **THEN** `doctor` reports the remote as having the upstream's URL

#### Scenario: Rewritten URL
- **WHEN** the publish remote's push URL is an alias that `url.<base>.insteadOf` rewrites to an upstream's URL
- **THEN** the tool refuses to run and names the remote, its URL and what git rewrites it to, and the upstream

#### Scenario: URL with credentials or a local host
- **WHEN** the publish remote's push URL is an upstream's `https` URL with a user and password, or `file://localhost/` and the path of an upstream's repository
- **THEN** the tool refuses to run, names the remote and the upstream, and shows no credentials
