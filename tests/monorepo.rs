//! A monorepo with several upstreams, each under its own path.

mod common;

use common::{Mono, merge_on_forge, merge_request_branch, mono_config, stdout};
use predicates::str::contains;

#[test]
fn first_import_keeps_full_history_under_each_path() {
    let m = Mono::new();
    let env = &m.env;
    let a_tip = env.rev(&m.a.bare, "main");
    let b_tip = env.rev(&m.b.bare, "main");
    let base = env.rev(&m.down, "main");

    // Upstreams are fetched by their URLs; no remote is added for them.
    let start = m.tool(&["start"]).success();
    let out = stdout(&start);
    assert!(out.contains("app-a: queued at"), "{out}");
    assert!(out.contains("app-b: queued at"), "{out}");
    assert_eq!(env.git(&m.down, &["remote"]), "");

    let wt = env.update_worktree(&m.down);
    assert_eq!(
        env.rev(&wt, "HEAD^1^1"),
        base,
        "one merge commit per upstream"
    );
    assert_eq!(env.rev(&wt, "HEAD^1^2"), a_tip);
    assert_eq!(env.rev(&wt, "HEAD^2"), b_tip);

    m.tool(&["verify"]).success();
    m.tool(&["finish"]).success().stdout(contains(
        "no publish remote is configured; nothing was pushed",
    ));
    env.assert_no_update(&m.down);

    assert_eq!(
        std::fs::read_to_string(m.down.join("apps/a/a.txt")).unwrap(),
        "a1\na2\na3\n"
    );
    assert_eq!(
        std::fs::read_to_string(m.down.join("apps/b/b.txt")).unwrap(),
        "b1\nb2\nb3\n"
    );
    assert!(!m.down.join("a.txt").exists());
    assert!(
        env.is_ancestor(&m.down, &a_tip, "main"),
        "upstream history is kept"
    );
    assert!(env.is_ancestor(&m.down, &b_tip, "main"));
    let subject = env.git(&m.down, &["log", "-1", "--format=%s", "main^1"]);
    assert!(
        subject.starts_with("sync(app-a): merge app-a/main at "),
        "{subject}"
    );

    let status = env.tool_json(&m.down, &["status", "--json"], 0);
    for (i, (name, path, tip)) in [("app-a", "apps/a", &a_tip), ("app-b", "apps/b", &b_tip)]
        .iter()
        .enumerate()
    {
        let u = &status["upstreams"][i];
        assert_eq!(u["name"], *name);
        assert_eq!(u["path"], *path);
        assert_eq!(u["branch"], "main");
        assert_eq!(u["last_synced"], tip.as_str());
        assert_eq!(u["tip"], tip.as_str());
        assert_eq!(u["commits_behind"], 0);
        assert_eq!(u["state"], "current");
    }
}

#[test]
fn later_update_merges_under_path_and_skips_unchanged() {
    let m = Mono::new();
    let env = &m.env;
    m.import_all();
    // The monorepo adapts the imported code; the upstream changes elsewhere in the same file.
    env.commit(
        &m.down,
        &[("apps/a/a.txt", "a1-mono\na2\na3\n")],
        "mono: adapt a",
    );
    let a2 = env.push_upstream(
        &m.a,
        &[("a.txt", "a1\na2\na3-up\n"), ("extra.txt", "e\n")],
        "app-a: change",
    );

    let start = m.tool(&["start"]).success();
    let out = stdout(&start);
    assert!(out.contains("app-b: current at"), "{out}");
    assert!(out.contains("app-a: queued at"), "{out}");
    assert!(!out.contains("app-b: queued"), "{out}");
    m.tool(&["verify"]).success();
    m.tool(&["finish"]).success();

    assert_eq!(
        std::fs::read_to_string(m.down.join("apps/a/a.txt")).unwrap(),
        "a1-mono\na2\na3-up\n"
    );
    assert!(m.down.join("apps/a/extra.txt").exists());
    assert!(!m.down.join("extra.txt").exists());
    assert_eq!(env.rev(&m.down, "main^2"), a2);
    let report = env.tool_json(&m.down, &["report", "--json"], 0);
    let u = &report["upstreams"][0];
    assert_eq!(u["name"], "app-a");
    assert_eq!(u["kind"], "subtree");
    assert_eq!(u["commits"], 1);
    assert_eq!(u["own_changes_before"]["files"], 1);
    assert_eq!(u["own_changes_after"]["files"], 1);
}

