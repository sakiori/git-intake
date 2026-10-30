## Why

`verify` runs every check, in order, every time. A check that fails late, or a slow suite, makes each attempt to fix one failure pay for all the others: there is no way to run only the check being fixed in the update worktree, with the update's environment, and see its log. Running the command by hand loses the variables `verify` gives and the guarantee that the tree is the committed one.

Checks also have no identity. Output, logs and the merge request's note about changed checks can only quote the command line, which says little when it is long, and gives nothing stable to select by.

## What changes

- Each check gets a name: `checks = [{ name = "lint", run = "pnpm lint" }]`. Names are unique ignoring letter case and limited to ASCII letters, digits, `-`, `_` and `.`. The list of plain command strings is refused with a message that shows the table form; there is no second form.
- `verify --only <name>` and `verify --skip <name>`, each repeatable and mutually exclusive, run the install command and the selected checks under the same preconditions and with the same environment as a full `verify`. An unknown name is refused, naming the configured checks.
- A run that leaves out a check is a **partial run**. It never counts as verification: in push mode `finish` refuses unless a full run passed on the head, and `sync` keeps running every check. A partial run is recorded beside the full verification, with its own logs, and never replaces or completes it; partial runs never add up to a full one. `status` shows both.
- Output and logs name checks by their names, and the merge request's note about an update that changes its own checks names each check added, removed or changed.
- The update state format becomes 6, since the configuration it records now carries named checks. An update of format 4 or 5 that waits for its merge request is still handed over to it, even while the committed configuration is in the old form; any other update of an earlier format is refused with the advice to finish or abort it with a release of that format. `migrate`, which carried format 3 over to the current format, is removed: v0.3.6 still carries it over.

## Capabilities

### New capabilities

None.

### Modified capabilities

- `git-intake`: the fork configuration, verification of the published tree, the resumable state, changes to the update's own checks, the shallow-repository rule; new requirements for selecting checks and for partial runs; migration of an update in progress is removed.

## Impact

- The change of form is a flag day: a release before this one refuses the table form, and this one refuses the plain list. Upgrade in this order: finish or abort every update in progress with its own release, except one waiting for its merge request; convert the checks on the integration branch and upgrade every runner, CI jobs included, together; then hand over a waiting update with `finish`.

- Every `.git-intake.toml` must rewrite its checks as tables before this release reads it; until then every command refuses the configuration and shows the form to use.
- An update in progress started by an earlier release, other than one waiting for its merge request, cannot be carried on by this release; finish or abort it first.
- Scripts that parse `verify` output or log file names see check names instead of command lines and numbered files.
- `status --json` gains `update.partial_run`, and its `checks` becomes a list of objects with `name` and `run`; other fields keep their meaning.
- The format list names v0.3.7 as the first release of format 6, on the assumption that this change ships in the release after v0.3.6 together with the forge API work; the number is checked again when it is released.
