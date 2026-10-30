//! A fork with one upstream at the repository root.

mod common;

use std::fs;

use common::{FORK_FILE, Fork, stderr, stdout};
use predicates::str::contains;

#[test]
fn full_cycle_merges_verifies_and_publishes() {
    let f = Fork::new();
    let env = &f.env;
    let t1 = env.push_upstream(
        &f.up,
        &[(FORK_FILE, "one\ntwo\nthree\nfour\n"), ("new.txt", "x\n")],
        "upstream: add four",
    );
    let base = env.rev(&f.down, "main");

    let start = f.tool(&["start"]).success();
    assert!(stdout(&start).contains("upstream: queued at"));
    assert!(stdout(&start).contains("next: git intake verify"));
    let wt = env.update_worktree(&f.down);
    assert!(wt.join("new.txt").exists());
    assert_eq!(
        env.rev(&f.down, "main"),
        base,
        "start must not move the integration branch"
    );
    assert!(
        !f.down.join("new.txt").exists(),
        "nothing is written to the main checkout"
    );

    // The upstream moves during the update; the recorded target stays fixed.
    let t2 = env.push_upstream(&f.up, &[("later.txt", "y\n")], "upstream: later");

    let status = f.tool(&["status"]).success();
    let text = stdout(&status);
    assert!(text.contains("update in progress: merged"), "{text}");
    assert!(text.contains("next: git intake verify"), "{text}");
    assert!(text.contains(&format!("target {}", &t1[..12])), "{text}");

    f.tool(&["verify"]).success().stdout(contains("verified"));
    assert!(
        env.intake_dir(&f.down)
            .join("logs")
            .join("01-install.log")
            .exists()
    );
    assert!(
        env.intake_dir(&f.down)
            .join("logs")
            .join("02-check.log")
            .exists()
    );

    f.tool(&["finish"])
        .success()
        .stdout(contains("pushed main to 'origin'"));
    env.assert_no_update(&f.down);

    let main = env.rev(&f.down, "main");
    assert_eq!(env.rev(&f.down, "main^1"), base);
    assert_eq!(env.rev(&f.down, "main^2"), t1);
    let msg = env.git(&f.down, &["log", "-1", "--format=%B", "main"]);
    assert!(
        msg.starts_with(&format!(
            "sync(upstream): merge upstream/main at {}",
            env.git(&f.down, &["rev-parse", "--short", &t1])
        )),
        "{msg}"
    );
    assert!(msg.contains("Intake-Upstream: upstream"), "{msg}");
    assert!(msg.contains(&format!("Intake-Target: {t1}")), "{msg}");
    assert!(!env.is_ancestor(&f.down, &t2, "main"));
    // The main checkout followed the fast-forward.
    assert!(f.down.join("new.txt").exists());
    assert!(env.git(&f.down, &["status", "--porcelain"]).is_empty());
    // Published without force.
    assert_eq!(env.rev(&f.publish, "main"), main);
    // The upstream received nothing, and no remote or branch of it exists.
    assert_eq!(env.rev(&f.up.bare, "main"), t2);
    assert_eq!(env.git(&f.down, &["remote"]), "origin");
    assert_eq!(
        env.git(
            &f.down,
            &["for-each-ref", "--format=%(refname)", "refs/heads/"]
        ),
        "refs/heads/main"
    );

    // The report of the finished update is still available.
    let report = env.tool_json(&f.down, &["report", "--json"], 0);
    assert_eq!(report["in_progress"], false);
    assert_eq!(report["upstreams"][0]["target"], t1.as_str());
    assert_eq!(report["upstreams"][0]["commits"], 1);

    // The last synced commit comes from the history; status fetches the
    // upstream itself and keeps nothing of it.
    let status = env.tool_json(&f.down, &["status", "--json"], 0);
    let up = &status["upstreams"][0];
    assert_eq!(up["last_synced"], t1.as_str());
    assert_eq!(up["tip"], t2.as_str());
    assert_eq!(up["mode"], "follow");
    assert_eq!(up["url"], f.up.url().as_str());
    assert_eq!(up["state"], "behind");
    assert_eq!(up["commits_behind"], 1);
    assert!(up.get("remote").is_none() && up.get("mirror").is_none());
    env.assert_no_update(&f.down);
}

