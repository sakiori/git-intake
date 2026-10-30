# git-intake Specification

## Purpose

Keep a repository current with one or more upstreams by merge: each upstream at the repository root or under its own path, with fixed targets, isolated merges, recorded verification, guarded publication and per-upstream status.

## Requirements

### Requirement: Fork configuration
The tool SHALL read a repository's settings from a `.git-intake.toml` file committed on the integration branch: the integration branch, an optional publication setting with the publish remote and the mode `push` or `merge-request`, an install command, an ordered list of check commands, and an ordered list of upstreams. Each upstream SHALL have a unique name, a remote, a branch, and optionally a remote URL, a path inside the repository, and a mirror branch. An upstream without a path is merged at the repository root. The tool SHALL read the committed file from the integration branch tip, not from a working tree, and SHALL refuse to run when the file is missing or invalid, naming the problem.

#### Scenario: Valid configuration
- **WHEN** the integration branch carries a valid configuration
- **THEN** every command uses its values and `status` shows them

#### Scenario: Uncommitted edits are ignored
- **WHEN** the working tree holds an edited but uncommitted configuration
- **THEN** the tool uses the committed version and says that the working copy differs

#### Scenario: Stacked downstream branch
- **WHEN** a repository's configuration names another repository's integration branch as its upstream and has no mirror branch
- **THEN** updates merge that branch into the local integration branch the same way as an upstream project

#### Scenario: Several upstreams at their paths
- **WHEN** the configuration lists several upstreams, each with its own path
- **THEN** every command treats each upstream as a separate source merged under its path

#### Scenario: Ambiguous layout
- **WHEN** two upstreams share a name or a remote, one upstream's path lies inside another's, or an upstream without a path is listed together with other upstreams
- **THEN** the tool refuses to run and names the conflicting entries

#### Scenario: Publication setting
- **WHEN** the configuration names a publish remote without a mode
- **THEN** the tool publishes by `push`

### Requirement: Upstream remotes
Before fetching, the tool SHALL ensure that every selected upstream's remote exists. It SHALL add a missing remote when the upstream has a configured URL, and SHALL refuse when the remote is missing without a URL or when an existing remote's URL differs from the configured one. `fetch` SHALL fetch the selected upstreams, or all of them when none is named, and report each upstream's outcome.

#### Scenario: First fetch in a new clone
- **WHEN** `fetch` runs in a repository where the configured remotes do not exist yet and every upstream has a URL
- **THEN** the tool adds each remote, fetches it, and reports each upstream as fetched

#### Scenario: Remote points elsewhere
- **WHEN** an existing remote's URL differs from the configured URL
- **THEN** the tool fetches nothing from that remote and names the upstream, the configured URL and the actual URL

#### Scenario: One upstream unreachable
- **WHEN** one of several upstreams cannot be fetched
- **THEN** the tool fetches the others, reports the failed upstream with the git error, and exits with a failure status

### Requirement: Sync record in history
Every merge commit the tool creates SHALL name the upstream and its target commit in commit trailers. The tool SHALL derive the last synced commit of each upstream from these trailers on the integration branch and SHALL NOT store it in the configuration or outside the history. For an upstream that carries no sync record, the last synced commit SHALL come from the history, and the tool SHALL show that it does: for a root upstream it is the newest upstream commit the integration branch contains; for a path upstream it is the second parent of the newest merge on the integration branch's first-parent line whose second parent belongs to the upstream's branch and which changes the upstream's path, as an import by `git subtree add` or `git subtree pull` leaves. This decision SHALL depend only on the integration branch and the upstream's own branch, never on other upstreams. When an upstream's last synced commit is no longer contained in its branch, the tool SHALL report that upstream's history as rewritten and SHALL refuse to update it.

#### Scenario: Last synced commit
- **WHEN** the integration branch contains a merge of an upstream
- **THEN** `status` shows that upstream's target from the most recent such merge as its last synced commit

#### Scenario: Upstream force-pushed
- **WHEN** an upstream's branch no longer contains the last synced commit
- **THEN** `status` reports the upstream as rewritten and `start` refuses to update it

