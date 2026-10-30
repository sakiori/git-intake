# git-intake Specification

## Purpose

Keep a repository current with one or more upstreams, each fetched by its URL only while it is needed: merged up to a fixed target at the repository root or under its own path, or taken commit by commit through a reviewed plan with a journal of decisions; with isolated updates, recorded verification, guarded publication and per-upstream status.

## Requirements

### Requirement: Fork configuration
The tool SHALL read a repository's settings from a `.git-intake.toml` file committed on the integration branch: the integration branch, an optional publication setting with the publish remote, the mode `push` or `merge-request`, and, for merge-request mode, an optional forge `gitlab` or `github` and an optional API base URL, an install command, an ordered list of named checks as "Named checks" describes them, and an ordered list of upstreams.

#### Scenario: Valid configuration
- **WHEN** the integration branch carries a valid configuration
- **THEN** every command uses its values and `status` shows them

#### Scenario: Publication setting
- **WHEN** the configuration names a publish remote without a mode
- **THEN** the tool publishes by `push`

### Requirement: Upstream settings
Each upstream SHALL have a unique name, a URL, a branch, a mode `follow` or `port` with `follow` as the default, optionally a path inside the repository, and, in `follow` mode, optionally a `tags` pattern: a glob over the upstream's tag names that selects its release tags. No two upstreams SHALL have the same URL and branch. An upstream without a path is taken at the repository root.

#### Scenario: Stacked downstream branch
- **WHEN** a repository's configuration names another repository's integration branch as its upstream
- **THEN** updates take that branch into the local integration branch the same way as an upstream project

#### Scenario: Several upstreams at their paths
- **WHEN** the configuration lists several upstreams, each with its own path
- **THEN** every command treats each upstream as a separate source merged under its path

#### Scenario: Ambiguous layout
- **WHEN** two upstreams share a name or the same URL and branch, one upstream's path lies inside another's, or an upstream without a path is listed together with other upstreams
- **THEN** the tool refuses to run and names the conflicting entries

### Requirement: Committed configuration
The tool SHALL read the committed `.git-intake.toml` from the integration branch tip, not from a working tree, and SHALL refuse to run when the file is missing or invalid, naming the problem, except where "Update state of format 4 or 5" lets `finish` and `abort` act on an update waiting from an earlier release.

#### Scenario: Uncommitted edits are ignored
- **WHEN** the working tree holds an edited but uncommitted configuration
- **THEN** the tool uses the committed version and says that the working copy differs

### Requirement: Refused settings
The tool SHALL refuse the settings `remote` and `mirror`, naming what replaces them, an upstream in `port` mode with a path or a `tags` pattern, a `tags` pattern that is not a valid glob or that starts with `refs/` or `tags/`, a forge other than `gitlab` or `github`, an API base that is not an `https` URL, and a publish remote whose fetch or push URL names a configured upstream's repository.

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

### Requirement: Sync record in history
Every merge commit the tool creates SHALL name the upstream and its target commit in commit trailers. Every commit the tool copies from an upstream in `port` mode SHALL name the upstream and the upstream commit it took in the trailers `Intake-Upstream` and `Intake-Taken`. A sync record SHALL count only on a merge commit whose second parent is the record's target.

#### Scenario: Copied commit
- **WHEN** a port update copies an upstream commit
- **THEN** the copy carries the upstream's name and the upstream commit's id in its trailers

#### Scenario: Squash message with trailers
- **WHEN** a squash commit's message ends with `Intake-Upstream` and `Intake-Target` trailers copied from an update's sync merge
- **THEN** the trailers do not count as a sync record

### Requirement: Tag in the sync record
When a tag of the upstream's repository points at the target, every merge commit the tool creates SHALL name that tag in the trailer `Intake-Tag`: the tag a revision named, the release a `tags` pattern selected, or otherwise, of the tags at the target, the one with the highest version among those matching the upstream's `tags` pattern whose name is a version after the pattern's literal prefix, or, without such a tag, among those whose name is a semantic version with an optional `v` or `V`.

#### Scenario: Target named by a tag
- **WHEN** an update merges a target that a tag of the upstream's repository points at, chosen by a release pattern, by naming the tag, or by naming the commit
- **THEN** the merge commit carries `Intake-Tag` with that tag, `report` names it, and `status` shows it as the tag of the last synced commit

#### Scenario: Target without a tag
- **WHEN** an update merges a target that no tag of the upstream's repository points at
- **THEN** the merge commit carries no `Intake-Tag` trailer

### Requirement: Recording of the upstream tag
A tag whose name is no version SHALL NOT be recorded in `Intake-Tag` as the one with the highest version, even when it matches the upstream's `tags` pattern. A tag whose name contains a comma SHALL NOT be recorded. The tag recorded in `Intake-Tag` SHALL be fixed when `start` fixes the target.

#### Scenario: Tag that is no version
- **WHEN** an update merges a target, named by its commit, at which the only tag is one that matches the upstream's `tags` pattern and whose name is no version
- **THEN** the merge commit carries no `Intake-Tag` trailer

### Requirement: Last synced commit
For an upstream in `follow` mode, the tool SHALL derive the last synced commit from its sync records on the integration line, as the target of the newest record whose target no other of its records' targets contains, and SHALL NOT store it in the configuration or outside the history.

#### Scenario: Last synced commit
- **WHEN** the integration line contains a merge of a `follow` upstream
- **THEN** `status` shows that upstream's target from the most recent such merge as its last synced commit

#### Scenario: Older record merged later
- **WHEN** a branch that carries a sync record of an older target is merged into the integration branch after a newer target's record landed
- **THEN** `status` still shows the newer target as the last synced commit, and nothing between the two targets counts as pending

### Requirement: Last synced commit without a sync record
For an upstream in `follow` mode that carries no sync record, the last synced commit SHALL come from the history, as "Last synced commit from the history" defines it, and the tool SHALL show that it does. The last synced commit taken from the history SHALL depend only on the integration branch and the upstream's own branch, never on other upstreams.

#### Scenario: Root upstream without a sync record
- **WHEN** the integration branch contains commits of a root `follow` upstream and carries no sync record of it
- **THEN** `status` shows the newest upstream commit the integration branch contains as the last synced commit, and shows that it comes from the history

### Requirement: Last synced commit from the history
For a `follow` upstream that carries no sync record, the last synced commit SHALL be, for a root upstream, the newest upstream commit the integration branch contains, and, for a path upstream, the second parent of the newest merge on the integration line whose second parent belongs to the upstream's branch and which changes the upstream's path, as an import by `git subtree add` or `git subtree pull` leaves.

#### Scenario: Path upstream imported by subtree
- **WHEN** the integration line holds a merge made by `git subtree add` of a path `follow` upstream's branch into its path, and the integration branch carries no sync record of that upstream
- **THEN** `status` shows that merge's second parent as the upstream's last synced commit

### Requirement: Rewritten follow upstream
When a `follow` upstream's last synced commit is no longer contained in its branch, the tool SHALL report that upstream's history as rewritten and SHALL refuse to update it.

#### Scenario: Upstream force-pushed
- **WHEN** a `follow` upstream's branch no longer contains the last synced commit
- **THEN** `status` reports the upstream as rewritten and `start` refuses to update it

### Requirement: Recorded resolutions
The tool SHALL enable git's recorded conflict resolution for the repository before merging or copying commits, so a resolution made in one update or attempt is reapplied when the same conflict recurs.

#### Scenario: Retried update
- **WHEN** an aborted update is started again and hits a conflict resolved in the earlier attempt
- **THEN** git reapplies the recorded resolution and the tool reports the file as resolved by a recorded resolution

### Requirement: Resumable state
The tool SHALL keep the state of an update in the repository's common git directory, visible from every worktree, and the update worktree at `.worktrees/git-intake/update` in the main worktree, never inside the git directory.

#### Scenario: Update worktree in the main worktree
- **WHEN** `start` creates the update worktree
- **THEN** it is `.worktrees/git-intake/update` in the main worktree, outside the git directory, and the main checkout shows no untracked file for it and counts as clean

#### Scenario: Directory of the update worktree that is a link
- **WHEN** `.worktrees` or `.worktrees/git-intake` in the main worktree is a symbolic link or not a directory
- **THEN** `start` refuses before it changes anything

#### Scenario: User's worktree on the update branch
- **WHEN** no update is in progress and a worktree other than the tool's update worktree has `git-intake/update` checked out
- **THEN** `abort`, `start` and `sync` refuse and `doctor` fails, each naming that worktree and saying to remove or switch it, and the worktree and its uncommitted changes are kept

### Requirement: Update worktree hidden from the main checkout
Unless the update worktree's path is ignored already, `start` and `sync` SHALL add the line `/.worktrees/` to the repository's `info/exclude` once, without changing a committed ignore file, so that the main checkout does not show the update worktree.

#### Scenario: Update worktree already ignored
- **WHEN** the main worktree already ignores `.worktrees/git-intake/update`, or `info/exclude` already holds `/.worktrees/`
- **THEN** `start` leaves `info/exclude` unchanged

### Requirement: Commands on the update state
Every command SHALL act on the update's state. `continue` SHALL carry out a plan, conclude a resolved merge, copy or catch-up merge, and go on with the remaining queued upstreams or planned commits.

#### Scenario: Continue after a conflict
- **WHEN** the user resolves a conflict and runs `continue` while other upstreams or planned commits are pending
- **THEN** the tool concludes the merge or copy and goes on with the pending ones in order, stopping again at the next conflict

### Requirement: Update state in status
`status` SHALL show the step the update reached, each queued upstream as merged, at its plan, in conflict or pending, a catch-up merge in conflict or, where verification is required, awaiting it, and what comes next.

#### Scenario: Resume after interruption
- **WHEN** the user returns to an update in a new shell
- **THEN** `status` names each upstream's target or plan, the base, the worktree, the state of every queued upstream and the next command

### Requirement: Aborting an update
`abort` SHALL remove the worktree, the update branch, the plan, the update's upstream references, its logs and the state without touching the integration branch.

#### Scenario: Abort
- **WHEN** the user aborts an update before `finish` has moved any branch
- **THEN** the integration branch is where `start` or the last fast-forward to the publish remote left it, and no worktree, update branch, plan, upstream reference or log of the update remains

### Requirement: Progress of the queued upstreams
A `follow` upstream SHALL count as merged only when the history of the update branch head carries its sync record for its target, and a `port` upstream as carried out only when its plan has been carried out to the journal commit.

#### Scenario: Merge without the sync record
- **WHEN** the update branch head's history carries no sync record of a queued `follow` upstream for its target
- **THEN** `status` does not show that upstream as merged

### Requirement: Abort during finish
Once `finish` has begun moving branches, `abort` SHALL refuse and name `finish` as the way to complete the update.

#### Scenario: Abort once finish has moved a branch
- **WHEN** `abort` runs after `finish` has begun moving branches
- **THEN** it refuses and names `finish` as the way to complete the update

### Requirement: Update state of format 4 or 5
The tool SHALL read an update state of format 4 or 5 only when the update waits for its merge request, as "Update waiting from an earlier release" hands it over: it SHALL read only what the hand-over and `abort` need, ignoring the checks and the verification the state records, SHALL hand it over and abort it while the committed configuration is invalid, using the publication setting the update recorded, and SHALL never write it.

#### Scenario: Hand-over with the old configuration
- **WHEN** an update of format 5 waits for its open merge request, the integration tip still lists its checks as plain commands, and `finish` runs
- **THEN** `finish` hands the update over, and the state file is unchanged until it is removed

#### Scenario: Withdrawal with the old configuration
- **WHEN** an update of format 5 waits for its open merge request, the integration tip still lists its checks as plain commands, and `abort --merge-request` runs
- **THEN** it withdraws the merge request, and the state file is unchanged until it is removed

### Requirement: Update state of another format
The tool SHALL refuse an update state of format 4 or 5 whose update does not wait for its merge request, and an update state in any other format than its own, older or newer, or without a format, naming both formats and, where the state records it, the release that started the update.

#### Scenario: State of a newer release
- **WHEN** an update started by a release with a newer state format is in progress
- **THEN** every command that reads the update refuses, names that release when the state records it, says that this release is older, and changes nothing

### Requirement: Releases named for a state of another format
The refusal of an update state of another format SHALL advise finishing or aborting the update with a release that keeps that format, naming the releases of each format (1: v0.1.0; 2: v0.2.0 to v0.2.2; 3: v0.3.0 to v0.3.1; 4: v0.3.2 to v0.3.5, also carried on by v0.3.6; 5: v0.3.6; 6: from v0.3.7), or for a newer format with the release that started it or a later one that reads it. For format 3, the refusal SHALL also say that v0.3.6 carries the update over with `migrate` and can then finish it.

#### Scenario: State of the previous release
- **WHEN** an update started by a release with state format 5 is in progress and does not wait for its merge request
- **THEN** every command that reads the update refuses, changes nothing, and says to finish or abort it with v0.3.6

#### Scenario: State of format 4
- **WHEN** an update started by a release with state format 4 is in progress and does not wait for its merge request
- **THEN** every command that reads the update refuses, changes nothing, and names v0.3.2 to v0.3.6 as the releases to finish or abort it with

#### Scenario: State that needs migration
- **WHEN** an update started by a release with state format 3, which kept the update worktree in the git directory, is in progress
- **THEN** every command that reads the update refuses, changes nothing, and names finishing or aborting it with a release that keeps format 3, or carrying it over with `migrate` of v0.3.6 and finishing it with that release

