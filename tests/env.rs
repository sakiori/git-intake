//! `env`: the variables `verify` gives the checks, read from history alone,
//! as a merge request pipeline runs them on a fresh clone.

mod common;

use std::fs;
use std::path::PathBuf;

use common::{Fork, MERGE_REQUEST_BRANCH, Mono, stderr, stdout};

/// A command that writes the tool's variables, sorted, to `$HOME/<file>`.
fn dump(file: &str) -> String {
    format!(
        "awk 'BEGIN {{ for (k in ENVIRON) if (k ~ /^GIT_INTAKE_/) print k \"=\" ENVIRON[k] }}' \
         | LC_ALL=C sort > \"$HOME/{file}\""
    )
}

fn dumped(env: &common::Env, file: &str) -> Vec<String> {
    fs::read_to_string(env.path("home").join(file))
        .unwrap_or_else(|e| panic!("no {file}: {e}"))
        .lines()
        .map(str::to_string)
        .collect()
}

/// The variables `env --json` prints, as sorted `NAME=value` lines.
fn json_lines(json: &serde_json::Value) -> Vec<String> {
    let mut lines: Vec<String> = json
        .as_object()
        .unwrap()
        .iter()
        .map(|(k, v)| format!("{k}={}", v.as_str().unwrap()))
        .collect();
    lines.sort();
    lines
}

/// A full clone of the publish remote, as a pipeline makes it, at `rev`
/// detached.
fn pipeline_clone(f: &Fork, rev: &str) -> PathBuf {
    let env = &f.env;
    let dir = env.path("pipeline");
    env.git(
        &env.path(""),
        &[
            "clone",
            "-q",
            &f.publish.to_string_lossy(),
            &dir.to_string_lossy(),
        ],
    );
    env.git(&dir, &["checkout", "-q", "--detach", rev]);
    dir
}

#[test]
fn a_merge_request_pipeline_gets_what_verify_gives() {
    let f = Fork::merge_request();
    let env = &f.env;
    f.reconfigure(
        "checks = [\"test -f app.txt\"]",
        &format!("checks = [{:?}]", dump("verify-env")),
    );
    env.push_upstream(&f.up, &[("n.txt", "n\n")], "upstream: n");
    f.tool(&["start"]).success();
    f.tool(&["verify"]).success();
    let verified = dumped(env, "verify-env");
    assert!(
        verified
            .iter()
            .any(|l| l.starts_with("GIT_INTAKE_UPSTREAM_UPSTREAM_PREVIOUS="))
    );
    f.tool(&["finish"]).success();

    let head = common::forge::branch_tip(&f.publish, MERGE_REQUEST_BRANCH).unwrap();
    let ci = pipeline_clone(&f, &head);
    env.tool(&ci, &["env", "--", "sh", "-c", &dump("pipeline-env")])
        .success();
    assert_eq!(dumped(env, "pipeline-env"), verified);
    let json = env.tool_json(&ci, &["env", "--json"], 0);
    assert_eq!(json_lines(&json), verified);
    let exports = stdout(&env.tool(&ci, &["env"]).success());
    assert!(
        exports.contains(&format!("export GIT_INTAKE_HEAD='{head}'\n")),
        "{exports}"
    );
}

#[test]
fn a_merged_results_pipeline_gets_the_target_as_its_base() {
    let f = Fork::merge_request();
    let env = &f.env;
    let target = env.push_upstream(&f.up, &[("n.txt", "n\n")], "upstream: n");
    f.tool(&["start"]).success();
    f.tool(&["finish"]).success();
    let head = common::forge::branch_tip(&f.publish, MERGE_REQUEST_BRANCH).unwrap();
    let ci = pipeline_clone(&f, "origin/main");
    let main = env.rev(&ci, "HEAD");
    env.git(
        &ci,
        &[
            "merge",
            "-q",
            "--no-ff",
            "-m",
            "Merge the merge request into main",
            &head,
        ],
    );
    let merged = env.rev(&ci, "HEAD");
    let json = env.tool_json(&ci, &["env", "--json"], 0);
    assert_eq!(json["GIT_INTAKE_BASE"], main.as_str());
    assert_eq!(json["GIT_INTAKE_HEAD"], merged.as_str());
    assert_eq!(json["GIT_INTAKE_UPSTREAMS"], "upstream");
    assert_eq!(json["GIT_INTAKE_UPSTREAM_UPSTREAM_TARGET"], target.as_str());
}