### Requirement: Fixed target and isolated merge
`start` SHALL accept upstream names, each optionally followed by `=` and a revision, and select every configured upstream when none is named. It SHALL fetch the selected upstreams, record as each upstream's target the commit its revision names, or the fetched upstream tip when no revision is given, record the current integration tip as the base, and queue, in configuration order, the selected upstreams whose target the base does not yet contain. A revision, without a leading `refs/tags/` or `tags/`, SHALL name a tag of the upstream's remote when that remote has a tag of that name, and otherwise resolve as a revision of the repository; the tool SHALL fetch no tag into the repository, and SHALL refuse a revision that resolves through one of the repository's own tags, or whose tag it cannot look up on the remote. `start` SHALL refuse a revision that names no commit, a target that the fetched upstream branch does not contain, a target older than the upstream's last synced commit, a target that does not contain the last synced commit, and a target that does not contain the upstream's existing mirror branch, naming the upstream and the revision. It SHALL create a dedicated worktree on a new branch from the base and merge each queued target in turn with its own merge commit: at the repository root for an upstream without a path, and under the upstream's path otherwise. A target SHALL NOT change for the rest of the update, and `status` and `report` SHALL show the revision a target was named by. `start` SHALL refuse when an update is already in progress, when the main checkout or the integration branch has uncommitted changes, when the integration branch is behind its published counterpart, or when a selected upstream is unknown, named more than once, or rewritten.

#### Scenario: Clean merge
- **WHEN** every queued target merges without conflicts
- **THEN** the worktree holds one merge commit per upstream and the next step is `verify`

#### Scenario: Conflicts
- **WHEN** a merge stops on conflicts
- **THEN** the tool names the upstream, lists the conflicted files, leaves the merge in progress in the worktree, reports which upstreams are merged and which are still pending, and tells the user to resolve the conflicts and run `continue`

#### Scenario: Upstream moved during the update
- **WHEN** an upstream receives new commits after `start`
- **THEN** the update still merges, verifies and publishes the recorded target

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
- **WHEN** a queued upstream's path exists at the base, the integration branch carries no sync record of that upstream, and no merge on the integration branch's first-parent line brought that upstream's history into its path
- **THEN** `start` refuses and names the upstream and the path

#### Scenario: Ancestry shared only through a template
- **WHEN** two upstreams descend from one template, one of them is imported, and the other's path already holds the repository's own content
- **THEN** the other upstream is not treated as imported, and `start` refuses for its path, whichever upstreams have been fetched

#### Scenario: Upstreams that merge each other
- **WHEN** an upstream imported without a sync record later appears in another upstream's history
- **THEN** it is still treated as imported at its path and is updated by a subtree merge

#### Scenario: Path imported by other means
- **WHEN** a queued upstream's path exists at the base and the integration branch carries no sync record of that upstream but its first-parent line holds a merge that brought the upstream's history into the path, as after an import by `git subtree add`
- **THEN** `status` shows the newest upstream commit contained in the integration branch as the last synced commit, and `start` updates the path by a subtree merge that records the sync

#### Scenario: Imported path removed downstream
- **WHEN** the integration branch carries a sync record of a path upstream but its path no longer exists at the base
- **THEN** `start` refuses and explains that the path was removed downstream

#### Scenario: File on the import path
- **WHEN** a leading component of a queued upstream's path is a file at the base
- **THEN** `start` refuses instead of replacing the file

#### Scenario: Chosen commit
- **WHEN** `start` names an upstream with a commit of its fetched branch that is older than the branch tip
- **THEN** that commit is the upstream's target: the update merges, verifies and publishes it, its sync record names it, and the mirror branch moves to it

#### Scenario: Chosen tag
- **WHEN** the revision is a tag of the upstream's remote on the upstream's branch, and the repository holds a tag of the same name on another commit
- **THEN** the commit of the upstream's tag is the target, and no tag is added to the repository

#### Scenario: Tag named with its prefix
- **WHEN** the revision is `refs/tags/<tag>` or `tags/<tag>` and the upstream's remote has the tag
- **THEN** the commit of the upstream's tag is the target

#### Scenario: Own tag instead of an upstream tag
- **WHEN** the upstream's remote has no tag of the revision's name, and the revision resolves through a tag of the repository, by itself or in an expression, even when a branch has the same name
- **THEN** `start` refuses and names the repository's tag, and creates nothing

#### Scenario: Expression on an upstream tag
- **WHEN** the revision is an expression on a tag that only the upstream's remote has, such as `<tag>~1`
- **THEN** `start` refuses because the revision names no commit in the repository, and creates nothing

#### Scenario: Remote unreachable for the tag lookup
- **WHEN** the upstream's remote cannot be asked for the tag after the fetch
- **THEN** `start` refuses, names the upstream and the remote with git's error, and creates nothing