#### Scenario: State of an older release
- **WHEN** an update started by a release with a state format older than 3 is in progress
- **THEN** every command that reads the update refuses and says to finish or abort it with a release that keeps that format

### Requirement: Update carried on by another release
Only `start` and `sync` SHALL record the release that started the update in the state. The first of `continue`, `verify` and `finish` run by another release of the same format SHALL warn once, without refusing, and record that it warned; it SHALL advise aborting and starting again only before `finish` has moved a branch or pushed the update.

#### Scenario: Another release of the same format
- **WHEN** an update started by one release is carried on by another release with the same state format
- **THEN** the first of `continue`, `verify` and `finish` it runs warns once that another release started it, advises `abort` only before `finish` has moved a branch or pushed the update, and every command carries on

### Requirement: Verification of the published tree
`verify` SHALL require the configuration at the update worktree head to be valid and to name the same integration branch, then run the install command and the update's checks, all of them or those a selection names, in the update worktree once every queued upstream is merged or its plan carried out, write each command's output to a log named after the install command or the check, report pass or fail per command, naming each check by its name, and record the worktree commit it verified.

#### Scenario: Checks pass
- **WHEN** every check succeeds
- **THEN** the tool records the verified commit and names `finish` as the next step

#### Scenario: Commit after verification
- **WHEN** a commit is added to the update branch after `verify`, in push mode
- **THEN** `finish` refuses until `verify` passes for the new commit

#### Scenario: Upstreams still pending
- **WHEN** `verify` runs while a queued upstream is at its plan, in conflict or pending
- **THEN** the tool refuses and names the upstreams that still need `continue`

### Requirement: Checks of an update
The checks of an update SHALL be those of the configuration it recorded: the one at the integration tip at `start`, or at its last catch-up merge.

#### Scenario: Checks changed on the integration branch
- **WHEN** a commit that changes a check lands on the integration branch after `start`, and the update has made no catch-up merge
- **THEN** `verify` runs the checks of the configuration at the integration tip at `start`

### Requirement: Full runs
A full run SHALL run every one of the update's checks, and only a full run SHALL verify the update; wherever an update is required to be verified, a partial run SHALL not count. In a full run, a failed check, or one skipped because the install command failed, SHALL leave the update unverified.

#### Scenario: Check fails in a full run
- **WHEN** a check fails in a full run, or the install command fails so that the checks are skipped
- **THEN** the update stays unverified

### Requirement: Environment of the checks
`verify` SHALL run the install command and every check with environment variables that describe the update, naming commits by their full ids: `GIT_INTAKE_BASE`, the update's base, which is the integration tip the update started from or, after a catch-up merge, the integration tip it caught up with; `GIT_INTAKE_HEAD`, the commit being verified; and `GIT_INTAKE_INTEGRATION`, the integration branch's name.

#### Scenario: Follow upstream with a last synced commit
- **WHEN** an update merges a follow upstream that was synced before
- **THEN** the install command and every check receive the base, the head, the integration branch, the upstream's name, its mode, its target and its last synced commit, and each commit exists and is contained in a reference

### Requirement: Commits named to the checks
Every commit that a `GIT_INTAKE_` variable names SHALL be reachable from a reference while the install command and the checks run.

#### Scenario: Commits of a port upstream
- **WHEN** the checks run on an update that carries out a port plan
- **THEN** the base, the head and the port upstream's target each exist and are contained in a reference while the commands run

### Requirement: Upstream list of the checks
`verify` SHALL also give the install command and every check `GIT_INTAKE_UPSTREAMS`, the names of the queued upstreams separated by spaces, in queue order.

#### Scenario: Several upstreams
- **WHEN** an update takes several upstreams, and leaves out another that is current
- **THEN** `GIT_INTAKE_UPSTREAMS` lists the upstreams taken in queue order, each with its own variables, and the current upstream has none

### Requirement: Upstream variables of the checks
For each queued upstream, with `<NAME>` its name in upper case and every character other than a letter or a digit replaced by `_`, `verify` SHALL set `GIT_INTAKE_UPSTREAM_<NAME>_MODE` to `follow` or `port` and `GIT_INTAKE_UPSTREAM_<NAME>_TARGET` to the follow target, or for a port upstream to the newest upstream commit the update takes or decides on, with no target when it takes and decides none.

#### Scenario: Port upstream
- **WHEN** an update carries out a port plan whose newest commits are left `later`
- **THEN** the commands receive the mode `port` and, as the target, the newest commit the plan takes or decides on, and no previous commit

### Requirement: Previous commit of an upstream
For a follow upstream with a last synced commit before the update, `verify` SHALL set `GIT_INTAKE_UPSTREAM_<NAME>_PREVIOUS` to that commit; a port upstream SHALL get no such variable.

#### Scenario: First import
- **WHEN** an update imports a path upstream for the first time
- **THEN** the commands receive the upstream's mode and target and no previous commit

### Requirement: Check variables after a catch-up
A catch-up merge SHALL change none of the upstream variables. The verification that `finish` runs after a catch-up SHALL set the same variables.

#### Scenario: After a catch-up
- **WHEN** the checks run on a catch-up merge
- **THEN** `GIT_INTAKE_BASE` is the integration tip the update caught up with, `GIT_INTAKE_HEAD` is the catch-up merge, and every upstream variable is as before the catch-up

### Requirement: Inherited check variables
Variables with the `GIT_INTAKE_` prefix that the tool inherits SHALL NOT reach the install command and the checks, and the tool SHALL NOT set these variables for the git commands it runs itself.

#### Scenario: Inherited variables
- **WHEN** `verify` is run with `GIT_INTAKE_BASE` or another `GIT_INTAKE_` variable already set
- **THEN** the commands receive only the variables of the update, with the update's values

### Requirement: Guarded publication
In push mode, and with `--no-publish`, `finish` SHALL require a verified update whose worktree commit is the verified one, or one whose head is a catch-up merge the tool made and has not verified, which it verifies itself; any other unverified head SHALL be refused until `verify` passes. In merge-request mode without `--no-publish` it SHALL require no verification. It SHALL never rewrite or force-push a branch, and SHALL never push to an upstream's URL.

#### Scenario: Commit after a catch-up
- **WHEN** a commit is added on top of a catch-up merge before it is verified, in push mode
- **THEN** `finish` refuses until `verify` passes for the new commit

#### Scenario: Merge-request mode
- **WHEN** the publication mode is `merge-request` and the update was never verified
- **THEN** `finish` publishes the update as a merge request, runs no check, and moves no local branch

### Requirement: Publish remote in finish
In `push` mode, unless `--no-publish` is given, `finish` SHALL first fetch the publish remote and refuse with git's reason when the fetch fails.

#### Scenario: Publish remote unreachable
- **WHEN** the publish remote cannot be fetched
- **THEN** `finish` refuses with git's reason and moves no branch, and `finish --no-publish` closes the update locally without fetching

### Requirement: Remote integration branch in finish
In `push` mode, unless `--no-publish` is given, after `finish` fetches the publish remote, when the remote's integration branch holds commits the local one lacks and the local one holds none the remote lacks, `finish` SHALL bring the local integration branch up to date as `start` does, which does not count as `finish` moving it; when each holds commits the other lacks, it SHALL refuse before moving anything and say that the local commits must be published or reconciled first.

#### Scenario: Remote moved
- **WHEN** the publish remote's integration branch is ahead of the local one, which holds no commit the remote lacks, and `finish` runs in push mode
- **THEN** `finish` fast-forwards the local integration branch to the remote's, catches up with it, verifies, and publishes the catch-up merge

#### Scenario: Diverged from the remote
- **WHEN** the publish remote's integration branch and the local one each hold commits the other lacks
- **THEN** `finish` refuses before moving any branch and says that the local commits must be published or reconciled first

### Requirement: Integration tip in finish
When the integration tip contains the update head, `finish` SHALL go on without a catch-up. The integration tip SHALL be the local integration branch's in `push` mode and the publish remote's in `merge-request` mode.

#### Scenario: Integration branch already holds the update
- **WHEN** the integration branch was moved to the verified commit, or past it, by other means
- **THEN** `finish` makes no catch-up merge, counts the move as done, pushes the verified commit in push mode, and completes the update

### Requirement: Catch-up in finish
When the integration tip does not contain the update head and is no longer the base, `finish` SHALL catch up with it, and where it requires verification, verify the result as `verify` does and go on only when the verification passes; it SHALL make at most one catch-up merge per run, and when the integration branch moves again meanwhile it SHALL stop and say to run `finish` again.

#### Scenario: Integration branch moved
- **WHEN** a commit lands on the integration branch after `start` and `finish` runs on the verified update in push mode
- **THEN** `finish` catches up with the new tip, runs the install command and the checks on the catch-up merge, fast-forwards the integration branch to it, publishes it, and afterwards `status` shows every merged upstream's target as its last synced commit

#### Scenario: Catch-up that conflicts in finish
- **WHEN** the commit that landed conflicts with the update
- **THEN** `finish` stops with exit status 1 in the catch-up step and moves no branch to the update, and after the conflict is resolved and `continue` has run, `finish` verifies and publishes

#### Scenario: Checks fail after a catch-up
- **WHEN** the checks fail on the catch-up merge in push mode
- **THEN** `finish` stops with the report of `verify`, the update stays unverified at the catch-up merge, and no branch moves to the update

#### Scenario: Integration branch moves during verification
- **WHEN** another commit lands on the integration branch while `finish` verifies a catch-up merge
- **THEN** `finish` moves no branch to the update and says to run it again, which catches up once more

### Requirement: Confirming the move in finish
In push mode and with `--no-publish`, before moving any branch `finish` SHALL confirm that the move of the integration branch is a fast-forward, that the verified commit contains the base and every merged target, and that no checkout of the integration branch would lose changes.

#### Scenario: Checkout that would lose changes
- **WHEN** `finish` runs in push mode on a verified update while a checkout of the integration branch holds uncommitted changes that the fast-forward would overwrite
- **THEN** `finish` moves no branch

### Requirement: Moving the integration branch in finish
In push mode and with `--no-publish`, `finish` SHALL record the planned move, fast-forward the integration branch to the verified commit, push exactly the planned commit without force when the publication mode is `push` and `--no-publish` is not given, and only after that publication succeeds, or at once with `--no-publish`, remove the worktree, the update branch, the update's upstream references and the state.

#### Scenario: No publish remote
- **WHEN** no publish remote is configured
- **THEN** `finish` advances the local integration branch and pushes nothing

### Requirement: Completing an interrupted finish
An interrupted or failed `finish` SHALL complete the remaining move and the push when run again, and SHALL never publish a commit other than the planned one.

#### Scenario: Local commit while finishing
- **WHEN** a commit is added to the integration branch after `finish` moved it and before the push succeeded
- **THEN** running `finish` again pushes only the planned commit

#### Scenario: Interrupted finish
- **WHEN** `finish` stops after moving the integration branch and before the push
- **THEN** `status` shows the finishing step, `abort` refuses, and running `finish` again completes the push

### Requirement: Rejected push in finish
When the push was rejected because the publish remote's integration branch moved past the base, a `finish` run again SHALL catch up with the remote's branch from the finishing step, verify the result, fast-forward the local integration branch to it and push it; when the local integration branch moved past the planned commit meanwhile, it SHALL make no catch-up and name `finish --no-publish`.

#### Scenario: Push rejected after the move
- **WHEN** the publish remote's integration branch moves after `finish` moved the local branch and before its push, so that the push is rejected
- **THEN** no remote branch moves, the update stays in its finishing step, and running `finish` again catches up with the remote's branch, verifies, fast-forwards the local integration branch and pushes the catch-up merge

#### Scenario: Local commit and rejected push
- **WHEN** a commit was added to the local integration branch after `finish` moved it, and the push was rejected because the remote moved
- **THEN** `finish` makes no catch-up merge, moves no branch, and names `finish --no-publish`

### Requirement: Finishing without publication
`finish --no-publish` SHALL not fetch, SHALL catch up only with the local integration branch, SHALL in the finishing step land a verified catch-up merge locally, and drop an unverified or conflicted one, recording the resolutions made so far and naming the commits it drops, closing the update at the planned commit, complete the local move and remove the worktree, the update branch, the upstream references and the state without pushing, and report that the integration branch remains unpublished.

#### Scenario: Publication never allowed
- **WHEN** the publish remote refuses every push to the integration branch, as for a protected branch
- **THEN** `finish --no-publish` leaves the local integration branch advanced, removes the update, and names the unpublished branch

### Requirement: Update report
`report` SHALL summarise an update in progress or just merged, per upstream. For a `follow` upstream it SHALL give the number of upstream commits merged, the upstream tag of the target when one points at it, the conflicted files and whether each was resolved by hand or by a recorded resolution, and how the repository's own changes relative to that upstream grew or shrank.

#### Scenario: Report after merge
- **WHEN** the user runs `report` after resolving conflicts in a `follow` update
- **THEN** the output lists, for each merged upstream, the merged range, each conflicted file with its resolution source, and the diff size against that upstream before and after

### Requirement: Port upstreams in the report
For a `port` upstream `report` SHALL give the taken commits in the order they were copied, each with its conflicted files and their resolution source, the commits recorded as `have` because their copy changed nothing, the `skip` and `have` decisions with their notes, the number of commits left pending, and, for each taken commit, the earlier upstream commits still pending that changed any of the same files.

