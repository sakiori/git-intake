//! Catching up with a moved integration branch: a catch-up merge takes the
//! new integration tip into the update, `finish` verifies and publishes it,
//! and the update branch only moves forward.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::{FORK_FILE, Fork, stderr, stdout};
use predicates::str::contains;

const TRAILER: &str = "Intake-Catch-Up: main";

/// Starts and verifies an update that takes `n.txt` from the upstream.
/// Returns the verified commit and the upstream target.
fn verify_update(f: &Fork) -> (String, String) {
    let env = &f.env;
    let target = env.push_upstream(&f.up, &[("n.txt", "n\n")], "upstream: n");
    f.tool(&["start"]).success();
    f.tool(&["verify"]).success();
    (env.rev(&env.update_worktree(&f.down), "HEAD"), target)
}

/// A clone of the publish remote where other work lands.
fn other(f: &Fork) -> PathBuf {
    let env = &f.env;
    let dir = env.path("other");
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
        env.git(&dir, &["pull", "-q", "--ff-only", "origin", "main"]);
    }
    dir
}

/// Other work lands on the publish remote's integration branch; returns it.
fn land(f: &Fork, files: &[(&str, &str)], message: &str) -> String {
    let dir = other(f);
    let c = f.env.commit(&dir, files, message);
    f.env.git(&dir, &["push", "-q", "origin", "HEAD:main"]);
    c
}

fn parents(f: &Fork, rev: &str) -> Vec<String> {
    f.env
        .git(&f.down, &["rev-list", "--parents", "-n", "1", rev])
        .split_whitespace()
        .skip(1)
        .map(str::to_string)
        .collect()
}

fn message(f: &Fork, rev: &str) -> String {
    f.env.git(&f.down, &["log", "-1", "--format=%B", rev])
}

fn edit_state(repo: &Path, edit: impl FnOnce(&mut serde_json::Value)) {
    let path = repo.join(".git").join("git-intake").join("state.json");
    let mut state: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    edit(&mut state);
    fs::write(&path, serde_json::to_string_pretty(&state).unwrap()).unwrap();
}