#[test]
fn nothing_to_update_creates_nothing() {
    let f = Fork::new();
    f.tool(&["start"])
        .success()
        .stdout(contains("upstream: current at"))
        .stdout(contains(
            "every selected upstream is current; nothing to update",
        ));
    f.env.assert_no_update(&f.down);
}

#[test]
fn start_refusals() {
    let f = Fork::new();
    let env = &f.env;
    env.push_upstream(&f.up, &[("n.txt", "n\n")], "upstream: n");

    f.tool(&["start", "nope"])
        .code(2)
        .stderr(contains("unknown upstream 'nope'"));

    std::fs::write(f.down.join(FORK_FILE), "dirty\n").unwrap();
    f.tool(&["start"])
        .code(2)
        .stderr(contains("the main checkout has uncommitted changes"));
    env.git(&f.down, &["checkout", "--", FORK_FILE]);

    // Someone published to the integration branch first, and it holds a
    // commit of its own: the two diverged.
    let base = env.rev(&f.down, "main");
    let other = env.path("other");
    env.git(
        &env.path(""),
        &[
            "clone",
            "-q",
            &f.publish.to_string_lossy(),
            &other.to_string_lossy(),
        ],
    );
    let theirs = env.commit(&other, &[("other.txt", "o\n")], "other: change");
    env.git(&other, &["push", "-q", "origin", "main"]);
    let local = env.commit(&f.down, &[("local.txt", "l\n")], "fork: local");
    f.tool(&["start"]).code(2).stderr(contains(
        "'main' and 'origin/main' have diverged: 'main' lacks 1 commit of 'origin/main', which lacks 1 of 'main'",
    ));
    assert_eq!(env.rev(&f.down, "main"), local);
    env.assert_no_update(&f.down);

    // Strictly behind: `start` fast-forwards it and starts from there.
    env.git(&f.down, &["reset", "-q", "--hard", &base]);
    f.tool(&["start"]).success().stdout(contains(format!(
        "fast-forwarded 'main' from {} to {} to match 'origin/main'",
        &base[..12],
        &theirs[..12]
    )));
    assert_eq!(env.rev(&f.down, "main"), theirs);
    assert!(f.down.join("other.txt").exists());
    let status = env.tool_json(&f.down, &["status", "--json"], 0);
    assert_eq!(status["update"]["base"], theirs.as_str());
    f.tool(&["start"])
        .code(2)
        .stderr(contains("an update is already in progress"));
}

#[test]
fn start_reads_the_configuration_again_after_a_fast_forward() {
    let f = Fork::new();
    let env = &f.env;
    env.push_upstream(&f.up, &[("n.txt", "n\n")], "upstream: n");
    let other = env.path("other");
    env.git(
        &env.path(""),
        &[
            "clone",
            "-q",
            &f.publish.to_string_lossy(),
            &other.to_string_lossy(),
        ],
    );
    // The new tip adds a check.
    let cfg = fs::read_to_string(other.join(".git-intake.toml"))
        .unwrap()
        .replace(
            "checks = [\"test -f app.txt\"]",
            "checks = [\"test -f n.txt\"]",
        );
    env.commit(&other, &[(".git-intake.toml", &cfg)], "other: check n");
    env.git(&other, &["push", "-q", "origin", "main"]);
    f.tool(&["start"]).success();
    f.tool(&["verify"])
        .success()
        .stdout(contains("passed   test -f n.txt"));
    f.tool(&["abort"]).success();

    // A new tip that names another publish remote: the fast-forward is kept,
    // and the update is not started.
    let mirror = env.bare("mirror");
    env.git(
        &f.down,
        &["remote", "add", "mirror", &mirror.to_string_lossy()],
    );
    let cfg = cfg.replace("remote = \"origin\"", "remote = \"mirror\"");
    let moved = env.commit(
        &other,
        &[(".git-intake.toml", &cfg)],
        "other: publish elsewhere",
    );
    env.git(&other, &["push", "-q", "origin", "main"]);
    let err = stderr(&f.tool(&["start"]).code(2));
    assert!(
        err.contains(
            "names 'mirror' as the publish remote instead of 'origin'; the fast-forward is kept"
        ),
        "{err}"
    );
    assert_eq!(env.rev(&f.down, "main"), moved);
    env.assert_no_update(&f.down);
}

