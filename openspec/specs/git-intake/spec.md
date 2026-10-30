# git-intake Specification

## Purpose

Keep a repository current with one or more upstreams, each fetched by its URL only while it is needed: merged up to a fixed target at the repository root or under its own path, or taken commit by commit through a reviewed plan with a journal of decisions; with isolated updates, recorded verification, guarded publication and per-upstream status.

## Requirements

### Requirement: Fork configuration
The tool SHALL read a repository's settings from a `.git-intake.toml` file committed on the integration branch: the integration branch, an optional publication setting with the publish remote and the mode `push` or `merge-request`, an install command, an ordered list of check commands, and an ordered list of upstreams. Each upstream SHALL have a unique name, a URL, a branch, a mode `follow` or `port` with `follow` as the default, optionally a path inside the repository, and, in `follow` mode, optionally a `tags` pattern: a glob over the upstream's tag names that selects its release tags. No two upstreams SHALL have the same URL and branch. An upstream without a path is taken at the repository root. The tool SHALL read the committed file from the integration branch tip, not from a working tree, and SHALL refuse to run when the file is missing or invalid, naming the problem. It SHALL refuse the settings `remote` and `mirror`, naming what replaces them, an upstream in `port` mode with a path or a `tags` pattern, a `tags` pattern that is not a valid glob or that starts with `refs/` or `tags/`, and a publish remote whose fetch or push URL names a configured upstream's repository.

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
Every merge commit the tool creates SHALL name the upstream and its target commit in commit trailers, and, when a tag of the upstream's repository points at the target, that tag in the trailer `Intake-Tag`: the tag a revision named, the release a `tags` pattern selected, or otherwise, of the tags at the target, the one with the highest version among those matching the upstream's `tags` pattern whose name is a version after the pattern's literal prefix, or, without such a tag, among those whose name is a semantic version with an optional `v` or `V`; a tag whose name is no version SHALL NOT be recorded this way, even when it matches the pattern. A tag whose name contains a comma SHALL NOT be recorded. The tag SHALL be fixed when `start` fixes the target. Every commit the tool copies from an upstream in `port` mode SHALL name the upstream and the upstream commit it took in the trailers `Intake-Upstream` and `Intake-Taken`. For an upstream in `follow` mode, the tool SHALL derive the last synced commit from its sync records on the integration line, as the target of the newest record whose target no other of its records' targets contains, and SHALL NOT store it in the configuration or outside the history. For such an upstream that carries no sync record, the last synced commit SHALL come from the history, and the tool SHALL show that it does: for a root upstream it is the newest upstream commit the integration branch contains; for a path upstream it is the second parent of the newest merge on the integration line whose second parent belongs to the upstream's branch and which changes the upstream's path, as an import by `git subtree add` or `git subtree pull` leaves. This decision SHALL depend only on the integration branch and the upstream's own branch, never on other upstreams. When a `follow` upstream's last synced commit is no longer contained in its branch, the tool SHALL report that upstream's history as rewritten and SHALL refuse to update it.

#### Scenario: Last synced commit
- **WHEN** the integration line contains a merge of a `follow` upstream
- **THEN** `status` shows that upstream's target from the most recent such merge as its last synced commit

#### Scenario: Older record merged later
- **WHEN** a branch that carries a sync record of an older target is merged into the integration branch after a newer target's record landed
- **THEN** `status` still shows the newer target as the last synced commit, and nothing between the two targets counts as pending

#### Scenario: Upstream force-pushed
- **WHEN** a `follow` upstream's branch no longer contains the last synced commit
- **THEN** `status` reports the upstream as rewritten and `start` refuses to update it

#### Scenario: Target named by a tag
- **WHEN** an update merges a target that a tag of the upstream's repository points at, chosen by a release pattern, by naming the tag, or by naming the commit
- **THEN** the merge commit carries `Intake-Tag` with that tag, `report` names it, and `status` shows it as the tag of the last synced commit

#### Scenario: Target without a tag
- **WHEN** an update merges a target that no tag of the upstream's repository points at
- **THEN** the merge commit carries no `Intake-Tag` trailer

#### Scenario: Copied commit
- **WHEN** a port update copies an upstream commit
- **THEN** the copy carries the upstream's name and the upstream commit's id in its trailers

### Requirement: Recorded resolutions
The tool SHALL enable git's recorded conflict resolution for the repository before merging or copying commits, so a resolution made in one update or attempt is reapplied when the same conflict recurs.

#### Scenario: Retried update
- **WHEN** an aborted update is started again and hits a conflict resolved in the earlier attempt
- **THEN** git reapplies the recorded resolution and the tool reports the file as resolved by a recorded resolution

### Requirement: Resumable state
The tool SHALL keep the state of an update in the repository's common git directory, visible from every worktree, and the update worktree at `.worktrees/git-intake/update` in the main worktree, never inside the git directory. Unless that path is ignored already, `start` SHALL add the line `/.worktrees/` to the repository's `info/exclude` once, without changing a committed ignore file, so that the main checkout does not show the update worktree. Every command SHALL act on that state: `continue` carries out a plan, concludes a resolved merge, copy or catch-up merge, and goes on with the remaining queued upstreams or planned commits; `status` shows the step reached, each queued upstream as merged, at its plan, in conflict or pending, a catch-up merge in conflict or awaiting verification, and what comes next; and `abort` removes the worktree, the update branch, the plan, the update's upstream references, its logs and the state without touching the integration branch. A `follow` upstream SHALL count as merged only when the history of the update branch head carries its sync record for its target, and a `port` upstream as carried out only when its plan has been carried out to the journal commit. Once `finish` has begun moving branches, `abort` SHALL refuse and name `finish` as the way to complete the update. While an update waits for its merge request, `abort` SHALL delete the merge request branch from the publish remote, remove the update, and name the merge request that is left to close. The tool SHALL read an update state of format 4 as its own, keeping a waiting update's merge request branch; the first command that changes the update SHALL write it in format 5 and say that releases before v0.3.6 can no longer carry it on, and the first of `continue`, `verify` and `finish` SHALL warn once that an earlier release started the update, as for another release of the same format. It SHALL refuse an update state in any other format than its own and format 4, older or newer, or without a format, name both formats and, where the state records it, the release that started the update, and advise finishing or aborting the update with a release that keeps that format, naming the releases of each format (1: v0.1.0; 2: v0.2.0 to v0.2.2; 3: v0.3.0 to v0.3.1; 4: v0.3.2 to v0.3.5; 5: from v0.3.6), or for a newer format with the release that started it or a later one that reads it. For format 3, the refusal SHALL name `migrate` first. Only `start` SHALL record the release that started the update in the state. The first of `continue`, `verify` and `finish` run by another release of the same format SHALL warn once, without refusing, and record that it warned; it SHALL advise aborting and starting again only before `finish` has moved a branch or opened the merge request.

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
- **THEN** `abort` and `start` refuse and `doctor` fails, each naming that worktree and saying to remove or switch it, and the worktree and its uncommitted changes are kept

