//! Publication through a merge request: `finish` requires the forge's merge
//! settings, pushes the update head to `git-intake/update` without force,
//! opens the merge request through the forge's API, and ends the local
//! update. The forge is a fake one; see `common::forge`.

mod common;

use std::fs;
use std::path::Path;

use common::forge::TOKEN;
use common::{Fork, How, MERGE_REQUEST_BRANCH, merge_on_fake, open_number, stderr, stdout};
use predicates::str::contains;
use serde_json::json;

/// Starts an update of one new upstream commit. Returns the update head and
/// the upstream target.
fn started(f: &Fork) -> (String, String) {
    let env = &f.env;
    let target = env.push_upstream(&f.up, &[("n.txt", "n\n")], "upstream: n");
    f.tool(&["start"]).success();
    (env.rev(&env.update_worktree(&f.down), "HEAD"), target)
}

/// The tip of `branch` on the publish remote, or `None`.
fn on_remote(f: &Fork, branch: &str) -> Option<String> {
    common::forge::branch_tip(&f.publish, branch)
}

/// Pushes `commit` to the update's branch on the publish remote, as another
/// run does.
fn push_branch(f: &Fork, commit: &str) {
    f.env.git(
        &f.down,
        &[
            "push",
            "-q",
            &f.publish.to_string_lossy(),
            &format!("{commit}:refs/heads/{MERGE_REQUEST_BRANCH}"),
        ],
    );
}

/// Asserts that nothing was pushed and no merge request opened or changed.
fn nothing_published(f: &Fork) {
    assert_eq!(on_remote(f, MERGE_REQUEST_BRANCH), None);
    assert!(f.forge().changes().is_empty(), "{:?}", f.forge().changes());
}

#[test]
fn finish_opens_the_merge_request_and_ends_the_update() {
    let f = Fork::merge_request();
    let env = &f.env;
    // Merge-request mode runs no check: the merge request's pipeline does.
    f.reconfigure("run = \"test -f app.txt\"", "run = \"false\"");
    let main = env.rev(&f.down, "main");
    let (head, target) = started(&f);

    let out = stdout(&f.tool(&["finish"]).success());
    assert!(out.contains("pushed the update head"), "{out}");
    assert!(out.contains("opened merge request !1"), "{out}");
    assert!(!out.contains("verifying"), "{out}");
    assert_eq!(
        on_remote(&f, MERGE_REQUEST_BRANCH).as_deref(),
        Some(head.as_str())
    );
    assert_eq!(env.rev(&f.down, "main"), main);
    env.assert_no_update(&f.down);

    let mr = f.forge().mr(1);
    assert_eq!(mr.source, MERGE_REQUEST_BRANCH);
    assert_eq!(mr.target, "main");
    assert_eq!(mr.squash, Some(false));
    assert_eq!(mr.remove_source_branch, Some(true));
    assert_eq!(mr.title, "Merge upstream update: upstream");
    assert!(
        mr.description.contains("`upstream` from upstream/main"),
        "{}",
        mr.description
    );
    assert!(
        mr.description
            .contains("without squashing it and without rebasing its branch"),
        "{}",
        mr.description
    );
    assert!(
        mr.description.contains("`git intake sync`"),
        "{}",
        mr.description
    );
    assert!(
        mr.description.ends_with(&format!(
            "<!-- git-intake targets\nupstream {target}\n-->\n"
        )),
        "{}",
        mr.description
    );
    // GitLab takes the token as `PRIVATE-TOKEN`.
    for r in f.forge().requests() {
        assert_eq!(r.header("private-token"), Some(TOKEN), "{r:?}");
    }
}

#[test]
fn finish_opens_a_pull_request_on_github() {
    let f = Fork::github();
    let (head, _) = started(&f);
    let out = stdout(&f.tool(&["finish"]).success());
    assert!(out.contains("opened pull request #1"), "{out}");
    assert!(out.contains("the pull request carries the update"), "{out}");
    assert!(!out.contains("merge request"), "{out}");
    assert_eq!(
        on_remote(&f, MERGE_REQUEST_BRANCH).as_deref(),
        Some(head.as_str())
    );
    let pr = f.forge().mr(1);
    assert_eq!(
        (pr.source.as_str(), pr.target.as_str()),
        (MERGE_REQUEST_BRANCH, "main")
    );
    assert!(
        pr.description
            .contains("Merge this pull request without squashing"),
        "{}",
        pr.description
    );
    for r in f.forge().requests() {
        assert_eq!(
            r.header("x-github-api-version"),
            Some("2022-11-28"),
            "{r:?}"
        );
        assert_eq!(
            r.header("authorization"),
            Some(format!("Bearer {TOKEN}").as_str()),
            "{r:?}"
        );
    }
    f.env.assert_no_update(&f.down);
}

