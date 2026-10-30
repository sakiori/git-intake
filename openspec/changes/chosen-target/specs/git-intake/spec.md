## MODIFIED Requirements

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
- **WHEN** the upstream's remote has no tag of the revision's name, and the revision resolves through a tag of the repository, by itself or in an expression
- **THEN** `start` refuses and names the repository's tag, and creates nothing

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
