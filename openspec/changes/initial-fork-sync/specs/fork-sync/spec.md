## ADDED Requirements

### Requirement: Fork configuration
The tool SHALL read a repository's settings from a `.fork-sync.toml` file committed on the integration branch: the integration branch, an optional publish remote, an install command, an ordered list of check commands, and an ordered list of upstreams. Each upstream SHALL have a unique name, a remote, a branch, and optionally a remote URL, a path inside the repository, and a mirror branch. An upstream without a path is merged at the repository root. The tool SHALL read the committed file from the integration branch tip, not from a working tree, and SHALL refuse to run when the file is missing or invalid, naming the problem.

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
Every merge commit the tool creates SHALL name the upstream and its target commit in commit trailers. The tool SHALL derive the last synced commit of each upstream from these trailers on the integration branch and SHALL NOT store it in the configuration or outside the history. When an upstream's last synced commit is no longer contained in its branch, the tool SHALL report that upstream's history as rewritten and SHALL refuse to update it.

#### Scenario: Last synced commit
- **WHEN** the integration branch contains a merge of an upstream
- **THEN** `status` shows that upstream's target from the most recent such merge as its last synced commit

#### Scenario: Upstream force-pushed
- **WHEN** an upstream's branch no longer contains the last synced commit
- **THEN** `status` reports the upstream as rewritten and `start` refuses to update it

### Requirement: Fixed target and isolated merge
`start` SHALL accept upstream names and select every configured upstream when none is named. It SHALL fetch the selected upstreams, record each fetched upstream tip as that upstream's target and the current integration tip as the base, and queue, in configuration order, the selected upstreams whose target the base does not yet contain. It SHALL create a dedicated worktree on a new branch from the base and merge each queued target in turn with its own merge commit: at the repository root for an upstream without a path, and under the upstream's path otherwise. A target SHALL NOT change for the rest of the update. `start` SHALL refuse when an update is already in progress, when the main checkout or the integration branch has uncommitted changes, when the integration branch is behind its published counterpart, or when a selected upstream is unknown or rewritten.

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
- **WHEN** a queued upstream has a path that does not exist at the base and its history is not yet part of the integration branch
- **THEN** the tool imports the target's tree under that path with a merge commit that keeps the upstream's full history

#### Scenario: Path already holds unrelated content
- **WHEN** a queued upstream's path exists at the base but none of the upstream's history is part of the integration branch
- **THEN** `start` refuses and names the upstream and the path

### Requirement: Recorded resolutions
The tool SHALL enable git's recorded conflict resolution for the repository before merging, so a resolution made in one update or attempt is reapplied when the same conflict recurs.

#### Scenario: Retried update
- **WHEN** an aborted update is started again and hits a conflict resolved in the earlier attempt
- **THEN** git reapplies the recorded resolution and the tool reports the file as resolved by a recorded resolution

### Requirement: Resumable state
The tool SHALL keep the state of an update in the repository's common git directory, visible from every worktree, and every command SHALL act on that state: `continue` concludes a resolved merge and merges the remaining queued upstreams, `status` shows the step reached, each queued upstream as merged, in conflict or pending, and what comes next, and `abort` removes the worktree, the update branch and the state without touching the integration or any mirror branch.

#### Scenario: Resume after interruption
- **WHEN** the user returns to an update in a new shell
- **THEN** `status` names each upstream's target, the base, the worktree, the state of every queued upstream and the next command

#### Scenario: Continue after a conflict
- **WHEN** the user resolves a conflict and runs `continue` while other upstreams are pending
- **THEN** the tool concludes the merge and merges the pending upstreams in order, stopping again at the next conflict

#### Scenario: Abort
- **WHEN** the user aborts an update at any step before `finish`
- **THEN** the integration and mirror branches are as before `start` and no worktree or update branch remains

### Requirement: Verification of the published tree
`verify` SHALL run the configured install command and every check in the update worktree once every queued upstream is merged, write each command's output to a log, report pass or fail per command, and record the worktree commit it verified. A failed or skipped check SHALL leave the update unverified.

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
`finish` SHALL require a verified update whose worktree commit is the verified one and whose base is still the integration tip. It SHALL fast-forward the integration branch to the verified commit and each merged upstream's mirror branch, where one is configured, to that upstream's target, push them without force when a publish remote is configured, then remove the worktree, the update branch and the state. It SHALL never rewrite or force-push a branch, and SHALL never push to an upstream remote.

#### Scenario: Integration branch moved
- **WHEN** a commit lands on the integration branch after `start`
- **THEN** `finish` refuses and explains that the update must merge the new integration tip and be verified again

#### Scenario: Remote moved
- **WHEN** the publish remote's branch is ahead of the local one
- **THEN** the push is rejected, the local branches stay advanced, and the tool reports how to reconcile without force

#### Scenario: No publish remote
- **WHEN** no publish remote is configured
- **THEN** `finish` advances the local branches and pushes nothing

### Requirement: Update report
`report` SHALL summarise an update in progress or just merged, per upstream: the number of upstream commits merged, the conflicted files and whether each was resolved by hand or by a recorded resolution, and how the repository's own changes relative to that upstream grew or shrank.

#### Scenario: Report after merge
- **WHEN** the user runs `report` after resolving conflicts
- **THEN** the output lists, for each merged upstream, the merged range, each conflicted file with its resolution source, and the diff size against that upstream before and after

### Requirement: Upstream status
`status` SHALL report, for every configured upstream, its path, its branch, its last synced commit, its fetched tip, the number of commits the integration branch lacks, and one state: current, behind, not imported, rewritten, or unavailable when its remote or branch is missing. Without options it SHALL use the remote-tracking refs as last fetched. `status --check` SHALL first fetch every upstream, additionally list the files a merge of each behind upstream's tip would conflict in, create no worktree, branch or state, and exit with a failure status unless every upstream is current, so that it can run on a schedule and gate a cutover.

#### Scenario: Several upstreams
- **WHEN** `status` runs on a repository with three upstreams, two of them behind
- **THEN** it reports each upstream on its own line with its path and state, and the commit count for the two that are behind

#### Scenario: Scheduled check
- **WHEN** `status --check` runs on a repository whose single upstream is 30 commits ahead
- **THEN** it reports the 30 commits and the files that would conflict, exits with a failure status, and leaves the repository unchanged

#### Scenario: Zero drift
- **WHEN** `status --check` runs and every upstream is current
- **THEN** it reports every upstream as current and exits successfully

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