#[test]
fn without_a_token_nothing_is_pushed_and_the_variables_are_named() {
    let f = Fork::merge_request();
    started(&f);
    f.forge().clear_requests();
    f.env.unset_var("GITLAB_TOKEN");
    let err = stderr(&f.tool(&["finish"]).code(2));
    assert!(err.contains("set GITLAB_TOKEN in the environment"), "{err}");
    nothing_published(&f);
    assert!(f.forge().requests().is_empty());

    let g = Fork::github();
    started(&g);
    g.forge().clear_requests();
    g.env.unset_var("GITHUB_TOKEN");
    let err = stderr(&g.tool(&["finish"]).code(2));
    // A GitHub Enterprise Server's own Actions jobs may use the job's token.
    assert!(
        err.contains("set GH_ENTERPRISE_TOKEN, GITHUB_ENTERPRISE_TOKEN, GH_TOKEN, or GITHUB_TOKEN"),
        "{err}"
    );
    assert!(g.forge().requests().is_empty());
}

#[test]
fn an_enterprise_server_outside_actions_takes_no_github_com_token() {
    let f = Fork::github();
    let env = &f.env;
    f.reconfigure(
        "via = \"merge-request\"",
        "via = \"merge-request\"\nforge = \"github\"",
    );
    started(&f);
    for var in [
        "GITHUB_ACTIONS",
        "GITHUB_SERVER_URL",
        "GITHUB_API_URL",
        "GITHUB_TOKEN",
    ] {
        env.unset_var(var);
    }
    env.set_var("GH_TOKEN", TOKEN);
    let err = stderr(&f.tool(&["finish"]).code(2));
    assert!(
        err.contains("set GH_ENTERPRISE_TOKEN, or GITHUB_ENTERPRISE_TOKEN"),
        "{err}"
    );
    assert!(!err.contains(TOKEN), "{err}");
    nothing_published(&f);
}

#[test]
fn an_unknown_host_names_the_forge_and_api_settings() {
    let f = Fork::merge_request();
    started(&f);
    f.forge().clear_requests();
    // Outside the forge's CI jobs, the loopback host is no known forge.
    for var in ["GITLAB_CI", "CI_SERVER_URL", "CI_API_V4_URL"] {
        f.env.unset_var(var);
    }
    let err = stderr(&f.tool(&["finish"]).code(2));
    assert!(
        err.contains("which the tool does not recognise as GitLab or GitHub"),
        "{err}"
    );
    assert!(
        err.contains("set `forge`") && err.contains("`api`"),
        "{err}"
    );
    nothing_published(&f);
    assert!(f.forge().requests().is_empty());
}

#[test]
fn an_api_on_another_host_is_refused() {
    let f = Fork::merge_request();
    f.reconfigure(
        "via = \"merge-request\"",
        "via = \"merge-request\"\nforge = \"gitlab\"\napi = \"https://elsewhere.example.com/api/v4\"",
    );
    f.env
        .push_upstream(&f.up, &[("n.txt", "n\n")], "upstream: n");
    let err = stderr(&f.tool(&["start"]).code(2));
    assert!(
        err.contains("the setting `api` names the host 'elsewhere.example.com'"),
        "{err}"
    );
    f.env.assert_no_update(&f.down);
    let err = stderr(&f.tool(&["sync"]).code(2));
    assert!(
        err.contains("the setting `api` names the host 'elsewhere.example.com'"),
        "{err}"
    );
    nothing_published(&f);
    assert!(f.forge().requests().is_empty());
}

