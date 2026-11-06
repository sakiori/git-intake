//! GitHub's REST API, at the pinned version `2022-11-28`, which every
//! supported GitHub Enterprise Server release knows.
//!
//! The rules for a branch come from `GET /repos/{o}/{r}/rules/branches/{b}`,
//! which returns the active rules of repository and organisation rulesets to
//! any token that can read the repository. Classic branch protection is
//! readable only with administration access, and the repository's merge
//! settings only with push access; whatever the token cannot read and the
//! rules do not settle is unknown, not false.

use std::collections::BTreeSet;

use anyhow::{Context, Result};
use serde_json::{Value, json};
use url::Url;

use super::http::{Auth, Client, Method, PER_PAGE};
use super::{
    Account, Failure, Forge, Kind, Location, Merge, MergeRequest, Pipeline, PipelineState, Queued,
    ReleaseLookup, Review, State, endpoint, failure, instant, pipeline_state, text,
};

/// The API version every request asks for.
pub const API_VERSION: &str = "2022-11-28";

/// The longest pull request body GitHub takes.
const DESCRIPTION_LIMIT: usize = 65_536;

/// Where GitHub Actions reads its workflows.
pub(crate) const WORKFLOWS: &str = ".github/workflows/";

pub struct GitHub {
    location: Location,
    client: Client,
    owner: String,
    repo: String,
}

impl GitHub {
    pub fn new(location: Location) -> Self {
        let client = Client::new(
            location.api.clone(),
            location.credential.as_ref().map(|c| c.secret.clone()),
            Auth::Bearer,
            vec![
                ("Accept", "application/vnd.github+json"),
                ("X-GitHub-Api-Version", API_VERSION),
            ],
            "GitHub",
        );
        let (owner, repo) = location
            .project
            .split_once('/')
            .map(|(o, r)| (o.to_string(), r.to_string()))
            .unwrap_or_default();
        Self {
            location,
            client,
            owner,
            repo,
        }
    }

    fn url(&self, rest: &[&str], query: &[(&str, &str)]) -> Url {
        let mut segments = vec!["repos", self.owner.as_str(), self.repo.as_str()];
        segments.extend(rest);
        endpoint(self.client.base(), &segments, query)
    }

    fn get(&self, rest: &[&str]) -> Result<Value> {
        self.client.json(Method::Get, &self.url(rest, &[]), None)
    }

    fn pulls(&self, source: &str, target: Option<&str>, state: &str) -> Result<Vec<MergeRequest>> {
        let head = format!("{}:{source}", self.owner);
        let mut query = vec![
            ("state", state),
            ("head", head.as_str()),
            ("sort", "updated"),
            ("direction", "desc"),
        ];
        if let Some(target) = target {
            query.push(("base", target));
        }
        let list = self
            .client
            .list(&self.url(&["pulls"], &query), "pull requests")?;
        // Only the repository's own pull requests: one from a fork in the
        // same organisation matches the head filter too, and one whose fork
        // was deleted has no head repository at all.
        list.iter()
            .filter(|v| from_the_repository(v))
            .map(pull)
            .collect()
    }

    /// The active rules of repository and organisation rulesets for `target`.
    fn rules(&self, target: &str) -> Result<Value> {
        let url = self.url(&["rules", "branches", target], &[]);
        let rules = self.client.send(Method::Get, &url, None)?;
        match rules.status {
            // An instance without rulesets, or a plan without them, has none.
            404 => Ok(Value::Array(Vec::new())),
            _ if rules.success() => Ok(rules.json),
            status => Err(Failure::answered(
                status,
                self.client.refusal(Method::Get, &url, &rules),
            )
            .into()),
        }
    }

    /// The classic protection of `target`, as far as the token can read it.
    fn protection(&self, target: &str) -> Result<Classic> {
        let branch_url = self.url(&["branches", target], &[]);
        let branch = self.client.send(Method::Get, &branch_url, None)?;
        let protected = match branch.status {
            404 => false,
            _ if branch.success() => {
                branch.json.get("protected").and_then(Value::as_bool) == Some(true)
            }
            status => {
                return Err(Failure::answered(
                    status,
                    self.client.refusal(Method::Get, &branch_url, &branch),
                )
                .into());
            }
        };
        if !protected {
            return Ok(Classic::None);
        }
        let url = self.url(&["branches", target, "protection"], &[]);
        let reply = self.client.send(Method::Get, &url, None)?;
        Ok(classic(reply.status, reply.json))
    }

