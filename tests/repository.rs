//! Configuration, remotes, doctor and machine-readable output.

mod common;

use common::{Fork, Mono, mono_config, stderr, stdout};
use predicates::str::contains;

#[test]
fn status_shows_configuration_values() {
    let f = Fork::new();
    let out = stdout(&f.tool(&["status"]).success());
    assert!(out.contains("integration: main at"), "{out}");
    assert!(out.contains("publish: origin"), "{out}");
    assert!(out.contains("install: true"), "{out}");
    assert!(out.contains("checks:\n  test -f app.txt"), "{out}");
    let json = f.env.tool_json(&f.down, &["status", "--json"], 0);
    assert_eq!(json["integration"], "main");
    assert_eq!(json["publish"], "origin");
    assert_eq!(json["install"], "true");
    assert_eq!(json["checks"], serde_json::json!(["test -f app.txt"]));
    assert_eq!(json["config_working_copy_differs"], false);
    assert_eq!(json["update"], serde_json::Value::Null);
    let up = &json["upstreams"][0];
    for key in [
        "name",
        "path",
        "branch",
        "last_synced",
        "tip",
        "commits_behind",
        "state",
    ] {
        assert!(up.get(key).is_some(), "missing {key}: {up}");
    }
    assert_eq!(up["path"], serde_json::Value::Null);
    assert_eq!(up["mirror"], "upstream-main");
}

#[test]
fn uncommitted_configuration_edit_is_ignored_with_a_notice() {
    let f = Fork::new();
    let path = f.down.join(".git-intake.toml");
    let edited = std::fs::read_to_string(&path)
        .unwrap()
        .replace("test -f app.txt", "edited check");
    std::fs::write(&path, edited).unwrap();
    let status = f.tool(&["status"]).success();
    assert!(stderr(&status).contains("the working copy of .git-intake.toml differs from the version committed on 'main'; using the committed version"));
    assert!(stdout(&status).contains("test -f app.txt"));
    assert!(!stdout(&status).contains("edited check"));
    let json = f.env.tool_json(&f.down, &["status", "--json"], 0);
    assert_eq!(json["config_working_copy_differs"], true);
    assert_eq!(json["checks"], serde_json::json!(["test -f app.txt"]));
}

#[test]
fn configuration_is_read_from_the_integration_tip() {
    let f = Fork::new();
    let env = &f.env;
    // A feature branch with a different committed configuration does not steer the tool.
    env.git(&f.down, &["checkout", "-q", "-b", "feature"]);
    let cfg = std::fs::read_to_string(f.down.join(".git-intake.toml"))
        .unwrap()
        .replace("test -f app.txt", "feature check");
    env.commit(
        &f.down,
        &[(".git-intake.toml", &cfg)],
        "feature: edit config",
    );
    let json = env.tool_json(&f.down, &["status", "--json"], 0);
    assert_eq!(json["checks"], serde_json::json!(["test -f app.txt"]));
}

#[test]
fn invalid_configuration_is_refused() {
    let m = Mono::new();
    let env = &m.env;

    let unknown = mono_config(&m.a, &m.b, "").replace("checks =", "squash = true\nchecks =");
    env.commit(
        &m.down,
        &[(".git-intake.toml", &unknown)],
        "bad: unknown key",
    );
    m.tool(&["status"])
        .code(2)
        .stderr(contains("unknown field `squash`"));

    let nested = mono_config(&m.a, &m.b, "").replace("path = \"apps/b\"", "path = \"apps/a/b\"");
    env.commit(&m.down, &[(".git-intake.toml", &nested)], "bad: nested");
    m.tool(&["start"]).code(2).stderr(contains(
        "upstream 'app-b' path 'apps/a/b' lies inside upstream 'app-a' path 'apps/a'",
    ));

    let dup = mono_config(&m.a, &m.b, "").replace("remote = \"app-b\"", "remote = \"app-a\"");
    env.commit(&m.down, &[(".git-intake.toml", &dup)], "bad: shared remote");
    m.tool(&["fetch"])
        .code(2)
        .stderr(contains("2 upstreams share the remote 'app-a'"));

    let root = mono_config(&m.a, &m.b, "").replace("path = \"apps/b\"\n", "");
    env.commit(
        &m.down,
        &[(".git-intake.toml", &root)],
        "bad: root with others",
    );
    m.tool(&["log"])
        .code(2)
        .stderr(contains("upstream 'app-b' has no path (the repository root) and cannot share the repository with 'app-a'"));

    let doctor = env.tool_json(&m.down, &["doctor", "--json"], 1);
    assert_eq!(doctor["healthy"], false);
    assert_eq!(doctor["checks"][0]["check"], "configuration");
    assert_eq!(doctor["checks"][0]["status"], "fail");

    std::fs::remove_file(m.down.join(".git-intake.toml")).unwrap();
    env.git(&m.down, &["commit", "-q", "-am", "remove config"]);
    m.tool(&["status"])
        .code(2)
        .stderr(contains("no .git-intake.toml is committed at HEAD"));
}