#### Scenario: Report of a port
- **WHEN** the user runs `report` after a plan that took a commit whose files an earlier pending upstream commit also changed
- **THEN** the output lists the taken commit with its conflicts and names the earlier pending commit beside it

### Requirement: Upstream status
`status` SHALL fetch every upstream and report, for each, its path, its branch, its mode and its fetched tip.

#### Scenario: Several upstreams
- **WHEN** `status` runs on a repository with three upstreams, two of them behind
- **THEN** it reports each upstream on its own line with its path and state, and the commit count for the two that are behind

### Requirement: Status of a follow upstream
For a `follow` upstream `status` SHALL report its last synced commit with the upstream tag its sync record names, the number of commits the integration branch lacks, and one state.

#### Scenario: Synced to a chosen target
- **WHEN** the last update of a `follow` upstream merged a chosen target that lies three commits before the fetched branch tip
- **THEN** `status` shows that target as the last synced commit, the tip, the state behind and three commits

### Requirement: States of a follow upstream
The state `status` reports for a `follow` upstream SHALL be one of current, behind, not imported, no release when it has a `tags` pattern and its fetched branch has no release tag, path removed when a synced path upstream's path no longer exists, rewritten, or unavailable when its URL cannot be fetched, its branch is missing, or the tags of an upstream with a `tags` pattern cannot be listed.

#### Scenario: No release
- **WHEN** a `follow` upstream with a `tags` pattern has no release tag on its fetched branch
- **THEN** `status` reports the state no release and `status --check` exits with a failure status

### Requirement: Newest release in status
For a `follow` upstream with a `tags` pattern, the newest release takes the place of the tip: `status` SHALL show it as the tip, count the commits the integration branch lacks up to it, predict the conflicts of merging it, and show the fetched branch tip and the number of commits it lies past the release separately.

#### Scenario: Development commits after a release
- **WHEN** a `follow` upstream with a `tags` pattern has merged its newest release, and its branch holds commits after it
- **THEN** `status` reports the upstream as current, shows the release as the tip and the branch tip with the number of commits past the release, and `status --check` succeeds

#### Scenario: Newer release
- **WHEN** a `follow` upstream with a `tags` pattern has a release newer than its last synced commit
- **THEN** `status` reports the upstream as behind by the commits up to that release, not up to the branch tip

### Requirement: Status of a port upstream
For a `port` upstream `status` SHALL report its journal's base, the numbers of taken, decided and pending commits, and one state: current, pending, no base, rewritten, or unavailable.

#### Scenario: Port with pending commits
- **WHEN** `status --check` runs for a `port` upstream with twelve first-parent commits after its base, of which three were taken, two skipped and seven are undecided
- **THEN** it reports the base, three taken, two decided and seven pending, the state pending, and exits with a failure status

#### Scenario: Port without a base
- **WHEN** `status --check` runs for a `port` upstream with no journal, no sync record and no history shared with the integration branch
- **THEN** it reports the state no base with the advice to write the journal and its base by hand, and exits with a failure status

### Requirement: Scheduled status check
`status --check` SHALL additionally list the files a merge of each behind `follow` upstream's tip would conflict in, create no worktree, branch or state, and exit with a failure status unless every upstream is current, so that it can run on a schedule and gate a cutover or a port review.

#### Scenario: Scheduled check
- **WHEN** `status --check` runs on a repository whose single `follow` upstream is 30 commits ahead
- **THEN** it reports the 30 commits and the files that would conflict, exits with a failure status, and leaves the repository unchanged

#### Scenario: Zero drift
- **WHEN** `status --check` runs and every upstream is current
- **THEN** it reports every upstream as current and exits successfully

### Requirement: Pending commits
`log` SHALL list, for the named upstreams or all of them, the commits on the fetched upstream branch that the integration branch does not contain for a `follow` upstream, and the pending commits for a `port` upstream.

#### Scenario: Inspecting an upstream before merging
- **WHEN** the user runs `log` for a `follow` upstream that is three commits behind
- **THEN** the tool lists those three commits with their hashes and subjects

#### Scenario: Inspecting a port upstream
- **WHEN** the user runs `log` for a `port` upstream with one taken, one skipped and two undecided commits after its base
- **THEN** the tool lists only the two undecided commits

### Requirement: Unmerged upstream branches
`branches` SHALL fetch every branch of the named upstreams, or of all of them, for the command, and list every branch that holds commits the integration branch does not contain and, for a `port` upstream, that no copied commit names as taken, with the number of such commits.

#### Scenario: Work left on a feature branch
- **WHEN** an upstream's repository has a feature branch with two commits that were never merged into its tracked branch
- **THEN** `branches` lists that branch under the upstream with two unmerged commits

#### Scenario: Nothing left behind
- **WHEN** every branch of every upstream is contained in the integration branch
- **THEN** `branches` reports that no upstream holds unmerged work

### Requirement: Doctor
`doctor` SHALL run its checks without changing the repository other than through the references of its own fetches. It SHALL report each check with its result and exit with a failure status when any check fails.

#### Scenario: Healthy repository
- **WHEN** every check passes
- **THEN** `doctor` lists the checks as passed and exits successfully

### Requirement: Configuration and upstream checks in doctor
`doctor` SHALL check that the configuration is valid, every upstream's URL can be fetched and its branch exists, every imported upstream's path exists on the integration branch, every `follow` upstream's last synced commit is still contained in its branch, and every `port` upstream's journal is valid and its base and its decided and taken commits that are not retired are still on its branch.

#### Scenario: Invalid configuration
- **WHEN** `doctor` runs while the configuration on the integration branch is invalid
- **THEN** the configuration check fails with the problem, and `doctor` exits with a failure status

### Requirement: Upstream dropped from the configuration
`doctor` SHALL warn, without failing, about each upstream whose sync records the integration branch carries but that the configuration no longer lists, naming it.

#### Scenario: Upstream removed from the configuration
- **WHEN** the integration branch carries sync records of an upstream that the configuration no longer lists
- **THEN** `doctor` reports that upstream by name as dropped from the configuration

### Requirement: Repository checks in doctor
`doctor` SHALL check that the main checkout is clean, no merge is in progress, the update state, if any, matches its worktree and branch, no git remote names a configured upstream's repository, no reference under `refs/git-intake/` is left without an update, and no journal belongs to an upstream that is missing or in `follow` mode.

#### Scenario: Remote left at an upstream's URL
- **WHEN** a git remote's URL is a configured upstream's URL
- **THEN** `doctor` fails the check, names the remote and the upstream, and says to remove the remote

#### Scenario: Partial clone remote at an upstream's URL
- **WHEN** such a remote is the one a partial clone fetches missing objects from
- **THEN** `doctor` additionally says that the missing objects have to be fetched before the remote is removed

#### Scenario: Stale journal
- **WHEN** the integration branch carries a journal for an upstream in `follow` mode
- **THEN** `doctor` fails the check and names the journal

### Requirement: Machine-readable output
`status`, `report`, `log`, `branches`, `doctor` and `env` SHALL accept `--json` and then write a single JSON document to standard output that carries the same facts as the text form, under field names that are stable across releases, with the same exit status. A rewritten `port` upstream SHALL carry the base and the commits that are off its line.

#### Scenario: Agent reads status
- **WHEN** an agent runs `status --json`
- **THEN** it receives, for every upstream, its name, path, branch, mode, fetched tip and state, for a `follow` upstream its last synced commit and commit count, and for a `port` upstream its base and its numbers of taken, decided and pending commits, without parsing text

#### Scenario: Agent reads a rewritten port upstream
- **WHEN** an agent runs `status --json` for a rewritten `port` upstream
- **THEN** it receives the state `rewritten` and each taken or decided commit off the upstream's line that is not retired, with what it was recorded as

#### Scenario: Agent reads the update variables
- **WHEN** an agent runs `env --json` on an update's head
- **THEN** it receives an object whose keys are the variable names and whose values are their values, and an empty object for a commit that carries no update

### Requirement: Same behaviour from every directory
Every command SHALL behave the same whichever directory of the repository it runs in: the main checkout, a subdirectory, a linked worktree, or the update worktree, whether the git directory is `.git` in the main worktree, lies outside it, or belongs to a submodule.

#### Scenario: Finish from the update worktree
- **WHEN** `finish` runs in the update worktree, with or without a publish remote, with `--no-publish`, or opening a merge request
- **THEN** it moves, publishes and cleans up exactly as from the main checkout, and notes that the current directory was removed

#### Scenario: Linked checkout of the integration branch
- **WHEN** a linked worktree has the integration branch checked out and `finish` runs in the update worktree
- **THEN** that checkout follows the fast-forward and stays clean

### Requirement: Commands that depend on the worktree
Only finding the integration branch from the configuration committed at `HEAD` when no update is in progress, the note about an uncommitted configuration edit, and resolving the revisions named to `start` SHALL depend on the worktree the command runs in.

#### Scenario: Revision named in a linked worktree
- **WHEN** `start` runs in a linked worktree with a revision relative to that worktree, such as `HEAD^2`
- **THEN** the revision is resolved in that worktree

### Requirement: Repository operations in the main worktree
The tool SHALL run git operations on the repository as a whole in the main worktree, or in the git directory when the repository is bare, so that a relative remote URL means what it means in the main worktree. `start` SHALL record the main worktree with the update so that the update's later commands find it from any worktree.

#### Scenario: Relative remote URL
- **WHEN** an upstream's URL is a relative path and `start` runs in a linked worktree, or a command fetches it from the update worktree
- **THEN** the URL is resolved against the main worktree, as when the command runs in the main checkout

#### Scenario: Git directory outside the checkout
- **WHEN** the downstream is a submodule, or its git directory was placed elsewhere, and an update is started in its checkout and finished from the update worktree
- **THEN** relative remote URLs resolve against the checkout, and `finish` fast-forwards the checkout

### Requirement: Update worktree of a bare repository
A bare repository has no main worktree, so its update worktree SHALL be `git-intake/worktree` in the git directory; when that path contains `/.git`, which compilers, bundlers and watchers ignore, `start` SHALL refuse, say why, advise a clone with a checkout, and change nothing.

#### Scenario: Bare repository
- **WHEN** an update is started in a bare repository and finished from the update worktree
- **THEN** the update worktree lies in the git directory, and `finish` moves the integration branch and removes it

#### Scenario: Bare repository at a path containing `/.git`
- **WHEN** `start` runs in a bare repository whose update worktree path would contain `/.git`
- **THEN** it refuses, names the reason, advises a clone with a checkout, and creates no worktree, branch or state

### Requirement: Main worktree not located
When the main worktree cannot be located, because the git directory lies outside it without naming it, no update is in progress and the command runs elsewhere, a command that needs the main checkout SHALL refuse and say to run it in the main worktree, `doctor` SHALL warn that it did not check the main checkout and why, and a relative remote URL SHALL resolve against the git directory.

#### Scenario: Main worktree that cannot be located
- **WHEN** the git directory lies outside the checkout without naming it, no update is in progress, and `start` runs in a linked worktree
- **THEN** `start` refuses and says to run it in the main worktree, and `doctor` warns that it did not check the main checkout

### Requirement: Choosing the main worktree
The tool SHALL prefer the worktree it runs in, when that is the main worktree, and the main worktree the update recorded over taking the parent of a git directory named `.git` for it.

#### Scenario: Separate git directory named `.git`
- **WHEN** the git directory is a directory named `.git` apart from the checkout, and an update is started in the checkout and finished from the update worktree
- **THEN** the checkout is taken as the main worktree: nothing is written beside the git directory, and relative remote URLs resolve against the checkout

### Requirement: Removing the current directory
A command that removes the update worktree it runs in SHALL complete and, after its own output, SHALL note that the current directory was removed.

#### Scenario: Abort from the update worktree
- **WHEN** `abort` runs in a directory of the update worktree
- **THEN** no worktree, update branch, log or state of the update remains, and the tool notes that the current directory was removed

### Requirement: Upstream fetch by URL
The tool SHALL fetch an upstream by its configured URL, in the main worktree and without tags, into a reference under `refs/git-intake/`, and SHALL NOT add, change or read a git remote for it. A command that reads upstreams SHALL fetch them itself. Apart from `contribute`, the tool SHALL create no branch, remote-tracking branch or tag from an upstream.

#### Scenario: Remote with an upstream's URL
- **WHEN** the repository has a git remote whose URL names a configured upstream's repository
- **THEN** commands fetch the upstream by URL without using or changing that remote, and `doctor` reports the remote

#### Scenario: Upstream unreachable
- **WHEN** one of several upstreams cannot be fetched
- **THEN** `status` reports that upstream as unavailable with git's error, reports the others, and exits with a failure status, and `start` refuses and creates nothing

### Requirement: Lifetime of upstream references
A command SHALL fetch upstreams into references of its own that no other command uses, and SHALL remove them before it exits, whatever its outcome. A command that acts on the update in progress SHALL restore a reference the update holds that is missing, from the update state, when its commit is in the repository.

#### Scenario: Reading command leaves nothing behind
- **WHEN** `status` runs on a repository with no update in progress
- **THEN** it reports every upstream from a fresh fetch, and afterwards the repository holds no reference under `refs/git-intake/`, no remote and no branch it did not hold before

