## Why

In a GitLab CI job the runner's own git configuration makes git push with the job token: it rewrites the server's URLs into ones that carry the token, or with its flag `FF_GIT_URLS_WITHOUT_TOKENS` answers the server through a credential helper. The job token may not push, or its pushes start no pipeline. `sync` meets this only at its first real push: while every upstream is current, the schedule stays green, and the setup looks right until an upstream moves.

## What changes

- In a GitLab CI job, `sync` in merge-request mode and `release tag` refuse, at the start of every run, a clone that would push to the publish remote with the job's `CI_JOB_TOKEN`: a push URL that carries it, or for a URL without a password, a credential helper that answers with it.
- `sync` in push mode warns of the same setup and goes on.
- The README's GitLab recipes push with the access token: they drop the runner's git configuration, bind the helper to the server's URL, and guard the push URL. Their variables name the checkout for an image that runs as another user than cloned it.

## Capabilities

### New capabilities

None.

### Modified capabilities

- `git-intake`: a new requirement for pushing from a GitLab CI job.

## Impact

- A job that pushes with the job token where a project allows it now fails at once instead of opening a merge request whose later pushes start no pipeline, which makes this a breaking change for such a job. A push-mode job keeps running, since a project may have chosen that and needs no pipeline for its pushes to land; it gets a warning.
- The check runs only where `CI_JOB_TOKEN` is set, and asks git's credential helpers, and no prompt, for a URL without a password. Nothing stores the answer. A helper that hangs would hang the run, as it would the push.