#[test]
fn remote_with_a_different_url_is_refused() {
    let m = Mono::new();
    let env = &m.env;
    env.git(
        &m.down,
        &[
            "remote",
            "add",
            "app-a",
            "https://example.com/elsewhere.git",
        ],
    );
    // Only the mismatched upstream is refused; the others are fetched.
    let fetch = m.tool(&["fetch"]).code(1);
    let err = stdout(&fetch);
    assert!(err.contains("app-a: refused: "), "{err}");
    assert!(err.contains("app-b: fetched app-b/main"), "{err}");
    assert!(env.has_rev(&m.down, "refs/remotes/app-b/main"));
    assert!(
        err.contains(
            "upstream 'app-a': remote 'app-a' points to https://example.com/elsewhere.git"
        ),
        "{err}"
    );
    assert!(
        err.contains(&format!("the configuration says {}", m.a.url())),
        "{err}"
    );
    assert_eq!(
        env.git(&m.down, &["remote", "get-url", "app-a"]),
        "https://example.com/elsewhere.git",
        "the remote is not rewritten"
    );
    assert!(!env.has_rev(&m.down, "refs/remotes/app-a/main"));
    m.tool(&["start"])
        .code(2)
        .stderr(contains("points to https://example.com/elsewhere.git"));
    m.tool(&["doctor"]).code(1).stdout(contains(
        "[FAIL] app-a: upstream 'app-a': remote 'app-a' points to",
    ));
}

#[test]
fn missing_remote_without_url_is_refused() {
    let m = Mono::new();
    let env = &m.env;
    let cfg = mono_config(&m.a, &m.b, "").replace(
        &format!("url = {:?}\npath = \"apps/b\"", m.b.url()),
        "path = \"apps/b\"",
    );
    env.commit(&m.down, &[(".git-intake.toml", &cfg)], "mono: no url for b");
    m.tool(&["fetch"])
        .code(1)
        .stdout(contains("app-a: fetched app-a/main"))
        .stdout(contains(
            "app-b: refused: upstream 'app-b': remote 'app-b' does not exist and no url is configured",
        ));
    m.tool(&["start"]).code(2).stderr(contains(
        "remote 'app-b' does not exist and no url is configured",
    ));
    m.env.assert_no_update(&m.down);
}

#[test]
fn first_fetch_adds_remotes_and_one_unreachable_upstream_fails() {
    let m = Mono::new();
    let env = &m.env;
    let ok = stdout(&m.tool(&["fetch"]).success());
    assert!(ok.contains("app-a: added remote 'app-a'"), "{ok}");
    assert!(ok.contains("app-a: fetched app-a/main"), "{ok}");
    assert!(ok.contains("app-b: fetched app-b/main"), "{ok}");

    // app-b becomes unreachable.
    let gone = env.path("gone.git");
    env.git(
        &m.down,
        &["remote", "set-url", "app-b", &gone.to_string_lossy()],
    );
    let cfg = mono_config(&m.a, &m.b, "").replace(&m.b.url(), &gone.to_string_lossy());
    env.commit(&m.down, &[(".git-intake.toml", &cfg)], "mono: b moved");
    let a2 = env.push_upstream(&m.a, &[("x.txt", "x\n")], "app-a: x");
    let failed = m.tool(&["fetch"]).code(1);
    let out = stdout(&failed);
    assert!(out.contains("app-a: fetched app-a/main"), "{out}");
    assert!(
        out.contains("app-b: fetch failed: `git fetch --quiet --prune --no-tags app-b` failed"),
        "{out}"
    );
    assert_eq!(env.rev(&m.down, "refs/remotes/app-a/main"), a2);

    let check = env.tool_json(&m.down, &["status", "--check", "--json"], 1);
    assert_eq!(check["upstreams"][1]["state"], "unavailable");
    assert!(
        check["upstreams"][1]["detail"]
            .as_str()
            .unwrap()
            .starts_with("fetch failed")
    );
    m.tool(&["start"])
        .code(2)
        .stderr(contains("cannot fetch the selected upstreams"));
}

#[test]
fn status_marks_missing_remote_unavailable() {
    let m = Mono::new();
    let json = m.env.tool_json(&m.down, &["status", "--json"], 0);
    assert_eq!(json["upstreams"][0]["state"], "unavailable");
    assert_eq!(
        json["upstreams"][0]["detail"],
        "remote 'app-a' does not exist"
    );
    let log = m.env.tool_json(&m.down, &["log", "--json"], 0);
    assert_eq!(log["upstreams"][0]["available"], false);
}

