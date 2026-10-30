//! Publication through a merge request: the verified commit is pushed to a
//! branch of its own, the update waits for the merge request, and a later
//! `finish` completes it once the publish remote's integration branch holds
//! the verified commit.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::{
    Fork, MERGE_REQUEST_BRANCH, MERGE_REQUEST_NUMBER, accept_push_options, recorded_push_options,
    stderr, stdout,
};
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

/// A clone of the publish remote that plays the forge merging the request.
fn forge(f: &Fork) -> PathBuf {
    common::forge(&f.env, &f.publish)
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
    let branch = MERGE_REQUEST_BRANCH.to_string();

    // Opening the merge request is a success: the update waits as it should.
    let open = f.tool(&["finish"]).code(0);
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
    // A squash would replace the verified commit, so the merge request is
    // opened with squashing off, whatever the project's default.
    assert!(options.contains(&"merge_request.squash=false".to_string()));
    // Nothing moved, locally or on the remote's integration branch.
    assert_eq!(env.rev(&f.down, "main"), base);
    assert_eq!(env.rev(&f.publish, "main"), published);
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

    // Not merged yet: the update waits and nothing changes. Waiting is not a
    // failure; `status --check` is what reports the upstream as pending.
    let waiting = f.tool(&["finish"]).code(0);
    assert!(
        stdout(&waiting).contains(&format!(
            "the update waits for the merge request from '{branch}' into 'main' on 'origin'"
        )),
        "{}",
        stdout(&waiting)
    );
    assert!(
        stdout(&waiting).contains("then run `git intake finish` again"),
        "{}",
        stdout(&waiting)
    );
    assert_eq!(env.rev(&f.down, "main"), base);
    f.tool(&["status", "--check"]).code(1);

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
    assert!(
        out.contains(&format!(
            "deleted the merge request branch '{branch}' from 'origin'"
        )),
        "{out}"
    );
    assert_eq!(env.rev(&f.down, "main"), merged);
    assert!(env.is_ancestor(&f.down, &commit, "main"));
    assert!(env.is_ancestor(&f.down, &target, "main"));
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
    let (commit, _) = verified(&f);
    let branch = MERGE_REQUEST_BRANCH.to_string();
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
    env.assert_no_update(&f.down);
}

#[test]
fn squash_merge_is_refused_and_moves_nothing() {
    let f = Fork::merge_request();
    let env = &f.env;
    let base = env.rev(&f.down, "main");
    let (commit, _) = verified(&f);
    let branch = MERGE_REQUEST_BRANCH.to_string();
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
    // The branch kept at the verified commit makes no catch-up.
    assert!(
        !stdout(&refused).contains("catching up"),
        "{}",
        stdout(&refused)
    );
    assert_eq!(env.rev(&f.publish, &format!("refs/heads/{branch}")), commit);
    assert!(
        err.contains("the merge request was merged by squash or rebase"),
        "{err}"
    );
    assert!(
        err.contains("with a merge commit or by fast-forward"),
        "{err}"
    );
    // A project that squashes every merge request overrides the request to
    // leave squashing off; the refusal names that setting.
    assert!(
        err.contains(
            "a project whose own or target branch rule's squash setting requires squashing"
        ),
        "{err}"
    );
    assert_eq!(env.rev(&f.down, "main"), base);
    assert_eq!(env.rev(&f.publish, "main"), squashed);
    let status = env.tool_json(&f.down, &["status", "--json"], 0);
    assert_eq!(status["update"]["step"], "waiting_for_merge_request");
}

/// Other work lands on the publish remote's integration branch through the
/// forge; returns it.
fn land(f: &Fork, files: &[(&str, &str)], message: &str) -> String {
    let forge = forge(f);
    let c = f.env.commit(&forge, files, message);
    push_forge_main(f, &forge);
    c
}

fn parents(f: &Fork, rev: &str) -> Vec<String> {
    f.env
        .git(&f.publish, &["rev-list", "--parents", "-n", "1", rev])
        .split_whitespace()
        .skip(1)
        .map(str::to_string)
        .collect()
}

