//! Publication through a merge request: the verified commit is pushed to a
//! branch of its own, the update waits for the merge request, and a later
//! `finish` completes it once the publish remote's integration branch holds
//! the verified commit.

mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use common::{Fork, recorded_push_options, stderr, stdout};
use predicates::str::contains;

/// A fork in merge-request mode with one verified update. Returns the
/// verified commit and the upstream target.
fn verified(f: &Fork) -> (String, String) {
    let env = &f.env;
    let target = env.push_upstream(&f.up, &[("n.txt", "n\n")], "upstream: n");
    f.tool(&["start"]).success();
    f.tool(&["verify"]).success();
    let commit = env.rev(&env.update_worktree(&f.down), "HEAD");
    (commit, target)
}

fn mr_branch(commit: &str) -> String {
    format!("git-intake/update-{}", &commit[..12])
}

/// A clone of the publish remote that plays the forge merging the request.
fn forge(f: &Fork) -> PathBuf {
    let env = &f.env;
    let dir = env.path("forge");
    if !dir.exists() {
        env.git(
            &env.path(""),
            &[
                "clone",
                "-q",
                &f.publish.to_string_lossy(),
                &dir.to_string_lossy(),
            ],
        );
    } else {
        env.git(&dir, &["fetch", "-q", "origin"]);
        env.git(&dir, &["reset", "-q", "--hard", "origin/main"]);
    }
    dir
}

fn push_forge_main(f: &Fork, forge: &Path) {
    f.env.git(forge, &["push", "-q", "origin", "HEAD:main"]);
}

#[test]
fn merge_request_opened_waits_and_completes_after_a_merge_commit() {
    let f = Fork::merge_request();
    let env = &f.env;
    let base = env.rev(&f.down, "main");
    let published = env.rev(&f.publish, "main");
    let (commit, target) = verified(&f);
    let branch = mr_branch(&commit);

    let open = f.tool(&["finish"]).success();
    let out = stdout(&open);
    assert!(
        out.contains(&format!(
            "requested a merge request from '{branch}' into 'main' on 'origin'"
        )),
        "{out}"
    );
    assert!(
        out.contains("no local branch moved; the update waits for the merge request"),
        "{out}"
    );
    // The branch holds the verified commit; the merge request was requested.
    assert_eq!(env.rev(&f.publish, &format!("refs/heads/{branch}")), commit);
    let options = recorded_push_options(&f.publish);
    assert_eq!(options[0], "merge_request.create");
    assert!(options.contains(&"merge_request.target=main".to_string()));
    assert!(options.contains(&"merge_request.title=Merge upstream update: upstream".to_string()));
    let description = options
        .iter()
        .find(|o| o.starts_with("merge_request.description="))
        .expect("description");
    assert!(
        description.contains(&format!("..{} (1 commit)", &target[..12])),
        "{description}"
    );
    assert!(description.contains(&commit), "{description}");
    assert!(options.contains(&"merge_request.remove_source_branch".to_string()));
    // Nothing moved, locally or on the remote's integration branch.
    assert_eq!(env.rev(&f.down, "main"), base);
    assert!(!env.has_rev(&f.down, "refs/heads/upstream-main"));
    assert_eq!(env.rev(&f.publish, "main"), published);
    assert!(!env.has_rev(&f.publish, "refs/heads/upstream-main"));
    assert!(!env.update_worktree(&f.down).exists());

    // Every command sees the waiting step.
    let status = env.tool_json(&f.down, &["status", "--json"], 0);
    assert_eq!(status["publish_via"], "merge_request");
    let update = &status["update"];
    assert_eq!(update["step"], "waiting_for_merge_request");
    assert_eq!(update["merge_request"]["source_branch"], branch.as_str());
    assert_eq!(update["merge_request"]["target_branch"], "main");
    assert_eq!(update["merge_request"]["commit"], commit.as_str());
    assert_eq!(update["merge_request"]["requested"], true);
    assert!(
        update["next"]
            .as_str()
            .unwrap()
            .contains("then run `git intake finish`")
    );
    let text = stdout(&f.tool(&["status"]).success());
    assert!(
        text.contains("publish: origin (via merge request)"),
        "{text}"
    );
    assert!(
        text.contains("update in progress: waiting for merge request"),
        "{text}"
    );
    assert!(
        text.contains(&format!("merge request: {branch} into main on origin")),
        "{text}"
    );
    f.tool(&["start"])
        .code(2)
        .stderr(contains("waits for its merge request"));
    f.tool(&["continue"])
        .code(2)
        .stderr(contains("waits for its merge request"));
    f.tool(&["verify"])
        .code(2)
        .stderr(contains("waits for its merge request"));
    let doctor = env.tool_json(&f.down, &["doctor", "--json"], 0);
    assert_eq!(doctor["healthy"], true);

    // Not merged yet: the update waits and nothing changes.
    let waiting = f.tool(&["finish"]).code(1);
    assert!(
        stdout(&waiting).contains(&format!(
            "the update waits for the merge request from '{branch}' into 'main' on 'origin'"
        )),
        "{}",
        stdout(&waiting)
    );
    assert_eq!(env.rev(&f.down, "main"), base);
    assert!(!env.has_rev(&f.down, "refs/heads/upstream-main"));

    // The forge merges the request with a merge commit and keeps the branch.
    let forge = forge(&f);
    env.git(&forge, &["fetch", "-q", "origin"]);
    env.git(
        &forge,
        &[
            "merge",
            "-q",
            "--no-ff",
            "-m",
            "Merge branch into main",
            &format!("origin/{branch}"),
        ],
    );
    push_forge_main(&f, &forge);
    let merged = env.rev(&f.publish, "main");

    let done = f.tool(&["finish"]).success();
    let out = stdout(&done);
    assert!(out.contains("contains the verified commit"), "{out}");
    assert!(out.contains("pushed upstream-main to 'origin'"), "{out}");
    assert!(
        out.contains(&format!(
            "deleted the merge request branch '{branch}' from 'origin'"
        )),
        "{out}"
    );
    assert_eq!(env.rev(&f.down, "main"), merged);
    assert!(env.is_ancestor(&f.down, &commit, "main"));
    assert_eq!(env.rev(&f.down, "upstream-main"), target);
    assert_eq!(env.rev(&f.publish, "upstream-main"), target);
    assert_eq!(env.rev(&f.publish, "main"), merged);
    assert!(!env.has_rev(&f.publish, &format!("refs/heads/{branch}")));
    // The main checkout followed the fast-forward.
    assert!(f.down.join("n.txt").exists());
    env.assert_no_update(&f.down);
    let report = env.tool_json(&f.down, &["report", "--json"], 0);
    assert_eq!(report["in_progress"], false);
    assert_eq!(report["unpublished"], serde_json::json!([]));
}

