//! GitLab's REST API, version 4.
//!
//! The project is addressed by its path, encoded as one segment. The merge
//! settings the tool reads are fields of the project, which any token that
//! can read the project sees; the merge train fields exist only on licensed
//! tiers and count as off when absent, as does automatic rebase on
//! instances that do not have it.

use anyhow::{Context, Result};
use serde_json::{Value, json};
use url::Url;

use super::http::{Auth, Client, Method};
use super::{
    Account, Failure, Forge, Kind, Location, Merge, MergeRequest, Pipeline, Queued, ReleaseLookup,
    Review, State, assigns, endpoint, instant, pipeline_state, text,
};

/// The merge statuses of a merge request whose mergeability GitLab is still
/// working out, after which a merge refused for them may pass.
const DECIDING: [&str; 5] = [
    "checking",
    "unchecked",
    "ci_still_running",
    "preparing",
    "approvals_syncing",
];

/// The merge statuses of a merge request that a catch-up does not change and
/// a pause of the merge queue would wait out for nothing. The others either
/// follow from the branch being behind or pass with its next pipeline.
const BLOCKING: [&str; 11] = [
    "discussions_not_resolved",
    "draft_status",
    "jira_association_missing",
    "locked_lfs_files",
    "locked_paths",
    "merge_request_blocked",
    "merge_time",
    "not_approved",
    "requested_changes",
    "security_policy_violations",
    "title_regex",
];

/// The longest description GitLab takes.
const DESCRIPTION_LIMIT: usize = 1_048_576;

/// The CI definition of a project that names none.
pub(crate) const DEFAULT_CI_FILE: &str = ".gitlab-ci.yml";

pub struct GitLab {
    location: Location,
    client: Client,
}

impl GitLab {
    pub fn new(location: Location) -> Self {
        let auth = match &location.credential {
            Some(c) if c.job => Auth::JobToken,
            _ => Auth::PrivateToken,
        };
        let client = Client::new(
            location.api.clone(),
            location.credential.as_ref().map(|c| c.secret.clone()),
            auth,
            vec![("Accept", "application/json")],
            "GitLab",
        );
        Self { location, client }
    }

    fn url(&self, rest: &[&str], query: &[(&str, &str)]) -> Url {
        let mut segments = vec!["projects", self.location.project.as_str()];
        segments.extend(rest);
        endpoint(self.client.base(), &segments, query)
    }

    fn project(&self) -> Result<Value> {
        self.client.json(Method::Get, &self.url(&[], &[]), None)
    }

    fn single(&self, number: u64, query: &[(&str, &str)]) -> Result<Value> {
        let n = number.to_string();
        self.client
            .json(Method::Get, &self.url(&["merge_requests", &n], query), None)
    }
}

/// The pipeline GitLab reports as a merge request's `head_pipeline`.
fn head_pipeline(v: &Value) -> Option<Pipeline> {
    let p = v.get("head_pipeline").filter(|p| p.is_object())?;
    Some(Pipeline {
        state: pipeline_state(p.get("status").and_then(Value::as_str).unwrap_or_default()),
        url: text(p, "web_url"),
    })
}

fn assignee_ids(v: &Value) -> Vec<u64> {
    v.get("assignees")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|a| a.get("id").and_then(Value::as_u64))
                .collect()
        })
        .unwrap_or_default()
}

/// Whether a merge request comes from a branch of the project itself, not
/// of a fork.
fn from_the_project(v: &Value) -> bool {
    let id = |key: &str| v.get(key).and_then(Value::as_u64);
    id("source_project_id").is_some() && id("source_project_id") == id("target_project_id")
}

