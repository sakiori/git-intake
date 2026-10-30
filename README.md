# git-intake

`git intake` keeps a downstream repository current with one or more upstreams. It takes upstream changes by merge commits or by copying chosen upstream commits, never rewrites history and never force-pushes. The work happens in a separate worktree. A conflict is resolved once, and the repository's own checks run on exactly the tree that gets published. The integration branch moves only by fast-forward, after verification.

It covers three cases:

- **A fork**: one upstream followed at the repository root, merged as a whole. A branch stacked on another fork's integration branch is a fork too, with that branch as its upstream.
- **A monorepo assembled from several repositories**: each source repository is an upstream merged under its own path. They keep being taken in until cutover, and `status --check` gates the cutover.
- **A product built on an open-source project**: the product owns its history and takes only the upstream commits it wants. Each update lists the upstream commits nobody has decided on yet; the product takes some, skips others, and records which ones it already has.

The tool runs the installed `git` for every repository operation. Each repository describes itself in a committed `.git-intake.toml`. Upstreams are fetched by their URL; the repository keeps no remote, remote-tracking branch or other branch of an upstream.

## Installation

Requires Git 2.43 or later (`git merge-tree -X` is used for conflict prediction) and a POSIX `sh`. The tool is a single binary named `git-intake`; with it on the `PATH`, git runs it as `git intake`.

Releases provide prebuilt binaries for the targets `x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl` and `aarch64-apple-darwin`. The musl binaries are statically linked and run on any Linux distribution and in minimal CI images.

### With mise

Pin the version in the repository's tool configuration, so that an update of the tool is a reviewed change:

```sh
mise use gitlab:sakiori-oss/git-intake@0.1.0
```

mise's GitLab backend picks the release asset for the running platform by its target triple and finds the binary at the archive root without further options.

### Direct download

Each release lists its archives, `git-intake-vX.Y.Z-<target>.tar.gz`, with a `.sha256` checksum next to each. The binary sits at the archive root, and the manual pages under `man/man1/`:

```
git-intake
man/man1/git-intake.1
man/man1/git-intake-<command>.1
```

To download an archive and its checksum directly:

```sh
tag=vX.Y.Z  # the release to install
target=x86_64-unknown-linux-musl
file="git-intake-$tag-$target.tar.gz"
base="https://gitlab.com/sakiori-oss/git-intake/-/releases/$tag/downloads/binaries"
curl --fail --location --remote-name "$base/$file"
curl --fail --location --remote-name "$base/$file.sha256"
sha256sum -c "$file.sha256"
tar -xzf "$file" -C /usr/local/bin git-intake
```

To install the manual pages as well, extract them into a manual directory, for example `tar -xzf "$file" -C /usr/local/share man`.

### From source

Contributors with a Rust toolchain can build any revision:

```sh
cargo install --locked --git https://gitlab.com/sakiori-oss/git-intake.git --tag v0.1.0
```

From a checkout, `cargo install --locked --path .` installs the working tree. `cargo run --locked -p xtask -- man <dir>` writes the manual pages into `<dir>`. A build from source reports the version `0.0.0`: only a release build carries its tag's version.

### Help and the manual page

`git intake -h`, and `-h` or `--help` after a command (`git intake finish --help`), print the built-in help. `git intake --help` is different: git turns it into `git help intake`, which opens the manual page `git-intake(1)`, so it works only where `man` finds that page.

`man` finds the pages of a release archive when its `man` directory is on the manual search path. On Linux, man-db derives that path from `PATH`: for every directory on `PATH` it also searches that directory's `man` subdirectory, so the pages are found when the directory holding `git-intake` is on `PATH`, as with `mise activate`. Otherwise, or when `MANPATH` is set, add the directory to `MANPATH`; with mise:

```sh
export MANPATH="$(mise where gitlab:sakiori-oss/git-intake)/man:$MANPATH"
```

After a direct download, extracting the pages into `/usr/local/share` puts them on the default search path.

## Releasing

A release is the pipeline of a protected tag `vX.Y.Z` on a commit of `main`; nothing else publishes. The tag, the release guard and the GitLab Release come from shared release components, which `.gitlab-ci.yml` includes at a pinned commit.

- A daily pipeline schedule on `main` cuts the tag. Its pipeline holds only the tagging job, which tags the tip of `main` with the next version unless a release tag already points at it. git-cliff computes the version from the Conventional Commits since the last release: in `0.x` a breaking change raises the minor version and a feature, fix, performance change or revert the patch version; other commits, such as documentation, ask for no release. Nobody picks a version or pushes a release tag by hand.
- An off-cycle release is the same schedule run by hand; it cuts the next version by the same rules.
- The tag's pipeline starts with the guard, which every build waits for. It refuses a tag that is malformed, does not point at a commit of `main`, does not sort after every other release, or whose release is published completely. The builds stamp the tag's version into `Cargo.toml` and `Cargo.lock` in the job's working tree only, so the binaries and their manual pages report it; the committed version stays `0.0.0`. The publish job uploads the archives and their `.sha256` files to the generic package registry, package `git-intake`, version `vX.Y.Z`, and last a `SHA256SUMS` over the archives, which marks the release complete. The GitLab Release follows, with notes git-cliff writes from the commits since the previous release and links to the archives.
- A failed release is finished, not cut again: retry its failed jobs, or run its tag's pipeline again while nothing of it is published completely and no newer release exists; after that the guard refuses a new pipeline, so only the failed jobs are retried. The schedule cuts no second tag for a commit that already has one. Rollback is forward, by a revert and an off-cycle release.
- A pipeline started by hand on `main` (`glab ci run --branch main`) is a rehearsal: it builds every target as a release does and publishes nothing. Rehearse after a change to the build before the next scheduled release; merge request pipelines also build when a change can affect the build.

## Configuration

`.git-intake.toml` lives at the repository root and is committed on the integration branch. The tool always reads the version committed at the integration branch tip. An uncommitted edit in the working copy is ignored, and the tool prints a note that the working copy differs. The tool finds the integration branch from the configuration committed at `HEAD`, or from the update in progress, so run it from a checkout of the integration branch or of a branch that carries the configuration.