#### Scenario: Start interrupted before holding its references
- **WHEN** `start` saved the update but stopped before writing the update's references, leaving the fetched commits held only by its own references
- **THEN** `status`, `continue`, `verify` and `finish` restore the update's references from the update state before they go on

### Requirement: References an update holds
An update SHALL hold `refs/git-intake/upstreams/<name>` for each queued upstream at the tip `start` or `sync` fetched, or, for an update carried on from a merge request, at the upstream's target or, for a port upstream, at the newest commit the update takes or decides on; only `start` and `sync` SHALL write it and only `finish`, `abort` and `sync` SHALL remove it.

#### Scenario: Update keeps its references
- **WHEN** an update stops on a conflict and the repository is later garbage-collected
- **THEN** every target and planned commit of the update is still present, and `continue` proceeds without fetching

#### Scenario: Reading command during an update
- **WHEN** `status` runs while an update is in progress and the upstream has moved since `start`
- **THEN** it reports the upstream's new tip, and the update's reference keeps the tip `start` fetched

### Requirement: Checking the update's references
`doctor` SHALL report each reference the update in progress holds that is missing or not at the tip the update state names, and how to restore it. It SHALL NOT report as left without an update, nor advise removing, a reference that holds the commit of a missing update reference.

#### Scenario: Update reference missing
- **WHEN** an update's reference is missing and only a reference left by an interrupted command holds its commit
- **THEN** `doctor` fails, says that `status` or `continue` restores the update's reference, and says to keep the other reference

### Requirement: Port journal
An upstream in `port` mode SHALL keep a journal at `.git-intake/<name>.toml`, committed on the integration branch, holding the upstream commit the port started from as its base, decisions that each name an upstream commit, the verdict `skip` or `have` and an optional note, and optionally a list of retired commits. Commit ids SHALL be full ids, compared in lower case. The tool SHALL read the journal from the integration tip and SHALL refuse an invalid one, naming the problem.

#### Scenario: Invalid journal
- **WHEN** the journal names a verdict other than `skip` or `have`, or decides one commit twice
- **THEN** the tool refuses to run for that upstream and names the journal and the problem

#### Scenario: Ids in upper case
- **WHEN** the journal names its base, a decision or a retired commit with an id in upper case
- **THEN** the tool reads it as the same commit as in lower case

### Requirement: Pending port commits
The pending commits of a `port` upstream SHALL be the commits on the first-parent line of its fetched branch after the journal's base, other than those that a commit on the integration branch names as taken and those the journal decides; commits the journal retires SHALL count as neither. The tool SHALL treat the upstream as rewritten, and refuse to plan it, when the base, or a taken or decided commit that is not retired, is not on that line.

#### Scenario: Decision changed by hand
- **WHEN** a `skip` decision is removed from the journal in an ordinary commit on the integration branch
- **THEN** the commit it named is pending again

#### Scenario: Commit copied by hand
- **WHEN** a commit on the integration branch carries the trailers `Intake-Upstream` and `Intake-Taken` naming a pending upstream commit
- **THEN** that upstream commit counts as taken

#### Scenario: Port upstream rewritten
- **WHEN** a commit that the journal decides is no longer on the first-parent line of the fetched upstream branch
- **THEN** `status` reports the upstream as rewritten and `start` refuses to plan it

### Requirement: Recovering a rewritten port upstream
For a rewritten `port` upstream, `status` and `doctor` SHALL name the base when it is off the first-parent line and every taken or decided commit off it that the journal does not retire, and say to list those commits under `retired` in the journal and to move the base to a commit of the line when it is off. A base off the line SHALL keep the upstream rewritten until it is moved.

#### Scenario: Taken commit force-pushed away
- **WHEN** an upstream commit that a copy took is no longer on the upstream's first-parent line after a force-push
- **THEN** `status` reports the upstream as rewritten, names the commit as taken in its text and JSON, and says to list it under `retired`

#### Scenario: Commits retired
- **WHEN** the owner lists every such commit under `retired` in the journal
- **THEN** the upstream is no longer rewritten, the retired commits count as neither taken nor decided, the rewritten versions of their changes are pending, and `start` plans them

#### Scenario: Base gone
- **WHEN** the journal's base is no longer on the upstream's first-parent line
- **THEN** the upstream stays rewritten, with the base named, until the base is moved to a commit of the line and the commits off it are retired

#### Scenario: Retired commit no longer in the repository
- **WHEN** a commit the journal retires has been pruned from the repository
- **THEN** the upstream is judged and planned as if the commit were present

### Requirement: Journal creation
When a `port` upstream has no journal, the first update that carries out its plan SHALL create the journal with the upstream's last synced commit, from a sync record or from the history, as the base. `start` SHALL refuse when the upstream has neither, and say that the journal and its base have to be written by hand.

#### Scenario: Switch from follow to port
- **WHEN** an upstream that was merged in `follow` mode is switched to `port`, and its first plan is carried out and published
- **THEN** the integration branch holds a journal whose base is the last merged target, and `status` shows as pending the first-parent commits after it that the plan did not decide

#### Scenario: No shared history
- **WHEN** a `port` upstream has no journal, no sync record, and no commit in common with the integration branch
- **THEN** `start` refuses and says that the journal and its base have to be written by hand

### Requirement: Port plan
For an upstream in `port` mode, `start` SHALL fetch the upstream and compute its pending commits, limited to those that the named revision contains when one is given under the rules for resolving a revision. When any are pending, it SHALL create the update worktree and branch from the integration tip, write a plan that lists the pending commits oldest first with the verb `later`, report the plan's path, and stop at the plan step.

#### Scenario: Plan written
- **WHEN** `start` runs for a `port` upstream with four pending commits
- **THEN** the update stops at the plan step, the plan lists the four commits oldest first with the verb `later`, and `status` names the plan's path and `continue` as the next command

#### Scenario: Nothing pending
- **WHEN** `start` runs for a `port` upstream whose every commit after the base is taken or decided
- **THEN** `start` reports the upstream as current and creates nothing

#### Scenario: Plan limited to a release
- **WHEN** `start` names a `port` upstream with a tag of its remote
- **THEN** the plan lists only the pending commits that the tag's commit contains

#### Scenario: Abort at the plan step
- **WHEN** `abort` runs while the update is at the plan step
- **THEN** no worktree, update branch, plan, upstream reference or state of the update remains, and the journal is unchanged

### Requirement: Plan validation
`continue` at the plan step SHALL refuse a plan that names a commit not listed, names a commit twice, uses a verb other than `take`, `skip`, `have` or `later`, or decides nothing, naming the line, and SHALL change nothing then.

#### Scenario: Invalid plan
- **WHEN** the plan names a commit that is not pending, or marks every commit `later`
- **THEN** `continue` refuses, names the line or says that nothing is decided, and changes nothing

### Requirement: Carrying out a port plan
`continue` SHALL copy each commit the plan marks `take` onto the update branch in the order the plan lists them, a merge commit as its change against its first parent, keeping the commit's author and message and adding the trailers `Intake-Upstream: <name>` and `Intake-Taken: <upstream commit>`.

#### Scenario: Plan carried out
- **WHEN** the plan marks two commits `take`, one `skip` with a note and one `later`, and `continue` runs
- **THEN** the update branch holds a copy of each taken commit with its trailers, followed by one commit that adds the skip decision and its note to the journal, the commit marked `later` stays pending, and the next step is `verify`

#### Scenario: Takes reordered
- **WHEN** the plan lists a newer upstream commit marked `take` before an older one
- **THEN** the newer commit is copied first

### Requirement: Recording a port plan's decisions
After the last copy, the tool SHALL commit the plan's `skip` and `have` decisions with their notes to the journal as one commit, creating the journal when it does not exist. Commits marked `later` SHALL stay pending.

#### Scenario: Only decisions
- **WHEN** the plan marks every commit `skip`, `have` or `later` and `continue` runs
- **THEN** the update branch holds one commit that adds the decisions to the journal, and the next step is `verify`

### Requirement: Outcome of a copy
A copy that stops on conflicts SHALL stop the update as a conflicted merge does, and `continue` SHALL conclude the copy and go on with the remaining ones. A copy whose commit fails SHALL stop the update with git's output and stay staged; `continue` SHALL commit what is then staged as that copy and SHALL never discard work in the worktree. A copy that changes nothing SHALL NOT be committed and SHALL be recorded as a `have` decision.

#### Scenario: Conflict while copying
- **WHEN** the copy of a taken commit stops on conflicts
- **THEN** the tool names the upstream commit and the conflicted files, and after the user resolves them and runs `continue`, the copy is committed with its trailers and the remaining takes are copied

#### Scenario: Change already present
- **WHEN** a commit marked `take` changes nothing on the update branch
- **THEN** no copy is committed, the journal records the commit as `have`, and `report` names it

#### Scenario: Commit of a copy refused by a hook
- **WHEN** a hook of the repository rejects the commit of a copy, and the user fixes and stages the copy and runs `continue`
- **THEN** the first `continue` stops with the hook's output, and the next commits the staged fix as the copy with the upstream author, message and trailers, keeping any unstaged work

### Requirement: Unstaged copy
A copy SHALL be recorded as `have` only when neither the index nor the worktree differs from the update branch head. When the index holds no change of the copy but files in the worktree differ, untracked files included, the tool SHALL stop the update as for a failed commit, change nothing, and say to stage what belongs to the copy.

#### Scenario: Interrupted copy left in the worktree
- **WHEN** `continue` resumes a copy whose changes are in the worktree but not staged
- **THEN** it stops, commits nothing and records no decision, and after the changes are staged the next `continue` commits them as the copy

#### Scenario: File added by the copy and unstaged
- **WHEN** the commit of a copy that adds a file fails, and the user unstages that file before `continue`
- **THEN** `continue` stops again instead of recording the commit as present

### Requirement: Git-intake files in copies
A copy SHALL NOT change `.git-intake.toml` or anything under `.git-intake/`. The tool SHALL restore those paths from the update branch head before committing a copy, report the paths it left out in the output of `continue` and in `report`, and record a copy that changed nothing else as `have`.

#### Scenario: Upstream commit with git-intake files of its own
- **WHEN** a taken upstream commit changes `.git-intake.toml` and a file under `.git-intake/` as well as other files, and another taken commit changes only a file under `.git-intake/`
- **THEN** the first copy changes only the other files and names the paths it left out, and the second is recorded as `have`

### Requirement: Upstream names
An upstream's name SHALL be usable in a git reference name, as `refs/git-intake/upstreams/<name>` needs it, and SHALL give its own environment variables to the checks. The tool SHALL refuse a name that contains `..` or ends with `.` or `.lock`, naming it. It SHALL refuse two upstream names whose variable part, the name in upper case with every character other than a letter or a digit replaced by `_`, is the same, naming both names and the shared variable prefix.

#### Scenario: Name git cannot use
- **WHEN** an upstream is named `app.lock`
- **THEN** the tool refuses to run and names the upstream and why

#### Scenario: Names that share variables
- **WHEN** two upstreams are named `app-a` and `APP.a`
- **THEN** the tool refuses to run, names both upstreams and `GIT_INTAKE_UPSTREAM_APP_A_*`, and starts no update

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

### Requirement: Contribution branch
`contribute <upstream>[=<revision>] <branch>` SHALL fetch the upstream and create the local branch `<branch>` at the fetched tip, or at the commit the revision names on the fetched branch as `start` resolves it, without the rules that compare the commit with the last synced commit. It SHALL set no tracking branch, change no checkout and never push the branch. It SHALL refuse when `<branch>` exists or is not a valid branch name.

#### Scenario: Branch for a contribution
- **WHEN** `contribute` runs for an upstream with a new branch name
- **THEN** the branch exists at the upstream's fetched tip, contains no commit that the upstream branch lacks, has no tracking branch, and nothing was pushed

#### Scenario: Branch exists
- **WHEN** `contribute` names a branch that exists
- **THEN** it refuses and changes nothing

### Requirement: Selecting follow upstreams
For upstreams in `follow` mode, `start` SHALL accept upstream names, each optionally followed by `=` and a revision, and select every configured upstream when none is named.

#### Scenario: No upstream named
- **WHEN** `start` runs without naming an upstream
- **THEN** every configured `follow` upstream is selected

### Requirement: Follow update with a fixed target
For upstreams in `follow` mode, `start` SHALL fetch the selected upstreams, record as each upstream's target the commit its revision names, or, when no revision is given, the newest release of an upstream with a `tags` pattern and the fetched upstream tip of any other, record the current integration tip as the base, and queue, in configuration order, the selected upstreams whose target the base does not yet contain.

#### Scenario: Several upstreams at chosen points
- **WHEN** `start` names one upstream with a revision and another without
- **THEN** the first is merged at the commit its revision names and the second at its fetched tip, in one update

#### Scenario: Nothing to update
- **WHEN** the integration branch already contains every selected target
- **THEN** `start` reports that every selected upstream is current and creates nothing

#### Scenario: Unchanged upstreams are skipped
- **WHEN** some selected upstreams have new commits and others do not
- **THEN** only the upstreams with new commits are queued and the others are reported as current

#### Scenario: First import of an upstream
- **WHEN** a queued upstream has a path that does not exist at the base and the integration branch carries no sync record of that upstream
- **THEN** the tool imports the target's tree under that path with a merge commit that keeps the upstream's full history