#[test]
fn a_branch_updated_by_the_forge_keeps_its_update() {
    let f = Fork::merge_request();
    let env = &f.env;
    let target = env.push_upstream(&f.up, &[("n.txt", "n\n")], "upstream: n");
    f.tool(&["start"]).success();
    f.tool(&["finish"]).success();
    // Other work lands, and a person merges it into the merge request
    // branch with the forge's update button.
    let dir = common::forge(env, &f.publish);
    let tip = env.commit(&dir, &[("other.txt", "o\n")], "other: lands");
    env.git(&dir, &["push", "-q", "origin", "HEAD:main"]);
    env.git(
        &dir,
        &[
            "checkout",
            "-q",
            "-b",
            "mr",
            &format!("origin/{MERGE_REQUEST_BRANCH}"),
        ],
    );
    env.git(
        &dir,
        &[
            "merge",
            "-q",
            "--no-ff",
            "-m",
            "Merge branch 'main' into 'git-intake/update'",
            "main",
        ],
    );
    env.git(
        &dir,
        &[
            "push",
            "-q",
            "origin",
            &format!("HEAD:{MERGE_REQUEST_BRANCH}"),
        ],
    );
    let updated = env.rev(&dir, "HEAD");
    let ci = pipeline_clone(&f, &updated);
    let json = env.tool_json(&ci, &["env", "--json"], 0);
    assert_eq!(json["GIT_INTAKE_BASE"], tip.as_str());
    assert_eq!(json["GIT_INTAKE_HEAD"], updated.as_str());
    assert_eq!(json["GIT_INTAKE_UPSTREAM_UPSTREAM_TARGET"], target.as_str());
}

#[test]
fn dotenv_lines_are_unquoted() {
    let m = Mono::new();
    let env = &m.env;
    m.tool(&["start"]).success();
    let wt = env.update_worktree(&m.down);
    let out = stdout(&env.tool(&wt, &["env", "--dotenv"]).success());
    assert!(out.contains("GIT_INTAKE_UPSTREAMS=app-a app-b\n"), "{out}");
    assert!(
        out.contains("GIT_INTAKE_UPSTREAM_APP_A_MODE=follow\n"),
        "{out}"
    );
    assert!(
        out.contains("GIT_INTAKE_UPSTREAM_APP_B_MODE=follow\n"),
        "{out}"
    );
    assert!(!out.contains('\''), "{out}");
    // The default form is quoted for the shell.
    let out = stdout(&env.tool(&wt, &["env"]).success());
    assert!(
        out.contains("export GIT_INTAKE_UPSTREAMS='app-a app-b'\n"),
        "{out}"
    );
}

#[test]
fn a_commit_without_an_update_gets_no_variable() {
    let f = Fork::new();
    let env = &f.env;
    let main = env.tool(&f.down, &["env"]).success();
    assert_eq!(stdout(&main), "");
    assert!(
        stderr(&main).contains("carries no update"),
        "{}",
        stderr(&main)
    );
    let json = env.tool_json(&f.down, &["env", "--json"], 0);
    assert_eq!(json, serde_json::json!({}));
    // A feature branch whose line holds no sync merge, copy or journal
    // change carries none either.
    env.git(&f.down, &["checkout", "-q", "-b", "feature"]);
    env.commit(&f.down, &[("feature.txt", "f\n")], "feature: work");
    let marker = env.path("home").join("ran");
    env.tool_env(
        &f.down,
        &[
            "env",
            "--",
            "sh",
            "-c",
            &format!("test -z \"$GIT_INTAKE_BASE\" && touch {}", marker.display()),
        ],
        &[("GIT_INTAKE_BASE", "inherited")],
    )
    .success()
    .stderr(predicates::str::contains("carries no update"));
    assert!(marker.exists());
}

