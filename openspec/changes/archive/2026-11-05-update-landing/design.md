## Context

A merge request for an update lands by fast-forward once its pipeline passed on its head. A project whose merge requests go through a queue moves its integration branch one merge at a time, and each move leaves the update's head behind it, so it needs a catch-up merge and a new pipeline. While the queue has work, the update can lose this race indefinitely.

## Decisions

### `land` belongs to the tool

`land` is the end of the same life `finish` and `sync` carry: it needs the update's merge request, the settings that make a merge safe, the catch-up merge and the forge layer, and all of them live here. A script beside the tool in each fork would repeat them and drift. A person still may merge by hand; nothing requires `land`.

### A pause takes the queue's assignments away

The established queue for GitLab outside its paid tier, marge-bot, takes the merge requests assigned to its account and has no pause: its `--embargo` is a fixed time window read at start. Stopping its process needs access to its host and stops every project it serves. Merge trains need GitLab Premium, and git-intake refuses them because it cannot confirm that a train lands the tested commit. Taking the update through the queue is out as well: a queue rebases, and a rebase drops an update's merge commits, while a queue's merge fusion would make the update's branch the first parent of the integration line.

So `land` pauses by unassigning. That needs nothing but the merge request API, serves any queue that works by assignment, and leaves each merge request exactly as it was.

The merge request the queue is already landing stays assigned, because taking it away wastes its pipeline. It is told by its head containing the tip of the integration branch, and it is at most one pipeline to wait for: once one of them merged or stopped being current the others cannot be next and are held, and one whose pipeline passed that the queue does not take is held after a short wait, as a queue can stop taking work at any time.

### A pause lives in a reference on the publish remote

A process can die during a pause, and merge requests that silently left a queue are the worst way this feature can fail. So what the pause took is recorded before the first assignment is taken away, and any later run in any clone can read it and end it. The record cannot be local state: the run that ends a pause may be elsewhere.

It lives in one reference, `refs/git-intake/pause`, on the publish remote: a parentless commit that nothing else reaches, whose message holds the queue's account, the merge request the pause was started for, the held merge requests with the assignees they kept, in return order, and those in flight when it began. Three things follow.

- **One place to look.** The record does not depend on which merge request the pause was started for, or on what became of that one, so a pause cannot be hidden by a newer merge request or lost with a merged one, and `sync` and `start` can warn of it at no cost.
- **Push rights to write it.** The tool acts on the record with its own token, assigning merge requests to the queue's account, so writing the record must take at least what merging into the project takes in practice. Reading it takes git alone, so `status` needs no token and no forge API for it, and a failing forge takes nothing else from its view.
- **An atomic swap that keeps two runs apart.** A push with a lease is compare-and-swap on the record: creating needs the reference absent, updating needs the value read, and ending deletes it under the same lease. A run that loses touches no assignment and stops. A run whose record disappeared knows that a `--resume` ended its pause, and stops holding merge requests.

The record goes last when a pause ends, after the merge requests returned, so that a run killed in between can be resumed. Returning goes in the recorded order, at least a second apart, so a queue that orders by assignment time keeps it. A plain note on the merge request still tells people what is going on; it holds no record and decides nothing, so failing to write it stops nothing.

Pauses exist only on GitLab, as `queue` is refused on GitHub, so nothing about them is asked of GitHub.

### Ending a pause is worth some persistence

The default of `--timeout` is six hours and a CI job's limit is usually one, so a signal is a normal way out: SIGINT, SIGTERM and SIGHUP set a flag the waits and the rounds check, and the pause ends before the process exits. A terminal that hangs up, over SSH for one, also takes the output with it, so a message that cannot be written is lost rather than ending the process. An outage is another: a 5xx or a network error counts as a failed round of the wait up to a limit, and each step of ending a pause is tried again, since a queue left standing costs more than a slower exit. SIGKILL leaves the record, which is why it is written first.

Git's own commands fail the same way, and worse: a real `git ls-remote` of a remote that had worked seconds before failed once with `Permission denied (publickey)`, and the same command worked a moment later. So a failed `git fetch`, `git ls-remote` or `git push` of the publish remote counts as a failure that may pass, whatever git says of it, and a credential that really is refused costs only the bounded attempts before the same message. Only what cannot pass is final: a lost swap of the record, which is another run's doing, and a push another run's push rejected.

Each command retries the largest step it can repeat safely, so that one classification serves them all. `land` repeats a round, which reads everything afresh and discards a catch-up that did not publish. `sync` in merge-request mode repeats its whole run, as it was built so that any end but a conflict leaves nothing behind for the next run to meet, up to its push: past it the next run would find nothing left to do, so the request that opens or rewrites the merge request is retried in place; retrying each git command instead would have needed a retry policy in every read, and would have slowed `status` and `doctor`, which report a failure at once and have nothing to protect. In push mode `sync` retries only what it reads before it starts, since a run that verified an update leaves it in progress.

### Pause only for the last pipeline

The first pipeline can fail, and a queue must not stand still for that. `land` pauses only when the update's pipeline has passed and the branch has moved, so the wait is one pipeline, and only for a tree that already passed once. A failed pipeline or a conflict ends the pause before `land` stops, because fixing them takes longer than anyone should wait. For the same reason it does not pause at all when it can already tell that the merge will not happen: a merge request the forge reports as a draft or without its approvals, or a catch-up predicted to conflict.

### `delta` compares two deltas

What a downstream carries on top of its upstream is one large difference, and what an update changes in it is usually small. `git range-diff` of two synthetic one-commit series, the downstream's tree on the previous target and on the new one, states exactly that, with a tool reviewers of forks already use. Comparing two plain diffs by text would lose the pairing by file and context.

### `max_conflicts` counts files, and prefers progress

A count of conflicted files is what `status --check` already predicts and what decides how long resolving takes. When every newer tag exceeds the limit, the oldest one is taken: refusing would leave the downstream behind for good, and the next update goes on from there.

## Alternatives

- **Prerelease tags through a wider pattern alone.** Allowing them whenever a pattern names a prerelease part would change what existing patterns such as `v*-*` select. An explicit key keeps the meaning of every configuration written so far.
- **A limit in commits.** Commits say little about work: one upstream commit can conflict in fifty files, and fifty in none.
- **Pause by a label the queue reads.** The queue has no such setting.
- **The record in a note on the merge request.** Anyone who can comment can write one, it sits on one merge request that a later one hides, reading it takes paged API requests and a token, and nothing keeps two runs apart. Checking the note's author and the forge's system notes would answer the first and none of the others.