#### Scenario: Path already holds unrelated content
- **WHEN** a queued upstream's path exists at the base, the integration branch carries no sync record of that upstream, and no merge on the integration line brought that upstream's history into its path
- **THEN** `start` refuses and names the upstream and the path

#### Scenario: Ancestry shared only through a template
- **WHEN** two upstreams descend from one template, one of them is imported, and the other's path already holds the repository's own content
- **THEN** the other upstream is not treated as imported, and `start` refuses for its path, whichever upstreams have been fetched

#### Scenario: Upstreams that merge each other
- **WHEN** an upstream imported without a sync record later appears in another upstream's history
- **THEN** it is still treated as imported at its path and is updated by a subtree merge

#### Scenario: Path imported by other means
- **WHEN** a queued upstream's path exists at the base and the integration branch carries no sync record of that upstream but its integration line holds a merge that brought the upstream's history into the path, as after an import by `git subtree add`, including one made on a branch that was merged with a merge commit
- **THEN** `status` shows the newest upstream commit contained in the integration branch as the last synced commit, and `start` updates the path by a subtree merge that records the sync

#### Scenario: Imported path removed downstream
- **WHEN** the integration branch carries a sync record of a path upstream but its path no longer exists at the base
- **THEN** `start` refuses and explains that the path was removed downstream

#### Scenario: File on the import path
- **WHEN** a leading component of a queued upstream's path is a file at the base
- **THEN** `start` refuses instead of replacing the file

#### Scenario: Chosen commit
- **WHEN** `start` names an upstream with a commit of its fetched branch that is older than the branch tip
- **THEN** that commit is the upstream's target: the update merges, verifies and publishes it, and its sync record names it

#### Scenario: Conflict at a chosen target
- **WHEN** the merge of a chosen target stops on conflicts, and the user resolves them and runs `continue` and then `verify` and `finish`
- **THEN** the merge records the chosen target, not the branch tip

### Requirement: Release tags as follow targets
For a `follow` upstream with a `tags` pattern named without a revision, `start` SHALL list the tags of the upstream's repository from its URL and take as the target the newest release among them. The tool SHALL fetch no tag into the repository.

#### Scenario: Repository tags untouched
- **WHEN** an update takes an upstream at a release tag
- **THEN** the repository's tags are the same before and after the update

### Requirement: Newest release of an upstream
The newest release of a `follow` upstream with a `tags` pattern SHALL be, of the tags of the upstream's repository whose name matches the pattern, is a semantic version without a prerelease part or build metadata once the pattern's literal prefix, and one `v` or `V` after it unless the prefix ends in one, are removed, and points at a commit of the fetched upstream branch, the one with the highest version by semantic versioning precedence.

#### Scenario: Newest release by version
- **WHEN** an upstream with the pattern `v*` has the tags `v0.9.0` and `v0.10.0` on its branch, `v0.9.0` on the newer commit, and commits after both
- **THEN** `start` takes the commit of `v0.10.0`, and the sync record names the tag `v0.10.0`

#### Scenario: Tags that are not releases
- **WHEN** an upstream's newest matching tags are a prerelease such as `v1.0.0-rc.1`, a version with build metadata such as `v1.0.0+build.5`, a name that is no semantic version, or a tag on a branch the fetched branch does not contain
- **THEN** `start` ignores them and takes the newest release among the rest

### Requirement: Upstream without a new release
When a `follow` upstream with a `tags` pattern is named to `start` without a revision and the integration branch already contains its newest release, `start` SHALL report the upstream as current. When such an upstream has no release, `start` SHALL refuse, name the upstream, its URL and the pattern, and say to name the target as a revision.

#### Scenario: Release already taken
- **WHEN** the integration branch contains the newest release, including after an earlier update took a later commit by naming it
- **THEN** `start` reports the upstream as current at that release and creates nothing for it

#### Scenario: No release on the branch
- **WHEN** no tag of the upstream's repository is a release on its fetched branch
- **THEN** `start` refuses, names the upstream, its URL and the pattern, and creates nothing

### Requirement: Revision instead of a release
A revision named to `start` for a `follow` upstream with a `tags` pattern SHALL override the pattern.

#### Scenario: Revision overrides the pattern
- **WHEN** `start` names an upstream with a `tags` pattern together with a revision of an untagged commit after its newest release
- **THEN** that commit is the target, and the sync record carries no `Intake-Tag`

### Requirement: Isolated merge
A follow update SHALL create a dedicated worktree on a new branch from the base and merge each queued target in turn with its own merge commit: at the repository root for an upstream without a path, and under the upstream's path otherwise.

#### Scenario: Clean merge
- **WHEN** every queued target merges without conflicts
- **THEN** the worktree holds one merge commit per upstream and the next step is `verify`

#### Scenario: Conflicts
- **WHEN** a merge stops on conflicts
- **THEN** the tool names the upstream, lists the conflicted files, leaves the merge in progress in the worktree, reports which upstreams are merged and which are still pending, and tells the user to resolve the conflicts and run `continue`

### Requirement: Fixed follow target
A target and its upstream tag SHALL NOT change for the rest of the update, and `status` and `report` SHALL show the upstream tag of a target, or otherwise the revision it was named by, and `report` SHALL name the revision. Where the target is shown as its abbreviated commit, a revision that is that commit's own id, full or abbreviated, SHALL NOT be repeated next to it.

#### Scenario: Upstream moved during the update
- **WHEN** an upstream receives new commits after `start`
- **THEN** the update still merges, verifies and publishes the recorded target

### Requirement: Revision of a follow target
A revision named to `start` for a `follow` upstream, without a leading `refs/tags/` or `tags/`, SHALL name a tag of the upstream's repository when it has a tag of that name, and otherwise resolve as a revision of the repository. The tool SHALL fetch no tag into the repository, and SHALL refuse a revision that resolves through one of the repository's own tags. `start` SHALL list the tags of every selected `follow` upstream from its URL, and refuse when it cannot.

#### Scenario: Chosen tag
- **WHEN** the revision is a tag of the upstream's repository on the upstream's branch, and the repository holds a tag of the same name on another commit
- **THEN** the commit of the upstream's tag is the target, and no tag is added to the repository

#### Scenario: Tag named with its prefix
- **WHEN** the revision is `refs/tags/<tag>` or `tags/<tag>` and the upstream's repository has the tag
- **THEN** the commit of the upstream's tag is the target

#### Scenario: Own tag instead of an upstream tag
- **WHEN** the upstream's repository has no tag of the revision's name, and the revision resolves through a tag of the repository, by itself or in an expression, even when a branch has the same name
- **THEN** `start` refuses and names the repository's tag, and creates nothing

#### Scenario: Expression on an upstream tag
- **WHEN** the revision is an expression on a tag that only the upstream's repository has, such as `<tag>~1`
- **THEN** `start` refuses because the revision names no commit in the repository, and creates nothing

#### Scenario: Upstream unreachable for the tag lookup
- **WHEN** the upstream's URL cannot be asked for its tags after the fetch
- **THEN** `start` refuses, names the upstream and its URL with git's error, and creates nothing

### Requirement: Follow target on the upstream branch
`start` SHALL refuse, naming the upstream and the revision, a revision that names no commit, a target that the fetched upstream branch does not contain, a target older than the upstream's last synced commit, and a target that does not contain the last synced commit.

#### Scenario: Revision outside the upstream branch
- **WHEN** the revision names no commit, or a commit the fetched upstream branch does not contain
- **THEN** `start` refuses, names the upstream, the revision and the branch, and creates nothing

#### Scenario: Target already taken in
- **WHEN** the revision names the upstream's last synced commit, or another commit of the branch that the integration branch already contains and that contains the last synced commit
- **THEN** `start` reports the upstream as current and creates nothing for it

#### Scenario: Target older than the last sync
- **WHEN** the revision names a commit that the last synced commit contains
- **THEN** `start` refuses and explains that an update never moves an upstream back

#### Scenario: Target beside the last sync
- **WHEN** the revision names a commit of the upstream branch that does not contain the last synced commit, such as a commit of a side branch merged after the last sync
- **THEN** `start` refuses and names the last synced commit the target lacks

### Requirement: Conditions for starting an update
`start` SHALL refuse when an update is in progress, when the main checkout or the integration branch has uncommitted changes, when the publish remote cannot be fetched, when the integration branch and its published counterpart each hold commits the other lacks, or when a selected upstream is unknown, named more than once, or rewritten.

#### Scenario: Update already in progress
- **WHEN** `start` runs while an update is in progress
- **THEN** it refuses, names the step the update has reached, and creates nothing

#### Scenario: Integration branch diverged from the publish remote
- **WHEN** the publish remote's integration branch and the local one each hold commits the other lacks
- **THEN** `start` refuses, names how many commits each side lacks, moves no branch and creates nothing

### Requirement: Conditions for starting in merge-request mode
In merge-request mode, `start` SHALL also refuse when the forge cannot be reached as `finish` needs it, when the local integration branch holds commits its published counterpart lacks, or when an update's merge request into the integration branch is open, naming it, `sync` to carry it on and `abort --merge-request` to withdraw it.

#### Scenario: Forge unreachable
- **WHEN** `start` runs in merge-request mode without a token for the forge
- **THEN** it refuses, names the token variables, and creates nothing

#### Scenario: Merge request open
- **WHEN** `start` runs in merge-request mode while the update's merge request is open
- **THEN** it refuses, names the merge request, `sync` and `abort --merge-request`, and creates nothing

### Requirement: Integration branch behind its published counterpart
When the integration branch is strictly behind its published counterpart, `start`, having read the integration branch and the publish remote from the configuration at `HEAD`, SHALL fast-forward it to that counterpart, moving a checkout of the branch with `git merge --ff-only` and the branch otherwise with `git update-ref` against its old value, refusing when a checkout would lose changes, and say that it did.

#### Scenario: Integration branch behind the publish remote
- **WHEN** the publish remote's integration branch holds commits the local one lacks, and the local one holds none the remote lacks
- **THEN** `start` fast-forwards the local integration branch to the remote's, says so, reads the configuration again at the new tip, and starts the update with that tip as its base

### Requirement: Configuration after the fast-forward in start
After fast-forwarding the integration branch to its published counterpart, `start` SHALL read the configuration again at the new tip and fix the base there, or refuse, keeping the fast-forward, when that configuration names another integration branch or publish remote.

#### Scenario: Other publish remote at the published tip
- **WHEN** the integration branch is strictly behind its published counterpart, and the configuration at the counterpart's tip names another publish remote
- **THEN** `start` fast-forwards the local integration branch, then refuses and keeps the fast-forward

### Requirement: Integration line
The tool SHALL find its sync records, and the merges that brought a path upstream's history into its path, on the integration line: the first-parent line of the integration branch, where each merge on it is followed by the commits that `git rev-list --first-parent <merge>^1..<merge>^2` lists for it. An update SHALL count the same whether it was published by push or through a merge request merged by fast-forward or with a merge commit, and whether or not it caught up with the integration branch.

#### Scenario: Merge request merged with a merge commit
- **WHEN** an update's merge request is merged with a merge commit, with or without semi-linear history
- **THEN** `status` shows each merged `follow` upstream's target as its last synced commit, taken from its sync record, and the next `start` plans each upstream from that commit

#### Scenario: Merge request merged by fast-forward
- **WHEN** an update's merge request is merged by fast-forward, or the update is published by push
- **THEN** `status` shows each merged `follow` upstream's target as its last synced commit, taken from its sync record, as before

#### Scenario: Path imported through a merge request
- **WHEN** an update that imports a path upstream is merged through a merge request with a merge commit
- **THEN** `status` shows the upstream as current with its sync record, and the next `start` updates its path by a subtree merge

#### Scenario: Branch without a record merged with a merge commit
- **WHEN** a branch that carries no sync record is merged into the integration branch with a merge commit
- **THEN** every upstream's last synced commit stays what it was

#### Scenario: Branch with a record merged with a merge commit
- **WHEN** a branch that carries a sync record is merged into the integration branch with a merge commit
- **THEN** that record counts as one of this repository's sync records

#### Scenario: Caught-up update merged with a merge commit
- **WHEN** an update that caught up is published through a merge request merged with a merge commit
- **THEN** its sync records and copies count, and a port upstream's taken commits are not pending

#### Scenario: Port started after a merge request merged with a merge commit
- **WHEN** a `follow` update merged through a merge request with a merge commit is followed by switching that upstream to `port` mode
- **THEN** the port's base is the update's target, and the journal the next port update writes starts there

### Requirement: Catch-up merges on the integration line
Among the commits that follow a merge on the first-parent line, a merge that carries the trailer `Intake-Catch-Up` SHALL in turn be followed by the first-parent line of its second parent, down to the first commit that the first parent of the merge on the first-parent line contains, at any depth; other merges among those commits SHALL contribute nothing further.

#### Scenario: Branch merged into another branch
- **WHEN** a branch that carries a sync record is merged into another branch with a merge that carries no `Intake-Catch-Up` trailer, which is then merged into the integration branch with a merge commit
- **THEN** that record does not count

#### Scenario: Update that caught up twice
- **WHEN** an update caught up twice and was published by push
- **THEN** its sync records and copies count, as for an update merged straight in

### Requirement: Upstream history on the integration line
A merge on the first-parent line that carries a sync record whose target is its second parent, as every sync merge does, SHALL bring nothing into the integration line, because what it merged is the upstream's history. Commits contained in a configured upstream's fetched branch SHALL NOT count as this repository's sync records.

