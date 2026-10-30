## ADDED Requirements

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

## MODIFIED Requirements

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