#[test]
fn commit_after_verify_blocks_finish() {
    let f = Fork::new();
    let env = &f.env;
    env.push_upstream(&f.up, &[("n.txt", "n\n")], "upstream: n");
    f.tool(&["start"]).success();
    f.tool(&["verify"]).success();
    let wt = env.update_worktree(&f.down);
    env.commit(&wt, &[("fix.txt", "fix\n")], "fork: adapt to upstream");
    f.tool(&["finish"])
        .code(2)
        .stderr(contains("run `git intake verify` for the new commit"));
    let status = env.tool_json(&f.down, &["status", "--json"], 0);
    assert_eq!(status["update"]["step"], "merged");
    f.tool(&["verify"]).success();
    f.tool(&["finish"]).success();
    assert!(f.down.join("fix.txt").exists());
}

#[test]
fn failed_check_leaves_update_unverified() {
    let f = Fork::with_checks(&["test -f missing.txt", "true"]);
    let env = &f.env;
    env.push_upstream(&f.up, &[("n.txt", "n\n")], "upstream: n");
    f.tool(&["start"]).success();
    let v = f.tool(&["verify"]).code(1);
    assert!(
        stdout(&v).contains("FAILED   test -f missing.txt"),
        "{}",
        stdout(&v)
    );
    assert!(stdout(&v).contains("passed   true"));
    f.tool(&["finish"]).code(2).stderr(contains("failed"));
    let status = env.tool_json(&f.down, &["status", "--json"], 0);
    assert_eq!(status["update"]["step"], "verification_failed");
}

#[test]
fn failed_install_skips_checks() {
    let env_checks = ["true"];
    let f = Fork::with_checks(&env_checks);
    let env = &f.env;
    // Replace the install command with a failing one.
    let cfg = std::fs::read_to_string(f.down.join(".git-intake.toml"))
        .unwrap()
        .replace("install = \"true\"", "install = \"exit 3\"");
    env.commit(
        &f.down,
        &[(".git-intake.toml", &cfg)],
        "fork: failing install",
    );
    env.git(&f.down, &["push", "-q", "origin", "main"]);
    env.push_upstream(&f.up, &[("n.txt", "n\n")], "upstream: n");
    f.tool(&["start"]).success();
    let v = f.tool(&["verify"]).code(1);
    let out = stdout(&v);
    assert!(out.contains("FAILED   exit 3  (exit 3;"), "{out}");
    assert!(out.contains("skipped  true"), "{out}");
}

#[test]
fn rewritten_upstream_is_detected_and_refused() {
    let f = Fork::new();
    let env = &f.env;
    env.push_upstream(&f.up, &[("n.txt", "n\n")], "upstream: n");
    f.tool(&["start"]).success();
    f.tool(&["verify"]).success();
    f.tool(&["finish"]).success();

    // Force-push a rewritten upstream that drops the synced commit.
    env.git(&f.up.work, &["reset", "-q", "--hard", "HEAD~1"]);
    env.commit(&f.up.work, &[("r.txt", "r\n")], "upstream: rewritten");
    env.git(
        &f.up.work,
        &["push", "-q", "--force", "origin", "HEAD:main"],
    );

    let status = env.tool_json(&f.down, &["status", "--json"], 0);
    assert_eq!(status["upstreams"][0]["state"], "rewritten");
    f.tool(&["status"]).success().stdout(contains("rewritten"));
    f.tool(&["start"])
        .code(2)
        .stderr(contains("upstream 'upstream' was rewritten"));
    env.assert_no_update(&f.down);
    f.tool(&["status", "--check"]).code(1);
    f.tool(&["doctor"])
        .code(1)
        .stdout(contains("the upstream was rewritten"));
}

