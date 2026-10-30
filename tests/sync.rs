//! `sync`: the follow upstreams taken in without local state, as a
//! scheduled CI job runs it. The forge is a fake one; see `common::forge`.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::forge::branch_tip;
use common::{Fork, How, MERGE_REQUEST_BRANCH, Mono, merge_on_fake, stderr, stdout};

fn on_remote(f: &Fork, branch: &str) -> Option<String> {
    branch_tip(&f.publish, branch)
}

/// Lands a commit on the publish remote's integration branch, as other work
/// does; returns it.
fn land(f: &Fork, files: &[(&str, &str)], message: &str) -> String {
    let dir = common::forge(&f.env, &f.publish);
    let c = f.env.commit(&dir, files, message);
    f.env.git(&dir, &["push", "-q", "origin", "HEAD:main"]);
    c
}

/// A fresh clone of the fork as a CI job makes it: `HEAD` detached at the
/// integration branch's tip, no local integration branch, the publish
/// remote's URL as in the fork.
fn ci_clone(f: &Fork, name: &str) -> PathBuf {
    let env = &f.env;
    let dir = env.path(name);
    env.git(
        &env.path(""),
        &[
            "clone",
            "-q",
            "--no-checkout",
            &f.publish.to_string_lossy(),
            &dir.to_string_lossy(),
        ],
    );
    let tip = env.rev(&dir, "origin/main");
    env.git(&dir, &["checkout", "-q", "--detach", &tip]);
    env.git(&dir, &["branch", "-D", "main"]);
    env.git(
        &dir,
        &["remote", "set-url", "origin", &f.forge().remote_url()],
    );
    dir
}

/// A fork in merge-request mode whose upstream has one new commit, and the
/// check `false`, which `sync` never runs in merge-request mode.
fn behind(f: &Fork) -> String {
    f.reconfigure("run = \"test -f app.txt\"", "run = \"false\"");
    f.env
        .push_upstream(&f.up, &[("n.txt", "n\n")], "upstream: n")
}

#[test]
fn a_scheduled_sync_opens_the_merge_request() {
    let f = Fork::merge_request();
    let env = &f.env;
    let target = behind(&f);
    let main = env.rev(&f.down, "main");
    let out = stdout(&f.tool(&["sync"]).code(0));
    assert!(out.contains("upstream: queued at"), "{out}");
    assert!(out.contains("opened merge request !1"), "{out}");
    assert!(!out.contains("verifying"), "{out}");
    let head = on_remote(&f, MERGE_REQUEST_BRANCH).expect("the update's branch");
    assert!(env.is_ancestor(&f.publish, &target, &head));
    let mr = f.forge().mr(1);
    assert_eq!(
        (mr.source.as_str(), mr.target.as_str(), mr.state),
        (MERGE_REQUEST_BRANCH, "main", "open")
    );
    assert_eq!(env.rev(&f.down, "main"), main);
    env.assert_no_update(&f.down);
    // The release that ran it is recorded on nothing local: no state remains.
    assert!(!env.intake_dir(&f.down).join("state.json").exists());
}

#[test]
fn a_clean_ci_clone_gets_its_integration_branch() {
    let f = Fork::merge_request();
    let env = &f.env;
    behind(&f);
    let ci = ci_clone(&f, "ci");
    let out = stdout(&env.tool(&ci, &["sync"]).success());
    assert!(out.contains("created 'main' at"), "{out}");
    assert!(out.contains("opened merge request !1"), "{out}");
    assert_eq!(env.rev(&ci, "main"), env.rev(&f.publish, "main"));
}

#[test]
fn nothing_behind_creates_nothing() {
    let f = Fork::merge_request();
    let out = stdout(&f.tool(&["sync"]).success());
    assert!(out.contains("every follow upstream is current"), "{out}");
    assert!(f.forge().changes().is_empty());
    assert_eq!(on_remote(&f, MERGE_REQUEST_BRANCH), None);
}

#[test]
fn an_update_in_progress_is_refused_with_its_step() {
    let f = Fork::merge_request();
    behind(&f);
    f.tool(&["start"]).success();
    let err = stderr(&f.tool(&["sync"]).code(2));
    assert!(
        err.contains("an update is in progress, at the step merged; next: git intake finish"),
        "{err}"
    );
    assert!(f.forge().changes().is_empty());
}

#[test]
fn sync_needs_a_publication_setting() {
    let f = Fork::new();
    f.reconfigure("[publish]\nremote = \"origin\"\nvia = \"push\"\n", "");
    let err = stderr(&f.tool(&["sync"]).code(2));
    assert!(err.contains("has no [publish] setting"), "{err}");
}

#[test]
fn unpublished_local_commits_are_refused() {
    let f = Fork::merge_request();
    let env = &f.env;
    behind(&f);
    env.commit(&f.down, &[("local.txt", "l\n")], "fork: not published");
    let err = stderr(&f.tool(&["sync"]).code(2));
    assert!(
        err.contains("'main' holds 1 commit that 'origin/main' lacks"),
        "{err}"
    );
    assert!(f.forge().changes().is_empty());
    env.assert_no_update(&f.down);
}

#[test]
fn an_unreachable_upstream_is_left_out_and_the_others_taken() {
    let m = Mono::merge_request();
    let env = &m.env;
    // app-b cannot be fetched.
    fs::rename(&m.b.bare, env.path("gone.git")).unwrap();
    let out = stdout(&m.tool(&["sync"]).code(1));
    assert!(
        out.contains("left out: upstream 'app-b' cannot be fetched"),
        "{out}"
    );
    assert!(out.contains("app-a: queued at"), "{out}");
    assert!(out.contains("opened merge request !1"), "{out}");
    let fake = m.forge.as_ref().unwrap();
    assert_eq!(fake.mr(1).title, "Merge upstream update: app-a");
    env.assert_no_update(&m.down);
}

