//! The integration tip is the publish remote's integration branch: every
//! command reads it there, and no local integration branch is read, created
//! or moved.

mod common;

use common::{Env, Fork, MERGE_REQUEST_BRANCH, open_number, stderr, stdout};
use predicates::str::contains;

/// Deletes the local `main`, leaving `HEAD` detached at its commit, as a
/// clone that keeps no local integration branch has it.
fn without_local_main(env: &Env, down: &std::path::Path) {
    env.git(down, &["switch", "-q", "--detach", "main"]);
    env.git(down, &["branch", "-q", "-D", "main"]);
}

/// Whether the local `main` exists.
fn local_main(env: &Env, down: &std::path::Path) -> Option<String> {
    env.has_rev(down, "refs/heads/main")
        .then(|| env.rev(down, "refs/heads/main"))
}

/// Lands a commit on the publish remote's `main` from another clone, as
/// other work does; returns it.
fn lands_elsewhere(f: &Fork, files: &[(&str, &str)], message: &str) -> String {
    let env = &f.env;
    let dir = common::forge(env, &f.publish);
    let c = env.commit(&dir, files, message);
    env.git(&dir, &["push", "-q", "origin", "HEAD:main"]);
    c
}

/// Changes the configuration on the publish remote's `main` from another
/// clone, replacing `from` with `to`; returns the commit.
fn reconfigured_elsewhere(f: &Fork, from: &str, to: &str) -> String {
    let dir = common::forge(&f.env, &f.publish);
    let text = std::fs::read_to_string(dir.join(".git-intake.toml")).unwrap();
    assert!(text.contains(from), "{text}");
    lands_elsewhere(
        f,
        &[(".git-intake.toml", &text.replace(from, to))],
        "other: reconfigure",
    )
}

#[test]
fn every_command_works_without_a_local_integration_branch() {
    let f = Fork::new();
    let env = &f.env;
    without_local_main(env, &f.down);
    let target = env.push_upstream(&f.up, &[("n.txt", "n\n")], "upstream: n");
    f.tool(&["status"])
        .success()
        .stdout(contains("integration: main on 'origin'"))
        .stdout(contains("behind"));
    f.tool(&["log"]).success().stdout(contains("upstream: n"));
    f.tool(&["branches"]).success();
    f.tool(&["doctor"]).success();
    f.tool(&["start"]).success();
    f.tool(&["status"]).success();
    f.tool(&["verify"]).success();
    f.tool(&["report"]).success();
    f.tool(&["finish"])
        .success()
        .stdout(contains("to 'origin/main'"));
    let tip = f.tip();
    assert!(env.is_ancestor(&f.publish, &target, &tip));
    assert_eq!(local_main(env, &f.down), None);
    // A commit of the integration branch carries no update.
    f.tool(&["env", &tip])
        .success()
        .stderr(contains("no update"));
    f.tool(&["conflicts"]).success();
    f.tool(&["status", "--check"]).success();
    f.tool(&["abort"])
        .success()
        .stdout(contains("no update is in progress"));
    env.assert_no_update(&f.down);
    assert_eq!(local_main(env, &f.down), None);
}

#[test]
fn start_and_finish_leave_the_local_integration_branch_alone() {
    let f = Fork::new();
    let env = &f.env;
    // Local work never published, and an uncommitted change in the main
    // checkout: neither is the update's business.
    let local = env.commit(&f.down, &[("local.txt", "l\n")], "local: work");
    std::fs::write(f.down.join(common::FORK_FILE), "edited\n").unwrap();
    let published = f.tip();
    let target = env.push_upstream(&f.up, &[("n.txt", "n\n")], "upstream: n");
    f.tool(&["start"]).success().stdout(contains(format!(
        "update started from 'origin/main' at {}",
        &published[..12]
    )));
    let wt = env.update_worktree(&f.down);
    assert!(env.is_ancestor(&wt, &published, "HEAD"));
    assert!(!env.is_ancestor(&wt, &local, "HEAD"));
    f.tool(&["verify"]).success();
    f.tool(&["finish"]).success();
    let tip = f.tip();
    assert!(env.is_ancestor(&f.publish, &target, &tip));
    assert_eq!(env.rev(&f.publish, &format!("{tip}^1")), published);
    assert_eq!(local_main(env, &f.down).as_deref(), Some(local.as_str()));
    assert_eq!(
        std::fs::read_to_string(f.down.join(common::FORK_FILE)).unwrap(),
        "edited\n"
    );
    env.assert_no_update(&f.down);
}

