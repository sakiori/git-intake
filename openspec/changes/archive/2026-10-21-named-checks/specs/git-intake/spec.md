## ADDED Requirements

### Requirement: Named checks
Each check SHALL be a table with exactly the keys `name` and `run`, the command. Names SHALL be unique ignoring letter case and contain only ASCII letters, digits, `-`, `_` and `.`, not starting with `-` or `.`. The tool SHALL refuse a check given as a plain command string, showing the table form for it, and a check with another key, without a name or a command, with an empty command, with a name that breaks the rule, or with the name of another check, naming the check.

#### Scenario: Checks as plain commands
- **WHEN** the configuration has `checks = ["make lint", "make test"]`
- **THEN** the tool refuses to run, and its message shows `checks = [{ name = "...", run = "make lint" }, ...]` as the form to use

#### Scenario: Duplicate check name
- **WHEN** one check is named `test` and another `Test`
- **THEN** the tool refuses to run and names both

#### Scenario: Unknown key
- **WHEN** a check is written `{ name = "lint", cmd = "make lint" }`
- **THEN** the tool refuses to run and names the check and the key

### Requirement: Selecting checks
`verify` SHALL accept `--only <name>` or `--skip <name>`, each repeatable, never both together, and run the install command and only the checks selected, in the order of the update's checks, under the preconditions and with the environment of a full run. A name given twice SHALL count once. A selection that leaves out no check SHALL be a full run; one that skips every check SHALL run only the install command.

#### Scenario: Only one check
- **WHEN** the update's checks are `lint` and `test`, and `verify --only test` runs
- **THEN** the install command and `test` run and `lint` does not, and the output reports `test` by its name

#### Scenario: Selection of every check
- **WHEN** the update's checks are `lint` and `test`, a partial run is recorded, and `verify --only lint --only test` runs and both pass
- **THEN** the update is verified as by a full `verify`, and no partial run remains

#### Scenario: Check added by a catch-up
- **WHEN** a catch-up merge brings a configuration with the new check `audit`
- **THEN** `verify --only audit` runs it, and a run that leaves it out is partial

### Requirement: Refused selections
`verify` SHALL refuse with exit status 2, running nothing, `--only` and `--skip` given together and a name that is not one of the update's checks, naming the checks it has.

#### Scenario: Unknown name
- **WHEN** the update's checks are `lint` and `test`, and `verify --skip tests` runs
- **THEN** `verify` runs nothing, exits with status 2, and names `lint` and `test`

#### Scenario: Both options
- **WHEN** `verify --only lint --skip test` runs
- **THEN** `verify` runs nothing and exits with status 2

### Requirement: Partial runs
A run that leaves out a check SHALL be recorded as the update's partial run, with its commit, the checks it ran and their outcomes, and logs of its own, beside the full verification. It SHALL never replace, change or complete the full verification, and partial runs SHALL not add up to a full one: after a partial run the update SHALL be verified exactly when it was before. A later partial run SHALL replace the earlier one with its logs; a full run and a catch-up merge SHALL drop it with its logs.

#### Scenario: Partial run after a full verification
- **WHEN** `verify` passed on the head, and `verify --only lint` then fails on the same head
- **THEN** the update stays verified, `status` shows the verified commit and the failed partial run, and `finish` publishes in push mode

#### Scenario: Partial runs do not add up
- **WHEN** the update's checks are `lint` and `test`, and `verify --only lint` and then `verify --only test` pass on the same unverified head
- **THEN** the update is not verified, and `status` shows only the second partial run

#### Scenario: Logs of a full verification are kept
- **WHEN** `verify --only test` runs after a full `verify`
- **THEN** the logs of the full verification are still there, and the partial run writes its own apart from them

### Requirement: Verification logs
Each run SHALL write the install command's log and one log per check it runs, named after the check, so that no check's log can take the install command's place; a partial run SHALL write its logs apart from the full verification's. `abort`, and `finish` once it removes the update, SHALL remove the logs of both.

#### Scenario: Check named install
- **WHEN** a check is named `install`
- **THEN** its log and the install command's log are two files

### Requirement: Outcome of a partial run
After a partial run, `verify` SHALL exit as a full run with the same outcomes would, say that this run did not verify the update, say when a full run verified the head that the update stays verified at that commit, and otherwise name `verify` without a selection as the way to verify it.

#### Scenario: Passing partial run
- **WHEN** `verify --only lint` passes on an unverified head
- **THEN** `verify` exits with status 0, says that the update is not verified, and names `git intake verify`

#### Scenario: Partial run on a verified head
- **WHEN** `verify --only lint` passes on a head a full run verified
- **THEN** `verify` says that this run did not verify the update and that the update stays verified at that commit