    /// Every item a commit's paged object lists under `key`, and how many it
    /// says there are.
    fn commit_list(&self, sha: &str, list: &str, key: &str) -> Result<(Vec<Value>, u64)> {
        let per_page = PER_PAGE.to_string();
        let mut items = Vec::new();
        let mut page = 1usize;
        loop {
            let number = page.to_string();
            let url = self.url(
                &["commits", sha, list],
                &[("per_page", per_page.as_str()), ("page", number.as_str())],
            );
            let reply = self.client.json(Method::Get, &url, None)?;
            let got = reply
                .get(key)
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let last = got.len() < PER_PAGE;
            items.extend(got);
            if last {
                let total = reply
                    .get("total_count")
                    .and_then(Value::as_u64)
                    .unwrap_or(items.len() as u64);
                return Ok((items, total));
            }
            page += 1;
        }
    }
}

/// An error reading what a pull request's head needs, with the permission
/// that gives it when the forge refused for lack of one.
fn with_permission(err: anyhow::Error, permission: &str) -> anyhow::Error {
    if failure(&err).is_some_and(|f| matches!(f.status, Some(401 | 403 | 404))) {
        return err.context(format!(
            "`land` reads the checks of the pull request's head, which on a private repository takes {permission}"
        ));
    }
    err
}

/// The checks that branch protection or a ruleset requires of `target`.
fn required_checks(rules: &Value, classic: &Classic) -> Vec<String> {
    let names = |list: Option<&Value>, key: Option<&str>| -> Vec<String> {
        list.and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|c| match key {
                Some(key) => c.get(key).and_then(Value::as_str),
                None => c.as_str(),
            })
            .map(str::to_string)
            .collect()
    };
    let mut required = Vec::new();
    for rule in rules.as_array().into_iter().flatten() {
        if rule.get("type").and_then(Value::as_str) == Some("required_status_checks") {
            required.extend(names(
                rule.pointer("/parameters/required_status_checks"),
                Some("context"),
            ));
        }
    }
    if let Classic::Read(p) = classic
        && let Some(checks) = p.get("required_status_checks")
    {
        required.extend(names(checks.get("contexts"), None));
        required.extend(names(checks.get("checks"), Some("context")));
    }
    required
}

/// Whether a pull request comes from a branch of its base repository
/// itself: its head repository is the base repository, compared by id, so
/// that a renamed or transferred repository keeps its pull requests. A
/// deleted fork leaves no head repository.
fn from_the_repository(v: &Value) -> bool {
    let id = |side: &str| {
        v.pointer(&format!("/{side}/repo/id"))
            .and_then(Value::as_u64)
    };
    id("head").is_some() && id("head") == id("base")
}

fn pull(v: &Value) -> Result<MergeRequest> {
    let number = v
        .get("number")
        .and_then(Value::as_u64)
        .context("GitHub answered with a pull request without a number")?;
    let merged_at = instant(v, "merged_at");
    let state = match (v.get("state").and_then(Value::as_str), &merged_at) {
        (Some("open"), _) => State::Open,
        (_, Some(_)) => State::Merged,
        _ => State::Closed,
    };
    let side = |key: &str, field: &str| v.get(key).map(|s| text(s, field)).unwrap_or_default();
    Ok(MergeRequest {
        number,
        url: text(v, "html_url"),
        state,
        head: side("head", "sha"),
        source: side("head", "ref"),
        target: side("base", "ref"),
        description: text(v, "body"),
        ended_at: match state {
            State::Open => None,
            _ => instant(v, "closed_at").or(merged_at),
        },
    })
}

impl Forge for GitHub {
    fn kind(&self) -> Kind {
        Kind::GitHub
    }

    fn location(&self) -> &Location {
        &self.location
    }

    fn check_access(&self) -> Result<()> {
        self.get(&[]).map(|_| ())
    }

