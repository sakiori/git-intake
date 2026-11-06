## ADDED Requirements

### Requirement: Pushing with the CI job token
Where `CI_JOB_TOKEN` is set, `sync` in merge-request mode and `release tag` SHALL refuse with status 2 at the start of every run, pushing nothing and changing no merge request, when a push to the publish remote would use that token. `sync` in push mode SHALL warn instead and go on, since a project may let the job token push.

#### Scenario: A rewrite of the runner, with nothing to do
- **WHEN** `sync` runs in merge-request mode in a job whose push URL carries `CI_JOB_TOKEN`, and every upstream is current
- **THEN** it exits with status 2, says that the push URL carries the job token, pushes nothing and changes no merge request

#### Scenario: A helper of the runner
- **WHEN** `sync` runs in merge-request mode in a job whose push URL has no password, and git's credential helpers answer that URL with `CI_JOB_TOKEN`
- **THEN** it exits with status 2 and says that the credential helpers answer with the job token

#### Scenario: A credential of the job's own
- **WHEN** the push URL carries another password, or the helpers answer with another token
- **THEN** the run goes on, whatever user name the URL or the helper gives

#### Scenario: A release tag
- **WHEN** `release tag` runs in a job whose push URL carries `CI_JOB_TOKEN`
- **THEN** it exits with status 2 and creates no tag

#### Scenario: Push mode
- **WHEN** `sync` runs in push mode in a job whose push URL carries `CI_JOB_TOKEN`
- **THEN** it warns of the job token and goes on

#### Scenario: Outside a job
- **WHEN** `CI_JOB_TOKEN` is not set
- **THEN** neither the push URL nor the credential helpers are checked

### Requirement: Telling a push with the CI job token
A push SHALL count as using the token when a push URL of the remote as git resolves it carries the token as its password, or when that URL is an `http` or `https` URL without a password and git's credential helpers answer with the token, asked without any prompt. The refusal SHALL say which of the two it found and where the README shows how to push instead, and SHALL show neither the URL nor the token.

#### Scenario: A refusal shows no secret
- **WHEN** `sync` refuses a push URL that carries `CI_JOB_TOKEN`
- **THEN** its message shows neither that URL nor the token