#### Scenario: Upstream that records syncs of its own
- **WHEN** an upstream's history carries sync records of its own, the integration branch merged it with a sync merge, and the upstream cannot be fetched
- **THEN** none of the upstream's records counts as one of this repository's, and `doctor` reports no upstream dropped from the configuration

### Requirement: Catching up with the integration branch
When the integration branch moved past an update's base, the tool SHALL take the new tip in by a catch-up merge: first parent the new tip, second parent the update branch head, trailer `Intake-Catch-Up` naming the integration branch. The base SHALL become the new tip and the update SHALL count as unverified. Upstream targets, last synced commits and port plans SHALL stay as they were, and the update branch SHALL only move forward.

#### Scenario: Catch-up without conflicts
- **WHEN** a commit that touches none of the update's files lands on the integration branch after `start`, and the update catches up
- **THEN** the update branch head is a merge whose first parent is the new integration tip and whose second parent is the previous update branch head, with the trailer `Intake-Catch-Up`, and every earlier commit of the update branch is unchanged

#### Scenario: Resolutions made by hand are kept
- **WHEN** an update resolved an upstream merge with edits beyond the conflicting hunks and added a commit on top, and then catches up with integration commits that change other files
- **THEN** the tree of the catch-up merge holds those edits and that commit's changes

#### Scenario: Second move
- **WHEN** the integration branch moves again after an update caught up
- **THEN** the next catch-up merge has the previous catch-up merge as its second parent, and the update branch only moved forward

### Requirement: Forming a catch-up merge
The tool SHALL record the tip it catches up with in the update state before it merges, build the merge on that tip with the update branch head merged in, and move the update branch from its recorded head to the merge only while it is still there. After an interruption, `status`, `continue` and `finish` SHALL recognise a catch-up merge in progress or already committed from the recorded tip, and never recreate a committed one.

#### Scenario: Interrupted after the commit
- **WHEN** the tool is killed after committing a catch-up merge and before moving the update branch to it
- **THEN** `continue` or `finish` moves the update branch to that merge, attaches the update worktree to the update branch, and makes no second catch-up merge

#### Scenario: Interrupted during the merge
- **WHEN** the tool is killed while a catch-up merge is in progress
- **THEN** `status` shows the catch-up step, and `continue` concludes the merge once nothing is left unresolved

#### Scenario: Update branch moved meanwhile
- **WHEN** the update branch no longer points at the head the catch-up started from when the tool would move it
- **THEN** the tool moves no branch, refuses, and names both commits

### Requirement: Catch-up preconditions
The tool SHALL make a catch-up merge only when the new tip contains the base and not the update head. It SHALL refuse and merge nothing when the new tip does not contain the base, or when the commits it adds carry a sync record or a copy of a queued upstream, change the journal of a queued port upstream, or change the configuration other than its install command and checks, naming the way on that "Way on from a refused catch-up" gives.

#### Scenario: Rewritten integration branch
- **WHEN** the integration branch moves to a commit that does not contain the update's base
- **THEN** the tool makes no catch-up merge, refuses, says that the integration branch was rewritten, names `abort`, and moves no branch

#### Scenario: Upstream taken in elsewhere
- **WHEN** a commit that landed on the integration branch carries a sync record of an upstream the update merges, as when another clone took that upstream in
- **THEN** the tool makes no catch-up merge, names the commit and the upstream, and names `abort`

#### Scenario: Configuration changed
- **WHEN** a commit that landed on the integration branch changes an upstream or the publication setting in `.git-intake.toml`
- **THEN** the tool makes no catch-up merge, names the change, and names `abort`

### Requirement: Way on from a refused catch-up
A refused catch-up SHALL name `abort`, or `finish --no-publish` once `finish` moved the integration branch, or for `sync` the merge request, `abort --merge-request` with the warning that withdrawing declines its targets, and `start` and `finish` to publish them again.

#### Scenario: Refused catch-up of an open merge request
- **WHEN** `sync` refuses the catch-up of an open merge request because a commit that landed on the integration branch carries a sync record of one of its upstreams
- **THEN** it names the merge request, `abort --merge-request` with the warning that withdrawing declines its targets, and `start` and `finish`

### Requirement: Configuration after a catch-up
After a catch-up merge, the tool SHALL read the configuration at the merge and run the install command and the checks of that configuration whenever it verifies the update.

#### Scenario: Check added on the integration branch
- **WHEN** a commit that landed on the integration branch adds a check, and the update catches up with it
- **THEN** the verification of the catch-up merge runs that check

### Requirement: Conflicts in a catch-up
A conflicted catch-up merge SHALL stop the tool with exit status 1 in a catch-up step, naming the conflicted files and those resolved by a recorded resolution. `continue` SHALL commit the staged resolution as the catch-up merge with its trailer and name `finish`. `abort` SHALL work in that step, except after `finish` moved the integration branch, where it SHALL refuse and name `finish` and `finish --no-publish`.

#### Scenario: Catch-up with a conflict
- **WHEN** a commit lands on the integration branch that conflicts with a file the update changed, and the update catches up
- **THEN** the tool stops with exit status 1, names the file and which side is the update and which the integration branch, `status` shows the catch-up step, and after the file is resolved and staged `continue` commits the catch-up merge with its trailer and names `finish`

#### Scenario: Abort in the catch-up step
- **WHEN** `abort` runs while a catch-up merge made before `finish` moved any branch is in conflict
- **THEN** the resolutions made by hand are recorded, the update is removed, and the integration branch stays where `start` or the last fast-forward to the publish remote left it

#### Scenario: Abort after the move
- **WHEN** `abort` runs in a catch-up step that a rejected push led to, after `finish` moved the integration branch
- **THEN** it refuses and names `finish` and `finish --no-publish`, and `finish --no-publish` records the resolutions made so far, drops the conflicted catch-up merge, and closes the update at the planned commit

### Requirement: Catch-up in status and doctor
`status` SHALL show a catch-up step with the tip caught up with and the next command, and `status --json` SHALL give it as the update step `catch_up_conflict` or `catch_up`. `doctor` SHALL accept a catch-up step.

#### Scenario: Status during a catch-up
- **WHEN** a catch-up merge stopped on a conflict and an agent runs `status --json`
- **THEN** the update step is `catch_up_conflict`, the tip caught up with is named, and the next command is `continue`

### Requirement: Catch-up in report
`report` SHALL list each catch-up merge with the tip it took in and its conflicted files with their resolution source, and measure upstream ranges and the repository's own changes from the base the update started from, or, for an update carried on from a merge request, from the base it was carried on at.

#### Scenario: Report after a catch-up
- **WHEN** `report` runs on an update that caught up once with a conflict
- **THEN** it lists the catch-up merge with its tip and its conflicted files, and each upstream's merged range and own-change sizes are those measured from the base the update started from

### Requirement: Forge of the publish remote
In merge-request mode the tool SHALL reach the forge of the publish remote's fetch URL, GitLab or GitHub, through its REST API, reading the publication setting from the configuration at the integration tip, never at an update's head. It SHALL take the forge from the setting `forge`; otherwise from the URL's host, `gitlab.com` as GitLab and `github.com` or a `ghe.com` subdomain as GitHub; otherwise from a GitLab CI job or GitHub Actions job whose server URL has that host.

#### Scenario: Forge recognised from the URL
- **WHEN** the publish remote's URL is `git@github.com:team/app.git` and merge-request mode is configured
- **THEN** the tool treats the publish remote as the GitHub repository `team/app`

#### Scenario: Unknown host
- **WHEN** the publish remote is on a host the tool cannot recognise, such as an SSH host alias, and the configuration sets no `forge`
- **THEN** the commands that need the forge refuse, push nothing, and name the settings `forge` and `api`

#### Scenario: Setting changed by an update
- **WHEN** an update's head changes the publication setting's `api` and the integration tip does not
- **THEN** the tool uses the setting at the integration tip

### Requirement: API base and project
The API base SHALL be the setting `api`, whose host SHALL be the remote's; else, in a CI job whose server has the remote's host, `CI_API_V4_URL` or `GITHUB_API_URL`; else `https://<host>/api/v4` for GitLab, `https://api.github.com` for `github.com`, `https://api.<host>` for `ghe.com` hosts and `https://<host>/api/v3` for other GitHub hosts. The project SHALL be the URL's path without credentials and `.git`, less the base's path before `/api/v4` or `/api/v3`.

#### Scenario: Self-managed GitLab in its own CI
- **WHEN** a GitLab CI job of a self-managed instance installed under a relative URL root runs the tool with the publish remote on that instance and no `api` setting
- **THEN** the tool reaches the API at `CI_API_V4_URL` for the project whose path is the remote's path without the relative URL root

#### Scenario: API on another host
- **WHEN** the setting `api` names a host other than the publish remote's
- **THEN** the tool refuses to reach the forge and names the setting

### Requirement: Forge token
The tool SHALL read the forge token from the environment only: `GITLAB_TOKEN` for GitLab; `GH_TOKEN`, then `GITHUB_TOKEN`, for `github.com` and `ghe.com` hosts; `GH_ENTERPRISE_TOKEN`, then `GITHUB_ENTERPRISE_TOKEN`, for another GitHub host, and in a GitHub Actions job on that host then `GH_TOKEN` and `GITHUB_TOKEN`. It SHALL refuse before pushing anything when none is set, naming the variables, and never print a token, store it, or send it to a host other than the API's, even on a redirect.

#### Scenario: No token
- **WHEN** merge-request mode is configured and none of the token variables of the forge is set
- **THEN** `finish` and `sync` refuse before pushing anything and name the variables

#### Scenario: Enterprise host outside Actions
- **WHEN** the publish remote is on a GitHub Enterprise Server host, only `GH_TOKEN` is set, and the tool runs outside GitHub Actions
- **THEN** the tool refuses and names `GH_ENTERPRISE_TOKEN` and `GITHUB_ENTERPRISE_TOKEN`

### Requirement: Forge requests
The tool SHALL ask GitHub for the API version `2022-11-28`. When the forge rejects the token or a request, the tool SHALL refuse with the forge's status and message, and before a push it SHALL push nothing. Messages about the update's publication SHALL use the forge's term, merge request or pull request.

#### Scenario: Token rejected
- **WHEN** the forge answers a request with an authentication or permission error
- **THEN** the tool refuses with the forge's status and message, and the token appears nowhere in its output

#### Scenario: Term of the forge
- **WHEN** `finish` opens the update's pull request on GitHub
- **THEN** its output speaks of a pull request

### Requirement: The update's merge request
The update's merge request SHALL be a merge request into the integration branch from `git-intake/update`, or from a branch `git-intake/update-<commit>` that an earlier release used. When a merge request from one of these branches into another branch is open, `finish` and `sync` SHALL refuse, pushing nothing, and name it.

#### Scenario: Merge request into another branch
- **WHEN** a merge request from `git-intake/update` into a branch other than the integration branch is open
- **THEN** `finish` and `sync` refuse, push nothing, and name the merge request and its target branch

### Requirement: GitLab merge settings
Before every push of an update branch to GitLab, the tool SHALL read the project and refuse, pushing nothing and naming each setting to change, unless `only_allow_merge_if_pipeline_succeeds` is on, `allow_merge_on_skipped_pipeline` is off, the merge method is `ff` or `rebase_merge`, merge trains are off, automatic rebase before merge is off or unknown to the instance, and the squash option is not `always`.

#### Scenario: Semi-linear history
- **WHEN** a GitLab project requires a successful pipeline, does not count a skipped one, merges with semi-linear history, uses no merge trains, rebases nothing automatically and squashes only on request
- **THEN** `finish` publishes the update

#### Scenario: Plain merge commits
- **WHEN** a GitLab project merges with the plain merge commit method, with merged results pipelines
- **THEN** `finish` and `sync` refuse, push nothing, and name the merge method to change

#### Scenario: Pipeline not required
- **WHEN** a GitLab project does not require a successful pipeline, or counts a skipped one as successful
- **THEN** `finish` and `sync` refuse, push nothing, and name the setting

#### Scenario: Rebasing merges
- **WHEN** a GitLab project rebases a merge request automatically before merging, or merges through merge trains
- **THEN** `finish` and `sync` refuse, push nothing, and say that a rebase drops the update's merge commits

### Requirement: GitHub merge settings
Before every push of an update branch to GitHub, the tool SHALL read the repository and the rules for the integration branch and refuse, pushing nothing and naming each setting to change, unless status checks naming at least one check are required with the branch up to date before merging, through the rules or the classic protection, no merge queue applies, merge commits are allowed, and no rule requires a linear history.

#### Scenario: Up-to-date requirement missing
- **WHEN** a GitHub repository's rules for the integration branch require status checks but not an up-to-date branch
- **THEN** `finish` and `sync` refuse, push nothing, and name the requirement to turn on

#### Scenario: Merge queue
- **WHEN** a merge queue applies to the integration branch
- **THEN** `finish` and `sync` refuse, push nothing, and say that the tool cannot confirm that a merge queue lands the tree it tested

#### Scenario: Linear history required
- **WHEN** a rule for the integration branch requires a linear history
- **THEN** `finish` and `sync` refuse, push nothing, and say that the update's merge commits cannot land

