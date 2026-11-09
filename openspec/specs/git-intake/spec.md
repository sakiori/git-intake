# git-intake Specification

## Purpose

Keep a repository current with one or more upstreams, each fetched by its URL only while it is needed: merged up to a fixed target at the repository root or under its own path, or taken commit by commit through a reviewed plan with a journal of decisions; with isolated updates, recorded verification, guarded publication and per-upstream status; and cut, check and describe the downstream's own releases, versioned after the upstream release they contain.

## Requirements

### Requirement: Fork configuration
The tool SHALL read a repository's settings from a `.git-intake.toml` file committed on the integration branch: the integration branch; an optional publication setting with the publish remote, the mode `push` or `merge-request` and, for merge-request mode or a release table, an optional forge `gitlab` or `github` and API base URL; an install command; ordered named checks as "Named checks" describes them; ordered upstreams; and an optional release table as "Release configuration" describes it.

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
Every sync merge the tool creates for a follow upstream SHALL name the upstream and its target commit in commit trailers; a catch-up merge SHALL carry the trailer `Intake-Catch-Up` instead. Every commit the tool copies from an upstream in `port` mode SHALL name the upstream and the upstream commit it took in the trailers `Intake-Upstream` and `Intake-Taken`. A sync record SHALL count only on a merge commit whose second parent is the record's target.

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
Where the tool compares a remote or URL with an upstream's URL, it SHALL compare each as configured and as git resolves it with its URL rewriting, the address of a `<transport>::<address>` URL standing for it, and URLs naming the same repository SHALL count as equal. A local path or `file://` URL, whatever its host, SHALL name the common git directory git reports for it, or for it with `.git` appended, which a linked worktree names too, made canonical; other URLs compare as hosted URLs.

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

#### Scenario: Linked worktree of an upstream
- **WHEN** the publish remote's push URL is the path of a linked worktree of an upstream's repository
- **THEN** the tool refuses to run and names the remote and the upstream

### Requirement: Same hosted repository
Hosted URLs, the scp-like form read as `ssh`, SHALL count as naming the same repository when host and path match, whatever the scheme and the credentials: the host in lower case without a trailing `.`, a port unless it is the scheme's default, and the path without doubled or trailing `/`, a `.git` suffix in any case, a leading `~/`, and its case. An `ssh` URL SHALL also match by the host and port `ssh -G` resolves for its host, when it answers.

#### Scenario: Another scheme
- **WHEN** an upstream's URL is `https://example.com/team/up.git` and the publish remote's push URL is `ssh://git@example.com/team/up`
- **THEN** the tool refuses to run and names the remote and the upstream, and `sync` and `finish` push nothing

#### Scenario: Home directory
- **WHEN** one URL is `example.com:team/up` and the other `ssh://example.com/~/team/up`
- **THEN** they name the same repository

#### Scenario: Path in another case
- **WHEN** one URL is `https://github.com/Team/Up` and the other `git@github.com:team/up.git`
- **THEN** they name the same repository, as on any other host

#### Scenario: SSH host alias
- **WHEN** an upstream's URL is `ssh://git@example.com/up.git`, the SSH client's configuration makes the host `up-alias` stand for `example.com`, and the publish remote's push URL is `ssh://up-alias/up.git`
- **THEN** the tool refuses to run, and `sync` and `finish` push nothing

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
- **WHEN** `start` names one upstream with a revision and another without, which has no `tags` pattern
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
For a `follow` upstream with a `tags` pattern named without a revision, `start` SHALL list the tags of the upstream's repository from its URL and take as the target the newest release among them, or with `max_conflicts` the release that setting selects. An update SHALL fetch no upstream tag into the repository.

#### Scenario: Repository tags untouched
- **WHEN** an update takes an upstream at a release tag
- **THEN** the repository's tags are the same before and after the update

### Requirement: Newest release of an upstream
The newest release of a `follow` upstream with a `tags` pattern SHALL be, of the tags of the upstream's repository whose name matches the pattern, is a semantic version without build metadata and, unless the upstream sets `prereleases = true`, without a prerelease part, once the pattern's literal prefix, and one `v` or `V` after it unless the prefix ends in one, are removed, and points at a commit of the fetched upstream branch, the one with the highest version by semantic versioning precedence.

#### Scenario: Newest release by version
- **WHEN** an upstream with the pattern `v*` has the tags `v0.9.0` and `v0.10.0` on its branch, `v0.9.0` on the newer commit, and commits after both
- **THEN** `start` takes the commit of `v0.10.0`, and the sync record names the tag `v0.10.0`

#### Scenario: Tags that are not releases
- **WHEN** an upstream without `prereleases` has among its newest matching tags a prerelease such as `v1.0.0-rc.1`, a version with build metadata such as `v1.0.0+build.5`, a name that is no semantic version, or a tag on a branch the fetched branch does not contain
- **THEN** `start` ignores them and takes the newest release among the rest

#### Scenario: Prereleases count
- **WHEN** an upstream with the pattern `v*-nightly.*` and `prereleases = true` has `v1.2.3-nightly.20260102.8` and `v1.2.3-nightly.20260101.7` on its branch
- **THEN** `start` takes the commit of `v1.2.3-nightly.20260102.8`, and the sync record names that tag

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
A revision named to `start` for a `follow` upstream, without a leading `refs/tags/` or `tags/`, SHALL name a tag of the upstream's repository when it has a tag of that name, and otherwise resolve as a revision of the repository. `start` SHALL fetch no upstream tag into the repository, and SHALL refuse a revision that resolves through one of the repository's own tags. `start` SHALL list the tags of every selected `follow` upstream from its URL, and refuse when it cannot.

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
In merge-request mode, and for the release commands in either mode, the tool SHALL reach the publish remote's forge, GitLab or GitHub, through its REST API, reading the publication setting at the integration tip, never at an update's head. It SHALL take the forge from the setting `forge`; otherwise from the host of the remote's fetch URL, `gitlab.com` as GitLab and `github.com` or a `ghe.com` subdomain as GitHub; otherwise from a GitLab CI or GitHub Actions job whose server URL has that host.

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
Forge tokens SHALL come from the environment only: `GITLAB_TOKEN` for GitLab; `GH_TOKEN`, then `GITHUB_TOKEN`, for `github.com` and `ghe.com` hosts; `GH_ENTERPRISE_TOKEN`, then `GITHUB_ENTERPRISE_TOKEN`, for another GitHub host, then in an Actions job on it `GH_TOKEN` and `GITHUB_TOKEN`. Without one, merge requests SHALL be refused before any push, naming the variables; release reads follow "Forge access for releases".

#### Scenario: No token
- **WHEN** merge-request mode is configured and none of the token variables of the forge is set
- **THEN** `finish` and `sync` refuse before pushing anything and name the variables

#### Scenario: Enterprise host outside Actions
- **WHEN** the publish remote is on a GitHub Enterprise Server host, only `GH_TOKEN` is set, and the tool runs outside GitHub Actions
- **THEN** the tool refuses and names `GH_ENTERPRISE_TOKEN` and `GITHUB_ENTERPRISE_TOKEN`

### Requirement: Forge token secrecy
The tool SHALL never print a forge token, a CI job token included, store it, or send it to a host other than the API's, even on a redirect.

#### Scenario: Redirect to another host
- **WHEN** the forge's API answers a request with a redirect to another host
- **THEN** the token does not reach the other host

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
`sync` SHALL take the repository's follow upstreams in without an update state. It SHALL refuse when an update is in progress, naming its step and the next command, and when the configuration has no publication setting. A conflict, except in a replacement's merges, or in push mode failed checks, SHALL stop it with the update in progress; any other end SHALL leave no update it created.

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
`sync` SHALL exit with status 2 when it refused or failed, 1 when it stopped on a conflict or failed checks, found that a replacement conflicts, or left out a follow upstream it could not take, and 0 otherwise.

#### Scenario: Nothing behind
- **WHEN** `sync` runs and every follow upstream is current, with no merge request open
- **THEN** it creates nothing, says that every follow upstream is current, and exits with status 0

#### Scenario: Concurrent runs
- **WHEN** two `sync` runs push the update branch at the same time and the second push is rejected as not a fast-forward
- **THEN** the second run exits with status 2, says to run it again, and opens no merge request

#### Scenario: Upstream left out while the merge request waits
- **WHEN** the update's merge request is open, its head contains the integration tip, and a follow upstream cannot be fetched
- **THEN** `sync` changes nothing, names the upstream it left out, and exits with status 1

### Requirement: Synchronising an open merge request
In merge-request mode `sync` SHALL run no install command and no check. With the update's merge request open, it SHALL fetch the merge request's head and apply "Rewritten merge request branch" first, then "Replacing an untouched merge request". When it does not replace the merge request and the head contains the integration tip, it SHALL change nothing, say that the update waits for the merge request, and name the upstreams that moved on since, which the next update takes.

#### Scenario: Run again without change
- **WHEN** `sync` runs while the update's merge request is open and the integration branch did not move
- **THEN** it pushes nothing, changes no merge request, says that the update waits for the merge request, and exits with status 0

#### Scenario: Upstream moved while the merge request waits
- **WHEN** an upstream moves on while its update's merge request is open, the integration branch did not move, and `replace_after` is not set
- **THEN** `sync` pushes nothing, names the upstream as moved on, and the next `sync` after the merge takes the new commits in a new merge request

### Requirement: Catching up an open merge request
When `sync` does not replace the update's open merge request and its head does not contain the integration tip, `sync` SHALL carry the merge request's update on locally, catch it up with the integration tip under the catch-up preconditions without running the checks, and publish it as `finish` does in merge-request mode.

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
When the update's merge request is not open and the one closed or merged most recently was closed without merging, `sync` SHALL read the follow targets recorded in its description. When it recorded every follow target `sync` would merge, it SHALL start nothing, name the closed merge request, say that it declined these targets, and exit with status 0. A description without readable targets, as that of a merge request closed by a replacement, SHALL decline nothing.

#### Scenario: Merge request closed without merging
- **WHEN** a person closed the update's merge request without merging and no upstream moved since
- **THEN** the next `sync` opens no merge request, names the closed one, and exits with status 0

#### Scenario: Upstream moved after a decline
- **WHEN** an upstream of a declined merge request moves on
- **THEN** the next `sync` opens a merge request with the new target

#### Scenario: Update by hand after a decline
- **WHEN** a person runs `start` and `finish` for the declined targets
- **THEN** the update is published and its merge request opened as for any other

#### Scenario: Merge request closed by a replacement
- **WHEN** the merge request closed most recently was closed by a replacement, and `sync` would merge the targets it held
- **THEN** `sync` publishes the update and opens a merge request for those targets

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
When an update changes the forge's CI definition, `.github/workflows/` on GitHub or the project's CI configuration file on GitLab, or the install command or checks of `.git-intake.toml`, `finish` and `sync` SHALL name those changes in their output, in push mode as in merge-request mode, and in the merge request's description, since the pipeline or the checks that verify the update then run what the update brings. For push mode, see *CI definition of an update published by push*.

#### Scenario: Upstream changes the pipeline
- **WHEN** a root upstream's update changes `.gitlab-ci.yml`
- **THEN** `sync` names the change in its output and in the merge request's description

### Requirement: CI definition of an update published by push
In push mode the forge's CI definition SHALL count as one of the update's own checks when the tool can tell the forge without its API, from the setting `forge`, the publish remote's host or the CI job it runs in; on GitLab it SHALL be the default `.gitlab-ci.yml`.

#### Scenario: Update published by push
- **WHEN** the publish remote's forge is GitLab by the setting `forge`, publication is by push, and a root upstream's update changes `.gitlab-ci.yml`
- **THEN** `finish` and `sync` name the change in their output

### Requirement: When changes to the update's own checks are named in push mode
In push mode `finish` and `sync` SHALL name the changes before the checks run whenever they verify the update themselves, and before the integration branch moves, with or without `--no-publish`.

#### Scenario: Checks that fail
- **WHEN** publication is by push and an update that changes `.gitlab-ci.yml` fails its checks in `sync`
- **THEN** `sync` names the change before it reports the failed checks

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
In merge-request mode `doctor` SHALL check that the forge is recognised, that a token is set and accepted, and that the merge settings qualify, failing a check that does not hold. On GitHub it SHALL warn when squash or rebase merges stay selectable. On GitLab a squash option of `default_on` or `default_off` SHALL pass, with a note that the tool opens and rewrites the update's merge request with squashing off, and SHALL NOT advise changing the option.

