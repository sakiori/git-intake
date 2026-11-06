## Why

`doctor` warns on a GitLab project whose squash option is `default_on` or `default_off` and suggests `Do not allow`. For a fork that squashes its features and not its updates, that setting is the intended one: the tool opens and rewrites the update's merge request with squashing off, and only `always` overrides that. The warning counts a correct setup as a problem, and its advice would take squashing away from every other merge request.

## What changes

- A GitLab project that leaves squashing optional passes the merge settings check, with a note that says what the tool does and advises no change of the setting.
- GitHub keeps its warning: a pull request cannot ask for one merge method, so the person merging chooses each time.

## Capabilities

### New capabilities

None.

### Modified capabilities

- `git-intake`: the merge settings check of `doctor`.

## Impact

- `doctor --json` reports the note with the status `pass` under `merge_settings`, where it reported `warn`.