fn merge_request(v: &Value) -> Result<MergeRequest> {
    let number = v
        .get("iid")
        .and_then(Value::as_u64)
        .context("GitLab answered with a merge request without a number")?;
    let state = match v.get("state").and_then(Value::as_str) {
        Some("merged") => State::Merged,
        Some("closed") => State::Closed,
        // `locked` is the moment of a merge, which has not happened yet.
        _ => State::Open,
    };
    let ended_at = match state {
        State::Merged => instant(v, "merged_at").or_else(|| instant(v, "closed_at")),
        State::Closed => instant(v, "closed_at"),
        State::Open => None,
    };
    Ok(MergeRequest {
        number,
        url: text(v, "web_url"),
        state,
        head: text(v, "sha"),
        source: text(v, "source_branch"),
        target: text(v, "target_branch"),
        description: text(v, "description"),
        ended_at,
    })
}

impl Forge for GitLab {
    fn kind(&self) -> Kind {
        Kind::GitLab
    }

    fn location(&self) -> &Location {
        &self.location
    }

    fn check_access(&self) -> Result<()> {
        self.project().map(|_| ())
    }

    fn merge_requests(
        &self,
        source: &str,
        target: Option<&str>,
        open_only: bool,
    ) -> Result<Vec<MergeRequest>> {
        let mut query = vec![
            ("source_branch", source),
            ("state", if open_only { "opened" } else { "all" }),
            ("order_by", "updated_at"),
            ("sort", "desc"),
        ];
        if let Some(target) = target {
            query.push(("target_branch", target));
        }
        let list = self
            .client
            .list(&self.url(&["merge_requests"], &query), "merge requests")?;
        // Only the project's own merge requests: one from a fork may come
        // from a branch of the same name, with any description.
        list.iter()
            .filter(|v| from_the_project(v))
            .map(merge_request)
            .collect()
    }

    fn merge_request(&self, number: u64) -> Result<MergeRequest> {
        let n = number.to_string();
        merge_request(&self.client.json(
            Method::Get,
            &self.url(&["merge_requests", &n], &[]),
            None,
        )?)
    }

    fn open(
        &self,
        source: &str,
        target: &str,
        title: &str,
        description: &str,
    ) -> Result<Option<MergeRequest>> {
        let url = self.url(&["merge_requests"], &[]);
        let body = json!({
            "source_branch": source,
            "target_branch": target,
            "title": title,
            "description": description,
            "squash": false,
            "remove_source_branch": true,
        });
        let reply = self.client.send(Method::Post, &url, Some(&body))?;
        match reply.status {
            // Another open merge request from the branch into the target.
            409 => Ok(None),
            _ if reply.success() => merge_request(&reply.json).map(Some),
            _ => Err(Failure::answered(
                reply.status,
                self.client.refusal(Method::Post, &url, &reply),
            )
            .into()),
        }
    }

    fn update(&self, number: u64, title: &str, description: &str) -> Result<MergeRequest> {
        let n = number.to_string();
        let body = json!({ "title": title, "description": description, "squash": false });
        merge_request(&self.client.json(
            Method::Put,
            &self.url(&["merge_requests", &n], &[]),
            Some(&body),
        )?)
    }

    fn close(&self, number: u64) -> Result<MergeRequest> {
        let n = number.to_string();
        let body = json!({ "state_event": "close" });
        merge_request(&self.client.json(
            Method::Put,
            &self.url(&["merge_requests", &n], &[]),
            Some(&body),
        )?)
    }

    fn review(&self, _target: &str) -> Result<Review> {
        Ok(review(&self.project()?))
    }

    fn description_limit(&self) -> usize {
        DESCRIPTION_LIMIT
    }

    // The releases API, unlike the project itself, takes a CI job token.
    fn release(&self, tag: &str) -> Result<ReleaseLookup> {
        let list = self.url(&["releases"], &[("per_page", "1")]);
        let reply = self.client.send(Method::Get, &list, None)?;
        match reply.status {
            404 => return Ok(ReleaseLookup::Hidden),
            _ if reply.success() => {}
            _ => anyhow::bail!("{}", self.client.refusal(Method::Get, &list, &reply)),
        }
        let one = self.url(&["releases", tag], &[]);
        let reply = self.client.send(Method::Get, &one, None)?;
        match reply.status {
            404 => Ok(ReleaseLookup::Unpublished),
            _ if reply.success() => Ok(ReleaseLookup::Published),
            _ => anyhow::bail!("{}", self.client.refusal(Method::Get, &one, &reply)),
        }
    }

