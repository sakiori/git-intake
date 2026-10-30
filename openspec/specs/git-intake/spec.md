# git-intake Specification

## Purpose

Keep a repository current with one or more upstreams, each fetched by its URL only while it is needed: merged up to a fixed target at the repository root or under its own path, or taken commit by commit through a reviewed plan with a journal of decisions; with isolated updates, recorded verification, guarded publication and per-upstream status.

## Requirements

### Requirement: Fork configuration
The tool SHALL read a repository's settings from a `.git-intake.toml` file committed on the integration branch: the integration branch, an optional publication setting with the publish remote and the mode `push` or `merge-request`, an install command, an ordered list of check commands, and an ordered list of upstreams. Each upstream SHALL have a unique name, a URL, a branch, a mode `follow` or `port` with `follow` as the default, and optionally a path inside the repository. No two upstreams SHALL have the same URL and branch. An upstream without a path is taken at the repository root. The tool SHALL read the committed file from the integration branch tip, not from a working tree, and SHALL refuse to run when the file is missing or invalid, naming the problem. It SHALL refuse the settings `remote` and `mirror`, naming what replaces them, an upstream in `port` mode with a path, and a publish remote whose fetch or push URL names a configured upstream's repository.

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

#### Scenario: Publish remote at an upstream's URL
- **WHEN** the publish remote's fetch or push URL names a configured upstream's repository
- **THEN** the tool refuses to run and names the remote and the upstream

### Requirement: Sync record in history
Every merge commit the tool creates SHALL name the upstream and its target commit in commit trailers, and every commit it copies from an upstream in `port` mode SHALL name the upstream and the upstream commit it took in the trailers `Intake-Upstream` and `Intake-Taken`. For an upstream in `follow` mode, the tool SHALL derive the last synced commit from its sync records on the integration line, as the target of the newest record whose target no other of its records' targets contains, and SHALL NOT store it in the configuration or outside the history. For such an upstream that carries no sync record, the last synced commit SHALL come from the history, and the tool SHALL show that it does: for a root upstream it is the newest upstream commit the integration branch contains; for a path upstream it is the second parent of the newest merge on the integration line whose second parent belongs to the upstream's branch and which changes the upstream's path, as an import by `git subtree add` or `git subtree pull` leaves. This decision SHALL depend only on the integration branch and the upstream's own branch, never on other upstreams. When a `follow` upstream's last synced commit is no longer contained in its branch, the tool SHALL report that upstream's history as rewritten and SHALL refuse to update it.

#### Scenario: Last synced commit
- **WHEN** the integration line contains a merge of a `follow` upstream
- **THEN** `status` shows that upstream's target from the most recent such merge as its last synced commit

#### Scenario: Older record merged later
- **WHEN** a branch that carries a sync record of an older target is merged into the integration branch after a newer target's record landed
- **THEN** `status` still shows the newer target as the last synced commit, and nothing between the two targets counts as pending

#### Scenario: Upstream force-pushed
- **WHEN** a `follow` upstream's branch no longer contains the last synced commit
- **THEN** `status` reports the upstream as rewritten and `start` refuses to update it

#### Scenario: Copied commit
- **WHEN** a port update copies an upstream commit
- **THEN** the copy carries the upstream's name and the upstream commit's id in its trailers

### Requirement: Recorded resolutions
The tool SHALL enable git's recorded conflict resolution for the repository before merging or copying commits, so a resolution made in one update or attempt is reapplied when the same conflict recurs.

#### Scenario: Retried update
- **WHEN** an aborted update is started again and hits a conflict resolved in the earlier attempt
- **THEN** git reapplies the recorded resolution and the tool reports the file as resolved by a recorded resolution