fn read_state(repo: &Path) -> serde_json::Value {
    let path = repo.join(".git").join("git-intake").join("state.json");
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

/// Makes the publish remote reject every push until [`accept_pushes`].
fn reject_pushes(f: &Fork) {
    let hook = f.publish.join("hooks").join("pre-receive");
    fs::create_dir_all(hook.parent().unwrap()).unwrap();
    fs::write(&hook, "#!/bin/sh\necho 'rejected for now' >&2\nexit 1\n").unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
}

fn accept_pushes(f: &Fork) {
    fs::remove_file(f.publish.join("hooks").join("pre-receive")).unwrap();
}

/// A conflicting commit on the integration branch: `n.txt`, which the update
/// adds with other content.
fn conflicting(f: &Fork) -> String {
    land(f, &[("n.txt", "theirs\n")], "other: add n")
}

#[test]
fn finish_catches_up_with_a_local_commit_and_publishes() {
    let f = Fork::new();
    let env = &f.env;
    let base = env.rev(&f.down, "main");
    let (verified, target) = verify_update(&f);
    let before: Vec<String> = env
        .git(&f.down, &["rev-list", "git-intake/update"])
        .lines()
        .map(str::to_string)
        .collect();
    let local = env.commit(&f.down, &[("local.txt", "l\n")], "fork: local work");

    let out = stdout(&f.tool(&["finish"]).success());
    assert!(
        out.contains(&format!(
            "catching up with 'main' at {}: merging the update at {} into it",
            &local[..12],
            &verified[..12]
        )),
        "{out}"
    );
    assert!(out.contains("caught up with 'main' at"), "{out}");
    assert!(out.contains("passed   test -f app.txt"), "{out}");
    assert!(out.contains("pushed main to 'origin'"), "{out}");
    let merge = env.rev(&f.down, "main");
    assert_eq!(parents(&f, &merge), [local.clone(), verified.clone()]);
    assert!(message(&f, &merge).contains(TRAILER));
    assert_eq!(env.rev(&f.publish, "main"), merge);
    // Every commit of the update branch stays as it was.
    for c in &before {
        assert!(env.is_ancestor(&f.down, c, &merge), "{c}");
    }
    assert!(f.down.join("n.txt").exists() && f.down.join("local.txt").exists());
    env.assert_no_update(&f.down);

    // The update counts as merged straight in.
    let status = env.tool_json(&f.down, &["status", "--json"], 0);
    let up = &status["upstreams"][0];
    assert_eq!(up["state"], "current");
    assert_eq!(up["last_synced"], target.as_str());
    assert_eq!(up["last_synced_source"], "record");
    let report = env.tool_json(&f.down, &["report", "--json"], 0);
    assert_eq!(report["base"], base.as_str());
    assert_eq!(report["catch_ups"][0]["tip"], local.as_str());
    assert_eq!(report["catch_ups"][0]["merge_commit"], merge.as_str());
}

#[test]
fn resolutions_and_fixes_made_by_hand_are_kept() {
    let f = Fork::new();
    let env = &f.env;
    env.commit(
        &f.down,
        &[(FORK_FILE, "one\nTWO-fork\nthree\n")],
        "fork: change two",
    );
    env.git(&f.down, &["push", "-q", "origin", "main"]);
    env.push_upstream(
        &f.up,
        &[(FORK_FILE, "one\nTWO-upstream\nthree\n")],
        "upstream: change two",
    );
    f.tool(&["start"]).code(1);
    let wt = env.update_worktree(&f.down);
    // The resolution edits a file beyond the conflicting hunk, and a fix is
    // committed on top.
    fs::write(wt.join(FORK_FILE), "one\nTWO-both\nthree\n").unwrap();
    fs::write(wt.join("build.txt"), "fixed while resolving\n").unwrap();
    env.git(&wt, &["add", FORK_FILE, "build.txt"]);
    f.tool(&["continue"]).success();
    env.commit(&wt, &[("fix.txt", "fix\n")], "fork: adapt to upstream");
    f.tool(&["verify"]).success();
    let other_work = land(&f, &[("other.txt", "o\n")], "other: lands first");

    f.tool(&["finish"]).success();
    let merge = env.rev(&f.down, "main");
    assert_eq!(env.rev(&f.down, "main^1"), other_work);
    assert!(message(&f, &merge).contains(TRAILER));
    let show = |path: &str| env.git(&f.down, &["show", &format!("{merge}:{path}")]);
    assert_eq!(show(FORK_FILE), "one\nTWO-both\nthree");
    assert_eq!(show("build.txt"), "fixed while resolving");
    assert_eq!(show("fix.txt"), "fix");
    assert_eq!(show("other.txt"), "o");
}

#[test]
fn a_conflicting_catch_up_stops_and_continue_concludes_it() {
    let f = Fork::new();
    let env = &f.env;
    let base = env.rev(&f.down, "main");
    let (verified, target) = verify_update(&f);
    let theirs = conflicting(&f);
    let published = env.rev(&f.publish, "main");

    // `finish` brings the local branch up to date, which is not a move of
    // its own, and stops on the conflict.
    let stopped = f.tool(&["finish"]).code(1);
    let out = stdout(&stopped);
    assert!(
        out.contains(&format!(
            "fast-forwarded 'main' from {} to {} to match 'origin/main'",
            &base[..12],
            &theirs[..12]
        )),
        "{out}"
    );
    assert!(
        out.contains(&format!(
            "ours is 'main' at {}, theirs is the update at {}",
            &theirs[..12],
            &verified[..12]
        )),
        "{out}"
    );
    assert!(out.contains("n.txt  (unresolved)"), "{out}");
    assert!(out.contains("run `git intake continue`"), "{out}");
    assert_eq!(env.rev(&f.down, "main"), theirs);
    assert_eq!(env.rev(&f.publish, "main"), published);
    assert_eq!(env.rev(&f.down, "git-intake/update"), verified);

    let status = env.tool_json(&f.down, &["status", "--json"], 0);
    let update = &status["update"];
    assert_eq!(update["step"], "catch_up_conflict");
    assert_eq!(update["catch_up"]["tip"], theirs.as_str());
    assert_eq!(update["catch_up"]["conflicts"][0]["path"], "n.txt");
    assert!(
        update["next"]
            .as_str()
            .unwrap()
            .contains("git intake continue"),
        "{update}"
    );
    f.tool(&["status"])
        .success()
        .stdout(contains("update in progress: catch up conflict"))
        .stdout(contains(format!(
            "catching up with the integration tip {}",
            &theirs[..12]
        )));
    let doctor = env.tool_json(&f.down, &["doctor", "--json"], 0);
    assert_eq!(doctor["healthy"], true, "{doctor}");
    f.tool(&["verify"])
        .code(2)
        .stderr(contains("run `git intake continue` before verifying"));
    f.tool(&["finish"])
        .code(2)
        .stderr(contains("then run `git intake continue`"));
    f.tool(&["continue"])
        .code(2)
        .stderr(contains("still has unresolved conflicts"));

    let wt = env.update_worktree(&f.down);
    fs::write(wt.join("n.txt"), "both\n").unwrap();
    env.git(&wt, &["add", "n.txt"]);
    f.tool(&["continue"])
        .success()
        .stdout(contains("caught up with 'main' at"))
        .stdout(contains("next: git intake finish"));
    let merge = env.rev(&f.down, "git-intake/update");
    assert_eq!(parents(&f, &merge), [theirs.clone(), verified.clone()]);
    assert!(message(&f, &merge).contains(TRAILER));
    assert_eq!(
        env.git(&wt, &["symbolic-ref", "HEAD"]),
        "refs/heads/git-intake/update"
    );
    let status = env.tool_json(&f.down, &["status", "--json"], 0);
    assert_eq!(status["update"]["step"], "catch_up");
    assert_eq!(status["update"]["next"], "git intake finish");
    assert_eq!(status["update"]["base"], theirs.as_str());
    assert_eq!(status["update"]["start_base"], base.as_str());
    let doctor = env.tool_json(&f.down, &["doctor", "--json"], 0);
    assert_eq!(doctor["healthy"], true, "{doctor}");

    // The report lists the catch-up with its conflict and measures the
    // upstream from the base the update started from.
    let report = env.tool_json(&f.down, &["report", "--json"], 0);
    assert_eq!(report["base"], base.as_str());
    assert_eq!(report["catch_ups"][0]["tip"], theirs.as_str());
    assert_eq!(report["catch_ups"][0]["conflicts"][0]["path"], "n.txt");
    assert_eq!(
        report["catch_ups"][0]["conflicts"][0]["resolution"],
        "manual"
    );
    let up = &report["upstreams"][0];
    assert_eq!(up["target"], target.as_str());
    assert_eq!(up["commits"], 1);
    assert_eq!(up["own_changes_before"]["files"], 1);
    f.tool(&["report"])
        .success()
        .stdout(contains(format!("update of main from {}", &base[..12])))
        .stdout(contains(format!(
            "caught up with main at {}",
            &theirs[..12]
        )))
        .stdout(contains("n.txt  (resolved by hand)"));

    let out = stdout(&f.tool(&["finish"]).success());
    assert!(out.contains(&format!("verified {}", &merge[..12])), "{out}");
    assert!(!out.contains("catching up"), "{out}");
    assert_eq!(env.rev(&f.publish, "main"), merge);
    assert_eq!(env.rev(&f.down, "main"), merge);
    env.assert_no_update(&f.down);
}

#[test]
fn failed_checks_after_a_catch_up_leave_it_unverified() {
    let f = Fork::with_checks(&["test ! -f breaks.txt"]);
    let env = &f.env;
    let (verified, _) = verify_update(&f);
    let local = env.commit(&f.down, &[("breaks.txt", "b\n")], "fork: breaks");

    let out = stdout(&f.tool(&["finish"]).code(1));
    assert!(out.contains("FAILED   test ! -f breaks.txt"), "{out}");
    assert!(out.contains("verification of the catch-up merge"), "{out}");
    let merge = env.rev(&f.down, "git-intake/update");
    assert_eq!(parents(&f, &merge), [local.clone(), verified]);
    assert_eq!(env.rev(&f.down, "main"), local);
    let status = env.tool_json(&f.down, &["status", "--json"], 0);
    assert_eq!(status["update"]["step"], "catch_up");
    assert_eq!(status["update"]["verified_commit"], serde_json::Value::Null);

    // A commit on top of the catch-up merge needs `verify`.
    let wt = env.update_worktree(&f.down);
    env.git(&wt, &["rm", "-q", "breaks.txt"]);
    env.git(&wt, &["commit", "-q", "-m", "fork: fix"]);
    f.tool(&["finish"])
        .code(2)
        .stderr(contains("run `git intake verify` for the new commit"));
    assert_eq!(env.rev(&f.down, "main"), local);
    f.tool(&["verify"]).success();
    f.tool(&["finish"]).success();
    assert_eq!(env.rev(&f.publish, "main"), env.rev(&f.down, "main"));
    assert_eq!(env.rev(&f.down, "main^1"), merge);
}

#[test]
fn a_move_during_verification_stops_and_the_next_finish_catches_up_again() {
    // The check lands a commit on the integration branch once, while
    // `finish` verifies the first catch-up merge.
    let f = Fork::with_checks(&[
        "test -f app.txt",
        "if [ -f \"$HOME/move\" ]; then rm \"$HOME/move\"; \
         git -C \"$(cat \"$HOME/down\")\" commit -q --allow-empty -m 'fork: meanwhile'; fi",
    ]);
    let env = &f.env;
    fs::write(
        env.path("home").join("down"),
        f.down.to_string_lossy().as_bytes(),
    )
    .unwrap();
    let (verified, target) = verify_update(&f);
    let first = env.commit(&f.down, &[("local.txt", "l\n")], "fork: local work");
    fs::write(env.path("home").join("move"), "").unwrap();

    let out = stdout(&f.tool(&["finish"]).code(1));
    assert!(
        out.contains("while the update was verified; no branch moved to the update"),
        "{out}"
    );
    assert!(out.contains("Run `git intake finish` again"), "{out}");
    let caught = env.rev(&f.down, "git-intake/update");
    assert_eq!(parents(&f, &caught), [first.clone(), verified.clone()]);
    let meanwhile = env.rev(&f.down, "main");
    assert_eq!(parents(&f, &meanwhile), std::slice::from_ref(&first));
    assert_eq!(env.rev(&f.publish, "main"), env.rev(&f.down, "main~2"));

    // The next run catches up once more: the update branch only moved
    // forward, and the second catch-up merge has the first as its second
    // parent.
    f.tool(&["finish"]).success();
    let second = env.rev(&f.down, "main");
    assert_eq!(parents(&f, &second), [meanwhile, caught.clone()]);
    assert!(message(&f, &second).contains(TRAILER));
    assert_eq!(env.rev(&f.publish, "main"), second);

    // An update that caught up twice counts as merged straight in.
    let status = env.tool_json(&f.down, &["status", "--json"], 0);
    let up = &status["upstreams"][0];
    assert_eq!(up["state"], "current");
    assert_eq!(up["last_synced"], target.as_str());
    assert_eq!(up["last_synced_source"], "record");
    let report = env.tool_json(&f.down, &["report", "--json"], 0);
    assert_eq!(report["catch_ups"].as_array().unwrap().len(), 2);
}

#[test]
fn an_integration_branch_that_holds_the_update_needs_no_catch_up() {
    let f = Fork::new();
    let env = &f.env;
    let (verified, _) = verify_update(&f);
    // The integration branch was moved to the verified commit, and past it,
    // by other means.
    env.git(&f.down, &["merge", "-q", "--ff-only", "git-intake/update"]);
    let past = env.commit(&f.down, &[("later.txt", "l\n")], "fork: later work");

    let out = stdout(&f.tool(&["finish"]).success());
    assert!(!out.contains("catching up"), "{out}");
    assert!(
        out.contains(&format!(
            "already contains the verified commit {}",
            &verified[..12]
        )),
        "{out}"
    );
    assert_eq!(env.rev(&f.publish, "main"), verified);
    assert_eq!(env.rev(&f.down, "main"), past);
    env.assert_no_update(&f.down);
}

#[test]
fn a_moved_publish_remote_is_followed_caught_up_with_and_published() {
    let f = Fork::new();
    let env = &f.env;
    let (verified, _) = verify_update(&f);
    let theirs = land(&f, &[("other.txt", "o\n")], "other: published first");

    let out = stdout(&f.tool(&["finish"]).success());
    assert!(out.contains("fast-forwarded 'main' from"), "{out}");
    assert!(out.contains("pushed main to 'origin'"), "{out}");
    let merge = env.rev(&f.publish, "main");
    assert_eq!(parents(&f, &merge), [theirs, verified]);
    assert_eq!(env.rev(&f.down, "main"), merge);
    env.assert_no_update(&f.down);
}

#[test]
fn a_local_branch_diverged_from_the_remote_is_refused() {
    let f = Fork::new();
    let env = &f.env;
    verify_update(&f);
    land(&f, &[("other.txt", "o\n")], "other: published first");
    let local = env.commit(&f.down, &[("local.txt", "l\n")], "fork: local work");

    let err = stderr(&f.tool(&["finish"]).code(2));
    assert!(
        err.contains("'main' and 'origin/main' have diverged: 'main' lacks 1 commit of 'origin/main', which lacks 1 of 'main'"),
        "{err}"
    );
    assert!(
        err.contains("The local commits must be published or reconciled first"),
        "{err}"
    );
    assert_eq!(env.rev(&f.down, "main"), local);
    let status = env.tool_json(&f.down, &["status", "--json"], 0);
    assert_eq!(status["update"]["step"], "verified");
}

#[test]
fn an_unreachable_publish_remote_is_refused_and_no_publish_closes_locally() {
    let f = Fork::new();
    let env = &f.env;
    let base = env.rev(&f.down, "main");
    let (verified, _) = verify_update(&f);
    env.git(
        &f.down,
        &[
            "remote",
            "set-url",
            "origin",
            &env.path("missing.git").to_string_lossy(),
        ],
    );
    let err = stderr(&f.tool(&["finish"]).code(2));
    assert!(
        err.contains("cannot fetch the publish remote 'origin'"),
        "{err}"
    );
    assert_eq!(env.rev(&f.down, "main"), base);

    // Without fetching, a local commit is all there is to catch up with.
    let local = env.commit(&f.down, &[("local.txt", "l\n")], "fork: local work");
    f.tool(&["finish", "--no-publish"])
        .success()
        .stdout(contains(
            "not published to 'origin' (advanced locally only): main",
        ));
    assert_eq!(parents(&f, "main"), [local, verified]);
    env.assert_no_update(&f.down);
}

#[test]
fn a_push_rejected_after_the_move_is_caught_up_on_the_next_finish() {
    let f = Fork::new();
    let env = &f.env;
    let base = env.rev(&f.down, "main");
    let (verified, _) = verify_update(&f);
    reject_pushes(&f);
    f.tool(&["finish"])
        .code(2)
        .stderr(contains("pushing to 'origin' was rejected"))
        .stderr(contains(
            "catches up with it, verifies the result and pushes that",
        ));
    assert_eq!(env.rev(&f.down, "main"), verified);
    accept_pushes(&f);
    let theirs = land(&f, &[("other.txt", "o\n")], "other: published meanwhile");
    f.tool(&["abort"])
        .code(2)
        .stderr(contains("`finish` has already moved main"));

    let out = stdout(&f.tool(&["finish"]).success());
    assert!(out.contains("resuming finish"), "{out}");
    assert!(
        out.contains(&format!("catching up with 'main' at {}", &theirs[..12])),
        "{out}"
    );
    let merge = env.rev(&f.publish, "main");
    assert!(
        out.contains(&format!("main: {} -> {}", &verified[..12], &merge[..12])),
        "{out}"
    );
    assert_eq!(parents(&f, &merge), [theirs, verified]);
    assert_eq!(env.rev(&f.down, "main"), merge);
    assert!(env.is_ancestor(&f.down, &base, &merge));
    env.assert_no_update(&f.down);
}

#[test]
fn abort_after_the_move_is_refused_and_no_publish_closes_at_the_planned_commit() {
    let f = Fork::new();
    let env = &f.env;
    let (verified, _) = verify_update(&f);
    reject_pushes(&f);
    f.tool(&["finish"]).code(2);
    accept_pushes(&f);
    let theirs = conflicting(&f);

    f.tool(&["finish"])
        .code(1)
        .stdout(contains("n.txt  (unresolved)"));
    let status = env.tool_json(&f.down, &["status", "--json"], 0);
    assert_eq!(status["update"]["step"], "catch_up_conflict");
    let err = stderr(&f.tool(&["abort"]).code(2));
    assert!(err.contains("Run `git intake finish`"), "{err}");
    assert!(err.contains("`git intake finish --no-publish`"), "{err}");

    f.tool(&["finish", "--no-publish"])
        .success()
        .stdout(contains("dropped the catch-up of the update"))
        .stdout(contains(
            "not published to 'origin' (advanced locally only): main",
        ));
    assert_eq!(env.rev(&f.down, "main"), verified);
    assert_eq!(env.rev(&f.publish, "main"), theirs);
    env.assert_no_update(&f.down);
}

#[test]
fn a_local_commit_and_a_rejected_push_make_no_catch_up() {
    let f = Fork::new();
    let env = &f.env;
    let (verified, _) = verify_update(&f);
    reject_pushes(&f);
    f.tool(&["finish"]).code(2);
    accept_pushes(&f);
    let local = env.commit(&f.down, &[("local.txt", "l\n")], "fork: local work");
    let theirs = land(&f, &[("other.txt", "o\n")], "other: published meanwhile");

    let err = stderr(&f.tool(&["finish"]).code(2));
    assert!(err.contains("the update makes no catch-up merge"), "{err}");
    assert!(err.contains("`git intake finish --no-publish`"), "{err}");
    assert_eq!(env.rev(&f.down, "main"), local);
    assert_eq!(env.rev(&f.publish, "main"), theirs);
    assert_eq!(env.rev(&f.down, "git-intake/update"), verified);
}

#[test]
fn a_rewritten_integration_branch_is_refused() {
    let f = Fork::new();
    let env = &f.env;
    let (verified, _) = verify_update(&f);
    env.git(
        &f.down,
        &["commit", "-q", "--amend", "-m", "fork: rewritten"],
    );
    env.git(&f.down, &["push", "-q", "-f", "origin", "main"]);
    let rewritten = env.rev(&f.down, "main");

    let err = stderr(&f.tool(&["finish"]).code(2));
    assert!(err.contains("'main' was rewritten"), "{err}");
    assert!(err.contains("Run `git intake abort`"), "{err}");
    assert_eq!(env.rev(&f.down, "main"), rewritten);
    assert_eq!(env.rev(&f.down, "git-intake/update"), verified);
    assert!(
        read_state(&f.down)["catch_ups"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn commits_that_change_what_the_update_means_are_refused() {
    // Another clone took the upstream in.
    let f = Fork::new();
    let env = &f.env;
    let (verified, target) = verify_update(&f);
    let record = land(
        &f,
        &[],
        &format!(
            "sync(upstream): merge upstream/main at {}\n\nIntake-Upstream: upstream\nIntake-Target: {target}\n",
            &target[..12]
        ),
    );
    let err = stderr(&f.tool(&["finish"]).code(2));
    assert!(
        err.contains(&format!(
            "{} carries a sync record or a copy of upstream 'upstream'",
            &record[..12]
        )),
        "{err}"
    );
    assert!(err.contains("Run `git intake abort`"), "{err}");
    assert_eq!(env.rev(&f.down, "git-intake/update"), verified);

    // The configuration changed an upstream.
    let f = Fork::new();
    let env = &f.env;
    let (verified, _) = verify_update(&f);
    let dir = other(&f);
    let cfg = fs::read_to_string(dir.join(".git-intake.toml"))
        .unwrap()
        .replace("branch = \"main\"\n", "branch = \"main\"\ntags = \"v*\"\n");
    land(
        &f,
        &[(".git-intake.toml", &cfg)],
        "other: take release tags",
    );
    let err = stderr(&f.tool(&["finish"]).code(2));
    assert!(
        err.contains("upstream 'upstream' changed in .git-intake.toml"),
        "{err}"
    );
    assert!(err.contains("Run `git intake abort`"), "{err}");
    assert_eq!(env.rev(&f.down, "git-intake/update"), verified);
    let status = env.tool_json(&f.down, &["status", "--json"], 0);
    assert_eq!(status["update"]["step"], "verified");
}

#[test]
fn checks_added_on_the_integration_branch_verify_the_catch_up() {
    let f = Fork::new();
    let env = &f.env;
    verify_update(&f);
    let dir = other(&f);
    let cfg = fs::read_to_string(dir.join(".git-intake.toml"))
        .unwrap()
        .replace(
            "checks = [\"test -f app.txt\"]",
            "checks = [\"test -f app.txt\", \"touch \\\"$HOME/added-check-ran\\\"\"]",
        );
    land(&f, &[(".git-intake.toml", &cfg)], "other: add a check");

    let out = stdout(&f.tool(&["finish"]).success());
    assert!(out.contains("passed   touch"), "{out}");
    assert!(env.path("home").join("added-check-ran").exists());
    env.assert_no_update(&f.down);
}

#[test]
fn continue_moves_the_branch_to_a_catch_up_merge_committed_before_an_interruption() {
    let f = Fork::new();
    let env = &f.env;
    let (verified, _) = verify_update(&f);
    let theirs = conflicting(&f);
    f.tool(&["finish"]).code(1);
    // The merge was committed, and the tool stopped before moving the
    // update branch.
    let wt = env.update_worktree(&f.down);
    fs::write(wt.join("n.txt"), "both\n").unwrap();
    env.git(&wt, &["add", "n.txt"]);
    env.git(&wt, &["commit", "-q", "--no-edit"]);
    let merge = env.rev(&wt, "HEAD");
    assert_eq!(env.rev(&f.down, "git-intake/update"), verified);

    f.tool(&["continue"]).success();
    assert_eq!(env.rev(&f.down, "git-intake/update"), merge);
    assert_eq!(parents(&f, &merge), [theirs, verified]);
    assert_eq!(
        env.git(&wt, &["symbolic-ref", "HEAD"]),
        "refs/heads/git-intake/update"
    );
    f.tool(&["finish"]).success();
    assert_eq!(env.rev(&f.publish, "main"), merge);
}

#[test]
fn finish_moves_the_branch_to_a_catch_up_merge_committed_before_an_interruption() {
    let f = Fork::new();
    let env = &f.env;
    let (verified, _) = verify_update(&f);
    conflicting(&f);
    f.tool(&["finish"]).code(1);
    let wt = env.update_worktree(&f.down);
    fs::write(wt.join("n.txt"), "both\n").unwrap();
    env.git(&wt, &["add", "n.txt"]);
    env.git(&wt, &["commit", "-q", "--no-edit"]);
    let merge = env.rev(&wt, "HEAD");
    assert_eq!(env.rev(&f.down, "git-intake/update"), verified);

    let out = stdout(&f.tool(&["finish"]).success());
    assert!(!out.contains("catching up with"), "{out}");
    assert_eq!(env.rev(&f.publish, "main"), merge);
}

#[test]
fn a_merge_interrupted_while_in_progress_is_concluded_by_continue() {
    let f = Fork::new();
    let env = &f.env;
    verify_update(&f);
    conflicting(&f);
    f.tool(&["finish"]).code(1);
    // Killed during the merge: the state says only that it began.
    edit_state(&f.down, |s| {
        let c = s["catch_ups"].as_array_mut().unwrap().last_mut().unwrap();
        c["step"] = "merging".into();
        c["conflicts"] = serde_json::json!([]);
    });
    let status = env.tool_json(&f.down, &["status", "--json"], 0);
    assert_eq!(status["update"]["step"], "catch_up_conflict");
    let wt = env.update_worktree(&f.down);
    fs::write(wt.join("n.txt"), "both\n").unwrap();
    env.git(&wt, &["add", "n.txt"]);
    f.tool(&["continue"])
        .success()
        .stdout(contains("caught up with 'main'"));
    let status = env.tool_json(&f.down, &["status", "--json"], 0);
    assert_eq!(status["update"]["step"], "catch_up");
}

#[test]
fn an_update_branch_moved_meanwhile_is_not_moved() {
    let f = Fork::new();
    let env = &f.env;
    let base = env.rev(&f.down, "main");
    let (verified, _) = verify_update(&f);
    conflicting(&f);
    f.tool(&["finish"]).code(1);
    let wt = env.update_worktree(&f.down);
    fs::write(wt.join("n.txt"), "both\n").unwrap();
    env.git(&wt, &["add", "n.txt"]);
    env.git(&wt, &["commit", "-q", "--no-edit"]);
    // The update branch, not checked out while the merge is built, moves.
    env.git(&f.down, &["branch", "-f", "git-intake/update", &base]);

    let err = stderr(&f.tool(&["continue"]).code(2));
    assert!(
        err.contains(&format!(
            "'git-intake/update' is at {}, no longer at {}",
            &base[..12],
            &verified[..12]
        )),
        "{err}"
    );
    assert!(err.contains("No branch moved"), "{err}");
    assert_eq!(env.rev(&f.down, "git-intake/update"), base);
}

#[test]
fn abort_in_the_catch_up_step_records_the_resolutions_made_by_hand() {
    let f = Fork::new();
    let env = &f.env;
    verify_update(&f);
    let theirs = conflicting(&f);
    f.tool(&["finish"]).code(1);
    let wt = env.update_worktree(&f.down);
    fs::write(wt.join("n.txt"), "both\n").unwrap();

    f.tool(&["abort"])
        .success()
        .stdout(contains("update aborted; 'main' is unchanged"));
    env.assert_no_update(&f.down);
    // The integration branch stays where the last fast-forward to the
    // publish remote left it.
    assert_eq!(env.rev(&f.down, "main"), theirs);

    // Started again on the new tip, the same conflict takes the resolution.
    let out = stdout(&f.tool(&["start"]).code(1));
    assert!(
        out.contains("n.txt  (resolved by a recorded resolution)"),
        "{out}"
    );
    assert_eq!(fs::read_to_string(wt.join("n.txt")).unwrap(), "both\n");
}

#[test]
fn a_state_of_format_4_is_carried_on_and_written_in_format_5() {
    let f = Fork::new();
    let env = &f.env;
    env.push_upstream(&f.up, &[("n.txt", "n\n")], "upstream: n");
    f.tool(&["start"]).success();
    edit_state(&f.down, |s| {
        s["version"] = 4.into();
        s["release"] = "0.3.5".into();
        let object = s.as_object_mut().unwrap();
        object.remove("start_base");
        object.remove("catch_ups");
    });
    f.tool(&["status"])
        .success()
        .stdout(contains("update in progress: merged"));
    assert_eq!(read_state(&f.down)["version"], 4);
    f.tool(&["migrate"]).code(2).stderr(contains(
        "keeps its state in format 4, which this release carries on without migration",
    ));
    let verify = f.tool(&["verify"]).success();
    let err = stderr(&verify);
    assert!(
        err.contains("the update was started by git-intake 0.3.5"),
        "{err}"
    );
    assert!(
        err.contains("releases before v0.3.6 can no longer carry the update on"),
        "{err}"
    );
    let state = read_state(&f.down);
    assert_eq!(state["version"], 5);
    assert_eq!(state["start_base"], state["base"]);
    f.tool(&["finish"]).success();
    env.assert_no_update(&f.down);
}