#[test]
fn fast_forward_merge_with_the_branch_already_removed_completes() {
    let f = Fork::merge_request();
    let env = &f.env;
    let (commit, target) = verified(&f);
    let branch = mr_branch(&commit);
    f.tool(&["finish"]).success();

    // The forge fast-forwards main and removes the source branch.
    env.git(&f.publish, &["update-ref", "refs/heads/main", &commit]);
    env.git(
        &f.publish,
        &["update-ref", "-d", &format!("refs/heads/{branch}")],
    );

    f.tool(&["finish"])
        .success()
        .stdout(contains(format!(
            "the merge request branch '{branch}' is already gone from 'origin'"
        )))
        .stdout(contains("update finished"));
    assert_eq!(env.rev(&f.down, "main"), commit);
    assert_eq!(env.rev(&f.publish, "upstream-main"), target);
    env.assert_no_update(&f.down);
}

#[test]
fn squash_merge_is_refused_and_moves_nothing() {
    let f = Fork::merge_request();
    let env = &f.env;
    let base = env.rev(&f.down, "main");
    let (commit, _) = verified(&f);
    let branch = mr_branch(&commit);
    f.tool(&["finish"]).success();

    // A squash: one new commit with the verified tree whose message joins
    // the messages of the squashed commits, sync record included.
    let forge = forge(&f);
    env.git(&forge, &["fetch", "-q", "origin"]);
    env.git(
        &forge,
        &["merge", "-q", "--squash", &format!("origin/{branch}")],
    );
    let message = env.git(
        &forge,
        &["log", "-1", "--format=%B", &format!("origin/{branch}")],
    );
    env.git(
        &forge,
        &[
            "commit",
            "-q",
            "-m",
            &format!("Squashed update\n\n{message}"),
        ],
    );
    push_forge_main(&f, &forge);
    let squashed = env.rev(&f.publish, "main");

    let refused = f.tool(&["finish"]).code(2);
    let err = stderr(&refused);
    assert!(
        err.contains("carries the sync records of this update (upstream)"),
        "{err}"
    );
    assert!(err.contains("merged by squash or rebase"), "{err}");
    assert!(
        err.contains("with a merge commit or by fast-forward"),
        "{err}"
    );
    assert_eq!(env.rev(&f.down, "main"), base);
    assert!(!env.has_rev(&f.down, "refs/heads/upstream-main"));
    assert!(!env.has_rev(&f.publish, "refs/heads/upstream-main"));
    assert_eq!(env.rev(&f.publish, "main"), squashed);
    let status = env.tool_json(&f.down, &["status", "--json"], 0);
    assert_eq!(status["update"]["step"], "waiting_for_merge_request");
}