#### Scenario: State of the previous release
- **WHEN** an update started by a release with state format 4 is in progress, at any step
- **THEN** every command carries it on, and the first command that changes the update writes the state in format 5 and says that earlier releases can no longer carry it on

#### Scenario: State that needs migration
- **WHEN** an update started by a release with state format 3, which kept the update worktree in the git directory, is in progress
- **THEN** every command that reads the update, other than `migrate`, refuses and names `migrate`, and also finishing or aborting the update with a release that keeps format 3

#### Scenario: State of an older release
- **WHEN** an update started by a release with a state format older than 3 is in progress
- **THEN** every command that reads the update refuses and says to finish or abort it with a release that keeps that format

#### Scenario: State of a newer release
- **WHEN** an update started by a release with a newer state format is in progress
- **THEN** every command that reads the update refuses, names that release when the state records it, says that this release is older, and changes nothing

#### Scenario: Another release of the same format
- **WHEN** an update started by one release is carried on by another release with the same state format
- **THEN** the first of `continue`, `verify` and `finish` it runs warns once that another release started it, advises `abort` only before `finish` has moved a branch or opened the merge request, and every command carries on

### Requirement: Migration of an update in progress
`migrate` SHALL carry an update in progress over from state format 3 to the current format, in place, keeping its branch, commits, merge in progress, index, uncommitted and untracked files, plan, logs and references. An update worktree in the git directory SHALL move with `git worktree move` to `.worktrees/git-intake/update` in the main worktree, under the placement rules of `start`, before the state is written; an update worktree outside the git directory, or that of a bare repository, SHALL stay, and only the state is rewritten. An update worktree already at the destination SHALL be kept: when git still records it at the old path, `migrate` SHALL repair git's record with `git worktree repair` instead of refusing. `migrate` SHALL say that the processes and editors using the old path must be stopped, since the directory moves. A verification recorded before a move, or at a path containing `/.git` that a bare repository keeps, SHALL be dropped unless `finish` has begun. `migrate` SHALL refuse, before changing anything, when no update is in progress, when the state is in a format other than 3, when the main worktree cannot be located, when the destination exists, or when git refuses the move, giving git's reason.

#### Scenario: Update with a merge in progress
- **WHEN** an update started by a release with state format 3 stopped on conflicts, some resolved and staged and some not, and `migrate` runs
- **THEN** the update worktree is at `.worktrees/git-intake/update` with the same merge in progress, index, and staged, unstaged and untracked changes, and `status`, `continue`, `verify` and `finish` work on it

#### Scenario: Migration in a bare repository
- **WHEN** `migrate` runs in a bare repository
- **THEN** the update worktree stays in the git directory and the state changes only in its format, except that a verification run at a path containing `/.git` is dropped

#### Scenario: Verification in the old location
- **WHEN** the update was verified in the old location and `finish` has not begun
- **THEN** `migrate` drops the verification, and `finish` refuses until `verify` runs again

#### Scenario: Destination taken or move refused
- **WHEN** `.worktrees/git-intake/update` exists and is not the update worktree, or git refuses the move, as for a locked worktree
- **THEN** `migrate` refuses, giving the reason and never suggesting removal, and the state, the worktree and its files are unchanged

#### Scenario: Interrupted migration
- **WHEN** a `migrate` moved the worktree but did not write the state
- **THEN** running `migrate` again finds the worktree at its new place and writes the state

#### Scenario: Worktree moved by hand
- **WHEN** the update worktree was moved to `.worktrees/git-intake/update` without git, which still records the old path
- **THEN** `migrate` repairs git's record, keeps the worktree and its files, and writes the state

#### Scenario: Verification kept once finish has begun
- **WHEN** `finish` has planned its moves, or pushed the merge request branch, before `migrate` runs
- **THEN** the verification is kept

#### Scenario: State of format 4
- **WHEN** `migrate` runs on an update whose state has format 4
- **THEN** it refuses, says that this release carries that update on without migration, and changes nothing

### Requirement: Verification of the published tree
`verify` SHALL require the configuration at the update worktree head to be valid and to name the same integration branch, then run the configured install command and every check in the update worktree once every queued upstream is merged or its plan carried out, write each command's output to a log, report pass or fail per command, and record the worktree commit it verified. A failed or skipped check SHALL leave the update unverified.

#### Scenario: Checks pass
- **WHEN** every check succeeds
- **THEN** the tool records the verified commit and names `finish` as the next step

#### Scenario: Commit after verification
- **WHEN** a commit is added to the update branch after `verify`
- **THEN** `finish` refuses until `verify` passes for the new commit

#### Scenario: Upstreams still pending
- **WHEN** `verify` runs while a queued upstream is at its plan, in conflict or pending
- **THEN** the tool refuses and names the upstreams that still need `continue`

