## Catch-up merge rather than a rebuild

Two ways take a moved integration tip into an update.

A **rebuild** resets the update branch to the new tip and replays the update: the upstream merges against their fixed targets, the copies of a port plan, and the commits added by hand. Its history looks like a fresh `start`. It relies on rerere to reproduce every resolution, and rerere records only the conflicting hunks: an edit made inside a merge commit beyond those hunks, such as a build fix while resolving, is lost, and commits added by hand between two upstream merges have to be replayed in place. This is the evil-merge problem of `git rebase --rebase-merges`, and it would make the tool's central promise, a conflict resolved once, depend on what rerere happened to record.

A **catch-up merge** keeps the update branch as it is and merges the new tip into it: one merge commit with the new integration tip as its first parent and the update branch head as its second. Everything already resolved and fixed stays exactly as it is, and only conflicts between the new integration commits and the update itself need a hand. The cost is one merge commit per absorbed move. The update branch keeps its own discipline: it moves only forward, which also lets a merge request branch move forward without force.

The tool uses the catch-up merge. Its parent order matters: with the integration tip as the first parent, the first-parent line of the integration branch stays the line of commits the branch itself received, and the update arrives on the second-parent side, exactly as when a merge request is merged with a merge commit.

## Repeated catch-ups and the integration line

The integration line follows each merge on the first-parent line into the first-parent line of its second parent, one level deep, and merges found there contribute nothing further. That rule keeps an unrelated branch, merged into a branch that was merged into the integration branch, from counting. A single catch-up merge on the first-parent line fits it: its second-parent line holds the update's sync merges and copies.

Two cases go one level deeper: an update that caught up twice, whose second catch-up merge has the first one on its second-parent line, and an update with a catch-up merge that is published through a merge request merged with a merge commit. A rebuild of the update from its pre-catch-up head would avoid the nesting, but it would need a force push of the merge request branch, a stored pre-catch-up head, and a replay of the fixes made after the first catch-up, which reintroduces the rebuild's weaknesses.

Instead, the trailer `Intake-Catch-Up: <integration branch>` marks the tool's catch-up merges, and the integration line descends through a marked merge to the first-parent line of its second parent, at any depth, bounded as every second-parent walk is by what the first parent of the merge on the first-parent line contains. An unmarked merge keeps its current meaning. The marker is as trustworthy as the sync records it sits beside: both are trailers the tool writes and anyone could forge, and the tool reads history it does not police.

## The base after a catch-up

The base becomes the catch-up merge's first parent, the new integration tip. The rules that guard publication keep their meaning: `finish` requires the integration tip to be the base, the verified commit contains the base, and abort leaves the integration branch where it is. For the checks, `GIT_INTAKE_BASE` names the integration tip the result will land on, so a ratchet comparing `GIT_INTAKE_BASE` with `GIT_INTAKE_HEAD` measures the update and not the commits that landed meanwhile. Upstream targets, previous synced commits and plans do not change: they belong to the upstreams, not to the integration branch.

A catch-up merge requires the new tip to contain the old base. A tip that does not was rewritten; the tool then refuses and names `abort`, as before.

## Who triggers a catch-up

`finish` is the command whose precondition the move breaks, so it catches up, then verifies the result itself and goes on to publish. Verification is required anyway, and a user who runs `finish` asks for the update to land. A conflict stops `finish` with exit status 1 and the update in a catch-up step; `continue` concludes the merge with its trailer, and the next `finish` verifies and publishes. Failing checks stop it as `verify` does.

A busy integration branch can move again while `finish` verifies a catch-up. `finish` then stops and says to run it again rather than looping: each run makes at most one catch-up, so its duration stays bounded and predictable.

In push mode `finish` fetches the publish remote before anything moves. A local integration branch strictly behind the remote is fast-forwarded first, as `start` now does, and then caught up with; one that holds commits of its own while the remote moved is refused, because those commits are the user's to publish or reconcile. A push that is still rejected, because the remote moved between the fetch and the push, leaves the update in its finishing step as today. The next `finish` catches up with the remote's branch from that step: the catch-up merge contains the planned commit, to which the local branch has already moved, so the local branch fast-forwards again, and nothing was published in between. Here the local integration branch is ahead of the base by the planned commit, which is the one case where the base is the remote's tip rather than the local one.

