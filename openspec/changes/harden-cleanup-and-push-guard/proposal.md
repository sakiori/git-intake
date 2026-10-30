## Why

Three guarantees of the tool could be broken by what an upstream brings or by how git is configured.

An upstream can bring a `.worktrees` symbolic link, or a file or directory at `.worktrees/git-intake/update`, into the tree. `finish` checked that tree out into the main checkout, replacing the directory that held the update worktree, and a later cleanup removed a directory reached through the link, outside the repository, as if it were the update worktree.

The rule that the tool never pushes to an upstream compared the URLs as configured. A push URL with credentials, a `url.<base>.insteadOf` or `url.<base>.pushInsteadOf` alias, or a `file://localhost/` URL named the upstream's repository under another spelling, and `sync` pushed into it. The rule was also checked only when the configuration was read, not before the push.

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
