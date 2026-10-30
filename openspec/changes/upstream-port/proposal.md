## Why

The tool assumes a downstream that follows its upstream as a whole: every update merges an upstream branch up to a target, and the repository keeps a permanent remote, its remote-tracking branches and, optionally, a mirror branch of the upstream. That fits a fork that carries a few patches. It does not fit a product built on an open-source project, where the product owns its history and takes only the upstream changes it wants. A merge cannot take part of an upstream: everything up to the target comes in, and whatever the product does not want has to be reverted and then kept out at every later update.

The permanent remote is also a standing link from the product to a foreign repository. Tools that read a repository's remotes, such as a hosting CLI, take it for the place to open pull requests in, and the upstream's branches sit next to the product's own. Nothing in the update cycle needs that link outside an update.

## What changes

**Upstreams without remotes.** An upstream is described by its URL and branch. The tool fetches it by URL into a reference of its own, `refs/git-intake/upstreams/<name>`, only for the command that needs it, and removes the reference when that command ends, or when an update that needs it ends. The repository keeps no remote, no remote-tracking branch and no mirror branch of an upstream. The `remote` and `mirror` settings and the `fetch` command are removed; commands that read an upstream fetch it themselves. `doctor` reports a git remote that still points at an upstream's URL. The integration branch is the product's own branch, normally `main`.

**Two ways to take an upstream.** Each upstream has a mode:

- `follow`, the default, is today's behaviour: an update merges the upstream branch up to a target.
- `port` takes chosen upstream commits. An update lists the upstream commits nobody has decided on yet in a plan; the user marks each one to take, to skip, as already present in the product, or to decide later. The update records the decisions, copies the taken commits onto the update branch one by one, and then verifies and publishes like any update. A `port` upstream is merged at the repository root.

**A journal of decisions.** A `port` upstream keeps a journal committed on the integration branch: the upstream commit it started from and the commits decided as skipped or already present. Each copied commit names the upstream commit it took in its trailers, so the history records what was taken. Together they answer, for every upstream commit after the starting point, whether it was taken, skipped, already present, or is still pending. `status`, `log` and `report` show these answers, and `report` points out a taken commit that touches files changed by earlier upstream commits still pending.

**Contribution branches.** `contribute` creates a local branch at the upstream's fetched tip, so that a change can be offered to the upstream from a branch that holds only upstream history, without a permanent remote. The tool never pushes it.

## Capabilities

### New capabilities

None.

### Modified capabilities

- `git-intake`: upstreams are fetched by URL into transient references; mirror branches and the `fetch` command are removed; an upstream is taken in `follow` or `port` mode; `port` upstreams keep a journal and are updated through a plan; `contribute` creates a branch for an upstream contribution.

## Impact

This is a breaking change to the configuration and the command line. A configuration with `remote` or `mirror` is refused with the setting to write instead, and every upstream needs `url`. `fetch` is gone, and `status`, `log`, `branches` and `doctor` contact each upstream on every run. The JSON output of `status` gains the mode and, for a `port` upstream, its counts of taken, decided and pending commits; it loses the mirror. An update state written by the previous release while an update was in progress is refused, with the advice to finish or abort the update with that release first.

A repository moving to this release removes its upstream remotes and mirror branches itself; the tool does not delete them. A remote that a partial clone uses to fetch missing objects has to have those objects fetched before it is removed, or the history they belong to can no longer be read; `doctor` says so for such a remote.