#[test]
fn status_check_reports_drift_and_conflicts_without_changes() {
    let f = Fork::new();
    let env = &f.env;
    env.commit(
        &f.down,
        &[(FORK_FILE, "one\nTWO-fork\nthree\n")],
        "fork: change two",
    );
    env.git(&f.down, &["push", "-q", "origin", "main"]);
    for i in 0..29 {
        env.commit(
            &f.up.work,
            &[(&format!("u{i}.txt") as &str, "u\n")],
            &format!("upstream: {i}"),
        );
    }
    env.push_upstream(
        &f.up,
        &[(FORK_FILE, "one\nTWO-upstream\nthree\n")],
        "upstream: change two",
    );
    let refs_before = env.git(&f.down, &["for-each-ref"]);

    let check = f.tool(&["status", "--check"]).code(1);
    let out = stdout(&check);
    assert!(
        out.contains("upstream: 30 commits behind, would conflict in:\n  app.txt"),
        "{out}"
    );
    env.assert_no_update(&f.down);
    assert!(
        !env.intake_dir(&f.down).exists(),
        "status --check wrote state"
    );
    let refs_after = env.git(&f.down, &["for-each-ref"]);
    let local_before: Vec<&str> = refs_before
        .lines()
        .filter(|l| !l.contains("refs/remotes/"))
        .collect();
    let local_after: Vec<&str> = refs_after
        .lines()
        .filter(|l| !l.contains("refs/remotes/"))
        .collect();
    assert_eq!(local_before, local_after);

    let json = env.tool_json(&f.down, &["status", "--check", "--json"], 1);
    assert_eq!(json["checked"], true);
    assert_eq!(json["all_current"], false);
    assert_eq!(json["upstreams"][0]["commits_behind"], 30);
    assert_eq!(json["upstreams"][0]["conflicts"][0], "app.txt");
}

#[test]
fn zero_drift_passes_the_check() {
    let f = Fork::new();
    let check = f.tool(&["status", "--check"]).success();
    assert!(stdout(&check).contains("every upstream is current"));
    let json = f
        .env
        .tool_json(&f.down, &["status", "--check", "--json"], 0);
    assert_eq!(json["all_current"], true);
    assert_eq!(json["upstreams"][0]["state"], "current");
}

#[test]
fn recorded_resolution_is_reapplied_after_abort() {
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

    let first = f.tool(&["start"]).code(1);
    let out = stdout(&first);
    assert!(
        out.contains("upstream: conflicts merging upstream/main"),
        "{out}"
    );
    assert!(out.contains("app.txt  (unresolved)"), "{out}");
    assert!(out.contains("run `git intake continue`"), "{out}");

    let wt = env.update_worktree(&f.down);
    f.tool(&["continue"])
        .code(2)
        .stderr(contains("still has unresolved conflicts"));
    let resolved = "one\nTWO-both\nthree\n";
    std::fs::write(wt.join(FORK_FILE), resolved).unwrap();
    env.git(&wt, &["add", FORK_FILE]);
    f.tool(&["continue"])
        .success()
        .stdout(contains("upstream: merged as"));
    let report = env.tool_json(&f.down, &["report", "--json"], 0);
    assert_eq!(
        report["upstreams"][0]["conflicts"][0]["resolution"],
        "manual"
    );

    f.tool(&["abort"]).success();
    env.assert_no_update(&f.down);

    let second = f.tool(&["start"]).code(1);
    let out = stdout(&second);
    assert!(
        out.contains("app.txt  (resolved by a recorded resolution)"),
        "{out}"
    );
    assert_eq!(
        std::fs::read_to_string(wt.join(FORK_FILE)).unwrap(),
        resolved
    );
    env.git(&wt, &["add", FORK_FILE]);
    f.tool(&["continue"]).success();
    let report = env.tool_json(&f.down, &["report", "--json"], 0);
    assert_eq!(report["upstreams"][0]["conflicts"][0]["path"], "app.txt");
    assert_eq!(
        report["upstreams"][0]["conflicts"][0]["resolution"],
        "recorded"
    );
    f.tool(&["report"])
        .success()
        .stdout(contains("app.txt  (resolved by a recorded resolution)"));
}

