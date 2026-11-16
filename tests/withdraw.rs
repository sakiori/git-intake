//! `abort --merge-request`, which withdraws the update's merge request
//! through the forge, and updates an earlier release left waiting for their
//! merge requests, which `finish` hands over to them.

mod common;

use std::fs;

use common::forge::branch_tip;
use common::{Fork, How, MERGE_REQUEST_BRANCH, as_earlier_release, merge_on_fake, stderr, stdout};
use serde_json::json;

fn behind(f: &Fork) -> String {
    f.env
        .push_upstream(&f.up, &[("n.txt", "n\n")], "upstream: n")
}

#[test]
fn withdrawing_closes_the_merge_request_and_deletes_its_branch() {
    let f = Fork::merge_request();
    let env = &f.env;
    behind(&f);
    f.tool(&["sync"]).success();
    let main = env.rev(&f.publish, "main");
    let out = stdout(&f.tool(&["abort", "--merge-request"]).success());
    assert!(out.contains("closed merge request !1"), "{out}");
    assert!(
        out.contains(&format!(
            "deleted the branch '{MERGE_REQUEST_BRANCH}' from 'origin'"
        )),
        "{out}"
    );
    assert!(out.contains("its targets count as declined"), "{out}");
    assert_eq!(f.forge().mr(1).state, "closed");
    assert_eq!(branch_tip(&f.publish, MERGE_REQUEST_BRANCH), None);
    assert_eq!(env.rev(&f.publish, "main"), main);
    // The next sync proposes nothing until an upstream moves on.
    let out = stdout(&f.tool(&["sync"]).success());
    assert!(out.contains("declined these targets"), "{out}");
}

#[test]
fn a_branch_moved_by_someone_else_is_kept() {
    let f = Fork::merge_request();
    let env = &f.env;
    behind(&f);
    f.tool(&["sync"]).success();
    let read = branch_tip(&f.publish, MERGE_REQUEST_BRANCH).unwrap();
    // A push the forge has not processed yet moved the branch.
    f.forge().pin_head(1, &read);
    let dir = common::forge(env, &f.publish);
    env.git(
        &dir,
        &[
            "checkout",
            "-q",
            "-b",
            "w",
            &format!("origin/{MERGE_REQUEST_BRANCH}"),
        ],
    );
    let work = env.commit(&dir, &[("w.txt", "w\n")], "someone's fix");
    env.git(
        &dir,
        &[
            "push",
            "-q",
            "origin",
            &format!("HEAD:{MERGE_REQUEST_BRANCH}"),
        ],
    );
    let out = stdout(&f.tool(&["abort", "--merge-request"]).success());
    assert!(out.contains("closed merge request !1"), "{out}");
    assert!(
        out.contains(&format!(
            "kept the branch '{MERGE_REQUEST_BRANCH}' on 'origin'"
        )),
        "{out}"
    );
    assert_eq!(branch_tip(&f.publish, MERGE_REQUEST_BRANCH), Some(work));
}

#[test]
fn an_interrupted_withdrawal_deletes_the_branch_left_behind() {
    let f = Fork::github();
    behind(&f);
    f.tool(&["sync"]).success();
    // Closed, but the branch was not deleted yet.
    f.forge().end(1, "closed");
    let out = stdout(&f.tool(&["abort", "--merge-request"]).success());
    assert!(
        out.contains(&format!(
            "deleted the branch '{MERGE_REQUEST_BRANCH}' from 'origin'"
        )),
        "{out}"
    );
    assert_eq!(branch_tip(&f.publish, MERGE_REQUEST_BRANCH), None);
    let out = stdout(&f.tool(&["abort", "--merge-request"]).success());
    assert!(out.contains("there is nothing to withdraw"), "{out}");
}

#[test]
fn withdrawing_while_carrying_on_removes_the_local_update() {
    let f = Fork::merge_request();
    let env = &f.env;
    env.push_upstream(
        &f.up,
        &[("app.txt", "upstream\ntwo\nthree\n")],
        "upstream: change",
    );
    f.tool(&["sync"]).success();
    let dir = common::forge(env, &f.publish);
    env.commit(&dir, &[("app.txt", "fork\ntwo\nthree\n")], "fork: change");
    env.git(&dir, &["push", "-q", "origin", "HEAD:main"]);
    f.tool(&["sync"]).code(1);
    let out = stdout(&f.tool(&["abort", "--merge-request"]).success());
    assert!(out.contains("update aborted"), "{out}");
    assert!(out.contains("closed merge request !1"), "{out}");
    env.assert_no_update(&f.down);
    assert_eq!(branch_tip(&f.publish, MERGE_REQUEST_BRANCH), None);
}