/// The update waits for its merge request, the integration branch moved,
/// and `finish` caught up and pushed the catch-up merge. Returns the
/// verified commit, the upstream target and the catch-up merge.
fn caught_up_while_waiting(f: &Fork) -> (String, String, String) {
    let env = &f.env;
    let base = env.rev(&f.down, "main");
    let (commit, target) = verified(f);
    let branch = MERGE_REQUEST_BRANCH;
    f.tool(&["finish"]).success();
    let other = land(f, &[("other.txt", "o\n")], "other: lands first");

    let out = stdout(&f.tool(&["finish"]).code(0));
    assert!(
        out.contains(&format!(
            "catching up with 'main' at {}: merging the update at {} into it",
            &other[..12],
            &commit[..12]
        )),
        "{out}"
    );
    let caught = env.rev(&f.publish, &format!("refs/heads/{branch}"));
    assert!(
        out.contains(&format!(
            "pushed the verified catch-up merge {} to 'origin' as branch '{branch}'",
            &caught[..12]
        )),
        "{out}"
    );
    assert!(
        out.contains("no local branch moved; the update waits for the merge request"),
        "{out}"
    );
    assert_eq!(parents(f, &caught), [other, commit.clone()]);
    assert!(
        env.git(&f.publish, &["log", "-1", "--format=%B", &caught])
            .contains("Intake-Catch-Up: main")
    );
    // The push brought the open merge request up to date and asked for no
    // second one.
    let options = recorded_push_options(&f.publish);
    assert!(
        !options.contains(&"merge_request.create".to_string()),
        "{options:?}"
    );
    assert!(options.contains(&"merge_request.title=Merge upstream update: upstream".to_string()));
    let description = options
        .iter()
        .find(|o| o.starts_with("merge_request.description="))
        .expect("description");
    assert!(description.contains(&caught), "{description}");
    assert!(
        description.contains(&format!("..{} (1 commit)", &target[..12])),
        "{description}"
    );
    assert_eq!(env.rev(&f.down, "main"), base);
    assert!(!env.update_worktree(&f.down).exists());
    let status = env.tool_json(&f.down, &["status", "--json"], 0);
    let update = &status["update"];
    assert_eq!(update["step"], "waiting_for_merge_request");
    assert_eq!(update["merge_request"]["commit"], caught.as_str());
    assert_eq!(update["merge_request"]["source_branch"], branch);
    assert_eq!(update["merge_request"]["number"], MERGE_REQUEST_NUMBER);
    // Still waiting, with nothing more to push.
    let out = stdout(&f.tool(&["finish"]).code(0));
    assert!(
        out.contains("the update waits for the merge request"),
        "{out}"
    );
    assert!(!out.contains("catching up"), "{out}");
    (commit, target, caught)
}

#[test]
fn integration_moved_before_the_merge_is_caught_up_and_merged_with_a_merge_commit() {
    let f = Fork::merge_request();
    let env = &f.env;
    let (commit, target, caught) = caught_up_while_waiting(&f);
    let merged = merge_on_forge(&f, MERGE_REQUEST_BRANCH);
    let out = stdout(&f.tool(&["finish"]).success());
    assert!(out.contains("received the verified commit"), "{out}");
    assert_eq!(env.rev(&f.down, "main"), merged);
    assert!(env.is_ancestor(&f.down, &caught, "main"));
    assert!(env.is_ancestor(&f.down, &commit, "main"));
    assert!(!env.has_rev(&f.publish, &format!("refs/heads/{MERGE_REQUEST_BRANCH}")));
    env.assert_no_update(&f.down);
    let status = env.tool_json(&f.down, &["status", "--json"], 0);
    let up = &status["upstreams"][0];
    assert_eq!(up["state"], "current");
    assert_eq!(up["last_synced"], target.as_str());
    assert_eq!(up["last_synced_source"], "record");
}

#[test]
fn integration_moved_before_the_merge_is_caught_up_and_merged_by_fast_forward() {
    let f = Fork::merge_request();
    let env = &f.env;
    let (_, target, caught) = caught_up_while_waiting(&f);
    env.git(&f.publish, &["update-ref", "refs/heads/main", &caught]);
    f.tool(&["finish"]).success();
    assert_eq!(env.rev(&f.down, "main"), caught);
    env.assert_no_update(&f.down);
    let status = env.tool_json(&f.down, &["status", "--json"], 0);
    assert_eq!(status["upstreams"][0]["last_synced"], target.as_str());
    assert_eq!(status["upstreams"][0]["last_synced_source"], "record");
}

#[test]
fn a_lagging_merge_request_head_follows_the_pushed_branch() {
    let f = Fork::merge_request();
    let env = &f.env;
    let base = env.rev(&f.down, "main");
    let (commit, _) = verified(&f);
    f.tool(&["finish"]).success();
    set_merge_request_head(&f, &commit);
    land(&f, &[("other.txt", "o\n")], "other: lands first");
    f.tool(&["finish"]).success();
    // The remote has not moved the merge request's head to the catch-up
    // merge yet: the update still waits.
    let out = stdout(&f.tool(&["finish"]).code(0));
    assert!(
        out.contains("the update waits for the merge request"),
        "{out}"
    );
    assert!(!out.contains("no longer points at"), "{out}");
    assert_eq!(env.rev(&f.down, "main"), base);
}