#[test]
fn stacked_downstream_branch_follows_another_fork() {
    // The "upstream" is another fork's integration branch that already
    // carries sync records of its own upstream under the same name.
    let f = Fork::new();
    let env = &f.env;
    env.push_upstream(&f.up, &[("n.txt", "n\n")], "upstream: n");
    f.tool(&["start"]).success();
    f.tool(&["verify"]).success();
    f.tool(&["finish"]).success();

    let stacked = env.path("stacked");
    env.git(
        &env.path(""),
        &[
            "clone",
            "-q",
            "-o",
            "upstream",
            &f.publish.to_string_lossy(),
            &stacked.to_string_lossy(),
        ],
    );
    env.git(&stacked, &["checkout", "-q", "-b", "develop"]);
    env.git(&stacked, &["remote", "remove", "upstream"]);
    let config = format!(
        "integration = \"develop\"\nchecks = [\"test -f app.txt\"]\n\n[[upstream]]\nname = \"upstream\"\nurl = {:?}\nbranch = \"main\"\n",
        f.publish.to_string_lossy()
    );
    env.commit(
        &stacked,
        &[(".git-intake.toml", &config)],
        "stacked: configuration",
    );

    // The stacked line starts from the base fork's line, which carries the
    // base fork's own sync record under the same upstream name. That record
    // is not this repository's: before its first merge, the last synced
    // commit comes from the history (the base fork commit the stacked line
    // contains), never from the base fork's record and its target.
    let base_record = env.git(
        &stacked,
        &[
            "log",
            "-1",
            "--format=%(trailers:key=Intake-Upstream,valueonly)",
            "develop^",
        ],
    );
    assert_eq!(
        base_record, "upstream",
        "the base fork's record is on the stacked line"
    );
    let cloned_tip = env.rev(&stacked, "develop^");
    let base_target = env.git(
        &stacked,
        &[
            "log",
            "-1",
            "--format=%(trailers:key=Intake-Target,valueonly)",
            "develop^",
        ],
    );
    let status = env.tool_json(&stacked, &["status", "--json"], 0);
    assert_eq!(status["upstreams"][0]["last_synced"], cloned_tip.as_str());
    assert_ne!(status["upstreams"][0]["last_synced"], base_target.as_str());
    assert_eq!(status["upstreams"][0]["last_synced_source"], "history");
    assert_eq!(status["upstreams"][0]["state"], "current");

    // The base fork moves on.
    env.commit(&f.down, &[("fork-feature.txt", "f\n")], "fork: feature");
    env.git(&f.down, &["push", "-q", "origin", "main"]);
    let fork_tip = env.rev(&f.down, "main");
    let status = env.tool_json(&stacked, &["status", "--json"], 0);
    let up = &status["upstreams"][0];
    assert_eq!(up["last_synced"], cloned_tip.as_str());
    assert_eq!(up["last_synced_source"], "history");
    assert_eq!(up["tip"], fork_tip.as_str());
    assert_eq!(up["state"], "behind");
    assert_eq!(up["commits_behind"], 1);
    let doctor = env.tool_json(&stacked, &["doctor", "--json"], 0);
    assert!(
        doctor["checks"]
            .as_array()
            .unwrap()
            .iter()
            .all(|c| c["check"] != "dropped_upstream"),
        "{doctor}"
    );

    env.tool(&stacked, &["start"]).success();
    env.tool(&stacked, &["verify"]).success();
    env.tool(&stacked, &["finish"])
        .success()
        .stdout(contains("nothing was pushed"));
    assert_eq!(env.rev(&stacked, "develop^2"), fork_tip);
    let status = env.tool_json(&stacked, &["status", "--json"], 0);
    assert_eq!(status["upstreams"][0]["last_synced"], fork_tip.as_str());
    assert_eq!(status["upstreams"][0]["state"], "current");

    // The base fork takes its own upstream in again; the stacked repository's
    // last synced commit stays its own record until it merges again.
    env.push_upstream(&f.up, &[("m.txt", "m\n")], "upstream: m");
    f.tool(&["start"]).success();
    f.tool(&["verify"]).success();
    f.tool(&["finish"]).success();
    let fork_tip2 = env.rev(&f.down, "main");
    let status = env.tool_json(&stacked, &["status", "--json"], 0);
    assert_eq!(status["upstreams"][0]["last_synced"], fork_tip.as_str());
    assert_eq!(status["upstreams"][0]["state"], "behind");
    env.tool(&stacked, &["start"]).success();
    env.tool(&stacked, &["verify"]).success();
    env.tool(&stacked, &["finish"]).success();
    let status = env.tool_json(&stacked, &["status", "--json"], 0);
    assert_eq!(status["upstreams"][0]["last_synced"], fork_tip2.as_str());
    assert_eq!(status["upstreams"][0]["state"], "current");
}