#[test]
fn the_publication_setting_comes_from_the_integration_tip() {
    let f = Fork::merge_request();
    let env = &f.env;
    let (_, _) = started(&f);
    // The update's head points `api` elsewhere; the integration tip does
    // not, so the token goes where the integration tip says.
    let wt = env.update_worktree(&f.down);
    let config = std::fs::read_to_string(wt.join(".git-intake.toml")).unwrap().replace(
        "via = \"merge-request\"",
        "via = \"merge-request\"\nforge = \"gitlab\"\napi = \"https://elsewhere.example.com/api/v4\"",
    );
    let head = env.commit(
        &wt,
        &[(".git-intake.toml", &config)],
        "update: point the API elsewhere",
    );
    f.tool(&["finish"]).success();
    assert_eq!(on_remote(&f, MERGE_REQUEST_BRANCH), Some(head));
    assert_eq!(f.forge().open_mrs().len(), 1);
}

#[test]
fn a_rejected_token_is_named_with_the_forge_s_answer_and_never_shown() {
    let f = Fork::merge_request();
    started(&f);
    f.forge()
        .fail("GET", "/projects/team%2Fapp", 401, "401 Unauthorized");
    let finish = f.tool(&["finish"]).code(2);
    let (out, err) = (stdout(&finish), stderr(&finish));
    assert!(err.contains("401 401 Unauthorized"), "{err}");
    assert!(err.contains("the token was not accepted"), "{err}");
    assert!(!out.contains(TOKEN) && !err.contains(TOKEN), "{out}{err}");
    assert_eq!(on_remote(&f, MERGE_REQUEST_BRANCH), None);
}

/// Asserts that `finish` refuses the project's settings, naming `named`,
/// and publishes nothing.
fn refused(f: &Fork, named: &[&str]) {
    let err = stderr(&f.tool(&["finish"]).code(2));
    assert!(
        err.contains("Nothing was pushed. Change these settings"),
        "{err}"
    );
    for n in named {
        assert!(err.contains(n), "{n}: {err}");
    }
    nothing_published(f);
    assert!(
        f.env.update_worktree(&f.down).exists(),
        "the update is kept"
    );
}

/// A GitLab fork with one update started and the project setting `key` set
/// to `value`.
fn gitlab_with(key: &str, value: serde_json::Value) -> Fork {
    let f = Fork::merge_request();
    started(&f);
    f.forge().set(key, value);
    f
}

#[test]
fn gitlab_plain_merge_commits_are_refused() {
    let f = gitlab_with("merge_method", json!("merge"));
    f.forge().set("merge_pipelines_enabled", json!(true));
    refused(
        &f,
        &["merge_method `rebase_merge` or `ff`) instead of `merge`"],
    );
}

#[test]
fn gitlab_without_a_required_pipeline_is_refused() {
    let f = gitlab_with("only_allow_merge_if_pipeline_succeeds", json!(false));
    refused(&f, &["only_allow_merge_if_pipeline_succeeds"]);
    let f = gitlab_with("allow_merge_on_skipped_pipeline", json!(true));
    refused(&f, &["allow_merge_on_skipped_pipeline"]);
}

#[test]
fn gitlab_rebasing_merges_are_refused() {
    let f = gitlab_with("automatic_rebase_enabled", json!(true));
    refused(
        &f,
        &[
            "automatic_rebase_enabled",
            "a rebase drops the update's merge commits",
        ],
    );
    let f = gitlab_with("merge_trains_enabled", json!(true));
    refused(
        &f,
        &[
            "merge_trains_enabled",
            "a rebase drops the update's merge commits",
        ],
    );
    let f = gitlab_with("squash_option", json!("always"));
    refused(&f, &["squash_option `always`"]);
}

#[test]
fn gitlab_fast_forward_without_trains_or_automatic_rebase_qualifies() {
    let f = gitlab_with("merge_method", json!("ff"));
    f.forge().set("merge_trains_enabled", json!(false));
    f.forge().set("automatic_rebase_enabled", json!(false));
    f.tool(&["finish"]).success();
    assert_eq!(f.forge().open_mrs().len(), 1);
}

/// A GitHub fork with one update started.
fn github_started() -> Fork {
    let f = Fork::github();
    started(&f);
    f
}

#[test]
fn github_status_checks_without_the_up_to_date_requirement_are_refused() {
    let f = github_started();
    f.forge().world().rules[0]["parameters"]["strict_required_status_checks_policy"] = json!(false);
    refused(&f, &["require branches to be up to date before merging"]);
}