### Requirement: Unconfirmed GitHub merge settings
When the token cannot read what the GitHub merge settings require, as a protected branch's classic protection, its up-to-date checks, merge queue and linear history, without administration access, or whether merge commits are allowed without push access, the tool SHALL refuse, pushing nothing, say which requirement could not be confirmed, and name a ruleset, or the access the token lacks, as the ways to let it.

#### Scenario: Classic protection unreadable
- **WHEN** a GitHub repository requires up-to-date status checks only through classic branch protection, and the token cannot read it
- **THEN** `finish` and `sync` refuse, push nothing, say that the requirement could not be confirmed, and name a ruleset or administration read access

### Requirement: Merging the update's merge request
The description of the update's merge request SHALL say to merge it without squashing and without rebasing its branch, including with the forge's rebase or update-with-rebase buttons, because either drops the update's history, and to keep it current with `sync` instead.

#### Scenario: Squash on request
- **WHEN** a GitLab project's squash option is `default_off`, and `finish` opens the update's merge request
- **THEN** the description says to merge it without squashing or rebasing and names `sync` to keep it current

### Requirement: Update line
The update line of a head over a base SHALL lead from the head, through the commits the base lacks, stepping from a merge carrying `Intake-Catch-Up`, and from a head that merges into a commit the base contains without a sync record, as a merged results commit, to its second parent, and from every other commit to its first parent. The update's own commits SHALL be the sync merges, copies and commits changing a port upstream's journal on that line; a line without any carries no update.

#### Scenario: Upstream with sync records of its own
- **WHEN** an update merges an upstream whose own history carries sync records of an upstream with the same name
- **THEN** only the update's own sync merge counts for the update

#### Scenario: Port update of decisions only
- **WHEN** an update's port plan only skips commits or records them as present, so that its only commit changes the journal
- **THEN** the update line carries that update, and `env` sets the port upstream's variables

#### Scenario: Branch updated by the forge
- **WHEN** a person merged the integration branch into the merge request branch with the forge's update button
- **THEN** the update line follows the merge request branch's side, and the update's sync records count

### Requirement: Rewritten merge request branch
The tool SHALL treat the update's open merge request as rewritten when its head's update line lacks a sync record for a follow target its description records, or, without readable targets, carries no update. `sync` SHALL refuse to act on it, naming it, saying that withdrawing it with `abort --merge-request` declines its targets, and naming `start` and `finish` to publish them again.

#### Scenario: Rebased merge request
- **WHEN** the merge request branch was rebased with the forge's rebase button
- **THEN** `sync` refuses, names the merge request, `abort --merge-request`, `start` and `finish`, and pushes nothing

#### Scenario: Fix amended by force
- **WHEN** a person force-pushed the merge request branch to amend their own fix commit, keeping the sync merges
- **THEN** `sync` acts on the merge request as on any other

#### Scenario: Port copies kept by a rebase
- **WHEN** a merge request of an update with a follow and a port upstream was rebased, keeping the copies and dropping the sync merge
- **THEN** `sync` refuses as for any rewritten merge request

### Requirement: Merge request merged without its history
A merged update's merge request SHALL count as merged without its history when the integration branch does not contain its final head, as the forge reports it, or when the integration line lacks a sync record for a follow target its description records. When `sync` then proposes the same targets again, it SHALL warn, naming that merge request, and name it in the new merge request's description.

#### Scenario: Squashed merge request
- **WHEN** the most recently merged merge request of the update was squashed and no upstream moved since
- **THEN** `sync` opens a merge request for the same targets, warns that the squash dropped the sync records, and names the squashed merge request in the new description

### Requirement: Unattended synchronisation
`sync` SHALL take the repository's follow upstreams in without an update state. It SHALL refuse when an update is in progress, naming its step and the next command, and when the configuration has no publication setting. A conflict, or in push mode failed checks, SHALL stop it with the update in progress; any other end SHALL leave no update it created.

#### Scenario: Update in progress
- **WHEN** `sync` runs while an update is in progress
- **THEN** it refuses, names the update's step and the next command, and changes nothing

#### Scenario: Conflict in CI
- **WHEN** a follow upstream's merge conflicts during `sync`
- **THEN** `sync` stops with exit status 1, names the conflicted files, publishes nothing, and running `sync` in another clone while nothing moved stops on conflicts in the same files, after which `continue` and `finish` publish the update

#### Scenario: Refused catch-up
- **WHEN** `sync` refuses the catch-up of an open merge request because the integration branch took in one of its upstreams
- **THEN** no update remains in the clone, and the next `sync` refuses the same way

### Requirement: Integration branch for synchronisation
Like `start`, `sync` SHALL read the configuration at `HEAD`, fetch the publish remote, fast-forward a local integration branch strictly behind the remote's, refuse when the two diverged, and read the configuration again at the new tip. It SHALL create a missing local integration branch at the remote's. In merge-request mode it SHALL refuse when the local integration branch holds commits the remote's lacks.

#### Scenario: Clean CI clone
- **WHEN** `sync` runs in a CI job whose clone has `HEAD` detached at a commit of the integration branch and no local integration branch
- **THEN** it creates the local integration branch at the publish remote's and carries on as in a clone where that branch exists

#### Scenario: Unpublished local commits
- **WHEN** `sync` runs in merge-request mode while the local integration branch holds a commit the publish remote lacks
- **THEN** it refuses, names the commit count, and changes nothing

### Requirement: Upstreams sync cannot take
`sync` SHALL leave out every follow upstream for which `start` would refuse when it is the only one selected, as one it cannot fetch, without a release, rewritten, or whose path cannot take it, name it with the reason, and take the others. It SHALL leave out port upstreams and name those with pending commits.

#### Scenario: Unreachable upstream
- **WHEN** one of two follow upstreams cannot be fetched and the other is behind
- **THEN** `sync` publishes the update of the other, names the unreachable one, and exits with status 1

#### Scenario: Port upstream pending
- **WHEN** `sync` runs while a port upstream has pending commits and no follow upstream is behind
- **THEN** it starts nothing, names the port upstream and its pending commits, and exits with status 0

### Requirement: Exit status of synchronisation
`sync` SHALL exit with status 2 when it refused or failed, 1 when it stopped on a conflict or failed checks or left out a follow upstream it could not take, and 0 otherwise.

#### Scenario: Nothing behind
- **WHEN** `sync` runs and every follow upstream is current, with no merge request open
- **THEN** it creates nothing, says that every follow upstream is current, and exits with status 0

#### Scenario: Concurrent runs
- **WHEN** two `sync` runs push the update branch at the same time and the second push is rejected as not a fast-forward
- **THEN** the second run exits with status 2, says to run it again, and opens no merge request

### Requirement: Synchronising an open merge request
In merge-request mode `sync` SHALL run no install command and no check. With the update's merge request open, it SHALL fetch the merge request's head and apply "Rewritten merge request branch" first. When the head contains the integration tip, it SHALL change nothing, say that the update waits for the merge request, and name the upstreams that moved on since, which the next update takes.

#### Scenario: Run again without change
- **WHEN** `sync` runs while the update's merge request is open and the integration branch did not move
- **THEN** it pushes nothing, changes no merge request, says that the update waits for the merge request, and exits with status 0

#### Scenario: Upstream moved while the merge request waits
- **WHEN** an upstream moves on while its update's merge request is open and the integration branch did not move
- **THEN** `sync` pushes nothing, names the upstream as moved on, and the next `sync` after the merge takes the new commits in a new merge request

### Requirement: Catching up an open merge request
When the head of the update's open merge request does not contain the integration tip, `sync` SHALL carry the merge request's update on locally, catch it up with the integration tip under the catch-up preconditions without running the checks, and publish it as `finish` does in merge-request mode.

#### Scenario: Integration branch moved
- **WHEN** a commit lands on the integration branch while the merge request is open, and `sync` runs
- **THEN** the merge request's head becomes a catch-up merge with the new tip as its first parent and the previous head as its second, pushed without force, its title and description are rewritten, and no second merge request exists

#### Scenario: Commit added to the merge request
- **WHEN** a person pushed a fix on top of the merge request branch, and the integration branch then moves
- **THEN** `sync` catches up from the merge request's head with the fix, and the fix stays in the update

#### Scenario: Catch-up conflict
- **WHEN** the catch-up of an open merge request conflicts during `sync`
- **THEN** `sync` stops with exit status 1, pushes nothing, the merge request keeps its head, and after `continue` the next `finish` pushes the catch-up merge to the same merge request

#### Scenario: Earlier release's branch
- **WHEN** the update's open merge request comes from `git-intake/update-<commit>` and the integration branch moved
- **THEN** `sync` pushes the catch-up merge to that branch

### Requirement: Synchronising without an open merge request
In merge-request mode with the update's merge request not open, `sync` SHALL start an update of every follow upstream the integration branch lacks the target of, the fetched tip or, with `tags`, the newest release, and publish it as `finish` does in merge-request mode once every merge concluded, without running the install command or the checks.

#### Scenario: Merge request opened by CI
- **WHEN** a scheduled CI job runs `sync` while an upstream is behind and no merge request is open
- **THEN** the update merges the upstream, `git-intake/update` holds it on the publish remote, a merge request from it into the integration branch exists, no install command or check ran, no update remains in the clone, and the job exits with status 0

### Requirement: Synchronisation by push
In push mode, `sync` SHALL start an update of every follow upstream the integration branch lacks the target of, as in merge-request mode, then run `verify`, and when the checks pass run `finish`. Failed checks SHALL stop it with exit status 1, the update in progress and unverified, and nothing published.

#### Scenario: Push mode sync
- **WHEN** `sync` runs in push mode while an upstream is behind, and the merge and the checks succeed
- **THEN** the publish remote's integration branch holds the verified update, no update remains, and `sync` exits with status 0

#### Scenario: Checks fail in push mode
- **WHEN** the checks fail during `sync` in push mode
- **THEN** `sync` exits with status 1, publishes nothing, and leaves the update in progress, unverified

### Requirement: Declined update targets
When the update's merge request is not open and the one closed or merged most recently was closed without merging, `sync` SHALL read the follow targets recorded in its description. When it recorded every follow target `sync` would merge, it SHALL start nothing, name the closed merge request, say that it declined these targets, and exit with status 0. A description without readable targets SHALL decline nothing.

#### Scenario: Merge request closed without merging
- **WHEN** a person closed the update's merge request without merging and no upstream moved since
- **THEN** the next `sync` opens no merge request, names the closed one, and exits with status 0

#### Scenario: Upstream moved after a decline
- **WHEN** an upstream of a declined merge request moves on
- **THEN** the next `sync` opens a merge request with the new target

#### Scenario: Update by hand after a decline
- **WHEN** a person runs `start` and `finish` for the declined targets
- **THEN** the update is published and its merge request opened as for any other

### Requirement: Update branch without an open merge request
With `git-intake/update` at the fetch URL and no merge request open from it, `finish` and `sync` SHALL treat the branch as left over when it holds only catch-up merges beyond the integration tip or a closed or merged merge request's head, and delete it on each push URL while it is at the commit read. Otherwise `sync` SHALL carry on a branch holding an update, except as "Declined branch" requires, `finish` SHALL push over one its head contains, and both SHALL refuse any other.

#### Scenario: Branch kept after a decline
- **WHEN** a pull request from `git-intake/update` was closed without merging, GitHub kept its branch, and an upstream moved on
- **THEN** the next `sync` deletes the branch, pushes the new update, and opens a pull request

#### Scenario: Interrupted job
- **WHEN** a CI job pushed `git-intake/update` and was killed before the merge request was opened
- **THEN** the next `sync` in a fresh clone opens the merge request for that branch without starting another update

#### Scenario: Catch-up left after a decline
- **WHEN** a catch-up merge was pushed to `git-intake/update` just after its merge request was closed, and the branch was not deleted
- **THEN** the next `sync` deletes the branch and opens no merge request for the declined targets

#### Scenario: Branch of unknown origin
- **WHEN** `git-intake/update` holds commits that carry no update and no merge request of the update points at
- **THEN** `finish` and `sync` refuse, push nothing, and name the branch, the push URL, and `abort --merge-request` as the way to delete it after fetching what should be kept

### Requirement: Declined branch
When every follow target on the update line of a branch `sync` would carry on was recorded by the merge request that "Declined update targets" would apply, `sync` SHALL refuse with status 2, naming the branch and `abort --merge-request`, which deletes the branch.

#### Scenario: Declined branch with a fix
- **WHEN** the branch of a declined pull request holds a fix a person pushed after the decline, and `sync` runs
- **THEN** `sync` refuses with exit status 2, pushes nothing, and names the branch and `abort --merge-request`

### Requirement: Carrying on a merge request's update
To carry on the update a merge request or its branch holds, `sync` SHALL put the update branch at its head in the update worktree, the base at the head's merge base with the integration tip, and queue as merged or carried out each upstream whose sync merge, copies or journal change is on the update line: a follow upstream at its newest sync record's target, with the last synced commit at the base as its previous one, and a port upstream with the commits its copies take and its journal decides.

#### Scenario: Status of a carried-on update
- **WHEN** `sync` stopped on a catch-up conflict of a merge request opened by another clone, and `status` runs
- **THEN** it shows the catch-up step, the upstreams of the merge request's update with their targets, and `continue` as the next command

#### Scenario: Abort of a carried-on update
- **WHEN** `abort` runs on an update carried on from a merge request
- **THEN** the local update is removed, the merge request stays open, and `abort` names `abort --merge-request` to withdraw it

