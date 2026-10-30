## ADDED Requirements

### Requirement: Forge of the publish remote
In merge-request mode the tool SHALL reach the forge of the publish remote's fetch URL, GitLab or GitHub, through its REST API, reading the publication setting from the configuration at the integration tip, never at an update's head. It SHALL take the forge from the setting `forge`; otherwise from the URL's host, `gitlab.com` as GitLab and `github.com` or a `ghe.com` subdomain as GitHub; otherwise from a GitLab CI job or GitHub Actions job whose server URL has that host.

#### Scenario: Forge recognised from the URL
- **WHEN** the publish remote's URL is `git@github.com:team/app.git` and merge-request mode is configured
- **THEN** the tool treats the publish remote as the GitHub repository `team/app`

#### Scenario: Unknown host
- **WHEN** the publish remote is on a host the tool cannot recognise, such as an SSH host alias, and the configuration sets no `forge`
- **THEN** the commands that need the forge refuse, push nothing, and name the settings `forge` and `api`

#### Scenario: Setting changed by an update
- **WHEN** an update's head changes the publication setting's `api` and the integration tip does not
- **THEN** the tool uses the setting at the integration tip

### Requirement: API base and project
The API base SHALL be the setting `api`, whose host SHALL be the remote's; else, in a CI job whose server has the remote's host, `CI_API_V4_URL` or `GITHUB_API_URL`; else `https://<host>/api/v4` for GitLab, `https://api.github.com` for `github.com`, `https://api.<host>` for `ghe.com` hosts and `https://<host>/api/v3` for other GitHub hosts. The project SHALL be the URL's path without credentials and `.git`, less the base's path before `/api/v4` or `/api/v3`.

#### Scenario: Self-managed GitLab in its own CI
- **WHEN** a GitLab CI job of a self-managed instance installed under a relative URL root runs the tool with the publish remote on that instance and no `api` setting
- **THEN** the tool reaches the API at `CI_API_V4_URL` for the project whose path is the remote's path without the relative URL root

#### Scenario: API on another host
- **WHEN** the setting `api` names a host other than the publish remote's
- **THEN** the tool refuses to reach the forge and names the setting

### Requirement: Forge token
The tool SHALL read the forge token from the environment only: `GITLAB_TOKEN` for GitLab; `GH_TOKEN`, then `GITHUB_TOKEN`, for `github.com` and `ghe.com` hosts; `GH_ENTERPRISE_TOKEN`, then `GITHUB_ENTERPRISE_TOKEN`, for another GitHub host, and in a GitHub Actions job on that host then `GH_TOKEN` and `GITHUB_TOKEN`. It SHALL refuse before pushing anything when none is set, naming the variables, and never print a token, store it, or send it to a host other than the API's, even on a redirect.

#### Scenario: No token
- **WHEN** merge-request mode is configured and none of the token variables of the forge is set
- **THEN** `finish` and `sync` refuse before pushing anything and name the variables

#### Scenario: Enterprise host outside Actions
- **WHEN** the publish remote is on a GitHub Enterprise Server host, only `GH_TOKEN` is set, and the tool runs outside GitHub Actions
- **THEN** the tool refuses and names `GH_ENTERPRISE_TOKEN` and `GITHUB_ENTERPRISE_TOKEN`

### Requirement: Forge requests
The tool SHALL ask GitHub for the API version `2022-11-28`. When the forge rejects the token or a request, the tool SHALL refuse with the forge's status and message, and before a push it SHALL push nothing. Messages about the update's publication SHALL use the forge's term, merge request or pull request.

#### Scenario: Token rejected
- **WHEN** the forge answers a request with an authentication or permission error
- **THEN** the tool refuses with the forge's status and message, and the token appears nowhere in its output

#### Scenario: Term of the forge
- **WHEN** `finish` opens the update's pull request on GitHub
- **THEN** its output speaks of a pull request

### Requirement: The update's merge request
The update's merge request SHALL be a merge request into the integration branch from `git-intake/update`, or from a branch `git-intake/update-<commit>` that an earlier release used. When a merge request from one of these branches into another branch is open, `finish` and `sync` SHALL refuse, pushing nothing, and name it.

#### Scenario: Merge request into another branch
- **WHEN** a merge request from `git-intake/update` into a branch other than the integration branch is open
- **THEN** `finish` and `sync` refuse, push nothing, and name the merge request and its target branch

### Requirement: GitLab merge settings
Before every push of an update branch to GitLab, the tool SHALL read the project and refuse, pushing nothing and naming each setting to change, unless `only_allow_merge_if_pipeline_succeeds` is on, `allow_merge_on_skipped_pipeline` is off, the merge method is `ff` or `rebase_merge`, merge trains are off, automatic rebase before merge is off or unknown to the instance, and the squash option is not `always`.

#### Scenario: Semi-linear history
- **WHEN** a GitLab project requires a successful pipeline, does not count a skipped one, merges with semi-linear history, uses no merge trains, rebases nothing automatically and squashes only on request
- **THEN** `finish` publishes the update

#### Scenario: Plain merge commits
- **WHEN** a GitLab project merges with the plain merge commit method, with merged results pipelines
- **THEN** `finish` and `sync` refuse, push nothing, and name the merge method to change

#### Scenario: Pipeline not required
- **WHEN** a GitLab project does not require a successful pipeline, or counts a skipped one as successful
- **THEN** `finish` and `sync` refuse, push nothing, and name the setting

#### Scenario: Rebasing merges
- **WHEN** a GitLab project rebases a merge request automatically before merging, or merges through merge trains
- **THEN** `finish` and `sync` refuse, push nothing, and say that a rebase drops the update's merge commits

### Requirement: GitHub merge settings
Before every push of an update branch to GitHub, the tool SHALL read the repository and the rules for the integration branch and refuse, pushing nothing and naming each setting to change, unless status checks naming at least one check are required with the branch up to date before merging, through the rules or the classic protection, no merge queue applies, merge commits are allowed, and no rule requires a linear history.

#### Scenario: Up-to-date requirement missing
- **WHEN** a GitHub repository's rules for the integration branch require status checks but not an up-to-date branch
- **THEN** `finish` and `sync` refuse, push nothing, and name the requirement to turn on

#### Scenario: Merge queue
- **WHEN** a merge queue applies to the integration branch
- **THEN** `finish` and `sync` refuse, push nothing, and say that the tool cannot confirm that a merge queue lands the tree it tested

