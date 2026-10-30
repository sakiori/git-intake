//! The forge in `start`, `status` and `doctor`: discovery, tokens, merge
//! settings, and the update's open merge request. The forge is a fake one;
//! see `common::forge`.

mod common;

use common::forge::TOKEN;
use common::{Fork, MERGE_REQUEST_BRANCH, stderr, stdout};
use serde_json::json;

fn behind(f: &Fork) -> String {
    f.env
        .push_upstream(&f.up, &[("n.txt", "n\n")], "upstream: n")
}

/// The doctor checks with the identifier `check`.
fn checks<'a>(doctor: &'a serde_json::Value, check: &str) -> Vec<&'a serde_json::Value> {
    doctor["checks"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["check"] == check)
        .collect()
}

#[test]
fn doctor_checks_the_forge_the_token_and_the_merge_settings() {
    let f = Fork::merge_request();
    let doctor = f.env.tool_json(&f.down, &["doctor", "--json"], 0);
    assert_eq!(checks(&doctor, "forge")[0]["status"], "pass");
    let detail = checks(&doctor, "forge")[0]["detail"].as_str().unwrap();
    assert!(detail.contains("the GitLab project 'team/app'"), "{detail}");
    assert!(detail.contains("GITLAB_TOKEN"), "{detail}");
    assert!(!detail.contains(TOKEN), "{detail}");
    assert_eq!(checks(&doctor, "forge_token")[0]["status"], "pass");
    let settings = checks(&doctor, "merge_settings");
    assert_eq!(settings[0]["status"], "pass");
    // Squashing on request stays selectable: a warning, not a failure.
    assert!(settings.iter().any(|c| {
        c["status"] == "warn"
            && c["detail"]
                .as_str()
                .unwrap()
                .contains("squash_option `default_off`")
    }));

    f.forge()
        .set("only_allow_merge_if_pipeline_succeeds", json!(false));
    let doctor = f.env.tool_json(&f.down, &["doctor", "--json"], 1);
    let failed: Vec<_> = checks(&doctor, "merge_settings")
        .into_iter()
        .filter(|c| c["status"] == "fail")
        .collect();
    assert_eq!(failed.len(), 1);
    assert!(
        failed[0]["detail"]
            .as_str()
            .unwrap()
            .contains("only_allow_merge_if_pipeline_succeeds")
    );
}

#[test]
fn doctor_reports_a_rejected_token_without_showing_it() {
    let f = Fork::github();
    f.forge()
        .fail("GET", "/repos/team/app", 401, "Bad credentials");
    let doctor = f.tool(&["doctor"]).code(1);
    let out = stdout(&doctor);
    assert!(out.contains("401 Bad credentials"), "{out}");
    assert!(
        !out.contains(TOKEN) && !stderr(&doctor).contains(TOKEN),
        "{out}"
    );
}

#[test]
fn doctor_warns_about_selectable_squash_and_rebase_merges_on_github() {
    let f = Fork::github();
    f.forge().set("allow_squash_merge", json!(true));
    f.forge().set("allow_rebase_merge", json!(true));
    let doctor = f.env.tool_json(&f.down, &["doctor", "--json"], 0);
    let warnings: Vec<_> = checks(&doctor, "merge_settings")
        .into_iter()
        .filter(|c| c["status"] == "warn")
        .collect();
    assert_eq!(warnings.len(), 2, "{warnings:?}");
}

#[test]
fn doctor_fails_an_unknown_forge() {
    let f = Fork::merge_request();
    for var in ["GITLAB_CI", "CI_SERVER_URL", "CI_API_V4_URL"] {
        f.env.unset_var(var);
    }
    let doctor = f.env.tool_json(&f.down, &["doctor", "--json"], 1);
    let detail = checks(&doctor, "forge")[0]["detail"].as_str().unwrap();
    assert!(detail.contains("does not recognise"), "{detail}");
}

#[test]
fn status_shows_the_open_merge_request() {
    let f = Fork::merge_request();
    let env = &f.env;
    behind(&f);
    f.tool(&["sync"]).success();
    let head = common::forge::branch_tip(&f.publish, MERGE_REQUEST_BRANCH).unwrap();
    let status = env.tool_json(&f.down, &["status", "--json"], 0);
    assert_eq!(status["merge_request"]["number"], 1);
    assert_eq!(status["merge_request"]["head"], head.as_str());
    assert_eq!(status["merge_request"]["current"], true);
    assert!(
        status["merge_request"]["url"]
            .as_str()
            .unwrap()
            .ends_with("/merge_requests/1")
    );
    assert_eq!(status["merged_without_history"], serde_json::Value::Null);

    // The integration branch moves past it.
    let dir = common::forge(env, &f.publish);
    env.commit(&dir, &[("other.txt", "o\n")], "other: lands");
    env.git(&dir, &["push", "-q", "origin", "HEAD:main"]);
    env.git(&f.down, &["pull", "-q", "--ff-only", "origin", "main"]);
    let out = stdout(&f.tool(&["status"]).success());
    assert!(out.contains("merge request !1 ("), "{out}");
    assert!(
        out.contains(&format!(
            "at {}: behind 'main'; run `git intake sync`",
            &head[..12]
        )),
        "{out}"
    );
    let status = env.tool_json(&f.down, &["status", "--json"], 0);
    assert_eq!(status["merge_request"]["current"], false);
}