#[test]
fn sync_by_push_neither_moves_nor_creates_a_local_integration_branch() {
    let f = Fork::new();
    let env = &f.env;
    let before = local_main(env, &f.down).unwrap();
    lands_elsewhere(&f, &[("other.txt", "o\n")], "other: lands");
    env.push_upstream(&f.up, &[("n.txt", "n\n")], "upstream: n");
    f.tool(&["sync"]).success();
    assert_eq!(local_main(env, &f.down).as_deref(), Some(before.as_str()));
    // A CI clone at a detached HEAD gets no local integration branch.
    without_local_main(env, &f.down);
    let target = env.push_upstream(&f.up, &[("m.txt", "m\n")], "upstream: m");
    f.tool(&["sync"]).success();
    assert!(env.is_ancestor(&f.publish, &target, &f.tip()));
    assert_eq!(local_main(env, &f.down), None);
    env.assert_no_update(&f.down);
}

#[test]
fn merge_request_mode_never_touches_the_local_integration_branch() {
    let f = Fork::merge_request();
    let env = &f.env;
    // A local commit the remote lacks rides along in nothing: the update
    // starts from the remote's tip.
    let local = env.commit(&f.down, &[("local.txt", "l\n")], "local: work");
    env.push_upstream(&f.up, &[("n.txt", "n\n")], "upstream: n");
    f.tool(&["start"]).success();
    f.tool(&["finish"]).success();
    let number = open_number(f.forge());
    let head = common::forge::branch_tip(&f.publish, MERGE_REQUEST_BRANCH).unwrap();
    assert!(!env.is_ancestor(&f.publish, &local, &head));
    // The integration branch moves on; sync catches the merge request up.
    let other = lands_elsewhere(&f, &[("other.txt", "o\n")], "other: lands");
    f.tool(&["sync"]).success();
    let caught = common::forge::branch_tip(&f.publish, MERGE_REQUEST_BRANCH).unwrap();
    assert_eq!(env.rev(&f.publish, &format!("{caught}^1")), other);
    // land merges it, from a clone without a local integration branch too.
    without_local_main(env, &f.down);
    f.tool(&["land", "--poll", "0", "--timeout", "120"])
        .success();
    assert_eq!(f.forge().mr(number).state, "merged");
    assert!(env.is_ancestor(&f.publish, &caught, &f.tip()));
    assert_eq!(local_main(env, &f.down), None);
    f.tool(&["sync"]).success();
    assert_eq!(local_main(env, &f.down), None);
    env.assert_no_update(&f.down);
}

#[test]
fn work_landing_on_the_publish_remote_during_an_update_is_caught_up_with() {
    let f = Fork::new();
    let env = &f.env;
    let before = local_main(env, &f.down).unwrap();
    env.push_upstream(&f.up, &[("n.txt", "n\n")], "upstream: n");
    f.tool(&["start"]).success();
    f.tool(&["verify"]).success();
    let other = lands_elsewhere(&f, &[("other.txt", "o\n")], "other: lands");
    let out = stdout(&f.tool(&["finish"]).success());
    assert!(
        out.contains(&format!("catching up with 'main' at {}", &other[..12])),
        "{out}"
    );
    let tip = f.tip();
    assert_eq!(env.rev(&f.publish, &format!("{tip}^1")), other);
    let message = env.git(&f.publish, &["log", "-1", "--format=%B", &tip]);
    assert!(message.contains("Intake-Catch-Up: main"), "{message}");
    assert_eq!(local_main(env, &f.down).as_deref(), Some(before.as_str()));
    env.assert_no_update(&f.down);
}

#[test]
fn checks_added_on_the_publish_remote_during_an_update_verify_its_catch_up() {
    let f = Fork::new();
    let env = &f.env;
    env.push_upstream(&f.up, &[("n.txt", "n\n")], "upstream: n");
    f.tool(&["start"]).success();
    f.tool(&["verify"]).success();
    reconfigured_elsewhere(
        &f,
        "{ name = \"app\", run = \"test -f app.txt\" }",
        "{ name = \"app\", run = \"test -f app.txt\" }, { name = \"other\", run = \"test -f n.txt\" }",
    );
    f.tool(&["finish"])
        .success()
        .stdout(contains("passed   other"));
    env.assert_no_update(&f.down);
}

#[test]
fn a_publish_remote_naming_another_integration_branch_is_refused() {
    let f = Fork::new();
    let env = &f.env;
    // Without an update, HEAD names where to fetch from.
    reconfigured_elsewhere(&f, "integration = \"main\"", "integration = \"trunk\"");
    let err = stderr(&f.tool(&["start"]).code(2));
    assert!(
        err.contains(
            "names 'trunk' as the integration branch and 'origin' as the publish remote, but the \
             .git-intake.toml at HEAD names 'main' and 'origin'"
        ),
        "{err}"
    );
    f.tool(&["status"]).code(2);
    env.assert_no_update(&f.down);

    // During an update, the update names it, and its catch-up refuses.
    let f = Fork::new();
    let env = &f.env;
    env.push_upstream(&f.up, &[("n.txt", "n\n")], "upstream: n");
    f.tool(&["start"]).success();
    f.tool(&["verify"]).success();
    reconfigured_elsewhere(&f, "integration = \"main\"", "integration = \"trunk\"");
    let err = stderr(&f.tool(&["status"]).code(2));
    assert!(
        err.contains("but the update in progress names 'main' and 'origin'"),
        "{err}"
    );
    let err = stderr(&f.tool(&["finish"]).code(2));
    assert!(
        err.contains("names 'trunk' as the integration branch instead of 'main'"),
        "{err}"
    );
    assert!(err.contains("Nothing was merged or pushed"), "{err}");
}