#### Scenario: Linear history required
- **WHEN** a rule for the integration branch requires a linear history
- **THEN** `finish` and `sync` refuse, push nothing, and say that the update's merge commits cannot land

### Requirement: Unconfirmed GitHub merge settings
When the token cannot read what the GitHub merge settings require, as a protected branch's classic protection, its up-to-date checks, merge queue and linear history, without administration access, or whether merge commits are allowed without push access, the tool SHALL refuse, pushing nothing, say which requirement could not be confirmed, and name a ruleset, or the access the token lacks, as the ways to let it.

#### Scenario: Classic protection unreadable
- **WHEN** a GitHub repository requires up-to-date status checks only through classic branch protection, and the token cannot read it
- **THEN** `finish` and `sync` refuse, push nothing, say that the requirement could not be confirmed, and name a ruleset or administration read access

### Requirement: Merging the update's merge request
The description of the update's merge request SHALL say to merge it without squashing and without rebasing its branch, including with the forge's rebase or update-with-rebase buttons, because either drops the update's history, and to keep it current with `sync` instead.

#### Scenario: Squash on request
- **WHEN** a GitLab project's squash option is `default_off`, and `finish` opens the update's merge request
- **THEN** the description says to merge it without squashing or rebasing and names `sync` to keep it current

### Requirement: Update line
The update line of a head over a base SHALL lead from the head, through the commits the base lacks, stepping from a merge carrying `Intake-Catch-Up`, and from a head that merges into a commit the base contains without a sync record, as a merged results commit, to its second parent, and from every other commit to its first parent. The update's own commits SHALL be the sync merges, copies and commits changing a port upstream's journal on that line; a line without any carries no update.

#### Scenario: Upstream with sync records of its own
- **WHEN** an update merges an upstream whose own history carries sync records of an upstream with the same name
- **THEN** only the update's own sync merge counts for the update

#### Scenario: Port update of decisions only
- **WHEN** an update's port plan only skips commits or records them as present, so that its only commit changes the journal
- **THEN** the update line carries that update, and `env` sets the port upstream's variables

#### Scenario: Branch updated by the forge
- **WHEN** a person merged the integration branch into the merge request branch with the forge's update button
- **THEN** the update line follows the merge request branch's side, and the update's sync records count

### Requirement: Rewritten merge request branch
The tool SHALL treat the update's open merge request as rewritten when its head's update line lacks a sync record for a follow target its description records, or, without readable targets, carries no update. `sync` SHALL refuse to act on it, naming it, saying that withdrawing it with `abort --merge-request` declines its targets, and naming `start` and `finish` to publish them again.

#### Scenario: Rebased merge request
- **WHEN** the merge request branch was rebased with the forge's rebase button
- **THEN** `sync` refuses, names the merge request, `abort --merge-request`, `start` and `finish`, and pushes nothing

#### Scenario: Fix amended by force
- **WHEN** a person force-pushed the merge request branch to amend their own fix commit, keeping the sync merges
- **THEN** `sync` acts on the merge request as on any other

#### Scenario: Port copies kept by a rebase
- **WHEN** a merge request of an update with a follow and a port upstream was rebased, keeping the copies and dropping the sync merge
- **THEN** `sync` refuses as for any rewritten merge request

### Requirement: Merge request merged without its history
A merged update's merge request SHALL count as merged without its history when the integration branch does not contain its final head, as the forge reports it, or when the integration line lacks a sync record for a follow target its description records. When `sync` then proposes the same targets again, it SHALL warn, naming that merge request, and name it in the new merge request's description.

#### Scenario: Squashed merge request
- **WHEN** the most recently merged merge request of the update was squashed and no upstream moved since
- **THEN** `sync` opens a merge request for the same targets, warns that the squash dropped the sync records, and names the squashed merge request in the new description

### Requirement: Unattended synchronisation
`sync` SHALL take the repository's follow upstreams in without an update state. It SHALL refuse when an update is in progress, naming its step and the next command, and when the configuration has no publication setting. A conflict, or in push mode failed checks, SHALL stop it with the update in progress; any other end SHALL leave no update it created.

#### Scenario: Update in progress
- **WHEN** `sync` runs while an update is in progress
- **THEN** it refuses, names the update's step and the next command, and changes nothing

#### Scenario: Conflict in CI
- **WHEN** a follow upstream's merge conflicts during `sync`
- **THEN** `sync` stops with exit status 1, names the conflicted files, publishes nothing, and running `sync` in another clone while nothing moved stops on conflicts in the same files, after which `continue` and `finish` publish the update

#### Scenario: Refused catch-up
- **WHEN** `sync` refuses the catch-up of an open merge request because the integration branch took in one of its upstreams
- **THEN** no update remains in the clone, and the next `sync` refuses the same way

### Requirement: Integration branch for synchronisation
Like `start`, `sync` SHALL read the configuration at `HEAD`, fetch the publish remote, fast-forward a local integration branch strictly behind the remote's, refuse when the two diverged, and read the configuration again at the new tip. It SHALL create a missing local integration branch at the remote's. In merge-request mode it SHALL refuse when the local integration branch holds commits the remote's lacks.

#### Scenario: Clean CI clone
- **WHEN** `sync` runs in a CI job whose clone has `HEAD` detached at a commit of the integration branch and no local integration branch
- **THEN** it creates the local integration branch at the publish remote's and carries on as in a clone where that branch exists

#### Scenario: Unpublished local commits
- **WHEN** `sync` runs in merge-request mode while the local integration branch holds a commit the publish remote lacks
- **THEN** it refuses, names the commit count, and changes nothing

### Requirement: Upstreams sync cannot take
`sync` SHALL leave out every follow upstream for which `start` would refuse when it is the only one selected, as one it cannot fetch, without a release, rewritten, or whose path cannot take it, name it with the reason, and take the others. It SHALL leave out port upstreams and name those with pending commits.

#### Scenario: Unreachable upstream
- **WHEN** one of two follow upstreams cannot be fetched and the other is behind
- **THEN** `sync` publishes the update of the other, names the unreachable one, and exits with status 1

#### Scenario: Port upstream pending
- **WHEN** `sync` runs while a port upstream has pending commits and no follow upstream is behind
- **THEN** it starts nothing, names the port upstream and its pending commits, and exits with status 0

### Requirement: Exit status of synchronisation
`sync` SHALL exit with status 2 when it refused or failed, 1 when it stopped on a conflict or failed checks or left out a follow upstream it could not take, and 0 otherwise.