### Requirement: Partial runs in status
`status` SHALL list the update's checks by name and show the partial run, with its commit, the checks it ran and left out, and the outcome of each, apart from the verified commit.

#### Scenario: Partial run in the text form
- **WHEN** the update's checks are `lint` and `test`, the head is unverified, and `status` runs after `verify --only lint` failed on it
- **THEN** it lists `lint` and `test`, shows the partial run at the head with `lint` failed and `test` left out, and shows no verified commit

### Requirement: Partial runs in JSON
`status --json` SHALL carry `checks` as a list of objects with `name` and `run`, and `update.partial_run` with `commit`, `install` as the install command's outcome, or null when no install command is configured, `checks` as a list of objects with `name` and `outcome`, `left_out` as the names left out and `passed`, or null without one. An outcome SHALL be `passed`, `failed`, or `skipped` when the install command failed; a check left out SHALL appear only in `left_out`.

#### Scenario: Partial run in JSON
- **WHEN** a full run verified the head, and `status --json` runs after `verify --skip test` then passed on it
- **THEN** `update.partial_run` carries the head, `lint` with the outcome `passed`, `test` in `left_out`, and `passed` true, and `update.verified_commit` is still the head

### Requirement: Partial runs and finish
In push mode, and with `--no-publish`, `finish` on an update whose head has only a partial run SHALL refuse, name the checks the run left out, and name `verify`, unless the head is a catch-up merge the tool made and has not verified, which `finish` verifies itself as "Guarded publication" says, dropping the partial run.

#### Scenario: Partial run only
- **WHEN** the update's checks are `lint` and `test`, and only `verify --only lint` passed on the head, in push mode
- **THEN** `finish` exits with status 2, names `test` as left out, names `verify`, and moves no branch

#### Scenario: Partial run on a catch-up merge
- **WHEN** `finish` stopped on a catch-up conflict, `continue` concluded it, and `verify --only lint` passed on the catch-up merge, in push mode
- **THEN** `finish` runs every check on the catch-up merge and publishes it when they pass

## MODIFIED Requirements

### Requirement: Fork configuration
The tool SHALL read a repository's settings from a `.git-intake.toml` file committed on the integration branch: the integration branch, an optional publication setting with the publish remote, the mode `push` or `merge-request`, and, for merge-request mode, an optional forge `gitlab` or `github` and an optional API base URL, an install command, an ordered list of named checks as "Named checks" describes them, and an ordered list of upstreams. Each upstream SHALL have a unique name, a URL, a branch, a mode `follow` or `port` with `follow` as the default, optionally a path inside the repository, and, in `follow` mode, optionally a `tags` pattern: a glob over the upstream's tag names that selects its release tags. No two upstreams SHALL have the same URL and branch. An upstream without a path is taken at the repository root. The tool SHALL read the committed file from the integration branch tip, not from a working tree, and SHALL refuse to run when the file is missing or invalid, naming the problem, except where "Resumable state" lets `finish` and `abort` act on an update waiting from an earlier release. It SHALL refuse the settings `remote` and `mirror`, naming what replaces them, an upstream in `port` mode with a path or a `tags` pattern, a `tags` pattern that is not a valid glob or that starts with `refs/` or `tags/`, a forge other than `gitlab` or `github`, an API base that is not an `https` URL, and a publish remote whose fetch or push URL names a configured upstream's repository.

#### Scenario: Valid configuration
- **WHEN** the integration branch carries a valid configuration
- **THEN** every command uses its values and `status` shows them

#### Scenario: Uncommitted edits are ignored
- **WHEN** the working tree holds an edited but uncommitted configuration
- **THEN** the tool uses the committed version and says that the working copy differs

#### Scenario: Stacked downstream branch
- **WHEN** a repository's configuration names another repository's integration branch as its upstream
- **THEN** updates take that branch into the local integration branch the same way as an upstream project

#### Scenario: Several upstreams at their paths
- **WHEN** the configuration lists several upstreams, each with its own path
- **THEN** every command treats each upstream as a separate source merged under its path

#### Scenario: Ambiguous layout
- **WHEN** two upstreams share a name or the same URL and branch, one upstream's path lies inside another's, or an upstream without a path is listed together with other upstreams
- **THEN** the tool refuses to run and names the conflicting entries

#### Scenario: Publication setting
- **WHEN** the configuration names a publish remote without a mode
- **THEN** the tool publishes by `push`

#### Scenario: Forge setting
- **WHEN** the publication setting names the forge `gitea`, or an API base `http://forge.example/api/v4`
- **THEN** the tool refuses to run and names the setting and the values it accepts

#### Scenario: Removed setting
- **WHEN** an upstream has a `remote` or a `mirror` setting
- **THEN** the tool refuses to run, names the upstream and the setting, and says that upstreams are fetched by their URL and that mirror branches are no longer kept