### Requirement: Environment of the checks
`verify` SHALL run the install command and every check with environment variables that describe the update, naming commits by their full ids: `GIT_INTAKE_BASE`, the update's base, which is the integration tip the update started from or, after a catch-up merge, the integration tip it caught up with; `GIT_INTAKE_HEAD`, the commit being verified; `GIT_INTAKE_INTEGRATION`, the integration branch's name; and `GIT_INTAKE_UPSTREAMS`, the names of the queued upstreams separated by spaces, in queue order. For each queued upstream, with `<NAME>` its name in upper case and every character other than a letter or a digit replaced by `_`, it SHALL set `GIT_INTAKE_UPSTREAM_<NAME>_MODE` to `follow` or `port` and `GIT_INTAKE_UPSTREAM_<NAME>_TARGET` to the follow target, or for a port upstream to the newest commit of its plan. For a follow upstream with a last synced commit before the update, it SHALL set `GIT_INTAKE_UPSTREAM_<NAME>_PREVIOUS` to that commit; a port upstream SHALL get no such variable. A catch-up merge SHALL change none of the upstream variables. The verification that `finish` runs after a catch-up SHALL set the same variables. Every commit named SHALL be reachable from a reference while the commands run. Variables with the `GIT_INTAKE_` prefix that the tool inherits SHALL NOT reach the commands, and the tool SHALL NOT set these variables for the git commands it runs itself.

#### Scenario: Follow upstream with a last synced commit
- **WHEN** an update merges a follow upstream that was synced before
- **THEN** the install command and every check receive the base, the head, the integration branch, the upstream's name, its mode, its target and its last synced commit, and each commit exists and is contained in a reference

#### Scenario: First import
- **WHEN** an update imports a path upstream for the first time
- **THEN** the commands receive the upstream's mode and target and no previous commit

#### Scenario: Several upstreams
- **WHEN** an update takes several upstreams, and leaves out another that is current
- **THEN** `GIT_INTAKE_UPSTREAMS` lists the upstreams taken in queue order, each with its own variables, and the current upstream has none

#### Scenario: Port upstream
- **WHEN** an update carries out a port plan
- **THEN** the commands receive the mode `port` and the newest commit of the plan as the target, and no previous commit

#### Scenario: After a catch-up
- **WHEN** the checks run on a catch-up merge
- **THEN** `GIT_INTAKE_BASE` is the integration tip the update caught up with, `GIT_INTAKE_HEAD` is the catch-up merge, and every upstream variable is as before the catch-up

#### Scenario: Inherited variables
- **WHEN** `verify` is run with `GIT_INTAKE_BASE` or another `GIT_INTAKE_` variable already set
- **THEN** the commands receive only the variables of the update, with the update's values

### Requirement: Guarded publication
`finish` SHALL require a verified update whose worktree commit is the verified one, or one whose head is a catch-up merge the tool made and has not verified, which it verifies itself; any other unverified head SHALL be refused until `verify` passes. In `push` mode, unless `--no-publish` is given, it SHALL first fetch the publish remote and refuse with git's reason when the fetch fails: when the remote's integration branch holds commits the local one lacks and the local one holds none the remote lacks, it SHALL bring the local integration branch up to date as `start` does, which does not count as `finish` moving it; when each holds commits the other lacks, it SHALL refuse before moving anything and say that the local commits must be published or reconciled first. When the integration tip contains the verified commit, `finish` SHALL go on without a catch-up. The integration tip SHALL be the local integration branch's in `push` mode and the publish remote's in `merge-request` mode. When the integration tip is otherwise no longer the base, `finish` SHALL catch up with it, verify the result as `verify` does, and go on only when the verification passes; it SHALL make at most one catch-up merge per run, and when the integration branch moves again meanwhile it SHALL stop and say to run `finish` again. Before moving any branch it SHALL confirm that the move of the integration branch is a fast-forward, that the verified commit contains the base and every merged target, and that no checkout of the integration branch would lose changes. It SHALL record the planned move, fast-forward the integration branch to the verified commit, push exactly the planned commit without force when the publication mode is `push`, and only after publication succeeds remove the worktree, the update branch, the update's upstream references and the state. An interrupted or failed `finish` SHALL complete the remaining move and the push when run again, and SHALL never publish a commit other than the planned one. When the push was rejected because the publish remote's integration branch moved past the base, a `finish` run again SHALL catch up with the remote's branch from the finishing step, verify the result, fast-forward the local integration branch to it and push it; when the local integration branch moved past the planned commit meanwhile, it SHALL make no catch-up and name `finish --no-publish`. `finish --no-publish` SHALL not fetch, SHALL catch up only with the local integration branch, SHALL in the finishing step drop any catch-up in progress and close the update at the planned commit, complete the local move and remove the worktree, the update branch, the upstream references and the state without pushing, and report that the integration branch remains unpublished. It SHALL never rewrite or force-push a branch, and SHALL never push to an upstream's URL.

#### Scenario: Integration branch moved
- **WHEN** a commit lands on the integration branch after `start` and `finish` runs on the verified update
- **THEN** `finish` catches up with the new tip, runs the install command and the checks on the catch-up merge, fast-forwards the integration branch to it, publishes it, and afterwards `status` shows every merged upstream's target as its last synced commit

#### Scenario: Catch-up that conflicts in finish
- **WHEN** the commit that landed conflicts with the update
- **THEN** `finish` stops with exit status 1 in the catch-up step and moves no branch to the update, and after the conflict is resolved and `continue` has run, `finish` verifies and publishes

#### Scenario: Checks fail after a catch-up
- **WHEN** the checks fail on the catch-up merge
- **THEN** `finish` stops with the report of `verify`, the update stays unverified at the catch-up merge, and no branch moves to the update

#### Scenario: Commit after a catch-up
- **WHEN** a commit is added on top of a catch-up merge before it is verified
- **THEN** `finish` refuses until `verify` passes for the new commit

#### Scenario: Integration branch moves during verification
- **WHEN** another commit lands on the integration branch while `finish` verifies a catch-up merge
- **THEN** `finish` moves no branch to the update and says to run it again, which catches up once more

#### Scenario: Integration branch already holds the update
- **WHEN** the integration branch was moved to the verified commit, or past it, by other means
- **THEN** `finish` makes no catch-up merge, counts the move as done, pushes the verified commit in push mode, and completes the update

#### Scenario: Remote moved
- **WHEN** the publish remote's integration branch is ahead of the local one, which holds no commit the remote lacks, and `finish` runs
- **THEN** `finish` fast-forwards the local integration branch to the remote's, catches up with it, verifies, and publishes the catch-up merge