#### Scenario: Nothing behind
- **WHEN** `sync` runs and every follow upstream is current, with no merge request open
- **THEN** it creates nothing, says that every follow upstream is current, and exits with status 0

#### Scenario: Concurrent runs
- **WHEN** two `sync` runs push the update branch at the same time and the second push is rejected as not a fast-forward
- **THEN** the second run exits with status 2, says to run it again, and opens no merge request

### Requirement: Synchronising an open merge request
In merge-request mode `sync` SHALL run no install command and no check. With the update's merge request open, it SHALL fetch the merge request's head and apply "Rewritten merge request branch" first. When the head contains the integration tip, it SHALL change nothing, say that the update waits for the merge request, and name the upstreams that moved on since, which the next update takes.

#### Scenario: Run again without change
- **WHEN** `sync` runs while the update's merge request is open and the integration branch did not move
- **THEN** it pushes nothing, changes no merge request, says that the update waits for the merge request, and exits with status 0

#### Scenario: Upstream moved while the merge request waits
- **WHEN** an upstream moves on while its update's merge request is open and the integration branch did not move
- **THEN** `sync` pushes nothing, names the upstream as moved on, and the next `sync` after the merge takes the new commits in a new merge request

### Requirement: Catching up an open merge request
When the head of the update's open merge request does not contain the integration tip, `sync` SHALL carry the merge request's update on locally, catch it up with the integration tip under the catch-up preconditions without running the checks, and publish it as `finish` does in merge-request mode.

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

### Requirement: Synchronising without an open merge request
In merge-request mode with the update's merge request not open, `sync` SHALL start an update of every follow upstream the integration branch lacks the target of, the fetched tip or, with `tags`, the newest release, and publish it as `finish` does in merge-request mode once every merge concluded, without running the install command or the checks.

#### Scenario: Merge request opened by CI
- **WHEN** a scheduled CI job runs `sync` while an upstream is behind and no merge request is open
- **THEN** the update merges the upstream, `git-intake/update` holds it on the publish remote, a merge request from it into the integration branch exists, no install command or check ran, no update remains in the clone, and the job exits with status 0

### Requirement: Synchronisation by push
In push mode, `sync` SHALL start an update of every follow upstream the integration branch lacks the target of, as in merge-request mode, then run `verify`, and when the checks pass run `finish`. Failed checks SHALL stop it with exit status 1, the update in progress and unverified, and nothing published.

#### Scenario: Push mode sync
- **WHEN** `sync` runs in push mode while an upstream is behind, and the merge and the checks succeed
- **THEN** the publish remote's integration branch holds the verified update, no update remains, and `sync` exits with status 0

#### Scenario: Checks fail in push mode
- **WHEN** the checks fail during `sync` in push mode
- **THEN** `sync` exits with status 1, publishes nothing, and leaves the update in progress, unverified

### Requirement: Declined update targets
When the update's merge request is not open and the one closed or merged most recently was closed without merging, `sync` SHALL read the follow targets recorded in its description. When it recorded every follow target `sync` would merge, it SHALL start nothing, name the closed merge request, say that it declined these targets, and exit with status 0. A description without readable targets SHALL decline nothing.

#### Scenario: Merge request closed without merging
- **WHEN** a person closed the update's merge request without merging and no upstream moved since
- **THEN** the next `sync` opens no merge request, names the closed one, and exits with status 0

#### Scenario: Upstream moved after a decline
- **WHEN** an upstream of a declined merge request moves on
- **THEN** the next `sync` opens a merge request with the new target

#### Scenario: Update by hand after a decline
- **WHEN** a person runs `start` and `finish` for the declined targets
- **THEN** the update is published and its merge request opened as for any other

### Requirement: Update branch without an open merge request
With `git-intake/update` at the fetch URL and no merge request open from it, `finish` and `sync` SHALL treat the branch as left over when it holds only catch-up merges beyond the integration tip or a closed or merged merge request's head, and delete it on each push URL while it is at the commit read. Otherwise `sync` SHALL carry on a branch holding an update, except as "Declined branch" requires, `finish` SHALL push over one its head contains, and both SHALL refuse any other.

#### Scenario: Branch kept after a decline
- **WHEN** a pull request from `git-intake/update` was closed without merging, GitHub kept its branch, and an upstream moved on
- **THEN** the next `sync` deletes the branch, pushes the new update, and opens a pull request

#### Scenario: Interrupted job
- **WHEN** a CI job pushed `git-intake/update` and was killed before the merge request was opened
- **THEN** the next `sync` in a fresh clone opens the merge request for that branch without starting another update

#### Scenario: Catch-up left after a decline
- **WHEN** a catch-up merge was pushed to `git-intake/update` just after its merge request was closed, and the branch was not deleted
- **THEN** the next `sync` deletes the branch and opens no merge request for the declined targets

#### Scenario: Branch of unknown origin
- **WHEN** `git-intake/update` holds commits that carry no update and no merge request of the update points at
- **THEN** `finish` and `sync` refuse, push nothing, and name the branch, the push URL, and `abort --merge-request` as the way to delete it after fetching what should be kept

### Requirement: Declined branch
When every follow target on the update line of a branch `sync` would carry on was recorded by the merge request that "Declined update targets" would apply, `sync` SHALL refuse with status 2, naming the branch and `abort --merge-request`, which deletes the branch.

#### Scenario: Declined branch with a fix
- **WHEN** the branch of a declined pull request holds a fix a person pushed after the decline, and `sync` runs
- **THEN** `sync` refuses with exit status 2, pushes nothing, and names the branch and `abort --merge-request`

### Requirement: Carrying on a merge request's update
To carry on the update a merge request or its branch holds, `sync` SHALL put the update branch at its head in the update worktree, the base at the head's merge base with the integration tip, and queue as merged or carried out each upstream whose sync merge, copies or journal change is on the update line: a follow upstream at its newest sync record's target, with the last synced commit at the base as its previous one, and a port upstream with the commits its copies take and its journal decides.

#### Scenario: Status of a carried-on update
- **WHEN** `sync` stopped on a catch-up conflict of a merge request opened by another clone, and `status` runs
- **THEN** it shows the catch-up step, the upstreams of the merge request's update with their targets, and `continue` as the next command

#### Scenario: Abort of a carried-on update
- **WHEN** `abort` runs on an update carried on from a merge request
- **THEN** the local update is removed, the merge request stays open, and `abort` names `abort --merge-request` to withdraw it