#[test]
fn a_catch_up_conflict_while_waiting_keeps_the_merge_request_and_abort_withdraws_it() {
    let f = Fork::merge_request();
    let env = &f.env;
    let base = env.rev(&f.down, "main");
    let (commit, _) = verified(&f);
    let branch = MERGE_REQUEST_BRANCH;
    f.tool(&["finish"]).success();
    land(&f, &[("n.txt", "theirs\n")], "other: add n");

    let out = stdout(&f.tool(&["finish"]).code(1));
    assert!(out.contains("n.txt  (unresolved)"), "{out}");
    assert!(
        out.contains("the update keeps waiting for the merge request"),
        "{out}"
    );
    assert_eq!(env.rev(&f.publish, &format!("refs/heads/{branch}")), commit);
    let status = env.tool_json(&f.down, &["status", "--json"], 0);
    let update = &status["update"];
    assert_eq!(update["step"], "catch_up_conflict");
    assert_eq!(update["merge_request"]["commit"], commit.as_str());
    f.tool(&["status"])
        .success()
        .stdout(contains("update in progress: catch up conflict"))
        .stdout(contains(format!(
            "merge request !7: {branch} into main on origin"
        )));
    let doctor = env.tool_json(&f.down, &["doctor", "--json"], 0);
    assert_eq!(doctor["healthy"], true, "{doctor}");

    let out = stdout(&f.tool(&["abort"]).success());
    assert!(
        out.contains(&format!(
            "deleted the merge request branch '{branch}' from 'origin'"
        )),
        "{out}"
    );
    assert!(out.contains("close the merge request"), "{out}");
    assert!(!env.has_rev(&f.publish, &format!("refs/heads/{branch}")));
    assert_eq!(env.rev(&f.down, "main"), base);
    env.assert_no_update(&f.down);
}

#[test]
fn failed_checks_of_a_catch_up_while_waiting_keep_the_merge_request() {
    let f = Fork::merge_request();
    let env = &f.env;
    let (commit, _) = verified(&f);
    f.tool(&["finish"]).success();
    // The integration branch removes the file the check needs.
    let forge = forge(&f);
    env.git(&forge, &["rm", "-q", common::FORK_FILE]);
    env.git(&forge, &["commit", "-q", "-m", "other: remove app"]);
    push_forge_main(&f, &forge);

    let out = stdout(&f.tool(&["finish"]).code(1));
    assert!(out.contains("FAILED   test -f app.txt"), "{out}");
    assert_eq!(
        env.rev(&f.publish, &format!("refs/heads/{MERGE_REQUEST_BRANCH}")),
        commit
    );
    let status = env.tool_json(&f.down, &["status", "--json"], 0);
    assert_eq!(status["update"]["step"], "catch_up");
    assert!(status["update"]["merge_request"].is_object());
    f.tool(&["abort"]).success();
    env.assert_no_update(&f.down);
}

#[test]
fn a_merge_during_the_catch_up_is_refused_and_nothing_is_pushed() {
    let f = Fork::merge_request();
    let env = &f.env;
    // While the catch-up merge is verified, the forge merges the merge
    // request as it was, with a merge commit.
    f.reconfigure(
        "checks = [\"test -f app.txt\"]",
        "checks = [\"test -f app.txt\", \"if [ -f \\\"$HOME/merge\\\" ]; then rm \\\"$HOME/merge\\\"; \
         cd \\\"$(cat \\\"$HOME/forge\\\")\\\" && git fetch -q origin && \
         git merge -q --no-ff -m merged origin/git-intake/update && git push -q origin HEAD:main; fi\"]",
    );
    let (commit, _) = verified(&f);
    f.tool(&["finish"]).success();
    land(&f, &[("other.txt", "o\n")], "other: lands first");
    fs::write(
        env.path("home").join("forge"),
        forge(&f).to_string_lossy().as_bytes(),
    )
    .unwrap();
    fs::write(env.path("home").join("merge"), "").unwrap();

    let finish = f.tool(&["finish"]).code(2);
    let err = stderr(&finish);
    assert!(err.contains("differs from the verified tree"), "{err}");
    assert!(err.contains("nothing was pushed"), "{err}");
    assert!(!env.path("home").join("merge").exists());
    assert!(
        !stdout(&finish).contains("pushed the verified catch-up merge"),
        "{}",
        stdout(&finish)
    );
    assert_eq!(
        env.rev(&f.publish, &format!("refs/heads/{MERGE_REQUEST_BRANCH}")),
        commit
    );
    let out = stdout(&f.tool(&["abort"]).success());
    assert!(out.contains("holds an unverified merge"), "{out}");
    env.assert_no_update(&f.down);
}

#[test]
fn a_branch_left_on_the_remote_is_refused_before_the_first_push() {
    let f = Fork::merge_request();
    let env = &f.env;
    let base = env.rev(&f.down, "main");
    let (commit, _) = verified(&f);
    let branch = MERGE_REQUEST_BRANCH;
    // A branch of that name that the base does not contain.
    let forge = forge(&f);
    env.git(&forge, &["checkout", "-q", "-b", "elsewhere"]);
    let foreign = env.commit(&forge, &[("foreign.txt", "f\n")], "someone: other work");
    env.git(&forge, &["push", "-q", "origin", &format!("HEAD:{branch}")]);
    env.git(&forge, &["checkout", "-q", "main"]);

    let err = stderr(&f.tool(&["finish"]).code(2));
    assert!(
        err.contains(&format!(
            "the branch '{branch}' already exists where 'origin' pushes to"
        )),
        "{err}"
    );
    assert!(
        err.contains(&format!("{} at {}", f.publish.display(), &foreign[..12])),
        "{err}"
    );
    assert!(
        err.contains("Nothing was pushed, and the update stays verified"),
        "{err}"
    );
    assert_eq!(
        env.rev(&f.publish, &format!("refs/heads/{branch}")),
        foreign
    );
    let status = env.tool_json(&f.down, &["status", "--json"], 0);
    assert_eq!(status["update"]["step"], "verified");

    // A branch the base contains, as one left by a merged merge request, is
    // taken over.
    env.git(
        &f.publish,
        &["update-ref", &format!("refs/heads/{branch}"), &base],
    );
    f.tool(&["finish"]).success().stdout(contains(format!(
        "asked 'origin' to open a merge request from '{branch}' into 'main'"
    )));
    assert_eq!(env.rev(&f.publish, &format!("refs/heads/{branch}")), commit);
}