#[test]
fn integration_moved_before_the_merge_is_refused_and_abort_withdraws() {
    let f = Fork::merge_request();
    let env = &f.env;
    let base = env.rev(&f.down, "main");
    let (commit, _) = verified(&f);
    let branch = mr_branch(&commit);
    f.tool(&["finish"]).success();
    assert!(env.intake_dir(&f.down).join("logs").exists());

    // Someone else's work lands on main before the merge request.
    let forge = forge(&f);
    env.commit(&forge, &[("other.txt", "o\n")], "other: lands first");
    push_forge_main(&f, &forge);

    let refused = f.tool(&["finish"]).code(2);
    let err = stderr(&refused);
    assert!(
        err.contains("no longer contains its tip and the merge request cannot be merged as it is"),
        "{err}"
    );
    assert!(err.contains("Run `git intake abort`"), "{err}");
    assert!(err.contains("`git intake start`"), "{err}");
    assert_eq!(env.rev(&f.down, "main"), base);

    let abort = f.tool(&["abort"]).success();
    let out = stdout(&abort);
    assert!(
        out.contains(&format!(
            "deleted the merge request branch '{branch}' from 'origin'"
        )),
        "{out}"
    );
    assert!(
        out.contains(&format!(
            "close the merge request from '{branch}' into 'main' on 'origin'"
        )),
        "{out}"
    );
    assert!(
        out.contains("update aborted; 'main' and 'upstream-main' are unchanged"),
        "{out}"
    );
    assert!(!env.has_rev(&f.publish, &format!("refs/heads/{branch}")));
    assert!(!env.intake_dir(&f.down).join("logs").exists());
    env.assert_no_update(&f.down);
    assert_eq!(env.rev(&f.down, "main"), base);
}

#[test]
fn abort_while_waiting_ignores_a_branch_already_gone_and_keeps_one_that_moved() {
    let f = Fork::merge_request();
    let env = &f.env;
    let (commit, _) = verified(&f);
    let branch = mr_branch(&commit);
    f.tool(&["finish"]).success();
    env.git(
        &f.publish,
        &["update-ref", "-d", &format!("refs/heads/{branch}")],
    );
    f.tool(&["abort"])
        .success()
        .stdout(contains(format!(
            "the merge request branch '{branch}' is already gone from 'origin'"
        )))
        .stdout(contains("close the merge request"));
    env.assert_no_update(&f.down);

    // A second attempt whose merge request branch received a commit.
    f.tool(&["start"]).success();
    f.tool(&["verify"]).success();
    let commit = env.rev(&env.update_worktree(&f.down), "HEAD");
    let branch = mr_branch(&commit);
    f.tool(&["finish"]).success();
    let forge = forge(&f);
    env.git(&forge, &["fetch", "-q", "origin"]);
    env.git(
        &forge,
        &[
            "checkout",
            "-q",
            "-b",
            "review",
            &format!("origin/{branch}"),
        ],
    );
    let reviewed = env.commit(&forge, &[("review.txt", "r\n")], "review: fix");
    env.git(&forge, &["push", "-q", "origin", &format!("HEAD:{branch}")]);
    f.tool(&["abort"]).success().stdout(contains(format!(
        "left the merge request branch '{branch}' on 'origin'"
    )));
    assert_eq!(
        env.rev(&f.publish, &format!("refs/heads/{branch}")),
        reviewed
    );
    env.assert_no_update(&f.down);
}

#[test]
fn remote_without_push_options_gets_the_branch_and_names_the_merge_request() {
    let f = Fork::merge_request();
    let env = &f.env;
    // This remote refuses push options.
    env.git(
        &f.publish,
        &["config", "receive.advertisePushOptions", "false"],
    );
    let base = env.rev(&f.down, "main");
    let (commit, _) = verified(&f);
    let branch = mr_branch(&commit);

    f.tool(&["finish"])
        .success()
        .stdout(contains(format!(
            "'origin' does not accept push options, so no merge request was opened; open one from '{branch}' into 'main'"
        )));
    assert_eq!(env.rev(&f.publish, &format!("refs/heads/{branch}")), commit);
    assert!(recorded_push_options(&f.publish).is_empty());
    assert_eq!(env.rev(&f.down, "main"), base);
    let status = env.tool_json(&f.down, &["status", "--json"], 0);
    assert_eq!(status["update"]["merge_request"]["requested"], false);
    assert!(
        status["update"]["next"]
            .as_str()
            .unwrap()
            .starts_with(&format!(
                "open a merge request from '{branch}' into 'main' on 'origin'"
            ))
    );
    let text = stdout(&f.tool(&["status"]).success());
    assert!(text.contains("open it by hand"), "{text}");
    f.tool(&["abort"]).success().stdout(contains(format!(
        "if a merge request from '{branch}' into 'main' was opened on 'origin', close it"
    )));
}

