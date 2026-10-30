//! Configuration, upstreams fetched by URL, doctor and machine-readable
//! output.

mod common;

use common::{Fork, MERGE_REQUEST_BRANCH, Mono, mono_config, stderr, stdout};
use predicates::str::contains;

#[test]
fn status_shows_configuration_values() {
    let f = Fork::new();
    let out = stdout(&f.tool(&["status"]).success());
    assert!(out.contains("integration: main at"), "{out}");
    assert!(out.contains("publish: origin (via push)"), "{out}");
    assert!(out.contains("install: true"), "{out}");
    assert!(out.contains("checks:\n  app: test -f app.txt"), "{out}");
    let json = f.env.tool_json(&f.down, &["status", "--json"], 0);
    assert_eq!(json["integration"], "main");
    assert_eq!(json["publish"], "origin");
    assert_eq!(json["publish_via"], "push");
    assert_eq!(json["install"], "true");
    assert_eq!(
        json["checks"],
        serde_json::json!([{"name": "app", "run": "test -f app.txt"}])
    );
    assert_eq!(json["config_working_copy_differs"], false);
    assert_eq!(json["update"], serde_json::Value::Null);
    let up = &json["upstreams"][0];
    for key in [
        "name",
        "path",
        "url",
        "branch",
        "mode",
        "last_synced",
        "tip",
        "commits_behind",
        "state",
        "base",
        "taken",
        "decided",
        "pending",
    ] {
        assert!(up.get(key).is_some(), "missing {key}: {up}");
    }
    assert_eq!(up["path"], serde_json::Value::Null);
    assert_eq!(up["url"], f.up.url().as_str());
    assert_eq!(up["mode"], "follow");
    assert_eq!(up["pending"], serde_json::Value::Null);
    assert!(up.get("remote").is_none() && up.get("mirror").is_none());
    assert!(
        out.contains("upstream  /     follow  main"),
        "the table shows the mode and the branch: {out}"
    );
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
    assert_eq!(
        json["checks"],
        serde_json::json!([{"name": "app", "run": "test -f app.txt"}])
    );
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
    assert_eq!(
        json["checks"],
        serde_json::json!([{"name": "app", "run": "test -f app.txt"}])
    );
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

    let dup = mono_config(&m.a, &m.b, "").replace(&m.b.url(), &m.a.url());
    env.commit(&m.down, &[(".git-intake.toml", &dup)], "bad: shared source");
    m.tool(&["branches"]).code(2).stderr(contains(format!(
        "2 upstreams share the URL '{}' and the branch 'main'",
        m.a.url()
    )));

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
fn publish_setting_is_a_table_with_a_mode() {
    let f = Fork::new();
    let env = &f.env;
    let committed = std::fs::read_to_string(f.down.join(".git-intake.toml")).unwrap();

    let string_form = committed.replace("[publish]\nremote = \"origin\"\nvia = \"push\"\n", "");
    let string_form = format!("publish = \"origin\"\n{string_form}");
    env.commit(
        &f.down,
        &[(".git-intake.toml", &string_form)],
        "bad: publish as a string",
    );
    let err = stderr(&f.tool(&["status"]).code(2));
    assert!(
        err.contains("publish: expected a table, not a string"),
        "{err}"
    );
    assert!(err.contains("publish = { remote = \"origin\" }"), "{err}");
    assert!(err.contains("placed after the top-level keys"), "{err}");

    let unknown = committed.replace("via = \"push\"", "via = \"pull-request\"");
    env.commit(
        &f.down,
        &[(".git-intake.toml", &unknown)],
        "bad: unknown publication mode",
    );
    f.tool(&["start"]).code(2).stderr(contains(
        "unknown variant `pull-request`, expected `push` or `merge-request`",
    ));

    let default = committed.replace("via = \"push\"\n", "");
    env.commit(
        &f.down,
        &[(".git-intake.toml", &default)],
        "publish by default mode",
    );
    let json = env.tool_json(&f.down, &["status", "--json"], 0);
    assert_eq!(json["publish"], "origin");
    assert_eq!(json["publish_via"], "push");
}

#[test]
fn removed_and_invalid_upstream_settings_are_refused() {
    let m = Mono::new();
    let env = &m.env;

    // A configuration of the previous release: a remote and a mirror.
    let old = mono_config(&m.a, &m.b, "").replace(
        &format!("url = {:?}\n", m.a.url()),
        "remote = \"app-a\"\nmirror = \"mirror-a\"\n",
    );
    env.commit(
        &m.down,
        &[(".git-intake.toml", &old)],
        "old: remote and mirror",
    );
    let err = stderr(&m.tool(&["status"]).code(2));
    assert!(
        err.contains(
            "upstream 'app-a' remote: upstreams are fetched by their URL and the tool keeps no remote for them; \
             replace `remote` with `url` set to that remote's URL"
        ),
        "{err}"
    );
    assert!(
        err.contains(
            "upstream 'app-a' mirror: mirror branches are no longer kept; remove `mirror`"
        ),
        "{err}"
    );

    let no_url = mono_config(&m.a, &m.b, "").replace(&format!("url = {:?}\n", m.b.url()), "");
    env.commit(&m.down, &[(".git-intake.toml", &no_url)], "bad: no url");
    m.tool(&["start"])
        .code(2)
        .stderr(contains("upstream 'app-b': `url` is required"));

    let port_path = mono_config(&m.a, &m.b, "")
        .replace("path = \"apps/b\"", "path = \"apps/b\"\nmode = \"port\"");
    env.commit(
        &m.down,
        &[(".git-intake.toml", &port_path)],
        "bad: port under a path",
    );
    m.tool(&["log"]).code(2).stderr(contains(
        "upstream 'app-b': an upstream in `port` mode is taken at the repository root and cannot have a path",
    ));

    let bad_name = mono_config(&m.a, &m.b, "").replace("name = \"app-b\"", "name = \"app.lock\"");
    env.commit(
        &m.down,
        &[(".git-intake.toml", &bad_name)],
        "bad: a name git cannot use",
    );
    m.tool(&["status"]).code(2).stderr(contains(
        "upstream 'app.lock': 'app.lock' cannot name a git reference",
    ));
    env.assert_no_update(&m.down);
}

#[test]
fn publish_remote_at_an_upstream_url_is_refused() {
    let f = Fork::new();
    let env = &f.env;
    env.git(&f.down, &["remote", "set-url", "origin", &f.up.url()]);
    let err = stderr(&f.tool(&["status"]).code(2));
    assert!(
        err.contains(&format!(
            "publish remote 'origin' has the URL {}, which names the repository of upstream 'upstream' ({}); \
             the tool never pushes to an upstream",
            f.up.url(),
            f.up.url()
        )),
        "{err}"
    );
    // A relative path to the same repository, from the main worktree.
    env.git(
        &f.down,
        &["remote", "set-url", "origin", "../upstream.git/"],
    );
    f.tool(&["status"]).code(2).stderr(contains(
        "has the URL ../upstream.git/, which names the repository of upstream 'upstream'",
    ));
    // Pushing to the upstream's repository counts as well.
    env.git(
        &f.down,
        &["remote", "set-url", "origin", &f.publish.to_string_lossy()],
    );
    f.tool(&["status"]).success();
    env.git(
        &f.down,
        &[
            "remote",
            "set-url",
            "--add",
            "--push",
            "origin",
            &f.up.url(),
        ],
    );
    f.tool(&["start"])
        .code(2)
        .stderr(contains("publish remote 'origin' has the URL"));
    env.assert_no_update(&f.down);
}

#[test]
fn local_paths_and_ssh_urls_are_matched_as_git_reads_them() {
    let f = Fork::new();
    let env = &f.env;
    // Git finds `../upstream` at `../upstream.git`.
    env.git(&f.down, &["remote", "set-url", "origin", "../upstream"]);
    f.tool(&["status"]).code(2).stderr(contains(
        "has the URL ../upstream, which names the repository of upstream 'upstream'",
    ));

    // The scp-like form and the `ssh://` form of one hosted repository.
    env.git(
        &f.down,
        &["remote", "set-url", "origin", &f.publish.to_string_lossy()],
    );
    f.reconfigure(
        &format!("url = {:?}", f.up.url()),
        "url = \"git@example.com:team/upstream.git\"",
    );
    env.git(
        &f.down,
        &[
            "remote",
            "set-url",
            "origin",
            "ssh://git@Example.com:22/team/upstream",
        ],
    );
    f.tool(&["status"]).code(2).stderr(contains(
        "has the URL ssh://git@Example.com:22/team/upstream, which names the repository of upstream 'upstream'",
    ));
}

#[test]
fn hosted_urls_naming_the_same_repository_count_as_one() {
    let f = Fork::new();
    let env = &f.env;
    f.reconfigure(
        &format!("url = {:?}", f.up.url()),
        "url = \"https://Example.com/team/upstream.git\"",
    );
    env.git(
        &f.down,
        &[
            "remote",
            "set-url",
            "origin",
            "https://example.com/team/upstream/",
        ],
    );
    f.tool(&["status"]).code(2).stderr(contains(
        "which names the repository of upstream 'upstream'",
    ));
    env.git(
        &f.down,
        &[
            "remote",
            "set-url",
            "origin",
            "https://example.com/team/other.git",
        ],
    );
    env.git(
        &f.down,
        &[
            "remote",
            "add",
            "mirror",
            "https://EXAMPLE.com/team/upstream",
        ],
    );
    // The hosted URL is served from the local upstream, in this test's own
    // git configuration, so that doctor's fetch stays local.
    std::fs::write(
        env.path("gitconfig"),
        format!(
            "[url {:?}]\n\tinsteadOf = https://Example.com/team/upstream.git\n",
            f.up.url()
        ),
    )
    .unwrap();
    let doctor = stdout(&f.tool(&["doctor"]).code(1));
    assert!(
        doctor.contains("[pass] upstream: branch 'main' of"),
        "{doctor}"
    );
    assert!(
        doctor.contains(
            "[FAIL] upstream: git remote 'mirror' names the repository of upstream 'upstream' \
             (https://Example.com/team/upstream.git)"
        ),
        "{doctor}"
    );
}

#[test]
fn remote_at_an_upstream_url_is_neither_used_nor_changed_and_doctor_names_it() {
    let m = Mono::new();
    let env = &m.env;
    env.git(&m.down, &["remote", "add", "app-a", &m.a.url()]);
    env.git(
        &m.down,
        &[
            "config",
            "remote.app-a.fetch",
            "+refs/heads/*:refs/remotes/app-a/*",
        ],
    );
    let json = env.tool_json(&m.down, &["status", "--json"], 0);
    assert_eq!(
        json["upstreams"][0]["tip"],
        env.rev(&m.a.bare, "main").as_str()
    );
    assert!(
        !env.has_rev(&m.down, "refs/remotes/app-a/main"),
        "the remote was not fetched"
    );
    assert_eq!(env.git(&m.down, &["remote", "get-url", "app-a"]), m.a.url());
    env.assert_no_update(&m.down);

    let doctor = stdout(&m.tool(&["doctor"]).code(1));
    assert!(
        doctor.contains(&format!(
            "[FAIL] app-a: git remote 'app-a' names the repository of upstream 'app-a' ({})",
            m.a.url()
        )),
        "{doctor}"
    );
    assert!(
        doctor.contains("remove it with `git remote remove app-a`"),
        "{doctor}"
    );
    assert!(!doctor.contains("partial clone"), "{doctor}");

    // The remote a partial clone fetches missing objects from.
    env.git(&m.down, &["config", "remote.app-a.promisor", "true"]);
    let doctor = stdout(&m.tool(&["doctor"]).code(1));
    assert!(
        doctor.contains(
            "This partial clone fetches missing objects from 'app-a': fetch the objects the history still lacks \
             before removing the remote"
        ),
        "{doctor}"
    );
    env.git(&m.down, &["remote", "remove", "app-a"]);
    m.tool(&["doctor"]).success().stdout(contains(
        "[pass] no git remote names an upstream's repository",
    ));
}

#[test]
fn one_unreachable_upstream_is_unavailable_and_start_creates_nothing() {
    let m = Mono::new();
    let env = &m.env;
    let gone = env.path("gone.git");
    let cfg = mono_config(&m.a, &m.b, "").replace(&m.b.url(), &gone.to_string_lossy());
    env.commit(&m.down, &[(".git-intake.toml", &cfg)], "mono: b moved");

    let status = env.tool_json(&m.down, &["status", "--json"], 1);
    assert_eq!(status["upstreams"][0]["state"], "not_imported");
    assert_eq!(
        status["upstreams"][0]["tip"],
        env.rev(&m.a.bare, "main").as_str()
    );
    let b = &status["upstreams"][1];
    assert_eq!(b["state"], "unavailable");
    let detail = b["detail"].as_str().unwrap();
    assert!(detail.starts_with("fetch failed: `git fetch"), "{detail}");
    assert!(detail.contains(&*gone.to_string_lossy()), "{detail}");
    let text = stdout(&m.tool(&["status"]).code(1));
    assert!(text.contains("app-b: fetch failed:"), "{text}");
    let log = env.tool_json(&m.down, &["log", "--json"], 1);
    assert_eq!(log["upstreams"][0]["available"], true);
    assert_eq!(log["upstreams"][1]["available"], false);

    m.tool(&["start"])
        .code(2)
        .stderr(contains("cannot fetch the selected upstreams"))
        .stderr(contains("upstream 'app-b':"));
    env.assert_no_update(&m.down);
}

#[test]
fn reading_commands_leave_no_reference_remote_or_branch_behind() {
    let m = Mono::new();
    let env = &m.env;
    m.import_all();
    env.push_upstream(&m.a, &[("x.txt", "x\n")], "app-a: x");
    let refs_before = env.git(&m.down, &["for-each-ref", "--format=%(refname)"]);
    for args in [
        vec!["status"],
        vec!["status", "--check"],
        vec!["log"],
        vec!["branches"],
        vec!["doctor"],
    ] {
        env.tool(&m.down, &args);
        assert_eq!(
            env.git(&m.down, &["for-each-ref", "--format=%(refname)"]),
            refs_before,
            "{args:?} left references behind"
        );
        assert_eq!(env.git(&m.down, &["remote"]), "", "{args:?} added a remote");
    }
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
        out.contains(&format!(
            "[pass] app-a: branch 'main' of {} was fetched at",
            m.a.url()
        )),
        "{out}"
    );
    assert!(
        out.contains("[pass] no reference under refs/git-intake/ is left without an update"),
        "{out}"
    );
    assert!(
        out.contains("[pass] no git remote names an upstream's repository"),
        "{out}"
    );
    assert!(
        out.contains("[pass] every journal belongs to an upstream in port mode"),
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
    env.assert_no_update(&m.down);

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

#[test]
fn state_files_of_another_format_are_named_or_ignored() {
    let f = Fork::new();
    let env = &f.env;
    let dir = env.intake_dir(&f.down);
    std::fs::create_dir_all(&dir).unwrap();
    // A finished update recorded by an earlier release only feeds `report`.
    std::fs::write(
        dir.join("last-update.json"),
        r#"{"version": 1, "config": {"integration": "main", "publish": "origin"}}"#,
    )
    .unwrap();
    f.tool(&["report"]).code(2).stderr(contains(
        "no update is in progress or finished; nothing to report",
    ));
    // An update in progress from the previous release, which kept remotes
    // and mirrors, is refused by every command that reads the update.
    std::fs::write(
        dir.join("state.json"),
        r#"{"version": 2, "config": {"integration": "main", "upstreams": [{"name": "upstream", "remote": "upstream", "branch": "main", "mirror": "upstream-main"}]}, "upstreams": [{"name": "upstream", "target": "0000000000000000000000000000000000000000"}]}"#,
    )
    .unwrap();
    for args in [
        vec!["status"],
        vec!["continue"],
        vec!["verify"],
        vec!["finish"],
        vec!["abort"],
        vec!["start"],
        vec!["report"],
    ] {
        let err = stderr(&f.tool(&args).code(2));
        assert!(
            err.contains(
                "the update in progress was started by a release that keeps its state in format 2; \
                 this release (git-intake 0.0.0) understands format 6. Finish or abort the update with a release that keeps format 2 (git-intake v0.2.0 to v0.2.2), then start it again with this release"
            ),
            "{args:?}: {err}"
        );
    }
    assert!(dir.join("state.json").exists(), "the state is kept");
    let doctor = stdout(&f.tool(&["doctor"]).code(1));
    assert!(doctor.contains("keeps its state in format 2"), "{doctor}");
}

#[test]
fn state_of_an_earlier_release_that_does_not_wait_is_refused() {
    let f = Fork::new();
    let env = &f.env;
    env.push_upstream(&f.up, &[("n.txt", "n\n")], "upstream: n");
    f.tool(&["start"]).success();
    f.tool(&["verify"]).success();
    let path = env.intake_dir(&f.down).join("state.json");
    let current: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let commands = [
        vec!["status"],
        vec!["continue"],
        vec!["verify"],
        vec!["finish"],
        vec!["abort"],
        vec!["start"],
        vec!["sync"],
        vec!["report"],
    ];
    for (version, advice) in [
        (
            5,
            "Finish or abort the update with a release that keeps format 5 (git-intake v0.3.6)",
        ),
        (
            4,
            "Finish or abort the update with a release that keeps format 4 (git-intake v0.3.2 to v0.3.5, \
             also carried on by v0.3.6)",
        ),
        (
            3,
            "Finish or abort the update with a release that keeps format 3 (git-intake v0.3.0 to v0.3.1), \
             or carry it over with `git intake migrate` of git-intake v0.3.6 and finish or abort it with that release",
        ),
    ] {
        let mut state = current.clone();
        common::as_earlier_release(&mut state, version, "0.3.6");
        let text = state.to_string();
        std::fs::write(&path, &text).unwrap();
        for args in &commands {
            let err = stderr(&f.tool(args).code(2));
            assert!(
                err.contains(&format!(
                    "the update in progress was started by a release (git-intake 0.3.6) that keeps its state in \
                     format {version}; this release (git-intake 0.0.0) understands format 6. {advice}"
                )),
                "{version} {args:?}: {err}"
            );
            assert_eq!(std::fs::read_to_string(&path).unwrap(), text, "{args:?}");
        }
        assert!(
            env.update_worktree(&f.down).exists(),
            "the worktree is kept"
        );
    }
}

#[test]
fn state_of_a_newer_or_unknown_format_is_refused() {
    let f = Fork::new();
    let env = &f.env;
    let dir = env.intake_dir(&f.down);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("state.json");
    let commands = [
        vec!["status"],
        vec!["continue"],
        vec!["verify"],
        vec!["finish"],
        vec!["abort"],
        vec!["start"],
        vec!["report"],
    ];

    // A newer release's state is refused, and the release that wrote it is
    // named when the state records it.
    let newer = r#"{"version": 99, "release": "9.9.9", "shape": "unknown to this release"}"#;
    std::fs::write(&path, newer).unwrap();
    for args in &commands {
        let err = stderr(&f.tool(args).code(2));
        assert!(
            err.contains(
                "the update in progress was started by a release (git-intake 9.9.9) that keeps its state in format 99; \
                 this release (git-intake 0.0.0) understands format 6, an older one"
            ),
            "{args:?}: {err}"
        );
        assert!(
            err.contains("Finish or abort the update with that release or a later one"),
            "{args:?}: {err}"
        );
    }
    assert_eq!(std::fs::read_to_string(&path).unwrap(), newer);

    // A state without a format version is refused as well.
    let unversioned = r#"{"config": {"integration": "main"}}"#;
    std::fs::write(&path, unversioned).unwrap();
    for args in &commands {
        let err = stderr(&f.tool(args).code(2));
        assert!(
            err.contains(
                "names no state format; this release (git-intake 0.0.0) understands format 6"
            ),
            "{args:?}: {err}"
        );
    }
    assert_eq!(std::fs::read_to_string(&path).unwrap(), unversioned);
}

/// Reads the update state of `f`, applies `edit` to it and writes it back.
fn edit_state(f: &Fork, edit: impl FnOnce(&mut serde_json::Value)) -> serde_json::Value {
    let path = f.env.intake_dir(&f.down).join("state.json");
    let mut state: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    edit(&mut state);
    std::fs::write(&path, serde_json::to_string_pretty(&state).unwrap()).unwrap();
    state
}

const OTHER_RELEASE: &str =
    "warning: the update was started by git-intake 0.0.1 and is carried on by git-intake 0.0.0";

#[test]
fn another_release_of_the_same_format_continues_with_a_warning_once() {
    let f = Fork::new();
    let env = &f.env;
    env.push_upstream(&f.up, &[("n.txt", "n\n")], "upstream: n");
    f.tool(&["start"]).success();
    // The state records the release that started the update.
    let state = edit_state(&f, |_| {});
    assert_eq!(state["release"], "0.0.0");

    // The same release carries on silently.
    let verified = f.tool(&["verify"]).success();
    assert!(
        !stderr(&verified).contains("warning:"),
        "{}",
        stderr(&verified)
    );

    // Another release warns once, and before `finish` advises how to use one
    // release throughout.
    edit_state(&f, |s| s["release"] = "0.0.1".into());
    let first = stderr(&f.tool(&["verify"]).success());
    assert!(first.contains(OTHER_RELEASE), "{first}");
    assert!(
        first.contains("run `git intake abort` and start the update again"),
        "{first}"
    );
    assert_eq!(edit_state(&f, |_| {})["release_warned"], "0.0.0");
    let again = stderr(&f.tool(&["verify"]).success());
    assert!(!again.contains("warning:"), "{again}");
    let finished = stderr(&f.tool(&["finish"]).success());
    assert!(!finished.contains("warning:"), "{finished}");
    env.assert_no_update(&f.down);
}

#[test]
fn another_release_after_the_push_is_not_advised_to_abort() {
    let f = Fork::merge_request();
    let env = &f.env;
    env.push_upstream(&f.up, &[("n.txt", "n\n")], "upstream: n");
    f.tool(&["start"]).success();
    // The forge refuses to open the merge request once the branch is
    // pushed: the update stays, with its head published.
    f.forge()
        .fail("POST", "/merge_requests", 500, "500 Internal Server Error");
    f.tool(&["finish"]).code(2);
    assert_eq!(
        env.rev(&f.publish, MERGE_REQUEST_BRANCH),
        env.rev(&env.update_worktree(&f.down), "HEAD")
    );

    // `abort` would leave the pushed branch behind, so the warning gives no
    // such advice once the update was pushed.
    edit_state(&f, |s| s["release"] = "0.0.1".into());
    f.forge().world().failures.clear();
    let first = stderr(&f.tool(&["finish"]).success());
    assert!(first.contains(OTHER_RELEASE), "{first}");
    assert!(!first.contains("abort"), "{first}");
    env.assert_no_update(&f.down);
}

/// A way the publish remote can push to the upstream's repository under a
/// URL that differs from the upstream's.
#[derive(Debug, Clone, Copy)]
enum Alias {
    /// An `https` URL with credentials, which git serves from the same
    /// repository as the upstream's URL.
    Credentials,
    /// A push URL that `url.<base>.insteadOf` rewrites to the upstream's.
    InsteadOf,
    /// The publish remote's own URL, which `url.<base>.pushInsteadOf`
    /// rewrites to the upstream's for pushes.
    PushInsteadOf,
    /// A `file://localhost/` push URL of the upstream's repository.
    FileLocalhost,
}

const ALIASES: [Alias; 4] = [
    Alias::Credentials,
    Alias::InsteadOf,
    Alias::PushInsteadOf,
    Alias::FileLocalhost,
];

impl Alias {
    /// What the configuration needs before an update starts: for
    /// [`Alias::Credentials`], the upstream reached by an `https` URL that
    /// the test's git configuration serves from its local repository.
    fn prepare(self, f: &Fork) {
        if let Alias::Credentials = self {
            let config = f.env.path("gitconfig");
            let mut text = std::fs::read_to_string(&config).unwrap();
            text.push_str(&format!(
                "[url {:?}]\n\tinsteadOf = https://example.com/\n\tinsteadOf = https://user:s3cret@example.com/\n",
                format!("{}/", f.env.path("").display())
            ));
            std::fs::write(&config, text).unwrap();
            f.reconfigure(
                &format!("url = {:?}", f.up.url()),
                "url = \"https://example.com/upstream.git\"",
            );
        }
    }

    /// Makes the publish remote push to the upstream's repository.
    fn apply(self, f: &Fork) {
        let env = &f.env;
        let up = f.up.url();
        match self {
            Alias::Credentials => env.git(
                &f.down,
                &[
                    "remote",
                    "set-url",
                    "--push",
                    "origin",
                    "https://user:s3cret@example.com/upstream.git",
                ],
            ),
            Alias::InsteadOf => {
                env.git(
                    &f.down,
                    &["config", &format!("url.{up}.insteadOf"), "upstream-alias:"],
                );
                env.git(
                    &f.down,
                    &["remote", "set-url", "--push", "origin", "upstream-alias:"],
                )
            }
            Alias::PushInsteadOf => env.git(
                &f.down,
                &[
                    "config",
                    &format!("url.{up}.pushInsteadOf"),
                    &f.publish.to_string_lossy(),
                ],
            ),
            Alias::FileLocalhost => env.git(
                &f.down,
                &[
                    "remote",
                    "set-url",
                    "--push",
                    "origin",
                    &format!("file://localhost{up}"),
                ],
            ),
        };
    }
}

/// Asserts that nothing of the downstream reached the upstream's
/// repository, whose `main` is still at `tip`.
fn assert_upstream_untouched(f: &Fork, tip: &str, alias: Alias) {
    assert_eq!(f.env.rev(&f.up.bare, "main"), tip, "{alias:?}");
    assert!(
        !f.env
            .git_output(&f.up.bare, &["cat-file", "-e", "main:.git-intake.toml"])
            .status
            .success(),
        "{alias:?}"
    );
}

#[test]
fn sync_never_pushes_to_an_upstream_under_another_url() {
    for alias in ALIASES {
        let f = Fork::new();
        alias.prepare(&f);
        alias.apply(&f);
        let tip = f
            .env
            .push_upstream(&f.up, &[("n.txt", "n\n")], "upstream: n");
        let err = stderr(&f.tool(&["sync"]).code(2));
        assert!(
            err.contains("names the repository of upstream 'upstream'"),
            "{alias:?}: {err}"
        );
        assert!(
            err.contains("the tool never pushes to an upstream"),
            "{alias:?}: {err}"
        );
        assert!(!err.contains("s3cret"), "{alias:?}: {err}");
        assert_upstream_untouched(&f, &tip, alias);
        f.env.assert_no_update(&f.down);
        let doctor = stdout(&f.tool(&["doctor"]).code(1));
        assert!(
            doctor.contains("names the repository of upstream 'upstream'")
                && doctor.contains("the tool never pushes to an upstream"),
            "{alias:?}: {doctor}"
        );
        assert!(!doctor.contains("s3cret"), "{alias:?}: {doctor}");
    }
}

#[test]
fn finish_never_pushes_to_an_upstream_under_another_url() {
    for alias in ALIASES {
        let f = Fork::new();
        alias.prepare(&f);
        let tip = f
            .env
            .push_upstream(&f.up, &[("n.txt", "n\n")], "upstream: n");
        f.tool(&["start"]).success();
        f.tool(&["verify"]).success();
        let main = f.env.rev(&f.down, "main");
        // The remote changes once the update is under way, after the
        // configuration was read.
        alias.apply(&f);
        let err = stderr(&f.tool(&["finish"]).code(2));
        assert!(
            err.contains("names the repository of upstream 'upstream'"),
            "{alias:?}: {err}"
        );
        assert!(
            err.contains("the tool never pushes to an upstream"),
            "{alias:?}: {err}"
        );
        assert!(!err.contains("s3cret"), "{alias:?}: {err}");
        assert_eq!(f.env.rev(&f.down, "main"), main, "{alias:?}");
        assert_upstream_untouched(&f, &tip, alias);
    }
}

/// Asserts that no run of `args` shows `secret`, in text or JSON.
fn assert_never_shown(f: &Fork, secret: &str, runs: &[&[&str]]) {
    for args in runs {
        let run = f.tool(args);
        let out = run.get_output();
        let shown = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(!shown.is_empty(), "{args:?}");
        assert!(!shown.contains(secret), "{args:?}: {shown}");
    }
}

#[test]
fn an_api_url_with_credentials_is_refused_without_showing_them() {
    let f = Fork::new();
    f.reconfigure(
        "via = \"push\"",
        "via = \"push\"\napi = \"https://user:s3cret@gitlab.example.com/api/v4\"",
    );
    let err = stderr(&f.tool(&["status"]).code(2));
    assert!(err.contains("carries credentials"), "{err}");
    assert!(err.contains("https://gitlab.example.com/api/v4"), "{err}");
    assert_never_shown(
        &f,
        "s3cret",
        &[
            &["status"],
            &["status", "--json"],
            &["doctor"],
            &["doctor", "--json"],
        ],
    );
}

#[test]
fn a_git_command_that_cannot_start_is_named_without_credentials() {
    let f = Fork::new();
    // An argument this long is more than the system lets a process start
    // with, so git cannot be run with it.
    let url = format!(
        "https://user:s3cret@example.com/{}.git",
        "a".repeat(3 * 1024 * 1024)
    );
    f.reconfigure(
        &format!("url = {:?}", f.up.url()),
        &format!("url = {url:?}"),
    );
    let err = stderr(&f.tool(&["status"]).code(2));
    assert!(
        err.contains("cannot run `git"),
        "{}",
        &err[..err.len().min(400)]
    );
    assert_never_shown(
        &f,
        "s3cret",
        &[&["status"], &["status", "--json"], &["doctor", "--json"]],
    );
}