#[test]
fn a_merge_request_closed_with_its_branch_kept_is_treated_as_open() {
    let f = Fork::merge_request();
    let env = &f.env;
    verified(&f);
    let branch = MERGE_REQUEST_BRANCH;
    f.tool(&["finish"]).success();
    // Closed on the forge, its branch kept: nothing tells it from an open one.
    f.tool(&["finish"])
        .code(0)
        .stdout(contains("the update waits for the merge request"));
    f.tool(&["abort"])
        .success()
        .stdout(contains(format!(
            "deleted the merge request branch '{branch}' from 'origin'"
        )))
        .stdout(contains(format!(
            "close the merge request from '{branch}' into 'main' on 'origin'"
        )));
    assert!(!env.has_rev(&f.publish, &format!("refs/heads/{branch}")));
    env.assert_no_update(&f.down);
}

#[test]
fn a_waiting_update_of_format_4_keeps_its_branch() {
    let f = Fork::merge_request();
    let env = &f.env;
    let (commit, target) = verified(&f);
    f.tool(&["finish"]).success();
    // As a release of state format 4 left it: the branch named after the
    // verified commit.
    let old = format!("git-intake/update-{}", &commit[..12]);
    env.git(
        &f.publish,
        &["update-ref", &format!("refs/heads/{old}"), &commit],
    );
    env.git(
        &f.publish,
        &[
            "update-ref",
            "-d",
            &format!("refs/heads/{MERGE_REQUEST_BRANCH}"),
        ],
    );
    let path = env.intake_dir(&f.down).join("state.json");
    let mut state: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    state["version"] = 4.into();
    state["merge_request"]["branch"] = old.clone().into();
    let fields = state.as_object_mut().unwrap();
    fields.remove("start_base");
    fields.remove("catch_ups");
    state["merge_request"]
        .as_object_mut()
        .unwrap()
        .remove("pushed");
    fs::write(&path, serde_json::to_string_pretty(&state).unwrap()).unwrap();
    let status = env.tool_json(&f.down, &["status", "--json"], 0);
    assert_eq!(
        status["update"]["merge_request"]["source_branch"],
        old.as_str()
    );

    land(&f, &[("other.txt", "o\n")], "other: lands first");
    let finish = f.tool(&["finish"]).success();
    assert!(
        stderr(&finish).contains("releases before v0.3.6 can no longer carry the update on"),
        "{}",
        stderr(&finish)
    );
    let caught = env.rev(&f.publish, &format!("refs/heads/{old}"));
    assert_eq!(parents(&f, &caught)[1], commit);
    assert!(!env.has_rev(&f.publish, &format!("refs/heads/{MERGE_REQUEST_BRANCH}")));

    merge_on_forge(&f, &old);
    f.tool(&["finish"]).success().stdout(contains(format!(
        "deleted the merge request branch '{old}' from 'origin'"
    )));
    assert!(env.is_ancestor(&f.down, &caught, "main"));
    let status = env.tool_json(&f.down, &["status", "--json"], 0);
    assert_eq!(status["upstreams"][0]["last_synced"], target.as_str());
    env.assert_no_update(&f.down);
}

