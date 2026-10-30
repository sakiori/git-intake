## Transient upstream references

An upstream is fetched with `git fetch --no-tags <url> +refs/heads/<branch>:<reference>`, run in the main worktree so that a relative URL keeps its meaning. The reference lives outside `refs/heads` and `refs/remotes`: no branch listing, checkout, `git push --all` or hosting tool sees it, and no remote exists for a tool to take as the place to open pull requests in.

Every command fetches into a namespace of its own, `refs/git-intake/read/<token>/`, and removes it before it exits, whatever its outcome. An update holds `refs/git-intake/upstreams/<name>` for each queued upstream, at the tip `start` fetched: the targets and, in `port` mode, the commits in the plan must stay reachable while the update is in progress, and a target must not depend on another fetch. `start` writes these references once the update exists, and only `finish` or `abort` removes them. A command that reads an upstream during an update never fetches into the update's reference: an earlier version fetched into it and restored it afterwards, but a command killed in between, or one racing with `start`, could leave the update's reference moved or gone, and the update's commits unprotected. `start` saves the update's state before it writes those references, so a `start` killed in between leaves an update whose fetched commits are held only by its own namespace. Every command that acts on the update in progress therefore restores a missing reference from the state, where the commit is still in the repository; the state names each tip, so the repair is exact and idempotent. `doctor` reports a missing or moved update reference, and does not call stray a reference that still holds the commit of a missing one, since removing it would let the update's commits be pruned. Otherwise `doctor` reports references under `refs/git-intake/` that no update accounts for, such as the namespace of a command that was killed; nothing uses them, and they are removed by hand.

The alternative was to keep the reference between commands as a cache under the same hidden prefix. It would let `status` and `log` work offline and keep the upstream objects that the integration branch does not reach. It was rejected because the point of the change is that the repository holds no standing reference to an upstream; a cache would be one, only harder to see. The costs are accepted. Every reading command contacts the upstreams. In `port` mode, upstream commits the integration branch does not contain become unreachable when the reference goes, so git may prune them, and a later fetch transfers them again. Fetch negotiation still sends the integration branch, which contains the upstream up to the journal's base, so the transfer is bounded by what happened upstream since the base.

`fetch` as a command loses its purpose: a fetch that leaves nothing behind has no result to keep. It is removed rather than kept as an alias, and `status` reports an upstream that cannot be fetched as `unavailable`, as it does today for a missing remote.

The publish remote stays a git remote: publishing is the repository's own business. The rule that the tool never pushes to an upstream becomes a rule on URLs: a publish remote whose fetch or push URL names a configured upstream's repository is refused, and `doctor` reports any other remote that does. Two URLs name the same repository when they are equal after normalisation. A local path or `file://` URL is resolved against the main worktree the way git finds a repository there, trying `<path>/.git`, `<path>`, `<path>.git/.git` and `<path>.git`, and made canonical, so that `../up`, `../up.git/` and the absolute path of that directory match. Any other URL is read as `scheme://user@host/path`, the scp-like `user@host:path` as `ssh://`; scheme and host go to lower case, a default port is dropped, and so are a leading `/` and a trailing `/` and `.git` of the path, the differences a hosting service ignores. Treating `host:path` and `ssh://host/path` alike can match two repositories on a server that resolves the scp-like path against a home directory; the cost is a refused configuration, while a missed match would let the tool push to an upstream. The path of a hosted URL keeps its case, since some services tell paths apart by case; a false match there would refuse a valid configuration, while a missed one only weakens a safety check. An upstream's name is part of these references, so it has to be a valid reference name component; the configuration refuses names git would refuse, such as one ending in `.lock`.

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

The tool fixes the base when it creates the journal and never moves it; only the owner moves it by hand, after a rewrite took it off the upstream's line (see *Recovering from a rewrite*). Commit ids in the journal are full ids, compared in lower case, as git writes them. Pending commits are computed every time: the first-parent commits after the base on the fetched branch, minus those the integration branch's trailers name as taken, minus those the journal decides. Nothing is stored that the history or the journal already says. A decision on a commit that is no longer on the upstream branch, a base that is no longer on it, or a taken commit that is no longer on it means the upstream was rewritten, and the tool refuses to plan against it, as `follow` mode refuses a rewritten upstream.

