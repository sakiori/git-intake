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
use super::{Forge, Kind, Location, MergeRequest, Review, State, endpoint, optional_text, text};

/// The longest description GitLab takes.
const DESCRIPTION_LIMIT: usize = 1_048_576;

/// The CI definition of a project that names none.
const DEFAULT_CI_FILE: &str = ".gitlab-ci.yml";

pub struct GitLab {
    location: Location,
    client: Client,
}

impl GitLab {
    pub fn new(location: Location) -> Self {
        let client = Client::new(
            location.api.clone(),
            location.token.clone(),
            Auth::PrivateToken,
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
        State::Merged => optional_text(v, "merged_at").or_else(|| optional_text(v, "closed_at")),
        State::Closed => optional_text(v, "closed_at"),
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
            ("per_page", "100"),
        ];
        if let Some(target) = target {
            query.push(("target_branch", target));
        }
        let list = self
            .client
            .json(Method::Get, &self.url(&["merge_requests"], &query), None)?;
        let list = list
            .as_array()
            .context("GitLab answered the list of merge requests with something else")?;
        list.iter().map(merge_request).collect()
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
            _ => anyhow::bail!("{}", self.client.refusal(Method::Post, &url, &reply)),
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
    let mut warnings = Vec::new();
    match project.get("squash_option").and_then(Value::as_str) {
        Some("always") => refusals.push(
            "set `Squash commits when merging` to anything but `Require` (squash_option `always`): a squash drops \
             the update's merge commits, whose trailers record what it merged"
                .to_string(),
        ),
        Some(option @ ("default_on" | "default_off")) => warnings.push(format!(
            "`Squash commits when merging` lets the person merging squash (squash_option `{option}`); a squashed \
             update loses its sync records, so merge the update's merge requests without squashing, or set the \
             option to `Do not allow`"
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
        warnings,
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
        assert_eq!(r.warnings.len(), 1);
        assert_eq!(r.ci_paths, [".gitlab-ci.yml"]);
        let mut ff = qualifying();
        ff["merge_method"] = "ff".into();
        ff["squash_option"] = "never".into();
        ff["merge_trains_enabled"] = false.into();
        ff["automatic_rebase_enabled"] = false.into();
        ff["ci_config_path"] = "ci/pipeline.yml".into();
        let r = review(&ff);
        assert!(r.refusals.is_empty() && r.warnings.is_empty());
        assert_eq!(r.ci_paths, ["ci/pipeline.yml"]);
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
        assert_eq!(mr.ended_at.as_deref(), Some("2026-01-02T00:00:00.000Z"));
        assert_eq!(mr.description, "");
        assert_eq!(
            merge_request(&json!({"iid": 1, "state": "locked"}))
                .unwrap()
                .state,
            State::Open
        );
    }
}
