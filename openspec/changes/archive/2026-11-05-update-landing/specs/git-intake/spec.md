## MODIFIED Requirements

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

## ADDED Requirements

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
- **WHEN** `sync` finds the update's open merge request current
- **THEN** it predicts no conflicts and names no tag as taken

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

### Requirement: Landing with the CI job token
Where `CI_JOB_TOKEN` is set and a push to the publish remote would use it, as "Telling a push with the CI job token" tells it, `land` SHALL refuse with status 2 at its start, before it records a pause or changes any merge request, since its catch-up push needs a pipeline and a push made with the job token starts none. A refusal while a pause is recorded SHALL name `land --resume`. `land --resume`, which needs no pipeline, SHALL warn instead and go on.

#### Scenario: Land in a job that pushes with the job token
- **WHEN** `land` runs in a job whose push URL carries `CI_JOB_TOKEN`, with a merge request assigned to the queue
- **THEN** it exits with status 2, says that the push URL carries the job token, records no pause and changes no merge request

#### Scenario: Resume in a job that pushes with the job token
- **WHEN** `land --resume` runs in a job whose push URL carries `CI_JOB_TOKEN`, with a pause recorded
- **THEN** it warns of the job token and gives the held merge requests back, and when the push that removes the record fails, the record stays and it exits with status 2
