//! Publication through a merge request: the verified commit is pushed to a
//! branch of its own, the update waits for the merge request, and a later
//! `finish` completes it once the publish remote's integration branch holds
//! the verified commit.

mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use common::{Fork, MERGE_REQUEST_NUMBER, recorded_push_options, stderr, stdout};
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
            "asked 'origin' to open a merge request from '{branch}' into 'main'"
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
    assert_eq!(update["merge_request"]["number"], MERGE_REQUEST_NUMBER);
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
        text.contains(&format!("merge request !7: {branch} into main on origin")),
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
    assert!(out.contains("received the verified commit"), "{out}");
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

    // A squash with GitLab's default message, the merge request title: one
    // new commit with the verified tree and no sync record.
    let forge = forge(&f);
    env.git(&forge, &["fetch", "-q", "origin"]);
    env.git(
        &forge,
        &["merge", "-q", "--squash", &format!("origin/{branch}")],
    );
    env.git(
        &forge,
        &["commit", "-q", "-m", "Merge upstream update: upstream"],
    );
    push_forge_main(&f, &forge);
    let squashed = env.rev(&f.publish, "main");

    let refused = f.tool(&["finish"]).code(2);
    let err = stderr(&refused);
    assert!(
        err.contains("holds the update's changes but not the verified commit"),
        "{err}"
    );
    assert!(
        err.contains("the merge request was merged by squash or rebase"),
        "{err}"
    );
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
                "unless a merge request from '{branch}' into 'main' is open on 'origin', open one"
            ))
    );
    let text = stdout(&f.tool(&["status"]).success());
    assert!(text.contains("open it unless it is open"), "{text}");
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

/// The forge merges the merge request branch with a merge commit.
fn merge_on_forge(f: &Fork, branch: &str) -> String {
    let env = &f.env;
    let forge = forge(f);
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
    push_forge_main(f, &forge);
    env.rev(&f.publish, "main")
}

#[test]
fn merge_request_branch_changed_before_the_merge_is_refused() {
    let f = Fork::merge_request();
    let env = &f.env;
    let base = env.rev(&f.down, "main");
    let (commit, _) = verified(&f);
    let branch = mr_branch(&commit);
    f.tool(&["finish"]).success();

    // A reviewer adds a commit to the merge request branch.
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
    let extra = env.commit(&forge, &[("review.txt", "r\n")], "review: unverified");
    env.git(&forge, &["push", "-q", "origin", &format!("HEAD:{branch}")]);
    env.git(&forge, &["checkout", "-q", "main"]);
    let waiting = stdout(&f.tool(&["finish"]).code(1));
    assert!(
        waiting.contains(&format!(
            "no longer points at the verified commit {} but at {}",
            &commit[..12],
            &extra[..12]
        )),
        "{waiting}"
    );

    let merged = merge_on_forge(&f, &branch);
    let err = stderr(&f.tool(&["finish"]).code(2));
    assert!(
        err.contains(&format!(
            "the merge request's head is {}, not the verified commit",
            &extra[..12]
        )),
        "{err}"
    );
    assert_eq!(env.rev(&f.down, "main"), base);
    assert!(!env.has_rev(&f.down, "refs/heads/upstream-main"));
    assert!(!env.has_rev(&f.publish, "refs/heads/upstream-main"));

    let out = stdout(&f.tool(&["abort"]).success());
    assert!(
        out.contains(&format!(
            "'origin/main' at {} holds an unverified merge",
            &merged[..12]
        )),
        "{out}"
    );
    assert!(
        out.contains("update aborted; 'main' and 'upstream-main' are unchanged"),
        "{out}"
    );
    assert!(!out.contains("close the merge request"), "{out}");
    // The branch moved beyond the verified commit, so it is left alone.
    assert!(out.contains("left the merge request branch"), "{out}");
    assert!(!env.has_rev(&f.down, "refs/heads/upstream-main"));
    assert_eq!(env.rev(&f.down, "main"), base);
    env.assert_no_update(&f.down);
}

#[test]
fn merged_after_the_integration_branch_moved_is_refused() {
    let f = Fork::merge_request();
    let env = &f.env;
    let base = env.rev(&f.down, "main");
    let (commit, _) = verified(&f);
    let branch = mr_branch(&commit);
    f.tool(&["finish"]).success();

    // Other work lands first; the forge still merges with a merge commit.
    let forge = forge(&f);
    env.commit(&forge, &[("other.txt", "o\n")], "other: lands first");
    push_forge_main(&f, &forge);
    merge_on_forge(&f, &branch);

    let err = stderr(&f.tool(&["finish"]).code(2));
    assert!(err.contains("differs from the verified tree"), "{err}");
    assert_eq!(env.rev(&f.down, "main"), base);
    assert!(!env.has_rev(&f.down, "refs/heads/upstream-main"));
    assert!(!env.has_rev(&f.publish, "refs/heads/upstream-main"));
}