#[test]
fn abort_while_waiting_ignores_a_branch_already_gone_and_keeps_one_that_moved() {
    let f = Fork::merge_request();
    let env = &f.env;
    verified(&f);
    let branch = MERGE_REQUEST_BRANCH.to_string();
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
    let branch = MERGE_REQUEST_BRANCH.to_string();
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
    let branch = MERGE_REQUEST_BRANCH.to_string();

    f.tool(&["finish"])
        .success()
        .stdout(contains(format!(
            "'origin' does not accept push options, so no merge request was opened; open one from '{branch}' into 'main' with squashing off"
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
                "unless a merge request from '{branch}' into 'main' is open on 'origin', open one with squashing off"
            ))
    );
    let text = stdout(&f.tool(&["status"]).success());
    assert!(
        text.contains("open it with squashing off unless it is open"),
        "{text}"
    );
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
            "not published to 'origin' (advanced locally only): main",
        ));
    assert_eq!(env.rev(&f.down, "main"), commit);
    assert!(env.is_ancestor(&f.down, &target, "main"));
    assert_eq!(env.rev(&f.publish, "main"), published);
    assert!(!env.has_rev(&f.publish, &format!("refs/heads/{MERGE_REQUEST_BRANCH}")));
    env.assert_no_update(&f.down);

    // While waiting: no branch moves before the merge.
    let f = Fork::merge_request();
    let env = &f.env;
    let base = env.rev(&f.down, "main");
    let (commit, target) = verified(&f);
    let branch = MERGE_REQUEST_BRANCH.to_string();
    f.tool(&["finish"]).success();
    // `--no-publish` cannot close an update whose merge request is published
    // but not merged: it refuses and says what to do instead.
    let refused = f.tool(&["finish", "--no-publish"]).code(2);
    let err = stderr(&refused);
    assert!(
        err.contains(&format!(
            "the update waits for the merge request from '{branch}' into 'main' on 'origin'"
        )),
        "{err}"
    );
    assert!(
        err.contains("`--no-publish` cannot close it before the merge request is merged"),
        "{err}"
    );
    assert!(err.contains("`git intake abort` withdraws it"), "{err}");
    assert_eq!(env.rev(&f.down, "main"), base);
    assert_eq!(env.rev(&f.publish, &format!("refs/heads/{branch}")), commit);
    let status = env.tool_json(&f.down, &["status", "--json"], 0);
    assert_eq!(status["update"]["step"], "waiting_for_merge_request");

    // After the merge: the local move only; the merge request branch stays
    // as it is on the remote.
    env.git(&f.publish, &["update-ref", "refs/heads/main", &commit]);
    f.tool(&["finish", "--no-publish"])
        .success()
        .stdout(contains("update finished"));
    assert_eq!(env.rev(&f.down, "main"), commit);
    assert!(env.is_ancestor(&f.down, &target, "main"));
    assert_eq!(env.rev(&f.publish, &format!("refs/heads/{branch}")), commit);
    env.assert_no_update(&f.down);
    let report = env.tool_json(&f.down, &["report", "--json"], 0);
    assert_eq!(report["unpublished"], serde_json::json!([]));
}

#[test]
fn merge_request_closed_without_merging_is_refused() {
    let f = Fork::merge_request();
    let env = &f.env;
    let base = env.rev(&f.down, "main");
    verified(&f);
    let branch = MERGE_REQUEST_BRANCH.to_string();
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
    common::merge_on_forge(&f.env, &f.publish, branch)
}

#[test]
fn merge_request_branch_changed_before_the_merge_is_refused() {
    let f = Fork::merge_request();
    let env = &f.env;
    let base = env.rev(&f.down, "main");
    let (commit, _) = verified(&f);
    let branch = MERGE_REQUEST_BRANCH.to_string();
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
    // The update cannot complete until someone acts: a negative result.
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
            "the merge request branch is at {}, not the verified commit",
            &extra[..12]
        )),
        "{err}"
    );
    assert_eq!(env.rev(&f.down, "main"), base);

    let out = stdout(&f.tool(&["abort"]).success());
    assert!(
        out.contains(&format!(
            "'origin/main' at {} holds an unverified merge",
            &merged[..12]
        )),
        "{out}"
    );
    assert!(out.contains("update aborted; 'main' is unchanged"), "{out}");
    assert!(!out.contains("close the merge request"), "{out}");
    // The branch moved beyond the verified commit, so it is left alone.
    assert!(out.contains("left the merge request branch"), "{out}");
    assert_eq!(env.rev(&f.down, "main"), base);
    env.assert_no_update(&f.down);
}

#[test]
fn merged_after_the_integration_branch_moved_is_refused() {
    let f = Fork::merge_request();
    let env = &f.env;
    let base = env.rev(&f.down, "main");
    verified(&f);
    let branch = MERGE_REQUEST_BRANCH.to_string();
    f.tool(&["finish"]).success();

    // Other work lands first; the forge still merges with a merge commit.
    let forge = forge(&f);
    env.commit(&forge, &[("other.txt", "o\n")], "other: lands first");
    push_forge_main(&f, &forge);
    merge_on_forge(&f, &branch);

    let err = stderr(&f.tool(&["finish"]).code(2));
    assert!(err.contains("differs from the verified tree"), "{err}");
    assert_eq!(env.rev(&f.down, "main"), base);
}

#[test]
fn abort_after_a_valid_merge_is_refused() {
    let f = Fork::merge_request();
    let env = &f.env;
    let (_, target) = verified(&f);
    let branch = MERGE_REQUEST_BRANCH.to_string();
    f.tool(&["finish"]).success();
    merge_on_forge(&f, &branch);

    f.tool(&["abort"])
        .code(2)
        .stderr(contains("was merged with the verified commit"))
        .stderr(contains("Run `git intake finish` to complete it"));
    assert!(env.has_rev(&f.publish, &format!("refs/heads/{branch}")));
    f.tool(&["finish"]).success();
    assert!(env.is_ancestor(&f.down, &target, "main"));
    env.assert_no_update(&f.down);
}