#[test]
fn two_upstreams_share_one_merge_request() {
    let m = Mono::merge_request();
    let out = stdout(&m.tool(&["sync"]).success());
    assert!(out.contains("opened merge request !1"), "{out}");
    let fake = m.forge.as_ref().unwrap();
    let mr = fake.mr(1);
    assert_eq!(mr.title, "Merge upstream updates: app-a, app-b");
    let targets = description_targets(&mr.description);
    assert_eq!(targets.len(), 2, "{targets:?}");
}

fn description_targets(description: &str) -> Vec<String> {
    let block = description
        .rsplit("<!-- git-intake targets")
        .next()
        .unwrap_or_default();
    block
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with("-->"))
        .map(str::to_string)
        .collect()
}

#[test]
fn a_pending_port_upstream_is_named_and_not_taken() {
    let f = Fork::port_merge_request();
    let env = &f.env;
    env.push_upstream(&f.up, &[("p.txt", "p\n")], "upstream: p");
    let out = stdout(&f.tool(&["sync"]).success());
    assert!(
        out.contains("upstream: port upstream with 1 pending commit"),
        "{out}"
    );
    assert!(out.contains("every follow upstream is current"), "{out}");
    assert!(f.forge().changes().is_empty());
    env.assert_no_update(&f.down);
}

/// A fork whose upstream and integration branch both change `app.txt`'s
/// first line, so that the update conflicts.
fn conflicting(f: &Fork) {
    f.reconfigure("run = \"test -f app.txt\"", "run = \"false\"");
    f.env.push_upstream(
        &f.up,
        &[("app.txt", "upstream\ntwo\nthree\n")],
        "upstream: change",
    );
    f.env.commit(
        &f.down,
        &[("app.txt", "fork\ntwo\nthree\n")],
        "fork: change",
    );
    f.env.git(&f.down, &["push", "-q", "origin", "main"]);
}

#[test]
fn a_conflict_stops_sync_with_the_update_in_progress() {
    let f = Fork::merge_request();
    let env = &f.env;
    conflicting(&f);
    let out = stdout(&f.tool(&["sync"]).code(1));
    assert!(out.contains("conflicts merging upstream/main"), "{out}");
    assert!(out.contains("app.txt"), "{out}");
    assert!(
        out.contains("`git intake continue` and `git intake finish`"),
        "{out}"
    );
    assert_eq!(on_remote(&f, MERGE_REQUEST_BRANCH), None);
    assert!(f.forge().changes().is_empty());

    // Another clone meets the same conflict while nothing moved.
    let ci = ci_clone(&f, "ci");
    let out = stdout(&env.tool(&ci, &["sync"]).code(1));
    assert!(out.contains("app.txt"), "{out}");
    // The person resolves it in their clone and publishes the update.
    let wt = env.update_worktree(&f.down);
    fs::write(wt.join("app.txt"), "both\ntwo\nthree\n").unwrap();
    env.git(&wt, &["add", "app.txt"]);
    f.tool(&["continue"]).success();
    f.tool(&["finish"])
        .success()
        .stdout(predicates::str::contains("opened merge request !1"));
    env.assert_no_update(&f.down);
}

#[test]
fn sync_again_without_change_changes_nothing() {
    let f = Fork::merge_request();
    behind(&f);
    f.tool(&["sync"]).success();
    let head = on_remote(&f, MERGE_REQUEST_BRANCH);
    f.forge().clear_requests();
    let out = stdout(&f.tool(&["sync"]).success());
    assert!(
        out.contains("the update waits for the merge request !1"),
        "{out}"
    );
    assert!(f.forge().changes().is_empty(), "{:?}", f.forge().changes());
    assert_eq!(on_remote(&f, MERGE_REQUEST_BRANCH), head);
}

#[test]
fn an_upstream_that_moved_on_waits_for_the_next_update() {
    let f = Fork::merge_request();
    let env = &f.env;
    behind(&f);
    f.tool(&["sync"]).success();
    let head = on_remote(&f, MERGE_REQUEST_BRANCH);
    let moved = env.push_upstream(&f.up, &[("m.txt", "m\n")], "upstream: m");
    let out = stdout(&f.tool(&["sync"]).success());
    assert!(
        out.contains(&format!("upstream: moved on to {}", &moved[..12])),
        "{out}"
    );
    assert_eq!(on_remote(&f, MERGE_REQUEST_BRANCH), head);
    assert!(f.forge().changes().iter().all(|r| r.method == "POST"));

    // After the merge, the next sync takes the new commits in a new one.
    merge_on_fake(env, f.forge(), 1, How::MergeCommit);
    let out = stdout(&f.tool(&["sync"]).success());
    assert!(out.contains("opened merge request !2"), "{out}");
    let head = on_remote(&f, MERGE_REQUEST_BRANCH).unwrap();
    assert!(env.is_ancestor(&f.publish, &moved, &head));
}

