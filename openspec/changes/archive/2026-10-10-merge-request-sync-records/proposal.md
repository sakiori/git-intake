## Why

The tool reads its own sync records only on the first-parent line of the integration branch, on the assumption that `finish` always fast-forwards the integration branch to the update branch. That holds when an update is published by push. With `publish.via = "merge-request"`, the README tells users to merge the update's merge request with a merge commit or by fast-forward, and recommends GitLab's *Merge commit with semi-linear history*. A merge commit takes the previous integration tip as its first parent and the verified commit as its second, so the update's sync merges and copies lie off the first-parent line.

Every reader of the history then misses the update: `status` shows a last synced commit derived from the history, or calls a path upstream not imported; the next `start` plans from that derived commit, and refuses a path the update itself imported; `log` and `doctor` judge from the same wrong record; and a path imported by `git subtree add` on a branch that was merged with a merge commit is not recognised either.

## What changes

The tool reads its own history along one **integration line**: the first-parent line of the integration branch, where each merge on it is followed by the commits its second parent brought in along that parent's own first-parent line (`git rev-list --first-parent M^1..M^2`). Only that one level counts. Sync records, and the merges that brought a path upstream into its path, are found on that line, so an update counts the same whether its merge request was merged by fast-forward or with a merge commit.

Commits contained in a configured upstream's fetched branch stay excluded. A sync merge brings nothing into the line, because what it merged is the upstream's history; this keeps an upstream that itself records syncs from contributing records when it cannot be fetched. Since a branch merged with a merge commit can carry an older record above a newer one, an upstream's last synced commit is the target of its newest record that no other of its records' targets contains, so it never moves back.

Where the tool reads an upstream's branch, as port mode does, and where `finish` looks for the commit that brought the verified commit in on the publish remote's integration branch, the first-parent line keeps its meaning.

## Capabilities

### New capabilities

None.

### Modified capabilities

- `git-intake`: sync records and imports by other means are read along the integration line, which includes what a merge commit brought in, so merge requests merged with a merge commit keep their sync records.

## Impact

No command-line, configuration or output format change. A repository whose update merge requests were merged with merge commits sees its records again: `status` reports their targets as the last synced commits with the source `record`, and the next update plans from them. A record carried by a branch merged with a merge commit, such as a sync merge made by hand on a feature branch, now counts as well.