#[test]
fn interrupted_opening_does_not_claim_a_merge_request() {
    let f = Fork::merge_request();
    let env = &f.env;
    let (commit, _) = verified(&f);
    let branch = MERGE_REQUEST_BRANCH.to_string();
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
    // Pushes go to a repository that holds the branch as well and cannot
    // receive any push: the branch is found in place there, and nothing is
    // pushed.
    let push_side = push_side(&f);
    env.git(
        &f.down,
        &[
            "push",
            "-q",
            &push_side.to_string_lossy(),
            &format!("{commit}:refs/heads/{branch}"),
        ],
    );
    env.git(&f.down, &["config", "remote.origin.receivepack", "false"]);

    let finish = f.tool(&["finish"]).success();
    let out = stdout(&finish);
    assert!(
        out.contains(&format!(
            "'{branch}' was already on 'origin', so nothing was pushed and no merge request was asked for; \
             if none is open, open one from '{branch}' into 'main'"
        )),
        "{out}"
    );
    assert!(!out.contains("pushed the verified commit"), "{out}");
    let status = env.tool_json(&f.down, &["status", "--json"], 0);
    assert_eq!(status["update"]["step"], "waiting_for_merge_request");
    assert_eq!(status["update"]["merge_request"]["requested"], false);
    f.tool(&["finish"]).code(0).stdout(contains(
        "if the merge request is not open yet, open it with squashing off",
    ));
}

/// A repository the publish remote pushes to while it is still fetched from
/// its own URL.
fn push_side(f: &Fork) -> PathBuf {
    let env = &f.env;
    let dir = env.bare("push-side");
    accept_push_options(env, &dir);
    env.git(
        &f.down,
        &["config", "remote.origin.pushurl", &dir.to_string_lossy()],
    );
    dir
}

#[test]
fn branch_only_on_the_fetch_side_is_pushed_where_pushes_go() {
    let f = Fork::merge_request();
    let env = &f.env;
    let (commit, _) = verified(&f);
    let branch = MERGE_REQUEST_BRANCH.to_string();
    env.git(
        &f.down,
        &[
            "push",
            "-q",
            "origin",
            &format!("{commit}:refs/heads/{branch}"),
        ],
    );
    let push_side = push_side(&f);

    f.tool(&["finish"]).success().stdout(contains(format!(
        "asked 'origin' to open a merge request from '{branch}' into 'main'"
    )));
    assert_eq!(env.rev(&push_side, &format!("refs/heads/{branch}")), commit);
    assert!(recorded_push_options(&push_side).contains(&"merge_request.create".to_string()));
    let status = env.tool_json(&f.down, &["status", "--json"], 0);
    assert_eq!(status["update"]["merge_request"]["requested"], true);
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
fn only_push_urls_without_the_branch_receive_the_push() {
    let f = Fork::merge_request();
    let env = &f.env;
    let (commit, _) = verified(&f);
    let branch = MERGE_REQUEST_BRANCH.to_string();
    let holding = env.bare("holding");
    env.git(
        &f.down,
        &[
            "push",
            "-q",
            &holding.to_string_lossy(),
            &format!("{commit}:refs/heads/{branch}"),
        ],
    );
    let lacking = env.bare("lacking");
    accept_push_options(env, &lacking);
    // A push to the URL that already holds the branch would fail.
    let read_only = read_only_url(&f, &holding);
    set_push_urls(&f, &[&read_only, &lacking.to_string_lossy()]);

    f.tool(&["finish"]).success().stdout(contains(format!(
        "asked 'origin' to open a merge request from '{branch}' into 'main'"
    )));
    assert_eq!(env.rev(&lacking, &format!("refs/heads/{branch}")), commit);
    assert!(recorded_push_options(&lacking).contains(&"merge_request.create".to_string()));
    let status = env.tool_json(&f.down, &["status", "--json"], 0);
    assert_eq!(status["update"]["merge_request"]["requested"], true);
}

#[test]
fn a_failed_push_names_the_push_urls_that_received_the_branch() {
    let f = Fork::merge_request();
    let env = &f.env;
    let (commit, _) = verified(&f);
    let branch = MERGE_REQUEST_BRANCH.to_string();
    let holding = env.bare("holding");
    env.git(
        &f.down,
        &[
            "push",
            "-q",
            &holding.to_string_lossy(),
            &format!("{commit}:refs/heads/{branch}"),
        ],
    );
    let receiving = env.bare("receiving").to_string_lossy().into_owned();
    let refusing = env.bare("refusing");
    let holding = read_only_url(&f, &holding);
    let refusing = read_only_url(&f, &refusing);
    set_push_urls(&f, &[&holding, &receiving, &refusing]);

    let finish = f.tool(&["finish"]).code(2);
    let err = stderr(&finish);
    assert!(
        err.contains(&format!(
            "The branch is already on {receiving}; no other ref moved, and the update stays verified; \
             fix the cause and run `git intake finish` again, which pushes only where the branch is missing"
        )),
        "{err}"
    );
    assert!(!err.contains("Nothing moved"), "{err}");
    assert!(!stdout(&finish).contains("pushed the verified commit"));
}

#[test]
fn a_failed_push_through_several_push_urls_says_some_may_hold_the_branch() {
    let f = Fork::merge_request();
    let env = &f.env;
    verified(&f);
    let receiving = env.bare("receiving").to_string_lossy().into_owned();
    let refusing = read_only_url(&f, &env.bare("refusing"));
    set_push_urls(&f, &[&receiving, &refusing]);

    let err = stderr(&f.tool(&["finish"]).code(2));
    assert!(
        err.contains(
            "The branch may already be on some of the push URLs of 'origin'; no other ref moved, \
             and the update stays verified"
        ),
        "{err}"
    );
    assert!(!err.contains("Nothing moved"), "{err}");
}

#[test]
fn a_failed_push_to_the_only_push_url_moved_nothing() {
    let f = Fork::merge_request();
    let env = &f.env;
    verified(&f);
    let refusing = read_only_url(&f, &env.bare("refusing"));
    set_push_urls(&f, &[&refusing]);

    let err = stderr(&f.tool(&["finish"]).code(2));
    assert!(
        err.contains(
            "Nothing moved, and the update stays verified; fix the cause and run `git intake finish` again"
        ),
        "{err}"
    );
}

#[test]
fn a_copy_left_on_one_push_url_is_not_reported_gone() {
    let f = Fork::merge_request();
    let env = &f.env;
    let (commit, _) = verified(&f);
    let branch = MERGE_REQUEST_BRANCH.to_string();
    let second = env.bare("second");
    let publish = f.publish.to_string_lossy().into_owned();
    set_push_urls(&f, &[&publish, &second.to_string_lossy()]);
    f.tool(&["finish"]).success();
    assert_eq!(env.rev(&second, &format!("refs/heads/{branch}")), commit);

    // The second URL now refuses pushes, so only the first copy is deleted.
    let read_only = read_only_url(&f, &second);
    set_push_urls(&f, &[&publish, &read_only]);
    let abort = f.tool(&["abort"]).code(2);
    let out = stdout(&abort);
    assert!(
        out.contains(&format!(
            "deleted the merge request branch '{branch}' from 'origin' at {publish}"
        )),
        "{out}"
    );
    assert!(!out.contains("already gone"), "{out}");
    assert!(
        stderr(&abort).contains(&format!(
            "cannot delete the merge request branch '{branch}' from 'origin'"
        )),
        "{}",
        stderr(&abort)
    );
    assert!(!env.has_rev(&f.publish, &format!("refs/heads/{branch}")));
    assert_eq!(env.rev(&second, &format!("refs/heads/{branch}")), commit);
    let status = env.tool_json(&f.down, &["status", "--json"], 0);
    assert_eq!(status["update"]["step"], "waiting_for_merge_request");
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
    let branch = MERGE_REQUEST_BRANCH.to_string();
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
    f.tool(&["abort"]).success().stdout(contains(format!(
        "holds an unverified merge: the merge request's head is {}, and it landed instead of the verified commit",
        &extra[..12]
    )));
    env.assert_no_update(&f.down);
}

#[test]
fn fast_forward_of_a_changed_branch_is_refused_without_the_merge_request_head() {
    let f = Fork::merge_request();
    let env = &f.env;
    let base = env.rev(&f.down, "main");
    verified(&f);
    let branch = MERGE_REQUEST_BRANCH.to_string();
    f.tool(&["finish"]).success();

    // The remote keeps no merge request head; the branch is still listed.
    let extra = add_to_merge_request_branch(&f, &branch);
    env.git(&f.publish, &["update-ref", "refs/heads/main", &extra]);

    let err = stderr(&f.tool(&["finish"]).code(2));
    assert!(
        err.contains(&format!(
            "the merge request branch is at {}, not the verified commit",
            &extra[..12]
        )),
        "{err}"
    );
    assert_eq!(env.rev(&f.down, "main"), base);
}

#[test]
fn branch_as_last_seen_judges_once_it_is_gone() {
    let f = Fork::merge_request();
    let env = &f.env;
    verified(&f);
    let branch = MERGE_REQUEST_BRANCH.to_string();
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
        "the merge request branch was at {}, not the verified commit",
        &extra[..12]
    )));
}