#[test]
fn a_moved_integration_branch_catches_the_merge_request_up() {
    let f = Fork::merge_request();
    let env = &f.env;
    behind(&f);
    f.tool(&["sync"]).success();
    let before = on_remote(&f, MERGE_REQUEST_BRANCH).unwrap();
    f.forge().describe(1, "edited by a person");
    let tip = land(&f, &[("other.txt", "o\n")], "other: lands");

    let out = stdout(&f.tool(&["sync"]).success());
    assert!(out.contains("caught up with 'main'"), "{out}");
    assert!(
        out.contains("rewrote the title and description of merge request !1"),
        "{out}"
    );
    let after = on_remote(&f, MERGE_REQUEST_BRANCH).unwrap();
    assert_eq!(env.rev(&f.publish, &format!("{after}^1")), tip);
    assert_eq!(env.rev(&f.publish, &format!("{after}^2")), before);
    assert_eq!(f.forge().mrs().len(), 1);
    let mr = f.forge().mr(1);
    assert!(
        mr.description.contains("<!-- git-intake targets"),
        "{}",
        mr.description
    );
    assert_eq!(mr.squash, Some(false));
    env.assert_no_update(&f.down);
}

#[test]
fn a_fix_pushed_to_the_merge_request_stays_in_the_update() {
    let f = Fork::merge_request();
    let env = &f.env;
    behind(&f);
    f.tool(&["sync"]).success();
    // A person pushes a fix onto the merge request branch.
    let dir = common::forge(env, &f.publish);
    env.git(
        &dir,
        &[
            "checkout",
            "-q",
            "-b",
            "fix",
            &format!("origin/{MERGE_REQUEST_BRANCH}"),
        ],
    );
    let fix = env.commit(&dir, &[("fix.txt", "fix\n")], "fix: make it work");
    env.git(
        &dir,
        &[
            "push",
            "-q",
            "origin",
            &format!("HEAD:{MERGE_REQUEST_BRANCH}"),
        ],
    );
    env.git(&dir, &["checkout", "-q", "main"]);
    land(&f, &[("other.txt", "o\n")], "other: lands");

    f.tool(&["sync"]).success();
    let after = on_remote(&f, MERGE_REQUEST_BRANCH).unwrap();
    assert_eq!(env.rev(&f.publish, &format!("{after}^2")), fix);
}

#[test]
fn a_catch_up_conflict_keeps_the_merge_request_s_head() {
    let f = Fork::merge_request();
    let env = &f.env;
    f.reconfigure("run = \"test -f app.txt\"", "run = \"false\"");
    env.push_upstream(
        &f.up,
        &[("app.txt", "upstream\ntwo\nthree\n")],
        "upstream: change",
    );
    f.tool(&["sync"]).success();
    let before = on_remote(&f, MERGE_REQUEST_BRANCH).unwrap();
    land(&f, &[("app.txt", "fork\ntwo\nthree\n")], "fork: change");

    let out = stdout(&f.tool(&["sync"]).code(1));
    assert!(out.contains("conflicts catching up with 'main'"), "{out}");
    assert!(out.contains("the merge request !1"), "{out}");
    assert_eq!(
        on_remote(&f, MERGE_REQUEST_BRANCH).as_deref(),
        Some(before.as_str())
    );
    let status = env.tool_json(&f.down, &["status", "--json"], 0);
    assert_eq!(status["update"]["step"], "catch_up_conflict");
    let next = status["update"]["next"].as_str().unwrap();
    assert!(
        next.starts_with("resolve the conflicts of the catch-up merge in "),
        "{next}"
    );
    assert!(next.ends_with("then run `git intake continue`"), "{next}");
    assert_eq!(status["update"]["upstreams"][0]["name"], "upstream");
    assert_eq!(status["update"]["carried_from"]["number"], 1);
    let report = stdout(&f.tool(&["report"]).success());
    assert!(
        report.contains("the conflicts of the steps taken before it was carried on are not known"),
        "{report}"
    );

    let wt = env.update_worktree(&f.down);
    fs::write(wt.join("app.txt"), "both\ntwo\nthree\n").unwrap();
    env.git(&wt, &["add", "app.txt"]);
    f.tool(&["continue"]).success();
    let out = stdout(&f.tool(&["finish"]).success());
    assert!(
        out.contains("rewrote the title and description of merge request !1"),
        "{out}"
    );
    let after = on_remote(&f, MERGE_REQUEST_BRANCH).unwrap();
    assert_eq!(env.rev(&f.publish, &format!("{after}^2")), before);
    assert_eq!(f.forge().mrs().len(), 1);
}

#[test]
fn abort_of_a_carried_on_update_keeps_the_merge_request() {
    let f = Fork::merge_request();
    let env = &f.env;
    f.reconfigure("run = \"test -f app.txt\"", "run = \"false\"");
    env.push_upstream(
        &f.up,
        &[("app.txt", "upstream\ntwo\nthree\n")],
        "upstream: change",
    );
    f.tool(&["sync"]).success();
    land(&f, &[("app.txt", "fork\ntwo\nthree\n")], "fork: change");
    f.tool(&["sync"]).code(1);
    f.forge().clear_requests();
    let out = stdout(&f.tool(&["abort"]).success());
    assert!(
        out.contains("`git intake abort --merge-request` withdraws it"),
        "{out}"
    );
    assert!(f.forge().changes().is_empty());
    assert_eq!(f.forge().mr(1).state, "open");
    env.assert_no_update(&f.down);
}

#[test]
fn finish_refuses_a_carried_on_update_whose_merge_request_closed() {
    let f = Fork::merge_request();
    let env = &f.env;
    f.reconfigure("run = \"test -f app.txt\"", "run = \"false\"");
    env.push_upstream(
        &f.up,
        &[("app.txt", "upstream\ntwo\nthree\n")],
        "upstream: change",
    );
    f.tool(&["sync"]).success();
    land(&f, &[("app.txt", "fork\ntwo\nthree\n")], "fork: change");
    f.tool(&["sync"]).code(1);
    let wt = env.update_worktree(&f.down);
    fs::write(wt.join("app.txt"), "both\ntwo\nthree\n").unwrap();
    env.git(&wt, &["add", "app.txt"]);
    f.tool(&["continue"]).success();
    f.forge().end(1, "closed");
    let err = stderr(&f.tool(&["finish"]).code(2));
    assert!(err.contains("merge request !1"), "{err}");
    assert!(err.contains("closed without merging"), "{err}");
    assert!(err.contains("`git intake abort`"), "{err}");
    assert!(wt.exists(), "the update is kept");
}