#[test]
fn plain_abort_never_closes_a_merge_request() {
    let f = Fork::merge_request();
    behind(&f);
    f.tool(&["sync"]).success();
    f.forge().clear_requests();
    let out = stdout(&f.tool(&["abort"]).success());
    assert!(!out.contains("update aborted"), "{out}");
    assert!(out.contains("merge request !1"), "{out}");
    assert!(
        out.contains("`git intake abort --merge-request` withdraws it"),
        "{out}"
    );
    assert!(f.forge().changes().is_empty());
    // Without the forge, it says it could not read it.
    f.env.unset_var("GITLAB_TOKEN");
    let out = stdout(&f.tool(&["abort"]).success());
    assert!(out.contains("could not read the forge"), "{out}");
}

#[test]
fn withdrawing_without_a_token_removes_nothing() {
    let f = Fork::merge_request();
    behind(&f);
    f.tool(&["start"]).success();
    f.env.unset_var("GITLAB_TOKEN");
    let err = stderr(&f.tool(&["abort", "--merge-request"]).code(2));
    assert!(err.contains("GITLAB_TOKEN"), "{err}");
    assert!(
        f.env.update_worktree(&f.down).exists(),
        "the update is kept"
    );
}

/// Turns the update in progress into one an earlier release left waiting
/// for its merge request: the update head pushed to `branch`, the worktree
/// removed, and the state naming the merge request. Returns the head.
fn waiting(f: &Fork, branch: &str, number: Option<u64>, version: u64) -> String {
    let env = &f.env;
    let wt = env.update_worktree(&f.down);
    let head = env.rev(&wt, "HEAD");
    env.git(
        &f.down,
        &[
            "push",
            "-q",
            &f.publish.to_string_lossy(),
            &format!("{head}:refs/heads/{branch}"),
        ],
    );
    env.git(
        &f.down,
        &["worktree", "remove", "--force", &wt.to_string_lossy()],
    );
    let path = env.intake_dir(&f.down).join("state.json");
    let mut state: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    // Another release, so that a command that wrote the state for its
    // release warning would change it.
    as_earlier_release(&mut state, version, "0.3.6");
    let mut mr = json!({
        "remote": "origin",
        "branch": branch,
        "commit": head,
        "requested": true,
        "last_seen_head": head,
        "pushed": [head],
    });
    if let Some(n) = number {
        mr["number"] = n.into();
    }
    state["merge_request"] = mr;
    fs::write(&path, state.to_string()).unwrap();
    head
}

#[test]
fn finish_hands_a_waiting_update_over_to_its_open_merge_request() {
    let f = Fork::merge_request();
    let env = &f.env;
    behind(&f);
    f.tool(&["start"]).success();
    f.tool(&["verify"]).success();
    let head = waiting(&f, MERGE_REQUEST_BRANCH, Some(1), 5);
    let n = f.forge().add(
        MERGE_REQUEST_BRANCH,
        "main",
        "Upstream changes verified by git intake.",
    );
    assert_eq!(n, 1);

    // A catch-up of the earlier release moved its base from the one it
    // started from.
    let path = env.intake_dir(&f.down).join("state.json");
    let mut state: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    let base = state["base"].as_str().unwrap().to_string();
    let started = env.rev(&f.down, "main~1");
    state["start_base"] = started.clone().into();
    fs::write(&path, state.to_string()).unwrap();

    // Other commands show it as left by an earlier release and name finish.
    let status = env.tool_json(&f.down, &["status", "--json"], 0);
    assert_eq!(status["update"]["step"], "waiting_for_merge_request");
    assert_eq!(status["update"]["base"], base.as_str());
    assert_eq!(status["update"]["start_base"], started.as_str());
    // The earlier release's queued upstreams are not read.
    assert_eq!(status["update"]["upstreams"], json!([]));
    assert!(
        status["update"]["next"]
            .as_str()
            .unwrap()
            .contains("`git intake finish` hands the update over")
    );
    let err = stderr(&f.tool(&["continue"]).code(2));
    assert!(err.contains("as an earlier release left it"), "{err}");
    let err = stderr(&f.tool(&["sync"]).code(2));
    assert!(
        err.contains("`git intake finish` hands the update over"),
        "{err}"
    );

    f.forge().clear_requests();
    let out = stdout(&f.tool(&["finish"]).success());
    assert!(out.contains("merge request !1"), "{out}");
    assert!(
        out.contains("which is open; it carries the update from now on"),
        "{out}"
    );
    env.assert_no_update(&f.down);
    assert_eq!(branch_tip(&f.publish, MERGE_REQUEST_BRANCH), Some(head));
    assert!(f.forge().changes().is_empty());
    // `sync` finds it open and waits for it.
    let out = stdout(&f.tool(&["sync"]).success());
    assert!(
        out.contains("the update waits for the merge request !1"),
        "{out}"
    );
}