#### Scenario: Doctor with unqualified settings
- **WHEN** `doctor` runs in merge-request mode on a GitLab project that does not require a successful pipeline
- **THEN** the merge settings check fails and names the setting

#### Scenario: Squashing left optional on GitLab
- **WHEN** `doctor` runs in merge-request mode on a GitLab project whose settings qualify and whose squash option is `default_on`
- **THEN** every merge settings check passes, and one of them names the option and says that the update's merge request has squashing off

#### Scenario: Squash merges selectable on GitHub
- **WHEN** `doctor` runs in merge-request mode on a GitHub repository whose settings qualify and that allows squash merges
- **THEN** it warns that squash merges stay selectable

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
Before any branch moves, `finish` SHALL refuse an update whose head, after a merge, copy or catch-up, has anything but a directory at `.worktrees` or `.worktrees/git-intake`, or anything at `.worktrees/git-intake/update`, naming the path and why. A checkout of a branch in the main worktree SHALL never follow a move to such a tree. Names SHALL be compared under full Unicode case folding, whatever `core.ignoreCase` says.

#### Scenario: Upstream brings a link
- **WHEN** an upstream's tree has `.worktrees` as a symbolic link, and `finish` runs on the verified update
- **THEN** it refuses, names `.worktrees` and says it is a symbolic link where the update worktree is kept, and neither the integration branch nor the publish remote moves

#### Scenario: Upstream brings a file at the update worktree
- **WHEN** an upstream's tree has a file at `.worktrees/git-intake/update`
- **THEN** `finish` refuses and names `.worktrees/git-intake/update`, and no branch moves

#### Scenario: Name in another case
- **WHEN** an upstream's tree has a symbolic link at `.worktrees/git-intaKe`, written with the Kelvin sign
- **THEN** `finish` refuses and names it, and no branch moves

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
The tool SHALL remove its state, plan and logs only inside its directory in the repository's common git directory, reached through directories alone, and SHALL refuse, removing nothing, when a directory on the way is a symbolic link or not a directory, or the path holds `.` or `..`. A symbolic link inside it SHALL be removed itself, never what it points to.

#### Scenario: Tool directory that is a link
- **WHEN** `<git-common-dir>/git-intake` is a symbolic link to another directory, and the update's logs are removed
- **THEN** the tool refuses, and the other directory keeps its files

### Requirement: Pushes never reach an upstream
Before every push and every deletion of a branch on the publish remote, the tool SHALL resolve where git pushes, the remote's push URLs or else its URLs after git's rewriting, or a URL by the rule `git push` applies to it, a name being a remote whenever git resolves it as one, and SHALL refuse, pushing nothing and moving no branch, when one names an upstream's repository, naming the remote, the URL and the upstream.

#### Scenario: Remote changed during an update
- **WHEN** after `start` the publish remote is given a push URL, or a `url.<base>.pushInsteadOf` rewriting, that names the upstream's repository, and `finish` runs
- **THEN** `finish` refuses, the integration branch stays at the base, and nothing reaches the upstream's repository

#### Scenario: Alias in sync
- **WHEN** the publish remote's push URL is an `insteadOf` alias, an `https` URL with credentials or a `file://localhost/` URL of the upstream's repository, and `sync` runs
- **THEN** `sync` refuses before it starts an update, and nothing reaches the upstream's repository

### Requirement: Messages without credentials
Every message the tool shows, in text or JSON, on standard output or standard error, SHALL pass one sanitising step that removes the password of every URL and the user of an `http` or `https` URL, and replaces every value of at least eight characters of a token variable the tool reads, wherever it appears. Git command lines in messages, including those of git commands that cannot be started, SHALL pass the same step.

#### Scenario: Forge quotes the token
- **WHEN** the forge answers with a message that contains the token, and `doctor`, `doctor --json` or `finish` reports it
- **THEN** the message is shown with `[hidden]` in place of the token

#### Scenario: API URL with credentials
- **WHEN** the setting `api` is an `https` URL with a user and password
- **THEN** the tool refuses it, shows the URL without them, and neither `status`, `doctor` nor their `--json` forms show them

#### Scenario: Git cannot be started
- **WHEN** a git command whose arguments hold a URL with credentials cannot be started
- **THEN** the message names the command without the credentials

#### Scenario: Short token value
- **WHEN** a token variable is set to a value of one character
- **THEN** the output, in text or JSON, is unchanged by it

### Requirement: Upstream URL without credentials
The tool SHALL refuse an upstream whose URL carries a password, or the user of an `http` or `https` URL, naming the upstream and showing the URL without them, so that no credential is committed with the configuration or recorded with an update. The user of an SSH URL names an account and SHALL be accepted.

#### Scenario: Token in an upstream's URL
- **WHEN** an upstream's URL is `https://user:secret@example.com/up.git`
- **THEN** every command refuses the configuration, names the upstream and shows `https://example.com/up.git`, and no output shows `secret`

### Requirement: Release configuration
The configuration MAY carry a `[release]` table with exactly the keys `upstream`, `tags`, `labels` and `versioning`. `upstream` SHALL name a `follow` upstream. `tags` SHALL be a glob over that upstream's tag names, as for an upstream's `tags`, defaulting to the upstream's own `tags`. `versioning` SHALL follow "Release versioning". The tool SHALL refuse the table when neither sets a pattern, or when the configuration names no publish remote, naming the key with the configuration's other problems.

#### Scenario: Upstream that does not follow
- **WHEN** `[release]` names an upstream in `port` mode, or a name no upstream has
- **THEN** every command refuses the configuration and names `release.upstream`

#### Scenario: No release tags
- **WHEN** `[release]` sets no `tags` and the upstream it names has no `tags` pattern
- **THEN** every command refuses the configuration and names `release.tags`

#### Scenario: Release tags without following them
- **WHEN** `[release]` sets `tags = "v*"` and its upstream sets none
- **THEN** updates take that upstream at its branch tip as before, and releases take their version from its `v*` tags

### Requirement: Release labels
`labels` SHALL be a non-empty list of distinct labels, defaulting to `["custom"]`, each a SemVer prerelease identifier of ASCII letters, digits and `-` that is not only digits. Under `dated`, the first SHALL be the label of new releases, and the others are labels that earlier releases took. Under `semver`, every label SHALL be one that earlier dated releases took.

#### Scenario: Numeric label
- **WHEN** `labels` is `["2024"]`
- **THEN** every command refuses the configuration and names `release.labels`

### Requirement: Upstream releases of a commit
An upstream release SHALL be a tag of the release upstream's repository that matches the release `tags` pattern and is a semantic version without a prerelease part or build metadata once the pattern's literal prefix, and one `v` or `V` after it unless the prefix ends in one, are removed. A commit SHALL contain it when its commit exists in the repository and is the commit or an ancestor of it. The tool SHALL list the upstream's tags from its URL and fetch none of them.

#### Scenario: Prefixed release tags
- **WHEN** the release `tags` pattern is `expo-device-hub@*` and the upstream has the tag `expo-device-hub@0.19.1`
- **THEN** that tag is an upstream release of version `0.19.1`

#### Scenario: Prerelease of the upstream
- **WHEN** the upstream has the tag `v0.1.7-nightly.20261031.3991` on a commit the integration tip contains
- **THEN** that tag is no upstream release

### Requirement: Release base
The release base of a commit SHALL be the version of the newest upstream release, by semantic versioning precedence, that the commit contains, with its patch number increased by one and nothing else changed. The tool SHALL refuse, naming the upstream, its URL and the pattern, when the commit contains no upstream release.

#### Scenario: Branch past a release
- **WHEN** the integration tip contains upstream `v0.11.0` and twelve upstream commits after it, and no later release
- **THEN** the release base is `0.11.1`

#### Scenario: Exactly at a release
- **WHEN** the integration tip took the upstream at `v0.11.0` and carries downstream commits on top
- **THEN** the release base is `0.11.1`

#### Scenario: Upstream release not taken
- **WHEN** the upstream has released `v0.12.0` and the integration tip does not contain it
- **THEN** the release base comes from the newest release the tip contains

### Requirement: Release tag names
Under `dated`, a release tag SHALL be named `v<base>-<label>.<YYYYMMDD>.<n>`, where `<base>` is a version `x.y.z` without leading zeros, `<label>` one of the configured labels, `<YYYYMMDD>` a calendar day, and `<n>` a number from 1 to 99 without a leading zero. Its version SHALL be the name without the `v`. Under `semver`, release tags SHALL be named as "Semver release tags" says.

#### Scenario: Day that does not exist
- **WHEN** a tag is named `v0.11.1-custom.20260230.1`
- **THEN** it is not a release tag

#### Scenario: Plain version under dated
- **WHEN** under `dated` the downstream's line carries the tag `v1.0.0`
- **THEN** that tag is no release

### Requirement: Downstream line
The downstream's line SHALL be the commits of the first-parent line of the publish remote's integration branch that the last synced commit of no upstream contains, the last synced commit being the one `status` reports. A release SHALL be a tag of release tag form of the publish remote that points, peeled through an annotated tag, at a commit of the downstream's line. Other tags SHALL be no releases.

#### Scenario: Release of an earlier label
- **WHEN** `labels` is `["nightly", "custom"]` and the downstream's line carries `v0.1.6-custom.20261012.3`
- **THEN** that tag is a release, and new releases take the label `nightly`

#### Scenario: Upstream tags of release form
- **WHEN** the repository carries the upstream's own tags such as `v0.0.21-nightly.20260419.73` on upstream commits of the first-parent line before the downstream's first commit
- **THEN** none of them is a release, and they take no part in the order of releases

#### Scenario: Tag off the line
- **WHEN** a tag of release form points at a commit that a merge brought into the integration branch through its second parent
- **THEN** that tag is no release

### Requirement: Release order
A new release SHALL have a higher precedence by semantic versioning than every other release. Under `dated`, its day and number, compared as `YYYYMMDD * 100 + n`, SHALL also be greater than those of every other release.

#### Scenario: Label that sorts first
- **WHEN** `labels` changes from `["nightly"]` to `["custom", "nightly"]` while a release `v0.1.7-nightly.20261031.1` exists, and a release with base `0.1.7` is cut
- **THEN** the tool refuses, since `v0.1.7-custom.<day>.1` sorts before that release

#### Scenario: Semver release after dated releases
- **WHEN** under `semver` the downstream's line carries `v0.11.1-sakiori.20261103.1` and the next release is `v1.0.0`
- **THEN** `v1.0.0` sorts after that release, and no day or number is compared

### Requirement: Next release
Under `dated`, the next release of a commit SHALL take the commit's release base, the current label, the current UTC day, and the number one past the highest number that any tag of release form of the publish remote took on that day, on the downstream's line or not, whatever its base or label, or else 1. The tool SHALL refuse a next release that breaks "Release order", naming the release it does not come after, on a day that has a tag numbered 99, and whose name the remote already has.

#### Scenario: Second release of a day
- **WHEN** the line carries `v0.11.1-custom.20261101.1` and a release is cut on the same UTC day with the same base
- **THEN** the next release is `v0.11.1-custom.20261101.2`

#### Scenario: Tag of release form off the line
- **WHEN** a tag `v0.11.1-custom.20261101.1` of the publish remote points at a commit off the downstream's line, and a release is cut on 2026-11-01 with the base `0.11.1`
- **THEN** that tag is no release, and the next release is `v0.11.1-custom.20261101.2`

#### Scenario: Base older than a release
- **WHEN** the line carries a release with base `0.12.1` and the tip's release base is `0.11.1`
- **THEN** the tool refuses to name a next release and names the release with base `0.12.1`

### Requirement: Published release
A release SHALL count as published when the forge of the publish remote has a release whose tag is the release tag. A project is expected to create that forge release as the last step of its release, after everything it publishes.

#### Scenario: Tag without a forge release
- **WHEN** a release tag exists and the forge has no release for it
- **THEN** the release is not published

### Requirement: Release state of the tip
`release next` SHALL report, for the publish remote's integration tip, the newest upstream release the tip contains and, under `dated`, the release base that comes from it, and either the release at the tip and whether it is published, or the next release, under `semver` with the change that sets it, or under `semver` that no release is due. It SHALL change nothing, and exit with status 0 when it can report and 2 otherwise.

#### Scenario: Tip changed since the last release
- **WHEN** the integration tip carries no release
- **THEN** `release next` names the next release and creates no tag

#### Scenario: Semver tip with a feature
- **WHEN** under `semver` the previous release is `v1.2.3` and a `feat:` commit landed since
- **THEN** `release next` names `v1.3.0` and the minor change, and no release base

