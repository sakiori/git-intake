## MODIFIED Requirements

### Requirement: Forge checks in doctor
In merge-request mode `doctor` SHALL check that the forge is recognised, that a token is set and accepted, and that the merge settings qualify, failing a check that does not hold. On GitHub it SHALL warn when squash or rebase merges stay selectable. On GitLab a squash option of `default_on` or `default_off` SHALL pass, with a note that the tool opens and rewrites the update's merge request with squashing off, and SHALL NOT advise changing the option.

#### Scenario: Doctor with unqualified settings
- **WHEN** `doctor` runs in merge-request mode on a GitLab project that does not require a successful pipeline
- **THEN** the merge settings check fails and names the setting

#### Scenario: Squashing left optional on GitLab
- **WHEN** `doctor` runs in merge-request mode on a GitLab project whose settings qualify and whose squash option is `default_on`
- **THEN** every merge settings check passes, and one of them names the option and says that the update's merge request has squashing off

#### Scenario: Squash merges selectable on GitHub
- **WHEN** `doctor` runs in merge-request mode on a GitHub repository whose settings qualify and that allows squash merges
- **THEN** it warns that squash merges stay selectable