#[test]
fn abort_after_a_valid_merge_is_refused() {
    let f = Fork::merge_request();
    let env = &f.env;
    let (commit, target) = verified(&f);
    let branch = mr_branch(&commit);
    f.tool(&["finish"]).success();
    merge_on_forge(&f, &branch);

    f.tool(&["abort"])
        .code(2)
        .stderr(contains("was merged with the verified commit"))
        .stderr(contains("Run `git intake finish` to complete it"));
    assert!(env.has_rev(&f.publish, &format!("refs/heads/{branch}")));
    f.tool(&["finish"]).success();
    assert_eq!(env.rev(&f.down, "upstream-main"), target);
    env.assert_no_update(&f.down);
}

#[test]
fn interrupted_opening_does_not_claim_a_merge_request() {
    let f = Fork::merge_request();
    let env = &f.env;
    let (commit, _) = verified(&f);
    let branch = mr_branch(&commit);
    // An earlier `finish` pushed the branch and stopped before saving the state.
    env.git(
        &f.down,
        &[
            "push",
            "-q",
            "origin",
            &format!("{commit}:refs/heads/{branch}"),
        ],
    );

    f.tool(&["finish"]).success().stdout(contains(format!(
        "'{branch}' was already on 'origin', so this push asked for no merge request; \
         if none is open, open one from '{branch}' into 'main'"
    )));
    let status = env.tool_json(&f.down, &["status", "--json"], 0);
    assert_eq!(status["update"]["step"], "waiting_for_merge_request");
    assert_eq!(status["update"]["merge_request"]["requested"], false);
    f.tool(&["finish"])
        .code(1)
        .stdout(contains("open the merge request if it is not open yet"));
}

/// Where the remote keeps the merge request's head, as GitLab does.
fn set_merge_request_head(f: &Fork, commit: &str) {
    f.env.git(
        &f.publish,
        &[
            "update-ref",
            &format!("refs/merge-requests/{MERGE_REQUEST_NUMBER}/head"),
            commit,
        ],
    );
}

/// The forge's reviewer adds a commit to the merge request branch; returns it.
fn add_to_merge_request_branch(f: &Fork, branch: &str) -> String {
    let env = &f.env;
    let forge = forge(f);
    env.git(&forge, &["fetch", "-q", "origin"]);
    env.git(
        &forge,
        &[
            "checkout",
            "-q",
            "-B",
            "review",
            &format!("origin/{branch}"),
        ],
    );
    let extra = env.commit(&forge, &[("review.txt", "r\n")], "review: unverified");
    env.git(&forge, &["push", "-q", "origin", &format!("HEAD:{branch}")]);
    env.git(&forge, &["checkout", "-q", "main"]);
    extra
}

/// A commit landed on the remote's integration branch after `commit`.
fn land_on_top(f: &Fork, commit: &str) -> String {
    let env = &f.env;
    env.git(&f.publish, &["update-ref", "refs/heads/main", commit]);
    let forge = forge(f);
    let later = env.commit(&forge, &[("later.txt", "l\n")], "later: other work");
    push_forge_main(f, &forge);
    later
}

#[test]
fn fast_forward_of_a_changed_merge_request_is_refused_by_its_head() {
    let f = Fork::merge_request();
    let env = &f.env;
    let base = env.rev(&f.down, "main");
    let (commit, _) = verified(&f);
    let branch = mr_branch(&commit);
    f.tool(&["finish"])
        .success()
        .stdout(contains("'origin' opened merge request !7"));
    set_merge_request_head(&f, &commit);

    // A commit lands on the merge request branch; the remote moves the
    // merge request's head with it, merges by fast-forward and removes the
    // branch.
    let extra = add_to_merge_request_branch(&f, &branch);
    set_merge_request_head(&f, &extra);
    env.git(&f.publish, &["update-ref", "refs/heads/main", &extra]);
    env.git(
        &f.publish,
        &["update-ref", "-d", &format!("refs/heads/{branch}")],
    );

    let err = stderr(&f.tool(&["finish"]).code(2));
    assert!(
        err.contains(&format!(
            "the merge request's head is {}, not the verified commit",
            &extra[..12]
        )),
        "{err}"
    );
    assert_eq!(env.rev(&f.down, "main"), base);
    assert!(!env.has_rev(&f.down, "refs/heads/upstream-main"));
    assert!(!env.has_rev(&f.publish, "refs/heads/upstream-main"));
    f.tool(&["abort"]).success().stdout(contains(format!(
        "holds an unverified merge: the merge request's head {} landed instead of the verified commit",
        &extra[..12]
    )));
    assert!(!env.has_rev(&f.down, "refs/heads/upstream-main"));
    env.assert_no_update(&f.down);
}