```toml
integration = "main"                  # the branch that receives the upstream changes
install = "pnpm install"              # optional: runs before the checks
checks = ["pnpm lint", "pnpm test"]   # run in order by verify

[publish]                             # optional: where and how finish publishes
remote = "origin"                     # the publish remote, a git remote of the repository
via = "push"                          # optional: "push" (default) or "merge-request"

[[upstream]]
name = "app-a"                        # unique; used on the command line, in sync records and trailers
url = "git@example.com:team/app-a.git"  # required: where the upstream is fetched from
branch = "main"                       # the upstream branch to take from
mode = "follow"                       # optional: "follow" (default) or "port"
path = "apps/app-a"                   # optional, follow mode only: merge under this path; absent = repository root
```

Validation rules. The tool refuses to run when any of them is broken, and it names the entries involved:

- Unknown keys are rejected. The settings `remote` and `mirror` of earlier releases are refused with what replaces them: upstreams are fetched by their `url`, and mirror branches are no longer kept.
- At least one `[[upstream]]` is required, and every upstream needs a `url`.
- Upstream names are unique, and no two upstreams have the same URL and branch. A name may contain only letters, digits, `-`, `_` and `.`, may not start with `-` or `.`, and, since it is part of a git reference name, may not contain `..` or end with `.` or `.lock`.
- A path is normalised: no leading or trailing slash, no empty, `.` or `..` segment, no `.git` segment in any letter case, and it may not be empty. Omit `path` for the root.
- Paths may not nest; one upstream's path cannot lie inside another's.
- An upstream at the root must be the only upstream.
- An upstream in `port` mode is taken at the repository root and may not have a path.
- `publish` is a table: `remote` is required, and `via` is `push` or `merge-request`. Write it as the `[publish]` section shown above, after the top-level keys (a section header ends the top level, so keys written below it belong to the section), or inline as `publish = { remote = "origin", via = "merge-request" }`. The string form `publish = "origin"` is rejected with the form to write instead.
- The publish remote may not be fetched from or pushed to (its `url` or any `pushurl`) a URL that names an upstream's repository, because the tool never pushes to an upstream. Two URLs name the same repository when a local path or `file://` URL resolves, against the main worktree and the way git finds a repository (`<path>/.git`, `<path>`, then `<path>.git`), to the same directory, so `../up`, `../up.git/` and the absolute path of that directory match; or when any other URL matches once the scp-like form `user@host:path` is read as `ssh://user@host/path`, scheme and host are put in lower case, a default port such as `:22` or `:443` is dropped, and a leading `/` and a trailing `/` and `.git` of the path are dropped. `doctor` compares the URLs of every remote with the upstreams' URLs the same way.

### Upstreams by URL

An upstream is fetched with `git fetch --no-tags <url> +refs/heads/<branch>:<reference>`, in the main worktree so that a relative URL means what it means there, into a reference under `refs/git-intake/`. That lives outside `refs/heads` and `refs/remotes`, so no branch listing, checkout, `git push --all` or hosting tool sees it, and no remote exists for a tool to take as the place to open pull requests in.

The references exist only while something needs them. Every command that fetches, `status`, `log`, `branches`, `doctor`, `contribute` and `start`, fetches into a namespace of its own, `refs/git-intake/read/<token>/`, and removes it before it exits, whatever its outcome; every run contacts the upstreams. `branches` fetches every branch of an upstream there. An update holds `refs/git-intake/upstreams/<name>` for each queued upstream, at the tip `start` fetched, so that its targets and planned commits stay in the repository even if the upstream moves or becomes unreachable; `continue`, `verify` and `finish` never fetch an upstream. Only `start` writes those references and only `finish` or `abort` removes them: a command that reads an upstream during the update fetches into its own namespace and never touches them. `start` saves the update before it writes them; if it is killed in between, `status`, `continue`, `verify` and `finish` restore the missing references from the update state, at the tips `start` fetched, and say so. `doctor` reports an update reference that is missing or elsewhere, and leaves alone, with a warning to keep it, a reference that still holds the commit of a missing one. It reports any other reference under `refs/git-intake/` that no update accounts for, such as the namespace of a command that was killed; nothing uses it, and `git update-ref -d <reference>` removes it.

### Modes

Each upstream has a mode, which describes the relationship to that source:

- `follow`, the default: an update merges the upstream branch up to a target, at the repository root or under the upstream's path. A fork that carries a few patches follows its upstream.
- `port`: an update takes chosen upstream commits, one by one, at the repository root. A product built on an upstream ports from it: it decides for every upstream commit whether to take it, skip it, or record that it already has it.

Switching an upstream from `follow` to `port` changes one setting. The first port update starts the journal at the last synced commit (see *Port mode*). Switching back to `follow` is allowed: the next update merges the upstream up to its target, git finds the merge base from the last merge, and changes taken in port mode come back as the same changes, which usually merge cleanly or conflict where the product changed them. The journal of an upstream in follow mode means nothing; `doctor` reports it as stale.

## The update cycle

```sh
git intake start        # fetch, fix targets and merge each follow upstream, or plan a port
# port mode: edit the plan, then
git intake continue     # carry out the plan
# on conflict: resolve in the update worktree, `git add`, then
git intake continue     # conclude the merge or copy, take in the rest
git intake verify       # run install and checks on the result
git intake finish       # fast-forward, publish, clean up
# with via = "merge-request": finish opens a merge request and the update waits;
# once the merge request is merged, run finish again to complete the update
```

Every command works the same from any directory of the repository: the main checkout, a linked worktree, or the update worktree itself. This holds for a submodule and for a repository whose git directory lies outside its checkout as well. Git operations on the repository as a whole, such as fetching, pushing and moving branches, run in the main worktree, so a relative URL means what it means there. A revision named to `start` or `contribute` is resolved in the worktree the command runs in. When `finish` or `abort` removes the update worktree it was run from, the command still completes and notes that the current directory is gone; change to another directory before the next command.