#[test]
fn conflict_then_continue_merges_the_next_upstream() {
    let m = Mono::new();
    let env = &m.env;
    m.import_all();
    env.commit(
        &m.down,
        &[("apps/a/a.txt", "a1\na2-mono\na3\n")],
        "mono: change a2",
    );
    env.push_upstream(&m.a, &[("a.txt", "a1\na2-up\na3\n")], "app-a: change a2");
    let b2 = env.push_upstream(&m.b, &[("b2.txt", "b\n")], "app-b: add b2");

    let start = m.tool(&["start"]).code(1);
    let out = stdout(&start);
    assert!(out.contains("app-a: conflicts merging app-a/main"), "{out}");
    assert!(out.contains("apps/a/a.txt  (unresolved)"), "{out}");
    assert!(out.contains("merged: none"), "{out}");
    assert!(out.contains("pending: app-b"), "{out}");

    let status = env.tool_json(&m.down, &["status", "--json"], 0);
    assert_eq!(status["update"]["step"], "conflict");
    assert_eq!(status["update"]["upstreams"][0]["step"], "conflict");
    assert_eq!(status["update"]["upstreams"][1]["step"], "pending");
    m.tool(&["status"])
        .success()
        .stdout(contains("app-a: target"))
        .stdout(contains("in conflict"));

    m.tool(&["verify"])
        .code(2)
        .stderr(contains("app-a (in conflict)"))
        .stderr(contains("app-b (pending)"));

    let wt = env.update_worktree(&m.down);
    std::fs::write(wt.join("apps/a/a.txt"), "a1\na2-both\na3\n").unwrap();
    env.git(&wt, &["add", "apps/a/a.txt"]);
    let report = env.tool_json(&m.down, &["report", "--json"], 0);
    assert_eq!(
        report["upstreams"][0]["conflicts"][0]["resolution"],
        "manual"
    );

    m.tool(&["continue"])
        .success()
        .stdout(contains("app-a: merged as"))
        .stdout(contains("app-b: merged as"))
        .stdout(contains("next: git intake verify"));
    m.tool(&["verify"]).success();
    m.tool(&["finish"]).success();
    assert_eq!(
        std::fs::read_to_string(m.down.join("apps/a/a.txt")).unwrap(),
        "a1\na2-both\na3\n"
    );
    assert_eq!(env.rev(&m.down, "main^2"), b2);
    assert!(m.down.join("apps/b/b2.txt").exists());
    let msg = env.git(&m.down, &["log", "-1", "--format=%B", "main^1"]);
    assert!(msg.contains("Intake-Upstream: app-a"), "{msg}");
}

#[test]
fn abort_restores_everything() {
    let m = Mono::new();
    let env = &m.env;
    m.import_all();
    env.commit(
        &m.down,
        &[("apps/a/a.txt", "a1\na2-mono\na3\n")],
        "mono: change a2",
    );
    env.push_upstream(&m.a, &[("a.txt", "a1\na2-up\na3\n")], "app-a: change a2");
    let main = env.rev(&m.down, "main");

    m.tool(&["start"]).code(1);
    m.tool(&["abort"])
        .success()
        .stdout(contains("removed worktree"))
        .stdout(contains("removed branch git-intake/update"))
        .stdout(contains("removed logs"))
        .stdout(contains(
            "removed reference refs/git-intake/upstreams/app-a",
        ))
        .stdout(contains("update aborted; 'main' is unchanged"));
    env.assert_no_update(&m.down);
    assert!(!env.intake_dir(&m.down).join("logs").exists());
    assert_eq!(env.rev(&m.down, "main"), main);
    assert!(env.git(&m.down, &["status", "--porcelain"]).is_empty());
    m.tool(&["abort"])
        .success()
        .stdout(contains("nothing to abort"));
    m.tool(&["continue"])
        .code(2)
        .stderr(contains("no update is in progress"));
}