    fn with_pipeline(&self, number: u64) -> Result<(MergeRequest, Option<Pipeline>)> {
        let v = self.single(number, &[])?;
        Ok((merge_request(&v)?, head_pipeline(&v)))
    }

    fn merge(&self, number: u64, head: &str) -> Result<Merge> {
        let n = number.to_string();
        // A merge reads `squash` only when true, and otherwise goes by the
        // merge request's own flag, which a person may have set.
        if self
            .single(number, &[])?
            .get("squash")
            .and_then(Value::as_bool)
            == Some(true)
        {
            self.client.json(
                Method::Put,
                &self.url(&["merge_requests", &n], &[]),
                Some(&json!({ "squash": false })),
            )?;
        }
        let url = self.url(&["merge_requests", &n, "merge"], &[]);
        let body = json!({ "sha": head, "squash": false, "should_remove_source_branch": true });
        let reply = self.client.send(Method::Put, &url, Some(&body))?;
        if reply.success() {
            return Ok(Merge::Accepted);
        }
        match reply.status {
            409 => Ok(Merge::HeadMoved),
            405 | 406 | 422 => {
                let now = self.single(number, &[])?;
                let status = text(&now, "detailed_merge_status");
                let mut why = reply.message();
                let error = text(&now, "merge_error");
                if !error.is_empty() {
                    why.push_str(&format!(": {error}"));
                }
                Ok(match status.as_str() {
                    s if DECIDING.contains(&s) => Merge::NotYet,
                    "mergeable" => Merge::Contested(why),
                    "" => Merge::Refused(why),
                    s => Merge::Refused(format!("{why} (its merge status is `{s}`)")),
                })
            }
            status => Err(Failure::answered(
                status,
                self.client.refusal(Method::Put, &url, &reply),
            )
            .into()),
        }
    }

    fn merge_blocker(&self, number: u64) -> Result<Option<String>> {
        let status = text(&self.single(number, &[])?, "detailed_merge_status");
        Ok(BLOCKING.contains(&status.as_str()).then_some(status))
    }

    fn account(&self, username: &str) -> Result<Option<Account>> {
        let url = endpoint(self.client.base(), &["users"], &[("username", username)]);
        let list = self.client.json(Method::Get, &url, None)?;
        Ok(list
            .as_array()
            .context("GitLab answered the list of users with something else")?
            .iter()
            .find(|u| text(u, "username").eq_ignore_ascii_case(username))
            .and_then(|u| {
                Some(Account {
                    id: u.get("id")?.as_u64()?,
                    username: text(u, "username"),
                })
            }))
    }

    fn assigned(&self, target: &str, account: &Account) -> Result<Vec<u64>> {
        let id = account.id.to_string();
        let query = [
            ("state", "opened"),
            ("target_branch", target),
            ("assignee_id", id.as_str()),
        ];
        let list = self
            .client
            .list(&self.url(&["merge_requests"], &query), "merge requests")?;
        Ok(list
            .iter()
            .filter(|v| assignee_ids(v).contains(&account.id))
            .filter_map(|v| v.get("iid").and_then(Value::as_u64))
            .collect())
    }

    fn queued(&self, number: u64) -> Result<Queued> {
        let v = self.single(number, &[("include_diverged_commits_count", "true")])?;
        let mr = merge_request(&v)?;
        Ok(Queued {
            number,
            url: mr.url,
            state: mr.state,
            assignees: assignee_ids(&v),
            current: v.get("diverged_commits_count").and_then(Value::as_u64) == Some(0),
            pipeline: head_pipeline(&v).map(|p| p.state),
        })
    }