#### Scenario: Diverged from the remote
- **WHEN** the publish remote's integration branch and the local one each hold commits the other lacks
- **THEN** `finish` refuses before moving any branch and says that the local commits must be published or reconciled first

#### Scenario: Publish remote unreachable
- **WHEN** the publish remote cannot be fetched
- **THEN** `finish` refuses with git's reason and moves no branch, and `finish --no-publish` closes the update locally without fetching

#### Scenario: Push rejected after the move
- **WHEN** the publish remote's integration branch moves after `finish` moved the local branch and before its push, so that the push is rejected
- **THEN** no remote branch moves, the update stays in its finishing step, and running `finish` again catches up with the remote's branch, verifies, fast-forwards the local integration branch and pushes the catch-up merge

#### Scenario: Local commit and rejected push
- **WHEN** a commit was added to the local integration branch after `finish` moved it, and the push was rejected because the remote moved
- **THEN** `finish` makes no catch-up merge, moves no branch, and names `finish --no-publish`

#### Scenario: Publication never allowed
- **WHEN** the publish remote refuses every push to the integration branch, as for a protected branch
- **THEN** `finish --no-publish` leaves the local integration branch advanced, removes the update, and names the unpublished branch

#### Scenario: Local commit while finishing
- **WHEN** a commit is added to the integration branch after `finish` moved it and before the push succeeded
- **THEN** running `finish` again pushes only the planned commit

#### Scenario: Interrupted finish
- **WHEN** `finish` stops after moving the integration branch and before the push
- **THEN** `status` shows the finishing step, `abort` refuses, and running `finish` again completes the push

#### Scenario: No publish remote
- **WHEN** no publish remote is configured
- **THEN** `finish` advances the local integration branch and pushes nothing

#### Scenario: Merge-request mode
- **WHEN** the publication mode is `merge-request`
- **THEN** `finish` publishes as the merge request publication requires and moves no local branch before the merge request is merged

### Requirement: Update report
`report` SHALL summarise an update in progress or just merged, per upstream. For a `follow` upstream it SHALL give the number of upstream commits merged, the upstream tag of the target when one points at it, the conflicted files and whether each was resolved by hand or by a recorded resolution, and how the repository's own changes relative to that upstream grew or shrank. For a `port` upstream it SHALL give the taken commits in the order they were copied, each with its conflicted files and their resolution source, the commits recorded as `have` because their copy changed nothing, the `skip` and `have` decisions with their notes, the number of commits left pending, and, for each taken commit, the earlier upstream commits still pending that changed any of the same files.

#### Scenario: Report after merge
- **WHEN** the user runs `report` after resolving conflicts in a `follow` update
- **THEN** the output lists, for each merged upstream, the merged range, each conflicted file with its resolution source, and the diff size against that upstream before and after

#### Scenario: Report of a port
- **WHEN** the user runs `report` after a plan that took a commit whose files an earlier pending upstream commit also changed
- **THEN** the output lists the taken commit with its conflicts and names the earlier pending commit beside it

### Requirement: Upstream status
`status` SHALL fetch every upstream and report, for each, its path, its branch, its mode and its fetched tip. For a `follow` upstream it SHALL report its last synced commit with the upstream tag its sync record names, the number of commits the integration branch lacks, and one state: current, behind, not imported, no release when it has a `tags` pattern and its fetched branch has no release tag, path removed when a synced path upstream's path no longer exists, rewritten, or unavailable when its URL cannot be fetched, its branch is missing, or the tags of an upstream with a `tags` pattern cannot be listed. For a `follow` upstream with a `tags` pattern, the newest release takes the place of the tip: `status` SHALL show it as the tip, count the commits the integration branch lacks up to it, predict the conflicts of merging it, and show the fetched branch tip and the number of commits it lies past the release separately. For a `port` upstream it SHALL report its journal's base, the numbers of taken, decided and pending commits, and one state: current, pending, no base, rewritten, or unavailable. `status --check` SHALL additionally list the files a merge of each behind `follow` upstream's tip would conflict in, create no worktree, branch or state, and exit with a failure status unless every upstream is current, so that it can run on a schedule and gate a cutover or a port review.

#### Scenario: Several upstreams
- **WHEN** `status` runs on a repository with three upstreams, two of them behind
- **THEN** it reports each upstream on its own line with its path and state, and the commit count for the two that are behind

#### Scenario: Scheduled check
- **WHEN** `status --check` runs on a repository whose single `follow` upstream is 30 commits ahead
- **THEN** it reports the 30 commits and the files that would conflict, exits with a failure status, and leaves the repository unchanged

#### Scenario: Zero drift
- **WHEN** `status --check` runs and every upstream is current
- **THEN** it reports every upstream as current and exits successfully

#### Scenario: Synced to a chosen target
- **WHEN** the last update of a `follow` upstream merged a chosen target that lies three commits before the fetched branch tip
- **THEN** `status` shows that target as the last synced commit, the tip, the state behind and three commits

#### Scenario: Development commits after a release
- **WHEN** a `follow` upstream with a `tags` pattern has merged its newest release, and its branch holds commits after it
- **THEN** `status` reports the upstream as current, shows the release as the tip and the branch tip with the number of commits past the release, and `status --check` succeeds

#### Scenario: Newer release
- **WHEN** a `follow` upstream with a `tags` pattern has a release newer than its last synced commit
- **THEN** `status` reports the upstream as behind by the commits up to that release, not up to the branch tip

#### Scenario: No release
- **WHEN** a `follow` upstream with a `tags` pattern has no release tag on its fetched branch
- **THEN** `status` reports the state no release and `status --check` exits with a failure status

#### Scenario: Port with pending commits
- **WHEN** `status --check` runs for a `port` upstream with twelve first-parent commits after its base, of which three were taken, two skipped and seven are undecided
- **THEN** it reports the base, three taken, two decided and seven pending, the state pending, and exits with a failure status