#[test]
fn merge_concluded_by_hand_is_accepted_by_continue() {
    let f = Fork::new();
    let env = &f.env;
    env.commit(
        &f.down,
        &[(FORK_FILE, "one\nTWO-fork\nthree\n")],
        "fork: change two",
    );
    env.git(&f.down, &["push", "-q", "origin", "main"]);
    let target = env.push_upstream(
        &f.up,
        &[(FORK_FILE, "one\nTWO-upstream\nthree\n")],
        "upstream: change two",
    );
    f.tool(&["start"]).code(1);
    let wt = env.update_worktree(&f.down);
    std::fs::write(wt.join(FORK_FILE), "one\nTWO-both\nthree\n").unwrap();
    env.git(&wt, &["add", FORK_FILE]);
    // Concluded with git directly; the prepared message keeps the sync record.
    env.git(&wt, &["commit", "-q", "--no-edit"]);
    f.tool(&["continue"])
        .success()
        .stdout(contains("upstream: merged as"));
    let trailer = env.git(
        &wt,
        &[
            "log",
            "-1",
            "--format=%(trailers:key=Intake-Target,valueonly)",
        ],
    );
    assert_eq!(trailer, target);
}

#[test]
fn checks_that_modify_tracked_files_fail_verification() {
    let f = Fork::with_checks(&["echo more >> app.txt"]);
    let env = &f.env;
    env.push_upstream(&f.up, &[("n.txt", "n\n")], "upstream: n");
    f.tool(&["start"]).success();
    f.tool(&["verify"])
        .code(1)
        .stdout(contains("the commands changed tracked files"))
        .stdout(contains("app.txt"));
    f.tool(&["finish"]).code(2);
}

#[test]
fn cloned_fork_without_records_shows_history_sourced_last_synced() {
    let f = Fork::new();
    let env = &f.env;
    // The fork was created by cloning: no sync record exists yet.
    let cloned = env.rev(&f.up.bare, "main");
    let status = env.tool_json(&f.down, &["status", "--json"], 0);
    let u = &status["upstreams"][0];
    assert_eq!(u["last_synced"], cloned.as_str());
    assert_eq!(u["last_synced_source"], "history");
    assert_eq!(u["state"], "current");
    f.tool(&["status"]).success().stdout(contains(
        "upstream: no sync record yet; last synced is the newest upstream commit",
    ));
    f.tool(&["doctor"]).success().stdout(contains(format!(
        "no sync record yet; last synced from history is {}",
        &cloned[..12]
    )));

    let target = env.push_upstream(&f.up, &[("n.txt", "n\n")], "upstream: n");
    let status = env.tool_json(&f.down, &["status", "--json"], 0);
    let u = &status["upstreams"][0];
    assert_eq!(u["last_synced"], cloned.as_str());
    assert_eq!(u["last_synced_source"], "history");
    assert_eq!(u["state"], "behind");
    assert_eq!(u["commits_behind"], 1);

    f.tool(&["start"]).success();
    let report = env.tool_json(&f.down, &["report", "--json"], 0);
    assert_eq!(report["upstreams"][0]["range_start"], cloned.as_str());
    f.tool(&["verify"]).success();
    f.tool(&["finish"]).success();
    let status = env.tool_json(&f.down, &["status", "--json"], 0);
    let u = &status["upstreams"][0];
    assert_eq!(u["last_synced"], target.as_str());
    assert_eq!(u["last_synced_source"], "record");
    assert_eq!(u["state"], "current");
}

