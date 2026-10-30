## MODIFIED Requirements

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

## ADDED Requirements

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