#[test]
fn an_earlier_release_s_branch_gets_the_catch_up() {
    let f = Fork::merge_request();
    let env = &f.env;
    behind(&f);
    f.tool(&["start"]).success();
    let head = env.rev(&env.update_worktree(&f.down), "HEAD");
    f.tool(&["abort"]).success();
    // An earlier release pushed the update to its own branch name and
    // opened the merge request; its description has no block.
    let earlier = format!("git-intake/update-{}", &head[..12]);
    env.git(
        &f.down,
        &[
            "push",
            "-q",
            &f.publish.to_string_lossy(),
            &format!("{head}:refs/heads/{earlier}"),
        ],
    );
    let n = f
        .forge()
        .add(&earlier, "main", "Upstream changes verified by git intake.");
    land(&f, &[("other.txt", "o\n")], "other: lands");

    f.tool(&["sync"]).success();
    let after = on_remote(&f, &earlier).unwrap();
    assert_eq!(env.rev(&f.publish, &format!("{after}^2")), head);
    assert_eq!(on_remote(&f, MERGE_REQUEST_BRANCH), None);
    assert!(
        f.forge()
            .mr(n)
            .description
            .contains("<!-- git-intake targets")
    );
}

#[test]
fn a_refused_catch_up_leaves_no_update() {
    let f = Fork::merge_request();
    let env = &f.env;
    let target = behind(&f);
    f.tool(&["sync"]).success();
    // The integration branch took the upstream in elsewhere.
    let dir = common::forge(env, &f.publish);
    env.git(&dir, &["fetch", "-q", &f.up.url(), "main"]);
    env.git(
        &dir,
        &[
            "merge",
            "-q",
            "--no-ff",
            "-m",
            &format!("sync(upstream): merge upstream/main\n\nIntake-Upstream: upstream\nIntake-Target: {target}\n"),
            &target,
        ],
    );
    env.git(&dir, &["push", "-q", "origin", "HEAD:main"]);
    for _ in 0..2 {
        let err = stderr(&f.tool(&["sync"]).code(2));
        assert!(
            err.contains("carries a sync record or a copy of upstream 'upstream'"),
            "{err}"
        );
        assert!(err.contains("`git intake abort --merge-request`"), "{err}");
        assert!(err.contains("declines its targets"), "{err}");
        env.assert_no_update(&f.down);
    }
}

#[test]
fn a_rebased_merge_request_is_left_alone() {
    let f = Fork::merge_request();
    let env = &f.env;
    behind(&f);
    f.tool(&["sync"]).success();
    // The forge's rebase button drops the sync merge.
    let dir = common::forge(env, &f.publish);
    env.git(&dir, &["checkout", "-q", "-b", "rebased", "origin/main"]);
    env.commit(&dir, &[("n.txt", "n\n")], "upstream: n");
    env.git(
        &dir,
        &[
            "push",
            "-q",
            "-f",
            "origin",
            &format!("HEAD:{MERGE_REQUEST_BRANCH}"),
        ],
    );
    env.git(&dir, &["checkout", "-q", "main"]);
    land(&f, &[("other.txt", "o\n")], "other: lands");
    let rebased = on_remote(&f, MERGE_REQUEST_BRANCH);
    f.forge().clear_requests();

    let err = stderr(&f.tool(&["sync"]).code(2));
    assert!(err.contains("the merge request !1"), "{err}");
    assert!(err.contains("was rewritten"), "{err}");
    assert!(err.contains("`git intake abort --merge-request`"), "{err}");
    assert!(
        err.contains("`git intake start` and `git intake finish`"),
        "{err}"
    );
    assert_eq!(on_remote(&f, MERGE_REQUEST_BRANCH), rebased);
    assert!(f.forge().changes().is_empty());
    env.assert_no_update(&f.down);
}

#[test]
fn a_fix_amended_by_force_keeps_the_merge_request_in_hand() {
    let f = Fork::merge_request();
    let env = &f.env;
    behind(&f);
    f.tool(&["sync"]).success();
    let dir = common::forge(env, &f.publish);
    env.git(
        &dir,
        &[
            "checkout",
            "-q",
            "-b",
            "fix",
            &format!("origin/{MERGE_REQUEST_BRANCH}"),
        ],
    );
    env.commit(&dir, &[("fix.txt", "first\n")], "fix: first try");
    env.git(
        &dir,
        &[
            "push",
            "-q",
            "origin",
            &format!("HEAD:{MERGE_REQUEST_BRANCH}"),
        ],
    );
    env.commit(&dir, &[("fix.txt", "second\n")], "fix: amended");
    env.git(&dir, &["reset", "-q", "--soft", "HEAD~2"]);
    env.git(&dir, &["commit", "-q", "-m", "fix: the amended fix"]);
    let fix = env.rev(&dir, "HEAD");
    env.git(
        &dir,
        &[
            "push",
            "-q",
            "-f",
            "origin",
            &format!("HEAD:{MERGE_REQUEST_BRANCH}"),
        ],
    );
    env.git(&dir, &["checkout", "-q", "main"]);
    land(&f, &[("other.txt", "o\n")], "other: lands");
    f.tool(&["sync"]).success();
    let after = on_remote(&f, MERGE_REQUEST_BRANCH).unwrap();
    assert_eq!(env.rev(&f.publish, &format!("{after}^2")), fix);
}