`start` fast-forwards a strictly behind integration branch the way `finish` moves it: a checkout of the branch by `git merge --ff-only`, otherwise `update-ref` against its old value, refusing when a checkout would lose changes.

## Forming the merge

`git merge` run on the update branch would make the update head the first parent, the opposite of what the integration line needs. The tool records the tip it catches up with in the state, checks out that tip detached in the update worktree, merges the update head into it, then moves the update branch from its recorded head to the result with `update-ref` against the old value, and attaches the update worktree to the branch again. The recorded tip makes every point of interruption recognisable: a merge in progress whose `MERGE_HEAD` is the update head, or a detached commit carrying `Intake-Catch-Up` whose first parent is the recorded tip, as the sync record makes an upstream merge recognisable. In the conflict message the integration side is ours and the update side theirs, so the message names both.

## What a catch-up refuses

A catch-up takes in the integration branch's own work. Some commits would change what the update means rather than add to it: a sync record or copy of an upstream the update merges, which another clone took in meanwhile; an edit of a queued port upstream's journal, which would let a merge produce a journal with duplicate decisions or a moved base; a change of an upstream or of the publication setting in the configuration, which the update fixed at `start`. These refuse and send the user to `abort` and `start`, which then plan from the new history. A change of the install command or the checks is the integration branch's business and is taken: verification after a catch-up reads them from the configuration at the catch-up merge, so the tree that lands is checked with the checks it carries.

## The report keeps the starting base

The base moves with each catch-up, but the update's own story starts where `start` fixed it. The state keeps that starting base for `report` and the merge request description, so upstream ranges and the repository's own change sizes stay comparable with what the merges were made on, and the catch-ups appear as entries of their own with their conflicts.

## Stable merge request branch

The merge request branch was named after the verified commit, so each verification would need a new branch and a new merge request. It becomes `git-intake/update`, the name of the local update branch. Because the update branch only moves forward, every later push to it is a fast-forward and needs no force; GitLab keeps the open merge request on the branch and its head reference follows the push.

GitLab applies push options to an open merge request of the pushed branch: `merge_request.title` and `merge_request.description` rewrite it, while `merge_request.create` opens a merge request only when the branch has none open. A catch-up push therefore sends the regenerated title and description without `merge_request.create`: the open merge request is brought up to date, and a push that races with its merge cannot open a second one. Just before that push, `finish` checks again that the merge request's head, its branch and the remote's integration branch are as it fetched them. A merge that still slips between that check and the push cannot land a verified tree, since the integration branch has moved past what the merge request carried; the next `finish` refuses it as an unverified merge, and no second merge request exists. The state keeps every commit the update pushed, so that a merge request merged before the integration branch moved is recognised whichever of them it carried.

A shared name has two costs. Two clones, such as a person and a CI job, that update the same repository at the same time share the branch; the first push wins and the other is rejected as not a fast-forward, which is the right outcome but no longer separated by name. A branch of that name left on the remote by an earlier update is accepted for the first push only when the base contains it, as after a merge request merged with its branch kept; any other leftover is refused and named per push URL, since fast-forwarding it could take over a merge request the tool knows nothing of. Without the forge's API, a merge request closed with its branch kept looks open; the follow-up change that moves publication to the API removes this limit.

## State format

The state gains the catch-up step, the tip being caught up with, the starting base and the commits pushed to the merge request. A format 4 release would drop the unknown fields when it saves, silently losing a catch-up in progress, so the format becomes 5. A format 4 state needs no move of its worktree, so this release reads it in place: a waiting update keeps its branch `git-intake/update-<commit>`, and a catch-up of it pushes to that branch. The first command that changes such an update writes format 5 and says that earlier releases can no longer carry it on. `migrate` keeps carrying format 3 over, now to format 5.
