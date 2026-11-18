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
    Account, Failure, Forge, Kind, Location, Merge, MergeRequest, NewTag, Pipeline, Queued,
    ReleaseLookup, Review, Signal, State, TagAccess, TagCreation, assigns, endpoint, instant,
    pipeline_state, signal, text,
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

/// The role creating a tag needs, and the lowest a group share or a group
/// membership lets create a protected tag.
const DEVELOPER: u64 = 30;
/// The highest role of a member.
const OWNER: u64 = 50;
/// The access level of a protected tag rule for the instance's
/// administrators.
const ADMINISTRATOR: u64 = 60;

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

    /// A JSON answer, `None` for 404.
    fn optional(&self, url: &Url) -> Result<Option<Value>> {
        let reply = self.client.send(Method::Get, url, None)?;
        match reply.status {
            404 => Ok(None),
            _ if reply.success() => Ok(Some(reply.json)),
            status => {
                Err(Failure::answered(status, self.client.refusal(Method::Get, url, &reply)).into())
            }
        }
    }

    /// Whether a project tag named `name` exists.
    fn has_tag(&self, name: &str) -> Result<bool> {
        Ok(self
            .optional(&self.url(&["repository", "tags", name], &[]))?
            .is_some())
    }

    /// The token's account: why it may not create a tag, or its id, name
    /// and whether it administers the instance.
    fn token_account(&self) -> Result<std::result::Result<(u64, String, bool), String>> {
        let variable = self.location.token_variable();
        let url = endpoint(self.client.base(), &["personal_access_tokens", "self"], &[]);
        let reply = self.client.send(Method::Get, &url, None)?;
        if reply.status == 401 {
            return Ok(Err(format!(
                "GitLab does not accept the token in {variable}: {}",
                crate::redact::redacted(&reply.message())
            )));
        }
        if !reply.success() {
            return Err(Failure::answered(
                reply.status,
                self.client.refusal(Method::Get, &url, &reply),
            )
            .into());
        }
        let token = reply.json;
        if token.get("active").and_then(Value::as_bool) == Some(false)
            || token.get("revoked").and_then(Value::as_bool) == Some(true)
        {
            return Ok(Err(format!(
                "the token in {variable} is revoked or expired"
            )));
        }
        let scopes: Vec<&str> = token
            .get("scopes")
            .and_then(Value::as_array)
            .map(|list| list.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        if !scopes.contains(&"api") {
            return Ok(Err(format!(
                "the token in {variable} has the scopes {}, and creating a tag through the API needs `api`",
                if scopes.is_empty() {
                    "none".to_string()
                } else {
                    scopes.join(", ")
                }
            )));
        }
        let user = self.client.json(
            Method::Get,
            &endpoint(self.client.base(), &["user"], &[]),
            None,
        )?;
        let id = user
            .get("id")
            .and_then(Value::as_u64)
            .context("GitLab answered with an account without an id")?;
        let admin = user.get("is_admin").and_then(Value::as_bool) == Some(true);
        Ok(Ok((id, text(&user, "username"), admin)))
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
        created_at: instant(v, "created_at"),
        ended_at,
        // `work_in_progress` is what instances before `draft` answer.
        draft: v
            .get("draft")
            .or_else(|| v.get("work_in_progress"))
            .and_then(Value::as_bool),
        assigned: match (v.get("assignees"), v.get("assignee")) {
            (Some(Value::Array(list)), _) => Some(!list.is_empty()),
            (_, Some(one)) => Some(!one.is_null()),
            _ => None,
        },
        // GitLab reports every auto-merge, "merge when checks pass"
        // included, as `merge_when_pipeline_succeeds`.
        auto_merge: v
            .get("merge_when_pipeline_succeeds")
            .and_then(Value::as_bool),
        review_requested: match v.get("reviewers") {
            Some(Value::Array(list)) => Some(!list.is_empty()),
            _ => None,
        },
        locked: v.get("state").and_then(Value::as_str) == Some("locked"),
        // Absent on instances without the feature, which nobody set then.
        merge_scheduled: v.get("merge_after").is_some_and(|a| !a.is_null()),
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

    fn close(&self, number: u64, description: Option<&str>) -> Result<MergeRequest> {
        let n = number.to_string();
        let mut body = json!({ "state_event": "close" });
        if let Some(description) = description {
            body["description"] = description.into();
        }
        merge_request(&self.client.json(
            Method::Put,
            &self.url(&["merge_requests", &n], &[]),
            Some(&body),
        )?)
    }

    // Every tier answers `approved_by`, which lists each approval whether
    // or not a rule requires it.
    fn reviewed(&self, number: u64) -> Result<Signal> {
        let n = number.to_string();
        let url = self.url(&["merge_requests", &n, "approvals"], &[]);
        signal(|| {
            match self
                .client
                .json(Method::Get, &url, None)?
                .get("approved_by")
            {
                Some(Value::Array(list)) => Ok(!list.is_empty()),
                _ => anyhow::bail!(
                    "GitLab answered the approvals of merge request !{number} without `approved_by`"
                ),
            }
        })
    }

    // System notes record pushes, description changes and other events;
    // every other note, on the diff as well, is someone's comment.
    fn commented(&self, number: u64) -> Result<Signal> {
        let n = number.to_string();
        let url = self.url(
            &["merge_requests", &n, "notes"],
            &[("sort", "asc"), ("order_by", "created_at")],
        );
        signal(|| {
            let mut found = false;
            self.client.pages(&url, "notes", |page| {
                found = page
                    .iter()
                    .any(|note| note.get("system").and_then(Value::as_bool) != Some(true));
                !found
            })?;
            Ok(found)
        })
    }

    // Award emoji on the merge request itself; those on its notes belong
    // to the notes.
    fn reacted(&self, number: u64) -> Result<Signal> {
        let n = number.to_string();
        let url = self.url(&["merge_requests", &n, "award_emoji"], &[]);
        signal(|| {
            let mut found = false;
            self.client.pages(&url, "award emoji", |page| {
                found = !page.is_empty();
                false
            })?;
            Ok(found)
        })
    }

    fn set_description(&self, number: u64, description: &str) -> Result<MergeRequest> {
        let n = number.to_string();
        let body = json!({ "description": description });
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

    // An annotated tag, since it has a message; GitLab writes the token's
    // account as its tagger, and starts the tag's pipeline as that account.
    fn create_tag(&self, tag: &NewTag) -> Result<TagCreation> {
        let url = self.url(&["repository", "tags"], &[]);
        let body = json!({
            "tag_name": tag.name,
            "ref": tag.commit,
            "message": tag.message,
        });
        let reply = self.client.send(Method::Post, &url, Some(&body))?;
        if reply.success() {
            return Ok(TagCreation::Created);
        }
        if self.has_tag(tag.name)? {
            return Ok(TagCreation::Taken);
        }
        Err(Failure::answered(
            reply.status,
            self.client.refusal(Method::Post, &url, &reply),
        )
        .into())
    }

    fn tag_access(&self, name: &str) -> Result<TagAccess> {
        let (user, username, admin) = match self.token_account()? {
            Ok(account) => account,
            Err(why) => return Ok(TagAccess::Denied(why)),
        };
        let account = format!("`{username}`");
        let member = self.optional(&self.url(&["members", "all", &user.to_string()], &[]))?;
        let level = member
            .as_ref()
            .and_then(|m| m.get("access_level"))
            .and_then(Value::as_u64)
            .unwrap_or(0);
        if level < DEVELOPER && !admin {
            return Ok(TagAccess::Denied(format!(
                "the account {account} has the role {} in the project '{}', and creating a tag needs Developer \
                 or higher",
                role(level),
                self.location.project
            )));
        }
        let rules: Vec<Value> = self
            .client
            .list(&self.url(&["protected_tags"], &[]), "protected tags")?
            .into_iter()
            .filter(|rule| wildcard_match(&text(rule, "name"), name))
            .collect();
        if rules.is_empty() {
            return Ok(TagAccess::Allowed {
                account: Some(account),
                unproven: None,
            });
        }
        let mut shares = None;
        let mut unknown = Vec::new();
        for access in rules.iter().flat_map(|rule| {
            rule.get("create_access_levels")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        }) {
            let id = |key: &str| access.get(key).and_then(Value::as_u64);
            let allowed = if let Some(named) = id("user_id") {
                // GitLab also requires a user a rule names to be a member.
                named == user && level > 0
            } else if let Some(group) = id("group_id") {
                match self.group_grant(&mut shares, group, user) {
                    Ok(granted) => granted,
                    Err(why) => {
                        unknown.push(format!("group {group}: {why}"));
                        false
                    }
                }
            } else if id("deploy_key_id").is_some() {
                false
            } else {
                match id("access_level").unwrap_or(0) {
                    0 => false,
                    ADMINISTRATOR => admin,
                    required => level >= required,
                }
            };
            if allowed {
                return Ok(TagAccess::Allowed {
                    account: Some(account),
                    unproven: None,
                });
            }
        }
        if !unknown.is_empty() {
            anyhow::bail!(
                "cannot tell whether the account {account} may create the tag {name}, which a protected tag lets \
                 members of a group create: {}",
                unknown.join("; ")
            );
        }
        let names: Vec<String> = rules
            .iter()
            .map(|rule| format!("`{}`", text(rule, "name")))
            .collect();
        Ok(TagAccess::Denied(format!(
            "the account {account}, with the role {}, may not create the tag {name}, which the protected tag {} \
             covers; let it create these tags, or raise its role",
            role(level),
            names.join(" and ")
        )))
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

impl GitLab {
    /// Whether a protected tag rule that names `group` lets `user` create
    /// the tag, as GitLab decides it: the project is shared with exactly
    /// that group with the Developer role or higher, and the user is a
    /// member of the group itself, not through a group above it, with the
    /// Developer role or higher. `Err` names what GitLab's answers leave
    /// open.
    fn group_grant(
        &self,
        shares: &mut Option<std::result::Result<Vec<(u64, u64)>, String>>,
        group: u64,
        user: u64,
    ) -> std::result::Result<bool, String> {
        let shares = shares.get_or_insert_with(|| {
            let project = self.project().map_err(|err| format!("{err:#}"))?;
            let list = project
                .get("shared_with_groups")
                .and_then(Value::as_array)
                .ok_or("GitLab does not show the groups the project is shared with to the token")?;
            Ok(list
                .iter()
                .filter_map(|s| {
                    Some((
                        s.get("group_id")?.as_u64()?,
                        s.get("group_access_level")?.as_u64()?,
                    ))
                })
                .collect())
        });
        let shares = shares.as_ref().map_err(String::clone)?;
        let roles = DEVELOPER..=OWNER;
        if !shares
            .iter()
            .any(|(id, level)| *id == group && roles.contains(level))
        {
            return Ok(false);
        }
        let url = endpoint(
            self.client.base(),
            &["groups", &group.to_string(), "members", &user.to_string()],
            &[],
        );
        let member = self.optional(&url).map_err(|err| format!("{err:#}"))?;
        Ok(member
            .and_then(|m| m.get("access_level").and_then(Value::as_u64))
            .is_some_and(|level| roles.contains(&level)))
    }
}

/// A role by its access level, as GitLab names it.
fn role(level: u64) -> String {
    let name = match level {
        0 => "none",
        5 => "Minimal Access",
        10 => "Guest",
        15 => "Planner",
        20 => "Reporter",
        30 => "Developer",
        40 => "Maintainer",
        50 => "Owner",
        _ => return format!("of access level {level}"),
    };
    format!("{name} ({level})")
}

/// Whether a protected tag's name or wildcard, in which `*` stands for any
/// text, matches `tag`.
fn wildcard_match(pattern: &str, tag: &str) -> bool {
    let mut parts = pattern.split('*');
    let first = parts.next().unwrap_or_default();
    let Some(mut rest) = tag.strip_prefix(first) else {
        return false;
    };
    let parts: Vec<&str> = parts.collect();
    let Some((last, middle)) = parts.split_last() else {
        return rest.is_empty();
    };
    for part in middle {
        match rest.find(part) {
            Some(at) => rest = &rest[at + part.len()..],
            None => return false,
        }
    }
    rest.len() >= last.len() && rest.ends_with(last)
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

    #[test]
    fn protected_tag_wildcards() {
        for (pattern, tag, matches) in [
            ("v*", "v1.2.3", true),
            ("v*", "app-v1", false),
            ("v1.2.3", "v1.2.3", true),
            ("v1.2.3", "v1.2.30", false),
            ("*-custom.*", "v0.11.1-custom.20261101.1", true),
            ("v*.*", "v1", false),
            ("*", "anything/at-all", true),
            ("v*1*1", "v1", false),
        ] {
            assert_eq!(wildcard_match(pattern, tag), matches, "{pattern} {tag}");
        }
    }

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
        // Whatever the answer does not say stays unknown.
        assert_eq!((mr.draft, mr.assigned, mr.auto_merge), (None, None, None));
        let open = merge_request(&json!({
            "iid": 8, "state": "opened", "created_at": "2026-01-01T10:00:00.000Z",
            "draft": false, "assignees": [], "merge_when_pipeline_succeeds": false
        }))
        .unwrap();
        assert_eq!(
            open.created_at,
            Some("2026-01-01T10:00:00Z".parse().unwrap())
        );
        assert_eq!(
            (open.draft, open.assigned, open.auto_merge),
            (Some(false), Some(false), Some(false))
        );
        let touched = merge_request(&json!({
            "iid": 9, "state": "opened", "work_in_progress": true,
            "assignee": {"username": "someone"}, "merge_when_pipeline_succeeds": true
        }))
        .unwrap();
        assert_eq!(
            (touched.draft, touched.assigned, touched.auto_merge),
            (Some(true), Some(true), Some(true))
        );
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
