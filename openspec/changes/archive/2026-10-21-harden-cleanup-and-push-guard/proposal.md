## Why

An upstream could bring a `.worktrees` link, or a file at `.worktrees/git-intake/update`, into the tree `finish` checks out in the main worktree, and a later cleanup removed a directory outside the repository reached through the link.

The rule that the tool never pushes to an upstream compared URLs as written, and only when the configuration was read: a push URL with credentials, an `insteadOf` or `pushInsteadOf` alias, or a `file://localhost/` URL let `sync` push into an upstream's repository.

Messages could show credentials: the refusal of an API URL that carries them, a forge's message that quotes the token, and a git command line that could not be started.

## What changes

- `finish`, and every fast-forward of the main checkout, refuses before any branch moves a tree that puts anything but a directory at `.worktrees` or `.worktrees/git-intake`, or anything at `.worktrees/git-intake/update`, naming the path.
- The tool removes the update worktree only where it placed it, reached through directories alone, and only when git registers it there for the repository. A registered worktree reached otherwise is refused; a directory git does not register is left and named, unless it is empty. The tool's own files are removed only inside its directory in the git directory, reached through directories alone.
- Remotes and URLs are compared with upstreams as git resolves them, with its URL rewriting, and without credentials; a `file://` URL is a local path whatever its host. The comparison is made when the configuration is read, by `doctor`, and before every push and branch deletion.
- Every message, in text or JSON, goes through one path that removes credentials from URLs and every token the tool reads.

## Capabilities

### New capabilities

None.

### Modified capabilities

- `git-intake`: comparison of URLs with upstreams; new requirements for the place of the update worktree in an update's tree, for removing only what the tool created, for pushes that never reach an upstream, and for messages without credentials.

## Impact

- A publish remote whose push URL names an upstream's repository only after git's rewriting, or only without its credentials, is now refused when the configuration is read, by `doctor`, and before a push.
- An update whose tree occupies the place of the update worktree can no longer land; the path is removed on the update branch first.
- A directory left at the place of the update worktree that git does not register is no longer removed by `abort` or `finish`; `start` names it, and the user removes it.
- A token value never appears in output; where a forge's message quotes it, the message shows `[hidden]` in its place.
