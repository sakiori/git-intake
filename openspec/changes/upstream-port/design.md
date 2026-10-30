## Transient upstream references

An upstream is fetched with `git fetch --no-tags <url> +refs/heads/<branch>:refs/git-intake/upstreams/<name>`, run in the main worktree so that a relative URL keeps its meaning. The reference lives outside `refs/heads` and `refs/remotes`: no branch listing, checkout, `git push --all` or hosting tool sees it, and no remote exists for a tool to take as the place to open pull requests in.

A command that only reads an upstream, such as `status`, `log`, `branches` or `doctor`, removes its references before it exits, whatever its outcome. An update keeps the references of its upstreams from `start` until `finish` or `abort` removes the update: the targets and, in `port` mode, the commits in the plan must stay reachable while the update is in progress, and a target must not depend on another fetch. `doctor` reports references under `refs/git-intake/` that no update accounts for, and the next command that fetches the same upstream replaces them.

The alternative was to keep the reference between commands as a cache under the same hidden prefix. It would let `status` and `log` work offline and keep the upstream objects that the integration branch does not reach. It was rejected because the point of the change is that the repository holds no standing reference to an upstream; a cache would be one, only harder to see. The costs are accepted. Every reading command contacts the upstreams. In `port` mode, upstream commits the integration branch does not contain become unreachable when the reference goes, so git may prune them, and a later fetch transfers them again. Fetch negotiation still sends the integration branch, which contains the upstream up to the journal's base, so the transfer is bounded by what happened upstream since the base.

`fetch` as a command loses its purpose: a fetch that leaves nothing behind has no result to keep. It is removed rather than kept as an alias, and `status` reports an upstream that cannot be fetched as `unavailable`, as it does today for a missing remote.

The publish remote stays a git remote: publishing is the repository's own business. The rule that the tool never pushes to an upstream becomes a rule on URLs: a publish remote whose fetch or push URL equals a configured upstream URL is refused.

## Mirror branches

A mirror branch exists to keep a copy of the upstream inside the product's branch namespace, which is the link this change removes. In `follow` mode the sync records already name every target, and in `port` mode a mirror has no meaning, since the product never contains the upstream branch as a whole. The setting is refused with the instruction to remove it, and every rule about moving, checking and pushing mirrors goes. A mirror branch that exists in a repository stays an ordinary branch; deleting it is the owner's choice.

## Modes

The mode belongs to each upstream, because it describes the relationship to that source: a fork with three patches follows its upstream, a product takes from it. The default is `follow`, which keeps every existing configuration's behaviour apart from the removed settings.

`port` is allowed only for an upstream at the repository root. A root upstream is already the only upstream, so a repository never mixes the two modes in one update. Porting into a path would mean replaying upstream commits under a prefix, which git supports only for whole trees (`-Xsubtree`) and not reliably for commits that move files across the prefix boundary; the monorepo case that uses paths follows its sources until cutover and has no need for it.

Switching an upstream from `follow` to `port` changes one setting. The first `port` update writes the journal with the last synced commit as its base, as the sync records or the history give it. Switching back to `follow` is allowed: the next update merges the upstream up to its target, git finds the merge base from the last merge, and changes taken in `port` mode come back as the same changes, which usually merge cleanly or conflict where the product changed them. The journal of an upstream in `follow` mode no longer means anything; `doctor` reports it as stale.

## What a port takes

The unit is a commit on the first-parent line of the upstream branch after the journal's base. A hosted project merges its pull requests onto that line, by squash, by merge commit or by rebase, so each first-parent commit is either one landed change or one commit of a rebased series. A merge commit is taken as its change against its first parent (`git cherry-pick -m 1`), so a pull request merged with a merge commit is one decision, not one per commit inside it.

A taken commit is copied with its author, its message, and two trailers: `Intake-Upstream: <name>` and `Intake-Taken: <upstream commit>`. `git cherry-pick -x` would record the source as a sentence the tool would have to parse; the trailers follow the existing sync record and are read with `git log --format=%(trailers)`. The copy keeps the upstream author so that `blame` on a taken change still names who wrote it.