    fn assigned_at(&self, number: u64, account: &Account) -> Result<Option<jiff::Timestamp>> {
        let n = number.to_string();
        let url = self.url(
            &["merge_requests", &n, "notes"],
            &[("sort", "desc"), ("order_by", "created_at")],
        );
        let mut at = None;
        self.client.pages(&url, "notes", |page| {
            let found = page.iter().find(|v| {
                v.get("system").and_then(Value::as_bool) == Some(true)
                    && assigns(&text(v, "body"), &account.username)
            });
            at = found.and_then(|v| instant(v, "created_at"));
            found.is_none()
        })?;
        Ok(at)
    }

    fn assign(&self, number: u64, assignees: &[u64]) -> Result<()> {
        let n = number.to_string();
        let body = json!({ "assignee_ids": assignees });
        self.client.json(
            Method::Put,
            &self.url(&["merge_requests", &n], &[]),
            Some(&body),
        )?;
        Ok(())
    }

    fn comment(&self, number: u64, body: &str) -> Result<()> {
        let n = number.to_string();
        self.client.json(
            Method::Post,
            &self.url(&["merge_requests", &n, "notes"], &[]),
            Some(&json!({ "body": body })),
        )?;
        Ok(())
    }
}

/// What a GitLab project's settings mean for an update's merge request.
pub fn review(project: &Value) -> Review {
    let flag = |key: &str| project.get(key).and_then(Value::as_bool);
    let mut refusals = Vec::new();
    if flag("only_allow_merge_if_pipeline_succeeds") != Some(true) {
        refusals.push(
            "turn on `Pipelines must succeed` (only_allow_merge_if_pipeline_succeeds) in the project's merge request \
             settings: without it a merge request can be merged with no pipeline that tested it"
                .to_string(),
        );
    }
    if flag("allow_merge_on_skipped_pipeline") == Some(true) {
        refusals.push(
            "turn off `Skipped pipelines are considered successful` (allow_merge_on_skipped_pipeline): a skipped \
             pipeline tests nothing"
                .to_string(),
        );
    }
    match project.get("merge_method").and_then(Value::as_str) {
        Some("ff" | "rebase_merge") => {}
        other => refusals.push(format!(
            "set the merge method to `Merge commit with semi-linear history` or `Fast-forward merge` \
             (merge_method `rebase_merge` or `ff`) instead of {}: GitLab then merges only a branch that contains \
             the target branch, so the merged tree is the one the pipeline tested, while with plain merge commits \
             the merged tree is a new merge, and GitLab does not require even a merged results pipeline to match \
             the current target branch",
            match other {
                Some(m) => format!("`{m}`"),
                None => "the method the project has".to_string(),
            }
        )),
    }
    if flag("merge_trains_enabled") == Some(true) {
        refusals.push(
            "turn off merge trains (merge_trains_enabled): whether a train lands the commit it tested could not be \
             confirmed, and with fast-forward or semi-linear history a train rebases, and a rebase drops the \
             update's merge commits, whose trailers record what it merged"
                .to_string(),
        );
    }
    if flag("automatic_rebase_enabled") == Some(true) {
        refusals.push(
            "turn off `Enable automatic rebase prior to merge` (automatic_rebase_enabled): a rebase drops the \
             update's merge commits, whose trailers record what it merged; `git intake sync` keeps the branch \
             mergeable by catch-up merges instead"
                .to_string(),
        );
    }
    let mut notes = Vec::new();
    match project.get("squash_option").and_then(Value::as_str) {
        Some("always") => refusals.push(
            "set `Squash commits when merging` to anything but `Require` (squash_option `always`): a squash drops \
             the update's merge commits, whose trailers record what it merged"
                .to_string(),
        ),
        // The tool asks for squashing off on the update's merge request, and
        // only `always` overrides that, so the project may squash its others.
        Some(option @ ("default_on" | "default_off")) => notes.push(format!(
            "squashing stays optional (squash_option `{option}`), which suits a project that squashes its other \
             merge requests: the tool opens and rewrites the update's merge request with squashing off, so leave \
             it off when merging, or the merge counts as one without the update's history"
        )),
        _ => {}
    }
    let ci = project
        .get("ci_config_path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .unwrap_or(DEFAULT_CI_FILE);
    // A definition in another project, `<file>@<group>/<project>`, or at an
    // address, is not part of the repository.
    let ci_paths = if ci.contains('@') || ci.contains("://") {
        Vec::new()
    } else {
        vec![ci.trim_start_matches('/').to_string()]
    };
    Review {
        refusals,
        warnings: Vec::new(),
        notes,
        ci_paths,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn qualifying() -> Value {
        json!({
            "only_allow_merge_if_pipeline_succeeds": true,
            "allow_merge_on_skipped_pipeline": false,
            "merge_method": "rebase_merge",
            "squash_option": "default_off",
        })
    }

    #[test]
    fn semi_linear_history_with_required_pipelines_qualifies() {
        let r = review(&qualifying());
        assert!(r.refusals.is_empty(), "{:?}", r.refusals);
        assert_eq!(r.ci_paths, [".gitlab-ci.yml"]);
        let mut ff = qualifying();
        ff["merge_method"] = "ff".into();
        ff["squash_option"] = "never".into();
        ff["merge_trains_enabled"] = false.into();
        ff["automatic_rebase_enabled"] = false.into();
        ff["ci_config_path"] = "ci/pipeline.yml".into();
        let r = review(&ff);
        assert!(r.refusals.is_empty() && r.warnings.is_empty() && r.notes.is_empty());
        assert_eq!(r.ci_paths, ["ci/pipeline.yml"]);
    }

    #[test]
    fn squashing_left_to_the_person_merging_is_a_note_and_advises_no_change() {
        for option in ["default_on", "default_off"] {
            let mut project = qualifying();
            project["squash_option"] = option.into();
            let r = review(&project);
            assert!(r.refusals.is_empty() && r.warnings.is_empty(), "{option}");
            let [note] = r.notes.as_slice() else {
                panic!("{option}: {:?}", r.notes);
            };
            assert!(note.contains(&format!("`{option}`")), "{note}");
            assert!(note.contains("with squashing off"), "{note}");
            assert!(!note.contains("Do not allow"), "{note}");
        }
    }

    #[test]
    fn every_unqualified_setting_is_named() {
        let r = review(&json!({
            "only_allow_merge_if_pipeline_succeeds": false,
            "allow_merge_on_skipped_pipeline": true,
            "merge_method": "merge",
            "merge_trains_enabled": true,
            "automatic_rebase_enabled": true,
            "squash_option": "always",
            "ci_config_path": "pipeline.yml@platform/ci",
        }));
        let all = r.refusals.join("\n");
        for setting in [
            "only_allow_merge_if_pipeline_succeeds",
            "allow_merge_on_skipped_pipeline",
            "merge_method `rebase_merge` or `ff`) instead of `merge`",
            "merge_trains_enabled",
            "automatic_rebase_enabled",
            "squash_option `always`",
        ] {
            assert!(all.contains(setting), "{setting}: {all}");
        }
        assert!(
            all.contains("a rebase drops the update's merge commits"),
            "{all}"
        );
        assert!(r.ci_paths.is_empty());
    }

    #[test]
    fn merge_requests_are_read_with_their_state() {
        let mr = merge_request(&json!({
            "iid": 7, "web_url": "https://gitlab.example.com/g/app/-/merge_requests/7", "state": "merged",
            "sha": "abc", "source_branch": "git-intake/update", "target_branch": "main",
            "description": null, "merged_at": "2026-01-02T00:00:00.000Z", "closed_at": null
        }))
        .unwrap();
        assert_eq!(mr.state, State::Merged);
        assert_eq!(mr.ended_at, Some("2026-01-02T00:00:00Z".parse().unwrap()));
        assert_eq!(mr.description, "");
        assert_eq!(
            merge_request(&json!({"iid": 1, "state": "locked"}))
                .unwrap()
                .state,
            State::Open
        );
        assert!(from_the_project(
            &json!({"source_project_id": 4, "target_project_id": 4})
        ));
        assert!(!from_the_project(
            &json!({"source_project_id": 9, "target_project_id": 4})
        ));
        assert!(!from_the_project(&json!({"iid": 1})));
    }
}