#### Scenario: Port without a base
- **WHEN** `status --check` runs for a `port` upstream with no journal, no sync record and no history shared with the integration branch
- **THEN** it reports the state no base with the advice to write the journal and its base by hand, and exits with a failure status

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
`doctor` SHALL check, without changing the repository other than through the references of its own fetches, that the configuration is valid, every upstream's URL can be fetched and its branch exists, every imported upstream's path exists on the integration branch, every `follow` upstream's last synced commit is still contained in its branch, every `port` upstream's journal is valid and its base and its decided and taken commits that are not retired are still on its branch, the main checkout is clean, no merge is in progress, the update state, if any, matches its worktree and branch, no git remote names a configured upstream's repository, no reference under `refs/git-intake/` is left without an update, and no journal belongs to an upstream that is missing or in `follow` mode. It SHALL report each check with its result and exit with a failure status when any check fails.

#### Scenario: Healthy repository
- **WHEN** every check passes
- **THEN** `doctor` lists the checks as passed and exits successfully

#### Scenario: Upstream removed from the configuration
- **WHEN** the integration branch carries sync records of an upstream that the configuration no longer lists
- **THEN** `doctor` reports that upstream by name as dropped from the configuration

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
`status`, `report`, `log`, `branches` and `doctor` SHALL accept `--json` and then write a single JSON document to standard output that carries the same facts as the text form, under field names that are stable across releases, with the same exit status. A rewritten `port` upstream SHALL carry the base and the commits that are off its line.

#### Scenario: Agent reads status
- **WHEN** an agent runs `status --json`
- **THEN** it receives, for every upstream, its name, path, branch, mode, fetched tip and state, for a `follow` upstream its last synced commit and commit count, and for a `port` upstream its base and its numbers of taken, decided and pending commits, without parsing text

#### Scenario: Agent reads a rewritten port upstream
- **WHEN** an agent runs `status --json` for a rewritten `port` upstream
- **THEN** it receives the state `rewritten` and each taken or decided commit off the upstream's line that is not retired, with what it was recorded as

### Requirement: Publication through a merge request
When the publication mode is `merge-request`, `finish` SHALL apply the same checks as in push mode, leave the local integration branch where it is until the merge, catching up with the publish remote's integration branch when it moved past the base, then push the verified commit without force to the branch `git-intake/update` on the publish remote, requesting through git push options a merge request from that branch into the integration branch, with squashing off whatever the project's default, whose title names the updated upstreams and whose description lists each `follow` upstream's merged range and each `port` upstream's taken commits and decisions. Before the first push of an update, the branch SHALL be absent where each push URL points or at a commit the base contains; otherwise `finish` SHALL push nothing, refuse, and name the branch and the push URL. It SHALL move no local branch, remove the update worktree, and keep the update in a step that waits for the merge request. When the remote does not accept push options, the tool SHALL push the branch without them and name the merge request to open, with squashing off. When the branch is already at the verified commit where pushes to the publish remote go, as after an interrupted `finish`, `finish` SHALL push nothing and count no merge request as requested; when the remote has several push URLs, it SHALL push only to those where the branch is not yet at the verified commit. Deleting the merge request branch SHALL likewise act on each push URL and report what remains on each. The tool SHALL keep every commit of the update it pushed to the merge request branch. A later `finish` SHALL fetch the publish remote and act on the first of these that holds. When the remote's integration branch received one of those commits itself by fast-forward, or by a merge whose tree equals that commit's tree and whose parents that commit contains, and the merge request's final head is that commit, it SHALL fast-forward the local integration branch to the remote one, delete the merge request branch if it remains, and remove the update. When the integration branch received one of those commits but the merge request's final head is another commit, it SHALL refuse because the head is not a verified commit. When the integration branch carries, without any of those commits, a commit whose tree is a pushed commit's tree, or the update's sync records or copy trailers, as after a squash or rebase merge, it SHALL refuse and explain that the merge request must be merged without squash or rebase and that a project whose own or target branch rule's squash setting requires squashing cannot take these merge requests. When the integration branch received a pushed commit through a merge whose tree differs from that commit's tree, it SHALL refuse. When the integration branch no longer contains the base, it SHALL refuse and say that it was rewritten. When the merge request branch disappeared, it SHALL refuse and name `abort`. When the merge request's head or branch no longer points at the verified commit, it SHALL say so and exit with the status of a negative result, because the update cannot complete until someone acts. When the integration branch moved past the base in any other way, it SHALL recreate the update worktree, catch up with the remote's integration branch, verify the result, check again just before pushing that the merge request's head and branch and the remote's integration branch are as fetched, and push the result without force to the same branch with push options that set the title and the description but ask for no new merge request; it SHALL keep the update waiting for the same merge request and exit successfully. A conflict or failed verification in that catch-up SHALL leave the update waiting for its merge request, in the catch-up step. Otherwise it SHALL report that the update waits for the merge request and what to do next, and exit successfully: waiting is the expected step, not a failure, and `status --check` reports the update's upstreams as not current until it completes. Without the forge's API the tool SHALL treat a merge request closed with its branch kept as open. `finish --no-publish` SHALL refuse while the merge request is open, because the verified commit is already published and there is no local move to make. It SHALL count a merge request as requested only when the push that carried the options to create it transferred the branch. When the remote's response names the merge request it opened, the tool SHALL record its number and judge the merge by the merge request's head as the remote keeps it (`refs/merge-requests/<number>/head`), which outlives the merge request branch: a merge request whose final head is not one of the update's pushed commits SHALL count as not having landed the verified tree, whichever merge method was used. Without that reference, the tool SHALL judge by the merge request branch as last seen and SHALL say, when it completes a fast-forward merge, that commits above the verified commit could not be attributed. `abort` while waiting, including in a catch-up step, SHALL refuse when the integration branch received one of the update's pushed commits as required and name `finish`; after a merge that did not land a verified tree it SHALL remove the update and say that the integration branch holds an unverified merge; otherwise it SHALL delete the merge request branch while it points at one of the update's pushed commits. An update that waits for a merge request opened by a release with state format 4 SHALL keep its merge request branch `git-intake/update-<short verified commit>`.