    fn merge_requests(
        &self,
        source: &str,
        target: Option<&str>,
        open_only: bool,
    ) -> Result<Vec<MergeRequest>> {
        self.pulls(source, target, if open_only { "open" } else { "all" })
    }

    fn merge_request(&self, number: u64) -> Result<MergeRequest> {
        pull(&self.get(&["pulls", &number.to_string()])?)
    }

    fn open(
        &self,
        source: &str,
        target: &str,
        title: &str,
        description: &str,
    ) -> Result<Option<MergeRequest>> {
        let url = self.url(&["pulls"], &[]);
        let body = json!({ "title": title, "head": source, "base": target, "body": description });
        let reply = self.client.send(Method::Post, &url, Some(&body))?;
        if reply.success() {
            return pull(&reply.json).map(Some);
        }
        // GitHub refuses a second pull request for the same head and base
        // as a validation failure; whether that is the reason is read back.
        if reply.status == 422 && !self.pulls(source, Some(target), "open")?.is_empty() {
            return Ok(None);
        }
        Err(Failure::answered(
            reply.status,
            self.client.refusal(Method::Post, &url, &reply),
        )
        .into())
    }

    fn update(&self, number: u64, title: &str, description: &str) -> Result<MergeRequest> {
        let body = json!({ "title": title, "body": description });
        pull(&self.client.json(
            Method::Patch,
            &self.url(&["pulls", &number.to_string()], &[]),
            Some(&body),
        )?)
    }

    fn close(&self, number: u64) -> Result<MergeRequest> {
        let body = json!({ "state": "closed" });
        pull(&self.client.json(
            Method::Patch,
            &self.url(&["pulls", &number.to_string()], &[]),
            Some(&body),
        )?)
    }

    fn review(&self, target: &str) -> Result<Review> {
        let repository = self.get(&[])?;
        let rules = self.rules(target)?;
        let classic = self.protection(target)?;
        Ok(review(target, &repository, &rules, &classic))
    }

    fn description_limit(&self) -> usize {
        DESCRIPTION_LIMIT
    }

    fn release(&self, tag: &str) -> Result<ReleaseLookup> {
        let list = self.url(&["releases"], &[("per_page", "1")]);
        let reply = self.client.send(Method::Get, &list, None)?;
        match reply.status {
            404 => return Ok(ReleaseLookup::Hidden),
            _ if reply.success() => {}
            _ => anyhow::bail!("{}", self.client.refusal(Method::Get, &list, &reply)),
        }
        let one = self.url(&["releases", "tags", tag], &[]);
        let reply = self.client.send(Method::Get, &one, None)?;
        match reply.status {
            404 => Ok(ReleaseLookup::Unpublished),
            _ if reply.success() => Ok(ReleaseLookup::Published),
            _ => anyhow::bail!("{}", self.client.refusal(Method::Get, &one, &reply)),
        }
    }

    fn with_pipeline(&self, number: u64) -> Result<(MergeRequest, Option<Pipeline>)> {
        let mr = self.merge_request(number)?;
        if mr.head.is_empty() || mr.state != State::Open {
            return Ok((mr, None));
        }
        let (runs, total) = self
            .commit_list(&mr.head, "check-runs", "check_runs")
            .map_err(|err| {
                with_permission(
                    err,
                    "*Checks: read*, which a fine-grained personal access token cannot have: use a GitHub App's \
                     installation token, or a classic personal access token with the `repo` scope",
                )
            })?;
        let (statuses, _) = self
            .commit_list(&mr.head, "status", "statuses")
            .map_err(|err| with_permission(err, "*Commit statuses: read*"))?;
        let required = required_checks(&self.rules(&mr.target)?, &self.protection(&mr.target)?);
        let pipeline = commit_pipeline(
            &CommitChecks {
                runs: &runs,
                runs_total: total,
                statuses: &statuses,
                required: &required,
            },
            format!("{}/checks", mr.url),
        );
        Ok((mr, pipeline))
    }

    fn merge(&self, number: u64, head: &str) -> Result<Merge> {
        let url = self.url(&["pulls", &number.to_string(), "merge"], &[]);
        let body = json!({ "sha": head, "merge_method": "merge" });
        let reply = self.client.send(Method::Put, &url, Some(&body))?;
        match reply.status {
            _ if reply.success() => Ok(Merge::Accepted),
            409 => Ok(Merge::HeadMoved),
            405 | 422 => Ok(Merge::Refused(reply.message())),
            status => Err(Failure::answered(
                status,
                self.client.refusal(Method::Put, &url, &reply),
            )
            .into()),
        }
    }

