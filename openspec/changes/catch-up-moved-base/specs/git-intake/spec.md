## ADDED Requirements

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

## MODIFIED Requirements

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