#[test]
fn a_merge_request_closed_without_merging_declines_its_targets() {
    let f = Fork::merge_request();
    let env = &f.env;
    behind(&f);
    f.tool(&["sync"]).success();
    f.forge().end(1, "closed");
    // The forge deleted nothing; a person deleted the branch.
    env.git(
        &f.publish,
        &[
            "update-ref",
            "-d",
            &format!("refs/heads/{MERGE_REQUEST_BRANCH}"),
        ],
    );
    let out = stdout(&f.tool(&["sync"]).success());
    assert!(out.contains("merge request !1"), "{out}");
    assert!(
        out.contains("was closed without merging, which declined these targets"),
        "{out}"
    );
    assert_eq!(f.forge().mrs().len(), 1);
    assert_eq!(on_remote(&f, MERGE_REQUEST_BRANCH), None);

    // An upstream moving on brings a new merge request with the new target.
    let moved = env.push_upstream(&f.up, &[("m.txt", "m\n")], "upstream: m");
    let out = stdout(&f.tool(&["sync"]).success());
    assert!(out.contains("opened merge request !2"), "{out}");
    assert!(
        f.forge()
            .mr(2)
            .description
            .contains(&format!("upstream {moved}"))
    );
}

#[test]
fn an_update_by_hand_publishes_declined_targets() {
    let f = Fork::merge_request();
    let env = &f.env;
    behind(&f);
    f.tool(&["sync"]).success();
    f.forge().end(1, "closed");
    env.git(
        &f.publish,
        &[
            "update-ref",
            "-d",
            &format!("refs/heads/{MERGE_REQUEST_BRANCH}"),
        ],
    );
    f.tool(&["start"]).success();
    f.tool(&["finish"])
        .success()
        .stdout(predicates::str::contains("opened merge request !2"));
}

#[test]
fn a_branch_kept_after_a_decline_is_deleted_when_an_upstream_moves() {
    let f = Fork::github();
    let env = &f.env;
    behind(&f);
    f.tool(&["sync"]).success();
    // GitHub keeps the branch of a closed pull request.
    f.forge().end(1, "closed");
    let kept = on_remote(&f, MERGE_REQUEST_BRANCH).unwrap();
    env.push_upstream(&f.up, &[("m.txt", "m\n")], "upstream: m");
    let out = stdout(&f.tool(&["sync"]).success());
    assert!(out.contains("deleting it"), "{out}");
    assert!(out.contains("opened pull request #2"), "{out}");
    let head = on_remote(&f, MERGE_REQUEST_BRANCH).unwrap();
    assert_ne!(head, kept);
    assert!(!env.is_ancestor(&f.publish, &kept, &head));
}

#[test]
fn an_interrupted_job_s_branch_gets_its_merge_request() {
    let f = Fork::merge_request();
    let env = &f.env;
    behind(&f);
    // The job pushed the branch and was killed before it opened the merge
    // request.
    f.forge()
        .fail("POST", "/merge_requests", 500, "500 Internal Server Error");
    f.tool(&["sync"]).code(2);
    env.assert_no_update(&f.down);
    let pushed = on_remote(&f, MERGE_REQUEST_BRANCH).unwrap();
    f.forge().world().failures.clear();

    let ci = ci_clone(&f, "ci");
    let out = stdout(&env.tool(&ci, &["sync"]).success());
    assert!(
        out.contains("holds an update no merge request carries; carrying it on"),
        "{out}"
    );
    assert!(!out.contains("queued at"), "{out}");
    assert!(out.contains("opened merge request !1"), "{out}");
    assert_eq!(on_remote(&f, MERGE_REQUEST_BRANCH), Some(pushed));
}

#[test]
fn a_catch_up_left_after_a_decline_is_deleted() {
    let f = Fork::github();
    let env = &f.env;
    behind(&f);
    f.tool(&["sync"]).success();
    let closed = on_remote(&f, MERGE_REQUEST_BRANCH).unwrap();
    f.forge().end(1, "closed");
    // A catch-up merge was pushed just after the decline.
    let tip = land(&f, &[("other.txt", "o\n")], "other: lands");
    let dir = common::forge(env, &f.publish);
    env.git(&dir, &["checkout", "-q", "--detach", &tip]);
    env.git(
        &dir,
        &[
            "merge",
            "-q",
            "--no-ff",
            "-m",
            "catch-up: merge main\n\nIntake-Catch-Up: main\n",
            &closed,
        ],
    );
    env.git(
        &dir,
        &[
            "push",
            "-q",
            "origin",
            &format!("HEAD:refs/heads/{MERGE_REQUEST_BRANCH}"),
        ],
    );
    env.git(&dir, &["checkout", "-q", "main"]);

    let out = stdout(&f.tool(&["sync"]).success());
    assert!(out.contains("deleting it"), "{out}");
    assert!(out.contains("declined these targets"), "{out}");
    assert_eq!(on_remote(&f, MERGE_REQUEST_BRANCH), None);
    assert_eq!(f.forge().mrs().len(), 1);
}

