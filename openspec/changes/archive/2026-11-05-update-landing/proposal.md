## Why

A fork of a fast upstream needs small updates often, and nothing today carries one through. With an upstream that lands more than a hundred commits a day, a downstream wants an update every day, and each stops on conflicts, so `sync` from a scheduled job cannot finish it alone. What is left around the tool is a script in each fork that picks a target, shows what the update changed, lands the merge request and keeps the project's merge queue out of its way.

- **Choosing a target takes a person.** `tags` takes only releases without a prerelease part, which an upstream may cut days apart. Daily builds are tagged prereleases, and taking one means naming it. Nothing keeps an update to a size that can be resolved in a day.
- **Reviewing an update means reading everything.** `git diff <target> <merged>` shows the whole of what the downstream carries, thousands of files, for an update that changed few of them.
- **Landing is left to the reader.** Once `finish` has opened the merge request, a person watches its pipeline, runs `sync` when the integration branch moved and merges it. With a merge queue on the project, the update needs the queue held by hand, since every merge the queue makes sends the update back to a new pipeline.
- **What an update resolved by hand is not recorded anywhere a later step can read**, such as an acceptance of the interfaces those resolutions touched.

## What changes

- `prereleases = true` on a follow upstream with `tags` lets its prerelease tags count as targets, for `start`, `sync`, `status` and `log`. The release commands keep to releases.
- `max_conflicts = <n>` on such an upstream makes an update without a named revision take the newest tag whose merge conflicts in at most `n` files, or the oldest newer tag when none does.
- `git intake delta` shows how the downstream's changes on top of the upstream differ between the previous target and this one, as a `git range-diff`.
- `git intake conflicts` lists the files that needed resolving in the updates on the integration line, since a named commit and under named paths.
- `git intake land` carries an update's open merge request to the integration branch: it waits for the pipeline, catches up when the branch moved, merges, and stops on a conflict or a failed pipeline.
- `queue = "<account>"` under `[publish]` names the account of a merge queue that takes merge requests assigned to it. `land` then pauses the queue for the last pipeline, records the pause in a reference on the publish remote, and returns what it paused in order. `land --resume` ends a pause left by an interrupted run, and `status` and `doctor` show one.

## Capabilities

### Modified capabilities

- `git-intake`: follow targets among prerelease tags and within a conflict limit, the commands `delta`, `conflicts` and `land`, and the merge queue setting.

## Impact

- **Configuration.** New keys `prereleases` and `max_conflicts` on `[[upstream]]` and `queue` under `[publish]`. A release before this one refuses them as unknown keys, so every place that runs the tool upgrades first.
- **The forge layer** gains reading a commit's pipeline and merging on GitLab and GitHub, and assigning and writing notes on GitLab. A merge queue is a GitLab setting; on GitHub `queue` is refused, as a repository's merge queue already is.
- **The publish remote** holds one more reference while a pause lasts, `refs/git-intake/pause`, which the credential that pushes the update's branch writes.
- **Tokens.** `land` needs what `finish` needs, and on GitHub also the permission to merge, *Checks: read* and *Commit statuses: read*; a fine-grained personal access token cannot have the first of those two.