#[test]
fn github_merge_queues_are_refused() {
    let f = github_started();
    f.forge()
        .world()
        .rules
        .as_array_mut()
        .unwrap()
        .push(json!({"type": "merge_queue", "parameters": {"merge_method": "MERGE"}}));
    refused(
        &f,
        &["cannot confirm that a merge queue lands the tree it tested"],
    );
}

#[test]
fn github_linear_history_is_refused() {
    let f = github_started();
    f.forge()
        .world()
        .rules
        .as_array_mut()
        .unwrap()
        .push(json!({"type": "required_linear_history"}));
    refused(&f, &["the update's merge commits cannot land"]);
}

#[test]
fn github_classic_protection_the_token_cannot_read_is_refused() {
    let f = github_started();
    {
        let mut w = f.forge().world();
        w.rules = json!([]);
        w.protected = true;
        w.protection = Err((403, "Resource not accessible by integration"));
    }
    refused(
        &f,
        &[
            "could not be confirmed",
            "ruleset",
            "`Administration: read`",
        ],
    );
    // Readable classic protection with strict checks qualifies.
    {
        let mut w = f.forge().world();
        w.protection = Ok(json!({
            "required_status_checks": {"strict": true, "contexts": ["ci"], "checks": [{"context": "ci"}]},
            "required_linear_history": {"enabled": false}
        }));
    }
    f.tool(&["finish"]).success();
    assert_eq!(f.forge().open_mrs().len(), 1);
}

#[test]
fn github_merge_settings_hidden_from_the_token_are_refused() {
    let f = github_started();
    f.forge().unset("allow_merge_commit");
    refused(&f, &["could not be confirmed", "push access"]);
}

#[test]
fn a_moved_integration_branch_is_caught_up_without_the_checks() {
    let f = Fork::merge_request();
    let env = &f.env;
    f.reconfigure("run = \"test -f app.txt\"", "run = \"false\"");
    let (head, _) = started(&f);
    let forge = common::forge(env, &f.publish);
    env.commit(&forge, &[("other.txt", "o\n")], "other: lands first");
    env.git(&forge, &["push", "-q", "origin", "HEAD:main"]);
    let tip = env.rev(&f.publish, "main");

    let out = stdout(&f.tool(&["finish"]).success());
    assert!(out.contains("caught up with 'main'"), "{out}");
    assert!(!out.contains("verifying"), "{out}");
    let pushed = on_remote(&f, MERGE_REQUEST_BRANCH).unwrap();
    assert_eq!(env.rev(&f.publish, &format!("{pushed}^1")), tip);
    assert_eq!(env.rev(&f.publish, &format!("{pushed}^2")), head);
    assert_eq!(f.forge().open_mrs().len(), 1);
}

#[test]
fn an_update_already_on_the_integration_branch_is_removed() {
    let f = Fork::merge_request();
    let env = &f.env;
    let (head, _) = started(&f);
    env.git(
        &f.down,
        &[
            "push",
            "-q",
            &f.publish.to_string_lossy(),
            &format!("{head}:refs/heads/main"),
        ],
    );
    let out = stdout(&f.tool(&["finish"]).success());
    assert!(out.contains("the update already landed"), "{out}");
    nothing_published(&f);
    env.assert_no_update(&f.down);
}

#[test]
fn an_open_merge_request_of_another_update_is_refused() {
    let f = Fork::merge_request();
    let env = &f.env;
    let (head, _) = started(&f);
    // Another clone published another update.
    let dir = common::forge(env, &f.publish);
    let other = env.commit(&dir, &[("x.txt", "x\n")], "other update");
    env.git(
        &dir,
        &[
            "push",
            "-q",
            "origin",
            &format!("HEAD:refs/heads/{MERGE_REQUEST_BRANCH}"),
        ],
    );
    let n = f.forge().add(MERGE_REQUEST_BRANCH, "main", "");

    let err = stderr(&f.tool(&["finish"]).code(2));
    assert!(err.contains(&format!("merge request !{n}")), "{err}");
    assert!(
        err.contains("Run `git intake abort`, then `git intake sync`"),
        "{err}"
    );
    assert!(err.contains("`git intake abort --merge-request`"), "{err}");
    assert_eq!(on_remote(&f, MERGE_REQUEST_BRANCH), Some(other));
    assert_eq!(env.rev(&env.update_worktree(&f.down), "HEAD"), head);
}