#[test]
fn a_publish_remote_naming_another_publish_remote_is_refused() {
    let f = Fork::new();
    reconfigured_elsewhere(&f, "remote = \"origin\"", "remote = \"elsewhere\"");
    for command in [&["log"][..], &["release", "next"], &["release", "tag"]] {
        let err = stderr(&f.tool(command).code(2));
        assert!(
            err.contains(
                "names 'main' as the integration branch and 'elsewhere' as the publish remote, but the \
                 .git-intake.toml at HEAD names 'main' and 'origin'"
            ),
            "{command:?}: {err}"
        );
    }
}

#[test]
fn a_configuration_without_a_publish_remote_is_refused() {
    let env = Env::new();
    let up = env.upstream("upstream", &[("a.txt", "a\n")]);
    let down = env.work("down");
    env.commit(
        &down,
        &[(
            ".git-intake.toml",
            &format!(
                "integration = \"main\"\n\n[[upstream]]\nname = \"upstream\"\nurl = {:?}\nbranch = \"main\"\n",
                up.url()
            ),
        )],
        "down: configure",
    );
    for command in ["start", "status", "sync"] {
        let err = stderr(&env.tool(&down, &[command]).code(2));
        assert!(
            err.contains("publish: a [publish] table naming the publish remote is required"),
            "{command}: {err}"
        );
    }
    // A [publish] table naming a remote the clone lacks is named too.
    env.commit(
        &down,
        &[(
            ".git-intake.toml",
            &format!(
                "integration = \"main\"\n\n[publish]\nremote = \"origin\"\n\n[[upstream]]\nname = \"upstream\"\n\
                 url = {:?}\nbranch = \"main\"\n",
                up.url()
            ),
        )],
        "down: publish",
    );
    env.tool(&down, &["start"]).code(2).stderr(contains(
        "the publish remote 'origin' does not exist; add the remote that holds the integration branch 'main'",
    ));
    env.assert_no_update(&down);
}

#[test]
fn an_unreachable_publish_remote_refuses_with_git_s_reason_and_doctor_reports_it() {
    let f = Fork::new();
    let env = &f.env;
    let gone = env.path("gone.git");
    env.git(
        &f.down,
        &["remote", "set-url", "origin", &gone.to_string_lossy()],
    );
    for command in ["start", "status", "sync", "log"] {
        let err = stderr(&f.tool(&[command]).code(2));
        assert!(
            err.contains(
                "cannot fetch the integration branch 'main' from the publish remote 'origin'"
            ),
            "{command}: {err}"
        );
        assert!(
            err.contains("does not appear to be a git repository"),
            "{command}: {err}"
        );
    }
    let doc = env.tool_json(&f.down, &["doctor", "--json"], 1);
    let checks = doc["checks"].as_array().unwrap();
    let remote = checks
        .iter()
        .find(|c| c["check"] == "publish_remote")
        .unwrap();
    assert_eq!(remote["status"], "fail");
    assert!(!checks.iter().any(|c| c["check"] == "fetch"), "{doc}");
    env.assert_no_update(&f.down);
}

#[test]
fn the_update_lives_on_its_fixed_branch_and_worktree() {
    let f = Fork::new();
    let env = &f.env;
    env.push_upstream(&f.up, &[("n.txt", "n\n")], "upstream: n");
    f.tool(&["start"]).success();
    assert!(env.update_worktree(&f.down).is_dir());
    assert!(env.has_rev(&f.down, "refs/heads/git-intake/update"));
    f.tool(&["verify"]).success();
    f.tool(&["finish"]).success();
    assert!(!f.down.join(".worktrees").exists());
    env.assert_no_update(&f.down);
    // The update branch is not a setting.
    reconfigured_elsewhere(
        &f,
        "integration = \"main\"\n",
        "integration = \"main\"\nupdate_branch = \"bot/intake\"\n",
    );
    f.tool(&["start"])
        .code(2)
        .stderr(contains("unknown field `update_branch`"));
}

#[test]
fn abort_without_an_update_needs_no_publish_remote() {
    let f = Fork::new();
    let env = &f.env;
    let gone = env.path("gone.git");
    env.git(
        &f.down,
        &["remote", "set-url", "origin", &gone.to_string_lossy()],
    );
    f.tool(&["abort"])
        .success()
        .stdout(contains("no update is in progress; nothing to abort"));
    // What an interrupted start left behind goes as well.
    env.git(&f.down, &["branch", "git-intake/update"]);
    f.tool(&["abort"])
        .success()
        .stdout(contains("removed branch git-intake/update"))
        .stdout(contains("removed the remains of an interrupted update"));
    env.assert_no_update(&f.down);
}