### Requirement: Release state in JSON
`release next --json` SHALL write one object with `tip`, `base`, `upstream_release` (the tag of the upstream release the base comes from), `release` (the release at the tip, or null), `published` (a boolean, or null without a release at the tip or a readable forge), `next` (the next release, or null), `unpublished_previous` (see "Unpublished release below the tip", or null) and `forge_error` (why the forge could not be read, or null).

#### Scenario: Released tip
- **WHEN** the integration tip carries a published release
- **THEN** `release` names it, `published` is true, and `next` is null

### Requirement: Unpublished release below the tip
When the integration tip carries no release, `release next` and `status` SHALL ask the forge whether the newest release on the downstream's line below the tip is published, and name it as `unpublished_previous` when it is not. `release tag` SHALL cut the next release all the same, and warn, naming that release and how a failed release is completed.

#### Scenario: Superseded failure
- **WHEN** the release below the tip was never published and the tip has moved on
- **THEN** `release next` names it as `unpublished_previous`, and `release tag` pushes the next release, warns naming it, and exits with status 0

### Requirement: Cutting a release
`release tag` SHALL act on the publish remote's integration tip, never on the local checkout. When no release points at the tip and the tip takes a next release, it SHALL create that release as an annotated tag on the tip with the message `Release <version>` and git's configured identity, push only that tag to the publish remote, and exit with status 0. It SHALL never move, replace or delete a tag, locally or on the remote.

#### Scenario: Tip changed
- **WHEN** the integration tip carries no release
- **THEN** `release tag` pushes the next release tag at the tip, and the remote's other tags are unchanged

#### Scenario: Local branch behind
- **WHEN** the local integration branch is behind the publish remote's
- **THEN** the tag points at the remote's tip, and the local branch does not move

### Requirement: Released tip
When a published release points at the integration tip, `release tag` SHALL create nothing, say so and exit with status 0.

#### Scenario: Nothing new
- **WHEN** the integration tip carries a published release
- **THEN** `release tag` pushes nothing and exits with status 0

### Requirement: Unpublished release on the tip
When a release points at the integration tip and is not published, `release tag` SHALL create nothing and exit with status 1, naming the tag and saying that its pipeline is still running or failed, and that a failed release is completed by running its pipeline's failed jobs again.

#### Scenario: Failed release
- **WHEN** the tag pipeline of the release at the tip failed before it created the forge release, and the scheduled job runs `release tag` the next day
- **THEN** the job fails, naming the release that is not published

### Requirement: Release tag push
Pushing a release tag SHALL obey "Pushes never reach an upstream". The push SHALL carry that tag alone, whatever git's push configuration says, and create it only if the remote lacks it; when another run created a tag of that name first, `release tag` SHALL report that with exit status 2 and the advice to run it again, and push nothing. The release commands SHALL fetch the publish remote's release tags only into references of their own, and create no tag but the release tag.

#### Scenario: Two runs at once
- **WHEN** two runs of `release tag` name the same next release and the other run pushes first
- **THEN** this run exits with status 2, and the remote's tag is the other run's

#### Scenario: Tags that follow a push
- **WHEN** git's configuration sets `push.followTags` and the repository has an annotated tag the remote lacks
- **THEN** `release tag` pushes the release tag alone

#### Scenario: Publish remote that names the upstream
- **WHEN** the publish remote's push URL names the release upstream's repository
- **THEN** `release tag` refuses, and no tag reaches the upstream's repository

### Requirement: Checking a release tag
`release check [<tag>]` SHALL check the tag named, or else `CI_COMMIT_TAG`, against the publish remote's integration branch and release tags. It SHALL report every problem that "Release tag problems" lists, and exit with status 1 when it found one and 0 otherwise. It SHALL ask whether the release is published only when it found no other problem.

#### Scenario: Pipeline run again
- **WHEN** a release's pipeline failed before it created the forge release, and its failed jobs run again
- **THEN** `release check` passes

### Requirement: Release tag problems
The problems of a release tag SHALL be: it is not of release tag form, under `dated` with the current label; the remote has no such tag; it does not point at the checked-out commit; that commit is not on the downstream's line; under `dated`, its base is not the release base of that commit; it breaks "Release order" against the other releases, among which it does not count itself; it is published; and under `semver` the problems of "Problems of a semver release tag".

#### Scenario: Release already published
- **WHEN** a pipeline of a published release's tag starts again
- **THEN** `release check` exits with status 1 and says the release is published and is never written again

#### Scenario: Tag on a merge request branch
- **WHEN** a release tag points at a commit that is not on the downstream's line
- **THEN** `release check` exits with status 1 and names the commit and the integration branch

### Requirement: Release version for builds
`release version` SHALL print the version a build makes. For a release tag, named by `--tag` or `CI_COMMIT_TAG`, it SHALL be the tag's version once the tag is of release tag form, under `dated` with the current label and a base that is the release base of the tagged commit, and under `semver` with the version of the tagged commit's next release, computed with the tag's upstream release by "Upstream release of a semver release"; otherwise it SHALL refuse with exit status 2.

#### Scenario: Tag pipeline
- **WHEN** the pipeline of `v0.11.1-custom.20261101.2` runs `release version`
- **THEN** it prints `0.11.1-custom.20261101.2`

#### Scenario: Semver tag pipeline
- **WHEN** under `semver` the pipeline of `v1.3.0` runs `release version`
- **THEN** it prints `1.3.0`

### Requirement: Rehearsal version
Without a release tag, `release version` SHALL print the rehearsal version of the checked-out commit, with the UTC day of `CI_PIPELINE_CREATED_AT`, or else of now: under `dated`, `<base>-<label>.<YYYYMMDD>.0` with the current label; under `semver`, `<next>-rehearsal.<YYYYMMDD>`, where `<next>` is the version of the commit's next release, or, when the commit takes no release, the version a patch change would give it. No release SHALL take a rehearsal version.

#### Scenario: Rehearsal
- **WHEN** a pipeline of the integration branch without a tag, created on 2026-11-01, runs `release version` on a commit whose release base is `0.11.1`
- **THEN** it prints `0.11.1-custom.20261101.0`

#### Scenario: Semver rehearsal
- **WHEN** under `semver` a pipeline without a tag, created on 2026-11-01, runs `release version` on a commit whose next release is `v1.3.0`
- **THEN** it prints `1.3.0-rehearsal.20261101`

### Requirement: Release version formats
`release version --json` SHALL write an object with `version`, `tag` (or null) and `rehearsal`. `release version --dotenv` SHALL write the line `RELEASE_VERSION=<version>` and, for a release tag, `RELEASE_TAG=<tag>`. Under `semver`, both SHALL also carry the upstream of "Upstream of a semver build".

#### Scenario: Dotenv in a tag pipeline
- **WHEN** the pipeline of `v0.11.1-custom.20261101.2` runs `release version --dotenv`
- **THEN** it writes `RELEASE_VERSION=0.11.1-custom.20261101.2` and `RELEASE_TAG=v0.11.1-custom.20261101.2`

### Requirement: Release notes
`release notes [<tag>]`, for the tag named or else `CI_COMMIT_TAG`, SHALL write Markdown notes for the integration line after the previous release up to the tag's commit. The previous release SHALL be the newest release before the tag on the downstream's line ending at the tag's commit.

#### Scenario: Notes in a tag pipeline
- **WHEN** the pipeline of a release tag runs `release notes` without an argument
- **THEN** it writes the notes of the tag `CI_COMMIT_TAG` names

### Requirement: Release notes entries
The notes SHALL list, newest first, the subject of each commit of that range that carries no sync record and is neither a catch-up merge nor a merge that brought sync records in, and for each sync record there the upstream's name and its recorded tag, or else the target's abbreviated commit. Under `semver`, they SHALL be ordered as "Breaking changes in the notes" says.

#### Scenario: Feature and update
- **WHEN** since the previous release one squashed feature and one update of the upstream `fff` to its branch tip landed
- **THEN** the notes list the feature's subject and the update of `fff` with the target's abbreviated commit, and none of the upstream's own commits

### Requirement: Notes without changes
Without a previous release, the notes SHALL say that this is the first release. With nothing to list, they SHALL say that there are no changes since the previous release, naming it.

#### Scenario: First release
- **WHEN** `release notes` runs for the first release on the downstream's line
- **THEN** the notes say that this is the first release

### Requirement: Release state in status
When the configuration carries `[release]`, `status` SHALL show what `release next` reports, and `status --json` SHALL carry it as `release`, the object of `release next --json`; without `[release]`, `release` SHALL be null. A forge that cannot be read, or that hides the project, SHALL leave `published` null and not change the exit status of `status`.

#### Scenario: Release state without the forge
- **WHEN** `status --json` runs where the forge cannot be reached
- **THEN** `release.published` is null, `release.forge_error` says why, and the exit status is the one the upstreams give

#### Scenario: Semver release state in status
- **WHEN** under `semver` only `docs:` commits landed since the previous release
- **THEN** `status` says that no release is due, and `status --json` carries `release.next` null and `release.bump` null

### Requirement: Release state errors in status
When `status` cannot read the release state, as when the publish remote cannot be fetched or its tip contains no upstream release, `status --json` SHALL carry `release` null and `release_error` saying why, and the exit status SHALL be the one the upstreams give. Otherwise `release_error` SHALL be null.

#### Scenario: Tip without an upstream release
- **WHEN** `status --json` runs with `[release]` and the publish remote's tip contains no upstream release
- **THEN** `release` is null, `release_error` names the upstream and the pattern, and the exit status is the one the upstreams give

### Requirement: Forge access for releases
The release commands SHALL reach the forge of the publish remote, in either publication mode, only to read the project and its releases. For GitLab they SHALL send `GITLAB_TOKEN`, or else, in a GitLab CI job whose server has the remote's host, `CI_JOB_TOKEN` as a job token; for GitHub the variables of "Forge token". Without a token they SHALL ask without credentials.

#### Scenario: Job token in a tag pipeline
- **WHEN** `release check` runs in a GitLab tag pipeline where only `CI_JOB_TOKEN` is set
- **THEN** it reads the project's release for the tag with the job token

### Requirement: Project hidden from the release commands
When the forge does not show the release commands the project, they SHALL refuse and name the token variables, so that a project hidden from them never reads as a project without releases.

#### Scenario: Private project without a token
- **WHEN** `release next` runs without a token against a private project
- **THEN** it refuses, names the token variables, and reports no release as unpublished

### Requirement: Configuration of the release commands
`release next`, `release tag` and `status` SHALL read the configuration at the publish remote's integration tip, and `release check`, `release version` and `release notes` at the checked-out commit.

#### Scenario: Tag pipeline run again later
- **WHEN** the integration branch changed `[release]` after a release, and that release's pipeline runs again
- **THEN** `release check` and `release version` use the configuration of the tagged commit

### Requirement: Release commands in CI
The release commands SHALL need no local integration branch. They SHALL fetch the publish remote's integration branch and its release tags into references of their own, applying none of the remote's configured refspecs, and leave the repository's branches and tags as they were, except for the tag `release tag` creates.

#### Scenario: Refspecs onto local refs
- **WHEN** the publish remote's configured fetch refspecs map its tags onto the repository's tags and its branches onto local branches
- **THEN** the release commands change no local tag or branch

#### Scenario: Detached tag checkout
- **WHEN** `release check` runs in a CI clone with `HEAD` detached at a tag and no local integration branch
- **THEN** it checks the tag against the publish remote's integration branch and creates no local branch

### Requirement: Upstream releases of the release commands
The release commands SHALL count only tags without a prerelease part, whatever `prereleases` says.

#### Scenario: The release commands ignore prereleases
- **WHEN** a repository whose `[release]` names an upstream with `prereleases = true` computes a release version
- **THEN** its base comes from tags of that upstream without a prerelease part

### Requirement: Conflict limit of a follow target
A `follow` upstream with a `tags` pattern MAY set `max_conflicts` to a positive number. The target of an update naming no revision SHALL then be the newest tag, among those the newest-release rules admit and that contain the last synced commit and the integration branch lacks, whose merge into the integration tip the tool predicts to conflict in at most that many files; when none does, it SHALL be the oldest of them.

#### Scenario: Newest within the limit
- **WHEN** three newer tags predict 40, 120 and 300 conflicted files, from oldest to newest, and `max_conflicts` is 150
- **THEN** `start` takes the second, and says that it passed over the third for the limit

#### Scenario: None within the limit
- **WHEN** every newer tag predicts more files than `max_conflicts`
- **THEN** `start` takes the oldest of them and says so