#[test]
fn path_with_unrelated_content_is_refused() {
    let m = Mono::new();
    let env = &m.env;
    env.commit(
        &m.down,
        &[("apps/b/local.txt", "mine\n")],
        "mono: unrelated content at apps/b",
    );
    let start = m.tool(&["start"]).code(2);
    let err = common::stderr(&start);
    assert!(
        err.contains(
            "upstream 'app-b': path 'apps/b' holds content unrelated to the history of app-b/main"
        ),
        "{err}"
    );
    assert!(
        !err.contains("move"),
        "the refusal must not advise moving content: {err}"
    );
    env.assert_no_update(&m.down);

    let check = env.tool_json(&m.down, &["status", "--check", "--json"], 1);
    assert_eq!(check["upstreams"][1]["state"], "not_imported");
    assert!(
        check["upstreams"][1]["detail"]
            .as_str()
            .unwrap()
            .contains("holds content unrelated to the history")
    );
    m.tool(&["doctor"]).code(1).stdout(contains(
        "[FAIL] app-b: upstream 'app-b': path 'apps/b' holds content unrelated",
    ));
    m.tool(&["start", "app-a"]).success();
}

#[test]
fn status_reports_three_upstreams_with_counts() {
    let m = Mono::new();
    let env = &m.env;
    let c = env.upstream("app-c", &[("c.txt", "c\n")]);
    let extra = format!(
        "\n[[upstream]]\nname = \"app-c\"\nurl = {:?}\nbranch = \"main\"\npath = \"libs/c\"\n",
        c.url()
    );
    env.commit(
        &m.down,
        &[(".git-intake.toml", &mono_config(&m.a, &m.b, &extra))],
        "mono: add app-c",
    );
    m.import_all();
    env.push_upstream(&m.a, &[("a2.txt", "x\n")], "app-a: one");
    env.push_upstream(&m.b, &[("b2.txt", "x\n")], "app-b: one");
    env.push_upstream(&m.b, &[("b3.txt", "x\n")], "app-b: two");

    let status = m.tool(&["status"]).success();
    let out = stdout(&status);
    let row = |name: &str| {
        out.lines()
            .find(|l| l.starts_with(name))
            .unwrap()
            .to_string()
    };
    assert!(row("app-a").contains("apps/a"), "{out}");
    assert!(row("app-a").ends_with("1       behind"), "{out}");
    assert!(row("app-b").ends_with("2       behind"), "{out}");
    assert!(row("app-c").contains("libs/c"), "{out}");
    assert!(row("app-c").ends_with("0       current"), "{out}");

    let check = m.tool(&["status", "--check"]).code(1);
    let out = stdout(&check);
    assert!(
        out.contains("app-a: 1 commit behind, would merge without conflicts"),
        "{out}"
    );
    assert!(
        out.contains("app-b: 2 commits behind, would merge without conflicts"),
        "{out}"
    );
}

#[test]
fn status_check_predicts_conflicts_under_a_path() {
    let m = Mono::new();
    let env = &m.env;
    m.import_all();
    env.commit(
        &m.down,
        &[("apps/a/a.txt", "a1\na2-mono\na3\n")],
        "mono: change a2",
    );
    env.push_upstream(&m.a, &[("a.txt", "a1\na2-up\na3\n")], "app-a: change a2");
    let json = env.tool_json(&m.down, &["status", "--check", "--json"], 1);
    assert_eq!(json["upstreams"][0]["state"], "behind");
    assert_eq!(
        json["upstreams"][0]["conflicts"],
        serde_json::json!(["apps/a/a.txt"])
    );
    assert_eq!(json["upstreams"][1]["state"], "current");
    assert_eq!(json["upstreams"][1]["conflicts"], serde_json::json!([]));
    env.assert_no_update(&m.down);
}