#[test]
fn abort_removes_logs_and_the_update_references() {
    let f = Fork::new();
    let env = &f.env;
    env.push_upstream(&f.up, &[("n.txt", "n\n")], "upstream: n");
    f.tool(&["start"]).success();
    assert_eq!(
        env.intake_refs(&f.down),
        vec!["refs/git-intake/upstreams/upstream".to_string()],
        "the update keeps its upstream reference"
    );
    f.tool(&["verify"]).success();
    let logs = env.intake_dir(&f.down).join("logs");
    assert!(logs.join("01-install.log").exists());

    let abort = f.tool(&["abort"]).success();
    let out = stdout(&abort);
    assert!(out.contains("update aborted; 'main' is unchanged"), "{out}");
    assert!(out.contains("removed logs"), "{out}");
    assert!(
        out.contains("removed reference refs/git-intake/upstreams/upstream"),
        "{out}"
    );
    assert!(!logs.exists(), "the update's logs remain");
    env.assert_no_update(&f.down);
}

#[test]
fn a_stopped_follow_update_keeps_its_target_through_garbage_collection() {
    let f = Fork::new();
    let env = &f.env;
    env.commit(
        &f.down,
        &[(FORK_FILE, "one\nTWO-fork\nthree\n")],
        "fork: change two",
    );
    env.git(&f.down, &["push", "-q", "origin", "main"]);
    let target = env.push_upstream(
        &f.up,
        &[(FORK_FILE, "one\nTWO-upstream\nthree\n")],
        "upstream: change two",
    );
    f.tool(&["start"]).code(1);
    assert_eq!(
        env.rev(&f.down, "refs/git-intake/upstreams/upstream"),
        target
    );

    // The upstream disappears and the repository is garbage-collected: the target stays, and continue,
    // verify and finish never fetch.
    let wt = env.update_worktree(&f.down);
    let away = env.path("upstream-away.git");
    std::fs::rename(&f.up.bare, &away).unwrap();
    env.git(&f.down, &["gc", "-q", "--prune=now"]);
    assert!(env.has_rev(&f.down, &format!("{target}^{{commit}}")));
    std::fs::write(wt.join(FORK_FILE), "one\nTWO-both\nthree\n").unwrap();
    env.git(&wt, &["add", FORK_FILE]);
    f.tool(&["continue"])
        .success()
        .stdout(contains("upstream: merged as"));
    f.tool(&["verify"]).success();
    f.tool(&["finish"]).success();
    assert_eq!(env.rev(&f.down, "main^2"), target);
    env.assert_no_update(&f.down);
}

/// The message of a sync merge of the fork's upstream at `target`, as the
/// tool writes it.
fn sync_message(target: &str) -> String {
    format!(
        "sync(upstream): merge upstream/main at {}\n\nIntake-Upstream: upstream\nIntake-Target: {target}\n",
        &target[..12]
    )
}

/// Merges the fetched upstream into a new branch `name` at `main` with a
/// sync record made by hand, and returns to `main`.
fn record_on_branch(f: &Fork, name: &str, target: &str) {
    let env = &f.env;
    env.git(&f.down, &["fetch", "-q", &f.up.url(), target]);
    env.git(&f.down, &["checkout", "-q", "-b", name, "main"]);
    env.git(
        &f.down,
        &[
            "merge",
            "-q",
            "--no-ff",
            "-m",
            &sync_message(target),
            target,
        ],
    );
    env.git(&f.down, &["checkout", "-q", "main"]);
}