### Requirement: Resumable state
The tool SHALL keep the state of an update in the repository's common git directory, visible from every worktree, and the update worktree at `.worktrees/git-intake/update` in the main worktree, never inside the git directory. Unless that path is ignored already, `start` SHALL add the line `/.worktrees/` to the repository's `info/exclude` once, without changing a committed ignore file, so that the main checkout does not show the update worktree. Every command SHALL act on that state: `continue` carries out a plan, concludes a resolved merge or copy, and goes on with the remaining queued upstreams or planned commits; `status` shows the step reached, each queued upstream as merged, at its plan, in conflict or pending, and what comes next; and `abort` removes the worktree, the update branch, the plan, the update's upstream references, its logs and the state without touching the integration branch. A `follow` upstream SHALL count as merged only when the update branch head carries its sync record for its target, and a `port` upstream as carried out only when its plan has been carried out to the journal commit. Once `finish` has begun moving branches, `abort` SHALL refuse and name `finish` as the way to complete the update. While an update waits for its merge request, `abort` SHALL delete the merge request branch from the publish remote, remove the update, and name the merge request that is left to close. The tool SHALL refuse an update state written by a release whose state format it does not read, and say to finish or abort that update with the release that started it.

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
- **THEN** the integration branch is as before `start`, and no worktree, update branch, plan, upstream reference or log of the update remains

#### Scenario: User's worktree on the update branch
- **WHEN** no update is in progress and a worktree other than the tool's update worktree has `git-intake/update` checked out
- **THEN** `abort` and `start` refuse and `doctor` fails, each naming that worktree and saying to remove or switch it, and the worktree and its uncommitted changes are kept

#### Scenario: State of an earlier release
- **WHEN** an update started by a release with another state format is in progress, such as one that kept the update worktree in the git directory
- **THEN** every command that reads the update refuses and says to finish or abort it with that release

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

### Requirement: Guarded publication
`finish` SHALL require a verified update whose worktree commit is the verified one and whose base is still the integration tip. Before moving any branch it SHALL confirm that the move of the integration branch is a fast-forward, that the verified commit contains the base and every merged target, and that no checkout of the integration branch would lose changes. It SHALL record the planned move, fast-forward the integration branch to the verified commit, push exactly the planned commit without force when the publication mode is `push`, and only after publication succeeds remove the worktree, the update branch, the update's upstream references and the state. An interrupted or failed `finish` SHALL complete the remaining move and the push when run again, and SHALL never publish a commit other than the planned one. `finish --no-publish` SHALL complete the local move and remove the worktree, the update branch, the upstream references and the state without pushing, and report that the integration branch remains unpublished. It SHALL never rewrite or force-push a branch, and SHALL never push to an upstream's URL.

#### Scenario: Integration branch moved
- **WHEN** a commit lands on the integration branch after `start`
- **THEN** `finish` refuses and explains that the update must take the new integration tip and be verified again

#### Scenario: Remote moved
- **WHEN** the publish remote's branch is ahead of the local one
- **THEN** the push is rejected, no remote branch moves, the local integration branch stays advanced, the update stays in its finishing step, and the tool explains that `finish --no-publish` closes the update locally so that the remote's work can be merged and the result published by a new verified update or the repository's usual review

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
`report` SHALL summarise an update in progress or just merged, per upstream. For a `follow` upstream it SHALL give the number of upstream commits merged, the conflicted files and whether each was resolved by hand or by a recorded resolution, and how the repository's own changes relative to that upstream grew or shrank. For a `port` upstream it SHALL give the taken commits in the order they were copied, each with its conflicted files and their resolution source, the commits recorded as `have` because their copy changed nothing, the `skip` and `have` decisions with their notes, the number of commits left pending, and, for each taken commit, the earlier upstream commits still pending that changed any of the same files.

#### Scenario: Report after merge
- **WHEN** the user runs `report` after resolving conflicts in a `follow` update
- **THEN** the output lists, for each merged upstream, the merged range, each conflicted file with its resolution source, and the diff size against that upstream before and after

#### Scenario: Report of a port
- **WHEN** the user runs `report` after a plan that took a commit whose files an earlier pending upstream commit also changed
- **THEN** the output lists the taken commit with its conflicts and names the earlier pending commit beside it

