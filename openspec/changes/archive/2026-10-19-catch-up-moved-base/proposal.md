## Why

An update fixes its base, the integration tip, at `start`, and everything after that assumes the integration branch stays there. In practice it rarely does: other work lands while an update waits for a conflict resolution, a long verification, or a review. Today the tool then refuses in three places and sends the user back to the beginning:

- `start` refuses when the local integration branch is behind its published counterpart, although bringing it up to date is a fast-forward the tool already knows how to make safely.
- `finish` refuses when the integration branch moved since `start`, and asks for `abort`, `start`, `verify` and `finish` again.
- In merge-request mode, `finish` refuses when another change landed on the remote's integration branch before the merge request was merged, and asks for the same restart, which also opens a second merge request.

The restart throws away the update branch with every resolution made by hand beyond the conflicting hunks that rerere records, and every fix committed on top of the merges. The refusal protects a real guarantee, that only a verified tree reaches the integration branch, but the guarantee does not need the restart: the update can take the new integration tip in and be verified again.

## What changes

The update branch belongs to the tool, so when the integration branch moves past the base, the tool takes the new tip into the update instead of refusing:

- A **catch-up merge** brings the new integration tip into the update: a merge commit whose first parent is the new tip and whose second parent is the update branch head, marked with the trailer `Intake-Catch-Up`. Every resolution and fix already on the update branch stays exactly as it is, and only real conflicts between the new integration commits and the update stop it. A conflict is resolved and concluded with `continue` like any other. A second move of the integration branch adds a second catch-up merge on top, so the update branch only ever moves forward.
- The integration line follows catch-up merges, so sync records and copies behind them count as if the update had been merged straight in.
- `start` fast-forwards a local integration branch that is strictly behind its published counterpart, and refuses only when the two have diverged.
- `finish` catches up when the integration branch moved, locally or on the publish remote, verifies the result itself, and publishes it. A push rejected because the remote moved is resolved the same way on the next `finish`.
- In merge-request mode the merge request branch becomes the stable `git-intake/update`. When the remote's integration branch moves before the merge, `finish` catches up, verifies, and pushes the result to the same branch without force, so the open merge request carries the new verified commit and the update keeps waiting.
- The update state format becomes 5. A state of format 4 is read in place; format 3 still goes through `migrate`.

The guarantees stay: the integration branch only fast-forwards, only a verified commit is published, and no branch is rewritten or force-pushed.

Two follow-up changes build on this one and are out of its scope: publication through the forge's API for GitLab and GitHub instead of git push options, and a single idempotent command for running updates in CI.

## Capabilities

### New capabilities

None.

### Modified capabilities

- `git-intake`: catching up with a moved integration branch, the conditions for starting an update, guarded publication, publication through a merge request, the integration line, the environment of the checks, the update state and its migration.

## Impact

- Users no longer abort and restart an update because the integration branch moved; `finish` does more work, including running the checks, when it did.
- History gains a catch-up merge for each move of the integration branch an update absorbs. Tools that read sync records from the integration line need the new rule; within this repository that is `integration_line`.
- The merge request branch name changes from `git-intake/update-<commit>` to `git-intake/update`. An update waiting for a merge request opened by an earlier release keeps its branch.
- A catch-up push rewrites the open merge request's title and description, and a catch-up refuses when the integration branch took in one of the update's upstreams, changed a queued port journal, or changed an upstream or the publication setting; `abort` and `start` then plan from the new history.
- The README's update cycle, merge request publication, environment, state format and integration line sections change with the behaviour.
- The state format changes. Earlier releases refuse an update started or saved by this one; this release carries on an update of format 4.