#### Scenario: Port upstream under a path
- **WHEN** an upstream in `port` mode has a path
- **THEN** the tool refuses to run and names the upstream

#### Scenario: Invalid release tag pattern
- **WHEN** an upstream's `tags` pattern is not a valid glob, starts with `refs/tags/`, or belongs to an upstream in `port` mode
- **THEN** the tool refuses to run and names the upstream and the pattern

#### Scenario: Publish remote at an upstream's URL
- **WHEN** the publish remote's fetch or push URL names a configured upstream's repository
- **THEN** the tool refuses to run and names the remote and the upstream

### Requirement: Verification of the published tree
`verify` SHALL require the configuration at the update worktree head to be valid and to name the same integration branch, then run the install command and the update's checks, all of them or those a selection names, in the update worktree once every queued upstream is merged or its plan carried out, write each command's output to a log named after the install command or the check, report pass or fail per command, naming each check by its name, and record the worktree commit it verified. In a full run, a failed check, or one skipped because the install command failed, SHALL leave the update unverified. The checks of an update SHALL be those of the configuration it recorded: the one at the integration tip at `start`, or at its last catch-up merge. A full run SHALL run every one of them, and only a full run SHALL verify the update; wherever an update is required to be verified, a partial run SHALL not count.

#### Scenario: Checks pass
- **WHEN** every check succeeds
- **THEN** the tool records the verified commit and names `finish` as the next step

#### Scenario: Commit after verification
- **WHEN** a commit is added to the update branch after `verify`, in push mode
- **THEN** `finish` refuses until `verify` passes for the new commit

#### Scenario: Upstreams still pending
- **WHEN** `verify` runs while a queued upstream is at its plan, in conflict or pending
- **THEN** the tool refuses and names the upstreams that still need `continue`

### Requirement: Resumable state
The tool SHALL keep the state of an update in the repository's common git directory, visible from every worktree, and the update worktree at `.worktrees/git-intake/update` in the main worktree, never inside the git directory. Unless that path is ignored already, `start` and `sync` SHALL add the line `/.worktrees/` to the repository's `info/exclude` once, without changing a committed ignore file, so that the main checkout does not show the update worktree. Every command SHALL act on that state: `continue` carries out a plan, concludes a resolved merge, copy or catch-up merge, and goes on with the remaining queued upstreams or planned commits; `status` shows the step reached, each queued upstream as merged, at its plan, in conflict or pending, a catch-up merge in conflict or, where verification is required, awaiting it, and what comes next; and `abort` removes the worktree, the update branch, the plan, the update's upstream references, its logs and the state without touching the integration branch. A `follow` upstream SHALL count as merged only when the history of the update branch head carries its sync record for its target, and a `port` upstream as carried out only when its plan has been carried out to the journal commit. Once `finish` has begun moving branches, `abort` SHALL refuse and name `finish` as the way to complete the update. The tool SHALL read an update state of format 4 or 5 only when the update waits for its merge request, as "Update waiting from an earlier release" hands it over: it SHALL read only what the hand-over and `abort` need, ignoring the checks and the verification the state records, SHALL hand it over and abort it while the committed configuration is invalid, using the publication setting the update recorded, and SHALL never write it. It SHALL refuse any other update state of format 4 or 5, and an update state in any other format than its own, older or newer, or without a format: name both formats and, where the state records it, the release that started the update, and advise finishing or aborting the update with a release that keeps that format, naming the releases of each format (1: v0.1.0; 2: v0.2.0 to v0.2.2; 3: v0.3.0 to v0.3.1; 4: v0.3.2 to v0.3.5, also carried on by v0.3.6; 5: v0.3.6; 6: from v0.3.7), or for a newer format with the release that started it or a later one that reads it. For format 3, the refusal SHALL also say that v0.3.6 carries the update over with `migrate` and can then finish it. Only `start` and `sync` SHALL record the release that started the update in the state. The first of `continue`, `verify` and `finish` run by another release of the same format SHALL warn once, without refusing, and record that it warned; it SHALL advise aborting and starting again only before `finish` has moved a branch or pushed the update.

#### Scenario: Update worktree in the main worktree
- **WHEN** `start` creates the update worktree
- **THEN** it is `.worktrees/git-intake/update` in the main worktree, outside the git directory, and the main checkout shows no untracked file for it and counts as clean

#### Scenario: Directory of the update worktree that is a link
- **WHEN** `.worktrees` or `.worktrees/git-intake` in the main worktree is a symbolic link or not a directory
- **THEN** `start` refuses before it changes anything

#### Scenario: Update worktree already ignored
- **WHEN** the main worktree already ignores `.worktrees/git-intake/update`, or `info/exclude` already holds `/.worktrees/`
- **THEN** `start` leaves `info/exclude` unchanged