The journal has no cursor of the form "decided up to commit X". A cursor would let a commit before it be undecided by mistake, and it would add a second way of saying what the decision list says.

A `port` upstream with no journal, no sync record and no history shared with the integration branch has nothing to count from. `status` gives it a state of its own, `no base`, rather than `pending` with no numbers, so that a schedule or an agent can tell it from an upstream that only waits for decisions; `status --check` fails for it, since nobody can review it until its journal is written by hand.

## Recovering from a rewrite

A rewrite of the upstream branch, a force-push that replaces commits, can take commits off the first-parent line that the product took or decided on. The tool cannot tell a commit that was replaced by an equivalent one from a change the upstream withdrew, or from an accident on the upstream's side, so it reports the upstream as `rewritten` and refuses to plan, naming every taken or decided commit that is off the line, and the base if it is gone.

The way out is an explicit list in the journal: `retired = ["<full id>", ...]`, the commits the owner acknowledges as gone. The tool ignores retired commits when it judges whether the upstream was rewritten and when it counts taken and decided commits; the rewritten versions of their changes then show up as pending, where the plan can mark them `have`. A base that is no longer on the line has to be moved by hand to a commit of the current line; until then the upstream stays rewritten, since nothing after a lost base can be accounted for. A retired commit needs no object in the repository, so a commit that garbage collection pruned after the rewrite is retired the same way.

The alternative was to ignore missing commits silently, or to let a moved base imply them. It was rejected because silence would hide a real rewrite: an upstream that drops a security fix the product took, or rewrites history by mistake, would look current. The list makes the acknowledgement a reviewed change of the journal, in the same merge request as anything else the owner decides, and it keeps the record of what was taken before the rewrite.

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

A copy whose commit fails, as when a hook of the repository rejects it, stops the update like a conflict and keeps the copy staged. The user fixes the cause in the update worktree and stages the result, and `continue` commits that as the copy, with the upstream author, message and trailers. `continue` never resets the worktree: a fix made by hand is the user's work, and picking the commit again would throw it away. Only the index is committed, so a copy whose changes sit in the worktree unstaged, as after an interruption, is not taken for an empty one: the tool stops and asks to stage what belongs to the copy, and records `have` only when neither the index nor the worktree differs.

A copy never changes `.git-intake.toml` or anything under `.git-intake/`. An upstream built with git intake carries its own configuration and journals, which describe the upstream's relationship to its own sources, not the product's; taking them would replace the product's configuration or create journals of upstreams the product does not have. The tool restores those paths from the update branch head before it commits a copy, says which paths it left out, and records a copy that changed nothing else as `have`, like a copy that changed nothing. The product's configuration changes only by its own commits.

An upstream commit often builds on earlier ones. The tool cannot know a semantic dependency, but it can point at a likely one: `report` lists, for each taken commit, the earlier upstream commits still pending that changed the same files. A conflict is not the only signal, since a copy can apply cleanly and still miss a function an earlier commit introduced; the checks and the review catch that, and the report tells the reviewer where to look.

## Contribution branches

`contribute <upstream> <branch>` fetches the upstream and creates `<branch>` at the fetched tip, or at the commit `<upstream>=<revision>` names under the same rules as `start`. The branch holds only upstream history; the user copies the change onto it and pushes it to wherever pull requests to that upstream come from, such as their own fork on the upstream's host. The tool sets no tracking branch and never pushes. Copying the commits could be part of the command, but that would duplicate `git cherry-pick` with less control over a conflict, for a step the user performs once per contribution.

## Update state

The update state gains the mode of each queued upstream and, for `port`, the plan's path and each planned commit's verb and progress, and loses the mirror. The state format's version goes up: a state written by the previous release refers to remotes and mirrors that no longer exist, so the tool refuses it and says to finish or abort that update with the release that started it.
