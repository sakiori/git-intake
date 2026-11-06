## ADDED Requirements

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

## MODIFIED Requirements

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