#### Scenario: Mirror ahead of the target
- **WHEN** an upstream's mirror branch holds a commit that its target does not contain
- **THEN** `start` refuses, names the mirror branch and the target, and creates nothing

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

#### Scenario: Conflict at a chosen target
- **WHEN** the merge of a chosen target stops on conflicts, and the user resolves them and runs `continue` and then `verify` and `finish`
- **THEN** the merge records the chosen target, not the branch tip, and the mirror branch moves to it

#### Scenario: Several upstreams at chosen points
- **WHEN** `start` names one upstream with a revision and another without
- **THEN** the first is merged at the commit its revision names and the second at its fetched tip, in one update

### Requirement: Recorded resolutions
The tool SHALL enable git's recorded conflict resolution for the repository before merging, so a resolution made in one update or attempt is reapplied when the same conflict recurs.

#### Scenario: Retried update
- **WHEN** an aborted update is started again and hits a conflict resolved in the earlier attempt
- **THEN** git reapplies the recorded resolution and the tool reports the file as resolved by a recorded resolution

### Requirement: Resumable state
The tool SHALL keep the state of an update in the repository's common git directory, visible from every worktree, and every command SHALL act on that state: `continue` concludes a resolved merge and merges the remaining queued upstreams, `status` shows the step reached, each queued upstream as merged, in conflict or pending, and what comes next, and `abort` removes the worktree, the update branch, the update's logs and the state without touching the integration or any mirror branch. An upstream SHALL count as merged only when the update branch head carries its sync record for its target. Once `finish` has begun moving branches, `abort` SHALL refuse and name `finish` as the way to complete the update. While an update waits for its merge request, `abort` SHALL delete the merge request branch from the publish remote, remove the update, and name the merge request that is left to close.

#### Scenario: Resume after interruption
- **WHEN** the user returns to an update in a new shell
- **THEN** `status` names each upstream's target, the base, the worktree, the state of every queued upstream and the next command

#### Scenario: Continue after a conflict
- **WHEN** the user resolves a conflict and runs `continue` while other upstreams are pending
- **THEN** the tool concludes the merge and merges the pending upstreams in order, stopping again at the next conflict

#### Scenario: Abort
- **WHEN** the user aborts an update before `finish` has moved any branch
- **THEN** the integration and mirror branches are as before `start`, no worktree, update branch or log of the update remains, and the message names only the branches the configuration has

### Requirement: Verification of the published tree
`verify` SHALL require the configuration at the update worktree head to be valid and to name the same integration branch, then run the configured install command and every check in the update worktree once every queued upstream is merged, write each command's output to a log, report pass or fail per command, and record the worktree commit it verified. A failed or skipped check SHALL leave the update unverified.

#### Scenario: Checks pass
- **WHEN** every check succeeds
- **THEN** the tool records the verified commit and names `finish` as the next step

#### Scenario: Commit after verification
- **WHEN** a commit is added to the update branch after `verify`
- **THEN** `finish` refuses until `verify` passes for the new commit

#### Scenario: Upstreams still pending
- **WHEN** `verify` runs while a queued upstream is in conflict or pending
- **THEN** the tool refuses and names the upstreams that still need `continue`

### Requirement: Guarded publication
`finish` SHALL require a verified update whose worktree commit is the verified one and whose base is still the integration tip. Before moving any branch it SHALL confirm that every move is a fast-forward, that the verified commit contains the base and every merged target, and that no checkout of a moved branch would lose changes. It SHALL record the planned moves, fast-forward the integration branch to the verified commit and each merged upstream's mirror branch, where one is configured, to that upstream's target, push exactly the planned commits without force in one atomic push when the publication mode is `push`, and only after publication succeeds remove the worktree, the update branch and the state. An interrupted or failed `finish` SHALL complete the remaining moves and the push when run again, and SHALL never publish a commit other than the planned ones. `finish --no-publish` SHALL complete the local moves and remove the worktree, the update branch and the state without pushing, and report which branches remain unpublished. It SHALL never rewrite or force-push a branch, and SHALL never push to an upstream remote.

#### Scenario: Integration branch moved
- **WHEN** a commit lands on the integration branch after `start`
- **THEN** `finish` refuses and explains that the update must merge the new integration tip and be verified again

#### Scenario: Remote moved
- **WHEN** the publish remote's branch is ahead of the local one
- **THEN** the push is rejected, no remote branch moves, the local branches stay advanced, the update stays in its finishing step, and the tool explains that `finish --no-publish` closes the update locally so that the remote's work can be merged and the result published by a new verified update or the repository's usual review