#[test]
fn the_command_s_exit_status_is_env_s() {
    let f = Fork::new();
    let env = &f.env;
    env.push_upstream(&f.up, &[("n.txt", "n\n")], "upstream: n");
    f.tool(&["start"]).success();
    let wt = env.update_worktree(&f.down);
    env.tool(
        &wt,
        &[
            "env",
            "--",
            "sh",
            "-c",
            "test -n \"$GIT_INTAKE_HEAD\" && exit 3",
        ],
    )
    .code(3);
    env.tool(&wt, &["env", "--", "true"]).code(0);
}

#[test]
fn an_unreachable_publish_remote_sets_no_variable() {
    let f = Fork::new();
    let env = &f.env;
    env.push_upstream(&f.up, &[("n.txt", "n\n")], "upstream: n");
    f.tool(&["start"]).success();
    env.git(
        &f.down,
        &[
            "remote",
            "set-url",
            "origin",
            &env.path("gone.git").to_string_lossy(),
        ],
    );
    let wt = env.update_worktree(&f.down);
    let marker = env.path("home").join("ran");
    let err = stderr(
        &env.tool(&wt, &["env", "--", "touch", &marker.to_string_lossy()])
            .code(2),
    );
    assert!(
        err.contains("cannot fetch 'main' from the publish remote 'origin'"),
        "{err}"
    );
    assert!(!marker.exists(), "the command ran");
}

#[test]
fn a_shallow_clone_runs_no_command() {
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
    let marker = env.path("home").join("ran");
    let err = stderr(
        &env.tool(&dir, &["env", "--", "touch", &marker.to_string_lossy()])
            .code(2),
    );
    assert!(err.contains("`git fetch --unshallow`"), "{err}");
    assert!(!marker.exists(), "the command ran");
}

#[test]
fn a_port_update_of_decisions_only_sets_its_variables() {
    let f = Fork::port();
    let env = &f.env;
    f.reconfigure(
        "checks = [\"test -f app.txt\"]",
        &format!("checks = [{:?}]", dump("verify-env")),
    );
    let c1 = env.push_upstream(&f.up, &[("p1.txt", "1\n")], "upstream: one");
    let c2 = env.push_upstream(&f.up, &[("p2.txt", "2\n")], "upstream: two");
    f.tool(&["start"]).success();
    let plan = env.intake_dir(&f.down).join("plan");
    fs::write(
        &plan,
        format!("skip {} # not ours\nlater {}\n", &c1[..12], &c2[..12]),
    )
    .unwrap();
    f.tool(&["continue"]).success();
    f.tool(&["verify"]).success();
    let verified = dumped(env, "verify-env");
    let wt = env.update_worktree(&f.down);
    let json = env.tool_json(&wt, &["env", "--json"], 0);
    assert_eq!(json["GIT_INTAKE_UPSTREAM_UPSTREAM_MODE"], "port");
    assert_eq!(json["GIT_INTAKE_UPSTREAM_UPSTREAM_TARGET"], c1.as_str());
    assert_eq!(json_lines(&json), verified);
}

#[test]
fn an_upstream_s_own_sync_records_do_not_count() {
    let f = Fork::new();
    let env = &f.env;
    // The upstream's history carries a sync merge of an upstream with the
    // same name, as a stacked downstream branch does.
    let work = &f.up.work;
    env.git(work, &["checkout", "-q", "-b", "side"]);
    let side = env.commit(work, &[("side.txt", "s\n")], "side: work");
    env.git(work, &["checkout", "-q", "main"]);
    env.git(
        work,
        &[
            "merge",
            "-q",
            "--no-ff",
            "-m",
            &format!(
                "sync(upstream): merge side\n\nIntake-Upstream: upstream\nIntake-Target: {side}\n"
            ),
            "side",
        ],
    );
    env.git(work, &["push", "-q", "origin", "main"]);
    let tip = env.rev(work, "HEAD");
    f.tool(&["start"]).success();
    let wt = env.update_worktree(&f.down);
    let json = env.tool_json(&wt, &["env", "--json"], 0);
    assert_eq!(json["GIT_INTAKE_UPSTREAM_UPSTREAM_TARGET"], tip.as_str());
}
