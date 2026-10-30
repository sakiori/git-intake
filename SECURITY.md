# Security policy

## Supported versions

Security fixes are made on `main` and ship in the next release. Only the latest release is supported; while the version is `0.x`, a fix is not backported to an earlier minor version.

## Reporting a vulnerability

Report a vulnerability privately through GitHub's private vulnerability reporting: open the repository's **Security** tab and choose **Report a vulnerability**, or go to <https://github.com/sakiori/git-intake/security/advisories/new>. If you cannot use GitHub, write to <oss@sakiori.app>.

Please do not open a public issue or pull request for a vulnerability before a fix is released. Include the release (`git intake --version`), the git version, the platform, and the steps or a repository that shows the problem.

## Verifying a release

Every archive of a release has a `.sha256` file next to it and a build provenance attestation made by the release workflow. Check both before installing:

```sh
sha256sum -c git-intake-vX.Y.Z-<target>.tar.gz.sha256
gh attestation verify git-intake-vX.Y.Z-<target>.tar.gz --repo sakiori/git-intake
```

The second command needs the GitHub CLI; it confirms that the archive was built by this repository's workflows. [RELEASING.md](RELEASING.md#checking-a-release) shows how to tie it to the tag's commit as well.

## Threat model

`git intake` merges or copies commits from upstream repositories into a downstream repository, runs the downstream's checks on the result and publishes it. The README's *Safety properties* and *Code from upstreams in CI* sections describe the guarantees in full; this section summarises what they cover and what they leave to the repository's owner.

### Configuration runs code

The install command and the checks of `.git-intake.toml` are shell commands. `verify`, and `finish` and `sync` when they verify, run each of them through `sh -c` in the update worktree, with the user's privileges and the process environment. Two kinds of variables differ: git's location variables (`GIT_DIR`, `GIT_WORK_TREE`, `GIT_INDEX_FILE`, `GIT_PREFIX`, `GIT_COMMON_DIR`) are removed, and the `GIT_INTAKE_` variables the tool inherited are replaced by those of the update. Anyone who can change those commands, or the files they run, can run code wherever the tool runs.

That includes upstreams:

- The checks run the tree being verified, which holds the upstream code the update brings in: a test suite or build script an upstream changed runs as soon as the update is verified.
- An update's commands are those of the configuration at the integration branch tip when it started. After a catch-up merge they are those of the configuration at that merge, which also holds what the update merged; in a follow upstream at the repository root, an upstream's change of `.git-intake.toml` is merged like any other file. Once an update lands, its configuration is the one the next update starts from.
- In merge-request mode the forge's pipeline runs the CI definition of the update's head, which an upstream may have changed.

When an update changes the install command or the checks, or the forge's CI definition, `finish` and `sync` name the change in their output, in push mode as in merge-request mode, and in the merge request's description. In push mode the CI definition counts when the tool can tell the forge without its API (the setting `forge`, the publish remote's host or the CI job), on GitLab as the default `.gitlab-ci.yml`. They do not stop the commands from running. Treat every upstream as code the repository chooses to run, and review an update before it runs where secrets are available, or run it where none are.

The checks are not shielded from credentials: a forge token in `GITLAB_TOKEN`, `GH_TOKEN` or the other token variables is visible to them, and so is a push credential in the clone's git configuration. In CI, give the job that runs `sync` only the credential it pushes with, keep deploy secrets in protected variables or protected environments that an update's branch cannot reach, and keep the token `sync` uses there too.

### What the tool protects

- **Only a verified tree is published.** In push mode, the integration branch moves only to a commit `verify` passed, by fast-forward, and only while that commit is the worktree head and the base is still the integration tip. In merge-request mode, the tool pushes only to a forge whose settings require a successful pipeline and merge exactly the tree it tested, and it reads those settings before every push.
- **History is never rewritten.** The tool never force-pushes; local branches and the update branch only fast-forward, and a branch the tool did not leave is never deleted or overwritten. A remote update branch is deleted only under a lease on the commit the tool read (`--force-with-lease`), so a branch someone else moved meanwhile stays.
- **Nothing is pushed to an upstream.** The tool compares repositories as git resolves them: URLs as configured and after git's `insteadOf` and `pushInsteadOf` rewriting; hosted URLs by host and path, whatever their scheme, user, default port and the case of the path, with SSH host aliases resolved; local paths as the canonical repository git reports, a link or a linked worktree of it included. The publish remote may not be fetched from or pushed to an upstream's repository, every push and branch deletion is checked again where git sends it, and the tool keeps no remote for an upstream.
- **Only the tool's own files are removed.** The tool removes only the update worktree git registers at the tool's place for it, reached through directories alone, and its own state, plan and logs under the git directory, never through a symbolic link. An update whose tree puts anything at that place, such as a link at `.worktrees`, is refused before any branch moves.
- **Forge tokens stay private.** A token is read from the environment only, sent only to the API of the publish remote's own host, not to the host of a redirect, and never printed, stored or logged. A github.com token is not sent to a GitHub Enterprise Server, except the token an Actions job of that server is given. The publication setting that chooses the forge is read from the integration branch tip, never from an update's head, so an upstream cannot point the token at another host. URLs in messages carry no password, nor the user of an `http` or `https` URL, and an upstream URL that carries either is refused, so no credential is committed with the configuration.
- **Targets are fixed.** Each upstream's target is fixed when the update starts, so what is merged or copied, verified and published is the same commit, and a rewritten upstream is detected and not taken.
- **The configuration is the committed one.** `.git-intake.toml` and a port's journal are read from the committed integration tip, never from a working copy; a commit copied in port mode never changes `.git-intake.toml` or `.git-intake/`.

### What the tool does not protect

- The effects of the install command, the checks and the forge's pipeline, including what they do with the secrets and credentials they can reach.
- The content of upstream changes: a merge brings in whatever the upstream committed, and a passing check is only as strong as the checks.
- The forge's own configuration beyond the merge settings the tool reads, and the permissions of the tokens and credentials it is given.
- The git installation, the repository's own git configuration and hooks, and the machine or CI runner the tool runs on.
