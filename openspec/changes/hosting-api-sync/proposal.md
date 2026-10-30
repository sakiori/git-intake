## Why

Publication through a merge request talks to the forge through git alone: it asks for the merge request with git push options and reads the merge request's number from the text GitLab prints while it receives the push. That leaves the tool blind wherever git cannot see:

- It cannot tell an open merge request from one closed with its branch kept, cannot change a merge request's title or description except through another push, and does not work with GitHub, which has no push options.
- It cannot see whether the forge will merge only a tested tree. To keep the guarantee that only a verified tree lands, the tool verifies the update locally, keeps the update waiting on the machine that opened the merge request, and after the merge reconstructs from trees and parents whether what landed was the verified commit. A commit someone adds to the merge request branch, even a fix, makes the update fail.
- Running updates unattended in CI needs a chain of commands and state on disk that a CI job does not keep. A repository that wants its upstreams taken in on a schedule has no single command to run.
- The checks a merge request pipeline runs on an update get none of the `GIT_INTAKE_*` variables that `verify` gives them locally.

## What changes

The tool talks to GitLab and GitHub through their REST APIs, and the forge becomes where an update lives once it is published for review:

- **Forge API.** The publish remote's forge, GitLab or GitHub, is recognised from its URL, from the CI environment, or from the configuration, and reached with a token from the environment. Merge requests and pull requests are found, opened, updated and closed through the API. Push options and the parsing of the remote's messages go away.
- **The merge request is the update.** In merge-request mode, `finish` pushes the update branch, opens or updates the merge request, and ends the local update. There is no local waiting step, no local judgement of what landed, and no local verification requirement: the merge request's pipeline verifies the update. The tool instead requires, through the API, that the forge merges only a tree its pipeline tested: on GitLab a successful pipeline is required and the merge method is fast-forward or merge commit with semi-linear history, with automatic rebase off; on GitHub at least one status check is required with the branch up to date before merging. Merge trains and merge queues are refused, since it could not be confirmed that they land the tree they tested without rebasing it. Squash and rebase merges stay incompatible with updates, since they drop the sync records. Commits added to the merge request branch are welcome; the pipeline checks them.
- **`git intake sync`.** One idempotent command that brings the repository's follow upstreams in without local state. In merge-request mode it reads the open merge request from the forge: it catches it up when the integration branch moved, leaves it alone otherwise, and opens a new one when none is open and an upstream is behind, unless the most recent merge request for those targets was closed without merging. In push mode it starts, verifies and publishes in one run. Upstreams it cannot fetch are left out and named, so one broken source does not stop the others. A conflict stops it, as `start` stops, leaving the update in progress for a person to resolve with `continue` and `finish`; in CI the job fails.
- **`git intake env`.** Prints, or passes to a command, the `GIT_INTAKE_*` variables of the update a commit carries, read from history, so a merge request pipeline gives its checks what `verify` gives them.
- **`abort --merge-request`** withdraws the update's merge request through the API, closing it and deleting its branch; plain `abort` keeps removing only the local update.
- `status` shows the open merge request and `doctor` checks the forge, the token and the merge settings.
- An update left waiting for its merge request by an earlier release is handed over to the merge request by the first `finish` or ended by `abort`.

The guarantees stay, restated for each mode: in push mode only a commit `verify` passed reaches the integration branch; in merge-request mode only a tree the forge's required pipeline tested can be merged, by anyone the forge's rules bind; no branch is rewritten or force-pushed; and nothing is pushed to an upstream.

## Capabilities

### New capabilities

None.

### Modified capabilities

- `git-intake`: fork configuration, the forge of the publish remote and its merge settings, publication through a merge request, unattended synchronisation, the update's variables outside `verify`, guarded publication, resumable state, the environment of the checks, machine-readable output, status and doctor, and shallow repositories.

## Impact

- Merge-request mode needs a token in the environment (`GITLAB_TOKEN`, or `GH_TOKEN` or `GITHUB_TOKEN`) and a forge whose merge settings require a tested tree; projects whose settings do not are refused, naming each setting to change. A GitLab project with the plain merge commit method does not qualify, even with merged results pipelines, because GitLab does not require a merged results pipeline to match the current target branch; nor does one with merge trains, nor a GitHub branch with a merge queue.
- `finish` in merge-request mode no longer waits, completes or catches up an update after the merge request is opened; `sync` catches up an open merge request, and the next `start` brings the local integration branch up to date after the merge. Users and scripts that ran `finish` again after a merge stop needing to.
- In merge-request mode `finish` no longer requires `verify`, and a catch-up no longer runs the checks; projects that relied on local verification before review rely on the merge request pipeline instead and use `git intake env` to pass the update's variables to it.
- In merge-request mode `start` refuses a local integration branch with commits the publish remote lacks, as `sync` does, since they would ride along in the merge request.
- A sync record counts only on a merge commit whose second parent is its target, so the trailers a squash commit's message copies do not count; such commits already on an integration branch stop counting as records.
- An update that changes the CI definition or the configured checks says so in its merge request, because the pipeline that verifies it then runs what it brings.
- The target variable of a port upstream becomes the newest commit the update takes or decides, the same for `verify` and `env`.
- Every command refuses to run in a shallow repository; CI jobs fetch the full history.
- New dependencies: an HTTP client with TLS and a JSON reader for the forge's responses, behind one forge interface.
- The README's merge request publication, commands, exit status and environment sections change, and gain a section on running updates in CI for GitLab and GitHub, including which tokens trigger the merge request's pipeline.
- The state format stays 5. An update of format 4 or 5 that waits for its merge request is handed over or withdrawn as above; earlier releases can carry on an update started by this one, under their own rules.
- The README states what running upstream code in CI exposes and how to contain it: protected variables, a GitHub token without the Workflows permission, and only the push credential in push-mode jobs.