#### Scenario: Status forecasts the next target
- **WHEN** `status --check` runs for an upstream with `max_conflicts` that is behind
- **THEN** it names the tag an update would take and its predicted conflicts

#### Scenario: Status agrees with start
- **WHEN** an upstream with `max_conflicts` has a tag of a lower version on its branch past the newest release taken in, or the local integration branch lags the publish remote's
- **THEN** `status --check` says what `start` would do, from the tip `start` would update from

#### Scenario: Nothing starts
- **WHEN** `sync` finds the update's open merge request current and plans no replacement: without `replace_after`, or while the merge request is too young, its branch is touched or a pause is recorded
- **THEN** it predicts no conflicts and names no tag as taken

#### Scenario: A replacement is planned
- **WHEN** `replace_after` is set, `sync` finds the update's open merge request current, and its age, branch and the publish remote let it be replaced
- **THEN** it plans the replacement's targets within the limit, as for an update about to start

### Requirement: Reporting and overriding the conflict limit
`start` and `sync` SHALL say which tag they took, the files predicted, and when they passed over a newer tag for the limit. The configuration SHALL refuse `max_conflicts` without `tags`. A revision named to `start` SHALL override the limit.

#### Scenario: A revision overrides the limit
- **WHEN** `start` names a revision for an upstream with `max_conflicts`
- **THEN** that revision is the target, whatever it predicts

### Requirement: Change of the delta
`git intake delta [COMMIT]` SHALL show how the downstream's changes on top of its upstream differ between the update's previous target and its target: a `git range-diff` of two one-commit series, the tree of the update's base on the previous target and the tree of COMMIT on the target. The commits it makes SHALL be reachable from no reference.

#### Scenario: Only what the update changed
- **WHEN** an update's resolution changes three files of the downstream's delta and the upstream's other changes merged cleanly
- **THEN** `delta` shows those three files' changes, and none of the delta the update left as it was

#### Scenario: A lost upstream line
- **WHEN** a resolution leaves out a line the upstream added
- **THEN** `delta` shows that line as a new difference from the upstream

### Requirement: Commit of the delta
COMMIT SHALL default to `HEAD`, and SHALL be resolved from history as `env` resolves it, so the command works in an update in progress, on an update's merge request head, and in a merge request pipeline. When the update takes other than exactly one follow upstream at the repository root, or that upstream has no previous target, or COMMIT carries no update, `delta` SHALL refuse with exit status 2 and say why.

#### Scenario: On a merge request's head
- **WHEN** `delta` is given the head of an update's open merge request after `finish` removed the local update
- **THEN** it shows the same comparison, with the base of the latest catch-up merge

#### Scenario: No update
- **WHEN** COMMIT carries no update
- **THEN** `delta` exits with status 2 and names the commit

### Requirement: Conflicts resolved by updates
`git intake conflicts [--since COMMIT] [--path GLOB]... [--json]` SHALL list the files that conflicted in the sync merges and catch-up merges reachable on the integration branch after COMMIT, recomputed from each merge's parents, one path per line, sorted and without repeats. `--path` SHALL keep the paths that match one of its globs. `--json` SHALL write each merge with its commit, its upstream or catch-up, and its paths. Without `--since`, it SHALL cover the integration branch's whole history.

#### Scenario: Sync merge beneath a catch-up merge
- **WHEN** an update landed with a catch-up merge, so its sync merge lies on the catch-up merge's second parent
- **THEN** the conflicts of both merges are listed

#### Scenario: Filtered by path
- **WHEN** updates resolved conflicts in `src/ui/a.tsx` and `src/server/b.ts`, and `--path 'src/ui/*'` is given
- **THEN** only `src/ui/a.tsx` is listed

#### Scenario: Nothing since the commit
- **WHEN** no update landed after COMMIT
- **THEN** the list is empty and the exit status is 0

### Requirement: Landing an update
`git intake land` SHALL carry the update's open merge request to the integration branch, from any clone and without an update in progress. It SHALL wait for the pipeline of the merge request's head while that runs, and act on its outcome. Before merging it SHALL read the forge's merge settings as `finish` does, and refuse as `finish` does.

#### Scenario: No open merge request
- **WHEN** the update's merge request is not open
- **THEN** `land` says so and exits with status 0

#### Scenario: Merged by someone else
- **WHEN** a person merges the merge request while `land` waits
- **THEN** `land` reports it and exits with status 0

### Requirement: Merging a landed update
When the pipeline passed and the head contains the integration tip, `land` SHALL merge that head, never squashed and never rebased, naming the head so that a head moved meanwhile is not merged, and it SHALL exit with status 0 once the forge reports the merge. When the pipeline passed and the integration branch moved, it SHALL take the new tip in by a catch-up merge as `sync` does, and go on with the new pipeline.

#### Scenario: The branch did not move
- **WHEN** the pipeline of the update's head passes and the head contains the integration tip
- **THEN** `land` merges it without a catch-up merge, and the sync records stay on the integration line

#### Scenario: The branch moved
- **WHEN** the pipeline passes and the integration branch moved past the head
- **THEN** `land` pushes a catch-up merge, waits for its pipeline, and merges it

#### Scenario: The branch moved while the pipeline ran
- **WHEN** the integration branch moves while the pipeline still runs
- **THEN** `land` waits for that pipeline's outcome before catching up

#### Scenario: A head the forge has not read
- **WHEN** the update's branch holds a newer head than the forge reports
- **THEN** `land` waits for the forge to read it, and merges nothing on the older head's pipeline

#### Scenario: The branch moves under the merge
- **WHEN** the forge refuses the merge because the integration branch moved just after `land` read it
- **THEN** `land` catches up and goes on

#### Scenario: A required check that has not registered
- **WHEN** on GitHub a required check has no run or status yet on the head, whatever the others say
- **THEN** `land` waits for it instead of trying the merge

#### Scenario: A read refused for a permission
- **WHEN** GitHub refuses to show a token the check runs or commit statuses of the head
- **THEN** `land` exits with status 2 and names the permission that gives them

### Requirement: What a landing asks of the merge
A squash the merge request itself asks for SHALL be cleared before the merge. A merge the forge refuses while it reads the merge request as mergeable SHALL be tried again a bounded number of times, and a refusal SHALL name the forge's reason. On GitLab the forge SHALL be asked to remove the source branch; on GitHub, whose merge cannot be asked that, `land` SHALL remove it under a lease on the head merged.

#### Scenario: A squash asked for
- **WHEN** a person ticked squash on the update's merge request
- **THEN** `land` clears it before it merges, and the sync records stay on the integration line

#### Scenario: A refusal that passes
- **WHEN** the forge refuses the merge and reads the merge request as mergeable, and then takes it
- **THEN** `land` merges it without stopping

#### Scenario: A branch pushed to after the merge
- **WHEN** on GitHub the source branch moved past the head merged
- **THEN** `land` keeps the branch and says so

### Requirement: Stops of a landing
A failed or canceled pipeline, or a conflict in the catch-up merge, SHALL stop `land` with exit status 1, naming what to do next. It SHALL exit with status 2 when it is refused, when the forge keeps failing, or after `--timeout`, by default six hours. A 5xx answer or a network error SHALL NOT end the wait until a bounded number of failed polls in a row, and the integration branch moving during the catch-up SHALL make `land` read again rather than stop.

#### Scenario: Failed pipeline
- **WHEN** the pipeline fails
- **THEN** `land` exits with status 1 and names the pipeline's address, and a fix commit on the update's branch followed by `land` carries on

#### Scenario: Conflict in the catch-up
- **WHEN** the catch-up merge conflicts
- **THEN** `land` exits with status 1 in the step `catch_up_conflict`, and `continue`, `finish` and `land` carry on

#### Scenario: An outage
- **WHEN** reads of the forge fail for fewer polls than the limit
- **THEN** `land` goes on waiting, and past the limit it exits with status 2 with the pause ended

#### Scenario: A failed fetch
- **WHEN** fetching or listing the publish remote fails
- **THEN** `land` counts it as a failed poll, and gives no advice to withdraw the merge request

#### Scenario: The branch moves during the catch-up
- **WHEN** the integration branch moves between the catch-up merge and its push
- **THEN** `land` pushes nothing and reads again

#### Scenario: A local integration branch with commits of its own
- **WHEN** a catch-up is due and the local integration branch has commits the publish remote lacks
- **THEN** `land` refuses as `sync` refuses it, before it pauses the queue

### Requirement: Failures of git's own network commands
A failed `git fetch`, `git ls-remote` or `git push` of the publish remote SHALL count as a failure that may pass, whatever git says of it, since a busy endpoint can answer as if the credential were refused. `land` SHALL give it the bounded retries of a failed read of the forge. A lost swap of the record of a pause, and a push another run's push rejected, SHALL NOT be tried again.

#### Scenario: One failed connection during a landing
- **WHEN** any one connection to the publish remote fails during a `land` that pauses the queue, catches up and merges, with what reads as a refused credential
- **THEN** the merge request merges, the paused merge requests return, and the record goes

#### Scenario: One failed connection while resuming
- **WHEN** any one connection to the publish remote fails during `land --resume`
- **THEN** the pause still ends

#### Scenario: A lost swap
- **WHEN** a swap of the record of a pause finds the reference at another value than it requires
- **THEN** the push is made once, and the run stops with exit status 2

### Requirement: Failures that may pass during a sync
In merge-request mode `sync` SHALL run again, a bounded number of times, when a run fails on what may pass: a 5xx or a network error of the forge, or a failed `git fetch`, `git ls-remote` or `git push` of the publish remote. Such a run leaves nothing behind, so the next one meets the same situation. In push mode only reading the publish remote before the update starts SHALL be tried again.

#### Scenario: A failed read just before the push
- **WHEN** the catch-up merge is made and `git ls-remote` fails once just before the push, reading like a refused credential
- **THEN** `sync` runs again, publishes the catch-up and exits with status 0

#### Scenario: An outage that lasts
- **WHEN** every connection to the publish remote fails
- **THEN** `sync` exits with status 2 after its attempts, with no update left

#### Scenario: Another run pushed first
- **WHEN** the push of the update's branch is rejected because another run pushed it
- **THEN** `sync` reports it with status 2 without running again

### Requirement: The publication after the push
Once the update's branch is pushed, the request that opens its merge request or rewrites its title and description SHALL be made again in place when it fails on what may pass, as often as a run, since a new run would find nothing left to do.

#### Scenario: The description fails after the push
- **WHEN** the catch-up is pushed and the request that rewrites the merge request's description fails once with a 5xx
- **THEN** that request is made again, the description is rewritten, and `sync` exits with status 0

### Requirement: Signals during a landing
SIGINT, SIGTERM and SIGHUP SHALL make `land` end its pause and exit with status 130, 143 and 129. A second SIGINT or SIGTERM SHALL end the process at once; a second SIGHUP SHALL count for nothing, since a terminal that hangs up can deliver it twice. A process killed in any way that cannot be caught SHALL leave a pause that `land --resume` ends.

#### Scenario: SIGTERM during a pause
- **WHEN** `land` gets SIGTERM during a pause
- **THEN** the paused merge requests return in their order, the record goes, and it exits with status 143

#### Scenario: The terminal hangs up
- **WHEN** the terminal of `land` hangs up during a pause, and SIGHUP comes once or twice
- **THEN** the paused merge requests return in their order, the record goes, and it exits with status 129

#### Scenario: SIGINT in the middle of the catch-up merge
- **WHEN** the terminal's SIGINT reaches `land` and the `git merge` of its catch-up
- **THEN** the pause ends, and `land` exits with status 130 rather than reporting the merge's failure

#### Scenario: SIGKILL during a pause
- **WHEN** `land` is killed with SIGKILL during a pause
- **THEN** the record stays, and `land --resume` returns the merge requests

### Requirement: A landing without its terminal
A signal `land` was started ignoring, as `nohup` starts it for SIGHUP, SHALL stay ignored. A message that cannot be written, to a closed pipe or a terminal that hung up, SHALL be lost without stopping `land`.

#### Scenario: Output gone
- **WHEN** the reader of the output of `land` goes away during a pause
- **THEN** `land` goes on, and the pause ends as on any other way out

#### Scenario: Started with nohup
- **WHEN** `land` started with SIGHUP ignored gets SIGHUP during a pause
- **THEN** it goes on landing

### Requirement: A result without its reader
A command's result, its text or JSON form, the variables `env` writes or the notes `release notes` writes, that cannot be written to standard output, as to a closed pipe, SHALL end the command with exit status 2 and an error that says so, so that nothing takes a cut-off result for a whole one. A message SHALL NOT count as a result.