#### Scenario: Resume after interruption
- **WHEN** the user returns to an update in a new shell
- **THEN** `status` names each upstream's target or plan, the base, the worktree, the state of every queued upstream and the next command

#### Scenario: Continue after a conflict
- **WHEN** the user resolves a conflict and runs `continue` while other upstreams or planned commits are pending
- **THEN** the tool concludes the merge or copy and goes on with the pending ones in order, stopping again at the next conflict

#### Scenario: Abort
- **WHEN** the user aborts an update before `finish` has moved any branch
- **THEN** the integration branch is where `start` or the last fast-forward to the publish remote left it, and no worktree, update branch, plan, upstream reference or log of the update remains

#### Scenario: User's worktree on the update branch
- **WHEN** no update is in progress and a worktree other than the tool's update worktree has `git-intake/update` checked out
- **THEN** `abort`, `start` and `sync` refuse and `doctor` fails, each naming that worktree and saying to remove or switch it, and the worktree and its uncommitted changes are kept

#### Scenario: State of the previous release
- **WHEN** an update started by a release with state format 5 is in progress and does not wait for its merge request
- **THEN** every command that reads the update refuses, changes nothing, and says to finish or abort it with v0.3.6

#### Scenario: Hand-over with the old configuration
- **WHEN** an update of format 5 waits for its open merge request, the integration tip still lists its checks as plain commands, and `finish` runs
- **THEN** `finish` hands the update over, and the state file is unchanged until it is removed

#### Scenario: Withdrawal with the old configuration
- **WHEN** an update of format 5 waits for its open merge request, the integration tip still lists its checks as plain commands, and `abort --merge-request` runs
- **THEN** it withdraws the merge request, and the state file is unchanged until it is removed

#### Scenario: State of format 4
- **WHEN** an update started by a release with state format 4 is in progress and does not wait for its merge request
- **THEN** every command that reads the update refuses, changes nothing, and names v0.3.2 to v0.3.6 as the releases to finish or abort it with

#### Scenario: State that needs migration
- **WHEN** an update started by a release with state format 3, which kept the update worktree in the git directory, is in progress
- **THEN** every command that reads the update refuses, changes nothing, and names finishing or aborting it with a release that keeps format 3, or carrying it over with `migrate` of v0.3.6 and finishing it with that release

#### Scenario: State of an older release
- **WHEN** an update started by a release with a state format older than 3 is in progress
- **THEN** every command that reads the update refuses and says to finish or abort it with a release that keeps that format

#### Scenario: State of a newer release
- **WHEN** an update started by a release with a newer state format is in progress
- **THEN** every command that reads the update refuses, names that release when the state records it, says that this release is older, and changes nothing

#### Scenario: Another release of the same format
- **WHEN** an update started by one release is carried on by another release with the same state format
- **THEN** the first of `continue`, `verify` and `finish` it runs warns once that another release started it, advises `abort` only before `finish` has moved a branch or pushed the update, and every command carries on

### Requirement: Changes to the update's own checks
When an update changes the forge's CI definition, `.github/workflows/` on GitHub or the project's CI configuration file on GitLab, or the install command or checks of `.git-intake.toml`, `finish` and `sync` SHALL name those changes in their output and in the merge request's description, naming each check added, removed or given another command by its name, and saying when only their order changed; a renamed check SHALL count as one removed and one added, and when `.git-intake.toml` differs between the base and the head and either side cannot be read, its install command and checks SHALL be named as changed without names, since the pipeline that verifies the update then runs what the update brings.

#### Scenario: Upstream changes the pipeline
- **WHEN** a root upstream's update changes `.gitlab-ci.yml`
- **THEN** `sync` names the change in its output and in the merge request's description

#### Scenario: Upstream changes a check
- **WHEN** a root upstream's update adds the check `audit` and changes the command of the check `test` in `.git-intake.toml`
- **THEN** `finish` names the checks `audit` as added and `test` as changed in its output and in the merge request's description

### Requirement: Shallow repository
Every command other than `abort` SHALL refuse in a shallow repository before it changes anything, saying that the integration line needs the full history and naming `git fetch --unshallow`.

#### Scenario: Shallow CI clone
- **WHEN** `sync` or `env` runs in a clone made with a limited depth
- **THEN** it refuses, changes nothing, and names how to fetch the full history

## REMOVED Requirements

### Requirement: Migration of an update in progress
**Reason**: The update state format becomes 6, whose configuration carries named checks, so a format 3 update can no longer be carried over to the current format; the configuration it was started with has checks without names, which this release does not read.
**Migration**: Carry a format 3 update over with `migrate` of v0.3.6 and finish or abort it with that release before upgrading.
