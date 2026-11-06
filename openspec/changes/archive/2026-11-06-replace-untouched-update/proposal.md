## Why

An update's merge request keeps the targets it was opened with: `sync` catches it up with the integration branch but never gives it a newer upstream release, since "Each target is fixed when the update starts". When nobody looks at the merge request for weeks, the upstream publishes release after release while the merge request still proposes the first one, and whoever finally reviews it reviews an outdated update and then waits for the next one.

Moving the target inside the same merge request is not safe. A person who read the diff and left nothing behind cannot be told through the forge's API from one who never opened it, so a merge after a moved target could land code that nobody read. Replacing the merge request keeps every reviewed diff intact: the old merge request is closed unmerged, and a new one, reviewed from scratch, proposes the newer release.

## What changes

- A new optional setting `replace_after` in `[publish]`, for merge-request mode, such as `replace_after = "7d"`. Without it `sync` behaves as before.
- With it, when an upstream with `tags` has a release newer than the open update's merge request holds, `sync` replaces the merge request: it plans a new update from the integration tip, with the targets an update about to start takes, `prereleases` and `max_conflicts` included, merges it locally, closes the old merge request with a description that records no targets, deletes its branch, and publishes the new update as a new merge request that names the one it replaces.
- It replaces only a merge request that is untouched and has been open for at least `replace_after`, and never while the publish remote records a pause of the merge queue. Untouched means its branch holds only the tool's sync merges and catch-up merges, at targets the tool's own rules chose, which a new trailer `Intake-Chosen` of the sync record proves, each the clean merge of its parents, and nobody approved, reviewed or commented on it, was asked to review it, reacted to it, assigned it, marked it as a draft, set it to merge automatically or scheduled its merge; the notes `land` writes count as comments. A replacement never moves an upstream of the merge request back. Otherwise `sync` keeps the merge request and says why it was not replaced.
- `land` never replaces the merge request it lands, and one that finds its merge request replaced meanwhile merges nothing and says so.
- A replacement whose new merges conflict replaces nothing: `sync` names the conflict and the upstream releases, discards the attempt, keeps the old merge request, catches it up as before, and exits with status 1.
- Several releases published while the merge request waits collapse into one replacement at the newest release. An upstream without `tags` never triggers a replacement; a replacement that another upstream triggered takes it at its fetched tip, as any new update does.
- A merge request the tool closed by a replacement declines nothing: the next `sync` may propose its targets again.

## Capabilities

### New capabilities

None.

### Modified capabilities

- `git-intake`: a new publication setting, the replacement of an untouched merge request by `sync`, and the requirements on synchronisation, declined targets, exit status and the conflict limit that the replacement touches. `sync` exits with status 1 when it leaves out a follow upstream while the merge request waits, as the exit status requirement already says.

## Impact

- Configuration: `[publish]` accepts `replace_after`. Earlier releases refuse the key as unknown, so every runner of a fork that sets it needs this release first.
- Forge: `sync` reads, for an open update's merge request when `replace_after` is set, its approvals or reviews, its comments, its reactions, its requested reviewers, its assignees, its draft flag, its automatic merge and, on GitLab, its `merge_after`. A GitHub token then needs read access to the pull request's reviews and comments, which the pull request permission gives, and *Issues: read* for the reactions; without it every pull request counts as touched.
- Sync record: merges whose target the tool chose carry `Intake-Chosen`. Merges made by earlier releases lack it, so a merge request opened before the upgrade is never replaced.
- Notifications: each replacement closes one merge request and opens another; watchers of the project and integrations subscribed to merge request events see both. The setting stays off by default for that reason.
- Safety: an update's targets stay fixed for its whole life. A replacement ends one update unmerged and starts another; no tree that a person may have reviewed is merged under another target.