#[test]
fn a_merge_request_into_another_branch_is_refused() {
    let f = Fork::merge_request();
    started(&f);
    let n = f.forge().add(MERGE_REQUEST_BRANCH, "release", "");
    let err = stderr(&f.tool(&["finish"]).code(2));
    assert!(
        err.contains(&format!(
            "merge request !{n} from 'git-intake/update' into 'release'"
        )),
        "{err}"
    );
    nothing_published(&f);
    f.tool(&["abort"]).success();
    let err = stderr(&f.tool(&["sync"]).code(2));
    assert!(err.contains("into 'release'"), "{err}");
    nothing_published(&f);
}

#[test]
fn an_interrupted_finish_opens_the_merge_request_when_run_again() {
    let f = Fork::merge_request();
    let env = &f.env;
    let (head, _) = started(&f);
    f.forge()
        .fail("POST", "/merge_requests", 500, "500 Internal Server Error");
    let err = stderr(&f.tool(&["finish"]).code(2));
    assert!(err.contains("500 500 Internal Server Error"), "{err}");
    assert_eq!(
        on_remote(&f, MERGE_REQUEST_BRANCH).as_deref(),
        Some(head.as_str())
    );
    assert!(env.update_worktree(&f.down).exists(), "the update is kept");

    f.forge().world().failures.clear();
    let out = stdout(&f.tool(&["finish"]).success());
    assert!(out.contains("is already at the update head"), "{out}");
    assert!(out.contains("opened merge request !1"), "{out}");
    env.assert_no_update(&f.down);
}

#[test]
fn a_merge_request_opened_meanwhile_is_rewritten_instead() {
    let f = Fork::merge_request();
    let (head, _) = started(&f);
    // Another run opened the merge request for the same branch after this
    // one listed none: GitLab answers 409.
    push_branch(&f, &head);
    let n = f
        .forge()
        .add(MERGE_REQUEST_BRANCH, "main", "opened by another run");
    f.forge().world().stale_lists = 1;
    let out = stdout(&f.tool(&["finish"]).success());
    assert!(
        out.contains(&format!(
            "rewrote the title and description of merge request !{n}"
        )),
        "{out}"
    );
    assert_eq!(f.forge().mrs().len(), 1);
    assert!(
        f.forge()
            .mr(n)
            .description
            .contains("<!-- git-intake targets")
    );
    assert!(f.forge().changes().iter().any(|r| r.method == "POST"));
}

#[test]
fn a_pull_request_opened_meanwhile_is_rewritten_instead() {
    let f = Fork::github();
    let (head, _) = started(&f);
    push_branch(&f, &head);
    let n = f
        .forge()
        .add(MERGE_REQUEST_BRANCH, "main", "opened by another run");
    f.forge().world().stale_lists = 1;
    let out = stdout(&f.tool(&["finish"]).success());
    assert!(
        out.contains(&format!(
            "rewrote the title and description of pull request #{n}"
        )),
        "{out}"
    );
    assert_eq!(f.forge().mrs().len(), 1);
}

#[test]
fn merged_and_the_next_update() {
    for how in [How::MergeCommit, How::FastForward] {
        let f = Fork::merge_request();
        let env = &f.env;
        let (head, target) = started(&f);
        f.tool(&["finish"]).success();
        merge_on_fake(env, f.forge(), open_number(f.forge()), how);
        f.tool(&["start"])
            .success()
            .stdout(contains("fast-forwarded 'main'"))
            .stdout(contains("every selected upstream is current"));
        assert!(env.is_ancestor(&f.down, &head, "main"));
        let status = env.tool_json(&f.down, &["status", "--json"], 0);
        assert_eq!(status["upstreams"][0]["last_synced"], target.as_str());
        assert_eq!(status["upstreams"][0]["state"], "current");
        assert_eq!(status["merge_request"], serde_json::Value::Null);
        assert_eq!(status["merged_without_history"], serde_json::Value::Null);
    }
}