#[test]
fn doctor_healthy_and_dropped_upstream() {
    let m = Mono::new();
    let env = &m.env;
    m.import_all();
    let healthy = m.tool(&["doctor"]).success();
    let out = stdout(&healthy);
    assert!(
        out.contains("[pass] the configuration on 'main' is valid"),
        "{out}"
    );
    assert!(
        out.contains("[pass] app-a: remote 'app-a' exists with the configured url"),
        "{out}"
    );
    assert!(
        out.contains("[pass] app-a: path 'apps/a' exists on 'main'"),
        "{out}"
    );
    assert!(out.contains("[pass] app-b: last synced commit"), "{out}");
    assert!(
        out.contains("[pass] the main checkout has no uncommitted changes"),
        "{out}"
    );
    assert!(
        out.contains("[pass] no merge is in progress in the main checkout"),
        "{out}"
    );
    assert!(out.contains("[pass] no update is in progress"), "{out}");
    assert!(out.contains("all checks passed"), "{out}");
    assert!(!out.contains("FAIL"), "{out}");

    // app-b is dropped from the configuration.
    let cfg = mono_config(&m.a, &m.b, "");
    let cut = cfg.find("\n[[upstream]]\nname = \"app-b\"").unwrap();
    env.commit(
        &m.down,
        &[(".git-intake.toml", &cfg[..cut])],
        "mono: drop app-b",
    );
    let json = env.tool_json(&m.down, &["doctor", "--json"], 0);
    let dropped: Vec<&serde_json::Value> = json["checks"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["check"] == "dropped_upstream")
        .collect();
    assert_eq!(dropped.len(), 1);
    assert_eq!(dropped[0]["upstream"], "app-b");
    assert_eq!(dropped[0]["status"], "warn");
    m.tool(&["doctor"]).success().stdout(contains(
        "upstream 'app-b' has sync records on 'main' but was dropped from the configuration",
    ));
}

#[test]
fn doctor_fails_on_dirty_checkout_and_inconsistent_state() {
    let m = Mono::new();
    let env = &m.env;
    m.import_all();
    std::fs::write(m.down.join("README.md"), "changed\n").unwrap();
    m.tool(&["doctor"]).code(1).stdout(contains(
        "[FAIL] the main checkout has uncommitted changes: README.md",
    ));
    env.git(&m.down, &["checkout", "--", "README.md"]);

    env.push_upstream(&m.a, &[("x.txt", "x\n")], "app-a: x");
    m.tool(&["start"]).success();
    m.tool(&["doctor"]).success().stdout(contains(
        "[pass] the update in progress matches its worktree and branch",
    ));
    // The worktree disappears behind the tool's back.
    std::fs::remove_dir_all(env.update_worktree(&m.down)).unwrap();
    m.tool(&["doctor"])
        .code(1)
        .stdout(contains("is missing; run `git intake abort`"));
    m.tool(&["verify"]).code(2).stderr(contains("is missing"));
    m.tool(&["abort"]).success();
    env.assert_no_update(&m.down);
    m.tool(&["doctor"]).success();
}

#[test]
fn report_json_and_text_while_in_progress() {
    let m = Mono::new();
    let env = &m.env;
    m.tool(&["report"])
        .code(2)
        .stderr(contains("nothing to report"));
    m.tool(&["start"]).success();
    let json = env.tool_json(&m.down, &["report", "--json"], 0);
    assert_eq!(json["in_progress"], true);
    let a = &json["upstreams"][0];
    assert_eq!(a["name"], "app-a");
    assert_eq!(a["kind"], "import");
    assert_eq!(a["range_start"], serde_json::Value::Null);
    assert_eq!(a["commits"], 1);
    assert_eq!(a["own_changes_before"], serde_json::Value::Null);
    assert_eq!(a["own_changes_after"]["files"], 0);
    for key in [
        "path",
        "source",
        "step",
        "target",
        "merge_commit",
        "conflicts",
    ] {
        assert!(a.get(key).is_some(), "missing {key}");
    }
    let text = stdout(&m.tool(&["report"]).success());
    assert!(
        text.contains("app-a (apps/a, app-a/main): merged"),
        "{text}"
    );
    assert!(text.contains("full history up to"), "{text}");
}

#[test]
fn every_reporting_command_emits_one_json_document() {
    let m = Mono::new();
    m.import_all();
    for args in [
        vec!["status", "--json"],
        vec!["report", "--json"],
        vec!["log", "--json"],
        vec!["branches", "--json"],
        vec!["doctor", "--json"],
    ] {
        let v = m.env.tool_json(&m.down, &args, 0);
        assert!(v.is_object(), "{args:?}");
    }
}

#[test]
fn running_outside_a_repository_is_an_error() {
    let env = common::Env::new();
    env.tool(&env.path("home"), &["status"])
        .code(2)
        .stderr(contains("not inside a git repository"));
}