#### Scenario: Merge request opened
- **WHEN** `finish` runs on a verified update in merge-request mode against a remote that accepts push options
- **THEN** the branch `git-intake/update` with the verified commit and a merge request into the integration branch, with squashing off, exist on the publish remote, the local integration branch has not moved, `finish` exits successfully, and `status` shows the update waiting for the merge request

#### Scenario: Merge request merged
- **WHEN** the merge request is merged by fast-forward or with a merge commit and `finish` runs again
- **THEN** the local integration branch contains the verified commit, and no update or upstream reference of it remains

#### Scenario: Merge request still open
- **WHEN** `finish` runs while the merge request is not merged
- **THEN** the tool reports that the update waits for the merge request and what to do next, changes nothing, and exits successfully, while `status --check` exits with a failure status

#### Scenario: Merged by squash or rebase
- **WHEN** the integration branch on the publish remote carries the update's changes but not the verified commit
- **THEN** `finish` refuses, explains that the merge request must be merged without squash or rebase and that a project whose own or target branch rule's squash setting requires squashing cannot take these merge requests, and moves no branch

#### Scenario: Squash merge with the branch kept
- **WHEN** the merge request is merged by squash while its branch and head stay at the verified commit
- **THEN** `finish` refuses as for a squash merge, makes no catch-up merge, and pushes nothing

#### Scenario: Integration branch moved before the merge
- **WHEN** a commit lands on the integration branch of the publish remote before the merge request is merged, and `finish` runs
- **THEN** `finish` catches up with the remote's integration branch, verifies the catch-up merge, pushes it to `git-intake/update` without force, the merge request's head becomes the catch-up merge and its description is rewritten, no second merge request exists, the update keeps waiting, and `finish` exits successfully

#### Scenario: Catch-up conflict while waiting
- **WHEN** the catch-up with the remote's integration branch conflicts, or its checks fail
- **THEN** `finish` stops with exit status 1, pushes nothing, the merge request keeps its head, `status` shows the update waiting for the merge request in the catch-up step, and `abort` deletes the merge request branch and removes the update

#### Scenario: Merged during a catch-up
- **WHEN** the merge request is merged while `finish` catches up, before the push
- **THEN** `finish` pushes nothing and opens no second merge request, and because that merge could not land a verified tree on the moved integration branch, `finish` refuses and `abort` names the unverified merge

#### Scenario: Merged after catching up
- **WHEN** the merge request that carries a catch-up merge is merged by fast-forward or with a merge commit and `finish` runs again
- **THEN** the local integration branch contains the catch-up merge, and `status` shows every merged upstream's target as its last synced commit

#### Scenario: Branch left on the remote
- **WHEN** before the first push of an update, `git-intake/update` exists where a push URL points at a commit the base does not contain
- **THEN** `finish` pushes nothing, refuses, names the branch and the push URL, and the update stays verified

#### Scenario: Remote without push options
- **WHEN** the publish remote does not accept push options
- **THEN** the tool pushes the branch without them and names the source and target branches of the merge request to open

#### Scenario: Merge request branch changed before the merge
- **WHEN** a commit is added to the merge request branch and the merge request is merged with a merge commit
- **THEN** `finish` refuses because the merged tree is not the verified one, moves no branch, and `abort` removes the update and names the unverified merge

#### Scenario: Merged after the integration branch moved
- **WHEN** the merge request is merged with a merge commit after another commit landed on the integration branch
- **THEN** `finish` refuses because the merged tree is not the verified one and moves no branch

#### Scenario: Abort after a valid merge
- **WHEN** `abort` runs after the merge request was merged with the verified commit
- **THEN** it refuses and names `finish` to complete the update

#### Scenario: Interrupted opening of the merge request
- **WHEN** `finish` stops after pushing the merge request branch and is run again
- **THEN** the tool does not claim that a merge request was requested, and names the merge request to open if none exists

#### Scenario: No publication while waiting
- **WHEN** `finish --no-publish` runs while the update waits for its merge request
- **THEN** it refuses, moves no branch, keeps the update waiting, and says to merge the merge request first or to withdraw it with `abort`

#### Scenario: Merge request closed without merging
- **WHEN** the merge request branch disappears while the integration branch did not receive the verified commit
- **THEN** `finish` refuses and names `abort`

#### Scenario: Merge request closed with its branch kept
- **WHEN** the merge request is closed without merging and its branch is kept
- **THEN** `finish` treats it as open, and `abort` deletes the branch and names the merge request

#### Scenario: Fast-forward merge of a changed merge request
- **WHEN** a commit is added to the merge request branch, the merge request is merged by fast-forward and its branch is removed
- **THEN** `finish` reads the merge request's head from the remote, refuses because it is not the verified commit, and moves no branch

#### Scenario: Merge request of the previous state format
- **WHEN** an update waits for a merge request on `git-intake/update-<short verified commit>`, opened by a release with state format 4
- **THEN** `finish` and `abort` act on that branch, and a catch-up pushes to it

### Requirement: Same behaviour from every directory
Every command SHALL behave the same whichever directory of the repository it runs in: the main checkout, a subdirectory, a linked worktree, or the update worktree, whether the git directory is `.git` in the main worktree, lies outside it, or belongs to a submodule. Only finding the integration branch from the configuration committed at `HEAD` when no update is in progress, the note about an uncommitted configuration edit, and resolving the revisions named to `start` SHALL depend on the worktree the command runs in. The tool SHALL run git operations on the repository as a whole in the main worktree, or in the git directory when the repository is bare, so that a relative remote URL means what it means in the main worktree. A bare repository has no main worktree, so its update worktree SHALL be `git-intake/worktree` in the git directory; when that path contains `/.git`, which compilers, bundlers and watchers ignore, `start` SHALL refuse, say why, advise a clone with a checkout, and change nothing. `start` SHALL record the main worktree with the update so that the update's later commands find it from any worktree. When the main worktree cannot be located, because the git directory lies outside it without naming it, no update is in progress and the command runs elsewhere, a command that needs the main checkout SHALL refuse and say to run it in the main worktree, `doctor` SHALL warn that it did not check the main checkout and why, and a relative remote URL SHALL resolve against the git directory. The tool SHALL prefer the worktree it runs in, when that is the main worktree, and the main worktree the update recorded over taking the parent of a git directory named `.git` for it. A command that removes the update worktree it runs in SHALL complete and, after its own output, SHALL note that the current directory was removed.

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
- **THEN** `start` refuses and says to run it in the main worktree, and `doctor` warns that it did not check the main checkout