#[test]
fn fast_forward_with_later_work_completes_when_the_head_is_verified() {
    let f = Fork::merge_request();
    let env = &f.env;
    let (commit, _) = verified(&f);
    let branch = MERGE_REQUEST_BRANCH.to_string();
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
    let branch = MERGE_REQUEST_BRANCH.to_string();
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
    let branch = MERGE_REQUEST_BRANCH.to_string();
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

#[test]
fn a_matching_merge_request_head_does_not_clear_a_changed_branch() {
    let f = Fork::merge_request();
    let env = &f.env;
    let base = env.rev(&f.down, "main");
    let (commit, _) = verified(&f);
    let branch = MERGE_REQUEST_BRANCH.to_string();
    f.tool(&["finish"]).success();
    // The remote's merge request head stays at the verified commit while
    // the branch gains a commit that is fast-forwarded onto main.
    set_merge_request_head(&f, &commit);
    let extra = add_to_merge_request_branch(&f, &branch);
    env.git(&f.publish, &["update-ref", "refs/heads/main", &extra]);

    let err = stderr(&f.tool(&["finish"]).code(2));
    assert!(
        err.contains(&format!(
            "the merge request branch is at {}, not the verified commit",
            &extra[..12]
        )),
        "{err}"
    );
    assert!(!err.contains("the merge request's head is"), "{err}");
    assert_eq!(env.rev(&f.down, "main"), base);
}

#[test]
fn only_the_link_for_the_pushed_branch_names_the_merge_request() {
    let f = Fork::merge_request();
    let env = &f.env;
    // The remote's reply carries an unrelated merge request link first.
    let hook = f.publish.join("hooks").join("pre-receive");
    let script = fs::read_to_string(&hook).unwrap().replacen(
        "if [ -n \"$create\" ]; then\n",
        "echo \"See https://example.com/team/fork/-/merge_requests/3\" >&2\nif [ -n \"$create\" ]; then\n",
        1,
    );
    fs::write(&hook, script).unwrap();
    verified(&f);
    f.tool(&["finish"])
        .success()
        .stdout(contains("opened merge request !7"));
    let status = env.tool_json(&f.down, &["status", "--json"], 0);
    assert_eq!(
        status["update"]["merge_request"]["number"],
        MERGE_REQUEST_NUMBER
    );
}

#[test]
fn a_reply_without_the_pushed_branch_names_no_merge_request() {
    let f = Fork::merge_request();
    let env = &f.env;
    // The reply links a merge request, but not under the pushed branch.
    let hook = f.publish.join("hooks").join("pre-receive");
    let script = fs::read_to_string(&hook)
        .unwrap()
        .replace("View merge request for ${ref#refs/heads/}:", "See also:");
    fs::write(&hook, script).unwrap();
    verified(&f);
    let out = stdout(&f.tool(&["finish"]).success());
    assert!(!out.contains("opened merge request !"), "{out}");
    let status = env.tool_json(&f.down, &["status", "--json"], 0);
    assert_eq!(
        status["update"]["merge_request"]["number"],
        serde_json::Value::Null
    );
}

#[test]
fn a_sync_record_merged_with_a_merge_commit_is_the_last_synced_commit() {
    let f = Fork::merge_request();
    let env = &f.env;
    let (commit, target) = verified(&f);
    f.tool(&["finish"]).success();
    // The forge merges with a merge commit while the integration branch is
    // where the update started, as the semi-linear method requires: the
    // update's commits, its sync record among them, are off the first-parent
    // line.
    merge_on_forge(&f, MERGE_REQUEST_BRANCH);
    f.tool(&["finish"]).success();
    assert_eq!(env.rev(&f.down, "main^2"), commit);

    let status = env.tool_json(&f.down, &["status", "--json"], 0);
    let up = &status["upstreams"][0];
    assert_eq!(up["state"], "current");
    assert_eq!(up["last_synced"], target.as_str());
    assert_eq!(up["last_synced_source"], "record");
    let doctor = stdout(&f.tool(&["doctor"]).success());
    assert!(
        doctor.contains(&format!(
            "last synced commit {} is contained in upstream/main",
            &target[..12]
        )),
        "{doctor}"
    );

    // A feature merged later with a merge commit carries no record and
    // hides none.
    let forge = forge(&f);
    env.git(&forge, &["checkout", "-q", "-b", "feature"]);
    env.commit(&forge, &[("feature.txt", "f\n")], "fork: a feature");
    env.git(&forge, &["checkout", "-q", "main"]);
    env.git(
        &forge,
        &[
            "merge",
            "-q",
            "--no-ff",
            "-m",
            "Merge branch 'feature'",
            "feature",
        ],
    );
    push_forge_main(&f, &forge);
    env.git(&f.down, &["pull", "-q", "--ff-only", "origin", "main"]);
    let status = env.tool_json(&f.down, &["status", "--json"], 0);
    let up = &status["upstreams"][0];
    assert_eq!(up["last_synced"], target.as_str());
    assert_eq!(up["last_synced_source"], "record");

    // The next update starts from that record, and its merge request lands
    // the same way.
    let next = env.push_upstream(&f.up, &[("m.txt", "m\n")], "upstream: m");
    f.tool(&["start"]).success();
    f.tool(&["verify"]).success();

    f.tool(&["finish"]).success();
    let description = recorded_push_options(&f.publish)
        .into_iter()
        .find(|o| o.starts_with("merge_request.description="))
        .expect("description");
    assert!(
        description.contains(&format!("{}..{} (1 commit)", &target[..12], &next[..12])),
        "{description}"
    );
    merge_on_forge(&f, MERGE_REQUEST_BRANCH);
    f.tool(&["finish"]).success();
    let status = env.tool_json(&f.down, &["status", "--json"], 0);
    let up = &status["upstreams"][0];
    assert_eq!(up["state"], "current");
    assert_eq!(up["last_synced"], next.as_str());
    assert_eq!(up["last_synced_source"], "record");
}

#[test]
fn a_sync_record_landed_by_fast_forward_is_the_last_synced_commit() {
    let f = Fork::merge_request();
    let env = &f.env;
    let (commit, target) = verified(&f);
    f.tool(&["finish"]).success();
    env.git(&f.publish, &["update-ref", "refs/heads/main", &commit]);
    f.tool(&["finish"]).success();
    assert_eq!(env.rev(&f.down, "main"), commit);

    let status = env.tool_json(&f.down, &["status", "--json"], 0);
    let up = &status["upstreams"][0];
    assert_eq!(up["state"], "current");
    assert_eq!(up["last_synced"], target.as_str());
    assert_eq!(up["last_synced_source"], "record");
}