#[test]
fn status_says_why_it_cannot_read_the_forge_and_keeps_its_exit_status() {
    let f = Fork::merge_request();
    f.env.unset_var("GITLAB_TOKEN");
    let status = f.env.tool_json(&f.down, &["status", "--json"], 0);
    assert!(
        status["forge_error"]
            .as_str()
            .unwrap()
            .contains("GITLAB_TOKEN")
    );
    assert_eq!(status["merge_request"], serde_json::Value::Null);
    let out = stdout(&f.tool(&["status"]).success());
    assert!(out.contains("cannot read the forge"), "{out}");
}

#[test]
fn start_refuses_without_a_token() {
    let f = Fork::merge_request();
    behind(&f);
    f.env.unset_var("GITLAB_TOKEN");
    let err = stderr(&f.tool(&["start"]).code(2));
    assert!(err.contains("GITLAB_TOKEN"), "{err}");
    f.env.assert_no_update(&f.down);
}

#[test]
fn start_refuses_while_the_merge_request_is_open() {
    let f = Fork::merge_request();
    behind(&f);
    f.tool(&["sync"]).success();
    f.env
        .push_upstream(&f.up, &[("m.txt", "m\n")], "upstream: m");
    let err = stderr(&f.tool(&["start"]).code(2));
    assert!(err.contains("merge request !1"), "{err}");
    assert!(err.contains("`git intake sync`"), "{err}");
    assert!(err.contains("`git intake abort --merge-request`"), "{err}");
    f.env.assert_no_update(&f.down);
}

#[test]
fn start_refuses_unpublished_local_commits_in_merge_request_mode() {
    let f = Fork::merge_request();
    behind(&f);
    f.env
        .commit(&f.down, &[("local.txt", "l\n")], "fork: local");
    let err = stderr(&f.tool(&["start"]).code(2));
    assert!(
        err.contains("'main' holds 1 commit that 'origin/main' lacks"),
        "{err}"
    );
    f.env.assert_no_update(&f.down);
}

#[test]
fn every_command_but_abort_refuses_a_shallow_clone() {
    let f = Fork::new();
    let env = &f.env;
    let dir = env.path("shallow");
    env.git(
        &env.path(""),
        &[
            "clone",
            "-q",
            "--depth",
            "1",
            &format!("file://{}", f.publish.display()),
            &dir.to_string_lossy(),
        ],
    );
    for args in [
        &["status"][..],
        &["doctor"],
        &["start"],
        &["log"],
        &["env"],
        &["verify"],
        &["finish"],
    ] {
        let err = stderr(&env.tool(&dir, args).code(2));
        assert!(err.contains("`git fetch --unshallow`"), "{args:?}: {err}");
    }
    env.tool(&dir, &["abort"]).success();
    // `migrate` is no longer a command.
    env.tool(&dir, &["migrate"])
        .code(2)
        .stderr(predicates::str::contains(
            "unrecognized subcommand 'migrate'",
        ));
}

#[test]
fn requests_go_through_the_proxy_of_the_environment() {
    let f = Fork::merge_request();
    // A proxy that tunnels nothing: the requests reach it, not the forge.
    let proxy = mockito::Server::new();
    f.env.set_var("HTTP_PROXY", &proxy.url());
    let status = f.env.tool_json(&f.down, &["status", "--json"], 0);
    let why = status["forge_error"].as_str().unwrap();
    assert!(why.contains("proxy"), "{why}");
    assert!(f.forge().requests().is_empty());
}