#### Scenario: Bare repository
- **WHEN** an update is started in a bare repository and finished from the update worktree
- **THEN** the update worktree lies in the git directory, and `finish` moves the integration branch and removes it

#### Scenario: Bare repository at a path containing `/.git`
- **WHEN** `start` runs in a bare repository whose update worktree path would contain `/.git`
- **THEN** it refuses, names the reason, advises a clone with a checkout, and creates no worktree, branch or state

#### Scenario: Separate git directory named `.git`
- **WHEN** the git directory is a directory named `.git` apart from the checkout, and an update is started in the checkout and finished from the update worktree
- **THEN** the checkout is taken as the main worktree: nothing is written beside the git directory, and relative remote URLs resolve against the checkout

#### Scenario: Revision named in a linked worktree
- **WHEN** `start` runs in a linked worktree with a revision relative to that worktree, such as `HEAD^2`
- **THEN** the revision is resolved in that worktree

#### Scenario: Linked checkout of the integration branch
- **WHEN** a linked worktree has the integration branch checked out and `finish` runs in the update worktree
- **THEN** that checkout follows the fast-forward and stays clean

### Requirement: Upstream fetch by URL
The tool SHALL fetch an upstream by its configured URL, in the main worktree and without tags, into a reference under `refs/git-intake/`, and SHALL NOT add, change or read a git remote for it. A command that reads upstreams SHALL fetch them itself. Apart from `contribute`, the tool SHALL create no branch, remote-tracking branch or tag from an upstream.

#### Scenario: Remote with an upstream's URL
- **WHEN** the repository has a git remote whose URL names a configured upstream's repository
- **THEN** commands fetch the upstream by URL without using or changing that remote, and `doctor` reports the remote

#### Scenario: Upstream unreachable
- **WHEN** one of several upstreams cannot be fetched
- **THEN** `status` reports that upstream as unavailable with git's error, reports the others, and exits with a failure status, and `start` refuses and creates nothing

### Requirement: Lifetime of upstream references
A command SHALL fetch upstreams into references of its own that no other command uses, and SHALL remove them before it exits, whatever its outcome. An update SHALL hold `refs/git-intake/upstreams/<name>` for each queued upstream at the tip `start` fetched; only `start` SHALL write it and only `finish` or `abort` SHALL remove it. A command that acts on the update in progress SHALL restore such a reference that is missing, from the update state, when its commit is in the repository.

#### Scenario: Reading command leaves nothing behind
- **WHEN** `status` runs on a repository with no update in progress
- **THEN** it reports every upstream from a fresh fetch, and afterwards the repository holds no reference under `refs/git-intake/`, no remote and no branch it did not hold before

#### Scenario: Update keeps its references
- **WHEN** an update stops on a conflict and the repository is later garbage-collected
- **THEN** every target and planned commit of the update is still present, and `continue` proceeds without fetching

#### Scenario: Start interrupted before holding its references
- **WHEN** `start` saved the update but stopped before writing the update's references, leaving the fetched commits held only by its own references
- **THEN** `status`, `continue`, `verify` and `finish` restore the update's references from the update state before they go on

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
Where the tool compares a URL with an upstream's URL, URLs that name the same repository SHALL count as equal: local paths and `file://` URLs once resolved against the main worktree as git finds a repository (`<path>/.git`, `<path>`, `<path>.git`) and made canonical; other URLs once the scp-like form is read as `ssh://`, scheme and host are in lower case, a default port is dropped, and a leading `/` and a trailing `/` and `.git` of the path are dropped.

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

### Requirement: Contribution branch
`contribute <upstream>[=<revision>] <branch>` SHALL fetch the upstream and create the local branch `<branch>` at the fetched tip, or at the commit the revision names on the fetched branch as `start` resolves it, without the rules that compare the commit with the last synced commit. It SHALL set no tracking branch, change no checkout and never push the branch. It SHALL refuse when `<branch>` exists or is not a valid branch name.

#### Scenario: Branch for a contribution
- **WHEN** `contribute` runs for an upstream with a new branch name
- **THEN** the branch exists at the upstream's fetched tip, contains no commit that the upstream branch lacks, has no tracking branch, and nothing was pushed

#### Scenario: Branch exists
- **WHEN** `contribute` names a branch that exists
- **THEN** it refuses and changes nothing

### Requirement: Follow update with a fixed target
For upstreams in `follow` mode, `start` SHALL accept upstream names, each optionally followed by `=` and a revision, and select every configured upstream when none is named. It SHALL fetch the selected upstreams, record as each upstream's target the commit its revision names, or, when no revision is given, the newest release of an upstream with a `tags` pattern and the fetched upstream tip of any other, record the current integration tip as the base, and queue, in configuration order, the selected upstreams whose target the base does not yet contain.

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

#### Scenario: Several upstreams at chosen points
- **WHEN** `start` names one upstream with a revision and another without
- **THEN** the first is merged at the commit its revision names and the second at its fetched tip, in one update

### Requirement: Release tags as follow targets
For a `follow` upstream with a `tags` pattern named without a revision, `start` SHALL take as the target the newest release: of the tags of the upstream's repository, listed from its URL, those whose name matches the pattern, is a semantic version without a prerelease part or build metadata once the pattern's literal prefix, and one `v` or `V` after it unless the prefix ends in one, are removed, and points at a commit of the fetched upstream branch, the one with the highest version by semantic versioning precedence. The tool SHALL fetch no tag into the repository. When the integration branch already contains the release, `start` SHALL report the upstream as current. When there is no release, `start` SHALL refuse, name the upstream, its URL and the pattern, and say to name the target as a revision. A revision named for the upstream SHALL override the pattern.

#### Scenario: Newest release by version
- **WHEN** an upstream with the pattern `v*` has the tags `v0.9.0` and `v0.10.0` on its branch, `v0.9.0` on the newer commit, and commits after both
- **THEN** `start` takes the commit of `v0.10.0`, and the sync record names the tag `v0.10.0`