    fn merge_blocker(&self, _number: u64) -> Result<Option<String>> {
        Err(no_queue())
    }

    fn account(&self, _username: &str) -> Result<Option<Account>> {
        Err(no_queue())
    }

    fn assigned(&self, _target: &str, _account: &Account) -> Result<Vec<u64>> {
        Err(no_queue())
    }

    fn queued(&self, _number: u64) -> Result<Queued> {
        Err(no_queue())
    }

    fn assigned_at(&self, _number: u64, _account: &Account) -> Result<Option<jiff::Timestamp>> {
        Err(no_queue())
    }

    fn assign(&self, _number: u64, _assignees: &[u64]) -> Result<()> {
        Err(no_queue())
    }

    fn comment(&self, _number: u64, _body: &str) -> Result<()> {
        Err(no_queue())
    }
}

/// What GitHub says of the checks of a commit.
struct CommitChecks<'a> {
    runs: &'a [Value],
    /// How many check runs GitHub says there are.
    runs_total: u64,
    statuses: &'a [Value],
    /// The checks a rule or the branch's protection requires.
    required: &'a [String],
}

/// The pipeline of a commit: its check runs and its commit statuses taken
/// together, failed when one failed, running while one has no outcome, and
/// passed when every one passed or, for a check run, ended neutral or
/// skipped. A required check that has not registered yet, or a list that came
/// back short, is no outcome either. `None` without any.
fn commit_pipeline(checks: &CommitChecks, url: String) -> Option<Pipeline> {
    let mut states = Vec::new();
    let mut seen = Vec::new();
    for run in checks.runs {
        seen.push(text(run, "name"));
        let conclusion = run.get("conclusion").and_then(Value::as_str);
        states.push(
            match (run.get("status").and_then(Value::as_str), conclusion) {
                (Some("completed"), Some("success" | "neutral" | "skipped")) => {
                    PipelineState::Passed
                }
                (Some("completed"), Some("cancelled")) => PipelineState::Canceled,
                (Some("completed"), _) => PipelineState::Failed,
                _ => PipelineState::Running,
            },
        );
    }
    for s in checks.statuses {
        seen.push(text(s, "context"));
        states.push(pipeline_state(
            s.get("state").and_then(Value::as_str).unwrap_or_default(),
        ));
    }
    let unsettled = (checks.runs.len() as u64) < checks.runs_total
        || checks.required.iter().any(|r| !seen.contains(r));
    let state = if states.is_empty() {
        return None;
    } else if states.contains(&PipelineState::Failed) {
        PipelineState::Failed
    } else if states.contains(&PipelineState::Canceled) {
        PipelineState::Canceled
    } else if unsettled || states.contains(&PipelineState::Running) {
        PipelineState::Running
    } else {
        PipelineState::Passed
    };
    Some(Pipeline { state, url })
}

/// What GitHub has no counterpart for.
fn no_queue() -> anyhow::Error {
    anyhow::anyhow!(
        "GitHub has no merge queue that takes the pull requests assigned to an account; `queue` is a GitLab setting"
    )
}

/// The classic protection a reply to `GET .../branches/{b}/protection`
/// shows. `protected` on the branch also counts rulesets, so a branch
/// covered by rulesets alone answers 404 with the message `Branch not
/// protected`: it has no classic protection. A 404 `Not Found`, or 401 or
/// 403, is what a token without administration access gets.
fn classic(status: u16, json: Value) -> Classic {
    match status {
        200..=299 => Classic::Read(json),
        404 if json.get("message").and_then(Value::as_str) == Some("Branch not protected") => {
            Classic::None
        }
        _ => Classic::Unreadable(status),
    }
}

/// The classic branch protection of the integration branch, as far as the
/// token can read it.
pub enum Classic {
    /// The branch is not protected.
    None,
    Read(Value),
    /// Protected, but reading the protection was answered with this status.
    Unreadable(u16),
}

