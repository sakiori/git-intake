## ADDED Requirements

### Requirement: Fork configuration
The tool SHALL read a fork's settings from a `.fork-sync.toml` file committed on the integration branch: the upstream remote and branch, an optional mirror branch, the integration branch, an optional publish remote, an install command, and an ordered list of check commands. It SHALL read the committed file from the integration branch tip, not from a working tree, and SHALL refuse to run when the file is missing or invalid, naming the problem.

#### Scenario: Valid configuration
- **WHEN** the integration branch carries a valid configuration
- **THEN** every command uses its values and `status` shows them

#### Scenario: Uncommitted edits are ignored
- **WHEN** the working tree holds an edited but uncommitted configuration
- **THEN** the tool uses the committed version and says that the working copy differs

#### Scenario: Stacked downstream branch
- **WHEN** a repository's configuration names another repository's integration branch as its upstream and has no mirror branch
- **THEN** updates merge that branch into the local integration branch the same way as an upstream project

### Requirement: Fixed target and isolated merge
`start` SHALL fetch the upstream remote, record the fetched upstream tip as the target and the current integration tip as the base, create a dedicated worktree on a new branch from the base, and merge the target into it with a merge commit. The target SHALL NOT change for the rest of the update. `start` SHALL refuse when an update is already in progress, when the main checkout or the integration branch has uncommitted changes, or when the integration branch is behind its published counterpart.

#### Scenario: Clean merge
- **WHEN** the target merges without conflicts
- **THEN** the worktree holds the merge commit and the next step is `verify`

#### Scenario: Conflicts
- **WHEN** the merge stops on conflicts
- **THEN** the tool lists the conflicted files, leaves the merge in progress in the worktree, and tells the user to resolve them and run `continue`

#### Scenario: Upstream moved during the update
- **WHEN** upstream receives new commits after `start`
- **THEN** the update still merges, verifies and publishes the recorded target

#### Scenario: Nothing to update
- **WHEN** the integration branch already contains the target
- **THEN** `start` reports that the fork is current and creates nothing

### Requirement: Recorded resolutions
The tool SHALL enable git's recorded conflict resolution for the repository before merging, so a resolution made in one update or attempt is reapplied when the same conflict recurs.

#### Scenario: Retried update
- **WHEN** an aborted update is started again and hits a conflict resolved in the earlier attempt
- **THEN** git reapplies the recorded resolution and the tool reports the file as resolved by a recorded resolution

### Requirement: Resumable state
The tool SHALL keep the state of an update in the repository's common git directory, visible from every worktree, and every command SHALL act on that state: `continue` concludes a resolved merge, `status` shows the step reached and what comes next, and `abort` removes the worktree, the update branch and the state without touching the integration or mirror branch.

#### Scenario: Resume after interruption
- **WHEN** the user returns to an update in a new shell
- **THEN** `status` names the target, the base, the worktree and the next command

#### Scenario: Abort
- **WHEN** the user aborts an update at any step before `finish`
- **THEN** the integration and mirror branches are as before `start` and no worktree or update branch remains

### Requirement: Verification of the published tree
`verify` SHALL run the configured install command and every check in the update worktree, write each command's output to a log, report pass or fail per command, and record the worktree commit it verified. A failed or skipped check SHALL leave the update unverified.

#### Scenario: Checks pass
- **WHEN** every check succeeds
- **THEN** the tool records the verified commit and names `finish` as the next step

#### Scenario: Commit after verification
- **WHEN** a commit is added to the update branch after `verify`
- **THEN** `finish` refuses until `verify` passes for the new commit

### Requirement: Guarded publication
`finish` SHALL require a verified update whose worktree commit is the verified one and whose base is still the integration tip. It SHALL fast-forward the integration branch to the verified commit and the mirror branch to the target, push both without force when a publish remote is configured, then remove the worktree, the update branch and the state. It SHALL never rewrite or force-push a branch.

#### Scenario: Integration branch moved
- **WHEN** a feature lands on the integration branch after `start`
- **THEN** `finish` refuses and explains that the update must merge the new integration tip and be verified again

#### Scenario: Remote moved
- **WHEN** the publish remote's branch is ahead of the local one
- **THEN** the push is rejected, the local branches stay advanced, and the tool reports how to reconcile without force

### Requirement: Update report
`report` SHALL summarise an update in progress or just merged: the number of upstream commits merged, the conflicted files and whether each was resolved by hand or by a recorded resolution, and how the fork's own changes relative to upstream grew or shrank.

#### Scenario: Report after merge
- **WHEN** the user runs `report` after resolving conflicts
- **THEN** the output lists the merged range, each conflicted file with its resolution source, and the fork's diff size before and after

### Requirement: Dry run
`status --check` SHALL report how far the integration branch is behind upstream and which files a merge of the current upstream tip would conflict in, without creating a worktree, branch or state, so that it can run on a schedule.

#### Scenario: Scheduled check
- **WHEN** `status --check` runs on a fork 30 commits behind upstream
- **THEN** it reports the 30 commits and the files that would conflict, and leaves the repository unchanged