### Requirement: Commands on a carried-on update
An update carried on from a merge request SHALL record that merge request and be recorded as started by the release that carried it on; `finish` SHALL refuse with status 2 to publish it once that merge request is no longer open, keeping the update and naming the merge request and `abort`, and every command SHALL act on it as on an update `start` began. `report` SHALL show what history holds of it and say that the conflicts of the steps taken before it was carried on are not known.

#### Scenario: Report of a carried-on update
- **WHEN** `report` runs on an update carried on from a merge request whose upstream merge had conflicts
- **THEN** it lists the upstreams with their merged ranges and says that the conflicts of the earlier steps are not known

### Requirement: Publishing an update as a merge request
In merge-request mode `finish` SHALL move no local branch, catch up with the publish remote's integration branch when it moved past the base, without running the checks, require the merge settings, push the update head without force to the update's branch on the publish remote, and through the API open the update's merge request from that branch, or use the one open. On GitLab it SHALL ask for squashing off and the branch removed on merge.

#### Scenario: Merge request opened
- **WHEN** `finish` runs in merge-request mode on an update whose merges concluded, against a GitLab project whose merge settings qualify
- **THEN** the branch `git-intake/update` holds the update head on the publish remote, a merge request from it into the integration branch exists with squashing off, the local integration branch has not moved, and `finish` exits successfully

#### Scenario: Pull request opened
- **WHEN** the publish remote is a GitHub repository whose merge settings qualify
- **THEN** `finish` opens a pull request from `git-intake/update` into the integration branch and names it as a pull request

#### Scenario: Integration branch moved before finish
- **WHEN** a commit lands on the publish remote's integration branch after `start`, and `finish` runs in merge-request mode
- **THEN** `finish` catches up with it without running the checks and pushes the catch-up merge as the merge request's head

#### Scenario: Merge request already open for this update
- **WHEN** `finish` runs on an update carried on from an open merge request, after its catch-up conflict was resolved
- **THEN** `finish` pushes the catch-up merge to the same branch without force and opens no second merge request

#### Scenario: Update already on the integration branch
- **WHEN** the publish remote's integration branch already contains the update head when `finish` runs in merge-request mode
- **THEN** `finish` pushes nothing, opens no merge request, removes the update, and says that the update already landed

### Requirement: Merge request branch on the remote
`finish` SHALL refuse, pushing nothing and keeping the update, when the update's open merge request has a head the update head does not contain, naming it, `abort` followed by `sync` to carry it on, and `abort --merge-request` to withdraw it, which also removes the local update and declines the merge request's targets until `start` and `finish` publish them again. It SHALL push only where the branch is not at the update head, and never rewrite or force-push it.

#### Scenario: Merge request of another update open
- **WHEN** an update started with `start` reaches `finish` while the update's merge request is open with a head the update head does not contain
- **THEN** `finish` pushes nothing, names the merge request, `abort` followed by `sync`, and `abort --merge-request`, and keeps the update

### Requirement: Merge request state before the push
Just before `sync` pushes to an open merge request's branch, it SHALL read the merge request's state and head and the remote's integration branch again, and push nothing unless all are as read before: a merge request merged or closed meanwhile SHALL end the run with status 0, and any other change with status 2 and the advice to run again. Right after the push it SHALL read the state again and, when the merge request is no longer open, delete the branch while it is at the pushed commit.

#### Scenario: Merged during a catch-up
- **WHEN** the merge request is merged while `sync` catches it up, before the push
- **THEN** `sync` pushes nothing, opens no merge request, says that the merge request was merged, and exits with status 0

### Requirement: Changes to the update's own checks
When an update changes the forge's CI definition, `.github/workflows/` on GitHub or the project's CI configuration file on GitLab, or the install command or checks of `.git-intake.toml`, `finish` and `sync` SHALL name those changes in their output and in the merge request's description, since the pipeline that verifies the update then runs what the update brings.

#### Scenario: Upstream changes the pipeline
- **WHEN** a root upstream's update changes `.gitlab-ci.yml`
- **THEN** `sync` names the change in its output and in the merge request's description

### Requirement: Title and description of the merge request
With every push of an update for a merge request, the tool SHALL set the title, naming the updated upstreams, and the description, listing each follow upstream's merged range with its target's tag and each port upstream's taken commits and decisions, and ending with a block, hidden when rendered, recording each follow target; a description too long for the forge SHALL be shortened before that block. On GitLab it SHALL ask again for squashing off. Title and description SHALL belong to the tool.

#### Scenario: Description after a catch-up
- **WHEN** a catch-up merge is pushed to an open merge request whose description a person edited
- **THEN** the merge request's title and description describe the update at its new head, with the hidden block of its targets

### Requirement: Local update after publication
Only after the merge request exists SHALL `finish` remove the update worktree, the update branch, the update's upstream references and the state, name the merge request with its address, and exit successfully; the merge request carries the update from then on. A `finish` interrupted before the merge request exists SHALL keep the update, and run again SHALL push only what is missing and open the merge request.

#### Scenario: Interrupted before the merge request
- **WHEN** `finish` pushed the branch and stopped before the merge request was opened, and runs again
- **THEN** it pushes nothing, opens the merge request, and removes the update

#### Scenario: Merged and the next update
- **WHEN** the merge request is merged with a merge commit or by fast-forward, and `start` runs
- **THEN** `start` fast-forwards the local integration branch to the publish remote's, and `status` shows every merged upstream's target as its last synced commit

### Requirement: Withdrawing a merge request
`abort --merge-request` SHALL remove a local update in progress, then close through the API the update's open merge request, and delete the update's branch on each push URL while it points at the commit read, open merge request or not, reporting what remains; with neither it SHALL say that there is nothing to withdraw. `abort` without it SHALL never close a merge request, and with no update in progress SHALL name `abort --merge-request` when one is open, or say that it could not read the forge.

#### Scenario: Withdraw the merge request
- **WHEN** a merge request opened by `sync` is open and `abort --merge-request` runs on a clone with no update in progress
- **THEN** the merge request is closed, `git-intake/update` is gone from the publish remote, and the integration branch is unchanged

#### Scenario: Branch moved by someone else
- **WHEN** the merge request branch points elsewhere than the merge request's head as read before `abort --merge-request` deletes it
- **THEN** it closes the merge request, keeps the branch, and names it