#[test]
fn a_branch_of_unknown_origin_is_refused() {
    let f = Fork::merge_request();
    let env = &f.env;
    started(&f);
    let dir = common::forge(env, &f.publish);
    let work = env.commit(&dir, &[("w.txt", "w\n")], "someone's work");
    env.git(
        &dir,
        &[
            "push",
            "-q",
            "origin",
            &format!("HEAD:refs/heads/{MERGE_REQUEST_BRANCH}"),
        ],
    );
    let err = stderr(&f.tool(&["finish"]).code(2));
    assert!(
        err.contains(&format!("the branch '{MERGE_REQUEST_BRANCH}' on 'origin'")),
        "{err}"
    );
    assert!(err.contains(&f.forge().remote_url()), "{err}");
    assert!(err.contains("`git intake abort --merge-request`"), "{err}");
    assert_eq!(on_remote(&f, MERGE_REQUEST_BRANCH), Some(work));
    assert!(f.forge().changes().is_empty());
}

#[test]
fn a_branch_left_by_a_closed_merge_request_is_deleted_first() {
    let f = Fork::github();
    let env = &f.env;
    started(&f);
    f.tool(&["finish"]).success();
    let first = open_number(f.forge());
    // The pull request is closed; GitHub keeps its branch.
    f.forge().end(first, "closed");
    let left = on_remote(&f, MERGE_REQUEST_BRANCH).unwrap();
    // A new update by hand publishes the same targets again. Its merge is
    // dated apart from the first, which a fast run could make in the same
    // second and so as the very same commit.
    env.tool_env(
        &f.down,
        &["start"],
        &[("GIT_COMMITTER_DATE", "2001-01-01T00:00:00Z")],
    )
    .success();
    let head = env.rev(&env.update_worktree(&f.down), "HEAD");
    assert_ne!(head, left);
    let out = stdout(&f.tool(&["finish"]).success());
    assert!(out.contains("deleting it"), "{out}");
    assert!(out.contains("opened pull request #2"), "{out}");
    assert_eq!(on_remote(&f, MERGE_REQUEST_BRANCH), Some(head));
}

#[test]
fn no_publish_closes_the_verified_update_locally() {
    let f = Fork::merge_request();
    let env = &f.env;
    let (head, _) = started(&f);
    let err = stderr(&f.tool(&["finish", "--no-publish"]).code(2));
    assert!(err.contains("the update is not verified"), "{err}");
    f.tool(&["verify"]).success();
    f.tool(&["finish", "--no-publish"])
        .success()
        .stdout(contains(
            "not published to 'origin' (advanced locally only): main",
        ));
    assert_eq!(env.rev(&f.down, "main"), head);
    nothing_published(&f);
}

/// A repository the publish remote pushes to while it is still fetched from
/// its own URL.
fn push_side(f: &Fork) -> std::path::PathBuf {
    let dir = f.env.bare("push-side");
    f.env.git(
        &f.down,
        &["config", "remote.origin.pushurl", &dir.to_string_lossy()],
    );
    dir
}

/// A URL that reads `bare` but refuses every push, through git's `ext`
/// transport, which the fork's repository is allowed to use.
fn read_only_url(f: &Fork, bare: &Path) -> String {
    f.env
        .git(&f.down, &["config", "protocol.ext.allow", "always"]);
    format!(
        "ext::sh -c test% %s% =% upload-pack% &&% exec% git% %s% {}",
        bare.display()
    )
}

/// Makes the publish remote push to exactly `urls`.
fn set_push_urls(f: &Fork, urls: &[&str]) {
    let env = &f.env;
    env.git_output(&f.down, &["config", "--unset-all", "remote.origin.pushurl"]);
    for url in urls {
        env.git(&f.down, &["config", "--add", "remote.origin.pushurl", url]);
    }
}

#[test]
fn the_branch_is_pushed_where_pushes_go() {
    let f = Fork::merge_request();
    let (head, _) = started(&f);
    let push_side = push_side(&f);
    f.tool(&["finish"]).success();
    assert_eq!(
        common::forge::branch_tip(&push_side, MERGE_REQUEST_BRANCH),
        Some(head)
    );
}

#[test]
fn only_push_urls_without_the_branch_receive_the_push() {
    let f = Fork::merge_request();
    let env = &f.env;
    let (head, _) = started(&f);
    let holding = env.bare("holding");
    env.git(
        &f.down,
        &[
            "push",
            "-q",
            &holding.to_string_lossy(),
            &format!("{head}:refs/heads/{MERGE_REQUEST_BRANCH}"),
        ],
    );
    let lacking = env.bare("lacking");
    // A push to the URL that already holds the branch would fail.
    let read_only = read_only_url(&f, &holding);
    set_push_urls(&f, &[&read_only, &lacking.to_string_lossy()]);
    f.tool(&["finish"]).success();
    assert_eq!(
        common::forge::branch_tip(&lacking, MERGE_REQUEST_BRANCH),
        Some(head)
    );
}