#[test]
fn no_publish_keeps_its_meaning_in_merge_request_mode() {
    // Before the merge request: local moves only, nothing pushed.
    let f = Fork::merge_request();
    let env = &f.env;
    let published = env.rev(&f.publish, "main");
    let (commit, target) = verified(&f);
    f.tool(&["finish", "--no-publish"])
        .success()
        .stdout(contains(
            "not published to 'origin' (advanced locally only): main, upstream-main",
        ));
    assert_eq!(env.rev(&f.down, "main"), commit);
    assert_eq!(env.rev(&f.down, "upstream-main"), target);
    assert_eq!(env.rev(&f.publish, "main"), published);
    assert!(!env.has_rev(&f.publish, &format!("refs/heads/{}", mr_branch(&commit))));
    env.assert_no_update(&f.down);

    // While waiting: no branch moves before the merge.
    let f = Fork::merge_request();
    let env = &f.env;
    let base = env.rev(&f.down, "main");
    let (commit, target) = verified(&f);
    let branch = mr_branch(&commit);
    f.tool(&["finish"]).success();
    f.tool(&["finish", "--no-publish"]).code(1).stdout(contains(
        "`--no-publish` moves no branch before the merge request is merged",
    ));
    assert_eq!(env.rev(&f.down, "main"), base);
    assert!(!env.has_rev(&f.down, "refs/heads/upstream-main"));

    // After the merge: local moves only; the mirror and the branch stay as they are on the remote.
    env.git(&f.publish, &["update-ref", "refs/heads/main", &commit]);
    f.tool(&["finish", "--no-publish"])
        .success()
        .stdout(contains(
            "not published to 'origin' (advanced locally only): upstream-main",
        ));
    assert_eq!(env.rev(&f.down, "main"), commit);
    assert_eq!(env.rev(&f.down, "upstream-main"), target);
    assert!(!env.has_rev(&f.publish, "refs/heads/upstream-main"));
    assert_eq!(env.rev(&f.publish, &format!("refs/heads/{branch}")), commit);
    env.assert_no_update(&f.down);
    let report = env.tool_json(&f.down, &["report", "--json"], 0);
    assert_eq!(report["unpublished"], serde_json::json!(["upstream-main"]));
}

#[test]
fn rejected_mirror_push_after_the_merge_resumes() {
    let f = Fork::merge_request();
    let env = &f.env;
    let (commit, target) = verified(&f);
    f.tool(&["finish"]).success();
    env.git(&f.publish, &["update-ref", "refs/heads/main", &commit]);

    // The mirror branch is protected on the remote.
    let hook = f.publish.join("hooks").join("update");
    fs::write(
        &hook,
        "#!/bin/sh\n[ \"$1\" = refs/heads/upstream-main ] && { echo 'protected branch' >&2; exit 1; }\nexit 0\n",
    )
    .unwrap();
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();

    let err = stderr(&f.tool(&["finish"]).code(2));
    assert!(
        err.contains("pushing them to 'origin' was rejected"),
        "{err}"
    );
    assert_eq!(env.rev(&f.down, "main"), commit);
    assert_eq!(env.rev(&f.down, "upstream-main"), target);
    let status = env.tool_json(&f.down, &["status", "--json"], 0);
    assert_eq!(status["update"]["step"], "finishing");
    assert!(status["update"]["publish_error"].is_string());
    f.tool(&["abort"])
        .code(2)
        .stderr(contains("can no longer be aborted"));

    fs::remove_file(&hook).unwrap();
    f.tool(&["finish"])
        .success()
        .stdout(contains("resuming finish"))
        .stdout(contains("pushed upstream-main to 'origin'"));
    assert_eq!(env.rev(&f.publish, "upstream-main"), target);
    env.assert_no_update(&f.down);
}

#[test]
fn merge_request_closed_without_merging_is_refused() {
    let f = Fork::merge_request();
    let env = &f.env;
    let base = env.rev(&f.down, "main");
    let (commit, _) = verified(&f);
    let branch = mr_branch(&commit);
    f.tool(&["finish"]).success();
    env.git(
        &f.publish,
        &["update-ref", "-d", &format!("refs/heads/{branch}")],
    );
    f.tool(&["finish"])
        .code(2)
        .stderr(contains("the merge request was closed without merging"));
    assert_eq!(env.rev(&f.down, "main"), base);
    f.tool(&["abort"]).success();
    env.assert_no_update(&f.down);
}