#### Scenario: Tags that are not releases
- **WHEN** an upstream's newest matching tags are a prerelease such as `v1.0.0-rc.1`, a version with build metadata such as `v1.0.0+build.5`, a name that is no semantic version, or a tag on a branch the fetched branch does not contain
- **THEN** `start` ignores them and takes the newest release among the rest

#### Scenario: Repository tags untouched
- **WHEN** an update takes an upstream at a release tag
- **THEN** the repository's tags are the same before and after the update

#### Scenario: Release already taken
- **WHEN** the integration branch contains the newest release, including after an earlier update took a later commit by naming it
- **THEN** `start` reports the upstream as current at that release and creates nothing for it

#### Scenario: No release on the branch
- **WHEN** no tag of the upstream's repository is a release on its fetched branch
- **THEN** `start` refuses, names the upstream, its URL and the pattern, and creates nothing

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
`start` SHALL refuse when an update is already in progress, when the main checkout or the integration branch has uncommitted changes, when the publish remote cannot be fetched, when the integration branch and its published counterpart each hold commits the other lacks, or when a selected upstream is unknown, named more than once, or rewritten. When the integration branch is strictly behind its published counterpart, `start`, having read the integration branch and the publish remote from the configuration at `HEAD`, SHALL fast-forward it to that counterpart, moving a checkout of the branch with `git merge --ff-only` and the branch otherwise with `git update-ref` against its old value, refusing when a checkout would lose changes, and say that it did. It SHALL then read the configuration again at the new tip and fix the base there, or refuse, keeping the fast-forward, when that configuration names another integration branch or publish remote.

#### Scenario: Update already in progress
- **WHEN** `start` runs while an update is in progress
- **THEN** it refuses, names the step the update has reached, and creates nothing

#### Scenario: Integration branch behind the publish remote
- **WHEN** the publish remote's integration branch holds commits the local one lacks, and the local one holds none the remote lacks
- **THEN** `start` fast-forwards the local integration branch to the remote's, says so, reads the configuration again at the new tip, and starts the update with that tip as its base

#### Scenario: Integration branch diverged from the publish remote
- **WHEN** the publish remote's integration branch and the local one each hold commits the other lacks
- **THEN** `start` refuses, names how many commits each side lacks, moves no branch and creates nothing

### Requirement: Integration line
The tool SHALL find its sync records, and the merges that brought a path upstream's history into its path, on the integration line: the first-parent line of the integration branch, where each merge on it is followed by the commits that `git rev-list --first-parent <merge>^1..<merge>^2` lists for it. A merge among those commits that carries the trailer `Intake-Catch-Up` SHALL in turn be followed by the first-parent line of its second parent, down to the first commit that the first parent of the merge on the first-parent line contains, at any depth; other merges among those commits SHALL contribute nothing further. An update SHALL count the same whether it was published by push or through a merge request merged by fast-forward or with a merge commit, and whether or not it caught up with the integration branch.

#### Scenario: Merge request merged with a merge commit
- **WHEN** an update's merge request is merged with a merge commit, with or without semi-linear history, and `finish` completes the update
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

#### Scenario: Branch merged into another branch
- **WHEN** a branch that carries a sync record is merged into another branch with a merge that carries no `Intake-Catch-Up` trailer, which is then merged into the integration branch with a merge commit
- **THEN** that record does not count

#### Scenario: Update that caught up twice
- **WHEN** an update caught up twice and was published by push
- **THEN** its sync records and copies count, as for an update merged straight in

#### Scenario: Caught-up update merged with a merge commit
- **WHEN** an update that caught up is published through a merge request merged with a merge commit
- **THEN** its sync records and copies count, and a port upstream's taken commits are not pending

#### Scenario: Port started after a merge request merged with a merge commit
- **WHEN** a `follow` update merged through a merge request with a merge commit is followed by switching that upstream to `port` mode
- **THEN** the port's base is the update's target, and the journal the next port update writes starts there

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
The tool SHALL make a catch-up merge only when the new tip contains the base and not the verified commit. It SHALL refuse and merge nothing, naming `abort`, or `finish --no-publish` once `finish` moved the integration branch, when the new tip does not contain the base, or when the commits it adds carry a sync record or a copy of a queued upstream, change the journal of a queued port upstream, or change the configuration other than its install command and checks.

#### Scenario: Rewritten integration branch
- **WHEN** the integration branch moves to a commit that does not contain the update's base
- **THEN** the tool makes no catch-up merge, refuses, says that the integration branch was rewritten, names `abort`, and moves no branch

#### Scenario: Upstream taken in elsewhere
- **WHEN** a commit that landed on the integration branch carries a sync record of an upstream the update merges, as when another clone took that upstream in
- **THEN** the tool makes no catch-up merge, names the commit and the upstream, and names `abort`

#### Scenario: Configuration changed
- **WHEN** a commit that landed on the integration branch changes an upstream or the publication setting in `.git-intake.toml`
- **THEN** the tool makes no catch-up merge, names the change, and names `abort`

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
- **THEN** it refuses and names `finish` and `finish --no-publish`, and `finish --no-publish` abandons the catch-up merge and closes the update at the planned commit

### Requirement: Catch-up in status, report and doctor
`status` SHALL show a catch-up step with the tip caught up with and the next command, and `status --json` SHALL give it as the update step `catch_up_conflict` or `catch_up`. `report` SHALL list each catch-up merge with the tip it took in and its conflicted files with their resolution source, and measure upstream ranges and the repository's own changes from the base the update started from. `doctor` SHALL accept a catch-up step.

#### Scenario: Status during a catch-up
- **WHEN** a catch-up merge stopped on a conflict and an agent runs `status --json`
- **THEN** the update step is `catch_up_conflict`, the tip caught up with is named, and the next command is `continue`

#### Scenario: Report after a catch-up
- **WHEN** `report` runs on an update that caught up once with a conflict
- **THEN** it lists the catch-up merge with its tip and its conflicted files, and each upstream's merged range and own-change sizes are those measured from the base the update started from