#### Scenario: The reader of a result gone
- **WHEN** `status --json` runs with its standard output a pipe that nobody reads
- **THEN** it exits with status 2 and says that it cannot write the result

### Requirement: Merge queue of the project
`[publish]` MAY name `queue`, the account of a merge queue that takes the merge requests assigned to it. It SHALL be refused on GitHub and with `via = "push"`. `doctor` SHALL check that the account exists on the forge.

#### Scenario: Queue on GitHub
- **WHEN** a configuration whose publish remote is on GitHub names `queue`
- **THEN** the commands that need the forge refuse it and name the setting

#### Scenario: Unknown account
- **WHEN** `queue` names an account the forge does not know
- **THEN** `doctor` fails and names it

### Requirement: Pausing the merge queue
When `queue` is set, the pipeline of the update's head passed, and the integration branch moved past that head, `land` SHALL pause the queue before it catches up. It SHALL never pause while the update's pipeline has no outcome, and when the branch did not move it SHALL not pause at all. The queue's merge requests, those open against the integration branch and assigned to the account, other than the update's, SHALL be taken in the order of their latest assignment to it.

#### Scenario: Order kept
- **WHEN** merge requests 7, 5 and 9 were assigned to the queue in that order
- **THEN** the record names 7, 5 and 9 in that order, before any of them loses its assignment

### Requirement: A pause that cannot help
`land` SHALL stop without touching the queue when the forge's merge status says the merge request cannot merge for a reason a catch-up does not change, and it SHALL not pause for a catch-up predicted to conflict.

#### Scenario: A merge that cannot happen
- **WHEN** the update's merge request is a draft, or lacks its approvals, and the integration branch moved past its head
- **THEN** `land` exits with status 2, names the merge status, and no assignment changes

#### Scenario: A conflict foreseen
- **WHEN** the catch-up merge is predicted to conflict
- **THEN** `land` stops on that conflict with status 1, and no assignment changes

### Requirement: Record of a pause
A pause SHALL be recorded in one reference on the publish remote, `refs/git-intake/pause`, which exists exactly while the pause lasts and points at a parentless commit that nothing else reaches. The record SHALL name the queue's account, the merge request the pause was started for, each held merge request with the assignees it kept, in the order they return in, and the merge requests in flight when the pause began.

#### Scenario: Recorded first
- **WHEN** `land` pauses the queue
- **THEN** the reference holds the record before the first assignment is taken away

### Requirement: Reading the record of a pause
The record SHALL be read with git alone, by listing the reference and fetching it, with no forge API and no token beyond what git uses. A record that cannot be read SHALL be reported as such and never taken for no pause.

#### Scenario: A note is no record
- **WHEN** a note on a merge request names merge requests or says that a pause ended
- **THEN** no command acts on it

#### Scenario: A record that cannot be read
- **WHEN** the reference points at something that is no record
- **THEN** `status` says so and keeps the rest of its view, `doctor` fails, and `land --resume` exits with status 2 and changes nothing

### Requirement: Writing the record of a pause
The record SHALL be written by compare-and-swap: creating it SHALL require the reference to be absent, updating it SHALL require the value read, and ending the pause SHALL delete it under the same lease, with `git push --force-with-lease=<ref>:<expected>`, empty for absent. A run that loses the swap SHALL touch no assignment and stop with exit status 2, naming another run as the likely cause. A run whose record has disappeared SHALL hold no merge request any more, and SHALL not pause again.

#### Scenario: Another run paused first
- **WHEN** the reference appears after `land` read it, as it is about to pause
- **THEN** it takes no assignment away, exits with status 2 and names another run

#### Scenario: Another run replaced the record
- **WHEN** the reference holds another value than the one a run last read
- **THEN** the run touches no assignment, leaves the reference, and exits with status 2

#### Scenario: A resume ended the pause
- **WHEN** `land --resume` ends a pause while a `land` runs
- **THEN** that run stops holding merge requests, a merge request assigned to the queue since stays assigned, and the run goes on landing

#### Scenario: The remote refuses the push
- **WHEN** the publish remote refuses the push of the record
- **THEN** `land` exits with status 2 and no assignment changes

### Requirement: A record read back
A record just created SHALL be read back from where every run reads it, the remote's fetch URL, before any assignment is taken; when it does not read back, the run SHALL remove it from where it pushed it, touch no assignment and stop with exit status 2. Whether the removal succeeded SHALL be judged by the push that removes it, and a removal that failed SHALL be named.

#### Scenario: A record pushed elsewhere
- **WHEN** the publish remote pushes to another repository than it fetches from
- **THEN** `land` removes the record it pushed, exits with status 2 naming the push URL, and no assignment changes

#### Scenario: A record that cannot be taken back
- **WHEN** the publish remote pushes to another repository than it fetches from, and that repository refuses the removal of the record
- **THEN** `land` exits with status 2, says that the record could not be removed from where it went, and no assignment changes

### Requirement: A record that cannot be read back
A record whose reading back fails, rather than answering with another value, SHALL stay, and the run SHALL touch no assignment, stop with exit status 2 and name `land --resume`, without taking the failure for a push elsewhere. A read that answered with another value SHALL decide for a push elsewhere, whatever the reads after it, since a failed read tells nothing.

#### Scenario: Git fails once the record is pushed
- **WHEN** git cannot reach the publish remote right after the record was pushed
- **THEN** `land` exits with status 2 naming `land --resume`, the record stays, no assignment changes, and `land --resume` ends the pause once git reaches the remote

### Requirement: Notes of a pause
A plain note on the merge request a pause was started for SHALL say, for people, that the queue is paused and which merge requests wait, and another SHALL say when the pause ended. A note SHALL hold no machine-readable block and decide nothing, and a failure to write one SHALL be a warning and not a stop.

#### Scenario: A note that cannot be written
- **WHEN** the forge refuses a note
- **THEN** `land` warns, and the pause and the landing go on as if it had been written

### Requirement: Merge requests a pause holds
A merge request whose head contains the integration tip and whose pipeline runs, waits or passed SHALL stay assigned, and `land` SHALL wait for it before it catches up. Each other SHALL lose its assignment. While the pause lasts `land` SHALL hold any merge request assigned meanwhile, after the ones it holds, recording it first.

#### Scenario: A merge request in flight
- **WHEN** the queue has rebased merge request 7 onto the integration tip and its pipeline runs
- **THEN** 7 stays assigned, the others are paused, and `land` waits for 7 before catching up

#### Scenario: Assigned during a pause
- **WHEN** merge request 11 is assigned to the queue during a pause
- **THEN** it is paused too, and returns after the ones paused before it

### Requirement: The wait for merge requests in flight
The wait for the merge requests in flight SHALL be for at most one pipeline: once one of them has merged or stopped being current the others SHALL be held, and one whose pipeline passed that the queue does not take SHALL be held after a bounded wait. A merge request in flight when a pause began SHALL keep its assignment when a later run carries the pause on.

#### Scenario: One of several in flight landed
- **WHEN** two merge requests are in flight and one of them merges
- **THEN** the other is held, and `land` does not wait for a pipeline of its own

#### Scenario: A queue that does not take a passed merge request
- **WHEN** a merge request in flight has a passed pipeline and the queue does not merge it
- **THEN** it is held after a bounded wait, well before `--timeout`

#### Scenario: A carried pause
- **WHEN** a later run carries on a pause whose record names a merge request in flight
- **THEN** that merge request keeps its assignment

### Requirement: Ending a pause
`land` SHALL end a pause on every way out: a merge, a failed pipeline, a conflict, a refusal, a timeout, a signal it can catch and an error of its own. Ending a pause SHALL assign the account to each paused merge request that is still open and has the assignees the pause left it with, keeping those assignees, in the recorded order, at least a second apart, and SHALL then write a note that the pause ended.

#### Scenario: Merge
- **WHEN** the update merges during a pause
- **THEN** the paused merge requests return in their order

#### Scenario: Failed pipeline during a pause
- **WHEN** the pipeline of the catch-up merge fails
- **THEN** the paused merge requests return before `land` exits

#### Scenario: A merge request that moved on
- **WHEN** a paused merge request has since merged, closed or changed its assignees from those the pause left it with
- **THEN** ending the pause leaves it as it is

#### Scenario: A merge request with another assignee
- **WHEN** a merge request assigned to the queue's account and to a person is paused, and its assignees do not change during the pause
- **THEN** it keeps the person throughout, and ending the pause assigns the account again beside the person

#### Scenario: A merge request deleted since
- **WHEN** a paused merge request answers 404, and the project still answers to the token
- **THEN** it counts as left as it is, and the others return

#### Scenario: A token that cannot see the project
- **WHEN** a paused merge request answers 404, and so does the project, as for a token without access to it
- **THEN** the run stops at that merge request, the record stays, and `land` and `land --resume` exit with status 2

### Requirement: The record while a pause ends
`land` SHALL delete the record after the last return and before the note that the pause ended, so that a run killed in between can be resumed, and it SHALL try each step of ending a pause again a bounded number of times when the forge or the remote fails in a way that may pass.

#### Scenario: Killed between the returns and the deletion
- **WHEN** a run dies after it returned merge requests and before it deleted the record
- **THEN** `land --resume` finds the record, returns the rest, and leaves those already returned as they are

#### Scenario: An outage while ending
- **WHEN** a step of ending a pause keeps failing past its attempts
- **THEN** the record stays, `land` exits with status 2 and names `land --resume`, which returns the rest in their order

#### Scenario: Git fails while the forge answers
- **WHEN** reading the record keeps failing as a pause ends, and the forge answers
- **THEN** the paused merge requests return all the same, the record stays, and `land` exits with status 2 and names `land --resume`, which removes it

### Requirement: A pause left behind
`land --resume` SHALL end the pause the publish remote records, whichever merge request it was started for and in whatever state that merge request is, with an update in progress or not, and do nothing else. A plain `land` that finds a recorded pause SHALL carry it on, and end it when the update has no open merge request. `status` and `doctor` SHALL name a recorded pause and its merge requests, without a request to the forge's API for it, and `sync` and `start` SHALL warn of one.

#### Scenario: The process dies
- **WHEN** `land` is killed during a pause
- **THEN** `status` names the pause, and `land --resume` from another clone returns the merge requests in their order

#### Scenario: Pause on a merged merge request
- **WHEN** the update's merge request merged and the process died before it returned the queue
- **THEN** `land --resume` finds the pause and returns them

#### Scenario: A newer merge request
- **WHEN** the merge request a pause was started for merged or closed and a newer one is open
- **THEN** `status`, `doctor`, `land --resume` and a plain `land` still find the pause, and `sync` and `start` warn of it

#### Scenario: An update in progress
- **WHEN** `land --resume` runs in a clone with an update in any step
- **THEN** it ends the pause and leaves the update as it is

#### Scenario: The forge cannot be read
- **WHEN** `status` cannot read the forge, or has no token
- **THEN** it still names a recorded pause, and a failure of the forge's notes takes nothing from its view of the merge request

#### Scenario: Nothing to end
- **WHEN** `land --resume` finds no recorded pause
- **THEN** it says so, changes nothing and exits with status 0

### Requirement: Pushing with the CI job token
Where `CI_JOB_TOKEN` is set, `sync` in merge-request mode and `release tag` SHALL refuse with status 2 at the start of every run, pushing nothing and changing no merge request, when a push to the publish remote would use that token. `sync` in push mode SHALL warn instead and go on, since a project may let the job token push.

#### Scenario: A rewrite of the runner, with nothing to do
- **WHEN** `sync` runs in merge-request mode in a job whose push URL carries `CI_JOB_TOKEN`, and every upstream is current
- **THEN** it exits with status 2, says that the push URL carries the job token, pushes nothing and changes no merge request

#### Scenario: A helper of the runner
- **WHEN** `sync` runs in merge-request mode in a job whose push URL has no password, and git's credential helpers answer that URL with `CI_JOB_TOKEN`
- **THEN** it exits with status 2 and says that the credential helpers answer with the job token

#### Scenario: A credential of the job's own
- **WHEN** the push URL carries another password, or the helpers answer with another token
- **THEN** the run goes on, whatever user name the URL or the helper gives

#### Scenario: A release tag
- **WHEN** `release tag` runs in a job whose push URL carries `CI_JOB_TOKEN`
- **THEN** it exits with status 2 and creates no tag

#### Scenario: Push mode
- **WHEN** `sync` runs in push mode in a job whose push URL carries `CI_JOB_TOKEN`
- **THEN** it warns of the job token and goes on

#### Scenario: Outside a job
- **WHEN** `CI_JOB_TOKEN` is not set
- **THEN** neither the push URL nor the credential helpers are checked