#### Scenario: Interrupted withdrawal
- **WHEN** `abort --merge-request` closed the merge request and stopped before deleting the branch, and runs again
- **THEN** it deletes the branch the closed merge request left

#### Scenario: Withdraw while carrying on
- **WHEN** `abort --merge-request` runs while an update carried on from the open merge request stopped on a catch-up conflict
- **THEN** the local update is removed, the merge request is closed, and its branch is deleted

### Requirement: Update variables from history
`env` SHALL set, from history alone, the variables `verify` sets for a commit, `HEAD` unless a revision is named, with the same values. The base SHALL be the commit's merge base with the integration tip. The upstream variables SHALL come from the sync records and copies on the update line, the journal changes after the base, and the integration line at the base.

#### Scenario: Merge request pipeline
- **WHEN** a pipeline job of an update's merge request runs `git intake env -- ./check.sh` on the merge request's head
- **THEN** `check.sh` has the base, head, integration branch, upstream names, modes, targets and previous commits that `verify` gives its checks for that commit

#### Scenario: Merged results pipeline
- **WHEN** the job runs on a commit that merges the update's head into the integration tip, as a merged results pipeline does
- **THEN** `GIT_INTAKE_BASE` is that integration tip, `GIT_INTAKE_HEAD` is the merge, and the upstream variables are those of the update

### Requirement: Integration tip of the update variables
The integration tip of `env` SHALL be the publish remote's integration branch, fetched for the command, or, when the configuration names no publish remote, the local integration branch. `env` SHALL refuse when the fetch fails, as for a configured remote the clone does not define.

#### Scenario: Publish remote unreachable
- **WHEN** `env` runs while the publish remote cannot be fetched
- **THEN** it refuses with git's reason and sets no variable, and `env -- <command>` does not run the command

### Requirement: Commit without an update
When the integration tip contains the commit, or its update line carries no update, `env` SHALL set no variable, say on standard error that the commit carries no update, and exit with status 0.

#### Scenario: Not an update
- **WHEN** `env` runs on a commit of the integration branch or of a branch whose update line holds no sync merge, copy or journal change
- **THEN** it prints no variable, says that the commit carries no update, exits with status 0, and `env -- <command>` runs the command without any `GIT_INTAKE_` variable

### Requirement: Output of the update variables
By default `env` SHALL print one `export` assignment per variable, quoted for the POSIX shell; with `--dotenv` one `NAME=value` line per variable, unquoted; and with `--json` one JSON object. Given a command after `--`, it SHALL run the command with the variables, without inherited `GIT_INTAKE_` variables, and exit with the command's status, or with status 2 without running it when it cannot read the variables.

#### Scenario: Command with the variables
- **WHEN** `git intake env -- make check` runs on an update's head
- **THEN** `make check` runs with the update's variables and `env` exits with its exit status

#### Scenario: Variables unreadable
- **WHEN** `git intake env -- make check` runs in a shallow clone
- **THEN** `make check` does not run and `env` exits with status 2

#### Scenario: Dotenv lines
- **WHEN** `git intake env --dotenv` runs on an update's head of two upstreams
- **THEN** each line is `NAME=value` with the value unquoted, and `GIT_INTAKE_UPSTREAMS` holds both names separated by a space

### Requirement: Shallow repository
Every command other than `abort` and `migrate` SHALL refuse in a shallow repository before it changes anything, saying that the integration line needs the full history and naming `git fetch --unshallow`.

#### Scenario: Shallow CI clone
- **WHEN** `sync` or `env` runs in a clone made with a limited depth
- **THEN** it refuses, changes nothing, and names how to fetch the full history

### Requirement: Merge request in status
In merge-request mode `status` SHALL show the update's open merge request with its number, address and head and whether the head contains the integration tip, and the newest merge request of the update merged without its history. When it cannot read the forge, it SHALL say why and keep its exit status. `status --json` SHALL carry `merge_request` with `number`, `url`, `head` and `current`, and `merged_without_history` with `number` and `url`, each or null.

#### Scenario: Status with an open merge request
- **WHEN** `status` runs while the update's merge request is open and the integration branch moved past its head
- **THEN** it shows the merge request's number, address and head, says that it is behind the integration branch, and names `sync`

#### Scenario: Status after a squash
- **WHEN** the newest merged merge request of the update was squashed
- **THEN** `status` names it as merged without its history, and `status --json` carries it as `merged_without_history`

### Requirement: Forge checks in doctor
In merge-request mode `doctor` SHALL check that the forge is recognised, that a token is set and accepted, and that the merge settings qualify, failing a check that does not hold and warning when squash or rebase merges stay selectable.

#### Scenario: Doctor with unqualified settings
- **WHEN** `doctor` runs in merge-request mode on a GitLab project that does not require a successful pipeline
- **THEN** the merge settings check fails and names the setting

### Requirement: Update waiting from an earlier release
The next `finish` SHALL hand an update of state format 4 or 5 that waits for its merge request over to the merge request: find it through the API by its recorded number or else by its branch, say whether it is open, merged or closed, record the resolutions of a catch-up step, and remove the update worktree, branch, upstream references and state, keeping the merge request and its branch.

#### Scenario: Hand-over of an open merge request
- **WHEN** `finish` runs on an update of format 5 waiting for its open merge request
- **THEN** no update remains locally, the merge request and its branch are unchanged, and `sync` afterwards finds it open

#### Scenario: Hand-over after the merge
- **WHEN** `finish` runs on such an update whose merge request was merged
- **THEN** no update remains locally, and the next `start` brings the local integration branch up to date

### Requirement: Other commands on a waiting update of an earlier release
`abort --merge-request` SHALL withdraw the open merge request of an update of an earlier release that waits for it and remove the update; `abort` SHALL remove the update and keep the merge request. Every other command that reads such an update SHALL show it as waiting from an earlier release and name `finish`.

#### Scenario: Abort of a waiting update of an earlier release
- **WHEN** `abort --merge-request` runs on an update of format 5 waiting for its open merge request
- **THEN** the merge request is closed, its branch is deleted, and no update remains locally

#### Scenario: Abort after the merge
- **WHEN** `abort` runs on such an update whose merge request was merged
- **THEN** it removes the update and says that the merge request was merged

## REMOVED Requirements

### Requirement: Publication through a merge request
**Reason**: The merge request is reached through the forge's API instead of git push options, its pipeline verifies the update, and the merge request carries the update once published; the local waiting step and the judgement of what landed by trees go away.
**Migration**: See "Publishing an update as a merge request", "Merge request branch on the remote", "Local update after publication", "Synchronising an open merge request" and "Update waiting from an earlier release".