#### Scenario: Publication never allowed
- **WHEN** the publish remote refuses every push to the integration branch, as for a protected branch
- **THEN** `finish --no-publish` leaves the local branches advanced, removes the update, and names the unpublished branches

#### Scenario: Local commit while finishing
- **WHEN** a commit is added to the integration branch after `finish` moved it and before the push succeeded
- **THEN** running `finish` again pushes only the planned commits

#### Scenario: Interrupted finish
- **WHEN** `finish` stops after moving some branches
- **THEN** `status` shows the finishing step, `abort` refuses, and running `finish` again completes the remaining moves and the push

#### Scenario: No publish remote
- **WHEN** no publish remote is configured
- **THEN** `finish` advances the local branches and pushes nothing

#### Scenario: Merge-request mode
- **WHEN** the publication mode is `merge-request`
- **THEN** `finish` publishes as the merge request publication requires and moves no local branch before the merge request is merged

### Requirement: Update report
`report` SHALL summarise an update in progress or just merged, per upstream: the number of upstream commits merged, the conflicted files and whether each was resolved by hand or by a recorded resolution, and how the repository's own changes relative to that upstream grew or shrank.

#### Scenario: Report after merge
- **WHEN** the user runs `report` after resolving conflicts
- **THEN** the output lists, for each merged upstream, the merged range, each conflicted file with its resolution source, and the diff size against that upstream before and after

### Requirement: Upstream status
`status` SHALL report, for every configured upstream, its path, its branch, its last synced commit, its fetched tip, the number of commits the integration branch lacks, and one state: current, behind, not imported, path removed when a synced path upstream's path no longer exists, rewritten, or unavailable when its remote or branch is missing. Without options it SHALL use the remote-tracking refs as last fetched. `status --check` SHALL first fetch every upstream, additionally list the files a merge of each behind upstream's tip would conflict in, create no worktree, branch or state, and exit with a failure status unless every upstream is current, so that it can run on a schedule and gate a cutover.

#### Scenario: Several upstreams
- **WHEN** `status` runs on a repository with three upstreams, two of them behind
- **THEN** it reports each upstream on its own line with its path and state, and the commit count for the two that are behind

#### Scenario: Scheduled check
- **WHEN** `status --check` runs on a repository whose single upstream is 30 commits ahead
- **THEN** it reports the 30 commits and the files that would conflict, exits with a failure status, and leaves the repository unchanged

#### Scenario: Zero drift
- **WHEN** `status --check` runs and every upstream is current
- **THEN** it reports every upstream as current and exits successfully

#### Scenario: Synced to a chosen target
- **WHEN** the last update of an upstream merged a chosen target that lies three commits before the fetched branch tip
- **THEN** `status` shows that target as the last synced commit, the tip, the state behind and three commits

### Requirement: Pending commits
`log` SHALL list, for the named upstreams or all of them, the commits on the fetched upstream branch that the integration branch does not contain.

#### Scenario: Inspecting an upstream before merging
- **WHEN** the user runs `log` for an upstream that is three commits behind
- **THEN** the tool lists those three commits with their hashes and subjects

### Requirement: Unmerged upstream branches
`branches` SHALL list, for the named upstreams or all of them, every branch of the upstream remote that holds commits the integration branch does not contain, with the number of such commits.

#### Scenario: Work left on a feature branch
- **WHEN** an upstream's remote has a feature branch with two commits that were never merged into its tracked branch
- **THEN** `branches` lists that branch under the upstream with two unmerged commits

#### Scenario: Nothing left behind
- **WHEN** every branch of every upstream is contained in the integration branch
- **THEN** `branches` reports that no upstream holds unmerged work

### Requirement: Doctor
`doctor` SHALL check, without changing the repository, that the configuration is valid, every upstream's remote exists with the configured URL, every upstream's branch exists, every imported upstream's path exists on the integration branch, every upstream's last synced commit is still contained in its branch, the main checkout is clean, no merge is in progress, and the update state, if any, matches its worktree and branch. It SHALL report each check with its result and exit with a failure status when any check fails.

#### Scenario: Healthy repository
- **WHEN** every check passes
- **THEN** `doctor` lists the checks as passed and exits successfully

#### Scenario: Upstream removed from the configuration
- **WHEN** the integration branch carries sync records of an upstream that the configuration no longer lists
- **THEN** `doctor` reports that upstream by name as dropped from the configuration