#[test]
fn a_declined_branch_with_a_fix_is_refused() {
    let f = Fork::github();
    let env = &f.env;
    behind(&f);
    f.tool(&["sync"]).success();
    f.forge().end(1, "closed");
    let dir = common::forge(env, &f.publish);
    env.git(
        &dir,
        &[
            "checkout",
            "-q",
            "-b",
            "fix",
            &format!("origin/{MERGE_REQUEST_BRANCH}"),
        ],
    );
    let fix = env.commit(&dir, &[("fix.txt", "fix\n")], "fix: after the decline");
    env.git(
        &dir,
        &[
            "push",
            "-q",
            "origin",
            &format!("HEAD:{MERGE_REQUEST_BRANCH}"),
        ],
    );
    let err = stderr(&f.tool(&["sync"]).code(2));
    assert!(
        err.contains(&format!("the branch '{MERGE_REQUEST_BRANCH}' on 'origin'")),
        "{err}"
    );
    assert!(err.contains("`git intake abort --merge-request`"), "{err}");
    assert_eq!(on_remote(&f, MERGE_REQUEST_BRANCH), Some(fix));
    assert_eq!(f.forge().mrs().len(), 1);
}

#[test]
fn a_branch_of_unknown_origin_is_refused() {
    let f = Fork::merge_request();
    let env = &f.env;
    behind(&f);
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
    let err = stderr(&f.tool(&["sync"]).code(2));
    assert!(err.contains("holds commits that carry no update"), "{err}");
    assert!(err.contains(&f.forge().remote_url()), "{err}");
    assert!(err.contains("`git intake abort --merge-request`"), "{err}");
    assert!(f.forge().changes().is_empty());
}

#[test]
fn a_squashed_merge_request_is_proposed_again() {
    let f = Fork::github();
    let env = &f.env;
    let target = behind(&f);
    f.tool(&["sync"]).success();
    merge_on_fake(env, f.forge(), 1, How::Squash);
    let sync = f.tool(&["sync"]).success();
    let (out, err) = (stdout(&sync), stderr(&sync));
    assert!(err.contains("warning: the pull request #1"), "{err}");
    assert!(
        err.contains("was merged without the update's history"),
        "{err}"
    );
    assert!(out.contains("opened pull request #2"), "{out}");
    let description = f.forge().mr(2).description;
    assert!(description.contains("The pull request #1"), "{description}");
    assert!(
        description.contains(&format!("upstream {target}")),
        "{description}"
    );
    let status = env.tool_json(&f.down, &["status", "--json"], 0);
    assert_eq!(status["merge_request"]["number"], 2);
    assert_eq!(status["merged_without_history"]["number"], 1);
}

#[test]
fn a_merge_during_the_catch_up_ends_the_run() {
    let f = Fork::merge_request();
    let env = &f.env;
    behind(&f);
    f.tool(&["sync"]).success();
    let before = on_remote(&f, MERGE_REQUEST_BRANCH);
    land(&f, &[("other.txt", "o\n")], "other: lands");
    // A person merges the merge request while the catch-up merge is made.
    let marker = env.path("merged-now");
    f.forge().world().merge_when = Some((1, marker.clone()));
    in_update_worktree_hook(
        &f.down,
        "post-merge",
        &format!("touch {}", marker.display()),
    );

    let out = stdout(&f.tool(&["sync"]).success());
    assert!(out.contains("merge request !1"), "{out}");
    assert!(
        out.contains("was merged before the update was pushed to it; nothing was pushed"),
        "{out}"
    );
    assert_eq!(on_remote(&f, MERGE_REQUEST_BRANCH), before);
    assert_eq!(f.forge().mrs().len(), 1);
    env.assert_no_update(&f.down);
}

#[test]
fn concurrent_runs_push_once() {
    let f = Fork::merge_request();
    let env = &f.env;
    behind(&f);
    // Another run pushes its update while this one merges.
    let dir = common::forge(env, &f.publish);
    let other = env.commit(&dir, &[("o.txt", "o\n")], "another run's update");
    env.git(
        &dir,
        &[
            "push",
            "-q",
            "origin",
            &format!("{other}:refs/heads/other-run"),
        ],
    );
    in_update_worktree_hook(
        &f.down,
        "post-merge",
        &format!(
            "git --git-dir={} update-ref refs/heads/{MERGE_REQUEST_BRANCH} {other}",
            f.publish.display()
        ),
    );
    let err = stderr(&f.tool(&["sync"]).code(2));
    assert!(err.contains("was rejected"), "{err}");
    assert!(err.contains("run the command again"), "{err}");
    assert!(f.forge().changes().is_empty());
    assert_eq!(on_remote(&f, MERGE_REQUEST_BRANCH), Some(other));
    env.assert_no_update(&f.down);
}

#[test]
fn an_update_that_changes_the_pipeline_says_so() {
    let f = Fork::merge_request();
    let env = &f.env;
    env.push_upstream(
        &f.up,
        &[(".gitlab-ci.yml", "test:\n  script: [make]\n")],
        "upstream: ci",
    );
    let out = stdout(&f.tool(&["sync"]).success());
    assert!(
        out.contains("the update changes its own checks: `.gitlab-ci.yml`"),
        "{out}"
    );
    let description = f.forge().mr(1).description;
    assert!(
        description.contains("This update changes its own checks"),
        "{description}"
    );
    assert!(description.contains("`.gitlab-ci.yml`"), "{description}");
}

#[test]
fn push_mode_sync_verifies_and_publishes() {
    let f = Fork::new();
    let env = &f.env;
    let target = env.push_upstream(&f.up, &[("n.txt", "n\n")], "upstream: n");
    let out = stdout(&f.tool(&["sync"]).success());
    assert!(out.contains("passed   app"), "{out}");
    assert!(out.contains("pushed main to 'origin'"), "{out}");
    assert!(env.is_ancestor(&f.publish, &target, "main"));
    env.assert_no_update(&f.down);
}