## MODIFIED Requirements

### Requirement: Fork configuration
The tool SHALL read a repository's settings from a `.git-intake.toml` file committed on the integration branch: the integration branch, an optional publication setting with the publish remote, the mode `push` or `merge-request`, and, for merge-request mode, an optional forge `gitlab` or `github` and an optional API base URL, an install command, an ordered list of check commands, and an ordered list of upstreams. Each upstream SHALL have a unique name, a URL, a branch, a mode `follow` or `port` with `follow` as the default, optionally a path inside the repository, and, in `follow` mode, optionally a `tags` pattern: a glob over the upstream's tag names that selects its release tags. No two upstreams SHALL have the same URL and branch. An upstream without a path is taken at the repository root. The tool SHALL read the committed file from the integration branch tip, not from a working tree, and SHALL refuse to run when the file is missing or invalid, naming the problem. It SHALL refuse the settings `remote` and `mirror`, naming what replaces them, an upstream in `port` mode with a path or a `tags` pattern, a `tags` pattern that is not a valid glob or that starts with `refs/` or `tags/`, a forge other than `gitlab` or `github`, an API base that is not an `https` URL, and a publish remote whose fetch or push URL names a configured upstream's repository.

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

#### Scenario: Forge setting
- **WHEN** the publication setting names the forge `gitea`, or an API base `http://forge.example/api/v4`
- **THEN** the tool refuses to run and names the setting and the values it accepts

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

### Requirement: Resumable state
The tool SHALL keep the state of an update in the repository's common git directory, visible from every worktree, and the update worktree at `.worktrees/git-intake/update` in the main worktree, never inside the git directory. Unless that path is ignored already, `start` and `sync` SHALL add the line `/.worktrees/` to the repository's `info/exclude` once, without changing a committed ignore file, so that the main checkout does not show the update worktree. Every command SHALL act on that state: `continue` carries out a plan, concludes a resolved merge, copy or catch-up merge, and goes on with the remaining queued upstreams or planned commits; `status` shows the step reached, each queued upstream as merged, at its plan, in conflict or pending, a catch-up merge in conflict or, where verification is required, awaiting it, and what comes next; and `abort` removes the worktree, the update branch, the plan, the update's upstream references, its logs and the state without touching the integration branch. A `follow` upstream SHALL count as merged only when the history of the update branch head carries its sync record for its target, and a `port` upstream as carried out only when its plan has been carried out to the journal commit. Once `finish` has begun moving branches, `abort` SHALL refuse and name `finish` as the way to complete the update. The tool SHALL read an update state of format 4 as its own; the first command that changes the update SHALL write it in format 5 and say that releases before v0.3.6 can no longer carry it on, and the first of `continue`, `verify` and `finish` SHALL warn once that an earlier release started the update, as for another release of the same format. It SHALL refuse an update state in any other format than its own and format 4, older or newer, or without a format, name both formats and, where the state records it, the release that started the update, and advise finishing or aborting the update with a release that keeps that format, naming the releases of each format (1: v0.1.0; 2: v0.2.0 to v0.2.2; 3: v0.3.0 to v0.3.1; 4: v0.3.2 to v0.3.5; 5: from v0.3.6), or for a newer format with the release that started it or a later one that reads it. For format 3, the refusal SHALL name `migrate` first. Only `start` and `sync` SHALL record the release that started the update in the state. The first of `continue`, `verify` and `finish` run by another release of the same format SHALL warn once, without refusing, and record that it warned; it SHALL advise aborting and starting again only before `finish` has moved a branch or pushed the update.

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
- **THEN** `abort`, `start` and `sync` refuse and `doctor` fails, each naming that worktree and saying to remove or switch it, and the worktree and its uncommitted changes are kept

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
- **THEN** the first of `continue`, `verify` and `finish` it runs warns once that another release started it, advises `abort` only before `finish` has moved a branch or pushed the update, and every command carries on

### Requirement: Environment of the checks
`verify` SHALL run the install command and every check with environment variables that describe the update, naming commits by their full ids: `GIT_INTAKE_BASE`, the update's base, which is the integration tip the update started from or, after a catch-up merge, the integration tip it caught up with; `GIT_INTAKE_HEAD`, the commit being verified; `GIT_INTAKE_INTEGRATION`, the integration branch's name; and `GIT_INTAKE_UPSTREAMS`, the names of the queued upstreams separated by spaces, in queue order. For each queued upstream, with `<NAME>` its name in upper case and every character other than a letter or a digit replaced by `_`, it SHALL set `GIT_INTAKE_UPSTREAM_<NAME>_MODE` to `follow` or `port` and `GIT_INTAKE_UPSTREAM_<NAME>_TARGET` to the follow target, or for a port upstream to the newest upstream commit the update takes or decides on, with no target when it takes and decides none. For a follow upstream with a last synced commit before the update, it SHALL set `GIT_INTAKE_UPSTREAM_<NAME>_PREVIOUS` to that commit; a port upstream SHALL get no such variable. A catch-up merge SHALL change none of the upstream variables. The verification that `finish` runs after a catch-up SHALL set the same variables. Every commit named SHALL be reachable from a reference while the commands run. Variables with the `GIT_INTAKE_` prefix that the tool inherits SHALL NOT reach the commands, and the tool SHALL NOT set these variables for the git commands it runs itself.

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
- **WHEN** an update carries out a port plan whose newest commits are left `later`
- **THEN** the commands receive the mode `port` and, as the target, the newest commit the plan takes or decides on, and no previous commit

#### Scenario: After a catch-up
- **WHEN** the checks run on a catch-up merge
- **THEN** `GIT_INTAKE_BASE` is the integration tip the update caught up with, `GIT_INTAKE_HEAD` is the catch-up merge, and every upstream variable is as before the catch-up

#### Scenario: Inherited variables
- **WHEN** `verify` is run with `GIT_INTAKE_BASE` or another `GIT_INTAKE_` variable already set
- **THEN** the commands receive only the variables of the update, with the update's values

