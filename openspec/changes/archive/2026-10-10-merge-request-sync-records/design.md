## Which merges count

A merge commit on the integration branch has two kinds of second parent: an update or feature branch of this repository, merged through a merge request, and an upstream's history, merged by the tool's own sync merge or by `git subtree add`. The line cannot tell them apart by shape, so it includes both and relies on two exclusions for the upstream case:

- Commits contained in a configured upstream's fetched branch are left out, as before. This covers the upstream's history whenever it is fetched, and for the path-import fallback, which always has the upstream's branch, it keeps the walk from listing the upstream's history at all.
- A sync merge on the first-parent line brings in nothing. It is recognised as the tool recognises its own merges: it carries a sync record whose target is its second parent, so what it merged is the upstream's history by construction. This holds even when the upstream cannot be fetched or was rewritten, which the first exclusion cannot cover. Without it, an unavailable upstream that records syncs of its own would contribute records, for example as an upstream that `doctor` reports as dropped. The rule belongs to the line, so the records and the path-import fallback read the same commits.

The line goes one level deep: a merge on a second parent's line contributes nothing. Updates land through one merge request, so one level reaches every record the tool writes, and going deeper would bring in history that only passed through other branches.

## Walking the line

Following each merge's second parent with its own `git rev-list --first-parent M^1..M^2` costs one git process per merge on the first-parent line, which in a repository that merges every change through a merge request is one per change, on every `status`.

Instead, one `git rev-list --parents --topo-order` lists the history with its parents, without the excluded commits, and the line is built in memory. The first-parent line is walked from its oldest commit while a set of the commits each first parent contains is extended; a merge's second-parent line is followed until it reaches a commit in that set, which is exactly the boundary `M^1..M^2` draws. The walk visits each listed commit once. Sync records are then read with one `git log --no-walk=unsorted --stdin --grep` over the line's commits, which keeps the line's order.

The listing reads the whole history that the exclusions leave, where the first-parent walk read one line. For a fork, that is the fork's own history, since the upstream's history is excluded; `taken` already walks the same history for copies.

The path-import fallback cannot share the line the records are read from. The records exclude every fetched upstream; the fallback depends only on the integration branch and its own upstream, so it excludes that upstream alone, and in a stacked repository another upstream's history can hold the import merge it looks for. It runs only for an upstream without a record.

## Which record is the last synced commit

With merge commits, the order of the line no longer follows the order in which records were written: a branch that started before an update and carries an older record, such as a sync merge made by hand, can be merged after that update and land above it. Taking the first record on the line would move the upstream back, show the commits in between as pending, and start a port's journal there. Of an upstream's records, the last synced commit is therefore the newest whose target no other of its records' targets contains. One `git merge-base --independent` over each upstream's targets finds them; a target that git no longer has, as after an upstream was rewritten and its old commits pruned, is compared with none, and among targets that do not contain each other, as after a rewrite, the order of the line decides.