#[test]
fn a_failed_push_names_where_the_branch_went() {
    let f = Fork::merge_request();
    let env = &f.env;
    let (head, _) = started(&f);
    let holding = env.bare("holding");
    env.git(
        &f.down,
        &[
            "push",
            "-q",
            &holding.to_string_lossy(),
            &format!("{head}:refs/heads/{MERGE_REQUEST_BRANCH}"),
        ],
    );
    let receiving = env.bare("receiving").to_string_lossy().into_owned();
    let holding = read_only_url(&f, &holding);
    let refusing = read_only_url(&f, &env.bare("refusing"));
    set_push_urls(&f, &[&holding, &receiving, &refusing]);
    let err = stderr(&f.tool(&["finish"]).code(2));
    assert!(
        err.contains(&format!("The branch is already on {receiving}")),
        "{err}"
    );
    assert!(err.contains("The branch is never forced"), "{err}");
    assert!(f.forge().changes().is_empty());

    let g = Fork::merge_request();
    started(&g);
    let receiving = g.env.bare("receiving").to_string_lossy().into_owned();
    let refusing = read_only_url(&g, &g.env.bare("refusing"));
    set_push_urls(&g, &[&receiving, &refusing]);
    let err = stderr(&g.tool(&["finish"]).code(2));
    assert!(
        err.contains("The branch may already be on some of the push URLs of 'origin'"),
        "{err}"
    );

    let h = Fork::merge_request();
    started(&h);
    let refusing = read_only_url(&h, &h.env.bare("refusing"));
    set_push_urls(&h, &[&refusing]);
    let err = stderr(&h.tool(&["finish"]).code(2));
    assert!(
        err.contains("Nothing was pushed, and no merge request was opened"),
        "{err}"
    );
}