### Requirement: Telling a push with the CI job token
A push SHALL count as using the token when a push URL of the remote as git resolves it carries the token as its password, or when that URL is an `http` or `https` URL without a password and git's credential helpers answer with the token, asked without any prompt. The refusal SHALL say which of the two it found and where the README shows how to push instead, and SHALL show neither the URL nor the token.

#### Scenario: A refusal shows no secret
- **WHEN** `sync` refuses a push URL that carries `CI_JOB_TOKEN`
- **THEN** its message shows neither that URL nor the token

### Requirement: Landing with the CI job token
Where `CI_JOB_TOKEN` is set and a push to the publish remote would use it, as "Telling a push with the CI job token" tells it, `land` SHALL refuse with status 2 at its start, before it records a pause or changes any merge request, since its catch-up push needs a pipeline and a push made with the job token starts none. A refusal while a pause is recorded SHALL name `land --resume`. `land --resume`, which needs no pipeline, SHALL warn instead and go on.

#### Scenario: Land in a job that pushes with the job token
- **WHEN** `land` runs in a job whose push URL carries `CI_JOB_TOKEN`, with a merge request assigned to the queue
- **THEN** it exits with status 2, says that the push URL carries the job token, records no pause and changes no merge request

#### Scenario: Resume in a job that pushes with the job token
- **WHEN** `land --resume` runs in a job whose push URL carries `CI_JOB_TOKEN`, with a pause recorded
- **THEN** it warns of the job token and gives the held merge requests back, and when the push that removes the record fails, the record stays and it exits with status 2

### Requirement: Replacement setting
The publication setting SHALL accept `replace_after`, the shortest time the update's merge request stays open before `sync` may replace it, as a positive duration of whole weeks, days, hours and minutes, such as `7d`, `36h`, `2w` or `1d 12h`, in which a day counts as 24 hours. Without `replace_after`, `sync` SHALL never replace a merge request.

#### Scenario: Value that is no duration
- **WHEN** `replace_after` is `"0d"`, `"-7d"`, `"1 month"` or `"soon"`
- **THEN** the tool refuses to run, names `replace_after` and the value, and says which durations it accepts

### Requirement: Refused replacement setting
The tool SHALL refuse `replace_after` with publication by push, and in a configuration without a `follow` upstream that has a `tags` pattern, naming the setting and the reason.

#### Scenario: Setting in push mode
- **WHEN** the publication setting has `via = "push"` and `replace_after = "7d"`
- **THEN** the tool refuses to run, names `replace_after`, and says that it needs merge-request mode

#### Scenario: No upstream with releases
- **WHEN** the publication setting has `replace_after` and no `follow` upstream has a `tags` pattern
- **THEN** the tool refuses to run, names `replace_after`, and says that only an upstream with `tags` triggers a replacement

### Requirement: Untouched merge request
The update's open merge request SHALL count as untouched only when its branch and its activity are both untouched. Its branch SHALL be untouched when every commit on its head's update line is a sync merge of a `follow` upstream or a catch-up merge, and every sync merge's record names in `Intake-Chosen` the rule by which the tool chose its target.

#### Scenario: Fix pushed by a person
- **WHEN** a person pushed a commit onto the update's merge request branch, or merged the integration branch into it with the forge's update button
- **THEN** the merge request is not untouched

#### Scenario: Chosen revision
- **WHEN** a person published the update with `start` naming a revision for an upstream, by its commit or by a tag such as the newest release, and `finish`
- **THEN** the merge request is not untouched, since the sync record of that upstream names no rule in `Intake-Chosen`

#### Scenario: Merge of an earlier release
- **WHEN** the sync merge on the update's merge request branch was made by a release that wrote no `Intake-Chosen`
- **THEN** the merge request is not untouched

### Requirement: Targets and resolutions on an untouched branch
On an untouched branch, every sync merge of an upstream with a `tags` pattern SHALL name a tag that the upstream's newest-release rules admit as a target, and the tree of every sync merge and catch-up merge of the update line SHALL be the clean merge of its parents, as `conflicts` recomputes it: a merge that needed a resolution, or holds changes beyond that merge, SHALL make the merge request touched.

#### Scenario: Prerelease target
- **WHEN** an upstream sets `prereleases = true`, and the sync merge of the update's merge request names its prerelease tag `v1.2.0-rc.1`
- **THEN** the branch of the merge request is untouched

#### Scenario: Resolved conflict
- **WHEN** `sync` stopped on a conflict of the update's merge, and a person resolved it, ran `continue` and published the update with `finish`
- **THEN** the merge request is not untouched

#### Scenario: Changed merge
- **WHEN** a person amended the clean sync merge on the update's merge request branch with a change of their own, keeping its message and parents
- **THEN** the merge request is not untouched

### Requirement: Activity on an untouched merge request
The activity of the update's open merge request SHALL be untouched when, as the forge reports it, nobody approved or reviewed it, on GitHub with a review in any state, it has no comment other than the forge's own system notes, nobody is assigned to it, it is not a draft, and it is not set to merge automatically. When the forge does not let the token read any of these, the merge request SHALL not count as untouched, and `sync` SHALL say what it could not read.

#### Scenario: Comment or approval
- **WHEN** a person commented on the update's merge request, approved it, reviewed it on GitHub, asked someone to review it, reacted to it with an emoji, assigned it, marked it as a draft or set it to merge automatically
- **THEN** the merge request is not untouched

#### Scenario: Catch-ups by the tool
- **WHEN** the update's merge request was opened by `sync` and has since been caught up only by `sync`, and the forge shows only the system notes of those pushes and description changes
- **THEN** the merge request is untouched

#### Scenario: Notes of a landing
- **WHEN** `land` wrote a note on the update's merge request about a pause of the merge queue
- **THEN** the merge request is not untouched

### Requirement: Review requests, reactions and schedules on an untouched merge request
The activity of the update's open merge request SHALL be untouched only when, as the forge reports it, nobody, person or team, is asked to review it, nobody reacted to the merge request itself with an emoji, and, on GitLab, nobody set `merge_after`. A request or reaction the token cannot read SHALL count as one; an answer without `merge_after`, as from an instance without the field, SHALL show no schedule.

#### Scenario: Reactions the token cannot read
- **WHEN** a GitHub token cannot read the reactions of the pull request
- **THEN** the pull request counts as touched, and `sync` says that its reactions could not be read

#### Scenario: Scheduled merge
- **WHEN** a person set `merge_after` on the update's merge request
- **THEN** the merge request is not untouched

### Requirement: Replacing an untouched merge request
With `replace_after` set and the update's merge request open and not rewritten, `sync` SHALL plan a replacement as "Plan of a replacement" describes, and replace the merge request when the plan takes a newer release, leaves no `follow` upstream out, queues every `follow` upstream the merge request's update takes and moves none back, and the merge request is untouched and was created at least `replace_after` ago, unless "Replacement during a pause" or "Merge request being merged" keeps it.

#### Scenario: Newer release after the wait
- **WHEN** `replace_after` is `7d`, the update's merge request at the release `v1.2.0` of an upstream with `tags` was opened by `sync` eight days ago and nobody touched it, and the upstream released `v1.3.0`
- **THEN** `sync` closes that merge request unmerged, deletes its branch, opens a new merge request from `git-intake/update` whose update merges `v1.3.0` into the integration tip, and exits with status 0

#### Scenario: Several releases while waiting
- **WHEN** the upstream released `v1.3.0` and `v1.4.0` while the untouched merge request waited for longer than `replace_after`
- **THEN** a single replacement takes `v1.4.0`

#### Scenario: Merge request too young
- **WHEN** the upstream released a newer version while the untouched merge request has been open for less than `replace_after`
- **THEN** `sync` keeps the merge request, says from when it may replace it, and otherwise acts as without `replace_after`

#### Scenario: Touched merge request
- **WHEN** the upstream released a newer version, the merge request has been open for longer than `replace_after`, and a person commented on it
- **THEN** `sync` keeps the merge request, says that it is not replaced because of the comment, and otherwise acts as without `replace_after`

#### Scenario: Upstream that cannot be fetched
- **WHEN** a newer release appeared for one upstream of the merge request, and another upstream of the merge request cannot be fetched
- **THEN** `sync` replaces nothing, names the upstream it left out, and exits with status 1

### Requirement: Plan of a replacement
`sync` SHALL plan a replacement from the integration tip as "Synchronising without an open merge request" does, with the targets an update about to start takes: the newest releases by the newest-release rules, within the limit of "Conflict limit of a follow target" where an upstream sets `max_conflicts`, and the fetched tip of an upstream without a `tags` pattern.

#### Scenario: Newer prerelease
- **WHEN** an upstream sets `prereleases = true`, the untouched merge request at `v1.2.0-rc.1` waited for longer than `replace_after`, and the upstream tagged `v1.2.0-rc.2`
- **THEN** `sync` replaces the merge request with an update at `v1.2.0-rc.2`

#### Scenario: Release within the conflict limit
- **WHEN** an upstream sets `max_conflicts = 1`, the untouched merge request at `v1.2.0` waited for longer than `replace_after`, and the upstream released `v1.3.0`, predicted to conflict in no file, and `v1.4.0`, predicted to conflict in two
- **THEN** `sync` replaces the merge request with an update at `v1.3.0` and says that it passed over `v1.4.0` for the limit

### Requirement: Order of the replacement checks
`sync` SHALL make the checks that ask nothing of the forge, the merge request's age, branch and a pause, before the conflict predictions of the plan of a replacement, and read the merge request's activity last.

#### Scenario: Merge request too young for a prediction
- **WHEN** an upstream sets `max_conflicts`, a newer release came out, and the merge request is younger than `replace_after`
- **THEN** `sync` predicts no conflicts, reads no activity, and says from when it may replace the merge request

### Requirement: Merge request being merged
`sync` SHALL NOT replace a merge request that GitLab reports `locked`, the state of a merge in progress.

#### Scenario: Merge in progress
- **WHEN** GitLab reports the untouched merge request `locked`, as during its merge, and a newer release came out
- **THEN** `sync` keeps the merge request and says that GitLab is merging it

### Requirement: No step back in a replacement
For every `follow` upstream of the merge request's update, the target the plan of a replacement takes SHALL be the merge request's target or contain it, and for an upstream with a `tags` pattern be a higher version by its newest-release rules; otherwise `sync` SHALL keep the merge request and say which upstream the replacement would move back.

#### Scenario: Older tag within the conflict limit
- **WHEN** the untouched merge request takes `app-a` at `v1.2.0`, `app-b` released `v1.1.0`, and `max_conflicts` of `app-a` now selects its `v1.1.0`
- **THEN** `sync` keeps the merge request and says that the replacement would move `app-a` back

#### Scenario: Maintenance release
- **WHEN** the untouched merge request takes `v1.3.0`, and the plan takes `v1.2.5`, a tag of a maintenance branch that does not contain `v1.3.0`
- **THEN** `sync` keeps the merge request and says that `v1.2.5` does not contain `v1.3.0`

### Requirement: Replacement during a pause
While the publish remote records a pause of the merge queue, or its record cannot be read, `sync` SHALL NOT replace the update's merge request, and SHALL say so when the plan takes a newer release.

#### Scenario: Recorded pause
- **WHEN** the upstream released a newer version, the untouched merge request has been open for longer than `replace_after`, and the publish remote records a pause of the merge queue
- **THEN** `sync` keeps the merge request without reading its activity, says that it is not replaced because a pause is recorded, and otherwise acts as without `replace_after`

### Requirement: Newer release for an open merge request
A plan SHALL take a newer release for the update's open merge request when a queued upstream with a `tags` pattern has a target the merge request's head lacks, which is a tag the upstream's newest-release rules admit; an upstream without a `tags` pattern SHALL never make a plan take one.

#### Scenario: Upstream without releases moved on
- **WHEN** the only upstream that moved on since the merge request was opened has no `tags` pattern
- **THEN** `sync` replaces nothing

### Requirement: Carrying out a replacement
To replace the merge request, `sync` SHALL merge the planned update locally without running the install command or the checks, and once every merge concluded and "Merge settings before a replacement" allows it, close the merge request as "Description of a replaced merge request" requires, delete its branch on each push URL while it is at the head read, and publish the new update as `finish` does in merge-request mode from `git-intake/update`, opening a new merge request.

#### Scenario: Merge request of an earlier release's branch replaced
- **WHEN** the replaced merge request comes from `git-intake/update-<commit>`
- **THEN** `sync` deletes that branch and opens the new merge request from `git-intake/update`