### Requirement: Upstream status
`status` SHALL fetch every upstream and report, for each, its path, its branch, its mode and its fetched tip. For a `follow` upstream it SHALL report its last synced commit, the number of commits the integration branch lacks, and one state: current, behind, not imported, path removed when a synced path upstream's path no longer exists, rewritten, or unavailable when its URL cannot be fetched or its branch is missing. For a `port` upstream it SHALL report its journal's base, the numbers of taken, decided and pending commits, and one state: current, pending, no base, rewritten, or unavailable. `status --check` SHALL additionally list the files a merge of each behind `follow` upstream's tip would conflict in, create no worktree, branch or state, and exit with a failure status unless every upstream is current, so that it can run on a schedule and gate a cutover or a port review.

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
When the publication mode is `merge-request`, `finish` SHALL apply the same checks as in push mode, then push the verified commit without force to a new branch `git-intake/update-<short verified commit>` on the publish remote, requesting through git push options a merge request from that branch into the integration branch whose title names the updated upstreams and whose description lists each `follow` upstream's merged range and each `port` upstream's taken commits and decisions. It SHALL move no local branch, remove the update worktree, and keep the update in a step that waits for the merge request. When the remote does not accept push options, the tool SHALL push the branch without them and name the merge request to open. When the branch is already at the verified commit where pushes to the publish remote go, as after an interrupted `finish`, `finish` SHALL push nothing and count no merge request as requested; when the remote has several push URLs, it SHALL push only to those where the branch is not yet at the verified commit. Deleting the merge request branch SHALL likewise act on each push URL and report what remains on each. A later `finish` SHALL fetch the publish remote and, once its integration branch received the verified commit itself by fast-forward or a merge whose tree equals the verified commit's tree, fast-forward the local integration branch to the remote one, delete the merge request branch if it remains, and remove the update. While the merge request is open it SHALL report that the update waits for it, exit with the status of a negative result, and say so when the merge request branch no longer points at the verified commit. It SHALL count a merge request as requested only when the push that carried the options transferred the branch. When the remote's response names the merge request it opened, the tool SHALL record its number and judge the merge by the merge request's head as the remote keeps it (`refs/merge-requests/<number>/head`), which outlives the merge request branch: a merge request whose final head is not the verified commit SHALL count as not having landed the verified tree, whichever merge method was used. Without that reference, the tool SHALL judge by the merge request branch as last seen and SHALL say, when it completes a fast-forward merge, that commits above the verified commit could not be attributed. It SHALL refuse to complete, and explain why, when the integration branch changed without receiving the verified commit (as by a squash or rebase merge), when what it received differs from the verified tree, when the integration branch moved so that the verified commit no longer contains its tip, or when the merge request branch disappeared without a merge. `abort` while waiting SHALL refuse when the integration branch received the verified commit as required and name `finish`; after a merge that did not land the verified tree it SHALL remove the update and say that the integration branch holds an unverified merge.

#### Scenario: Merge request opened
- **WHEN** `finish` runs on a verified update in merge-request mode against a remote that accepts push options
- **THEN** the branch with the verified commit and a merge request into the integration branch exist on the publish remote, the local integration branch has not moved, and `status` shows the update waiting for the merge request

#### Scenario: Merge request merged
- **WHEN** the merge request is merged by fast-forward or with a merge commit and `finish` runs again
- **THEN** the local integration branch contains the verified commit, and no update or upstream reference of it remains

#### Scenario: Merge request still open
- **WHEN** `finish` runs while the merge request is not merged
- **THEN** the tool reports that the update waits for the merge request and changes nothing

#### Scenario: Merged by squash or rebase
- **WHEN** the integration branch on the publish remote carries the update's changes but not the verified commit
- **THEN** `finish` refuses, explains that the merge request must be merged without squash or rebase, and moves no branch

#### Scenario: Integration branch moved before the merge
- **WHEN** a commit lands on the integration branch of the publish remote before the merge request is merged
- **THEN** `finish` refuses and explains that the update must be aborted and started again on the new tip

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
- **THEN** it moves no branch and reports that the update waits

#### Scenario: Merge request closed without merging
- **WHEN** the merge request branch disappears while the integration branch did not receive the verified commit
- **THEN** `finish` refuses and names `abort`