#[test]
fn the_token_is_not_sent_to_another_host_on_a_redirect() {
    let f = Fork::merge_request();
    started(&f);
    // The project answers with a redirect to another host, which records
    // what it receives.
    let mut elsewhere = mockito::Server::new();
    let target = elsewhere
        .mock("GET", "/moved")
        .match_header("private-token", mockito::Matcher::Missing)
        .with_status(404)
        .with_body(r#"{"message": "404 Not found"}"#)
        .create();
    let moved = format!(
        "{}/moved",
        elsewhere.url().replace("127.0.0.1", "localhost")
    );
    f.forge().redirect("GET", "/projects/team%2Fapp", &moved);
    let err = stderr(&f.tool(&["finish"]).code(2));
    assert!(err.contains("404"), "{err}");
    assert!(!err.contains(TOKEN), "{err}");
    target.assert();
    nothing_published(&f);
}

#[test]
fn a_sync_record_merged_with_a_merge_commit_is_the_last_synced_commit() {
    let f = Fork::merge_request();
    let env = &f.env;
    let (head, target) = started(&f);
    f.tool(&["finish"]).success();
    merge_on_fake(env, f.forge(), open_number(f.forge()), How::MergeCommit);
    env.git(&f.down, &["pull", "-q", "--ff-only", "origin", "main"]);
    assert_eq!(env.rev(&f.down, "main^2"), head);
    let status = env.tool_json(&f.down, &["status", "--json"], 0);
    let up = &status["upstreams"][0];
    assert_eq!(up["last_synced"], target.as_str());
    assert_eq!(up["last_synced_source"], "record");

    // The next update describes its range from that record.
    let next = env.push_upstream(&f.up, &[("m.txt", "m\n")], "upstream: m");
    f.tool(&["start"]).success();
    f.tool(&["finish"]).success();
    let description = f.forge().mr(open_number(f.forge())).description;
    assert!(
        description.contains(&format!("{}..{} (1 commit)", &target[..12], &next[..12])),
        "{description}"
    );
}

#[test]
fn a_squash_with_the_trailers_does_not_count_as_a_sync_record() {
    let f = Fork::github();
    let env = &f.env;
    let (_, target) = started(&f);
    f.tool(&["finish"]).success();
    let n = open_number(f.forge());
    merge_on_fake(env, f.forge(), n, How::Squash);
    env.git(&f.down, &["pull", "-q", "--ff-only", "origin", "main"]);
    // The squash commit's message carries the sync merge's trailers.
    let message = env.git(&f.down, &["log", "-1", "--format=%B", "main"]);
    assert!(
        message.contains(&format!("Intake-Target: {target}")),
        "{message}"
    );
    let status = env.tool_json(&f.down, &["status", "--json"], 0);
    assert_ne!(status["upstreams"][0]["last_synced"], target.as_str());
    assert_eq!(status["merged_without_history"]["number"], n);
    let out = stdout(&f.tool(&["status"]).success());
    assert!(out.contains(&format!("pull request #{n}")), "{out}");
    assert!(
        out.contains("was merged without the update's history"),
        "{out}"
    );
}

#[test]
fn checks_the_update_changes_are_named_in_the_merge_request() {
    let f = Fork::merge_request();
    let env = &f.env;
    f.reconfigure(
        &common::checks_line(&[common::FORK_CHECK]),
        &common::checks_line(&[common::FORK_CHECK, ("lint", "true")]),
    );
    started(&f);
    // The update adds the check `audit`, gives `app` another command and
    // removes `lint`.
    let wt = env.update_worktree(&f.down);
    let cfg = fs::read_to_string(wt.join(".git-intake.toml"))
        .unwrap()
        .replace(
            &common::checks_line(&[common::FORK_CHECK, ("lint", "true")]),
            &common::checks_line(&[("app", "test -s app.txt"), ("audit", "true")]),
        );
    env.commit(
        &wt,
        &[(".git-intake.toml", &cfg)],
        "fork: change the checks",
    );
    let out = stdout(&f.tool(&["finish"]).success());
    let named = "the check `audit` added to `.git-intake.toml`, the check `lint` removed from \
                 `.git-intake.toml`, the check `app` given another command in `.git-intake.toml`";
    assert!(
        out.contains(&format!("the update changes its own checks: {named}")),
        "{out}"
    );
    let description = f.forge().mr(1).description;
    assert!(description.contains(named), "{description}");
}

#[test]
fn checks_only_reordered_are_named_as_such() {
    let f = Fork::merge_request();
    let env = &f.env;
    f.reconfigure(
        &common::checks_line(&[common::FORK_CHECK]),
        &common::checks_line(&[common::FORK_CHECK, ("lint", "true")]),
    );
    started(&f);
    let wt = env.update_worktree(&f.down);
    let cfg = fs::read_to_string(wt.join(".git-intake.toml"))
        .unwrap()
        .replace(
            &common::checks_line(&[common::FORK_CHECK, ("lint", "true")]),
            &common::checks_line(&[("lint", "true"), common::FORK_CHECK]),
        );
    env.commit(
        &wt,
        &[(".git-intake.toml", &cfg)],
        "fork: reorder the checks",
    );
    let out = stdout(&f.tool(&["finish"]).success());
    assert!(
        out.contains(
            "the update changes its own checks: the order of the checks of `.git-intake.toml`;"
        ),
        "{out}"
    );

    let description = f.forge().mr(1).description;
    assert!(
        description.contains("the order of the checks of `.git-intake.toml`"),
        "{description}"
    );
}

#[test]
fn reading_the_publish_remote_leaves_the_repository_s_tags_and_branches() {
    let f = Fork::merge_request();
    let env = &f.env;
    let local = env.rev(&f.down, "main");
    let dir = common::forge(env, &f.publish);
    env.commit(&dir, &[("other.txt", "o\n")], "other: elsewhere");
    env.git(
        &dir,
        &[
            "push",
            "-q",
            "origin",
            "HEAD:refs/heads/side",
            "HEAD:refs/tags/own",
        ],
    );
    env.git(&f.down, &["branch", "side", "main"]);
    env.git(&f.down, &["tag", "own", "main"]);
    for spec in [
        "+refs/tags/*:refs/tags/*",
        "+refs/heads/side:refs/heads/side",
        "+refs/heads/*:refs/heads/mirror/*",
    ] {
        env.git(&f.down, &["config", "--add", "remote.origin.fetch", spec]);
    }
    f.tool(&["status"]).success();
    assert_eq!(env.rev(&f.down, "own^{commit}"), local);
    assert_eq!(env.rev(&f.down, "side"), local);
    assert!(!env.has_rev(&f.down, "refs/heads/mirror/main"));
}