#[test]
fn fast_forward_of_a_changed_branch_is_refused_without_the_merge_request_head() {
    let f = Fork::merge_request();
    let env = &f.env;
    let base = env.rev(&f.down, "main");
    let (commit, _) = verified(&f);
    let branch = mr_branch(&commit);
    f.tool(&["finish"]).success();

    // The remote keeps no merge request head; the branch is still listed.
    let extra = add_to_merge_request_branch(&f, &branch);
    env.git(&f.publish, &["update-ref", "refs/heads/main", &extra]);

    let err = stderr(&f.tool(&["finish"]).code(2));
    assert!(
        err.contains(&format!(
            "the merge request's head is {}, not the verified commit",
            &extra[..12]
        )),
        "{err}"
    );
    assert_eq!(env.rev(&f.down, "main"), base);
    assert!(!env.has_rev(&f.down, "refs/heads/upstream-main"));
}

#[test]
fn branch_as_last_seen_judges_once_it_is_gone() {
    let f = Fork::merge_request();
    let env = &f.env;
    let (commit, _) = verified(&f);
    let branch = mr_branch(&commit);
    f.tool(&["finish"]).success();

    let extra = add_to_merge_request_branch(&f, &branch);
    // `finish` sees the changed branch while waiting.
    f.tool(&["finish"]).code(1).stdout(contains(
        "the merge request branch no longer points at the verified commit",
    ));
    env.git(&f.publish, &["update-ref", "refs/heads/main", &extra]);
    env.git(
        &f.publish,
        &["update-ref", "-d", &format!("refs/heads/{branch}")],
    );
    f.tool(&["finish"]).code(2).stderr(contains(format!(
        "the merge request's head is {}, not the verified commit",
        &extra[..12]
    )));
}

#[test]
fn fast_forward_with_later_work_completes_when_the_head_is_verified() {
    let f = Fork::merge_request();
    let env = &f.env;
    let (commit, target) = verified(&f);
    let branch = mr_branch(&commit);
    f.tool(&["finish"]).success();
    set_merge_request_head(&f, &commit);
    let later = land_on_top(&f, &commit);
    env.git(
        &f.publish,
        &["update-ref", "-d", &format!("refs/heads/{branch}")],
    );

    let out = stdout(&f.tool(&["finish"]).success());
    assert!(!out.contains("could not be attributed"), "{out}");
    assert_eq!(env.rev(&f.down, "main"), later);
    assert_eq!(env.rev(&f.publish, "upstream-main"), target);
    env.assert_no_update(&f.down);
}

#[test]
fn without_a_merge_request_number_later_work_is_reported_as_unattributed() {
    let f = Fork::merge_request();
    let env = &f.env;
    // A remote without push options names no merge request.
    env.git(
        &f.publish,
        &["config", "receive.advertisePushOptions", "false"],
    );
    let (commit, _) = verified(&f);
    let branch = mr_branch(&commit);
    f.tool(&["finish"]).success();
    let status = env.tool_json(&f.down, &["status", "--json"], 0);
    assert_eq!(
        status["update"]["merge_request"]["number"],
        serde_json::Value::Null
    );
    let later = land_on_top(&f, &commit);
    env.git(
        &f.publish,
        &["update-ref", "-d", &format!("refs/heads/{branch}")],
    );

    let out = stdout(&f.tool(&["finish"]).success());
    assert!(
        out.contains("1 commit above the verified commit on 'origin/main' could not be attributed"),
        "{out}"
    );
    assert!(out.contains("later: other work"), "{out}");
    assert_eq!(env.rev(&f.down, "main"), later);
    env.assert_no_update(&f.down);
}

#[test]
fn missing_merge_request_head_falls_back_to_the_branch() {
    let f = Fork::merge_request();
    let env = &f.env;
    let (commit, _) = verified(&f);
    let branch = mr_branch(&commit);
    // The number is known, but the remote keeps no head for it.
    f.tool(&["finish"])
        .success()
        .stdout(contains("opened merge request !7"));
    let later = land_on_top(&f, &commit);
    env.git(
        &f.publish,
        &["update-ref", "-d", &format!("refs/heads/{branch}")],
    );
    f.tool(&["finish"])
        .success()
        .stdout(contains("could not be attributed to the merge request"))
        .stdout(contains("later: other work"));
    assert_eq!(env.rev(&f.down, "main"), later);
    env.assert_no_update(&f.down);
}