Git cannot locate the main worktree of a repository whose git directory lies outside it, without naming it, from a linked worktree. The tool then finds it in the update in progress, which `start` records. Without an update, run `start` in the main worktree; from elsewhere it refuses, `doctor` warns that it did not check the main checkout, and a relative URL is resolved against the git directory.

`start` refuses to begin if any of these hold:

- an update is already in progress;
- `.worktrees` or `.worktrees/git-intake` in the main worktree is a symbolic link or not a directory;
- the repository is bare and the path of its update worktree contains `/.git`, as when the bare repository is itself a directory named `.git`; run the tool in a clone with a checkout instead;
- the main checkout or a checkout of the integration branch has uncommitted changes to tracked files;
- the integration branch is behind its counterpart on the publish remote;
- a selected upstream is unknown, named more than once, cannot be fetched, or was rewritten;
- a revision named for an upstream is refused, as described under *Choosing a target* below, or the upstream's URL cannot be asked for its tag;
- a path upstream that has no sync record yet finds its path holding content unrelated to the upstream's history, or finds a file where a directory of its path would go;
- a path upstream that was imported before finds its path removed on the integration branch;
- a port upstream has an invalid journal, or neither a journal nor a last synced commit to start one from.

When `start` runs, it does the following:

1. Fetches the selected upstreams. With no names it selects all of them.
2. For a follow upstream, records the fetched tip, or the commit a revision names (see *Choosing a target*), as that upstream's **target**. For every upstream it records the integration tip as the **base**. Targets never change during the update, even if the upstream or a tag moves on.
3. Queues, in configuration order, the follow upstreams whose target the base does not contain yet, and a port upstream that has pending commits. The others are reported as current. If nothing is queued, nothing is created and no reference is kept.
4. Creates the worktree `.worktrees/git-intake/update` in the main worktree on the branch `git-intake/update` from the base. It is never placed inside the git directory, because compilers, bundlers and watchers ignore paths that contain `/.git`. Unless that path is ignored already, `start` adds the line `/.worktrees/` to the repository's `info/exclude` once, so the main checkout does not show the update worktree; a committed `.gitignore` is never changed. The files of the main checkout are never touched. While an update is in progress, its tree sits under `.worktrees/` in the checkout, so a tool that crawls dot-directories of the main checkout may see it. A bare repository has no main worktree, so its update worktree is `<git-common-dir>/git-intake/worktree`.
5. Enables `rerere.enabled` for the repository. A resolution recorded in one attempt is reapplied when the same conflict recurs, in a merge or in a copy, and the tool reports such files as *resolved by a recorded resolution*. Review those files and stage them like any other resolution.
6. For a port upstream, writes the plan and stops (see *Port mode*). For follow upstreams, merges each queued target with its own merge commit:
   - a root upstream: `git merge --no-ff`;
   - a path upstream, first import (no sync record for it yet, and the path is absent): an `ours` merge followed by `git read-tree --prefix=<path>/`. This keeps the upstream's full history. History shared through a common ancestor, such as a template both repositories started from, does not count as imported. The import is all or nothing: if any step fails, the merge is undone and the upstream stays pending;
   - a path upstream, later updates (it has a sync record): `git merge --no-ff -X subtree=<path>`;
   - a path upstream imported by other means, such as `git subtree add` (no sync record, but a merge on the integration line, described below, brought the upstream's history into its path): the same subtree merge, which writes the first sync record. A path that holds content no such merge brought in is refused, even when it shares a common template with the upstream.

   Merges run with `--no-log` so that git configuration cannot push the trailers out of the last paragraph. Before an upstream counts as merged, the tool checks that the commit carries its sync record for the target, that the target is merged, and that a path upstream's path exists. If a hook rewrote the message, the tool stops and says so. Each step can be repeated: after an interruption, `continue` recognises a merge that is already committed and carries its record.

On a conflict the tool stops with exit status 1. It names the upstream, lists the conflicted files, and reports which upstreams are merged and which are pending. A file is labelled *resolved by a recorded resolution* only when git's rerere reports that it resolved it (including files `rerere.autoupdate` already staged); every other conflicted file waits for a resolution by hand. Resolve the files in the worktree, stage them, and run `continue`. If one upstream cannot be resolved now, `abort` and run `start` again with only the other upstreams named. Recorded resolutions make the repeated merges cheap.

`verify` runs `install` and then every check through `sh -c` in the update worktree, once every queued upstream is merged or its plan carried out. Each command's output goes to its own log under `<git-common-dir>/git-intake/logs/`. If `install` fails, the checks are skipped. Verification fails in any of these cases:

- the `.git-intake.toml` of the update branch is missing or invalid, or names a different integration branch (the commands are then skipped);
- a command fails or is skipped;
- the commands change tracked files;
- the update branch moves while the checks run.

When it passes, `verify` records the commit it verified.

`finish` checks the following before moving any ref:

- every upstream is merged or its plan carried out;
- the worktree head is the verified commit, so any commit added after `verify` needs a new `verify`;
- the base is still the integration tip. If the integration branch moved, `abort` and `start` again so the update takes the new tip, then verify again;
- the verified commit contains the base, every merged target, and every copy and journal commit of a port, so the integration branch only fast-forwards;
- every checkout of the integration branch has no uncommitted changes and no untracked file the fast-forward would overwrite.

It then fast-forwards the integration branch to the verified commit. A checkout of that branch is updated with `git merge --ff-only`; otherwise the branch moves with `git update-ref` against its old value. In push mode (`via = "push"`, the default), `finish` pushes the integration branch to the publish remote without force. Finally it removes the worktree, the update branch, the plan, the update's upstream references and the state.

`finish` saves its planned move in the update state before it moves anything, and `status` shows the step as `finishing`. Running `finish` again after an interruption completes the move; a branch already at its planned value, or at a commit containing it, counts as done. It then publishes and cleans up. Once the branch has moved, `abort` refuses and points to `finish` and `finish --no-publish`.

The push sends exactly the planned commit (`<commit>:refs/heads/<branch>`), never whatever the local branch points to by then. A commit added to the integration branch while the update is finishing stays local.

If the push is rejected, nothing is published, the local integration branch stays advanced, and the update stays in the `finishing` step. Running `finish` again retries the same push. If the remote holds work the local branch lacks, or never accepts the push (for example a protected branch), run `finish --no-publish`. It closes the update locally: it completes the local move with the same checks, removes the worktree, the update branch, the plan, the references and the state, and names the branch it left unpublished (also in `report`). Then merge the remote's work and publish the result through a new verified update or the repository's usual review. `--no-publish` also works on a verified update before any push attempt.

### Choosing a target

By default an update takes each follow upstream at the tip it just fetched. To take an upstream at a reviewed point instead, such as a release tag, name it with a revision:

```sh
git intake start upstream=v2.3              # a fork: take the upstream's tag v2.3
git intake start app-a=1a2b3c4d app-b       # app-a at a commit, app-b at its fetched tip
```

The name ends at the first `=`, since an upstream name cannot contain one, and the rest is the revision. Upstreams named without a revision take their fetched tip, and naming an upstream twice is refused. The revision is resolved as follows:

- If the upstream's repository has a tag of that name, the tag's commit is the target, peeled through an annotated tag. A leading `refs/tags/` or `tags/` is dropped first, so `v2.3`, `tags/v2.3` and `refs/tags/v2.3` name the same tag. Looking the tag up contacts the upstream: the tool asks its URL for that one tag with `git ls-remote`, after the fetch, whenever the revision is a valid tag name, including a commit id. No tag is fetched, so the repository's own tags never clash with the upstream's. If the URL cannot be asked, `start` refuses and names the upstream, its URL and git's error.
- Otherwise the revision is resolved in the repository, like any argument to `git rev-parse`: a full or abbreviated commit id, or an expression such as `1a2b3c4d~2`. `start` fetches before it resolves the revision, so commits of the fetched branch resolve even when the repository did not have them before. A revision that resolves through one of the repository's own tags, by itself or at the start of an expression such as `v2.3~1`, is refused: the repository's tags are not the upstream's, so a tag `v2.3` in the repository never stands in for an upstream tag `v2.3`, even when the upstream has none. The tool looks for that tag by name, so it is found even when a branch has the same name.

Only a plain tag name is looked up on the upstream. An expression is always resolved in the repository, where upstream tags never are, so an expression on a tag only the upstream has, such as `v2.3~1`, names no commit and is refused; name the commit, or an expression on the fetched branch, instead.

The commit, however it was named, must be on the fetched upstream branch: the tool takes from a branch, not from arbitrary commits. Only that branch is fetched, so a commit of another upstream branch is not in the repository and names no commit. With `L` the upstream's last synced commit, `start` then decides:

- a target older than `L` is refused, because an update never moves an upstream back;
- a target the integration branch already contains, such as `L` itself, is reported as current;
- a target that does not contain `L`, such as a commit of a side branch that the upstream merged after `L`, is refused, because the last synced commit only moves forward;
- otherwise the upstream is queued at that target.

The rest of the update does not change: the sync record names the target, and `status` and `report` show the revision next to the target. Afterwards `status` shows the upstream as `behind`, by the commits between the target and the fetched tip.

### Port mode

An upstream in port mode takes chosen upstream commits. The unit is a commit on the first-parent line of the upstream branch: a hosted project lands its pull requests on that line, by squash, merge commit or rebase, so each first-parent commit is one landed change or one commit of a rebased series. A merge commit is taken as its change against its first parent (`git cherry-pick -m 1`), so a pull request merged with a merge commit is one decision.

**The journal.** A port upstream keeps its decisions in `.git-intake/<name>.toml`, committed on the integration branch and read from its tip like the configuration:

```toml
base = "<the upstream commit the port started from>"
retired = ["<upstream commit>"]   # optional: see *Recovering from a rewrite*

[[decision]]
commit = "<upstream commit>"
verdict = "skip"          # or "have": the product already has the change
note = "optional reason"
```

Commit ids are full ids; upper case is read as lower case. The tool sets the base when it creates the journal and never moves it; only the owner moves it, after a rewrite. A taken commit is recorded by the commit that took it, not in the journal. The tool refuses a journal with another verdict, a commit decided or retired twice, or ids that are not full commit ids, and names the journal and the problem. The journal is an ordinary file: a decision is reviewed in the same merge request as the code, and an ordinary commit can change it, for instance remove a skip so that its commit is pending again. When the tool records decisions, it writes the journal in the form shown above.

**Pending commits** are computed every time: the first-parent commits of the fetched upstream branch after the base, minus those that a commit on the integration branch names as taken in its trailers `Intake-Upstream: <name>` and `Intake-Taken: <upstream commit>`, minus those the journal decides. A commit copied by hand counts as taken when it carries both trailers, wherever it sits in the integration branch's history. When the base, or a decided or taken commit the journal does not retire, is no longer on the first-parent line of the fetched branch, the upstream was rewritten: `status` reports it and `start` refuses to plan it.

**Recovering from a rewrite.** When the upstream force-pushes commits the product took or decided on off its first-parent line, `status` (in its text, and in `off_line` of its JSON) and `doctor` name each such commit and what it was recorded as, and the base when it is gone too. The tool cannot tell a commit replaced by an equivalent one from a change the upstream withdrew, so it waits for the owner to acknowledge them: list them under `retired` in the journal, in an ordinary commit on the integration branch, and if the base is gone, move it to a commit of the upstream's current first-parent line. Retired commits count as neither taken nor decided, so the rewritten versions of their changes show as pending, and the next plan can mark them `have`. A retired commit needs no object in the repository; one that garbage collection pruned is retired the same way. While the base is off the line, the upstream stays rewritten.

**The first plan creates the journal.** Without a journal, the base is the upstream's last synced commit, as the newest sync record names it or, without one, as the history gives it (the newest upstream commit the integration branch contains), so an upstream followed until now, or a product created by cloning its upstream, starts porting where it is. The first update that carries out a plan commits the journal with that base. When there is neither a journal nor a last synced commit, because the product shares no history with the upstream, `status` shows the state `no base`, `status --check` fails, and `start` refuses; each says to write the journal with its base by hand.

**The plan.** `start` fetches the upstream, computes its pending commits and, when there are any, creates the update worktree and branch and writes the plan to `<git-common-dir>/git-intake/plan`, then stops and names it:

```
# git intake plan for example-app: 4 pending commits, oldest first.
# take  copy the commit onto the update branch
# skip  record that the product does not take it
# have  record that the product already has it
# later leave it pending
# Takes are copied in the order listed here. Text after the subject and ' # ' is the
# note of a skip or have decision. A commit left out of the plan stays pending.
later 1a2b3c4d5e6f fix(web): keep the composer focused after a paste
later 5d6e7f8a9b0c feat: add a settings page for keybindings
later 9a8b7c6d5e4f chore: update dependencies
later 0f1e2d3c4b5a fix(server): close the watcher on shutdown
```

Edit it, as a person or an agent, and run `continue`. A line is a verb, the commit (any unique prefix of at least four hex digits of a listed commit), and optionally the subject and a note after ` # `; comment and empty lines are ignored. `continue` refuses a plan that names a commit it does not list, names one twice, uses another verb, or decides nothing (every commit `later`), names the line, and changes nothing. `start <name>=<revision>` limits the plan to the pending commits that the revision contains, so that a review can stop at an upstream release; the revision is resolved as under *Choosing a target*, without the rules about the last synced commit.

**Carrying out the plan.** `continue` copies each commit marked `take` onto the update branch in the order the plan lists them, so a later fix can be taken before an earlier feature. A copy keeps the upstream commit's author and message and adds the trailers:

```
Intake-Upstream: example-app
Intake-Taken: <full id of the upstream commit>
```

A copy that stops on conflicts stops the update as a conflicted merge does: the tool names the upstream commit and the conflicted files, lists the commits copied and still to copy, and exits with status 1. Resolve the files, stage them, and run `continue`, which commits the copy with its trailers and goes on with the remaining takes; a copy committed by hand with both trailers is accepted too. A copy that changes nothing, because the product already has the change, is not committed; it is recorded as `have` with the note `taken, but its copy changed nothing`, and `report` names it.

When the commit of a copy fails, for example because a pre-commit hook of the repository rejects it, the update stops at the step `commit_failed` with git's output and exit status 1, and the copy stays staged in the update worktree. Fix the cause there and stage the result; `continue` commits what is staged as the copy, with the upstream author, message and trailers. `continue` never resets the worktree, so a fix made by hand, staged or not, is never thrown away. Only what is staged is committed: when a copy resumed after an interruption has its changes in the worktree but nothing staged, the update stops the same way and asks to stage what belongs to the copy, and a copy counts as changing nothing only when neither the index nor the worktree differs.

A copy never changes `.git-intake.toml` or anything under `.git-intake/`: an upstream built with git intake carries its own configuration and journals, which are not the product's. The tool restores those paths from the update branch head before it commits a copy, including paths in conflict, names the paths it left out in the output of `continue` and in `report`, and records a copy that changed nothing else as `have`, with the note `taken, but its copy changed nothing outside .git-intake.toml and .git-intake/`. Last, the `skip` and `have` decisions with their notes are committed to the journal in one commit, which also creates the journal; a plan without decisions on an existing journal adds no journal commit. Commits marked `later`, or left out of the plan, stay pending. The update then goes on to `verify` and `finish` like any other.

An upstream commit often builds on earlier ones. The tool cannot know such a dependency, but it points at a likely one: for each taken commit, `report` lists the earlier upstream commits still pending that changed any of the same files. A copy can apply cleanly and still miss what an earlier commit introduced; the checks and the review catch that, and the report says where to look.

### Contributing to an upstream

```sh
git intake contribute example-app fix-watcher        # a branch at the fetched tip
git intake contribute example-app=v1.4 fix-watcher   # a branch at the upstream's tag v1.4
```

`contribute` fetches the upstream and creates the local branch at its fetched tip, or at the commit a revision names on the fetched branch, resolved as for `start` but without the rules about the last synced commit. The branch holds only upstream history: copy the change onto it (for example with `git cherry-pick`) and push it to where pull requests to the upstream come from, such as your fork on the upstream's host. The tool sets no tracking branch, changes no checkout, and never pushes the branch. It refuses a branch that exists or a name that is not a valid branch name.

### Publication through a merge request

With `via = "merge-request"`, nobody needs to push to the integration branch, so it can stay protected. `finish` applies the same checks as above and then publishes the verified commit itself, without moving any local branch:

1. It pushes the verified commit, without force, to a new branch `git-intake/update-<commit>` on the publish remote, where `<commit>` is the first twelve hex digits of the verified commit.
2. With the same push it asks the remote to open a merge request from that branch into the integration branch, through git push options (`merge_request.create`, `merge_request.target`, `merge_request.title`, `merge_request.description`, `merge_request.remove_source_branch`), which GitLab applies while it receives the push. The title names the updated upstreams. The description lists each follow upstream's merged range, each port upstream's taken commits, skipped commits and commits already present, and the verified commit. The tool shows what the remote replies, such as the address of the new merge request, and records the merge request's number from the address GitLab gives under `View merge request for <branch>:` for the pushed branch (`.../-/merge_requests/<number>`); `status` shows it as `!<number>`.
3. It removes the update worktree and keeps the update in the step `waiting_for_merge_request`. `status` shows the merge request branch and the next command; `start`, `continue` and `verify` refuse while the update waits.

A remote applies push options only to a push that updates the branch. When the branch is already on the remote with the verified commit, as after an interrupted `finish`, the tool finds it there (looking where pushes go, which a remote's `pushurl` may set apart from where it fetches), pushes nothing and asks for no merge request; it then says so and names the merge request to open unless one is open. A remote that does not support push options refuses them before anything is written (`the receiving end does not support push options`); the tool then pushes the branch without options and names the merge request to open: from `git-intake/update-<commit>` into the integration branch.

Merge the merge request **with a merge commit or by fast-forward**, and leave its branch as it is. What lands must be the verified commit itself, or a merge of it whose tree is the verified tree. A squash or a rebase replaces the verified commit with commits that were never verified, and a rebase also rewrites the upstream merges and copies. A commit added to the merge request branch, or a merge commit made after other work landed on the integration branch, gives a tree that was never verified. In GitLab, set the project's merge method to *Merge commit with semi-linear history* or *Fast-forward merge*, which merge only a branch that contains the current target, and leave squashing off for these merge requests. With the plain *Merge commit* method, merge only while the integration branch is still where the update started.

Then run `finish` again. It looks up the merge request's head and branch on the publish remote, fetches the remote's integration branch, finds on its first-parent line the commit that brought the verified commit in, and acts on what it finds.

The merge request's head decides what was merged. GitLab keeps it as `refs/merge-requests/<number>/head`, which follows every push to the merge request branch and stays after the branch is removed. A merge request whose head is not the verified commit did not land the verified tree, whichever merge method was used: a commit added to the branch and merged by fast-forward leaves the verified commit on the integration branch, but with an unverified commit on top. The merge request branch counts as well, as listed now or, once it is gone, as the tool last saw it (when it pushed the branch, or at a later `finish`): if the integration branch took it in at another commit than the verified one, the update is refused even when the merge request's head matches. Without a number, or on a remote that keeps no such reference, the branch is all the tool can judge by. That fallback cannot tell a commit added to the branch after it was last seen and merged by fast-forward from work that landed on the integration branch afterwards: when it completes such a fast-forward, `finish` lists the commits above the verified commit that it could not attribute, so that they can be checked.

- **The verified tree landed.** The merge request's head is the verified commit, and the commit that brought it in is the verified commit itself (a fast-forward) or a merge whose tree is the verified tree and whose parents the verified commit contains. `finish` fast-forwards the local integration branch to the remote one, with the same care for its checkouts as in push mode, deletes the merge request branch if it still exists, and removes the update.
- **Nothing landed yet, and the merge request branch exists.** `finish` reports that the update waits for the merge request, changes nothing, and exits with status 1. If the merge request's head or branch no longer points at the verified commit, it says so: merging it would not land the verified tree.
- **What landed differs from the verified tree.** The integration branch contains the verified commit, but the merge request's head was another commit, or the verified commit came in through a merge with another tree. `finish` refuses and moves no branch; that merge was never verified. `abort` then ends the update (below).
- **The integration branch changed without receiving the verified commit.** When a commit with the verified tree landed instead, or the update's sync records or copy trailers did, the merge request was merged by squash or rebase; otherwise another change landed first, or the merge request was merged by squash or rebase after other changes. `finish` refuses and moves no branch. Run `abort`, bring the local integration branch up to date with the remote one, and run `start`: the upstreams are taken in again on the new tip and recorded properly.
- **The merge request branch is gone and nothing landed.** The merge request was closed or its branch deleted; `finish` refuses, and `abort` ends the update.

`finish --no-publish` keeps its meaning in this mode: it makes the local move and pushes nothing. Before the merge request is opened, it closes the update locally as in push mode. While the update waits, it moves no branch before the merge request is merged, and reports that the update waits. After the merge, it fast-forwards the local integration branch, leaves the merge request branch in place, and removes the update.

### Abort

`abort` removes the worktree, the update branch, the plan, the update's upstream references, its logs and the state at any step before `finish` moves the integration branch, which stays exactly as it was; a port's journal is unchanged. Resolutions already made by hand in a stopped merge or copy are recorded first, so a retry reapplies them.

While the update waits for its merge request, `abort` first checks the publish remote's integration branch as `finish` does:

- If the verified tree landed, `abort` refuses and names `finish`, which completes the update.
- If a merge that did not land the verified tree is there, `abort` removes the update and says that the integration branch holds an unverified merge.
- Otherwise it removes the update and names the merge request that is left to close on the forge.

In each case where it proceeds, it deletes the merge request branch from the publish remote only while that branch still points at the verified commit. A branch that is already gone counts as deleted; a branch that received other commits is left alone.

### The update state

The update state lives in `<git-common-dir>/git-intake/state.json`, which every worktree shares and no commit can carry. Its format has a version. A state written by a release with another format, such as an update started by a release that kept the update worktree in the git directory, is refused by every command that reads the update, with the advice to finish or abort that update with the release that started it.

## The sync record

Every merge commit the tool creates looks like this:

```
sync(app-a): merge app-a/main at 1a2b3c4

Intake-Upstream: app-a
Intake-Target: <full commit id>
```

Every commit it copies in port mode keeps the upstream commit's message and adds `Intake-Upstream: <name>` and `Intake-Taken: <upstream commit>`; the journal commit of a port is titled `sync(<name>): record <n> port decisions` or `sync(<name>): start the port journal at <commit>`.

The tool reads its own history along the **integration line**: the first-parent line of the integration branch, where each merge on it is followed by the commits its second parent brought in along that parent's own first-parent line, as `git rev-list --first-parent <merge>^1..<merge>^2` lists them. An update published by push, or a merge request merged by fast-forward, puts the tool's merges on the first-parent line itself; a merge request merged with a merge commit puts them right after that merge commit. Only that one level counts: a branch merged into another branch before that one was merged brings in nothing of its own. A sync merge brings in nothing either: what it merged is the upstream's history.

A follow upstream's **last synced commit** is the target of its newest sync record on the integration line whose target no other of its records' targets contains, so a branch carrying an older record that is merged after a newer one never moves the upstream back. It is not stored anywhere else. An upstream without a record has its last synced commit taken from the history instead, and the decision depends only on the integration branch and that upstream's own branch, never on other upstreams or on which of them are fetched. For a root upstream, such as a fork created by cloning, it is the newest upstream commit the integration branch contains. For a path upstream, it is the second parent of the newest merge on the integration line whose second parent belongs to the upstream's branch and which changes the upstream's path. That is the merge `git subtree add` or `git subtree pull` leaves, so ancestry shared through a template does not count, and the import still counts when another upstream later merges this one's history. `status` says so in text, and `status --json` marks the source as `history` instead of `record`. The first update writes a record. Only this repository's own records count. Records inside a configured upstream's fetched branch are skipped; this happens, for example, when a stacked branch starts from another fork's integration branch that carries that fork's records. If a follow upstream's branch no longer contains its last synced commit, the upstream was rewritten: `status` reports it and `start` refuses to update it.

## Commands

| Command | Purpose |
| --- | --- |
| `start [UPSTREAM[=REV]...]` | Starts an update, as described above. A follow upstream named as `UPSTREAM=REV` is taken at the commit REV names instead of its fetched tip (see *Choosing a target*); a port upstream's plan is limited to the pending commits REV contains. |
| `continue` | Carries out the edited plan of a port upstream, or concludes the merge or copy stopped on conflicts, and takes in the rest. It also accepts a merge concluded by hand when the commit keeps the sync record, and a copy committed by hand when it keeps both copy trailers. |
| `abort` | Removes the update worktree, branch, plan, upstream references, logs and state. While the update waits for its merge request, also deletes the merge request branch from the publish remote. |
| `verify` | Runs `install` and the checks and records the verified commit. |
| `finish [--no-publish]` | Fast-forwards and publishes the verified update by push, or opens its merge request and, run again once it is merged, completes the update. With `--no-publish` it makes only the local move, closes the update without pushing, and names the unpublished branch. |
| `status [--check] [--json]` | Fetches every upstream and shows the configuration, then for every upstream its path, mode, branch, fetched tip and one state. A follow upstream also shows its last synced commit and the number of commits the integration branch lacks, with the state `current`, `behind`, `not imported`, `path removed`, `rewritten` or `unavailable`. A port upstream shows its journal's base and its numbers of taken, decided and pending commits, with the state `current`, `pending`, `no base`, `rewritten` or `unavailable`; `no base` means it has no journal and nothing to start one from, and a rewritten one names the commits off its line. `unavailable` means the upstream could not be fetched, its branch is missing, or, for a port upstream, its journal is invalid; any upstream that cannot be fetched makes `status` exit with status 1. `path removed` means a synced path upstream, from a record or from history, whose path no longer exists on the integration branch; `start` refuses it and `status --check` counts it as drift. When several states apply, `unavailable` comes first, then `rewritten`, then `path removed`. A path upstream is `not imported` until a sync record for it exists or, without one, while its path is absent or holds content that no merge of the upstream brought in; a root upstream is `not imported` while it shares no history with the integration branch. While an update is in progress it also shows the step reached, the base, the worktree, each queued upstream's target or plan and step, and the next command. |
| `report [--json]` | For the update in progress, or the one just finished, per upstream. For a follow upstream: the merged range and commit count, each conflicted file with its resolution source (by hand or by a recorded resolution), and the size of the repository's own changes against that upstream before and after the merge. For a port upstream: the taken commits in the order copied, each with its conflicted files, the git-intake paths its copy left out and, where there are any, the earlier pending commits that change the same files; the commits recorded as present because their copy changed nothing; the skip and have decisions with their notes; and the number of commits left pending. |
| `log [UPSTREAM...] [--json]` | Fetches the selected upstreams and lists, for a follow upstream, the commits on its branch that the integration branch does not contain, and for a port upstream its pending commits, oldest first. |
| `branches [UPSTREAM...] [--json]` | Fetches every branch of the selected upstreams for the command and lists every branch that holds commits the integration branch does not contain, with their number; for a port upstream, commits a copy names as taken do not count. |
| `doctor [--json]` | Checks the repository and reports each check as pass, fail or warn. It changes nothing but the references of its own fetches, which it removes. Checks: the configuration is valid; the update in progress, if any, holds its references at the tips `start` fetched; no reference under `refs/git-intake/` is left without an update; every upstream's URL can be fetched and its branch exists; every imported path exists on the integration branch; every follow upstream's last synced commit is still contained in its branch; every port upstream's journal is valid and its base and its decided and taken commits that are not retired are still on its branch, naming each one that is not; no git remote names an upstream's repository, with a note when the remote is the one a partial clone fetches missing objects from; no journal belongs to an upstream that is missing or in follow mode; the main checkout is clean; no merge is in progress there; the update state, if any, matches its worktree and branch. It warns about upstreams that still have sync records but were dropped from the configuration. |
| `contribute UPSTREAM[=REV] BRANCH` | Creates the local branch BRANCH at the upstream's fetched tip, or at the commit REV names on its branch, for a contribution to the upstream (see *Contributing to an upstream*). Never pushes. |

`status --check` additionally predicts, with `git merge-tree`, the files a merge of each behind follow upstream would conflict in. It creates no worktree, branch or state, and fails unless every upstream is current, so a port upstream with pending commits fails it too. It can run on a schedule and gate a cutover or a port review.

### JSON output

`status`, `report`, `log`, `branches` and `doctor` accept `--json`. They then write one JSON document to standard output, with the same facts as the text form under stable snake_case field names, and exit with the same status. Notes and errors go to standard error. For example, `status --json` carries `publish` (the publish remote or null) and `publish_via` (`push`, `merge_request` or null); while an update waits for its merge request, its `update` carries `step` `waiting_for_merge_request` and `merge_request` with `remote`, `source_branch`, `target_branch`, `commit`, `requested` (whether the push that asked the remote to open the merge request transferred the branch; when false, the merge request is opened by hand) and `number` (the merge request's number from the remote's reply, or null).

Every entry of `status --json` `upstreams` carries `name`, `path`, `url`, `branch`, `mode` (`follow` or `port`), `tip`, `state` (`current`, `behind`, `pending`, `no_base`, `not_imported`, `path_removed`, `rewritten`, `unavailable`) and `detail`. A follow upstream fills `last_synced`, `last_synced_source` (`record`, `history` or null) and `commits_behind`, and with `--check` `conflicts`; a port upstream fills `base`, `journal` (whether the journal exists), `taken`, `decided` and `pending`, and when rewritten `off_line`, with `base` (the base when it is off the line, or null) and `commits` (each with `commit` and `recorded_as`: `taken`, `skip` or `have`). Fields that do not apply are null.

Each queued upstream under `update` in `status --json` carries `name`, `mode`, `fetched`, `rev`, `step` (`plan`, `pending`, `conflict`, `merged`) and `conflicts`; a follow upstream adds `target`, `kind` and `merge_commit`, a port upstream `plan` (the plan file), `commits` (each with `commit`, `subject`, `verb`, `note`, `copy` — `none`, `conflict`, `commit_failed`, `copied` or `empty` — `copied`, `failure`, git's output when the commit of the copy failed, and `left_out`) and `journal_commit`. The update's `step` is `plan` while a plan waits to be edited, and `commit_failed` while a copy whose commit failed waits for `continue`. Each entry of `report --json` `upstreams` carries `mode` and `rev`; a follow upstream fills `kind`, `range_start`, `target`, `commits`, `merge_commit`, `conflicts` and the diff sizes, and a port upstream fills `port` with `taken` (each with `commit`, `subject`, `copy`, `copied`, `conflicts`, `left_out` and `earlier_pending`), `unchanged` (each with `commit`, `subject` and `left_out`), `decisions` (each with `commit`, `subject`, `verdict` and `note`), `left_pending` and `journal_commit`. Each entry of `log --json` and `branches --json` carries `mode`.

## Exit status

| Status | Meaning |
| --- | --- |
| 0 | Success. |
| 1 | A negative result: `status --check` found drift or pending port commits, an upstream could not be fetched by `status`, `log` or `branches`, a `doctor` check failed, `verify` failed, `start`/`continue` stopped on conflicts or on the failed commit of a copy, or `finish` found that the update still waits for its merge request. |
| 2 | An error: invalid configuration, a refused command (for example `start` with an upstream that cannot be fetched, `continue` with an invalid plan, `finish` on an unverified update, or on a merge request merged by squash or without the verified tree), a git failure (including a push rejected by the publish remote, after which `finish` retries the same push and `finish --no-publish` closes the update locally), or invalid usage. |

## Moving from upstream remotes and mirror branches

Earlier releases kept a git remote per upstream, its remote-tracking branches, and optionally a mirror branch, and had a `fetch` command. This release refuses such a configuration and names what replaces each setting. To move a repository:

1. Finish or abort any update in progress with the release that started it; this release refuses its state.
2. In `.git-intake.toml`, give every upstream a `url` (the URL of its remote, `git remote get-url <remote>`), remove `remote` and `mirror`, and commit the change on the integration branch. The mode stays `follow` unless set.
3. Remove the upstream remotes with `git remote remove <remote>`, which also removes their remote-tracking branches. `doctor` names every remote left at an upstream's URL. In a partial clone, the remote that missing objects are fetched from must first provide the objects the history still lacks, or that history can no longer be read; `doctor` says so for such a remote.
4. A mirror branch stays an ordinary branch; delete it locally and on the publish remote if nothing else uses it.
5. Replace scripts that ran `git intake fetch`: `status`, `log`, `branches`, `doctor` and `start` fetch what they need.

## Monorepo migration

1. Commit a configuration that lists one `[[upstream]]` per source repository, each with its own `path` and `url`.
2. Run `git intake start`. Each source is imported under its path with its full history, then verified and finished like any update. A path imported earlier with `git subtree add` is adopted by its first update. A path that holds content unrelated to the source's history is refused, and so is an import that would replace a file.
3. Until cutover, keep taking the sources in with the same cycle. Conflicts against the monorepo's new layout are resolved once and recorded. Files moved within an upstream's path merge as renames. Files moved out of the path may conflict or arrive at their old location; resolve them like any other conflict.
4. Run `git intake status --check` on a schedule. It fails whenever a source has moved on and lists the files that would conflict. Zero drift, meaning every upstream is current, is the cutover gate.
5. Before freezing the sources, run `git intake branches` to find work left on other branches of each source. It reports when no upstream holds unmerged work.
6. After cutover, remove an upstream from the configuration. `doctor` keeps warning about upstreams that are dropped but still have sync records, so the removal is visible.

## Safety properties

- The tool never rewrites history and never force-pushes. Local branches only fast-forward.
- It never pushes to an upstream. The publish remote may not be fetched from or pushed to an upstream's URL, and the tool keeps no remote for an upstream: a reference to an upstream exists only while a command or an update needs it.
- No file of the main checkout is changed. The update lives in its own worktree, `.worktrees/git-intake/update` in the main worktree (in the git directory of a bare repository), on `git-intake/update`, with its state and plan in `<git-common-dir>/git-intake/`, which every worktree shares and no commit can carry.
- Each target is fixed when the update starts, and an update keeps the references that hold its targets and planned commits until it ends. What is merged or copied, verified and published is the same commit.
- `finish` moves no ref unless the verified commit is the worktree head, the base is still the integration tip, and the move is a fast-forward. It validates the move before making it, and it can be resumed.
- Publication by push pushes exactly the planned commit. Nothing unverified is ever published by a retry, and `finish --no-publish` closes an update that cannot be published.
- Publication through a merge request pushes only the verified commit, to a branch of its own. The local integration branch moves only after the publish remote's integration branch received the verified commit, by fast-forward or by a merge whose tree is the verified tree. A merge by squash or rebase is refused, and so is a merge request whose head, as the remote keeps it or as the tool last saw its branch, is not the verified commit, or whose merge brought in another tree. Without the merge request's head, commits above a fast-forwarded verified commit cannot be attributed; `finish` lists them.
- The tool never adds or changes a git remote, and never fetches an upstream through one.
- A rewritten upstream is detected from the sync record or the port journal and copies, and is not taken.
- The configuration and a port's journal are read from the committed integration tip, never from a working copy or from the tree being taken in.