#[test]
fn branches_lists_unmerged_upstream_work() {
    let m = Mono::new();
    let env = &m.env;
    m.import_all();
    m.tool(&["branches"])
        .success()
        .stdout(contains("no upstream holds unmerged work"));

    env.git(&m.a.work, &["checkout", "-q", "-b", "feature"]);
    env.commit(&m.a.work, &[("f1.txt", "1\n")], "app-a: feature 1");
    env.commit(&m.a.work, &[("f2.txt", "2\n")], "app-a: feature 2");
    env.git(&m.a.work, &["push", "-q", "origin", "feature"]);

    m.tool(&["branches"])
        .success()
        .stdout(contains("app-a (apps/a):"))
        .stdout(contains("app-a/feature  2 unmerged commits"));
    let json = env.tool_json(&m.down, &["branches", "--json"], 0);
    assert_eq!(json["unmerged_work"], true);
    let a = &json["upstreams"][0];
    assert_eq!(a["name"], "app-a");
    assert_eq!(a["branches"][0]["branch"], "feature");
    assert_eq!(a["branches"][0]["unmerged_commits"], 2);
    assert_eq!(a["branches"][0]["tracked"], false);
    assert_eq!(json["upstreams"][1]["branches"], serde_json::json!([]));
    let only_b = env.tool_json(&m.down, &["branches", "app-b", "--json"], 0);
    assert_eq!(only_b["unmerged_work"], false);
    // Every branch was fetched for the command only.
    env.assert_no_update(&m.down);
    assert!(!env.has_rev(&m.down, "refs/heads/feature"));
}

#[test]
fn log_lists_pending_commits() {
    let m = Mono::new();
    let env = &m.env;
    m.import_all();
    let mut commits = Vec::new();
    for i in 1..=3 {
        commits.push(env.push_upstream(
            &m.a,
            &[(&format!("n{i}.txt") as &str, "n\n")],
            &format!("app-a: change {i}"),
        ));
    }
    let log = m.tool(&["log", "app-a"]).success();
    let out = stdout(&log);
    assert!(
        out.contains("app-a (apps/a, app-a/main): 3 commits not in main"),
        "{out}"
    );
    for (i, c) in commits.iter().enumerate() {
        assert!(
            out.contains(&format!("{}  app-a: change {}", &c[..12], i + 1)),
            "{out}"
        );
    }
    let json = env.tool_json(&m.down, &["log", "--json"], 0);
    let a = &json["upstreams"][0];
    assert_eq!(a["commits"].as_array().unwrap().len(), 3);
    assert_eq!(a["commits"][0]["commit"], commits[2].as_str());
    assert_eq!(a["commits"][0]["subject"], "app-a: change 3");
    assert_eq!(json["upstreams"][1]["commits"], serde_json::json!([]));
}

#[test]
fn paths_imported_through_a_merge_request_merged_with_a_merge_commit_stay_imported() {
    let m = Mono::merge_request();
    let env = &m.env;
    let publish = m.publish.clone().unwrap();
    let a_tip = env.rev(&m.a.bare, "main");
    let b_tip = env.rev(&m.b.bare, "main");
    m.tool(&["start"]).success();
    m.tool(&["verify"]).success();
    let commit = env.rev(&env.update_worktree(&m.down), "HEAD");
    m.tool(&["finish"]).success();
    merge_on_forge(env, &publish, &merge_request_branch(&commit));
    m.tool(&["finish"]).success();
    assert_eq!(env.rev(&m.down, "main^2"), commit);

    let status = env.tool_json(&m.down, &["status", "--json"], 0);
    for (i, tip) in [&a_tip, &b_tip].iter().enumerate() {
        let u = &status["upstreams"][i];
        assert_eq!(u["state"], "current");
        assert_eq!(u["last_synced"], tip.as_str());
        assert_eq!(u["last_synced_source"], "record");
    }
    let doctor = env.tool_json(&m.down, &["doctor", "--json"], 0);
    assert_eq!(doctor["healthy"], true);

    // The next update merges into the imported path.
    env.push_upstream(&m.a, &[("a2.txt", "x\n")], "app-a: one");
    m.tool(&["start"])
        .success()
        .stdout(contains("app-a: queued at"));
    let wt = env.update_worktree(&m.down);
    assert!(wt.join("apps/a/a2.txt").exists());
    let subject = env.git(&wt, &["log", "-1", "--format=%s"]);
    assert!(
        subject.starts_with("sync(app-a): merge app-a/main at "),
        "{subject}"
    );
}