/// What a repository, the rules for `target` and its classic protection mean
/// for an update's pull request.
pub fn review(target: &str, repository: &Value, rules: &Value, classic: &Classic) -> Review {
    let rules: Vec<&Value> = rules
        .as_array()
        .map(|r| r.iter().collect())
        .unwrap_or_default();
    let of_type = |t: &'static str| {
        rules
            .iter()
            .filter(move |r| r.get("type").and_then(Value::as_str) == Some(t))
            .copied()
    };
    let mut refusals = Vec::new();
    let mut warnings = Vec::new();

    if of_type("merge_queue").next().is_some() {
        refusals.push(format!(
            "a merge queue applies to '{target}': the tool cannot confirm that a merge queue lands the tree it \
             tested, rather than a new merge; remove the merge queue rule from the rulesets of '{target}'"
        ));
    }
    let classic_linear = match classic {
        Classic::Read(p) => {
            p.pointer("/required_linear_history/enabled")
                .and_then(Value::as_bool)
                == Some(true)
        }
        _ => false,
    };
    if of_type("required_linear_history").next().is_some() || classic_linear {
        refusals.push(format!(
            "a rule requires a linear history on '{target}', which forbids merge commits, so the update's merge \
             commits cannot land; turn off `Require linear history` for '{target}'"
        ));
    }

    // The merge methods the pull request rules allow, when any restrict them.
    let mut allowed: Option<BTreeSet<String>> = None;
    for rule in of_type("pull_request") {
        if let Some(methods) = rule
            .pointer("/parameters/allowed_merge_methods")
            .and_then(Value::as_array)
        {
            let methods: BTreeSet<String> = methods
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect();
            allowed = Some(match allowed {
                None => methods,
                Some(before) => before.intersection(&methods).cloned().collect(),
            });
        }
    }
    let rule_allows = |method: &str| allowed.as_ref().is_none_or(|a| a.contains(method));
    let setting = |key: &str| repository.get(key).and_then(Value::as_bool);
    match (setting("allow_merge_commit"), &allowed) {
        (_, Some(a)) if !a.contains("merge") => refusals.push(format!(
            "the pull request rule for '{target}' does not allow merge commits; add `merge` to its allowed merge \
             methods"
        )),
        (Some(false), _) => refusals.push(
            "allow merge commits in the repository's settings (allow_merge_commit): the update lands only with its \
             merge commits"
                .to_string(),
        ),
        (Some(true), _) | (None, Some(_)) => {}
        (None, None) => refusals.push(
            "whether the repository allows merge commits could not be confirmed: GitHub shows the repository's \
             merge settings only to a token with push access. Give the token push access (`Contents: write`), \
             or add a ruleset whose pull request rule allows merge commits for this branch"
                .to_string(),
        ),
    }

    // Status checks: required, naming one check at least, with the branch
    // up to date before merging.
    let named = |checks: Option<&Value>| {
        checks
            .and_then(Value::as_array)
            .is_some_and(|c| !c.is_empty())
    };
    let mut loose = false;
    let mut strict = false;
    for rule in of_type("required_status_checks") {
        if named(rule.pointer("/parameters/required_status_checks")) {
            if rule
                .pointer("/parameters/strict_required_status_checks_policy")
                .and_then(Value::as_bool)
                == Some(true)
            {
                strict = true;
            } else {
                loose = true;
            }
        }
    }
    if let Classic::Read(p) = classic
        && let Some(checks) = p.get("required_status_checks")
        && (named(checks.get("contexts")) || named(checks.get("checks")))
    {
        if checks.get("strict").and_then(Value::as_bool) == Some(true) {
            strict = true;
        } else {
            loose = true;
        }
    }
    let unreadable = match classic {
        Classic::Unreadable(status) => Some(*status),
        _ => None,
    };
    if let Some(status) = unreadable {
        let what = if strict {
            "whether its classic branch protection requires a linear history"
        } else {
            "its required status checks, whether they require the branch to be up to date, and whether its \
             classic branch protection requires a linear history"
        };
        refusals.push(format!(
            "'{target}' is protected by classic branch protection, which GitHub showed this token only as {status}, \
             so {what} could not be confirmed. Express the requirements as a ruleset for '{target}', or give the \
             token administration read access (`Administration: read`)"
        ));
    } else if !strict {
        refusals.push(if loose {
            format!(
                "require branches to be up to date before merging (strict status checks) for '{target}': without it \
                 a pull request merges on checks run against an older base, so the merged tree was never tested"
            )
        } else {
            format!(
                "require at least one status check to pass before merging '{target}', with the branch up to date \
                 before merging, in a ruleset or the branch protection of '{target}'"
            )
        });
    }

    for (key, method, what) in [
        ("allow_squash_merge", "squash", "squash merges"),
        ("allow_rebase_merge", "rebase", "rebase merges"),
    ] {
        if !rule_allows(method) {
            continue;
        }
        match setting(key) {
            Some(false) => {}
            Some(true) => warnings.push(format!(
                "{what} stay selectable ({key}); they drop the update's merge commits and their sync records, so \
                 merge the update's pull requests with a merge commit, or turn them off"
            )),
            None => warnings.push(format!(
                "{what} may stay selectable: the token cannot read {key}, which GitHub shows only with push access"
            )),
        }
    }
    Review {
        refusals,
        warnings,
        notes: Vec::new(),
        ci_paths: vec![WORKFLOWS.to_string()],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo(merge: Option<bool>) -> Value {
        match merge {
            Some(m) => {
                json!({"allow_merge_commit": m, "allow_squash_merge": false, "allow_rebase_merge": false})
            }
            None => json!({}),
        }
    }

    fn checks(strict: bool) -> Value {
        json!([{"type": "required_status_checks", "parameters": {
            "required_status_checks": [{"context": "ci"}],
            "strict_required_status_checks_policy": strict
        }}])
    }

    #[test]
    fn strict_checks_through_a_ruleset_qualify() {
        let r = review("main", &repo(Some(true)), &checks(true), &Classic::None);
        assert!(r.refusals.is_empty(), "{:?}", r.refusals);
        assert!(r.warnings.is_empty());
        assert_eq!(r.ci_paths, [WORKFLOWS]);
    }

    #[test]
    fn checks_without_the_up_to_date_requirement_are_refused() {
        let r = review("main", &repo(Some(true)), &checks(false), &Classic::None);
        assert_eq!(r.refusals.len(), 1);
        assert!(
            r.refusals[0].contains("require branches to be up to date"),
            "{:?}",
            r.refusals
        );
        let r = review("main", &repo(Some(true)), &json!([]), &Classic::None);
        assert!(
            r.refusals[0].contains("require at least one status check"),
            "{:?}",
            r.refusals
        );
    }

    #[test]
    fn a_pipeline_passes_only_with_every_check_it_needs() {
        let passed = json!({"name": "ci", "status": "completed", "conclusion": "success"});
        let failed = json!({"name": "lint", "status": "completed", "conclusion": "failure"});
        let required = ["ci".to_string()];
        let state = |runs: &[Value], total: u64, required: &[String]| {
            commit_pipeline(
                &CommitChecks {
                    runs,
                    runs_total: total,
                    statuses: &[],
                    required,
                },
                String::new(),
            )
            .map(|p| p.state)
        };
        assert_eq!(
            state(std::slice::from_ref(&passed), 1, &required),
            Some(PipelineState::Passed)
        );
        assert_eq!(state(&[], 0, &required), None);
        // GitHub counts more runs than it listed.
        assert_eq!(
            state(std::slice::from_ref(&passed), 2, &required),
            Some(PipelineState::Running)
        );
        // Another check passed, the required one has not registered.
        let lint = json!({"name": "lint", "status": "completed", "conclusion": "success"});
        assert_eq!(
            state(std::slice::from_ref(&lint), 1, &required),
            Some(PipelineState::Running)
        );
        // A failure stands whatever is missing.
        assert_eq!(
            state(std::slice::from_ref(&failed), 3, &required),
            Some(PipelineState::Failed)
        );
    }

    #[test]
    fn required_checks_come_from_rules_and_classic_protection() {
        let classic_read = Classic::Read(json!({"required_status_checks": {
            "contexts": ["old"], "checks": [{"context": "new"}]
        }}));
        assert_eq!(
            required_checks(&checks(true), &classic_read),
            ["ci", "old", "new"]
        );
        assert!(required_checks(&json!([]), &Classic::None).is_empty());
    }

    #[test]
    fn merge_queues_and_linear_history_are_refused() {
        let mut rules = checks(true);
        rules
            .as_array_mut()
            .unwrap()
            .push(json!({"type": "merge_queue", "parameters": {}}));
        rules
            .as_array_mut()
            .unwrap()
            .push(json!({"type": "required_linear_history"}));
        let r = review("main", &repo(Some(true)), &rules, &Classic::None);
        let all = r.refusals.join("\n");
        assert!(
            all.contains("cannot confirm that a merge queue lands the tree it tested"),
            "{all}"
        );
        assert!(all.contains("merge commits cannot land"), "{all}");
    }

    #[test]
    fn classic_protection_counts_when_it_can_be_read() {
        let classic = Classic::Read(json!({
            "required_status_checks": {"strict": true, "contexts": ["ci"], "checks": []},
            "required_linear_history": {"enabled": false}
        }));
        let r = review("main", &repo(Some(true)), &json!([]), &classic);
        assert!(r.refusals.is_empty(), "{:?}", r.refusals);
        let r = review(
            "main",
            &repo(Some(true)),
            &json!([]),
            &Classic::Unreadable(403),
        );
        assert_eq!(r.refusals.len(), 1);
        assert!(
            r.refusals[0].contains("could not be confirmed"),
            "{:?}",
            r.refusals
        );
        assert!(r.refusals[0].contains("ruleset"), "{:?}", r.refusals);
        assert!(
            r.refusals[0].contains("Administration: read"),
            "{:?}",
            r.refusals
        );
    }

    #[test]
    fn merge_commits_unknown_without_push_access() {
        let r = review("main", &repo(None), &checks(true), &Classic::None);
        assert_eq!(r.refusals.len(), 1);
        assert!(r.refusals[0].contains("push access"), "{:?}", r.refusals);
        assert_eq!(r.warnings.len(), 2);
        // A pull request rule that allows merge commits settles it.
        let mut rules = checks(true);
        rules
            .as_array_mut()
            .unwrap()
            .push(json!({"type": "pull_request", "parameters": {
                "allowed_merge_methods": ["merge"]
            }}));
        let r = review("main", &repo(None), &rules, &Classic::None);
        assert!(r.refusals.is_empty(), "{:?}", r.refusals);
        assert!(r.warnings.is_empty(), "{:?}", r.warnings);
    }

    #[test]
    fn a_branch_covered_only_by_rulesets_has_no_classic_protection() {
        assert!(matches!(
            classic(404, json!({"message": "Branch not protected"})),
            Classic::None
        ));
        assert!(matches!(
            classic(404, json!({"message": "Not Found"})),
            Classic::Unreadable(404)
        ));
        assert!(matches!(classic(403, json!({})), Classic::Unreadable(403)));
        assert!(matches!(classic(200, json!({})), Classic::Read(_)));
    }

    #[test]
    fn only_the_repository_s_own_pull_requests_count() {
        let own = |head: Value| {
            from_the_repository(&json!({
                "head": {"repo": head},
                "base": {"repo": {"id": 7, "full_name": "team/app"}}
            }))
        };
        // Renamed since the remote's URL was written: the ids still match.
        assert!(own(json!({"id": 7, "full_name": "acme/app-renamed"})));
        assert!(!own(json!({"id": 8, "full_name": "team/app"})));
        assert!(!own(Value::Null));
    }

    #[test]
    fn pull_requests_are_read_with_their_state() {
        let p = pull(&json!({
            "number": 3, "html_url": "https://github.com/team/app/pull/3", "state": "closed",
            "merged_at": "2026-01-02T00:00:00Z", "closed_at": "2026-01-02T00:00:00Z",
            "head": {"sha": "abc", "ref": "git-intake/update"}, "base": {"ref": "main"}, "body": null
        }))
        .unwrap();
        assert_eq!(p.state, State::Merged);
        assert_eq!(p.head, "abc");
        assert_eq!(p.target, "main");
        assert_eq!(
            pull(&json!({"number": 4, "state": "closed"}))
                .unwrap()
                .state,
            State::Closed
        );
    }
}