### Requirement: Merge settings before a replacement
Once the merges of a replacement concluded, `sync` SHALL read the forge's merge settings and refuse as `finish` does before it closes anything.

#### Scenario: Merge settings that refuse the publication
- **WHEN** the project's merge settings let a tree its required pipeline did not test be merged, and a replacement is due
- **THEN** `sync` refuses as `finish` does, closes no merge request, and pushes nothing

### Requirement: Replacement that conflicts
When a merge of a replacement stops on conflicts, `sync` SHALL replace nothing: it SHALL name the upstreams with their new targets and the conflicted files, discard the update it started, push nothing for the replacement, close no merge request, say that the merge request is kept and that merging it or withdrawing it with `abort --merge-request` lets the next update take the newer release, and then act on the merge request as without `replace_after`.

#### Scenario: Newer release that conflicts
- **WHEN** the newer release of an untouched merge request's upstream conflicts with the integration tip
- **THEN** `sync` names the release and the conflicted files, the merge request stays open with its head, no update remains in the clone, and `sync` exits with status 1

#### Scenario: Conflict while the integration branch moved
- **WHEN** a replacement conflicts and the integration branch moved past the merge request's head
- **THEN** `sync` catches the kept merge request up as "Catching up an open merge request" describes

### Requirement: Merge request state before a replacement
Just before it closes the merge request it replaces, `sync` SHALL read again its activity, then in one listing its branch and the integration branch, and its state, head and the traces the forge's answer shows last, and close nothing and push nothing unless all are as read before. A merge request merged, closed or being merged meanwhile, or no longer untouched, SHALL end the run with status 0 and a message saying so; any other change SHALL end it with status 2 and the advice to run again.

#### Scenario: Assigned just before the last read
- **WHEN** a person assigns the merge request, or sets it to merge automatically, after `sync` read its activity and before it reads its state
- **THEN** `sync` closes nothing, says that the merge request is no longer untouched, and exits with status 0

#### Scenario: Merged just before the last read
- **WHEN** a merge queue merges the merge request after `sync` read its activity and before it reads its state
- **THEN** that read finds it merged, `sync` closes nothing and changes no description, and exits with status 0

#### Scenario: Comment during the replacement
- **WHEN** a person comments on the merge request while `sync` merges its replacement
- **THEN** `sync` closes nothing, pushes nothing, says that the merge request is no longer untouched, removes the update it started, and exits with status 0

### Requirement: Pause before the close
The listing of the publish remote just before the close SHALL include the record of a pause, and a pause recorded meanwhile SHALL end the run as a merge request no longer untouched does.

#### Scenario: Pause during the replacement
- **WHEN** `land` records a pause of the merge queue while `sync` merges a replacement
- **THEN** `sync` closes nothing, pushes nothing, says that a pause was recorded, and exits with status 0

### Requirement: Answer to the close of a replaced merge request
`sync` SHALL read the state the forge answers to the request that closes the merge request. When the forge reports it merged or being merged, `sync` SHALL restore its description, push nothing and exit with status 0, saying so; when it reports it still open, `sync` SHALL restore its description, push nothing and exit with status 2 with the advice to run again.

#### Scenario: Merged just before the close
- **WHEN** a merge queue merges the merge request after `sync` read its state and before the forge answers the close
- **THEN** the merge request stays merged with its description as it was, nothing is pushed, and `sync` exits with status 0

### Requirement: Restoring the description after a close
`sync` SHALL make the request that restores the description of a merge request the forge did not close again in place, as the request that opens a merge request after the push. When that fails, `sync` SHALL exit with status 2, naming the merge request, saying that its description could not be restored, and asking to put it back from its history.

#### Scenario: Description that cannot be restored
- **WHEN** the forge merges the merge request as `sync` closes it and fails every request that would restore its description
- **THEN** `sync` exits with status 2, names the merge request, says that its description could not be restored, and does not run again

### Requirement: Description kept by a close
When the forge closed the merge request but answers with a description that records targets, `sync` SHALL send the note of "Description of a replaced merge request" in a second request, made again in place as the request that opens a merge request after the push, before it goes on. When that fails, `sync` SHALL exit with status 2, naming the closed merge request and saying that its description still records its targets.

#### Scenario: Close that kept the description
- **WHEN** the forge closes the merge request but answers with its old description
- **THEN** `sync` replaces the description in a second request, so that the closed merge request declines nothing, and goes on with the replacement

#### Scenario: Note that cannot be written
- **WHEN** the forge closes the merge request with its old description and fails every request that would replace it
- **THEN** `sync` exits with status 2, names the closed merge request, says that its description still records its targets, and does not run again

### Requirement: Description of a replaced merge request
`sync` SHALL close a replaced merge request and replace its description in one request with a note that says the tool replaced it with an update at the newer releases, that nothing of it was merged, and records no targets. The new merge request's description SHALL name the merge request it replaces and the releases that caused the replacement.

#### Scenario: Descriptions after a replacement
- **WHEN** `sync` replaced `!5` with `!6`
- **THEN** the description of `!5` says it was replaced and has no hidden block of targets, and the description of `!6` names `!5`

### Requirement: Interrupted replacement
A replacement interrupted after the old merge request was closed SHALL be completed by the next `sync` as an update without an open merge request: a left branch holding the closed merge request's head is deleted, a branch holding the new update gets its merge request, and otherwise the next `sync` plans the update again. A failure that may pass after the close SHALL be taken up the same way by the next attempt of the same run.

#### Scenario: Killed after the close
- **WHEN** `sync` closed the replaced merge request and was killed before it deleted the branch
- **THEN** the next `sync` deletes the branch, opens a merge request for the newer release whose description names the replaced one, and declines nothing

#### Scenario: A failure after the close
- **WHEN** `sync` closed the replaced merge request and the next request to the forge fails once with a 5xx
- **THEN** the run's next attempt opens a merge request for the newer release, declines nothing, and `sync` exits with status 0

### Requirement: Description after an interrupted replacement
The merge request that a later attempt or run opens after an interrupted replacement SHALL name the merge request closed most recently, when its description is the note of a replaced merge request, and the releases that merge request's head lacks.

#### Scenario: Named after an interruption
- **WHEN** a run closed `!5` for a replacement and failed before it opened the new merge request
- **THEN** the description of the merge request the next attempt opens names `!5` and the newer release

### Requirement: A replaced merge request during a landing
`land` SHALL never replace the merge request it lands. When `land` finds the merge request it lands closed by a replacement, it SHALL merge nothing, end a pause it holds, say that `sync` replaced the merge request and that `land` lands the new one, and exit with status 0.

#### Scenario: Replaced while the pipeline runs
- **WHEN** `land`, in a pause of the merge queue, waits for the pipeline of a merge request that a replacement closes, which happens only when the pause was recorded after the replacement's last reads, or when a run of another release replaced it
- **THEN** `land` merges nothing, says that the merge request was replaced, ends the pause, and exits with status 0

### Requirement: Rule in the sync record
When the tool chose a follow target by its own rules, without a revision named, the merge commit SHALL name the rule in the trailer `Intake-Chosen`: `newest-release` for the newest release of a `tags` pattern, `conflict-limit` for the tag `max_conflicts` selected, and `branch-tip` for the fetched tip of an upstream without a `tags` pattern. A merge of a revision a person named SHALL carry no `Intake-Chosen`, whatever the revision names.

#### Scenario: Rule in the sync record
- **WHEN** `sync` merges the newest release `v1.2.0`, and `start upstream=v1.2.0` merges the same release in another update
- **THEN** the first merge commit carries `Intake-Chosen: newest-release`, and the second carries `Intake-Tag: v1.2.0` and no `Intake-Chosen`

### Requirement: Reason a kept merge request gives
With `replace_after` set, `sync` SHALL say why it keeps the update's open merge request only when the plan takes a newer release. When only upstreams without a `tags` pattern moved on, it SHALL say instead that only a newer release of an upstream with `tags` replaces the merge request.

#### Scenario: Nothing moved with the setting
- **WHEN** `replace_after` is set, the update's merge request is open, and no upstream moved on since it was opened
- **THEN** `sync` says nothing of a replacement and reads no activity of the merge request

#### Scenario: Only an upstream without releases moved
- **WHEN** `replace_after` is set and the only upstream that moved on since the merge request was opened has no `tags` pattern
- **THEN** `sync` names the upstream as moved on and says that only a newer release of an upstream with `tags` replaces the merge request

### Requirement: Release versioning
`[release].versioning` SHALL be `dated` or `semver`, and `dated` when it is not set. Under `dated`, releases SHALL take the dated versions of "Release tag names" and "Next release"; under `semver`, the semantic versions of "Next semver release", computed from the changes since the previous release. The tool SHALL refuse any other value, naming `release.versioning` with the configuration's other problems.

#### Scenario: Configuration without versioning
- **WHEN** `[release]` sets no `versioning`
- **THEN** releases take dated versions, as before the setting existed

#### Scenario: Unknown versioning
- **WHEN** `[release]` sets `versioning = "calendar"`
- **THEN** every command refuses the configuration and names `release.versioning`

### Requirement: Upstream release of a semver commit
Under `semver`, a commit the release commands version SHALL contain an upstream release, and the tool SHALL refuse one that contains none, as "Release base" says for the release base.

#### Scenario: Semver tip without an upstream release
- **WHEN** under `semver` the publish remote's integration tip contains no upstream release
- **THEN** `release next` exits with status 2 and names the upstream and the pattern

### Requirement: Semver release tags
Under `semver`, a release tag SHALL be named `v<x.y.z>`, a version without leading zeros, prerelease part or build metadata, and its version SHALL be the name without the `v`. A tag of the dated form of "Release tag names" whose label is one of `labels` SHALL stay a release tag, so that the releases cut before the change count as releases.

#### Scenario: Prerelease tag
- **WHEN** under `semver` the publish remote has the tag `v1.2.0-rc.1` on a commit of the downstream's line
- **THEN** that tag is no release

#### Scenario: Dated releases before the change
- **WHEN** `[release]` sets `versioning = "semver"` and `labels = ["sakiori"]`, and the downstream's line carries `v0.11.1-sakiori.20261103.1`
- **THEN** that tag is a release, and it takes part in the order of releases

### Requirement: Commits of a semver release
Under `semver`, the previous release of a commit SHALL be the newest release on the downstream's line below the commit: the highest release of the nearest earlier commit of the line that has one. The commits of the commit's next release SHALL be those the commit has and the previous release lacks, merges excluded, including the upstream commits that updates brought in; without a previous release, every commit the commit has, merges excluded.

#### Scenario: Update with upstream fixes
- **WHEN** since the previous release only an update landed, and it brought in the upstream commits `fix: a` and `docs: b`
- **THEN** the commits of the next release are those two upstream commits, and the update's merge is not one of them

### Requirement: Change of a commit
Under `semver`, the change of a commit whose message is a Conventional Commit SHALL be major for a breaking change, marked by `!` before the colon of the subject or by a `BREAKING CHANGE` or `BREAKING-CHANGE` footer, whatever the type; minor for the type `feat`; patch for the types `fix`, `perf` and `revert`; and no change for any other type. Types SHALL compare without regard to letter case.

#### Scenario: Chores only
- **WHEN** every commit since the previous release has the type `docs`, `test`, `ci` or `chore`, and none is breaking
- **THEN** the commits make no change

#### Scenario: Breaking footer
- **WHEN** a commit `refactor: rename the service` has the footer `BREAKING CHANGE: clients use the new name`
- **THEN** its change is major

### Requirement: Change of another commit
Under `semver`, the change of a commit whose subject is not a Conventional Commit SHALL be patch.

#### Scenario: Subject of git revert
- **WHEN** a commit since the previous release has the subject `Revert "feat: add the widget"`
- **THEN** its change is patch

#### Scenario: Plain upstream subject
- **WHEN** an update brought in an upstream commit with the subject `Add the widget`
- **THEN** its change is patch

### Requirement: Change of an upstream release
Under `semver`, the next release's change SHALL also be major when the commit's upstream release does not satisfy `^<v>`, `<v>` being the previous release's upstream release, both by "Upstream release of a semver release": it is lower than `<v>`, or has another major version, at major version 0 another minor version, at 0.0 another patch version. Without a previous release, or one without an upstream release, the upstream release SHALL change nothing.

#### Scenario: Upstream minor release at major version 0
- **WHEN** the previous release contains upstream `v0.11.0`, and since then only an update to upstream `v0.12.0` landed, whose upstream commits are all `fix` commits
- **THEN** the next release raises the major version