#[test]
fn a_record_merged_from_a_branch_counts_one_level_deep() {
    let f = Fork::new();
    let env = &f.env;
    let t1 = env.push_upstream(&f.up, &[("one.txt", "1\n")], "upstream: one");

    // A branch that carries a sync record, merged with a merge commit.
    record_on_branch(&f, "feature", &t1);
    env.git(
        &f.down,
        &[
            "merge",
            "-q",
            "--no-ff",
            "-m",
            "Merge branch 'feature'",
            "feature",
        ],
    );
    let status = env.tool_json(&f.down, &["status", "--json"], 0);
    let up = &status["upstreams"][0];
    assert_eq!(up["last_synced"], t1.as_str());
    assert_eq!(up["last_synced_source"], "record");

    // A record on a branch merged into another branch before that one was
    // merged is two merges away from the integration branch's first-parent
    // line, and does not count.
    let t2 = env.push_upstream(&f.up, &[("two.txt", "2\n")], "upstream: two");
    record_on_branch(&f, "inner", &t2);
    env.git(&f.down, &["checkout", "-q", "-b", "outer", "main"]);
    env.commit(&f.down, &[("outer.txt", "o\n")], "fork: outer work");
    env.git(
        &f.down,
        &[
            "merge",
            "-q",
            "--no-ff",
            "-m",
            "Merge branch 'inner'",
            "inner",
        ],
    );
    env.git(&f.down, &["checkout", "-q", "main"]);
    env.git(
        &f.down,
        &[
            "merge",
            "-q",
            "--no-ff",
            "-m",
            "Merge branch 'outer'",
            "outer",
        ],
    );
    let status = env.tool_json(&f.down, &["status", "--json"], 0);
    let up = &status["upstreams"][0];
    assert_eq!(up["last_synced"], t1.as_str());
    assert_eq!(up["last_synced_source"], "record");
}

#[test]
fn records_an_upstream_carries_never_count_even_when_it_cannot_be_fetched() {
    let f = Fork::new();
    let env = &f.env;
    // The upstream is itself a fork that records syncs of its own upstream.
    let own = env.push_upstream(&f.up, &[("own.txt", "o\n")], "base: own");
    env.push_upstream(
        &f.up,
        &[("synced.txt", "s\n")],
        &format!("sync(base): merge base/main\n\nIntake-Upstream: base\nIntake-Target: {own}\n"),
    );
    f.tool(&["start"]).success();
    f.tool(&["verify"]).success();
    f.tool(&["finish"]).success();

    // Without the upstream's branch to exclude, its history below the sync
    // merge still holds no record of this repository.
    fs::rename(&f.up.bare, env.path("moved.git")).unwrap();
    let doctor = env.tool_json(&f.down, &["doctor", "--json"], 1);
    assert!(
        doctor["checks"]
            .as_array()
            .unwrap()
            .iter()
            .all(|c| c["check"] != "dropped_upstream"),
        "{doctor}"
    );
}

#[test]
fn an_older_record_merged_later_never_moves_the_upstream_back() {
    let f = Fork::new();
    let env = &f.env;
    let t1 = env.push_upstream(&f.up, &[("one.txt", "1\n")], "upstream: one");
    // A branch with a record of t1 starts before the update to t2 lands.
    record_on_branch(&f, "stale", &t1);
    let t2 = env.push_upstream(&f.up, &[("two.txt", "2\n")], "upstream: two");
    f.tool(&["start"]).success();
    f.tool(&["verify"]).success();
    f.tool(&["finish"]).success();
    env.git(
        &f.down,
        &[
            "merge",
            "-q",
            "--no-ff",
            "-m",
            "Merge branch 'stale'",
            "stale",
        ],
    );

    let status = env.tool_json(&f.down, &["status", "--json"], 0);
    let up = &status["upstreams"][0];
    assert_eq!(up["state"], "current");
    assert_eq!(up["last_synced"], t2.as_str());
    assert_eq!(up["last_synced_source"], "record");
    let log = env.tool_json(&f.down, &["log", "--json"], 0);
    assert_eq!(log["upstreams"][0]["commits"], serde_json::json!([]));
    f.reconfigure("mode = \"follow\"", "mode = \"port\"");
    let status = env.tool_json(&f.down, &["status", "--json"], 0);
    assert_eq!(status["upstreams"][0]["base"], t2.as_str());
}