### Requirement: Guarded publication
In push mode, and with `--no-publish`, `finish` SHALL require a verified update whose worktree commit is the verified one, or one whose head is a catch-up merge the tool made and has not verified, which it verifies itself; any other unverified head SHALL be refused until `verify` passes. In merge-request mode without `--no-publish` it SHALL require no verification. In `push` mode, unless `--no-publish` is given, it SHALL first fetch the publish remote and refuse with git's reason when the fetch fails: when the remote's integration branch holds commits the local one lacks and the local one holds none the remote lacks, it SHALL bring the local integration branch up to date as `start` does, which does not count as `finish` moving it; when each holds commits the other lacks, it SHALL refuse before moving anything and say that the local commits must be published or reconciled first. When the integration tip contains the update head, `finish` SHALL go on without a catch-up. The integration tip SHALL be the local integration branch's in `push` mode and the publish remote's in `merge-request` mode. When the integration tip is otherwise no longer the base, `finish` SHALL catch up with it, and where it requires verification, verify the result as `verify` does and go on only when the verification passes; it SHALL make at most one catch-up merge per run, and when the integration branch moves again meanwhile it SHALL stop and say to run `finish` again. In push mode and with `--no-publish`, before moving any branch it SHALL confirm that the move of the integration branch is a fast-forward, that the verified commit contains the base and every merged target, and that no checkout of the integration branch would lose changes. It SHALL record the planned move, fast-forward the integration branch to the verified commit, push exactly the planned commit without force when the publication mode is `push`, and only after publication succeeds remove the worktree, the update branch, the update's upstream references and the state. An interrupted or failed `finish` SHALL complete the remaining move and the push when run again, and SHALL never publish a commit other than the planned one. When the push was rejected because the publish remote's integration branch moved past the base, a `finish` run again SHALL catch up with the remote's branch from the finishing step, verify the result, fast-forward the local integration branch to it and push it; when the local integration branch moved past the planned commit meanwhile, it SHALL make no catch-up and name `finish --no-publish`. `finish --no-publish` SHALL not fetch, SHALL catch up only with the local integration branch, SHALL in the finishing step land a verified catch-up merge locally, and drop an unverified or conflicted one, recording the resolutions made so far and naming the commits it drops, closing the update at the planned commit, complete the local move and remove the worktree, the update branch, the upstream references and the state without pushing, and report that the integration branch remains unpublished. It SHALL never rewrite or force-push a branch, and SHALL never push to an upstream's URL.

#### Scenario: Integration branch moved
- **WHEN** a commit lands on the integration branch after `start` and `finish` runs on the verified update in push mode
- **THEN** `finish` catches up with the new tip, runs the install command and the checks on the catch-up merge, fast-forwards the integration branch to it, publishes it, and afterwards `status` shows every merged upstream's target as its last synced commit

#### Scenario: Catch-up that conflicts in finish
- **WHEN** the commit that landed conflicts with the update
- **THEN** `finish` stops with exit status 1 in the catch-up step and moves no branch to the update, and after the conflict is resolved and `continue` has run, `finish` verifies and publishes

#### Scenario: Checks fail after a catch-up
- **WHEN** the checks fail on the catch-up merge in push mode
- **THEN** `finish` stops with the report of `verify`, the update stays unverified at the catch-up merge, and no branch moves to the update

#### Scenario: Commit after a catch-up
- **WHEN** a commit is added on top of a catch-up merge before it is verified, in push mode
- **THEN** `finish` refuses until `verify` passes for the new commit

#### Scenario: Integration branch moves during verification
- **WHEN** another commit lands on the integration branch while `finish` verifies a catch-up merge
- **THEN** `finish` moves no branch to the update and says to run it again, which catches up once more

#### Scenario: Integration branch already holds the update
- **WHEN** the integration branch was moved to the verified commit, or past it, by other means
- **THEN** `finish` makes no catch-up merge, counts the move as done, pushes the verified commit in push mode, and completes the update

#### Scenario: Remote moved
- **WHEN** the publish remote's integration branch is ahead of the local one, which holds no commit the remote lacks, and `finish` runs in push mode
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
- **WHEN** the publication mode is `merge-request` and the update was never verified
- **THEN** `finish` publishes the update as a merge request, runs no check, and moves no local branch

### Requirement: Conditions for starting an update
`start` SHALL refuse when an update is in progress, when the main checkout or the integration branch has uncommitted changes, when the publish remote cannot be fetched, when the integration branch and its published counterpart each hold commits the other lacks, when a selected upstream is unknown, named more than once, or rewritten, or, in merge-request mode, when the forge cannot be reached as `finish` needs it, when the local integration branch holds commits its published counterpart lacks, or when an update's merge request into the integration branch is open, naming it, `sync` to carry it on and `abort --merge-request` to withdraw it. When the integration branch is strictly behind its published counterpart, `start`, having read the integration branch and the publish remote from the configuration at `HEAD`, SHALL fast-forward it to that counterpart, moving a checkout of the branch with `git merge --ff-only` and the branch otherwise with `git update-ref` against its old value, refusing when a checkout would lose changes, and say that it did. It SHALL then read the configuration again at the new tip and fix the base there, or refuse, keeping the fast-forward, when that configuration names another integration branch or publish remote.

#### Scenario: Update already in progress
- **WHEN** `start` runs while an update is in progress
- **THEN** it refuses, names the step the update has reached, and creates nothing

#### Scenario: Forge unreachable
- **WHEN** `start` runs in merge-request mode without a token for the forge
- **THEN** it refuses, names the token variables, and creates nothing

#### Scenario: Merge request open
- **WHEN** `start` runs in merge-request mode while the update's merge request is open
- **THEN** it refuses, names the merge request, `sync` and `abort --merge-request`, and creates nothing

#### Scenario: Integration branch behind the publish remote
- **WHEN** the publish remote's integration branch holds commits the local one lacks, and the local one holds none the remote lacks
- **THEN** `start` fast-forwards the local integration branch to the remote's, says so, reads the configuration again at the new tip, and starts the update with that tip as its base

#### Scenario: Integration branch diverged from the publish remote
- **WHEN** the publish remote's integration branch and the local one each hold commits the other lacks
- **THEN** `start` refuses, names how many commits each side lacks, moves no branch and creates nothing

### Requirement: Machine-readable output
`status`, `report`, `log`, `branches`, `doctor` and `env` SHALL accept `--json` and then write a single JSON document to standard output that carries the same facts as the text form, under field names that are stable across releases, with the same exit status. A rewritten `port` upstream SHALL carry the base and the commits that are off its line.

#### Scenario: Agent reads status
- **WHEN** an agent runs `status --json`
- **THEN** it receives, for every upstream, its name, path, branch, mode, fetched tip and state, for a `follow` upstream its last synced commit and commit count, and for a `port` upstream its base and its numbers of taken, decided and pending commits, without parsing text