### Requirement: Machine-readable output
`status`, `report`, `log`, `branches` and `doctor` SHALL accept `--json` and then write a single JSON document to standard output that carries the same facts as the text form, under field names that are stable across releases, with the same exit status.

#### Scenario: Agent reads status
- **WHEN** an agent runs `status --json`
- **THEN** it receives, for every upstream, its name, path, branch, last synced commit, fetched tip, commit count and state without parsing text

### Requirement: Publication through a merge request
When the publication mode is `merge-request`, `finish` SHALL apply the same checks as in push mode, then push the verified commit without force to a new branch `git-intake/update-<short verified commit>` on the publish remote, requesting through git push options a merge request from that branch into the integration branch whose title names the merged upstreams and whose description lists each upstream's merged range. It SHALL move no local branch, remove the update worktree, and keep the update in a step that waits for the merge request. When the remote does not accept push options, the tool SHALL push the branch without them and name the merge request to open. A later `finish` SHALL fetch the publish remote and, once its integration branch received the verified commit itself by fast-forward or a merge whose tree equals the verified commit's tree, fast-forward the local integration branch to the remote one, fast-forward each merged upstream's mirror branch to its target and push the mirrors without force, delete the merge request branch if it remains, and remove the update. While the merge request is open it SHALL report that the update waits for it, exit with the status of a negative result, and say so when the merge request branch no longer points at the verified commit. It SHALL count a merge request as requested only when the push that carried the options transferred the branch. When the remote's response names the merge request it opened, the tool SHALL record its number and judge the merge by the merge request's head as the remote keeps it (`refs/merge-requests/<number>/head`), which outlives the merge request branch: a merge request whose final head is not the verified commit SHALL count as not having landed the verified tree, whichever merge method was used. Without that reference, the tool SHALL judge by the merge request branch as last seen and SHALL say, when it completes a fast-forward merge, that commits above the verified commit could not be attributed. It SHALL refuse to complete, and explain why, when the integration branch changed without receiving the verified commit (as by a squash or rebase merge), when what it received differs from the verified tree, when the integration branch moved so that the verified commit no longer contains its tip, or when the merge request branch disappeared without a merge. `abort` while waiting SHALL refuse when the integration branch received the verified commit as required and name `finish`; after a merge that did not land the verified tree it SHALL remove the update, leave the mirror branches unchanged, and say that the integration branch holds an unverified merge.

#### Scenario: Merge request opened
- **WHEN** `finish` runs on a verified update in merge-request mode against a remote that accepts push options
- **THEN** the branch with the verified commit and a merge request into the integration branch exist on the publish remote, the local integration and mirror branches have not moved, and `status` shows the update waiting for the merge request

#### Scenario: Merge request merged
- **WHEN** the merge request is merged by fast-forward or with a merge commit and `finish` runs again
- **THEN** the local integration branch contains the verified commit, the mirror branches are at their targets locally and on the publish remote, and no update remains

#### Scenario: Merge request still open
- **WHEN** `finish` runs while the merge request is not merged
- **THEN** the tool reports that the update waits for the merge request and changes nothing

#### Scenario: Merged by squash or rebase
- **WHEN** the integration branch on the publish remote carries the update's sync records but not the verified commit
- **THEN** `finish` refuses, explains that the merge request must be merged without squash or rebase, and moves no branch

#### Scenario: Integration branch moved before the merge
- **WHEN** a commit lands on the integration branch of the publish remote before the merge request is merged
- **THEN** `finish` refuses and explains that the update must be aborted and started again on the new tip

#### Scenario: Remote without push options
- **WHEN** the publish remote does not accept push options
- **THEN** the tool pushes the branch without them and names the source and target branches of the merge request to open

#### Scenario: Merge request branch changed before the merge
- **WHEN** a commit is added to the merge request branch and the merge request is merged with a merge commit
- **THEN** `finish` refuses because the merged tree is not the verified one, moves no branch, and `abort` removes the update, leaves the mirrors unchanged and names the unverified merge

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
- **THEN** it moves no branch and reports that the update waits

#### Scenario: Merge request closed without merging
- **WHEN** the merge request branch disappears while the integration branch did not receive the verified commit
- **THEN** `finish` refuses and names `abort`

#### Scenario: Fast-forward merge of a changed merge request
- **WHEN** a commit is added to the merge request branch, the merge request is merged by fast-forward and its branch is removed
- **THEN** `finish` reads the merge request's head from the remote, refuses because it is not the verified commit, and moves no branch

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