#[test]
fn the_forge_is_that_of_the_url_git_uses() {
    let f = Fork::merge_request();
    let env = &f.env;
    // A shorthand that git's URL rewriting expands to the forge's URL.
    let url = f.forge().remote_url();
    let base = url.strip_suffix("team/app.git").unwrap();
    env.git(
        &f.down,
        &["config", &format!("url.{base}.insteadOf"), "forge:"],
    );
    env.git(
        &f.down,
        &["remote", "set-url", "origin", "forge:team/app.git"],
    );
    behind(&f);
    let out = stdout(&f.tool(&["sync"]).success());
    assert!(out.contains("opened merge request !1"), "{out}");
    assert!(common::forge::branch_tip(&f.publish, MERGE_REQUEST_BRANCH).is_some());

    // A rewrite to a host that is no forge leaves the forge unknown, though
    // the configured URL is on the forge's host.
    let g = Fork::merge_request();
    let elsewhere = g.env.bare("elsewhere");
    g.env.git(
        &g.down,
        &[
            "config",
            &format!("url.{}.insteadOf", elsewhere.display()),
            &g.forge().remote_url(),
        ],
    );
    behind(&g);
    let err = stderr(&g.tool(&["sync"]).code(2));
    assert!(err.contains("names no host"), "{err}");
    assert!(g.forge().requests().is_empty());
}

#[test]
fn status_judges_the_merge_request_against_the_remote_s_integration_branch() {
    let f = Fork::merge_request();
    let env = &f.env;
    behind(&f);
    f.tool(&["sync"]).success();
    // Other work lands on the remote; the local integration branch lags.
    let dir = common::forge(env, &f.publish);
    env.commit(&dir, &[("other.txt", "o\n")], "other: lands");
    env.git(&dir, &["push", "-q", "origin", "HEAD:main"]);
    let status = env.tool_json(&f.down, &["status", "--json"], 0);
    assert_eq!(status["merge_request"]["current"], false);
    // Merged with a merge commit and not pulled: merged with its history.
    f.tool(&["sync"]).success();
    common::merge_on_fake(env, f.forge(), 1, common::How::MergeCommit);
    let status = env.tool_json(&f.down, &["status", "--json"], 0);
    assert_eq!(status["merge_request"], serde_json::Value::Null);
    assert_eq!(status["merged_without_history"], serde_json::Value::Null);
    let out = stdout(&f.tool(&["status"]).success());
    assert!(!out.contains("merged without"), "{out}");
}

/// A merge request from a fork's `git-intake/update` whose description
/// records the targets the next update takes.
fn fork_request(f: &Fork, target: &str) -> u64 {
    let block = format!("<!-- git-intake targets\nupstream {target}\n-->\n");
    f.forge()
        .add_from_fork(MERGE_REQUEST_BRANCH, "main", &block, &"f".repeat(40))
}

#[test]
fn merge_requests_from_forks_are_not_the_update_s() {
    for f in [Fork::merge_request(), Fork::github()] {
        let target = behind(&f);
        let fork = fork_request(&f, &target);
        let status = f.env.tool_json(&f.down, &["status", "--json"], 0);
        assert_eq!(status["merge_request"], serde_json::Value::Null);
        let out = stdout(&f.tool(&["abort", "--merge-request"]).success());
        assert!(out.contains("there is nothing to withdraw"), "{out}");
        assert_eq!(f.forge().mr(fork).state, "open");
        f.tool(&["start"]).success();
        f.tool(&["abort"]).success();
        let out = stdout(&f.tool(&["sync"]).success());
        assert!(
            out.contains(&format!(
                "opened {}",
                if f.forge().kind == common::forge::Kind::GitHub {
                    "pull request #2"
                } else {
                    "merge request !2"
                }
            )),
            "{out}"
        );
        assert_eq!(f.forge().mr(fork).state, "open");
    }
    // A fork's closed merge request declines nothing.
    let f = Fork::merge_request();
    let target = behind(&f);
    let fork = fork_request(&f, &target);
    f.forge().end(fork, "closed");
    let out = stdout(&f.tool(&["sync"]).success());
    assert!(out.contains("opened merge request !2"), "{out}");
}