#### Scenario: Fast-forward merge of a changed merge request
- **WHEN** a commit is added to the merge request branch, the merge request is merged by fast-forward and its branch is removed
- **THEN** `finish` reads the merge request's head from the remote, refuses because it is not the verified commit, and moves no branch

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
An upstream's name SHALL be usable in a git reference name, as `refs/git-intake/upstreams/<name>` needs it. The tool SHALL refuse a name that contains `..` or ends with `.` or `.lock`, naming it.

#### Scenario: Name git cannot use
- **WHEN** an upstream is named `app.lock`
- **THEN** the tool refuses to run and names the upstream and why

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
For upstreams in `follow` mode, `start` SHALL accept upstream names, each optionally followed by `=` and a revision, and select every configured upstream when none is named. It SHALL fetch the selected upstreams, record as each upstream's target the commit its revision names, or the fetched upstream tip when no revision is given, record the current integration tip as the base, and queue, in configuration order, the selected upstreams whose target the base does not yet contain.

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

### Requirement: Isolated merge
A follow update SHALL create a dedicated worktree on a new branch from the base and merge each queued target in turn with its own merge commit: at the repository root for an upstream without a path, and under the upstream's path otherwise.

#### Scenario: Clean merge
- **WHEN** every queued target merges without conflicts
- **THEN** the worktree holds one merge commit per upstream and the next step is `verify`

#### Scenario: Conflicts
- **WHEN** a merge stops on conflicts
- **THEN** the tool names the upstream, lists the conflicted files, leaves the merge in progress in the worktree, reports which upstreams are merged and which are still pending, and tells the user to resolve the conflicts and run `continue`

### Requirement: Fixed follow target
A target SHALL NOT change for the rest of the update, and `status` and `report` SHALL show the revision a target was named by.

#### Scenario: Upstream moved during the update
- **WHEN** an upstream receives new commits after `start`
- **THEN** the update still merges, verifies and publishes the recorded target

### Requirement: Revision of a follow target
A revision named to `start` for a `follow` upstream, without a leading `refs/tags/` or `tags/`, SHALL name a tag of the upstream's repository when it has a tag of that name, and otherwise resolve as a revision of the repository. The tool SHALL fetch no tag into the repository, and SHALL refuse a revision that resolves through one of the repository's own tags, or whose tag it cannot look up on the upstream's URL.

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
- **WHEN** the upstream's URL cannot be asked for the tag after the fetch
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
`start` SHALL refuse when an update is already in progress, when the main checkout or the integration branch has uncommitted changes, when the integration branch is behind its published counterpart, or when a selected upstream is unknown, named more than once, or rewritten.

#### Scenario: Update already in progress
- **WHEN** `start` runs while an update is in progress
- **THEN** it refuses, names the step the update has reached, and creates nothing

#### Scenario: Integration branch behind the publish remote
- **WHEN** the publish remote's integration branch holds a commit the local one lacks
- **THEN** `start` refuses and creates nothing

### Requirement: Integration line
The tool SHALL find its sync records, and the merges that brought a path upstream's history into its path, on the integration line: the first-parent line of the integration branch, where each merge on it is followed by the commits that `git rev-list --first-parent <merge>^1..<merge>^2` lists for it. Merges among those commits SHALL contribute nothing further. An update SHALL count the same whether it was published by push or through a merge request merged by fast-forward or with a merge commit.

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
- **WHEN** a branch that carries a sync record is merged into another branch, which is then merged into the integration branch with a merge commit
- **THEN** that record does not count

#### Scenario: Port started after a merge request merged with a merge commit
- **WHEN** a `follow` update merged through a merge request with a merge commit is followed by switching that upstream to `port` mode
- **THEN** the port's base is the update's target, and the journal the next port update writes starts there

### Requirement: Upstream history on the integration line
A merge on the first-parent line that carries a sync record whose target is its second parent, as every sync merge does, SHALL bring nothing into the integration line, because what it merged is the upstream's history. Commits contained in a configured upstream's fetched branch SHALL NOT count as this repository's sync records.

#### Scenario: Upstream that records syncs of its own
- **WHEN** an upstream's history carries sync records of its own, the integration branch merged it with a sync merge, and the upstream cannot be fetched
- **THEN** none of the upstream's records counts as one of this repository's, and `doctor` reports no upstream dropped from the configuration