#[test]
fn push_mode_sync_stops_on_failed_checks() {
    let f = Fork::with_checks(&[("false", "false")]);
    let env = &f.env;
    env.push_upstream(&f.up, &[("n.txt", "n\n")], "upstream: n");
    let published = env.rev(&f.publish, "main");
    let out = stdout(&f.tool(&["sync"]).code(1));
    assert!(
        out.contains("the checks failed; nothing was published"),
        "{out}"
    );
    assert_eq!(env.rev(&f.publish, "main"), published);
    let status = env.tool_json(&f.down, &["status", "--json"], 0);
    assert_eq!(status["update"]["step"], "verification_failed");
}

#[test]
fn sync_refuses_in_a_shallow_clone() {
    let f = Fork::merge_request();
    let env = &f.env;
    behind(&f);
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
    let err = stderr(&env.tool(&dir, &["sync"]).code(2));
    assert!(err.contains("shallow clone"), "{err}");
    assert!(err.contains("`git fetch --unshallow`"), "{err}");
    assert!(f.forge().requests().is_empty());
}

/// Installs a hook of the repository at `repo` that runs `script` when git
/// runs it in the update worktree.
fn in_update_worktree_hook(repo: &Path, name: &str, script: &str) {
    let path = repo.join(".git").join("hooks").join(name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(
        &path,
        format!(
            "#!/bin/sh\ncase \"$(git rev-parse --git-dir)\" in\n*/worktrees/*) {script} ;;\nesac\n"
        ),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
}

/// A merge request opened by `sync` whose catch-up stopped on a conflict in
/// this clone, resolved and concluded with `continue`.
fn resolved_catch_up(f: &Fork) {
    let env = &f.env;
    f.reconfigure("run = \"test -f app.txt\"", "run = \"false\"");
    env.push_upstream(
        &f.up,
        &[("app.txt", "upstream\ntwo\nthree\n")],
        "upstream: change",
    );
    f.tool(&["sync"]).success();
    land(f, &[("app.txt", "fork\ntwo\nthree\n")], "fork: change");
    f.tool(&["sync"]).code(1);
    let wt = env.update_worktree(&f.down);
    fs::write(wt.join("app.txt"), "both\ntwo\nthree\n").unwrap();
    env.git(&wt, &["add", "app.txt"]);
    f.tool(&["continue"]).success();
}

#[test]
fn finish_reads_the_open_merge_request_again_before_the_push() {
    let f = Fork::merge_request();
    let env = &f.env;
    resolved_catch_up(&f);
    let before = on_remote(&f, MERGE_REQUEST_BRANCH);
    // The integration branch moves again, and a person merges the merge
    // request while `finish` makes its catch-up merge.
    land(&f, &[("other.txt", "o\n")], "other: lands");
    let marker = env.path("merged-now");
    f.forge().world().merge_when = Some((1, marker.clone()));
    in_update_worktree_hook(
        &f.down,
        "post-merge",
        &format!("touch {}", marker.display()),
    );
    let err = stderr(&f.tool(&["finish"]).code(2));
    assert!(err.contains("the merge request !1"), "{err}");
    assert!(
        err.contains("is now merged, so the update is not published to it; nothing was pushed"),
        "{err}"
    );
    assert!(err.contains("`git intake abort`"), "{err}");
    assert_eq!(on_remote(&f, MERGE_REQUEST_BRANCH), before);
    assert!(env.update_worktree(&f.down).exists(), "the update is kept");
}

#[test]
fn finish_deletes_the_branch_a_push_recreated_after_a_merge() {
    let f = Fork::merge_request();
    let env = &f.env;
    resolved_catch_up(&f);
    // The merge request is merged, and its branch removed, in the instant
    // between the last read and the push.
    let marker = env.path("merged-now");
    f.forge().world().merge_when = Some((1, marker.clone()));
    let hook = f.down.join(".git").join("hooks").join("pre-push");
    fs::write(&hook, format!("#!/bin/sh\ntouch {}\n", marker.display())).unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
    f.forge().clear_requests();
    let err = stderr(&f.tool(&["finish"]).code(2));
    assert!(err.contains("the merge request !1"), "{err}");
    assert!(err.contains("is now merged"), "{err}");
    assert!(
        err.contains("the branch the push recreated after it ended was deleted again"),
        "{err}"
    );
    assert!(err.contains("`git intake abort`"), "{err}");
    assert_eq!(on_remote(&f, MERGE_REQUEST_BRANCH), None);
    assert!(env.update_worktree(&f.down).exists(), "the update is kept");
    assert!(
        !f.forge()
            .changes()
            .iter()
            .any(|r| r.json().get("title").is_some()),
        "{:?}",
        f.forge().changes()
    );
}

#[test]
fn sync_carries_a_port_update_on_from_its_history() {
    let f = Fork::port_merge_request();
    let env = &f.env;
    let c0 = env.push_upstream(
        &f.up,
        &[("app.txt", "ported\ntwo\nthree\n")],
        "upstream: c0",
    );
    let c1 = env.push_upstream(&f.up, &[("skipped.txt", "s\n")], "upstream: c1");
    f.tool(&["start"]).success();
    let plan = env.intake_dir(&f.down).join("plan");
    fs::write(
        &plan,
        format!("take {} \nskip {} # not ours\n", &c0[..12], &c1[..12]),
    )
    .unwrap();
    f.tool(&["continue"]).success();
    f.tool(&["finish"]).success();
    let before = on_remote(&f, MERGE_REQUEST_BRANCH).unwrap();
    // The integration branch changes the file the copy changed.
    land(&f, &[("app.txt", "fork\ntwo\nthree\n")], "fork: change");

    // A fresh `sync` rebuilds the port's plan from the merge request's
    // history and stops on the catch-up's conflict.
    f.tool(&["sync"]).code(1);
    let status = env.tool_json(&f.down, &["status", "--json"], 0);
    let up = &status["update"]["upstreams"][0];
    assert_eq!(up["mode"], "port");
    assert_eq!(up["step"], "merged");
    let commits = up["commits"].as_array().unwrap();
    let verbs: Vec<(&str, &str)> = commits
        .iter()
        .map(|c| (c["commit"].as_str().unwrap(), c["verb"].as_str().unwrap()))
        .collect();
    assert_eq!(verbs, [(c0.as_str(), "take"), (c1.as_str(), "skip")]);
    assert_eq!(commits[0]["copy"], "copied");
    assert!(up["journal_commit"].is_string());

    // Resolved, the catch-up merge is published: `finish` accepts the
    // copies and the journal commit the plan was rebuilt from.
    let wt = env.update_worktree(&f.down);
    fs::write(wt.join("app.txt"), "both\ntwo\nthree\n").unwrap();
    env.git(&wt, &["add", "app.txt"]);
    f.tool(&["continue"]).success();
    let json = env.tool_json(&wt, &["env", "--json"], 0);
    assert_eq!(json["GIT_INTAKE_UPSTREAM_UPSTREAM_TARGET"], c1.as_str());
    f.tool(&["finish"]).success();
    let after = on_remote(&f, MERGE_REQUEST_BRANCH).unwrap();
    assert_eq!(env.rev(&f.publish, &format!("{after}^2")), before);
    let description = f.forge().mr(1).description;
    assert!(
        description.contains(&format!(
            "`upstream` from upstream/main, ported: took {}; skipped {}",
            &c0[..12],
            &c1[..12]
        )),
        "{description}"
    );
    env.assert_no_update(&f.down);
}

/// A pre-push hook of the fork that, at the push of the update, has the
/// forge merge merge request 1, and at the next push, the deletion of the
/// branch it recreated, moves that branch on the remote, as someone pushing
/// in between does, so that the deletion's lease fails.
fn merge_then_move_on_push(f: &Fork) {
    let env = &f.env;
    let marker = env.path("merged-now");
    let count = env.path("pushes");
    f.forge().world().merge_when = Some((1, marker.clone()));
    let main = env.rev(&f.publish, "main");
    let hook = f.down.join(".git").join("hooks").join("pre-push");
    fs::write(
        &hook,
        format!(
            "#!/bin/sh\nif [ -e {count} ]; then\n  git --git-dir={bare} update-ref refs/heads/{MERGE_REQUEST_BRANCH} {main}\n\
             else\n  touch {count} {marker}\nfi\n",
            count = count.display(),
            marker = marker.display(),
            bare = f.publish.display(),
        ),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
fn finish_keeps_a_recreated_branch_that_moved_and_says_so() {
    let f = Fork::merge_request();
    let env = &f.env;
    resolved_catch_up(&f);
    merge_then_move_on_push(&f);
    let main = env.rev(&f.publish, "main");
    let err = stderr(&f.tool(&["finish"]).code(2));
    assert!(err.contains("is now merged"), "{err}");
    assert!(
        err.contains("the branch the push recreated after it ended could not be deleted"),
        "{err}"
    );
    assert!(err.contains("`git intake abort --merge-request`"), "{err}");
    assert!(!err.contains("deleted again"), "{err}");
    assert_eq!(on_remote(&f, MERGE_REQUEST_BRANCH), Some(main));
    assert!(env.update_worktree(&f.down).exists(), "the update is kept");
}

#[test]
fn sync_keeps_a_recreated_branch_that_moved_and_says_so() {
    let f = Fork::merge_request();
    let env = &f.env;
    behind(&f);
    f.tool(&["sync"]).success();
    land(&f, &[("other.txt", "o\n")], "other: lands");
    merge_then_move_on_push(&f);
    let main = env.rev(&f.publish, "main");
    let out = stdout(&f.tool(&["sync"]).success());
    assert!(
        out.contains("the branch the push recreated after it ended could not be deleted"),
        "{out}"
    );
    assert!(out.contains("`git intake abort --merge-request`"), "{out}");
    assert_eq!(on_remote(&f, MERGE_REQUEST_BRANCH), Some(main));
    env.assert_no_update(&f.down);
}

#[test]
fn finish_refuses_an_open_merge_request_whose_branch_is_gone() {
    let f = Fork::merge_request();
    let env = &f.env;
    resolved_catch_up(&f);
    let before = on_remote(&f, MERGE_REQUEST_BRANCH).unwrap();
    f.forge().pin_head(1, &before);
    env.git(
        &f.publish,
        &[
            "update-ref",
            "-d",
            &format!("refs/heads/{MERGE_REQUEST_BRANCH}"),
        ],
    );
    f.forge().clear_requests();
    let err = stderr(&f.tool(&["finish"]).code(2));
    assert!(err.contains("the merge request !1"), "{err}");
    assert!(
        err.contains(&format!(
            "is open, but its branch '{MERGE_REQUEST_BRANCH}' is not on 'origin'"
        )),
        "{err}"
    );
    assert!(err.contains("`git intake abort --merge-request`"), "{err}");
    assert!(!err.contains("run the command again"), "{err}");
    assert_eq!(on_remote(&f, MERGE_REQUEST_BRANCH), None);
    assert!(f.forge().changes().is_empty());
    assert!(env.update_worktree(&f.down).exists(), "the update is kept");
}