#[test]
fn credentials_in_urls_never_reach_the_output() {
    let f = Fork::merge_request();
    let env = &f.env;
    behind(&f);
    // The publish remote's URL carries a credential.
    let url = f
        .forge()
        .remote_url()
        .replace("ssh://", "ssh://user:s3cret-word@");
    env.git(&f.down, &["remote", "set-url", "origin", &url]);
    // A branch of unknown origin makes the refusal name the remote's URL.
    let dir = common::forge(env, &f.publish);
    env.commit(&dir, &[("w.txt", "w\n")], "someone's work");
    env.git(
        &dir,
        &[
            "push",
            "-q",
            "origin",
            &format!("HEAD:refs/heads/{MERGE_REQUEST_BRANCH}"),
        ],
    );
    let sync = f.tool(&["sync"]).code(2);
    let (out, err) = (stdout(&sync), stderr(&sync));
    // An SSH URL keeps its user, never the password.
    assert!(err.contains("ssh://user@127.0.0.1/team/app.git"), "{err}");
    assert!(
        !out.contains("s3cret") && !err.contains("s3cret"),
        "{out}{err}"
    );
    // An upstream whose URL carries a credential and cannot be fetched.
    let g = Fork::new();
    g.reconfigure(
        &format!("url = {:?}", g.up.url()),
        "url = \"https://user:s3cret-word@127.0.0.1:9/team/up.git\"",
    );
    let status = g.tool(&["status"]).code(1);
    let (out, err) = (stdout(&status), stderr(&status));
    assert!(out.contains("https://127.0.0.1:9/team/up.git"), "{out}");
    assert!(
        !out.contains("s3cret") && !err.contains("s3cret"),
        "{out}{err}"
    );
    let json = g.env.tool_json(&g.down, &["status", "--json"], 1);
    assert!(!json.to_string().contains("s3cret"), "{json}");
}

#[test]
fn a_job_s_api_on_another_host_is_refused() {
    let f = Fork::merge_request();
    behind(&f);
    let mut elsewhere = mockito::Server::new();
    let nothing = elsewhere
        .mock("GET", mockito::Matcher::Any)
        .expect(0)
        .create();
    let api = format!(
        "{}/api/v4",
        elsewhere.url().replace("127.0.0.1", "localhost")
    );
    f.env.set_var("CI_API_V4_URL", &api);
    let err = stderr(&f.tool(&["sync"]).code(2));
    assert!(
        err.contains("CI_API_V4_URL names the host 'localhost'"),
        "{err}"
    );
    nothing.assert();
    assert!(f.forge().requests().is_empty());
}

#[test]
fn a_branch_covered_by_rulesets_alone_has_no_classic_protection() {
    let f = Fork::github();
    behind(&f);
    f.tool(&["start"]).success();
    // GitHub counts rulesets as protection, and the classic protection of
    // such a branch answers 404 "Branch not protected".
    {
        let mut w = f.forge().world();
        w.protected = true;
        w.protection = Err((404, "Branch not protected"));
    }
    f.tool(&["finish"]).success();
    assert_eq!(f.forge().open_mrs().len(), 1);

    // A 404 "Not Found" is a token that cannot read the protection.
    let g = Fork::github();
    behind(&g);
    g.tool(&["start"]).success();
    {
        let mut w = g.forge().world();
        w.protected = true;
        w.protection = Err((404, "Not Found"));
    }
    let err = stderr(&g.tool(&["finish"]).code(2));
    assert!(err.contains("could not be confirmed"), "{err}");
    assert!(g.forge().open_mrs().is_empty());
}

#[test]
fn a_project_the_forge_does_not_show_is_named_as_such() {
    let f = Fork::merge_request();
    f.forge()
        .fail("GET", "/projects/team%2Fapp", 404, "404 Project Not Found");
    let doctor = stdout(&f.tool(&["doctor"]).code(1));
    assert!(doctor.contains("404 404 Project Not Found"), "{doctor}");
    assert!(
        doctor.contains("the project was not found, or the token cannot see it"),
        "{doctor}"
    );
}

#[test]
fn a_renamed_github_repository_keeps_its_pull_requests() {
    let f = Fork::github();
    behind(&f);
    f.tool(&["sync"]).success();
    // GitHub now names the repository otherwise than the remote's URL.
    f.forge().world().full_name = "acme/app-renamed".to_string();
    let out = stdout(&f.tool(&["sync"]).success());
    assert!(
        out.contains("the update waits for the pull request #1"),
        "{out}"
    );
    let status = f.env.tool_json(&f.down, &["status", "--json"], 0);
    assert_eq!(status["merge_request"]["number"], 1);
}

#[test]
fn a_forge_message_that_echoes_the_token_is_shown_without_it() {
    let f = Fork::merge_request();
    f.forge().fail(
        "GET",
        "/projects/team%2Fapp",
        401,
        &format!("401 Unauthorized: the token {TOKEN} is not valid"),
    );
    let shown = |args: &[&str], code: i32| {
        let run = f.tool(args).code(code);
        format!("{}{}", stdout(&run), stderr(&run))
    };
    behind(&f);
    f.tool(&["start"]).success();
    f.tool(&["verify"]).success();
    for (args, code) in [
        (&["doctor"][..], 1),
        (&["doctor", "--json"], 1),
        (&["finish"], 2),
    ] {
        let out = shown(args, code);
        assert!(
            out.contains("401 Unauthorized: the token"),
            "{args:?}: {out}"
        );
        assert!(!out.contains(TOKEN), "{args:?}: {out}");
    }
}