### Requirement: Commands on a carried-on update
An update carried on from a merge request SHALL record that merge request and be recorded as started by the release that carried it on; `finish` SHALL refuse with status 2 to publish it once that merge request is no longer open, keeping the update and naming the merge request and `abort`, and every command SHALL act on it as on an update `start` began. `report` SHALL show what history holds of it and say that the conflicts of the steps taken before it was carried on are not known.

#### Scenario: Report of a carried-on update
- **WHEN** `report` runs on an update carried on from a merge request whose upstream merge had conflicts
- **THEN** it lists the upstreams with their merged ranges and says that the conflicts of the earlier steps are not known

### Requirement: Publishing an update as a merge request
In merge-request mode `finish` SHALL move no local branch, catch up with the publish remote's integration branch when it moved past the base, without running the checks, require the merge settings, push the update head without force to the update's branch on the publish remote, and through the API open the update's merge request from that branch, or use the one open. On GitLab it SHALL ask for squashing off and the branch removed on merge.

#### Scenario: Merge request opened
- **WHEN** `finish` runs in merge-request mode on an update whose merges concluded, against a GitLab project whose merge settings qualify
- **THEN** the branch `git-intake/update` holds the update head on the publish remote, a merge request from it into the integration branch exists with squashing off, the local integration branch has not moved, and `finish` exits successfully

#### Scenario: Pull request opened
- **WHEN** the publish remote is a GitHub repository whose merge settings qualify
- **THEN** `finish` opens a pull request from `git-intake/update` into the integration branch and names it as a pull request

#### Scenario: Integration branch moved before finish
- **WHEN** a commit lands on the publish remote's integration branch after `start`, and `finish` runs in merge-request mode
- **THEN** `finish` catches up with it without running the checks and pushes the catch-up merge as the merge request's head

#### Scenario: Merge request already open for this update
- **WHEN** `finish` runs on an update carried on from an open merge request, after its catch-up conflict was resolved
- **THEN** `finish` pushes the catch-up merge to the same branch without force and opens no second merge request

#### Scenario: Update already on the integration branch
- **WHEN** the publish remote's integration branch already contains the update head when `finish` runs in merge-request mode
- **THEN** `finish` pushes nothing, opens no merge request, removes the update, and says that the update already landed

### Requirement: Merge request branch on the remote
`finish` SHALL refuse, pushing nothing and keeping the update, when the update's open merge request has a head the update head does not contain, naming it, `abort` followed by `sync` to carry it on, and `abort --merge-request` to withdraw it, which also removes the local update and declines the merge request's targets until `start` and `finish` publish them again. It SHALL push only where the branch is not at the update head, and never rewrite or force-push it.

#### Scenario: Merge request of another update open
- **WHEN** an update started with `start` reaches `finish` while the update's merge request is open with a head the update head does not contain
- **THEN** `finish` pushes nothing, names the merge request, `abort` followed by `sync`, and `abort --merge-request`, and keeps the update

### Requirement: Merge request state before the push
Just before `sync` pushes to an open merge request's branch, it SHALL read the merge request's state and head and the remote's integration branch again, and push nothing unless all are as read before: a merge request merged or closed meanwhile SHALL end the run with status 0, and any other change with status 2 and the advice to run again. Right after the push it SHALL read the state again and, when the merge request is no longer open, delete the branch while it is at the pushed commit.

#### Scenario: Merged during a catch-up
- **WHEN** the merge request is merged while `sync` catches it up, before the push
- **THEN** `sync` pushes nothing, opens no merge request, says that the merge request was merged, and exits with status 0

### Requirement: Changes to the update's own checks
When an update changes the forge's CI definition, `.github/workflows/` on GitHub or the project's CI configuration file on GitLab, or the install command or checks of `.git-intake.toml`, `finish` and `sync` SHALL name those changes in their output and in the merge request's description, since the pipeline that verifies the update then runs what the update brings.

#### Scenario: Upstream changes the pipeline
- **WHEN** a root upstream's update changes `.gitlab-ci.yml`
- **THEN** `sync` names the change in its output and in the merge request's description

### Requirement: Changed checks by name
When an update changes the checks of `.git-intake.toml`, `finish` and `sync` SHALL, in their output and in the merge request's description, name each check added, removed or given another command by its name, and say when only their order changed; a renamed check SHALL count as one removed and one added.

#### Scenario: Upstream changes a check
- **WHEN** a root upstream's update adds the check `audit` and changes the command of the check `test` in `.git-intake.toml`
- **THEN** `finish` names the checks `audit` as added and `test` as changed in its output and in the merge request's description

### Requirement: Unreadable configuration in the update's own checks
When `.git-intake.toml` differs between the base and the head and either side cannot be read, `finish` and `sync` SHALL name its install command and checks as changed without names, in their output and in the merge request's description.

#### Scenario: Configuration that cannot be read
- **WHEN** an update's head holds a `.git-intake.toml` that differs from the base's and cannot be read
- **THEN** `finish` names the install command and the checks as changed, without naming a check, in its output and in the merge request's description

### Requirement: Title and description of the merge request
With every push of an update for a merge request, the tool SHALL set the title, naming the updated upstreams, and the description, listing each follow upstream's merged range with its target's tag and each port upstream's taken commits and decisions, and ending with a block, hidden when rendered, recording each follow target; a description too long for the forge SHALL be shortened before that block. On GitLab it SHALL ask again for squashing off. Title and description SHALL belong to the tool.

#### Scenario: Description after a catch-up
- **WHEN** a catch-up merge is pushed to an open merge request whose description a person edited
- **THEN** the merge request's title and description describe the update at its new head, with the hidden block of its targets

### Requirement: Local update after publication
Only after the merge request exists SHALL `finish` remove the update worktree, the update branch, the update's upstream references and the state, name the merge request with its address, and exit successfully; the merge request carries the update from then on. A `finish` interrupted before the merge request exists SHALL keep the update, and run again SHALL push only what is missing and open the merge request.

#### Scenario: Interrupted before the merge request
- **WHEN** `finish` pushed the branch and stopped before the merge request was opened, and runs again
- **THEN** it pushes nothing, opens the merge request, and removes the update

#### Scenario: Merged and the next update
- **WHEN** the merge request is merged with a merge commit or by fast-forward, and `start` runs
- **THEN** `start` fast-forwards the local integration branch to the publish remote's, and `status` shows every merged upstream's target as its last synced commit

### Requirement: Withdrawing a merge request
`abort --merge-request` SHALL remove a local update in progress, then close through the API the update's open merge request, and delete the update's branch on each push URL while it points at the commit read, open merge request or not, reporting what remains; with neither it SHALL say that there is nothing to withdraw. `abort` without it SHALL never close a merge request, and with no update in progress SHALL name `abort --merge-request` when one is open, or say that it could not read the forge.

#### Scenario: Withdraw the merge request
- **WHEN** a merge request opened by `sync` is open and `abort --merge-request` runs on a clone with no update in progress
- **THEN** the merge request is closed, `git-intake/update` is gone from the publish remote, and the integration branch is unchanged

#### Scenario: Branch moved by someone else
- **WHEN** the merge request branch points elsewhere than the merge request's head as read before `abort --merge-request` deletes it
- **THEN** it closes the merge request, keeps the branch, and names it

#### Scenario: Interrupted withdrawal
- **WHEN** `abort --merge-request` closed the merge request and stopped before deleting the branch, and runs again
- **THEN** it deletes the branch the closed merge request left

#### Scenario: Withdraw while carrying on
- **WHEN** `abort --merge-request` runs while an update carried on from the open merge request stopped on a catch-up conflict
- **THEN** the local update is removed, the merge request is closed, and its branch is deleted

### Requirement: Update variables from history
`env` SHALL set, from history alone, the variables `verify` sets for a commit, `HEAD` unless a revision is named, with the same values. The base SHALL be the commit's merge base with the integration tip. The upstream variables SHALL come from the sync records and copies on the update line, the journal changes after the base, and the integration line at the base.

#### Scenario: Merge request pipeline
- **WHEN** a pipeline job of an update's merge request runs `git intake env -- ./check.sh` on the merge request's head
- **THEN** `check.sh` has the base, head, integration branch, upstream names, modes, targets and previous commits that `verify` gives its checks for that commit

#### Scenario: Merged results pipeline
- **WHEN** the job runs on a commit that merges the update's head into the integration tip, as a merged results pipeline does
- **THEN** `GIT_INTAKE_BASE` is that integration tip, `GIT_INTAKE_HEAD` is the merge, and the upstream variables are those of the update

### Requirement: Integration tip of the update variables
The integration tip of `env` SHALL be the publish remote's integration branch, fetched for the command, or, when the configuration names no publish remote, the local integration branch. `env` SHALL refuse when the fetch fails, as for a configured remote the clone does not define.

#### Scenario: Publish remote unreachable
- **WHEN** `env` runs while the publish remote cannot be fetched
- **THEN** it refuses with git's reason and sets no variable, and `env -- <command>` does not run the command

### Requirement: Commit without an update
When the integration tip contains the commit, or its update line carries no update, `env` SHALL set no variable, say on standard error that the commit carries no update, and exit with status 0.

#### Scenario: Not an update
- **WHEN** `env` runs on a commit of the integration branch or of a branch whose update line holds no sync merge, copy or journal change
- **THEN** it prints no variable, says that the commit carries no update, exits with status 0, and `env -- <command>` runs the command without any `GIT_INTAKE_` variable

### Requirement: Output of the update variables
By default `env` SHALL print one `export` assignment per variable, quoted for the POSIX shell; with `--dotenv` one `NAME=value` line per variable, unquoted; and with `--json` one JSON object. Given a command after `--`, it SHALL run the command with the variables, without inherited `GIT_INTAKE_` variables, and exit with the command's status, or with status 2 without running it when it cannot read the variables.

#### Scenario: Command with the variables
- **WHEN** `git intake env -- make check` runs on an update's head
- **THEN** `make check` runs with the update's variables and `env` exits with its exit status

#### Scenario: Variables unreadable
- **WHEN** `git intake env -- make check` runs in a shallow clone
- **THEN** `make check` does not run and `env` exits with status 2

#### Scenario: Dotenv lines
- **WHEN** `git intake env --dotenv` runs on an update's head of two upstreams
- **THEN** each line is `NAME=value` with the value unquoted, and `GIT_INTAKE_UPSTREAMS` holds both names separated by a space

### Requirement: Shallow repository
Every command other than `abort` SHALL refuse in a shallow repository before it changes anything, saying that the integration line needs the full history and naming `git fetch --unshallow`.

#### Scenario: Shallow CI clone
- **WHEN** `sync` or `env` runs in a clone made with a limited depth
- **THEN** it refuses, changes nothing, and names how to fetch the full history

### Requirement: Merge request in status
In merge-request mode `status` SHALL show the update's open merge request with its number, address and head and whether the head contains the integration tip, and the newest merge request of the update merged without its history. When it cannot read the forge, it SHALL say why and keep its exit status. `status --json` SHALL carry `merge_request` with `number`, `url`, `head` and `current`, and `merged_without_history` with `number` and `url`, each or null.

#### Scenario: Status with an open merge request
- **WHEN** `status` runs while the update's merge request is open and the integration branch moved past its head
- **THEN** it shows the merge request's number, address and head, says that it is behind the integration branch, and names `sync`

#### Scenario: Status after a squash
- **WHEN** the newest merged merge request of the update was squashed
- **THEN** `status` names it as merged without its history, and `status --json` carries it as `merged_without_history`

### Requirement: Forge checks in doctor
In merge-request mode `doctor` SHALL check that the forge is recognised, that a token is set and accepted, and that the merge settings qualify, failing a check that does not hold and warning when squash or rebase merges stay selectable.

#### Scenario: Doctor with unqualified settings
- **WHEN** `doctor` runs in merge-request mode on a GitLab project that does not require a successful pipeline
- **THEN** the merge settings check fails and names the setting

### Requirement: Update waiting from an earlier release
The next `finish` SHALL hand an update of state format 4 or 5 that waits for its merge request over to the merge request: find it through the API by its recorded number or else by its branch, say whether it is open, merged or closed, record the resolutions of a catch-up step, and remove the update worktree, branch, upstream references and state, keeping the merge request and its branch.

#### Scenario: Hand-over of an open merge request
- **WHEN** `finish` runs on an update of format 5 waiting for its open merge request
- **THEN** no update remains locally, the merge request and its branch are unchanged, and `sync` afterwards finds it open

#### Scenario: Hand-over after the merge
- **WHEN** `finish` runs on such an update whose merge request was merged
- **THEN** no update remains locally, and the next `start` brings the local integration branch up to date

### Requirement: Other commands on a waiting update of an earlier release
`abort --merge-request` SHALL withdraw the open merge request of an update of an earlier release that waits for it and remove the update; `abort` SHALL remove the update and keep the merge request. Every other command that reads such an update SHALL show it as waiting from an earlier release and name `finish`.

#### Scenario: Abort of a waiting update of an earlier release
- **WHEN** `abort --merge-request` runs on an update of format 5 waiting for its open merge request
- **THEN** the merge request is closed, its branch is deleted, and no update remains locally

#### Scenario: Abort after the merge
- **WHEN** `abort` runs on such an update whose merge request was merged
- **THEN** it removes the update and says that the merge request was merged

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
