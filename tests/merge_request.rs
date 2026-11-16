//! Publication through a merge request: `finish` requires the forge's merge
//! settings, pushes the update head to `git-intake/update` without force,
//! opens the merge request through the forge's API, and ends the local
//! update. The forge is a fake one; see `common::forge`.

mod common;

use std::fs;
use std::path::Path;

use common::forge::{Action, Step, TOKEN};
use common::{
    BOT, Fork, How, MERGE_REQUEST_BRANCH, PAUSE_REF, dead_pause, dead_pause_in_flight,
    merge_on_fake, open_number, record, stderr, stdout, survives_each_failed_request,
};
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
fn finish_opens_the_merge_request_through_a_failed_request() {
    // Past the push nothing runs `finish` again, so the request is retried
    // in place.
    let f = Fork::merge_request();
    started(&f);
    f.forge().fail_times("POST", "/merge_requests", 502, 1);
    let out = stdout(
        &f.env
            .tool_env(
                &f.down,
                &["finish"],
                &[("GIT_INTAKE_TEST_RETRY_GAP_MS", "0")],
            )
            .success(),
    );
    assert!(out.contains("opened merge request !1"), "{out}");
    assert_eq!(open_number(f.forge()), 1);
    f.env.assert_no_update(&f.down);
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
        let main = env.rev(&f.down, "main");
        merge_on_fake(env, f.forge(), open_number(f.forge()), how);
        // The next update reads the merged integration branch from the
        // publish remote, and the local `main` stays where it was.
        f.tool(&["start"])
            .success()
            .stdout(contains("every selected upstream is current"));
        assert!(env.is_ancestor(&f.publish, &head, "main"));
        assert_eq!(env.rev(&f.down, "main"), main);
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

/// Opens the update's merge request with `finish`; returns its number and
/// head.
fn opened(f: &Fork) -> (u64, String) {
    started(f);
    f.tool(&["finish"]).success();
    let number = open_number(f.forge());
    (number, on_remote(f, MERGE_REQUEST_BRANCH).unwrap())
}

/// A commit on top of the publish remote's integration branch, pushed to
/// `branch`; the integration branch itself moves only when `branch` is it.
fn on_main(f: &Fork, files: &[(&str, &str)], branch: &str) -> String {
    let env = &f.env;
    let dir = common::forge(env, &f.publish);
    env.git(&dir, &["fetch", "-q", "origin"]);
    env.git(&dir, &["checkout", "-q", "--detach", "origin/main"]);
    let c = env.commit(&dir, files, "other: lands");
    env.git(
        &dir,
        &[
            "push",
            "-q",
            "origin",
            &format!("+HEAD:refs/heads/{branch}"),
        ],
    );
    c
}

fn land(f: &Fork, args: &[&str]) -> assert_cmd::assert::Assert {
    let mut all = vec!["land", "--poll", "0"];
    // A wait that does not end fails the test instead of hanging it.
    if !args.contains(&"--timeout") {
        all.extend(["--timeout", "120"]);
    }
    all.extend(args);
    f.tool(&all)
}

/// The merge requests asked to merge, with what they asked.
fn merges(f: &Fork) -> Vec<serde_json::Value> {
    f.forge()
        .requests()
        .into_iter()
        .filter(|r| r.method == "PUT" && r.path.ends_with("/merge"))
        .map(|r| r.json())
        .collect()
}

#[test]
fn land_merges_a_head_that_contains_the_integration_tip() {
    let f = Fork::merge_request();
    let env = &f.env;
    let (number, head) = opened(&f);
    let out = stdout(&land(&f, &[]).success());
    assert!(
        out.contains(&format!("merged the merge request !{number}")),
        "{out}"
    );
    assert!(!out.contains("catching"), "{out}");
    assert_eq!(f.forge().mr(number).state, "merged");
    // Semi-linear history lands the head with a merge commit.
    assert_eq!(env.rev(&f.publish, "main^2"), head);
    assert_eq!(on_remote(&f, MERGE_REQUEST_BRANCH), None);
    assert_eq!(
        merges(&f),
        [json!({"sha": head, "squash": false, "should_remove_source_branch": true})]
    );
    // The sync records stay on the integration line.
    f.tool(&["start"])
        .success()
        .stdout(contains("every selected upstream is current"));
    env.assert_no_update(&f.down);
    // Nothing more to land.
    land(&f, &[]).success().stdout(contains(
        "the update has no open merge request into 'main'; nothing to land",
    ));
    // Fast-forward merges land the head itself.
    let f = gitlab_with("merge_method", json!("ff"));
    let (number, head) = {
        f.tool(&["finish"]).success();
        (
            open_number(f.forge()),
            on_remote(&f, MERGE_REQUEST_BRANCH).unwrap(),
        )
    };
    land(&f, &[]).success();
    assert_eq!(f.forge().mr(number).state, "merged");
    assert_eq!(f.env.rev(&f.publish, "main"), head);
}

#[test]
fn land_catches_a_moved_branch_up_and_merges_the_new_head() {
    let f = Fork::merge_request();
    let env = &f.env;
    let (number, head) = opened(&f);
    let other = on_main(&f, &[("other.txt", "o\n")], "main");
    let out = stdout(&land(&f, &[]).success());
    assert!(
        out.contains(&format!(
            "'main' moved to {} past the head {}",
            &other[..12],
            &head[..12]
        )),
        "{out}"
    );
    let caught = env.rev(&f.publish, "main^2");
    let parents = env.git(&f.publish, &["rev-list", "--parents", "-n", "1", &caught]);
    assert_eq!(parents, format!("{caught} {other} {head}"));
    let message = env.git(&f.publish, &["log", "-1", "--format=%B", &caught]);
    assert!(message.contains("Intake-Catch-Up: main"), "{message}");
    assert_eq!(f.forge().mr(number).state, "merged");
    assert_eq!(merges(&f).len(), 1);
    assert_eq!(merges(&f)[0]["sha"], caught.as_str());
}

#[test]
fn land_waits_for_the_pipeline_before_it_catches_up() {
    let f = Fork::merge_request();
    let env = &f.env;
    let (number, head) = opened(&f);
    let other = on_main(&f, &[("other.txt", "o\n")], "side");
    f.forge().script(
        &head,
        vec![
            Step::is("running"),
            Step::is("running").then(Action::SetRef("refs/heads/main".into(), other.clone())),
            Step::is("success"),
        ],
    );
    let out = stdout(&land(&f, &[]).success());
    let waited = out.find("waiting for the pipeline of the head").unwrap();
    let caught = out.find("'main' moved to").unwrap();
    assert!(waited < caught, "{out}");
    assert_eq!(env.rev(&f.publish, "main^2^1"), other);
    assert_eq!(f.forge().mr(number).state, "merged");
}

#[test]
fn land_stops_on_a_failed_pipeline_and_a_fix_carries_on() {
    let f = Fork::merge_request();
    let env = &f.env;
    let (number, head) = opened(&f);
    f.forge().script(&head, vec![Step::is("failed")]);
    let out = stdout(&land(&f, &[]).code(1));
    assert!(
        out.contains(&format!(
            "the pipeline of the head {} of the merge request !{number}",
            &head[..12]
        )),
        "{out}"
    );
    assert!(out.contains("/-/pipelines/"), "{out}");
    assert!(
        out.contains(&format!(
            "Push a fix to the branch '{MERGE_REQUEST_BRANCH}'"
        )),
        "{out}"
    );
    assert!(merges(&f).is_empty());
    // A fix on the update's branch lands.
    let dir = common::forge(env, &f.publish);
    env.git(&dir, &["fetch", "-q", "origin"]);
    env.git(
        &dir,
        &[
            "checkout",
            "-q",
            "--detach",
            &format!("origin/{MERGE_REQUEST_BRANCH}"),
        ],
    );
    let fix = env.commit(&dir, &[("fix.txt", "f\n")], "fork: fix");
    env.git(
        &dir,
        &[
            "push",
            "-q",
            "origin",
            &format!("HEAD:refs/heads/{MERGE_REQUEST_BRANCH}"),
        ],
    );
    land(&f, &[]).success();
    assert_eq!(env.rev(&f.publish, "main^2"), fix);
    // A canceled pipeline stops it the same way.
    let f = Fork::merge_request();
    let (_, head) = opened(&f);
    f.forge()
        .script(&head, vec![Step::is("running"), Step::is("canceled")]);
    land(&f, &[]).code(1).stdout(contains("was canceled"));
}

#[test]
fn land_stops_on_a_conflicting_catch_up_and_carries_on_after_it() {
    let f = Fork::merge_request();
    let env = &f.env;
    let (number, _) = opened(&f);
    on_main(&f, &[("n.txt", "theirs\n")], "main");
    let out = stdout(&land(&f, &[]).code(1));
    assert!(out.contains("land stopped on the catch-up merge"), "{out}");
    assert!(
        out.contains("`git intake land` carries the merge request"),
        "{out}"
    );
    let status = env.tool_json(&f.down, &["status", "--json"], 0);
    assert_eq!(status["update"]["step"], "catch_up_conflict");
    land(&f, &[])
        .code(2)
        .stderr(contains("an update is in progress"));
    let wt = env.update_worktree(&f.down);
    fs::write(wt.join("n.txt"), "both\n").unwrap();
    env.git(&wt, &["add", "n.txt"]);
    f.tool(&["continue"]).success();
    f.tool(&["finish"]).success();
    land(&f, &[]).success();
    assert_eq!(f.forge().mr(number).state, "merged");
    assert_eq!(env.git(&f.publish, &["show", "main:n.txt"]), "both");
}

#[test]
fn land_reports_a_merge_someone_else_made() {
    let f = Fork::merge_request();
    let (number, head) = opened(&f);
    f.forge().script(
        &head,
        vec![
            Step::is("running"),
            Step::is("success").then(Action::Merge(number)),
        ],
    );
    let out = stdout(&land(&f, &[]).success());
    assert!(
        out.contains(&format!("the merge request !{number}")),
        "{out}"
    );
    assert!(out.contains("is merged into 'main'"), "{out}");
    assert!(merges(&f).is_empty());
}

#[test]
fn land_refuses_what_finish_refuses_and_gives_up_after_its_timeout() {
    let f = Fork::merge_request();
    let (_, head) = opened(&f);
    f.forge().set("merge_method", json!("merge"));
    let err = stderr(&land(&f, &[]).code(2));
    assert!(
        err.contains("Nothing was merged. Change these settings"),
        "{err}"
    );
    assert!(err.contains("merge_method"), "{err}");
    f.forge().set("merge_method", json!("rebase_merge"));
    f.forge().script(&head, vec![Step::is("running")]);
    let err = stderr(&land(&f, &["--timeout", "0"]).code(2));
    assert!(
        err.contains("gave up after 0s waiting for the pipeline of the head"),
        "{err}"
    );
    assert!(merges(&f).is_empty());
    // Push mode has no merge request to land.
    let f = Fork::new();
    f.tool(&["land"])
        .code(2)
        .stderr(contains("with `via = \"push\"` the update has none"));
}

#[test]
fn land_merges_a_pull_request_with_a_merge_commit() {
    let f = Fork::github();
    let env = &f.env;
    let (number, head) = opened(&f);
    let main = env.rev(&f.publish, "main");
    f.forge()
        .script(&head, vec![Step::is("running"), Step::is("success")]);
    let out = stdout(&land(&f, &[]).success());
    assert!(
        out.contains(&format!("merged the pull request #{number}")),
        "{out}"
    );
    let merged = env.rev(&f.publish, "main");
    let parents = env.git(&f.publish, &["rev-list", "--parents", "-n", "1", &merged]);
    assert_eq!(parents, format!("{merged} {main} {head}"));
    assert_eq!(merges(&f), [json!({"sha": head, "merge_method": "merge"})]);
    // GitHub's merge takes no request to remove the branch, so `land` does.
    assert_eq!(on_remote(&f, MERGE_REQUEST_BRANCH), None);
    // A push after the merge is someone's work, and is kept.
    let f = Fork::github();
    let (number, head) = opened(&f);
    let later = on_main(&f, &[("later.txt", "l\n")], "side");
    f.forge().script(&head, vec![Step::is("success")]);
    f.forge().on_merge(Action::SetRef(
        format!("refs/heads/{MERGE_REQUEST_BRANCH}"),
        later.clone(),
    ));
    f.forge().pin_head(number, &head);
    let out = stdout(&land(&f, &[]).success());
    assert!(out.contains("kept the branch"), "{out}");
    assert_eq!(on_remote(&f, MERGE_REQUEST_BRANCH), Some(later));
}

#[test]
fn land_on_github_reads_every_check_and_waits_for_a_required_one() {
    // A failed check past the first page counts.
    let f = Fork::github();
    let (_, _) = opened(&f);
    {
        let mut w = f.forge().world();
        for i in 0..120 {
            w.check_runs
                .push((format!("job-{i}"), "completed", Some("success")));
        }
        w.check_runs
            .push(("late".into(), "completed", Some("failure")));
    }
    land(&f, &[]).code(1).stdout(contains("failed"));
    assert!(merges(&f).is_empty());

    // A required check that has not registered yet is a wait: another check
    // that passed does not make the head pass.
    let f = Fork::github();
    let (number, head) = opened(&f);
    f.forge()
        .world()
        .check_runs
        .push(("lint".into(), "completed", Some("success")));
    f.forge()
        .script(&head, vec![Step::none(), Step::none(), Step::is("success")]);
    let out = stdout(&land(&f, &[]).success());
    assert!(
        out.contains("waiting for the pipeline of the head"),
        "{out}"
    );
    assert_eq!(merges(&f).len(), 1);
    assert_eq!(f.forge().mr(number).state, "merged");
}

#[test]
fn land_on_github_names_the_permission_a_refused_read_needs() {
    for (suffix, permission) in [
        ("/check-runs", "Checks: read"),
        ("/status", "Commit statuses: read"),
    ] {
        let f = Fork::github();
        let (_, _) = opened(&f);
        f.forge().fail(
            "GET",
            suffix,
            403,
            "Resource not accessible by personal access token",
        );
        let err = stderr(&land(&f, &[]).code(2));
        assert!(err.contains(permission), "{err}");
        assert!(err.contains("Resource not accessible"), "{err}");
        assert!(merges(&f).is_empty());
    }
}

#[test]
fn lists_of_the_forge_are_read_to_their_end() {
    for github in [false, true] {
        // The open merge request lies past the first page of the update's.
        let f = if github {
            Fork::github()
        } else {
            Fork::merge_request()
        };
        let (number, _) = opened(&f);
        for _ in 0..100 {
            let older = f.forge().add(MERGE_REQUEST_BRANCH, "main", "");
            f.forge().edit(older, |m| {
                m.state = "closed";
                m.head = Some("0".repeat(40));
            });
        }
        let status = f.env.tool_json(&f.down, &["status", "--json"], 0);
        assert_eq!(status["merge_request"]["number"], number, "{status}");
    }

    // A merge request's assignment lies past the first page of its notes, and
    // still decides its place in the queue.
    let (f, _, _, q) = queued_fork(2);
    let fake = f.forge();
    fake.assign(q[1], BOT);
    for i in 0..150 {
        fake.add_note(q[1], &format!("note {i}"));
    }
    fake.assign(q[0], BOT);
    on_main(&f, &[("other.txt", "o\n")], "main");
    let out = stdout(&land(&f, &[]).success());
    assert!(
        out.contains(&format!(
            "paused the merge queue of @merge-bot: !{}, !{}",
            q[1], q[0]
        )),
        "{out}"
    );

    // The queue goes on past the first page of the merge requests.
    let (f, number, _, _) = queued_fork(0);
    let fake = f.forge();
    let many: Vec<u64> = (0..101)
        .map(|_| {
            let n = fake.add("other", "main", "");
            fake.edit(n, |m| {
                m.assignees = vec![BOT];
                m.pinned_head = Some("0".repeat(40));
            });
            n
        })
        .collect();
    let head = on_remote(&f, MERGE_REQUEST_BRANCH).unwrap();
    on_main(&f, &[("other.txt", "o\n")], "main");
    fake.script(&head, vec![Step::is("success"), Step::is("failed")]);
    fake.script_branch(MERGE_REQUEST_BRANCH, vec![Step::is("failed")]);
    land(&f, &[]).code(1);
    let (_, record) = fake.world().assignments[0].clone();
    let record = parsed_record(record.as_deref().unwrap());
    assert_eq!(held_in(&record).len(), many.len());
    assert_eq!(fake.mr(number).state, "open");
    for n in &many {
        assert_eq!(fake.mr(*n).assignees, [BOT]);
    }
}

/// A GitLab fork whose merge queue is the account `merge-bot`, with the
/// update's merge request open and `queued` other merge requests assigned
/// to the queue, in this order of numbers on the forge. Returns the update's
/// number and head, and the queued numbers.
fn queued_fork(queued: usize) -> (Fork, u64, String, Vec<u64>) {
    let f = Fork::merge_request();
    f.env.set_var(
        "GIT_INTAKE_TEST_RETURN_GAPS",
        &f.env.path("return-gaps").to_string_lossy(),
    );
    f.reconfigure(
        "via = \"merge-request\"\n",
        "via = \"merge-request\"\nqueue = \"merge-bot\"\n",
    );
    f.forge().add_user(BOT, "merge-bot");
    f.forge().add_user(7, "someone");
    let (number, head) = opened(&f);
    let numbers = (0..queued)
        .map(|i| {
            let branch = format!("feature-{i}");
            on_main(&f, &[(&format!("{branch}.txt"), "f\n")], &branch);
            f.forge().add(&branch, "main", "")
        })
        .collect();
    (f, number, head, numbers)
}

/// The requests that change assignments, by merge request number.
fn reassigned(f: &Fork) -> Vec<(u64, serde_json::Value)> {
    f.forge()
        .requests()
        .into_iter()
        .filter(|r| r.method == "PUT" && r.json().get("assignee_ids").is_some())
        .map(|r| {
            let number = r.path.rsplit('/').next().unwrap().parse().unwrap();
            (number, r.json()["assignee_ids"].clone())
        })
        .collect()
}

/// The bodies of the notes the tool wrote on merge request `number`.
fn written(f: &Fork, number: u64) -> Vec<String> {
    f.forge()
        .notes(number)
        .into_iter()
        .filter(|n| !n.system)
        .map(|n| n.body)
        .collect()
}

#[test]
fn land_pauses_the_queue_in_its_order_for_the_last_pipeline() {
    let (f, number, _, q) = queued_fork(3);
    let fake = f.forge();
    // Assigned in the order of the second, the first and the third.
    for n in [q[1], q[0], q[2]] {
        fake.assign(n, BOT);
    }
    on_main(&f, &[("other.txt", "o\n")], "main");
    fake.clear_requests();
    let out = stdout(&land(&f, &[]).success());
    let order = format!("!{}, !{}, !{}", q[1], q[0], q[2]);
    assert!(
        out.contains(&format!("paused the merge queue of @merge-bot: {order}")),
        "{out}"
    );
    assert!(
        out.contains(&format!(
            "ended the pause of the merge queue of @merge-bot; assigned to it again: {order}"
        )),
        "{out}"
    );
    let notes = written(&f, number);
    assert_eq!(notes.len(), 2, "{notes:?}");
    assert!(
        notes[0].contains(&format!(
            "get it back in this order once the pause ends: {order}."
        )),
        "{}",
        notes[0]
    );
    assert!(notes[1].contains("ended the pause"), "{}", notes[1]);
    assert!(!notes.iter().any(|n| n.contains("<!--")), "{notes:?}");
    // The record stands before any assignment is taken and until the last
    // has returned, and the assignments return in the recorded order once
    // the update merged.
    let requests = fake.requests();
    let saw = fake.world().assignments.clone();
    assert_eq!(saw.len(), 6);
    for (_, record) in &saw {
        let record = parsed_record(record.as_deref().expect("the record stands"));
        assert_eq!(record["account"]["username"], "merge-bot");
        assert_eq!(record["merge_request"]["number"], number);
        assert_eq!(held_in(&record), [q[1], q[0], q[2]]);
    }
    assert!(recorded(&f).is_none());
    assert_eq!(
        reassigned(&f),
        [
            (q[1], json!([])),
            (q[0], json!([])),
            (q[2], json!([])),
            (q[1], json!([BOT])),
            (q[0], json!([BOT])),
            (q[2], json!([BOT])),
        ]
    );
    let merged = requests
        .iter()
        .position(|r| r.path.ends_with("/merge"))
        .unwrap();
    let back = requests
        .iter()
        .rposition(|r| r.json().get("assignee_ids") == Some(&json!([])))
        .unwrap();
    assert!(back < merged);
    assert_eq!(fake.mr(number).state, "merged");
    for n in &q {
        assert_eq!(fake.mr(*n).assignees, [BOT]);
    }
    // The forge's notes show the same order.
    let at: Vec<u64> = [q[1], q[0], q[2]]
        .iter()
        .map(|n| fake.notes(*n).last().unwrap().at)
        .collect();
    assert!(at.windows(2).all(|w| w[0] < w[1]), "{at:?}");
}

#[test]
fn the_returns_are_spaced_so_the_queue_keeps_their_order() {
    let (f, number, _, q) = queued_fork(3);
    dead_pause(&f, number, &[q[0], q[1], q[2]]);
    f.tool(&["land", "--resume"]).success();
    // One wait of a second between each two returns, recorded instead of taken.
    let gaps = fs::read_to_string(f.env.path("return-gaps")).unwrap();
    assert_eq!(gaps, "1000\n1000\n");
    assert_eq!(reassigned(&f).len(), 3);
}

#[test]
fn land_clears_a_squash_the_merge_request_asks_for() {
    // A person ticked squash on the merge request: `squash: false` in the
    // merge changes nothing on GitLab, so the flag goes first.
    let f = Fork::merge_request();
    let env = &f.env;
    let (number, head) = opened(&f);
    f.forge().edit(number, |m| m.squash = Some(true));
    f.forge().clear_requests();
    land(&f, &[]).success();
    assert_eq!(f.forge().mr(number).state, "merged");
    assert!(env.is_ancestor(&f.publish, &head, "main"));
    assert_eq!(env.rev(&f.publish, "main^2"), head);
    let changes = f.forge().changes();
    assert_eq!(changes[0].json(), json!({"squash": false}));
    assert!(changes[1].path.ends_with("/merge"));
    // One that does not ask for it is not touched.
    let f = Fork::merge_request();
    let (_, _) = opened(&f);
    f.forge().clear_requests();
    land(&f, &[]).success();
    assert_eq!(f.forge().changes().len(), 1);
}

#[test]
fn land_does_not_merge_a_head_the_forge_has_not_read() {
    // Someone pushed a fix; the forge still reports the head before it, with
    // that head's passed pipeline.
    let f = Fork::merge_request();
    let env = &f.env;
    let (number, head) = opened(&f);
    let dir = common::forge(env, &f.publish);
    env.git(&dir, &["fetch", "-q", "origin"]);
    env.git(
        &dir,
        &[
            "checkout",
            "-q",
            "--detach",
            &format!("origin/{MERGE_REQUEST_BRANCH}"),
        ],
    );
    let fix = env.commit(&dir, &[("fix.txt", "f\n")], "fork: fix");
    env.git(
        &dir,
        &[
            "push",
            "-q",
            "origin",
            &format!("HEAD:refs/heads/{MERGE_REQUEST_BRANCH}"),
        ],
    );
    f.forge().lag_head(number, &head, 3);
    f.forge().script(&fix, vec![Step::is("success")]);
    let out = stdout(&land(&f, &[]).success());
    assert!(
        out.contains(&format!(
            "waiting for the forge to read the head {}",
            &fix[..12]
        )),
        "{out}"
    );
    assert_eq!(merges(&f).len(), 1);
    assert_eq!(merges(&f)[0]["sha"], fix.as_str());
    assert_eq!(env.rev(&f.publish, "main^2"), fix);
}

#[test]
fn land_waits_while_the_forge_decides_or_the_head_moves_under_the_merge() {
    let f = Fork::merge_request();
    let (number, head) = opened(&f);
    f.forge().answer_merges(&[
        (409, "SHA does not match HEAD of source branch", "mergeable"),
        (405, "405 Method Not Allowed", "checking"),
        (422, "Branch cannot be merged", "ci_still_running"),
    ]);
    let out = stdout(&land(&f, &[]).success());
    assert!(
        out.contains(&format!(
            "waiting for GitLab to let the merge request !{number} ("
        )),
        "{out}"
    );
    assert_eq!(merges(&f).len(), 4);
    assert_eq!(f.env.rev(&f.publish, "main^2"), head);
}

#[test]
fn land_catches_up_when_the_branch_moves_under_the_merge() {
    let f = Fork::merge_request();
    let env = &f.env;
    let (number, head) = opened(&f);
    let other = on_main(&f, &[("other.txt", "o\n")], "side");
    f.forge()
        .on_merge(Action::SetRef("refs/heads/main".into(), other.clone()));
    let out = stdout(&land(&f, &[]).success());
    assert!(out.contains("'main' moved to"), "{out}");
    assert_eq!(merges(&f).len(), 2);
    assert_eq!(merges(&f)[0]["sha"], head.as_str());
    assert_eq!(env.rev(&f.publish, "main^2^1"), other);
    assert_eq!(f.forge().mr(number).state, "merged");
}

#[test]
fn a_refused_merge_names_why_and_a_contested_one_is_tried_again() {
    // Refused for what a wait does not change, with the forge's reason.
    let f = Fork::merge_request();
    let (number, _) = opened(&f);
    f.forge().edit(number, |m| m.blocked = Some("not_approved"));
    let err = stderr(&land(&f, &[]).code(2));
    assert!(err.contains("GitLab refused to merge"), "{err}");
    assert!(err.contains("405 Method Not Allowed"), "{err}");
    assert!(err.contains("not_approved"), "{err}");
    assert_eq!(merges(&f).len(), 1);

    // The forge's reason for a merge that failed comes along.
    let f = Fork::merge_request();
    let (number, _) = opened(&f);
    f.forge()
        .edit(number, |m| m.merge_error = Some("the hook said no"));
    f.forge()
        .answer_merges(&[(422, "Branch cannot be merged", "need_rebase")]);
    let err = stderr(&land(&f, &[]).code(2));
    assert!(err.contains("Branch cannot be merged"), "{err}");
    assert!(err.contains("the hook said no"), "{err}");

    // Refused while the status reads as mergeable: tried again, a few times.
    let f = Fork::merge_request();
    let (number, _) = opened(&f);
    f.forge().answer_merges(&[
        (405, "405 Method Not Allowed", "mergeable"),
        (405, "405 Method Not Allowed", "mergeable"),
    ]);
    land(&f, &[]).success();
    assert_eq!(merges(&f).len(), 3);
    assert_eq!(f.forge().mr(number).state, "merged");

    let f = Fork::merge_request();
    let (number, _) = opened(&f);
    f.forge().fail(
        "PUT",
        &format!("/merge_requests/{number}/merge"),
        405,
        "405 Method Not Allowed",
    );
    land(&f, &[]).code(2).stderr(contains("refused to merge"));
    assert_eq!(merges(&f).len(), 6);
}

#[test]
fn land_never_pauses_a_branch_that_did_not_move_or_a_pipeline_without_outcome() {
    let (f, number, head, q) = queued_fork(1);
    f.forge().assign(q[0], BOT);
    f.forge().script(&head, vec![Step::is("failed")]);
    on_main(&f, &[("other.txt", "o\n")], "main");
    land(&f, &[]).code(1);
    assert!(written(&f, number).is_empty());
    assert!(reassigned(&f).is_empty());
    // A head that contains the integration tip lands without a pause.
    let (f, number, _, q) = queued_fork(1);
    f.forge().assign(q[0], BOT);
    land(&f, &[]).success();
    assert!(written(&f, number).is_empty());
    assert!(reassigned(&f).is_empty());
}

#[test]
fn land_waits_for_the_merge_request_the_queue_is_landing() {
    let (f, number, _, q) = queued_fork(2);
    let fake = f.forge();
    let tip = on_main(&f, &[("other.txt", "o\n")], "main");
    // The queue rebased the first onto the integration tip, and its
    // pipeline runs.
    let landing = on_main(&f, &[("a.txt", "a\n")], "feature-0");
    assert!(f.env.is_ancestor(&f.publish, &tip, &landing));
    fake.script(
        &landing,
        vec![
            Step::is("running"),
            Step::is("running"),
            Step::is("success").then(Action::Merge(q[0])),
        ],
    );
    fake.assign(q[0], BOT);
    fake.assign(q[1], BOT);
    let out = stdout(&land(&f, &[]).success());
    assert!(
        out.contains(&format!("paused the merge queue of @merge-bot: !{}", q[1])),
        "{out}"
    );
    assert!(
        out.contains(&format!(
            "the queue is landing !{}; it keeps its assignment",
            q[0]
        )),
        "{out}"
    );
    assert!(
        out.contains(&format!("waiting for the merge queue to land !{}", q[0])),
        "{out}"
    );
    assert_eq!(reassigned(&f), [(q[1], json!([])), (q[1], json!([BOT]))]);
    // The catch-up took in what the queue landed.
    assert_eq!(f.env.rev(&f.publish, "main^2^1^2"), landing);
    assert_eq!(fake.mr(q[0]).state, "merged");
    assert_eq!(fake.mr(number).state, "merged");
    let (_, first) = fake.world().assignments[0].clone();
    let first = parsed_record(first.as_deref().expect("the record stands"));
    assert_eq!(held_in(&first), [q[1]]);
    assert_eq!(first["in_flight"], json!([q[0]]));
    assert!(written(&f, number)[0].contains(&format!("The queue is landing !{}", q[0])));
}

#[test]
fn land_holds_a_merge_request_assigned_during_the_pause() {
    let (f, number, head, q) = queued_fork(3);
    let fake = f.forge();
    fake.assign(q[0], BOT);
    fake.assign(q[1], BOT);
    on_main(&f, &[("other.txt", "o\n")], "main");
    fake.script(&head, vec![Step::is("success")]);
    // A person queues the third while the catch-up merge's pipeline runs.
    fake.script_branch(
        MERGE_REQUEST_BRANCH,
        vec![
            Step::is("running").then(Action::Assign(q[2], BOT)),
            Step::is("running"),
            Step::is("success"),
        ],
    );
    let out = stdout(&land(&f, &[]).success());
    assert!(
        out.contains(&format!(
            "paused !{}, assigned to the merge queue during the pause",
            q[2]
        )),
        "{out}"
    );
    let order = format!("!{}, !{}, !{}", q[0], q[1], q[2]);
    assert!(
        out.contains(&format!("assigned to it again: {order}")),
        "{out}"
    );
    let notes = written(&f, number);
    assert_eq!(notes.len(), 3, "{notes:?}");
    assert!(notes[1].contains(&order), "{}", notes[1]);
    let (taken, record) = fake.world().assignments[2].clone();
    assert_eq!(taken, q[2]);
    assert_eq!(
        held_in(&parsed_record(record.as_deref().unwrap())),
        [q[0], q[1], q[2]]
    );
    for n in &q {
        assert_eq!(fake.mr(*n).assignees, [BOT]);
    }
}

#[test]
fn land_ends_the_pause_when_the_last_pipeline_fails_or_it_fails_itself() {
    let (f, number, head, q) = queued_fork(2);
    let fake = f.forge();
    fake.assign(q[0], BOT);
    fake.assign(q[1], 7);
    fake.assign(q[1], BOT);
    on_main(&f, &[("other.txt", "o\n")], "main");
    // The first pipeline passed; the catch-up merge's fails.
    fake.script(&head, vec![Step::is("success")]);
    fake.script_branch(MERGE_REQUEST_BRANCH, vec![Step::is("failed")]);
    let out = stdout(&land(&f, &[]).code(1));
    assert!(out.contains("failed"), "{out}");
    assert!(out.contains("ended the pause"), "{out}");
    assert_eq!(fake.mr(q[0]).assignees, [BOT]);
    // The second kept its other assignee throughout.
    assert_eq!(fake.mr(q[1]).assignees, [7, BOT]);
    assert_eq!(reassigned(&f)[1], (q[1], json!([7])));
    assert_eq!(fake.mr(number).state, "open");

    // An error of its own ends the pause too.
    let (f, number, _, q) = queued_fork(1);
    let fake = f.forge();
    fake.assign(q[0], BOT);
    on_main(&f, &[("other.txt", "o\n")], "main");
    fake.fail(
        "PUT",
        &format!("/merge_requests/{number}"),
        403,
        "403 Forbidden",
    );
    let err = stderr(&land(&f, &[]).code(2));
    assert!(err.contains("403"), "{err}");
    assert_eq!(fake.mr(q[0]).assignees, [BOT]);
    assert!(
        written(&f, number)
            .last()
            .unwrap()
            .contains("ended the pause")
    );
}

/// The record the publish remote holds, as JSON.
fn recorded(f: &Fork) -> Option<serde_json::Value> {
    f.env.has_rev(&f.publish, PAUSE_REF).then(|| {
        parsed_record(
            &f.env
                .git(&f.publish, &["log", "-1", "--format=%B", PAUSE_REF]),
        )
    })
}

fn parsed_record(message: &str) -> serde_json::Value {
    let (_, body) = message.split_once("\n\n").expect("a record");
    serde_json::from_str(body).expect("a record")
}

/// The merge requests a record holds, in their order.
fn held_in(record: &serde_json::Value) -> Vec<u64> {
    record["held"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["number"].as_u64().unwrap())
        .collect()
}

#[test]
fn land_resume_ends_a_pause_a_dead_run_left() {
    let (f, number, _, q) = queued_fork(4);
    let fake = f.forge();
    for n in &q {
        fake.assign(*n, BOT);
    }
    dead_pause(&f, number, &[q[2], q[0], q[1], q[3]]);
    for n in &q {
        fake.world()
            .mrs
            .iter_mut()
            .find(|m| m.number == *n)
            .unwrap()
            .assignees
            .clear();
    }
    // Since then the second was merged, and the fourth assigned to a person.
    common::merge_on_fake(&f.env, fake, q[1], How::MergeCommit);
    fake.assign(q[3], 7);
    let status = f.env.tool_json(&f.down, &["status", "--json"], 0);
    assert_eq!(status["pause"]["number"], number);
    assert_eq!(status["pause"]["account"], "merge-bot");
    assert_eq!(
        status["pause"]["merge_requests"],
        json!([q[2], q[0], q[1], q[3]])
    );
    let text = stdout(&f.tool(&["status"]).success());
    assert!(
        text.contains(&format!(
            "a pause of the merge queue of @merge-bot is recorded on 'origin', started for the merge request !{number}"
        )),
        "{text}"
    );
    assert!(
        text.contains(&format!(
            "which holds !{}, !{}, !{}, !{}",
            q[2], q[0], q[1], q[3]
        )),
        "{text}"
    );

    // Another clone ends it.
    let other = f.env.path("other");
    f.env.git(
        &f.env.path(""),
        &[
            "clone",
            "-q",
            &f.publish.to_string_lossy(),
            &other.to_string_lossy(),
        ],
    );
    f.env
        .git(&other, &["remote", "set-url", "origin", &fake.remote_url()]);
    fake.clear_requests();
    let out = stdout(&f.env.tool(&other, &["land", "--resume"]).success());
    assert!(
        out.contains(&format!("assigned to it again: !{}, !{}", q[2], q[0])),
        "{out}"
    );
    assert!(
        out.contains(&format!(
            "left as they are, merged, closed, deleted, assigned again or given other assignees meanwhile: !{}, !{}",
            q[1], q[3]
        )),
        "{out}"
    );
    assert_eq!(reassigned(&f), [(q[2], json!([BOT])), (q[0], json!([BOT]))]);
    assert_eq!(fake.mr(q[3]).assignees, [7]);
    assert!(merges(&f).is_empty());
    let status = f.env.tool_json(&f.down, &["status", "--json"], 0);
    assert!(status["pause"].is_null());
    assert!(recorded(&f).is_none());

    // Nothing left to end.
    fake.clear_requests();
    f.tool(&["land", "--resume"])
        .success()
        .stdout(contains("no pause of the merge queue is recorded"));
    assert!(fake.changes().is_empty());
}

#[test]
fn land_resume_finds_a_pause_on_a_merged_merge_request_and_land_carries_one_on() {
    let (f, number, _, q) = queued_fork(1);
    let fake = f.forge();
    dead_pause(&f, number, &[q[0]]);
    common::merge_on_fake(&f.env, fake, number, How::FastForward);
    f.tool(&["land", "--resume"])
        .success()
        .stdout(contains(format!("assigned to it again: !{}", q[0])));
    assert_eq!(fake.mr(q[0]).assignees, [BOT]);

    // A plain `land` carries a recorded pause on, and ends it.
    let (f, number, _, q) = queued_fork(2);
    let fake = f.forge();
    dead_pause(&f, number, &[q[0]]);
    fake.assign(q[1], BOT);
    let out = stdout(&land(&f, &[]).success());
    assert!(
        out.contains(&format!(
            "carrying on the pause of the merge queue started for the merge request !{number}"
        )),
        "{out}"
    );
    assert!(
        out.contains(&format!(
            "paused !{}, assigned to the merge queue during the pause",
            q[1]
        )),
        "{out}"
    );
    assert!(
        out.contains(&format!("assigned to it again: !{}, !{}", q[0], q[1])),
        "{out}"
    );
    assert_eq!(fake.mr(number).state, "merged");
}

#[test]
fn the_queue_needs_an_account_the_forge_knows_and_no_github() {
    let (f, _, _, _) = queued_fork(0);
    let doctor = f.env.tool_json(&f.down, &["doctor", "--json"], 0);
    let text = doctor.to_string();
    assert!(
        text.contains("the merge queue's account @merge-bot exists"),
        "{text}"
    );
    f.reconfigure("queue = \"merge-bot\"", "queue = \"nobody\"");
    let doctor = stdout(&f.tool(&["doctor"]).code(1));
    assert!(
        doctor.contains("publish queue: GitLab knows no account 'nobody'"),
        "{doctor}"
    );
    land(&f, &[])
        .code(2)
        .stderr(contains("knows no account 'nobody'"));

    let f = Fork::github();
    f.reconfigure(
        "via = \"merge-request\"\n",
        "via = \"merge-request\"\nqueue = \"merge-bot\"\n",
    );
    f.env
        .push_upstream(&f.up, &[("n.txt", "n\n")], "upstream: n");
    for args in [&["start"][..], &["land"]] {
        f.tool(args)
            .code(2)
            .stderr(contains("publish queue: 'merge-bot' names a merge queue"));
    }
}

#[test]
fn land_ends_the_pause_on_a_conflict_and_on_its_timeout() {
    // A pause carried on meets a conflict the pause did not cause.
    let (f, number, _, q) = queued_fork(1);
    let fake = f.forge();
    dead_pause(&f, number, &[q[0]]);
    on_main(&f, &[("n.txt", "theirs\n")], "main");
    let out = stdout(&land(&f, &[]).code(1));
    assert!(out.contains("land stopped on the catch-up merge"), "{out}");
    assert!(out.contains("ended the pause"), "{out}");
    assert_eq!(fake.mr(q[0]).assignees, [BOT]);
    assert!(recorded(&f).is_none());

    let (f, number, _, q) = queued_fork(1);
    let fake = f.forge();
    fake.assign(q[0], BOT);
    on_main(&f, &[("other.txt", "o\n")], "main");
    fake.script_branch(MERGE_REQUEST_BRANCH, vec![Step::is("running")]);
    let head = on_remote(&f, MERGE_REQUEST_BRANCH).unwrap();
    fake.script(&head, vec![Step::is("success")]);
    let err = stderr(&land(&f, &["--timeout", "0"]).code(2));
    assert!(
        err.contains("gave up after 0s waiting for the pipeline of the head"),
        "{err}"
    );
    assert_eq!(fake.mr(q[0]).assignees, [BOT]);
    assert!(
        written(&f, number)
            .last()
            .unwrap()
            .contains("ended the pause")
    );
    assert_eq!(fake.mr(number).state, "open");
}

#[test]
fn a_pause_is_found_whichever_merge_request_it_was_started_for() {
    let (f, number, _, q) = queued_fork(1);
    let fake = f.forge();
    dead_pause(&f, number, &[q[0]]);
    // A person merges the merge request it was started for, and a scheduled
    // run opens the next one.
    merge_on_fake(&f.env, fake, number, How::FastForward);
    f.env
        .push_upstream(&f.up, &[("m.txt", "m\n")], "upstream: m");
    let warned = stderr(&f.tool(&["start"]).success());
    assert!(
        warned.contains(&format!(
            "warning: a pause of the merge queue of @merge-bot is recorded on 'origin', started for the merge request \
             !{number}"
        )),
        "{warned}"
    );
    let warned = stderr(&f.tool(&["finish"]).success());
    assert!(!warned.contains("is recorded"), "{warned}");
    let newest = fake
        .open_mrs()
        .into_iter()
        .find(|m| m.source == MERGE_REQUEST_BRANCH)
        .unwrap()
        .number;
    assert_ne!(newest, number);
    let warned = stderr(&f.tool(&["sync"]).success());
    assert!(warned.contains("warning: a pause"), "{warned}");

    let status = f.env.tool_json(&f.down, &["status", "--json"], 0);
    assert_eq!(status["pause"]["number"], number);
    assert_eq!(status["pause"]["merge_requests"], json!([q[0]]));
    assert_eq!(status["merge_request"]["number"], newest);
    let doctor = f.env.tool_json(&f.down, &["doctor", "--json"], 0);
    let check = doctor["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["check"] == "merge_queue_pause")
        .unwrap()
        .clone();
    assert_eq!(check["status"], "warn");
    assert!(check["detail"].as_str().unwrap().contains("which holds"));

    // A plain `land` carries it on and ends it, on the merge request now open.
    let out = stdout(&land(&f, &[]).success());
    assert!(
        out.contains(&format!(
            "carrying on the pause of the merge queue started for the merge request !{number}"
        )),
        "{out}"
    );
    assert_eq!(fake.mr(newest).state, "merged");
    assert_eq!(fake.mr(q[0]).assignees, [BOT]);
    assert!(recorded(&f).is_none());

    // `--resume` finds one started for a merge request long merged, and one
    // whose update has no open merge request any more.
    dead_pause(&f, number, &[q[0]]);
    fake.world().assignments.clear();
    fake.world()
        .mrs
        .iter_mut()
        .find(|m| m.number == q[0])
        .unwrap()
        .assignees
        .clear();
    let out = stdout(&f.tool(&["land", "--resume"]).success());
    assert!(
        out.contains(&format!("assigned to it again: !{}", q[0])),
        "{out}"
    );
    assert!(recorded(&f).is_none());
    dead_pause(&f, number, &[]);
    let out = stdout(&land(&f, &[]).success());
    assert!(
        out.contains("a pause of the merge queue is still on; ending it"),
        "{out}"
    );
    assert!(recorded(&f).is_none());
}

#[test]
fn a_note_decides_nothing_and_one_that_fails_stops_nothing() {
    // A note written to look like a record of the earlier format holds no
    // merge request, and one that claims the end hides no pause.
    let (f, number, _, q) = queued_fork(1);
    let fake = f.forge();
    fake.add_note(
        number,
        &format!(
            "<!-- git-intake pause\naccount {BOT} merge-bot\n{}\n-->\n",
            q[0]
        ),
    );
    f.tool(&["land", "--resume"])
        .success()
        .stdout(contains("no pause of the merge queue is recorded"));
    assert!(fake.mr(q[0]).assignees.is_empty());
    dead_pause(&f, number, &[q[0]]);
    fake.add_note(number, "as the docs say: <!-- git-intake pause ended -->");
    let status = f.env.tool_json(&f.down, &["status", "--json"], 0);
    assert_eq!(status["pause"]["number"], number);
    f.tool(&["land", "--resume"])
        .success()
        .stdout(contains(format!("assigned to it again: !{}", q[0])));

    // A forge that takes no note changes nothing but the warning.
    let (f, number, _, q) = queued_fork(1);
    let fake = f.forge();
    fake.assign(q[0], BOT);
    on_main(&f, &[("other.txt", "o\n")], "main");
    fake.fail("POST", "/notes", 403, "403 Forbidden");
    let out = land(&f, &[]).success();
    let err = stderr(&out);
    assert!(
        err.contains("warning: the note on the merge request"),
        "{err}"
    );
    assert_eq!(fake.mr(number).state, "merged");
    assert_eq!(fake.mr(q[0]).assignees, [BOT]);
    assert!(recorded(&f).is_none());
}

#[test]
fn status_keeps_the_forge_view_and_asks_the_forge_nothing_of_a_pause() {
    let f = Fork::merge_request();
    let (number, _) = opened(&f);
    f.forge().fail("GET", "/notes", 403, "403 Forbidden");
    f.forge().clear_requests();
    let status = f.env.tool_json(&f.down, &["status", "--json"], 0);
    assert_eq!(status["merge_request"]["number"], number);
    assert!(status["forge_error"].is_null());
    assert!(status["pause"].is_null());
    assert!(
        !f.forge()
            .requests()
            .iter()
            .any(|r| r.path.contains("/notes")),
    );

    // Read with git alone, it shows without a token, and with the forge down.
    let (f, number, _, q) = queued_fork(1);
    dead_pause(&f, number, &[q[0]]);
    let out = f
        .env
        .tool_without(&f.down, &["status", "--json"], &["GITLAB_TOKEN"])
        .success();
    let doc: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(doc["pause"]["merge_requests"], json!([q[0]]));
    assert!(!doc["forge_error"].is_null());

    // A record that cannot be read is said to be one, and keeps the rest.
    let f = Fork::merge_request();
    let (number, _) = opened(&f);
    let main = f.env.rev(&f.publish, "main");
    f.env.git(&f.publish, &["update-ref", PAUSE_REF, &main]);
    let status = f.env.tool_json(&f.down, &["status", "--json"], 0);
    assert_eq!(status["merge_request"]["number"], number);
    assert!(status["pause"].is_null());
    assert!(
        status["pause_error"].as_str().unwrap().contains(PAUSE_REF),
        "{status}"
    );
    land(&f, &["--resume"]).code(2).stderr(contains(PAUSE_REF));
    assert_eq!(f.env.rev(&f.publish, PAUSE_REF), main);
    let doctor = stdout(&f.tool(&["doctor"]).code(1));
    assert!(doctor.contains(PAUSE_REF), "{doctor}");
}

#[test]
fn a_carried_pause_leaves_the_merge_request_in_flight_assigned() {
    let (f, number, head, q) = queued_fork(2);
    let fake = f.forge();
    on_main(&f, &[("other.txt", "o\n")], "main");
    let landing = on_main(&f, &[("a.txt", "a\n")], "feature-0");
    fake.script(&landing, vec![Step::is("running")]);
    fake.assign(q[0], BOT);
    // The dead run held the second, and left the first in flight.
    dead_pause_in_flight(&f, number, &[q[1]], &[q[0]]);
    fake.script(&head, vec![Step::is("failed")]);
    land(&f, &[]).code(1);
    assert_eq!(fake.mr(q[0]).assignees, [BOT]);
    assert!(!reassigned(&f).iter().any(|(n, _)| *n == q[0]));
}

#[test]
fn a_run_that_loses_the_record_touches_no_assignment_and_stops() {
    // Another run paused between this run's first read and its own pause.
    let (f, number, head, q) = queued_fork(1);
    let fake = f.forge();
    fake.assign(q[0], BOT);
    on_main(&f, &[("other.txt", "o\n")], "main");
    let theirs = record(&f, number, &[], &[]);
    fake.script(
        &head,
        vec![Step::is("success").then(Action::SetRef(PAUSE_REF.into(), theirs.clone()))],
    );
    let err = stderr(&land(&f, &[]).code(2));
    assert!(err.contains("another `git intake land`"), "{err}");
    assert!(reassigned(&f).is_empty());
    assert_eq!(f.env.rev(&f.publish, PAUSE_REF), theirs);

    // Another run replaced the record during this run's pause.
    let (f, number, head, q) = queued_fork(1);
    let fake = f.forge();
    fake.assign(q[0], BOT);
    on_main(&f, &[("other.txt", "o\n")], "main");
    let theirs = record(&f, number, &[q[0]], &[]);
    fake.script(&head, vec![Step::is("success")]);
    fake.script_branch(
        MERGE_REQUEST_BRANCH,
        vec![
            Step::is("running").then(Action::SetRef(PAUSE_REF.into(), theirs.clone())),
            Step::is("running"),
        ],
    );
    let err = stderr(&land(&f, &[]).code(2));
    assert!(err.contains("another `git intake land`"), "{err}");
    assert!(!err.contains("gave up"), "{err}");
    assert_eq!(reassigned(&f), [(q[0], json!([]))]);
    assert_eq!(f.env.rev(&f.publish, PAUSE_REF), theirs);

    // A `--resume` ended this run's pause meanwhile: what was assigned since
    // stays assigned, and the update still lands, with no new pause when the
    // branch moves once more.
    let (f, number, head, q) = queued_fork(2);
    let fake = f.forge();
    fake.assign(q[0], BOT);
    on_main(&f, &[("other.txt", "o\n")], "main");
    let later = on_main(&f, &[("later.txt", "l\n")], "side");
    fake.script(&head, vec![Step::is("success")]);
    fake.script_branch(
        MERGE_REQUEST_BRANCH,
        vec![
            Step::is("running")
                .then(Action::DeleteRef(PAUSE_REF.into()))
                .then(Action::Assign(q[0], BOT))
                .then(Action::Assign(q[1], BOT)),
            Step::is("running"),
            Step::is("success").then(Action::SetRef("refs/heads/main".into(), later.clone())),
        ],
    );
    let out = stdout(&land(&f, &[]).success());
    assert!(
        out.contains("the pause of the merge queue was ended elsewhere"),
        "{out}"
    );
    assert_eq!(fake.mr(number).state, "merged");
    assert!(f.env.is_ancestor(&f.publish, &later, "main"));
    assert_eq!(fake.mr(q[0]).assignees, [BOT]);
    assert_eq!(fake.mr(q[1]).assignees, [BOT]);
    assert_eq!(reassigned(&f), [(q[0], json!([]))]);
    assert!(recorded(&f).is_none());

    // A remote that refuses the push pauses nothing.
    let (f, number, _, q) = queued_fork(1);
    let fake = f.forge();
    fake.assign(q[0], BOT);
    on_main(&f, &[("other.txt", "o\n")], "main");
    let hook = f.publish.join("hooks/update");
    fs::write(
        &hook,
        format!("#!/bin/sh\ncase \"$1\" in {PAUSE_REF}) exit 1;; esac\n"),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
    land(&f, &[])
        .code(2)
        .stderr(contains(format!("cannot push {PAUSE_REF}")));
    assert!(reassigned(&f).is_empty());
    assert_eq!(fake.mr(number).state, "open");

    // A record replaced just before the end, once the last round read it:
    // nothing goes back.
    let (f, number, head, q) = queued_fork(2);
    let fake = f.forge();
    fake.assign(q[0], BOT);
    fake.assign(q[1], BOT);
    on_main(&f, &[("other.txt", "o\n")], "main");
    fake.script(&head, vec![Step::is("success")]);
    let theirs = record(&f, number, &[q[1]], &[]);
    let replaced = Action::SetRef(PAUSE_REF.into(), theirs.clone());
    fake.script_branch(
        MERGE_REQUEST_BRANCH,
        vec![
            Step::is("running"),
            // The list of the queue comes after that round's read of the record.
            Step::is("failed").after(Action::OnRequest(
                "GET".into(),
                "/merge_requests".into(),
                vec![replaced],
            )),
        ],
    );
    land(&f, &[])
        .code(2)
        .stderr(contains("changed under this run"));
    assert!(fake.mr(q[0]).assignees.is_empty());
    assert!(fake.mr(q[1]).assignees.is_empty());
    assert_eq!(f.env.rev(&f.publish, PAUSE_REF), theirs);

    // A record pushed where no run reads it, as through a `pushurl` of
    // another repository, is taken back before any assignment.
    let (f, _, head, q) = queued_fork(2);
    let fake = f.forge();
    fake.assign(q[0], BOT);
    fake.assign(q[1], BOT);
    on_main(&f, &[("other.txt", "o\n")], "main");
    fake.script(&head, vec![Step::is("success")]);
    let side = push_side(&f);
    land(&f, &[])
        .code(2)
        .stderr(contains("does not read back"))
        .stderr(contains("pushurl"));
    assert!(reassigned(&f).is_empty());
    assert!(recorded(&f).is_none());
    assert!(!f.env.has_rev(&side, PAUSE_REF));
}

/// Installs the hook `name` of the repository at `git_dir`.
fn repository_hook(git_dir: &Path, name: &str, body: &str) {
    use std::os::unix::fs::PermissionsExt;
    let hooks = git_dir.join("hooks");
    fs::create_dir_all(&hooks).unwrap();
    let path = hooks.join(name);
    fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
fn a_record_read_back_tells_a_failed_read_from_one_pushed_elsewhere() {
    // Pushed elsewhere, where its removal is refused: the push says so,
    // whatever the fetch URL shows.
    let (f, _, head, q) = queued_fork(1);
    let fake = f.forge();
    fake.assign(q[0], BOT);
    on_main(&f, &[("other.txt", "o\n")], "main");
    fake.script(&head, vec![Step::is("success")]);
    let side = push_side(&f);
    repository_hook(
        &side,
        "update",
        &format!(
            "case \"$1:$3\" in {PAUSE_REF}:0000000000000000000000000000000000000000) exit 1;; esac"
        ),
    );
    land(&f, &[])
        .code(2)
        .stderr(contains("pushurl"))
        .stderr(contains("could not be removed from where it went"));
    assert!(f.env.has_rev(&side, PAUSE_REF));
    assert!(recorded(&f).is_none());
    assert!(reassigned(&f).is_empty());

    // Recorded where it is read, then git fails: the record stays for
    // `--resume`, and the remote is not blamed.
    let (f, _, head, q) = queued_fork(1);
    let fake = f.forge();
    fake.assign(q[0], BOT);
    on_main(&f, &[("other.txt", "o\n")], "main");
    fake.script(&head, vec![Step::is("success")]);
    let down = f.env.path("ssh-down");
    repository_hook(
        &f.publish,
        "post-receive",
        &format!(
            "if grep -q '{PAUSE_REF}'; then touch '{}'; fi",
            down.display()
        ),
    );
    let err = stderr(&land(&f, &[]).code(2));
    assert!(err.contains("cannot read the record back"), "{err}");
    assert!(err.contains("land --resume"), "{err}");
    assert!(!err.contains("pushurl"), "{err}");
    assert!(recorded(&f).is_some());
    assert!(reassigned(&f).is_empty());
    fs::remove_file(&down).unwrap();
    fs::remove_file(f.publish.join("hooks/post-receive")).unwrap();
    f.tool(&["land", "--resume", "--poll", "0"]).success();
    assert!(recorded(&f).is_none());
    assert_eq!(fake.mr(q[0]).assignees, [BOT]);

    // Pushed elsewhere, and git fails after the first read: that read's
    // answer decides, since a failed read tells nothing.
    let (f, _, head, q) = queued_fork(1);
    let fake = f.forge();
    fake.assign(q[0], BOT);
    on_main(&f, &[("other.txt", "o\n")], "main");
    fake.script(&head, vec![Step::is("success")]);
    let side = push_side(&f);
    let armed = f.env.path("armed");
    repository_hook(
        &side,
        "post-receive",
        &format!(
            "if grep -q '{PAUSE_REF}'; then touch '{}'; fi",
            armed.display()
        ),
    );
    // The first connection once the record is pushed goes through, and git
    // fails after it.
    let wrapper = f.env.path("ssh-then-down");
    fs::write(
        &wrapper,
        format!(
            "#!/bin/sh\nif [ -e '{armed}' ]; then rm '{armed}'; '{shim}' \"$@\"; s=$?; touch '{down}'; exit $s; fi\n\
             exec '{shim}' \"$@\"\n",
            armed = armed.display(),
            shim = f.env.path("ssh-shim").display(),
            down = f.env.path("ssh-down").display()
        ),
    )
    .unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755)).unwrap();
    }
    f.env.git(
        &f.down,
        &["config", "core.sshCommand", &wrapper.to_string_lossy()],
    );
    let err = stderr(&land(&f, &[]).code(2));
    assert!(f.env.path("ssh-down").exists());
    assert!(err.contains("pushurl"), "{err}");
    assert!(!err.contains("cannot read the record back"), "{err}");
    assert!(!f.env.has_rev(&side, PAUSE_REF));
    assert!(reassigned(&f).is_empty());
}

/// Puts the fork in a GitLab CI job whose push URL carries the job token,
/// at an address that refuses every connection.
fn in_a_job_pushing_with_its_token(f: &Fork) {
    f.env.set_var("CI_JOB_TOKEN", "job-token-1");
    f.env.git(
        &f.down,
        &[
            "remote",
            "set-url",
            "--add",
            "--push",
            "origin",
            "https://gitlab-ci-token:job-token-1@127.0.0.1:1/team/app.git",
        ],
    );
}

#[test]
fn land_refuses_a_push_with_the_ci_job_token() {
    let (f, number, head, q) = queued_fork(1);
    let fake = f.forge();
    fake.assign(q[0], BOT);
    fake.script(&head, vec![Step::is("success")]);
    in_a_job_pushing_with_its_token(&f);
    fake.clear_requests();
    let err = stderr(&land(&f, &[]).code(2));
    assert!(
        err.contains("would push to 'origin' with its job token"),
        "{err}"
    );
    assert!(err.contains("the push URL carries it"), "{err}");
    assert!(!err.contains("job-token-1"), "{err}");
    assert!(
        !err.contains("A pause of the merge queue is recorded"),
        "{err}"
    );
    assert!(fake.requests().is_empty());
    assert!(recorded(&f).is_none());
    assert_eq!(fake.mr(q[0]).assignees, [BOT]);
    assert_eq!(fake.mr(number).state, "open");

    // A pause recorded and no open merge request: the refusal comes before
    // the merge requests would go back.
    let (f, number, _, q) = queued_fork(1);
    let fake = f.forge();
    dead_pause(&f, number, &[q[0]]);
    fake.end(number, "closed");
    in_a_job_pushing_with_its_token(&f);
    fake.clear_requests();
    let err = stderr(&land(&f, &[]).code(2));
    assert!(err.contains("with its job token"), "{err}");
    assert!(
        err.contains("A pause of the merge queue is recorded; `git intake land --resume`"),
        "{err}"
    );
    assert!(fake.requests().is_empty());
    assert!(recorded(&f).is_some());
    assert!(fake.mr(q[0]).assignees.is_empty());
}

#[test]
fn land_resume_warns_of_the_ci_job_token_and_goes_on() {
    // Ending a pause needs no pipeline: the merge requests go back, and the
    // record, whose removal git cannot push, stays.
    let (f, number, _, q) = queued_fork(1);
    let fake = f.forge();
    dead_pause(&f, number, &[q[0]]);
    in_a_job_pushing_with_its_token(&f);
    let err = stderr(&f.tool(&["land", "--resume", "--poll", "0"]).code(2));
    assert!(
        err.contains("warning: this GitLab CI job would push to 'origin' with its job token"),
        "{err}"
    );
    assert!(
        err.contains(&format!("assigned to the merge queue again: !{}", q[0])),
        "{err}"
    );
    assert!(
        err.contains("the record of the pause stays on 'origin'"),
        "{err}"
    );
    assert!(err.contains("run where git can push to 'origin'"), "{err}");
    assert!(err.contains(&format!("cannot push {PAUSE_REF}")), "{err}");
    assert!(!err.contains("job-token-1"), "{err}");
    assert_eq!(fake.mr(q[0]).assignees, [BOT]);
    assert!(recorded(&f).is_some());
}

#[test]
fn a_pause_keeps_the_account_it_was_started_with() {
    let (f, number, head, q) = queued_fork(1);
    let fake = f.forge();
    fake.add_user(43, "new-bot");
    f.reconfigure("queue = \"merge-bot\"", "queue = \"new-bot\"");
    fake.assign(q[0], 7);
    dead_pause(&f, number, &[]);
    fake.script(&head, vec![Step::is("running")]);
    fake.script_branch(MERGE_REQUEST_BRANCH, vec![Step::is("running")]);
    // Assigned to the old account during the pause, with a person beside it.
    fake.world()
        .pipelines
        .get_mut(&head)
        .unwrap()
        .front_mut()
        .unwrap()
        .then
        .push(Action::Assign(q[0], BOT));
    land(&f, &["--timeout", "0"]).code(2);
    assert_eq!(reassigned(&f)[0], (q[0], json!([7])));
    assert_eq!(fake.mr(q[0]).assignees, [7, BOT]);
}

#[test]
fn land_resume_works_with_an_update_in_progress() {
    let (f, number, _, q) = queued_fork(1);
    let fake = f.forge();
    on_main(&f, &[("n.txt", "theirs\n")], "main");
    land(&f, &[]).code(1);
    let status = f.env.tool_json(&f.down, &["status", "--json"], 0);
    assert_eq!(status["update"]["step"], "catch_up_conflict");
    dead_pause(&f, number, &[q[0]]);
    // A plain `land` still refuses, and names the way to end the pause.
    land(&f, &[])
        .code(2)
        .stderr(contains("an update is in progress"))
        .stderr(contains("`git intake land --resume` ends it now"));
    f.tool(&["land", "--resume"])
        .success()
        .stdout(contains(format!("assigned to it again: !{}", q[0])));
    assert_eq!(fake.mr(q[0]).assignees, [BOT]);
    assert!(recorded(&f).is_none());
    // The update stays where it was.
    let status = f.env.tool_json(&f.down, &["status", "--json"], 0);
    assert_eq!(status["update"]["step"], "catch_up_conflict");
}

#[test]
fn land_rides_out_an_outage_of_the_forge() {
    // Failed reads of the merge request do not end the wait.
    let (f, number, head, q) = queued_fork(1);
    let fake = f.forge();
    fake.assign(q[0], BOT);
    on_main(&f, &[("other.txt", "o\n")], "main");
    fake.script(&head, vec![Step::is("success")]);
    let single = format!("/merge_requests/{number}");
    fake.script_branch(
        MERGE_REQUEST_BRANCH,
        vec![
            Step::is("running").then(Action::FailTimes("GET".into(), single.clone(), 502, 4)),
            Step::is("running"),
            Step::is("success"),
        ],
    );
    let out = stdout(&land(&f, &[]).success());
    assert!(out.contains("502"), "{out}");
    assert_eq!(fake.mr(number).state, "merged");
    assert_eq!(fake.mr(q[0]).assignees, [BOT]);
    assert!(recorded(&f).is_none());

    // Failures that are not in a row add up to nothing.
    let (f, number, head, q) = queued_fork(1);
    let fake = f.forge();
    fake.assign(q[0], BOT);
    on_main(&f, &[("other.txt", "o\n")], "main");
    fake.script(&head, vec![Step::is("success")]);
    let single = format!("/merge_requests/{number}");
    let nine = || Action::FailTimes("GET".into(), single.clone(), 502, 9);
    fake.script_branch(
        MERGE_REQUEST_BRANCH,
        vec![
            Step::is("running").then(nine()),
            Step::is("running").then(nine()),
            Step::is("success"),
        ],
    );
    land(&f, &[]).success();
    assert_eq!(fake.mr(number).state, "merged");

    // Too many in a row give up, with the pause ended all the same.
    let (f, number, head, q) = queued_fork(1);
    let fake = f.forge();
    fake.assign(q[0], BOT);
    on_main(&f, &[("other.txt", "o\n")], "main");
    fake.script(&head, vec![Step::is("success")]);
    let single = format!("/merge_requests/{number}");
    fake.script_branch(
        MERGE_REQUEST_BRANCH,
        vec![Step::is("running").then(Action::FailTimes("GET".into(), single.clone(), 503, 1000))],
    );
    let err = stderr(&land(&f, &[]).code(2));
    assert!(
        err.contains("gave up after 10 failed rounds in a row"),
        "{err}"
    );
    assert!(err.contains("503"), "{err}");
    assert_eq!(fake.mr(q[0]).assignees, [BOT]);
    assert!(recorded(&f).is_none());
    assert_eq!(fake.mr(number).state, "open");

    // An outage that outlasts the end of the pause too leaves the record and
    // says how to go on.
    let (f, number, head, q) = queued_fork(1);
    let fake = f.forge();
    fake.assign(q[0], BOT);
    on_main(&f, &[("other.txt", "o\n")], "main");
    fake.script(&head, vec![Step::is("success")]);
    fake.script_branch(
        MERGE_REQUEST_BRANCH,
        vec![
            Step::is("running")
                .then(Action::FailTimes(
                    "GET".into(),
                    format!("/merge_requests/{number}"),
                    503,
                    1000,
                ))
                .then(Action::FailTimes(
                    "GET".into(),
                    format!("/merge_requests/{}", q[0]),
                    503,
                    1000,
                )),
        ],
    );
    let err = stderr(&land(&f, &[]).code(2));
    assert!(err.contains("could not be ended"), "{err}");
    assert!(err.contains("`git intake land --resume`"), "{err}");
    assert!(fake.mr(q[0]).assignees.is_empty());
    assert!(recorded(&f).is_some());
    fake.world().flaky.clear();
    f.tool(&["land", "--resume"]).success();
    assert_eq!(fake.mr(q[0]).assignees, [BOT]);
    assert!(recorded(&f).is_none());

    // A refusal is not an outage: it stops at once.
    let f = Fork::merge_request();
    let (number, _) = opened(&f);
    f.forge().clear_requests();
    f.forge().fail(
        "GET",
        &format!("/merge_requests/{number}"),
        403,
        "403 Forbidden",
    );
    land(&f, &[]).code(2).stderr(contains("403"));
    let reads = f
        .forge()
        .requests()
        .iter()
        .filter(|r| r.method == "GET" && r.path.ends_with(&format!("/merge_requests/{number}")))
        .count();
    assert_eq!(reads, 1);

    // With git refused and the forge answering, the merge requests go back
    // all the same, and the record stays for `--resume`.
    let (f, number, head, q) = queued_fork(2);
    let fake = f.forge();
    fake.assign(q[0], BOT);
    fake.assign(q[1], BOT);
    on_main(&f, &[("other.txt", "o\n")], "main");
    fake.script(&head, vec![Step::is("success")]);
    let down = f.env.path("ssh-down");
    fake.script_branch(
        MERGE_REQUEST_BRANCH,
        vec![Step::is("running").then(Action::Touch(down.clone()))],
    );
    land(&f, &[]).code(2).stderr(contains("land --resume"));
    assert_eq!(fake.mr(q[0]).assignees, [BOT]);
    assert_eq!(fake.mr(q[1]).assignees, [BOT]);
    assert!(recorded(&f).is_some());
    assert_eq!(fake.mr(number).state, "open");
    fs::remove_file(&down).unwrap();
    let out = stdout(&land(&f, &["--resume"]).success());
    assert!(out.contains("assigned again"), "{out}");
    assert!(recorded(&f).is_none());
    assert_eq!(fake.mr(q[0]).assignees, [BOT]);
}

#[test]
fn each_step_of_ending_a_pause_is_tried_again() {
    let (f, number, _, q) = queued_fork(2);
    let fake = f.forge();
    dead_pause(&f, number, &[q[0], q[1]]);
    // A read and an assignment fail for a while, and so does removing the
    // record, which a killed run could still resume.
    fake.fail_times("GET", &format!("/merge_requests/{}", q[0]), 502, 2);
    fake.fail_times("PUT", &format!("/merge_requests/{}", q[1]), 503, 2);
    let hook = f.publish.join("hooks/update");
    let flag = f.env.path("refuse-once");
    fs::write(&flag, "").unwrap();
    fs::write(
        &hook,
        format!(
            "#!/bin/sh\nif [ -e '{0}' ]; then rm '{0}'; exit 1; fi\n",
            flag.display()
        ),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
    let out = stdout(&f.tool(&["land", "--resume", "--poll", "0"]).success());
    assert!(
        out.contains(&format!("assigned to it again: !{}, !{}", q[0], q[1])),
        "{out}"
    );
    assert!(recorded(&f).is_none());
    assert_eq!(fake.mr(q[0]).assignees, [BOT]);
    assert_eq!(fake.mr(q[1]).assignees, [BOT]);

    // An outage that outlasts the attempts leaves the rest to `--resume`:
    // the record stays, and the order goes on from where it stopped.
    let (f, number, _, q) = queued_fork(2);
    let fake = f.forge();
    dead_pause(&f, number, &[q[0], q[1]]);
    fake.fail_times("PUT", &format!("/merge_requests/{}", q[1]), 503, 5);
    f.tool(&["land", "--resume", "--poll", "0"])
        .code(2)
        .stderr(contains("503"));
    assert_eq!(fake.mr(q[0]).assignees, [BOT]);
    assert!(fake.mr(q[1]).assignees.is_empty());
    assert!(recorded(&f).is_some());
    let out = stdout(&f.tool(&["land", "--resume", "--poll", "0"]).success());
    assert!(
        out.contains(&format!("assigned to it again: !{}", q[1])),
        "{out}"
    );
    assert!(recorded(&f).is_none());
    assert_eq!(fake.mr(q[1]).assignees, [BOT]);
}

#[test]
fn a_held_merge_request_deleted_since_is_left_as_it_is() {
    let (f, number, _, q) = queued_fork(2);
    let fake = f.forge();
    dead_pause(&f, number, &[q[0], 999, q[1]]);
    let out = stdout(&f.tool(&["land", "--resume"]).success());
    assert!(
        out.contains(&format!("assigned to it again: !{}, !{}", q[0], q[1])),
        "{out}"
    );
    assert!(out.contains("meanwhile: !999"), "{out}");
    assert_eq!(fake.mr(q[0]).assignees, [BOT]);
    assert_eq!(fake.mr(q[1]).assignees, [BOT]);
    assert!(recorded(&f).is_none());
    assert!(written(&f, number).last().unwrap().contains("!999"));

    // A 404 on the return itself counts the same.
    let (f, number, _, q) = queued_fork(2);
    let fake = f.forge();
    dead_pause(&f, number, &[q[0], q[1]]);
    fake.fail(
        "PUT",
        &format!("/merge_requests/{}", q[0]),
        404,
        "404 Not found",
    );
    let out = stdout(&land(&f, &["--resume"]).success());
    assert!(
        out.contains(&format!("assigned to it again: !{}", q[1])),
        "{out}"
    );
    assert!(out.contains(&format!("meanwhile: !{}", q[0])), "{out}");
    assert!(recorded(&f).is_none());

    // One that answers 404 as the pause begins is passed over, and the
    // others are held.
    let (f, _, head, q) = queued_fork(2);
    let fake = f.forge();
    fake.assign(q[0], BOT);
    fake.assign(q[1], BOT);
    on_main(&f, &[("other.txt", "o\n")], "main");
    fake.script(&head, vec![Step::is("success")]);
    fake.script_branch(MERGE_REQUEST_BRANCH, vec![Step::is("failed")]);
    fake.fail(
        "PUT",
        &format!("/merge_requests/{}", q[0]),
        404,
        "404 Not found",
    );
    land(&f, &[]).code(1);
    let changes = reassigned(&f);
    assert_eq!(changes[..2], [(q[0], json!([])), (q[1], json!([]))]);
    assert_eq!(changes.last(), Some(&(q[1], json!([BOT]))));
    assert!(recorded(&f).is_none());

    // A token that cannot see the project gets 404 for everything, which
    // tells no deleted merge request: the pause stays for a token that can.
    let (f, number, _, q) = queued_fork(2);
    let fake = f.forge();
    dead_pause(&f, number, &[q[0], q[1]]);
    fake.hide();
    land(&f, &["--resume"])
        .code(2)
        .stderr(contains("does not show the project 'team/app'"));
    assert!(recorded(&f).is_some());
    assert!(fake.mr(q[0]).assignees.is_empty());

    // The same for a run whose token loses the project during its pause.
    let (f, _, head, q) = queued_fork(2);
    let fake = f.forge();
    fake.assign(q[0], BOT);
    fake.assign(q[1], BOT);
    on_main(&f, &[("other.txt", "o\n")], "main");
    fake.script(&head, vec![Step::is("success")]);
    fake.script_branch(
        MERGE_REQUEST_BRANCH,
        vec![Step::is("running"), Step::is("failed").after(Action::Hide)],
    );
    land(&f, &[])
        .code(2)
        .stderr(contains("does not show the project 'team/app'"))
        .stderr(contains("land --resume"));
    assert_eq!(held_in(&recorded(&f).unwrap()), [q[0], q[1]]);
    assert!(fake.mr(q[0]).assignees.is_empty());
    assert!(fake.mr(q[1]).assignees.is_empty());
}

#[test]
fn land_tells_a_failed_fetch_from_a_missing_branch() {
    // A failed fetch is an outage, without the advice to withdraw.
    let f = Fork::merge_request();
    let (number, head) = opened(&f);
    let down = f.env.path("ssh-down");
    f.forge().script(
        &head,
        vec![
            Step::is("success").then(Action::Touch(down.clone())),
            Step::is("success").then(Action::Remove(down.clone())),
        ],
    );
    let out = land(&f, &[]).success();
    let shown = stdout(&out);
    assert!(!shown.contains("abort --merge-request"), "{shown}");
    assert_eq!(f.forge().mr(number).state, "merged");

    let f = Fork::merge_request();
    let (number, head) = opened(&f);
    let down = f.env.path("ssh-down");
    f.forge()
        .script(&head, vec![Step::is("success").then(Action::Touch(down))]);
    let out = land(&f, &[]).code(2);
    let (shown, err) = (stdout(&out), stderr(&out));
    assert!(!shown.contains("abort --merge-request"), "{shown}");
    assert!(!err.contains("abort --merge-request"), "{err}");
    assert!(
        err.contains("gave up after 10 failed rounds in a row"),
        "{err}"
    );
    assert_eq!(f.forge().mr(number).state, "open");

    // A merge just after the read removed the branch: it is reported as
    // what it is, with status 0.
    let f = Fork::merge_request();
    let (number, head) = opened(&f);
    f.forge().script(
        &head,
        vec![Step::is("success").after(Action::Merge(number))],
    );
    let out = stdout(&land(&f, &[]).success());
    assert!(out.contains("is merged into 'main'"), "{out}");
    assert!(merges(&f).is_empty());

    // An open merge request without its branch gets the advice.
    let f = Fork::merge_request();
    let (_, head) = opened(&f);
    f.forge().script(
        &head,
        vec![Step::is("success").after(Action::DeleteRef(format!(
            "refs/heads/{MERGE_REQUEST_BRANCH}"
        )))],
    );
    land(&f, &[])
        .code(2)
        .stderr(contains("is not on 'origin'"))
        .stderr(contains("abort --merge-request"));
}

/// A `land` started in a pause of the merge queue's merge requests, waiting
/// for the last pipeline, with SIGHUP ignored as `nohup` starts it when
/// `ignoring_hangup`, and its stdout from there on.
fn land_in_a_pause(
    f: &Fork,
    ignoring_hangup: bool,
) -> (
    std::process::Child,
    std::io::Lines<std::io::BufReader<std::process::ChildStdout>>,
) {
    use std::io::{BufRead, BufReader};
    use std::process::{Command, Stdio};
    // A signal it ignores fails the test instead of hanging it.
    let tool = f
        .env
        .tool_command(&f.down, &["land", "--poll", "0", "--timeout", "120"]);
    let mut cmd = Command::new("sh");
    cmd.arg("-c")
        .arg(if ignoring_hangup {
            "trap '' HUP; exec \"$@\""
        } else {
            "exec \"$@\""
        })
        .arg("sh")
        .arg(tool.get_program())
        .args(tool.get_args());
    if let Some(dir) = tool.get_current_dir() {
        cmd.current_dir(dir);
    }
    for (k, v) in tool.get_envs() {
        match v {
            Some(v) => cmd.env(k, v),
            None => cmd.env_remove(k),
        };
    }
    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    let mut paused = false;
    for line in lines.by_ref() {
        let line = line.unwrap();
        paused |= line.contains("paused the merge queue");
        if paused && line.contains("waiting for the pipeline") {
            break;
        }
    }
    assert!(paused);
    (child, lines)
}

fn kill(child: &std::process::Child, signal: &str) {
    let sent = std::process::Command::new("kill")
        .args([format!("-{signal}"), child.id().to_string()])
        .status()
        .unwrap();
    assert!(sent.success());
}

/// Starts `land` in a pause and sends the running process `signals`.
fn signaled_during_a_pause(f: &Fork, signals: &[&str]) -> std::process::ExitStatus {
    let (mut child, lines) = land_in_a_pause(f, false);
    for signal in signals {
        kill(&child, signal);
    }
    for line in lines {
        line.unwrap();
    }
    child.wait().unwrap()
}

#[test]
fn a_signal_ends_the_pause_and_a_kill_leaves_it_resumable() {
    use std::os::unix::process::ExitStatusExt;
    // A terminal that hangs up may deliver SIGHUP twice.
    for (signals, code) in [
        (&["TERM"][..], 143),
        (&["INT"], 130),
        (&["HUP"], 129),
        (&["HUP", "HUP"], 129),
    ] {
        let (f, number, head, q) = queued_fork(2);
        let fake = f.forge();
        fake.assign(q[0], BOT);
        fake.assign(q[1], BOT);
        on_main(&f, &[("other.txt", "o\n")], "main");
        fake.script(&head, vec![Step::is("success")]);
        fake.script_branch(MERGE_REQUEST_BRANCH, vec![Step::is("running")]);
        let status = signaled_during_a_pause(&f, signals);
        assert_eq!(status.code(), Some(code), "{signals:?}: {status:?}");
        assert_eq!(fake.mr(q[0]).assignees, [BOT], "{signals:?}");
        assert_eq!(fake.mr(q[1]).assignees, [BOT], "{signals:?}");
        assert!(recorded(&f).is_none(), "{signals:?}");
        assert_eq!(fake.mr(number).state, "open");
        assert!(
            written(&f, number)
                .last()
                .unwrap()
                .contains("ended the pause")
        );
    }
    let (f, _, head, q) = queued_fork(2);
    let fake = f.forge();
    fake.assign(q[0], BOT);
    fake.assign(q[1], BOT);
    on_main(&f, &[("other.txt", "o\n")], "main");
    fake.script(&head, vec![Step::is("success")]);
    fake.script_branch(MERGE_REQUEST_BRANCH, vec![Step::is("running")]);
    let status = signaled_during_a_pause(&f, &["KILL"]);
    assert_eq!(status.signal(), Some(9));
    assert!(fake.mr(q[0]).assignees.is_empty());
    assert!(recorded(&f).is_some());
    let doc = f.env.tool_json(&f.down, &["status", "--json"], 0);
    assert_eq!(doc["pause"]["merge_requests"], json!([q[0], q[1]]));
    f.tool(&["land", "--resume"]).success();
    assert_eq!(fake.mr(q[0]).assignees, [BOT]);
    assert_eq!(fake.mr(q[1]).assignees, [BOT]);
    assert!(recorded(&f).is_none());
}

#[test]
fn land_outlives_its_terminal_and_a_nohup_keeps_it_going() {
    // Its output gone, as after the reader of a pipe or the terminal left.
    let (f, number, head, q) = queued_fork(1);
    let fake = f.forge();
    fake.assign(q[0], BOT);
    on_main(&f, &[("other.txt", "o\n")], "main");
    fake.script(&head, vec![Step::is("success")]);
    fake.script_branch(MERGE_REQUEST_BRANCH, vec![Step::is("running")]);
    let (mut child, lines) = land_in_a_pause(&f, false);
    drop(lines);
    drop(child.stderr.take());
    fake.script_branch(MERGE_REQUEST_BRANCH, vec![Step::is("success")]);
    assert_eq!(child.wait().unwrap().code(), Some(0));
    assert_eq!(fake.mr(number).state, "merged");
    assert_eq!(fake.mr(q[0]).assignees, [BOT]);
    assert!(recorded(&f).is_none());

    // Started with SIGHUP ignored, it goes on through a hangup.
    let (f, number, head, q) = queued_fork(1);
    let fake = f.forge();
    fake.assign(q[0], BOT);
    on_main(&f, &[("other.txt", "o\n")], "main");
    fake.script(&head, vec![Step::is("success")]);
    fake.script_branch(MERGE_REQUEST_BRANCH, vec![Step::is("running")]);
    let (mut child, lines) = land_in_a_pause(&f, true);
    kill(&child, "HUP");
    fake.script_branch(MERGE_REQUEST_BRANCH, vec![Step::is("success")]);
    let shown: Vec<String> = lines.map(Result::unwrap).collect();
    assert_eq!(child.wait().unwrap().code(), Some(0), "{shown:?}");
    assert_eq!(fake.mr(number).state, "merged");
    assert_eq!(fake.mr(q[0]).assignees, [BOT]);
}

#[test]
fn a_second_signal_ends_the_process_at_once() {
    let (f, _, head, q) = queued_fork(2);
    let fake = f.forge();
    fake.assign(q[0], BOT);
    fake.assign(q[1], BOT);
    on_main(&f, &[("other.txt", "o\n")], "main");
    fake.script(&head, vec![Step::is("success")]);
    fake.script_branch(MERGE_REQUEST_BRANCH, vec![Step::is("running")]);
    let (mut child, lines) = land_in_a_pause(&f, false);
    // The second comes while the first return is answered.
    fake.on_request(
        "PUT",
        &format!("/merge_requests/{}", q[0]),
        vec![Action::Signal(child.id(), "TERM")],
    );
    kill(&child, "TERM");
    for line in lines {
        line.unwrap();
    }
    assert_eq!(child.wait().unwrap().code(), Some(143));
    assert_eq!(fake.mr(q[0]).assignees, [BOT]);
    assert!(fake.mr(q[1]).assignees.is_empty());
    assert!(recorded(&f).is_some());
    land(&f, &["--resume"]).success();
    assert_eq!(fake.mr(q[1]).assignees, [BOT]);
    assert!(recorded(&f).is_none());
}

#[test]
fn a_signal_that_kills_the_catch_up_merge_is_reported_as_the_signal() {
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::process::CommandExt;
    let (f, number, head, q) = queued_fork(1);
    let fake = f.forge();
    fake.assign(q[0], BOT);
    on_main(&f, &[("other.txt", "o\n")], "main");
    fake.script(&head, vec![Step::is("success")]);
    // The terminal's SIGINT goes to its whole process group, git's merge in
    // the middle of the catch-up included.
    let hooks = f.down.join(".git/hooks");
    fs::create_dir_all(&hooks).unwrap();
    let hook = hooks.join("pre-merge-commit");
    fs::write(&hook, "#!/bin/sh\nkill -INT 0\n").unwrap();
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
    let out = f
        .env
        .tool_command(&f.down, &["land", "--poll", "0", "--timeout", "120"])
        .process_group(0)
        .output()
        .unwrap();
    let shown = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(130), "{shown}");
    assert!(shown.contains("interrupted by signal 2"), "{shown}");
    assert_eq!(fake.mr(q[0]).assignees, [BOT]);
    assert!(recorded(&f).is_none());
    assert_eq!(fake.mr(number).state, "open");
}

/// Makes the publish remote lose its answer to the next push that updates
/// `name`: `post-receive` runs once the reference moved, and kills the
/// `receive-pack` that would answer.
fn lose_the_answer_to_a_push(f: &Fork, name: &str) {
    use std::os::unix::fs::PermissionsExt;
    let once = f.env.path("lose-the-answer");
    fs::write(&once, "").unwrap();
    let hook = f.publish.join("hooks/post-receive");
    fs::write(
        &hook,
        format!(
            "#!/bin/sh\nif grep -q '{name}' && [ -e '{once}' ]; then rm -f '{once}'; kill -9 $PPID; fi\n",
            once = once.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
fn a_push_of_the_record_whose_answer_is_lost_counts_as_made() {
    // Its creation.
    let (f, number, head, q) = queued_fork(1);
    let fake = f.forge();
    fake.assign(q[0], BOT);
    on_main(&f, &[("other.txt", "o\n")], "main");
    fake.script(&head, vec![Step::is("success")]);
    fake.script_branch(
        MERGE_REQUEST_BRANCH,
        vec![Step::is("running"), Step::is("success")],
    );
    lose_the_answer_to_a_push(&f, PAUSE_REF);
    land(&f, &[]).success();
    assert!(!f.env.path("lose-the-answer").exists());
    assert_eq!(fake.mr(number).state, "merged");
    assert_eq!(reassigned(&f), [(q[0], json!([])), (q[0], json!([BOT]))]);
    assert!(recorded(&f).is_none());

    // Its deletion.
    let (f, number, _, q) = queued_fork(1);
    dead_pause(&f, number, &[q[0]]);
    lose_the_answer_to_a_push(&f, PAUSE_REF);
    land(&f, &["--resume"]).success();
    assert!(!f.env.path("lose-the-answer").exists());
    assert_eq!(f.forge().mr(q[0]).assignees, [BOT]);
    assert!(recorded(&f).is_none());
}

#[test]
fn land_goes_on_when_the_branch_moves_during_the_catch_up() {
    let (f, number, head, q) = queued_fork(1);
    let fake = f.forge();
    fake.assign(q[0], BOT);
    let other = on_main(&f, &[("other.txt", "o\n")], "main");
    let later = on_main(&f, &[("later.txt", "l\n")], "side");
    fake.script(
        &head,
        vec![
            Step::is("success"),
            Step::is("success"),
            Step::is("success").then(Action::SetRef("refs/heads/main".into(), later.clone())),
        ],
    );
    let out = stdout(&land(&f, &[]).success());
    assert!(
        out.contains("changed meanwhile; nothing was pushed"),
        "{out}"
    );
    assert!(!out.contains("Run the command again"), "{out}");
    assert!(
        !out.contains("once the catch-up merge is resolved"),
        "{out}"
    );
    assert_eq!(fake.mr(number).state, "merged");
    assert!(f.env.is_ancestor(&f.publish, &later, "main"));
    assert!(f.env.is_ancestor(&f.publish, &other, "main"));
    assert_eq!(fake.mr(q[0]).assignees, [BOT]);
}

#[test]
fn land_holds_the_rest_of_the_queue_once_one_in_flight_landed() {
    let (f, number, _, q) = queued_fork(2);
    let fake = f.forge();
    on_main(&f, &[("other.txt", "o\n")], "main");
    let first = on_main(&f, &[("a.txt", "a\n")], "feature-0");
    let second = on_main(&f, &[("b.txt", "b\n")], "feature-1");
    fake.script(
        &first,
        vec![
            Step::is("running"),
            Step::is("running"),
            Step::is("success").then(Action::Merge(q[0])),
        ],
    );
    fake.script(&second, vec![Step::is("running")]);
    fake.assign(q[0], BOT);
    fake.assign(q[1], BOT);
    let out = stdout(&land(&f, &[]).success());
    assert!(
        out.contains(&format!("the queue is landing !{}, !{}", q[0], q[1])),
        "{out}"
    );
    // The second is not waited on for a pipeline of its own.
    assert_eq!(fake.mr(q[0]).state, "merged");
    assert_eq!(reassigned(&f), [(q[1], json!([])), (q[1], json!([BOT]))]);
    let (_, record) = fake.world().assignments[0].clone();
    let record = parsed_record(record.as_deref().unwrap());
    assert_eq!(held_in(&record), [q[1]]);
    assert_eq!(record["in_flight"], json!([]));
    assert_eq!(fake.mr(number).state, "merged");
    assert!(recorded(&f).is_none());
}

#[test]
fn land_holds_one_still_current_once_another_in_flight_merged() {
    // With fast-forward merges a merge request stacked on the one that merged
    // stays current, and its pipeline would be a second one to wait for.
    let (f, number, _, q) = queued_fork(2);
    let env = &f.env;
    let fake = f.forge();
    fake.set("merge_method", json!("ff"));
    on_main(&f, &[("other.txt", "o\n")], "main");
    let first = on_main(&f, &[("a.txt", "a\n")], "feature-0");
    let dir = common::forge(env, &f.publish);
    env.git(&dir, &["fetch", "-q", "origin"]);
    env.git(&dir, &["checkout", "-q", "--detach", &first]);
    let second = env.commit(&dir, &[("b.txt", "b\n")], "other: stacked");
    env.git(
        &dir,
        &["push", "-q", "origin", "+HEAD:refs/heads/feature-1"],
    );
    fake.script(
        &first,
        vec![
            Step::is("running"),
            Step::is("running"),
            Step::is("success").then(Action::Merge(q[0])),
        ],
    );
    fake.script(&second, vec![Step::is("running")]);
    fake.assign(q[0], BOT);
    fake.assign(q[1], BOT);
    land(&f, &[]).success();
    assert_eq!(fake.mr(q[0]).state, "merged");
    assert_eq!(reassigned(&f), [(q[1], json!([])), (q[1], json!([BOT]))]);
    assert_eq!(fake.mr(number).state, "merged");
    assert!(recorded(&f).is_none());
}

#[test]
fn land_holds_a_passed_merge_request_the_queue_does_not_take() {
    let (f, number, _, q) = queued_fork(1);
    let fake = f.forge();
    on_main(&f, &[("other.txt", "o\n")], "main");
    on_main(&f, &[("a.txt", "a\n")], "feature-0");
    fake.assign(q[0], BOT);
    let out = stdout(&land(&f, &[]).success());
    assert!(
        out.contains(&format!("waiting for the merge queue to land !{}", q[0])),
        "{out}"
    );
    assert_eq!(reassigned(&f), [(q[0], json!([])), (q[0], json!([BOT]))]);
    assert_eq!(fake.mr(number).state, "merged");
}

#[test]
fn land_does_not_pause_for_a_merge_that_cannot_happen() {
    // Blocked for what a pause does not change: nothing is touched.
    for status in ["draft_status", "discussions_not_resolved", "not_approved"] {
        let (f, number, _, q) = queued_fork(1);
        let fake = f.forge();
        fake.assign(q[0], BOT);
        on_main(&f, &[("other.txt", "o\n")], "main");
        fake.edit(number, |m| m.blocked = Some(status));
        let err = stderr(&land(&f, &[]).code(2));
        assert!(err.contains(status), "{err}");
        assert!(reassigned(&f).is_empty(), "{status}");
        assert!(recorded(&f).is_none(), "{status}");
        assert!(written(&f, number).is_empty(), "{status}");
    }
    // The catch-up will conflict: it stops as it would, with the queue left
    // working all along.
    let (f, number, _, q) = queued_fork(1);
    let fake = f.forge();
    fake.assign(q[0], BOT);
    on_main(&f, &[("n.txt", "theirs\n")], "main");
    let out = stdout(&land(&f, &[]).code(1));
    assert!(out.contains("land stopped on the catch-up merge"), "{out}");
    assert!(reassigned(&f).is_empty());
    assert!(recorded(&f).is_none());
    assert!(written(&f, number).is_empty());
}

/// Makes connection `n` to the publish remote fail, from now on, as a busy
/// SSH endpoint fails: with what reads as an authentication error.
fn fail_connection(f: &Fork, n: usize) {
    fs::write(f.env.path("ssh-fail-at"), n.to_string()).unwrap();
}

fn connections(f: &Fork) -> usize {
    fs::read_to_string(f.env.path("ssh-log"))
        .unwrap_or_default()
        .lines()
        .count()
}

/// Runs a scenario once to count its connections to the publish remote, then
/// once more with each one of them failing, which must change nothing it
/// checks. It returns the connections it made.
fn survives_each_failed_connection(scenario: impl Fn(Option<usize>) -> usize + Sync) {
    let made = scenario(None);
    assert!(made > 2, "{made} connections");
    let numbers: Vec<usize> = (1..=made).collect();
    std::thread::scope(|s| {
        for chunk in numbers.chunks(made.div_ceil(4)) {
            let scenario = &scenario;
            s.spawn(move || {
                for n in chunk {
                    scenario(Some(*n));
                }
            });
        }
    });
}

#[test]
fn land_gets_through_any_one_failed_connection() {
    survives_each_failed_connection(|fail_at| {
        let (f, number, head, q) = queued_fork(1);
        let fake = f.forge();
        fake.assign(q[0], BOT);
        on_main(&f, &[("other.txt", "o\n")], "main");
        fake.script(&head, vec![Step::is("success")]);
        fake.script_branch(
            MERGE_REQUEST_BRANCH,
            vec![Step::is("running"), Step::is("success")],
        );
        let before = connections(&f);
        if let Some(n) = fail_at {
            fail_connection(&f, n);
        }
        let out = land(&f, &[]);
        assert_eq!(
            out.get_output().status.code(),
            Some(0),
            "connection {fail_at:?}:\n{}\n{}",
            stdout(&out),
            stderr(&out)
        );
        assert_eq!(fake.mr(number).state, "merged", "connection {fail_at:?}");
        assert_eq!(fake.mr(q[0]).assignees, [BOT], "connection {fail_at:?}");
        assert!(recorded(&f).is_none(), "connection {fail_at:?}");
        f.env.assert_no_update(&f.down);
        connections(&f) - before
    });
}

#[test]
fn land_resume_gets_through_any_one_failed_connection() {
    survives_each_failed_connection(|fail_at| {
        let (f, number, _, q) = queued_fork(1);
        dead_pause(&f, number, &[q[0]]);
        let before = connections(&f);
        if let Some(n) = fail_at {
            fail_connection(&f, n);
        }
        let out = f.tool(&["land", "--resume", "--poll", "0"]);
        assert_eq!(
            out.get_output().status.code(),
            Some(0),
            "connection {fail_at:?}:\n{}\n{}",
            stdout(&out),
            stderr(&out)
        );
        assert_eq!(
            f.forge().mr(q[0]).assignees,
            [BOT],
            "connection {fail_at:?}"
        );
        assert!(recorded(&f).is_none(), "connection {fail_at:?}");
        connections(&f) - before
    });
}

#[test]
fn sync_gets_through_any_one_failed_connection() {
    // The catch-up of an open merge request.
    survives_each_failed_connection(|fail_at| {
        let f = Fork::merge_request();
        let (_, head) = opened(&f);
        let other = on_main(&f, &[("other.txt", "o\n")], "main");
        let before = connections(&f);
        if let Some(n) = fail_at {
            fail_connection(&f, n);
        }
        let out = f.tool(&["sync"]);
        assert_eq!(
            out.get_output().status.code(),
            Some(0),
            "connection {fail_at:?}:\n{}\n{}",
            stdout(&out),
            stderr(&out)
        );
        let caught = on_remote(&f, MERGE_REQUEST_BRANCH).unwrap();
        assert_ne!(caught, head, "connection {fail_at:?}");
        assert!(f.env.is_ancestor(&f.publish, &other, &caught));
        f.env.assert_no_update(&f.down);
        connections(&f) - before
    });
    // A new update.
    survives_each_failed_connection(|fail_at| {
        let f = Fork::merge_request();
        f.env
            .push_upstream(&f.up, &[("n.txt", "n\n")], "upstream: n");
        let before = connections(&f);
        if let Some(n) = fail_at {
            fail_connection(&f, n);
        }
        let out = f.tool(&["sync"]);
        assert_eq!(
            out.get_output().status.code(),
            Some(0),
            "connection {fail_at:?}:\n{}\n{}",
            stdout(&out),
            stderr(&out)
        );
        assert_eq!(open_number(f.forge()), 1, "connection {fail_at:?}");
        assert!(on_remote(&f, MERGE_REQUEST_BRANCH).is_some());
        f.env.assert_no_update(&f.down);
        connections(&f) - before
    });
    // A branch a closed merge request left, deleted on the way.
    survives_each_failed_connection(|fail_at| {
        let f = Fork::merge_request();
        let (number, _) = opened(&f);
        f.forge().end(number, "closed");
        f.env
            .push_upstream(&f.up, &[("m.txt", "m\n")], "upstream: m");
        let before = connections(&f);
        if let Some(n) = fail_at {
            fail_connection(&f, n);
        }
        let out = f.tool(&["sync"]);
        assert_eq!(
            out.get_output().status.code(),
            Some(0),
            "connection {fail_at:?}:\n{}\n{}",
            stdout(&out),
            stderr(&out)
        );
        assert_ne!(open_number(f.forge()), number, "connection {fail_at:?}");
        f.env.assert_no_update(&f.down);
        connections(&f) - before
    });
}

/// A request to the forge: its method and its path without the query.
#[test]
fn land_gets_through_any_one_failed_request() {
    survives_each_failed_request(|failing| {
        let (f, number, head, q) = queued_fork(1);
        let fake = f.forge();
        fake.assign(q[0], BOT);
        on_main(&f, &[("other.txt", "o\n")], "main");
        fake.script(&head, vec![Step::is("success")]);
        fake.script_branch(
            MERGE_REQUEST_BRANCH,
            vec![Step::is("running"), Step::is("success")],
        );
        fake.clear_requests();
        if let Some((method, path)) = failing {
            fake.fail_times(method, path, 502, 1);
        }
        let out = land(&f, &[]);
        assert_eq!(
            out.get_output().status.code(),
            Some(0),
            "{failing:?}:\n{}\n{}",
            stdout(&out),
            stderr(&out)
        );
        assert_eq!(fake.mr(number).state, "merged", "{failing:?}");
        assert_eq!(fake.mr(q[0]).assignees, [BOT], "{failing:?}");
        assert!(recorded(&f).is_none(), "{failing:?}");
        f.env.assert_no_update(&f.down);
        fake.requests()
    });
}

#[test]
fn sync_gets_through_any_one_failed_request() {
    // The catch-up of an open merge request, whose description is then
    // rewritten.
    survives_each_failed_request(|failing| {
        let f = Fork::merge_request();
        let (number, head) = opened(&f);
        on_main(&f, &[("other.txt", "o\n")], "main");
        let fake = f.forge();
        fake.clear_requests();
        if let Some((method, path)) = failing {
            fake.fail_times(method, path, 502, 1);
        }
        let out = f
            .env
            .tool_env(&f.down, &["sync"], &[("GIT_INTAKE_TEST_RETRY_GAP_MS", "0")]);
        assert_eq!(
            out.get_output().status.code(),
            Some(0),
            "{failing:?}:\n{}\n{}",
            stdout(&out),
            stderr(&out)
        );
        assert_ne!(on_remote(&f, MERGE_REQUEST_BRANCH).unwrap(), head);
        let single = format!("/merge_requests/{number}");
        let rewrites = fake
            .requests()
            .iter()
            .filter(|r| r.method == "PUT" && r.path.ends_with(&single))
            .count();
        let failed = failing.is_some_and(|(m, p)| m == "PUT" && p.ends_with(&single));
        assert!(rewrites > usize::from(failed), "{failing:?}");
        f.env.assert_no_update(&f.down);
        fake.requests()
    });
    // A new update.
    survives_each_failed_request(|failing| {
        let f = Fork::merge_request();
        f.env
            .push_upstream(&f.up, &[("n.txt", "n\n")], "upstream: n");
        let fake = f.forge();
        fake.clear_requests();
        if let Some((method, path)) = failing {
            fake.fail_times(method, path, 502, 1);
        }
        let out = f
            .env
            .tool_env(&f.down, &["sync"], &[("GIT_INTAKE_TEST_RETRY_GAP_MS", "0")]);
        assert_eq!(
            out.get_output().status.code(),
            Some(0),
            "{failing:?}:\n{}\n{}",
            stdout(&out),
            stderr(&out)
        );
        assert_eq!(open_number(fake), 1, "{failing:?}");
        f.env.assert_no_update(&f.down);
        fake.requests()
    });
}

#[test]
fn sync_runs_again_after_a_failed_read_just_before_its_push() {
    // As seen on a real project: the catch-up merge is made, and the check
    // just before the push fails once, reading like a refused credential.
    let f = Fork::merge_request();
    let (number, head) = opened(&f);
    on_main(&f, &[("other.txt", "o\n")], "main");
    f.forge().script(
        &head,
        vec![Step::is("success").then(Action::Write(f.env.path("ssh-fail-at"), "1".into()))],
    );
    let shown = stdout(&f.tool(&["sync"]).success());
    assert!(shown.contains("Permission denied (publickey)"), "{shown}");
    assert_eq!(shown.matches("trying again").count(), 1, "{shown}");
    assert_eq!(f.forge().mr(number).state, "open");
    assert_ne!(on_remote(&f, MERGE_REQUEST_BRANCH).unwrap(), head);
    f.env.assert_no_update(&f.down);

    // An outage that lasts gives up, with nothing left of the run.
    let f = Fork::merge_request();
    let (_, head) = opened(&f);
    on_main(&f, &[("other.txt", "o\n")], "main");
    f.forge().script(
        &head,
        vec![Step::is("success").then(Action::Touch(f.env.path("ssh-down")))],
    );
    let out = f.tool(&["sync"]).code(2);
    assert_eq!(stdout(&out).matches("trying again").count(), 2);
    assert!(
        stderr(&out).contains("Network is unreachable"),
        "{}",
        stderr(&out)
    );
    fs::remove_file(f.env.path("ssh-down")).unwrap();
    f.env.assert_no_update(&f.down);
    assert_eq!(on_remote(&f, MERGE_REQUEST_BRANCH), Some(head));
}

/// How many pushes of the record of a pause git's trace `file` shows.
fn record_pushes(file: &Path) -> usize {
    fs::read_to_string(file)
        .unwrap()
        .lines()
        .filter(|l| {
            l.contains("git push") && l.contains(&format!("--force-with-lease={PAUSE_REF}"))
        })
        .count()
}

#[test]
fn a_lost_swap_is_not_tried_again() {
    let trace_of = |f: &Fork| f.env.path("trace").to_string_lossy().into_owned();
    let land_traced = |f: &Fork, args: &[&str]| {
        let trace = trace_of(f);
        f.env
            .tool_env(&f.down, args, &[("GIT_TRACE", trace.as_str())])
    };

    // Creating: another run recorded a pause after this run read none.
    let (f, number, head, q) = queued_fork(1);
    let fake = f.forge();
    fake.assign(q[0], BOT);
    on_main(&f, &[("other.txt", "o\n")], "main");
    let theirs = record(&f, number, &[], &[]);
    // After the checks of the round, just before the pause.
    fake.script(
        &head,
        vec![
            Step::is("success"),
            Step::is("success").then(Action::SetRef(PAUSE_REF.into(), theirs.clone())),
        ],
    );
    let out = land_traced(&f, &["land", "--poll", "0", "--timeout", "120"]).code(2);
    assert!(
        stderr(&out).contains("another `git intake land`"),
        "{}",
        stderr(&out)
    );
    assert_eq!(record_pushes(&f.env.path("trace")), 1);
    assert!(reassigned(&f).is_empty());
    assert_eq!(f.env.rev(&f.publish, PAUSE_REF), theirs);

    // Updating: the record changed between its read and the swap.
    let (f, number, head, q) = queued_fork(1);
    let fake = f.forge();
    on_main(&f, &[("other.txt", "o\n")], "main");
    let theirs = record(&f, number, &[], &[]);
    fake.script(&head, vec![Step::is("success")]);
    fake.script_branch(
        MERGE_REQUEST_BRANCH,
        vec![Step::is("running").then(Action::Assign(q[0], BOT))],
    );
    fake.script_branch(
        "feature-0",
        vec![Step::none().then(Action::SetRef(PAUSE_REF.into(), theirs.clone()))],
    );
    let out = land_traced(&f, &["land", "--poll", "0", "--timeout", "120"]).code(2);
    assert!(
        stderr(&out).contains("another `git intake land`"),
        "{}",
        stderr(&out)
    );
    assert_eq!(record_pushes(&f.env.path("trace")), 2);
    assert!(reassigned(&f).is_empty());
    assert_eq!(fake.mr(q[0]).assignees, [BOT]);
    assert_eq!(f.env.rev(&f.publish, PAUSE_REF), theirs);

    // Deleting: the record changed while the merge requests returned.
    let (f, number, _, q) = queued_fork(1);
    let fake = f.forge();
    dead_pause(&f, number, &[q[0]]);
    let theirs = record(&f, number, &[], &[]);
    fake.script_branch(
        "feature-0",
        vec![Step::none().then(Action::SetRef(PAUSE_REF.into(), theirs.clone()))],
    );
    let out = land_traced(&f, &["land", "--resume", "--poll", "0"]).code(2);
    assert!(
        stderr(&out).contains("another `git intake land`"),
        "{}",
        stderr(&out)
    );
    assert_eq!(record_pushes(&f.env.path("trace")), 1);
    assert_eq!(f.env.rev(&f.publish, PAUSE_REF), theirs);
}