## The journal

A taken commit is recorded by the commit that took it. A skipped commit, or one the product already has by other means, has no commit of its own, so its decision needs a place in the tree. The journal is `.git-intake/<name>.toml`:

```toml
base = "<upstream commit the port started from>"

[[decision]]
commit = "<upstream commit>"
verdict = "skip"          # or "have"
note = "optional reason"
```

It is committed on the integration branch, so a decision is reviewed in the same merge request as the code, and the tool reads it from the integration tip as it reads the configuration. Decisions are not stored as trailers on empty commits: a skip is a statement about the product's scope, which belongs where reviewers read it and where it can be changed later by an ordinary edit, for instance to take a commit that was skipped before.

The base is fixed when the journal is created and never moves. Pending commits are computed every time: the first-parent commits after the base on the fetched branch, minus those the integration branch's trailers name as taken, minus those the journal decides. Nothing is stored that the history or the journal already says. A decision on a commit that is no longer on the upstream branch, a base that is no longer on it, or a taken commit that is no longer on it means the upstream was rewritten, and the tool refuses to plan against it, as `follow` mode refuses a rewritten upstream.

The journal has no cursor of the form "decided up to commit X". A cursor would let a commit before it be undecided by mistake, and it would add a second way of saying what the decision list says.

## The plan

A `port` update needs a decision per commit, and a command line is a poor place for dozens of them. The update instead stops at a plan step, in the manner of the todo list of `git rebase --interactive`: `start` writes a plan into the update state and names its path; the user or an agent edits it; `continue` carries it out. Editing a file works the same for a person and for an agent, and needs no editor to be launched.

```
# git intake plan for example-app: 4 pending commits, oldest first.
# take  copy the commit onto the update branch
# skip  record that the product does not take it
# have  record that the product already has it
# later leave it pending
later 1a2b3c4 fix(web): keep the composer focused after a paste
later 5d6e7f8 feat: add a settings page for keybindings
later 9a8b7c6 chore: update dependencies
later 0f1e2d3 fix(server): close the watcher on shutdown
```

Text after the subject and a ` # ` is the note of a `skip` or `have` decision. `continue` refuses a plan that names a commit not in the pending list, names one twice, uses an unknown verb, or decides nothing. It copies the taken commits in the order the plan lists them, so the user can reorder takes when a later fix should come before an earlier feature. A conflict stops the update as a merge conflict does today, and `continue` goes on after it is resolved. A copy that changes nothing, because the product already holds the change, is not committed; it becomes a `have` decision, and the report names it. The journal changes are committed last, as one commit, so that they include those decisions. `start <name>=<revision>` limits the plan to the pending commits that the revision contains, so a review can stop at an upstream release.

An upstream commit often builds on earlier ones. The tool cannot know a semantic dependency, but it can point at a likely one: `report` lists, for each taken commit, the earlier upstream commits still pending that changed the same files. A conflict is not the only signal, since a copy can apply cleanly and still miss a function an earlier commit introduced; the checks and the review catch that, and the report tells the reviewer where to look.

## Contribution branches

`contribute <upstream> <branch>` fetches the upstream and creates `<branch>` at the fetched tip, or at the commit `<upstream>=<revision>` names under the same rules as `start`. The branch holds only upstream history; the user copies the change onto it and pushes it to wherever pull requests to that upstream come from, such as their own fork on the upstream's host. The tool sets no tracking branch and never pushes. Copying the commits could be part of the command, but that would duplicate `git cherry-pick` with less control over a conflict, for a step the user performs once per contribution.

## Update state

The update state gains the mode of each queued upstream and, for `port`, the plan's path and each planned commit's verb and progress, and loses the mirror. The state format's version goes up: a state written by the previous release refers to remotes and mirrors that no longer exist, so the tool refuses it and says to finish or abort that update with the release that started it.
