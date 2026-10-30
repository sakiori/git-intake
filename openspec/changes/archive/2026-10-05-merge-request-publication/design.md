## Merge requests through push options

The tool opens the merge request with git push options (`merge_request.create`, `merge_request.target`, `merge_request.title`, `merge_request.description`, `merge_request.remove_source_branch`), which GitLab applies while it receives the push. This needs no forge API client and no token beyond the git credentials the push already uses, so the tool stays a plain-git program. A remote that does not support push options refuses them before anything is written; the tool then pushes the branch without options and states which merge request to open, so other forges are served by the same mode with one manual step.

## The verified commit is what gets merged

The merge request's branch points at the verified commit itself, and the update completes only when the publish remote's integration branch contains that commit. A merge by fast-forward or by a merge commit keeps it. A squash or a rebase replaces it with a commit that was never verified, and a rebase would also rewrite the upstream merges; the tool then refuses to complete and explains that the merge request must be merged without squash or rebase. When the integration branch moves before the merge request is merged, the verified commit no longer contains its tip, and the update has to be started again on the new tip, as with a moved integration branch in push mode.

## Mirror branches follow the merge

Mirror branches are advanced and pushed only after the integration branch on the publish remote contains the verified commit, so a mirror never runs ahead of an integration branch that has not taken the same upstream commits. A mirror is pushed directly; a repository that protects its mirrors as well publishes them by hand after `finish --no-publish`.

## Waiting is a step of the update

Between opening the merge request and its merge, the update is in a step of its own that keeps the verified commit, the targets and the branch name; the worktree is no longer needed and is removed. Every command reads that step: `status` names the merge request branch, `start` refuses because an update is in progress, and `abort` deletes the branch from the publish remote and removes the update, leaving the merge request to be closed on the forge.