#[test]
fn finish_hands_over_an_update_whose_merge_request_was_merged() {
    let f = Fork::merge_request();
    let env = &f.env;
    behind(&f);
    f.tool(&["start"]).success();
    f.tool(&["verify"]).success();
    // Format 4 named its branch after the verified commit and recorded no
    // number.
    let head = env.rev(&env.update_worktree(&f.down), "HEAD");
    let branch = format!("git-intake/update-{}", &head[..12]);
    waiting(&f, &branch, None, 4);
    let n = f.forge().add(&branch, "main", "");
    merge_on_fake(env, f.forge(), n, How::MergeCommit);
    let out = stdout(&f.tool(&["finish"]).success());
    assert!(out.contains("which is merged"), "{out}");
    env.assert_no_update(&f.down);
    // The next update starts from the publish remote's integration branch,
    // which holds the merged update.
    let main = env.rev(&f.down, "main");
    f.tool(&["start"])
        .success()
        .stdout(predicates::str::contains(
            "every selected upstream is current",
        ));
    assert!(env.is_ancestor(&f.publish, &head, "main"));
    assert_eq!(env.rev(&f.down, "main"), main);
}

#[test]
fn withdrawing_a_waiting_update_closes_its_merge_request() {
    let f = Fork::merge_request();
    let env = &f.env;
    behind(&f);
    f.tool(&["start"]).success();
    waiting(&f, MERGE_REQUEST_BRANCH, Some(1), 5);
    f.forge().add(MERGE_REQUEST_BRANCH, "main", "");
    let out = stdout(&f.tool(&["abort", "--merge-request"]).success());
    assert!(out.contains("closed merge request !1"), "{out}");
    assert_eq!(branch_tip(&f.publish, MERGE_REQUEST_BRANCH), None);
    env.assert_no_update(&f.down);
}

#[test]
fn aborting_a_waiting_update_names_its_merged_merge_request() {
    let f = Fork::merge_request();
    let env = &f.env;
    behind(&f);
    f.tool(&["start"]).success();
    waiting(&f, MERGE_REQUEST_BRANCH, Some(1), 5);
    f.forge().add(MERGE_REQUEST_BRANCH, "main", "");
    merge_on_fake(env, f.forge(), 1, How::MergeCommit);
    let out = stdout(&f.tool(&["abort"]).success());
    assert!(out.contains("update aborted"), "{out}");
    assert!(out.contains("merge request !1"), "{out}");
    assert!(out.contains("is merged"), "{out}");
    env.assert_no_update(&f.down);
}

#[test]
fn finish_hands_over_a_waiting_update_while_the_checks_are_plain_commands() {
    let f = Fork::merge_request();
    let env = &f.env;
    behind(&f);
    f.tool(&["start"]).success();
    f.tool(&["verify"]).success();
    let head = waiting(&f, MERGE_REQUEST_BRANCH, Some(1), 5);
    f.forge().add(MERGE_REQUEST_BRANCH, "main", "");
    // The integration tip still lists its checks as plain commands, as the
    // earlier release reads them.
    f.plain_checks();
    let path = env.intake_dir(&f.down).join("state.json");
    let recorded = fs::read_to_string(&path).unwrap();

    // Other commands refuse, and the state file stays as it was.
    for args in [
        &["continue"][..],
        &["verify"],
        &["sync"],
        &["start"],
        &["status"],
    ] {
        f.tool(args).code(2);
        assert_eq!(fs::read_to_string(&path).unwrap(), recorded, "{args:?}");
    }
    let err = stderr(&f.tool(&["status"]).code(2));
    assert!(
        err.contains("write `checks = [{ name = \"...\", run = \"test -f app.txt\" }]`"),
        "{err}"
    );

    let out = stdout(&f.tool(&["finish"]).success());
    assert!(
        out.contains("which is open; it carries the update from now on"),
        "{out}"
    );
    env.assert_no_update(&f.down);
    assert!(!env.intake_dir(&f.down).join("logs").exists());
    assert_eq!(branch_tip(&f.publish, MERGE_REQUEST_BRANCH), Some(head));
}

#[test]
fn abort_withdraws_a_waiting_update_while_the_checks_are_plain_commands() {
    let f = Fork::merge_request();
    let env = &f.env;
    behind(&f);
    f.tool(&["start"]).success();
    waiting(&f, MERGE_REQUEST_BRANCH, Some(1), 5);
    f.forge().add(MERGE_REQUEST_BRANCH, "main", "");
    f.plain_checks();
    let path = env.intake_dir(&f.down).join("state.json");
    let recorded = fs::read_to_string(&path).unwrap();
    f.tool(&["report"]).code(2);
    assert_eq!(fs::read_to_string(&path).unwrap(), recorded);

    let out = stdout(&f.tool(&["abort", "--merge-request"]).success());
    assert!(out.contains("update aborted"), "{out}");
    assert!(out.contains("closed merge request !1"), "{out}");
    assert_eq!(branch_tip(&f.publish, MERGE_REQUEST_BRANCH), None);
    env.assert_no_update(&f.down);
}