#### Scenario: Agent reads a rewritten port upstream
- **WHEN** an agent runs `status --json` for a rewritten `port` upstream
- **THEN** it receives the state `rewritten` and each taken or decided commit off the upstream's line that is not retired, with what it was recorded as

#### Scenario: Agent reads the update variables
- **WHEN** an agent runs `env --json` on an update's head
- **THEN** it receives an object whose keys are the variable names and whose values are their values, and an empty object for a commit that carries no update

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
- **THEN** `migrate` drops the verification, and in push mode `finish` refuses until `verify` runs again

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
- **WHEN** a commit is added to the update branch after `verify`, in push mode
- **THEN** `finish` refuses until `verify` passes for the new commit

#### Scenario: Upstreams still pending
- **WHEN** `verify` runs while a queued upstream is at its plan, in conflict or pending
- **THEN** the tool refuses and names the upstreams that still need `continue`

### Requirement: Lifetime of upstream references
A command SHALL fetch upstreams into references of its own that no other command uses, and SHALL remove them before it exits, whatever its outcome. An update SHALL hold `refs/git-intake/upstreams/<name>` for each queued upstream at the tip `start` or `sync` fetched, or, for an update carried on from a merge request, at the upstream's target or, for a port upstream, at the newest commit the update takes or decides on; only `start` and `sync` SHALL write it and only `finish`, `abort` and `sync` SHALL remove it. A command that acts on the update in progress SHALL restore such a reference that is missing, from the update state, when its commit is in the repository.

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

### Requirement: Integration line
The tool SHALL find its sync records, and the merges that brought a path upstream's history into its path, on the integration line: the first-parent line of the integration branch, where each merge on it is followed by the commits that `git rev-list --first-parent <merge>^1..<merge>^2` lists for it. A merge among those commits that carries the trailer `Intake-Catch-Up` SHALL in turn be followed by the first-parent line of its second parent, down to the first commit that the first parent of the merge on the first-parent line contains, at any depth; other merges among those commits SHALL contribute nothing further. An update SHALL count the same whether it was published by push or through a merge request merged by fast-forward or with a merge commit, and whether or not it caught up with the integration branch.

#### Scenario: Merge request merged with a merge commit
- **WHEN** an update's merge request is merged with a merge commit, with or without semi-linear history
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

### Requirement: Catch-up preconditions
The tool SHALL make a catch-up merge only when the new tip contains the base and not the update head. It SHALL refuse and merge nothing, naming `abort`, or `finish --no-publish` once `finish` moved the integration branch, or for `sync` naming the merge request, `abort --merge-request` with the warning that withdrawing declines its targets, and `start` and `finish` to publish them again, when the new tip does not contain the base, or when the commits it adds carry a sync record or a copy of a queued upstream, change the journal of a queued port upstream, or change the configuration other than its install command and checks.

#### Scenario: Rewritten integration branch
- **WHEN** the integration branch moves to a commit that does not contain the update's base
- **THEN** the tool makes no catch-up merge, refuses, says that the integration branch was rewritten, names `abort`, and moves no branch

#### Scenario: Upstream taken in elsewhere
- **WHEN** a commit that landed on the integration branch carries a sync record of an upstream the update merges, as when another clone took that upstream in
- **THEN** the tool makes no catch-up merge, names the commit and the upstream, and names `abort`

#### Scenario: Configuration changed
- **WHEN** a commit that landed on the integration branch changes an upstream or the publication setting in `.git-intake.toml`
- **THEN** the tool makes no catch-up merge, names the change, and names `abort`

### Requirement: Catch-up in status, report and doctor
`status` SHALL show a catch-up step with the tip caught up with and the next command, and `status --json` SHALL give it as the update step `catch_up_conflict` or `catch_up`. `report` SHALL list each catch-up merge with the tip it took in and its conflicted files with their resolution source, and measure upstream ranges and the repository's own changes from the base the update started from, or, for an update carried on from a merge request, from the base it was carried on at. `doctor` SHALL accept a catch-up step.

#### Scenario: Status during a catch-up
- **WHEN** a catch-up merge stopped on a conflict and an agent runs `status --json`
- **THEN** the update step is `catch_up_conflict`, the tip caught up with is named, and the next command is `continue`

#### Scenario: Report after a catch-up
- **WHEN** `report` runs on an update that caught up once with a conflict
- **THEN** it lists the catch-up merge with its tip and its conflicted files, and each upstream's merged range and own-change sizes are those measured from the base the update started from

### Requirement: Sync record in history
Every merge commit the tool creates SHALL name the upstream and its target commit in commit trailers, and, when a tag of the upstream's repository points at the target, that tag in the trailer `Intake-Tag`: the tag a revision named, the release a `tags` pattern selected, or otherwise, of the tags at the target, the one with the highest version among those matching the upstream's `tags` pattern whose name is a version after the pattern's literal prefix, or, without such a tag, among those whose name is a semantic version with an optional `v` or `V`; a tag whose name is no version SHALL NOT be recorded this way, even when it matches the pattern. A tag whose name contains a comma SHALL NOT be recorded. The tag SHALL be fixed when `start` fixes the target. Every commit the tool copies from an upstream in `port` mode SHALL name the upstream and the upstream commit it took in the trailers `Intake-Upstream` and `Intake-Taken`. A sync record SHALL count only on a merge commit whose second parent is the record's target. For an upstream in `follow` mode, the tool SHALL derive the last synced commit from its sync records on the integration line, as the target of the newest record whose target no other of its records' targets contains, and SHALL NOT store it in the configuration or outside the history. For such an upstream that carries no sync record, the last synced commit SHALL come from the history, and the tool SHALL show that it does: for a root upstream it is the newest upstream commit the integration branch contains; for a path upstream it is the second parent of the newest merge on the integration line whose second parent belongs to the upstream's branch and which changes the upstream's path, as an import by `git subtree add` or `git subtree pull` leaves. This decision SHALL depend only on the integration branch and the upstream's own branch, never on other upstreams. When a `follow` upstream's last synced commit is no longer contained in its branch, the tool SHALL report that upstream's history as rewritten and SHALL refuse to update it.

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

#### Scenario: Squash message with trailers
- **WHEN** a squash commit's message ends with `Intake-Upstream` and `Intake-Target` trailers copied from an update's sync merge
- **THEN** the trailers do not count as a sync record