#### Scenario: Upstream patch release
- **WHEN** the previous release contains upstream `v0.11.0`, and since then only an update to upstream `v0.11.3` landed, whose upstream commits are all `fix` commits
- **THEN** the next release raises the patch version

#### Scenario: Upstream release withdrawn
- **WHEN** the previous release records upstream `v0.11.3`, the upstream deletes that tag, and the tip's newest upstream release is `v0.11.0`
- **THEN** the next release raises the major version

#### Scenario: Chores and an incompatible upstream release
- **WHEN** since the previous release, which records upstream `v0.11.0`, only an update to upstream `v0.12.0` landed, whose commits are all `docs` commits
- **THEN** the next release raises the major version

### Requirement: Upstream release of a semver release
Under `semver`, the upstream release of a semver release SHALL be the one its tag's annotation records in the trailers `Intake-Upstream`, `Intake-Target` and `Intake-Tag`, when they name the release upstream and a tag that the release `tags` pattern selects as an upstream release. For a dated release or a tag without all three, it SHALL be the newest upstream release its commit contains, and so SHALL the upstream release of a commit without a release.

#### Scenario: Upstream tag after the release
- **WHEN** a release `v1.0.1` records upstream `v0.11.0`, and the upstream later tags `v0.12.0` on a commit that `v1.0.1` contains
- **THEN** the upstream release of `v1.0.1` stays `v0.11.0`, `release check` and `release version` of `v1.0.1` pass as before, and the next release raises the major version

### Requirement: Recorded upstream release of a tag
When `release check`, `release version` or `release notes` act on a semver release tag whose annotation records an upstream release, it SHALL be a tag of the release upstream that the release `tags` pattern selects, and the tagged commit SHALL contain its recorded commit, which they read locally. `release check` SHALL report a record that does not hold as a problem, and `release version` and `release notes` SHALL refuse with exit status 2.

#### Scenario: Recorded release the commit lacks
- **WHEN** the annotation of `v1.0.0` records upstream `v0.12.0` at a commit that the tagged commit does not contain
- **THEN** `release check` exits with status 1 and names `v0.12.0` and the commit

#### Scenario: Upstream release withdrawn after the release
- **WHEN** the release `v1.0.0` records upstream `v0.11.3`, and the upstream deletes that tag
- **THEN** `release check`, `release version` and `release notes` of `v1.0.0` give what they gave before

### Requirement: Moved upstream release tag
`release check` SHALL also report as a problem a recorded upstream release whose tag the upstream still lists at another commit than the recorded one. A tag the upstream no longer lists SHALL be no problem.

#### Scenario: Upstream tag moved after the release
- **WHEN** the release `v1.0.0` records upstream `v0.11.3`, and the upstream moves that tag to another commit
- **THEN** `release check` exits with status 1 and names both commits, and `release version` and `release notes` give what they gave before

### Requirement: Network reads of release version
Under `semver`, `release version` SHALL read the publish remote's integration branch and tags, and list the release upstream's tags. It SHALL take the last synced commits from the sync records of the commit's history, leaving out the records below the synced commits they give and those whose merge's configuration names the upstream at another URL or branch, and SHALL fetch an upstream only when that history has no such record of it.

#### Scenario: Build of a merge request
- **WHEN** under `semver` a merge request pipeline runs `release version` where the upstream's branch cannot be fetched, and the history has a sync record of every upstream
- **THEN** it prints the rehearsal version

#### Scenario: Fork of a fork
- **WHEN** a downstream was cloned from a fork that follows its own upstream under the same name, and has no sync record of its own
- **THEN** `release version` fetches the upstream, and takes none of the fork's sync records for its own

### Requirement: Next semver release
Under `semver`, a commit's next change SHALL be the largest change of its commits and its upstream release; with none, it SHALL take no release. After a semver release `v<x>.<y>.<z>`, the next release SHALL be `v<x+1>.0.0` for a major change, `v<x>.<y+1>.0` for a minor one and `v<x>.<y>.<z+1>` for a patch. The tool SHALL refuse a next release that breaks "Release order", naming the release it does not come after, or whose name the remote already has.

#### Scenario: Feature since a release
- **WHEN** the previous release is `v1.2.3` and since then `fix: a` and `feat: b` landed
- **THEN** the next release is `v1.3.0`, and its change is minor

### Requirement: First semver release
Under `semver`, when there is no previous release or the previous release is of the dated form, a commit that takes a release SHALL take the first semver release, with a major change: `v1.0.0`, or, when a release on the downstream's line below the commit has a version of 1.0.0 or above by precedence, `v<x+1>.0.0`, `<x>` being the major version of the highest of them.

#### Scenario: First release after dated releases
- **WHEN** `[release]` changes to `versioning = "semver"` with `labels = ["sakiori"]`, the previous release is `v0.11.1-sakiori.20261103.1`, and since then `fix: a` landed
- **THEN** the next release is `v1.0.0`, and its change is major

#### Scenario: Dated releases at 1.0.0 or above
- **WHEN** the previous release is `v1.4.1-custom.20261103.1` and the configuration changes to `versioning = "semver"`
- **THEN** the first semver release is `v2.0.0`

#### Scenario: First release of a line
- **WHEN** under `semver` the downstream's line carries no release and its commits make a change
- **THEN** the next release is `v1.0.0`

### Requirement: Nothing to release
Under `semver`, when the integration tip carries no release and takes none, `release tag` SHALL create nothing, say that the commits since the previous release ask for no release, naming that release, and exit with status 0, warning as "Unpublished release below the tip" says when the previous release is not published. `release next` SHALL report that no release is due.

#### Scenario: Documentation only
- **WHEN** under `semver` only `docs:` and `ci:` commits landed since the previous release
- **THEN** `release tag` pushes nothing and exits with status 0, and `release next --json` carries `bump` null and `next` null

### Requirement: Upstream release in the notes
Under `semver`, the notes SHALL start with a line that names the release upstream and the tag's upstream release by "Upstream release of a semver release", and says how many commits of that upstream the tagged commit carries after it: those the upstream's last synced commit at the tagged commit has and the upstream release lacks, as `status` reports the last synced commit. Without a last synced commit of that upstream, the count SHALL be zero.

#### Scenario: Branch past an upstream release
- **WHEN** a `semver` release contains upstream `v0.11.0` of the upstream `fff` and twelve commits of `fff` after it
- **THEN** the first line of its notes names `fff`, `v0.11.0` and twelve upstream commits after it

### Requirement: Versioning in the release state
`release next --json` SHALL also carry `versioning`, `dated` or `semver`, and `bump`: under `semver` `major`, `minor` or `patch`, the change that sets the next release, and null under `dated`, with a release at the tip, or when no release is due. Under `semver`, `base` SHALL be null, and `next` SHALL be null when no release is due.

#### Scenario: Semver tip
- **WHEN** under `semver` the integration tip carries no release and a `fix:` commit landed since the previous release `v1.2.3`
- **THEN** `versioning` is `semver`, `base` is null, `bump` is `patch` and `next` is `v1.2.4`

### Requirement: Upstream release in a semver tag
Under `semver`, the annotation of the tag `release tag` creates SHALL end with the trailers `Intake-Upstream: <upstream>`, `Intake-Target: <commit>` and `Intake-Tag: <tag>` of the newest upstream release the tip contains, after the message.

#### Scenario: Semver release
- **WHEN** under `semver` the previous release is `v1.2.3` and a `feat:` commit landed since
- **THEN** `release tag` pushes the annotated tag `v1.3.0` with the message `Release 1.3.0` and the trailers of the tip's upstream release

### Requirement: Checking a semver release tag
Under `semver`, `release check` SHALL compute again the version the tagged commit takes, from the releases below it and the upstream releases that they and the tag record.

#### Scenario: Semver pipeline run again after an upstream release
- **WHEN** under `semver` the upstream tagged a release on a commit that the release already contained, and the release's pipeline runs again
- **THEN** `release check` passes, and `release version` prints the same version

### Requirement: Problems of a semver release tag
Under `semver`, the problems of a release tag SHALL also be: it is not of the form `v<x.y.z>`; the upstream release its annotation records does not hold by "Recorded upstream release of a tag"; its commit takes no release; its version is not the version of its commit's next release by "Next semver release".

#### Scenario: Semver tag with the wrong version
- **WHEN** under `semver` the tag `v2.0.0` points at a commit whose next release is `v1.3.0`
- **THEN** `release check` exits with status 1 and names both versions

### Requirement: Upstream of a semver build
Under `semver`, `release version --json` SHALL carry `upstream`, and `--dotenv` the line `RELEASE_UPSTREAM=<upstream>`. `<upstream>` SHALL be the release upstream's name, with every character but ASCII letters, digits and `-` written `-`, and the version of the commit's upstream release by "Upstream release of a semver release", joined by `-`, so that it is valid build metadata.

#### Scenario: Upstream of a semver release
- **WHEN** under `semver` the pipeline of `v1.2.0`, whose commit contains upstream `v0.11.0` of the upstream `fff`, runs `release version --dotenv`
- **THEN** it writes `RELEASE_VERSION=1.2.0`, `RELEASE_TAG=v1.2.0` and `RELEASE_UPSTREAM=fff-0.11.0`

#### Scenario: Upstream name outside build metadata
- **WHEN** under `semver` the release upstream is named `app_core` and the commit's upstream release is `v0.4.0`
- **THEN** `RELEASE_UPSTREAM` is `app-core-0.4.0`

### Requirement: Breaking changes in the notes
Under `semver`, the notes SHALL list the breaking changes first: a line naming the previous and the new upstream release when "Change of an upstream release" made the change major, then the breaking changes among the listed commits, newest first, each with the text of its `BREAKING CHANGE` footer when it has one. The other entries SHALL come after them.

#### Scenario: Breaking change in a semver release
- **WHEN** under `semver` since the previous release `fix: a` and then `feat!: speak protocol 3` with the footer `BREAKING CHANGE: clients need protocol 3` landed
- **THEN** the notes list `feat!: speak protocol 3` with the text `clients need protocol 3` first, and `fix: a` after it

#### Scenario: Incompatible upstream release in the notes
- **WHEN** under `semver` the previous release records upstream `v0.11.0`, and the release records `v0.12.0`
- **THEN** the breaking changes start with a line saying that the upstream moved from `v0.11.0` to `v0.12.0`

### Requirement: Change of a semver build
Under `semver`, `release version --json` SHALL also carry `previous`, the tag of the previous release of the versioned commit or null when it has none, and `bump`, the change that sets that commit's next release by "Next semver release", `"major"`, `"minor"` or `"patch"`, or null when the commit takes no release. The versioned commit SHALL be the tagged commit for a release tag and the checked-out commit for a rehearsal.

#### Scenario: Change of a semver rehearsal
- **WHEN** under `semver` the previous release of the checked-out commit is `v1.2.3`, `feat: b` landed since, and a pipeline without a tag created on 2026-11-01 runs `release version --json`
- **THEN** the object has `version` `1.3.0-rehearsal.20261101`, `previous` `v1.2.3` and `bump` `"minor"`

#### Scenario: Rehearsal without a release due
- **WHEN** under `semver` only `docs: a` landed since the previous release `v1.2.3`, and a pipeline without a tag created on 2026-11-01 runs `release version --json`
- **THEN** the object has `version` `1.2.4-rehearsal.20261101`, `previous` `v1.2.3` and `bump` null

#### Scenario: Change of an incompatible upstream release
- **WHEN** under `semver` the previous release `v1.0.0` records upstream `v0.11.0`, and an update since then took upstream `v0.12.0` with only `docs:` commits
- **THEN** the object has `previous` `v1.0.0` and `bump` `"major"`

#### Scenario: Change of a semver release tag
- **WHEN** under `semver` the pipeline of `v2.0.0`, whose previous release is `v1.4.1`, runs `release version --json`
- **THEN** the object has `previous` `v1.4.1` and `bump` `"major"`

#### Scenario: Change of a line's first release
- **WHEN** under `semver` the checked-out commit has no release below it on the downstream's line and `fix: a` landed
- **THEN** `previous` is null and `bump` is `"major"`, the change of "First semver release"

#### Scenario: Change after dated releases
- **WHEN** `[release]` changes to `versioning = "semver"`, the previous release of the checked-out commit is `v0.11.1-custom.20261101.1`, and since then `fix: a` landed
- **THEN** `previous` is `v0.11.1-custom.20261101.1` and `bump` is `"major"`

#### Scenario: Dated object
- **WHEN** under `dated` a pipeline runs `release version --json`
- **THEN** the object has neither `previous` nor `bump`
