## Why

Repositories that keep their integration branch protected accept changes only through merge requests: nobody, including the maintainers, pushes to the branch. `finish` publishes by pushing the integration branch, so in such a repository it is always rejected, and `finish --no-publish` leaves the verified result only in the local branches, from which the merge request is opened by hand. The forks and monorepos the tool serves are exactly the repositories that protect their integration branch, so publication through a merge request is their normal path, not an exception.

## What changes

The publish setting becomes a table with the remote and the way of publishing: `push`, as today, or `merge-request`. In merge-request mode `finish` pushes the verified commit as a branch of its own to the publish remote and opens a merge request into the integration branch, without moving any local branch. The update then waits for the merge request. A later `finish` completes the update once the publish remote's integration branch contains the verified commit: it fast-forwards the local integration branch, advances and pushes the mirror branches, and removes the update. It refuses when the merge request was merged without keeping the verified commit, or when the integration branch moved so that the verified commit can no longer be merged as it is.

The release archives also carry a manual page, and `abort` removes the update's logs together with its state.

## Capabilities

### New capabilities

None.

### Modified capabilities

- `git-intake`: configuration of the publish remote and its mode, the update state gains a step that waits for a merge request, and publication can go through a merge request.

## Impact

The `publish` key changes from a remote name to a table; configurations written for 0.1.0 must be updated, so the next release raises the minor version. Merge requests are opened with git push options, which GitLab understands; on a remote without push options the tool pushes the branch and names the merge request to open.
