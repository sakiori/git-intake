//! Failure handling: partial merges, hostile configuration, interrupted
//! commands, and layouts that must not be overwritten.

mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use common::{Env, FORK_FILE, Fork, Mono, stderr, stdout};
use predicates::str::contains;

fn hook(repo: &Path, name: &str, body: &str) {
    let dir = repo.join(".git").join("hooks");
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
}

fn edit_state(repo: &Path, f: impl FnOnce(&mut serde_json::Value)) {
    let path = repo.join(".git").join("fork-sync").join("state.json");
    let mut state: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    f(&mut state);
    fs::write(&path, serde_json::to_string_pretty(&state).unwrap()).unwrap();
}

#[test]
fn failed_import_is_undone_and_stays_pending() {
    let m = Mono::new();
    let env = &m.env;
    hook(&m.down, "commit-msg", "exit 1");
    m.tool(&["start"])
        .code(2)
        .stderr(contains("importing upstream 'app-a' under 'apps/a' failed"))
        .stderr(contains("the import was undone"));
    let wt = env.update_worktree(&m.down);
    assert!(!env.has_rev(&wt, "MERGE_HEAD"), "the merge is aborted");
    assert!(!wt.join("apps/a").exists(), "the import left no files");
    let status = env.tool_json(&m.down, &["status", "--json"], 0);
    assert_eq!(status["update"]["upstreams"][0]["step"], "pending");

    fs::remove_file(m.down.join(".git/hooks/commit-msg")).unwrap();
    m.tool(&["continue"])
        .success()
        .stdout(contains("app-a: merged as"))
        .stdout(contains("app-b: merged as"));
    assert!(wt.join("apps/a/a.txt").exists());
    assert!(wt.join("apps/b/b.txt").exists());
}

#[test]
fn merge_without_its_sync_record_is_not_counted() {
    let f = Fork::new();
    let env = &f.env;
    env.push_upstream(&f.up, &[("n.txt", "n\n")], "upstream: n");
    hook(&f.down, "commit-msg", "echo rewritten > \"$1\"");
    f.tool(&["start"])
        .code(2)
        .stderr(contains("is not a complete sync merge"))
        .stderr(contains("does not end with the sync record trailers"));
    let status = env.tool_json(&f.down, &["status", "--json"], 0);
    assert_eq!(status["update"]["upstreams"][0]["step"], "pending");
    f.tool(&["verify"])
        .code(2)
        .stderr(contains("upstream (pending)"));
}

#[test]
fn git_directory_path_segment_is_rejected() {
    let m = Mono::new();
    let env = &m.env;
    let cfg =
        common::mono_config(&m.a, &m.b, "").replace("path = \"apps/a\"", "path = \"vendor/.Git\"");
    env.commit(&m.down, &[(".fork-sync.toml", &cfg)], "bad: .git path");
    m.tool(&["status"]).code(2).stderr(contains(
        "path 'vendor/.Git': the path may not contain a '.git' segment",
    ));
}

#[test]
fn import_refuses_to_replace_a_file_on_the_path() {
    let m = Mono::new();
    let env = &m.env;
    env.commit(
        &m.down,
        &[("apps", "a file, not a directory\n")],
        "mono: apps file",
    );
    m.tool(&["start"]).code(2).stderr(contains(
        "'apps' is a file on the integration branch; importing under 'apps/a' would replace it",
    ));
    env.assert_no_update(&m.down);
    assert_eq!(
        fs::read_to_string(m.down.join("apps")).unwrap(),
        "a file, not a directory\n"
    );
    let check = env.tool_json(&m.down, &["status", "--check", "--json"], 1);
    assert!(
        check["upstreams"][0]["detail"]
            .as_str()
            .unwrap()
            .contains("'apps' is a file")
    );
}

#[test]
fn finish_never_moves_integration_backwards_or_without_targets() {
    let f = Fork::new();
    let env = &f.env;
    env.push_upstream(&f.up, &[("n.txt", "n\n")], "upstream: n");
    f.tool(&["start"]).success();
    let base = env.rev(&f.down, "main");
    let wt = env.update_worktree(&f.down);
    // With the integration branch not checked out, finish moves it with
    // update-ref, which does not itself insist on a fast-forward.
    env.git(&f.down, &["checkout", "-q", "--detach"]);

    // A verified head that does not contain the base.
    env.git(&wt, &["reset", "-q", "--hard", "HEAD^1^"]);
    env.git(&wt, &["checkout", &base, "--", ".fork-sync.toml"]);
    env.git(&wt, &["commit", "-q", "-m", "elsewhere"]);
    f.tool(&["verify"]).success();
    f.tool(&["finish"])
        .code(2)
        .stderr(contains("does not contain the base"));
    assert_eq!(env.rev(&f.down, "main"), base);

    // A verified head that contains the base but not the target.
    env.git(&wt, &["reset", "-q", "--hard", &base]);
    env.commit(&wt, &[("x.txt", "x\n")], "no upstream here");
    f.tool(&["verify"]).success();
    f.tool(&["finish"])
        .code(2)
        .stderr(contains("does not contain the targets of"));
    assert_eq!(env.rev(&f.down, "main"), base);
    assert!(!env.has_rev(&f.down, "refs/heads/upstream-main"));
}

#[test]
fn finish_validates_every_checkout_before_moving_anything() {
    let f = Fork::new();
    let env = &f.env;
    // The mirror exists and is checked out in another worktree.
    env.git(&f.down, &["branch", "upstream-main", "upstream/main"]);
    let mirror_wt = env.path("mirror-wt");
    env.git(
        &f.down,
        &[
            "worktree",
            "add",
            "-q",
            &mirror_wt.to_string_lossy(),
            "upstream-main",
        ],
    );
    env.push_upstream(&f.up, &[("n.txt", "n\n")], "upstream: n");
    f.tool(&["start"]).success();
    f.tool(&["verify"]).success();
    let base = env.rev(&f.down, "main");

    // An untracked file the mirror's fast-forward would overwrite.
    fs::write(mirror_wt.join("n.txt"), "mine\n").unwrap();
    f.tool(&["finish"])
        .code(2)
        .stderr(contains("'upstream-main' is checked out in"))
        .stderr(contains("cannot follow the fast-forward"));
    assert_eq!(env.rev(&f.down, "main"), base, "nothing moved");
    assert_eq!(env.rev(&f.publish, "main"), base, "nothing published");

    fs::remove_file(mirror_wt.join("n.txt")).unwrap();
    f.tool(&["finish"]).success();
    assert_eq!(fs::read_to_string(mirror_wt.join("n.txt")).unwrap(), "n\n");
    assert!(!env.has_rev(&f.down, "refs/heads/fork-sync/update"));
    assert!(!env.fork_sync_dir(&f.down).join("state.json").exists());
}

#[test]
fn interrupted_finish_resumes_and_blocks_abort() {
    let f = Fork::new();
    let env = &f.env;
    let target = env.push_upstream(&f.up, &[("n.txt", "n\n")], "upstream: n");
    f.tool(&["start"]).success();
    f.tool(&["verify"]).success();
    let base = env.rev(&f.down, "main");
    let head = env.rev(&env.update_worktree(&f.down), "HEAD");

    // finish planned its moves and moved the integration branch, then stopped.
    edit_state(&f.down, |s| {
        s["finishing"] = serde_json::json!({
            "moves": [
                {"branch": "main", "old": base, "new": head},
                {"branch": "upstream-main", "old": null, "new": target},
            ],
            "push_error": null,
        });
    });
    env.git(&f.down, &["merge", "-q", "--ff-only", &head]);

    let status = env.tool_json(&f.down, &["status", "--json"], 0);
    assert_eq!(status["update"]["step"], "finishing");
    assert_eq!(status["update"]["next"], "git fork-sync finish");
    f.tool(&["status"])
        .success()
        .stdout(contains("update in progress: finishing"));
    f.tool(&["abort"])
        .code(2)
        .stderr(contains("`finish` has already moved main"));
    f.tool(&["continue"])
        .code(2)
        .stderr(contains("`finish` is in progress"));
    f.tool(&["verify"])
        .code(2)
        .stderr(contains("`finish` is in progress"));

    f.tool(&["finish"])
        .success()
        .stdout(contains("resuming finish"))
        .stdout(contains(format!("main: at {}", &head[..12])))
        .stdout(contains("pushed main, upstream-main to 'origin'"));
    assert_eq!(env.rev(&f.down, "upstream-main"), target);
    assert_eq!(env.rev(&f.publish, "main"), head);
    env.assert_no_update(&f.down);
}

#[test]
fn merge_log_configuration_does_not_break_the_record() {
    let f = Fork::new();
    let env = &f.env;
    env.git(&f.down, &["config", "merge.log", "true"]);
    let target = env.push_upstream(&f.up, &[("n.txt", "n\n")], "upstream: n");
    f.tool(&["start"]).success();
    f.tool(&["verify"]).success();
    f.tool(&["finish"]).success();
    let msg = env.git(&f.down, &["log", "-1", "--format=%B", "main"]);
    assert!(
        msg.trim_end()
            .ends_with(&format!("Fork-Sync-Target: {target}")),
        "{msg}"
    );
    let status = env.tool_json(&f.down, &["status", "--json"], 0);
    assert_eq!(status["upstreams"][0]["last_synced"], target.as_str());
}

/// A downstream repository and a path upstream created from one template,
/// so they share an ancestor.
fn template_pair() -> (Env, common::Upstream, std::path::PathBuf) {
    let env = Env::new();
    let tmpl = env.work("template");
    env.commit(&tmpl, &[("template.txt", "t\n")], "template");
    let bare = env.bare("app-a");
    let work = env.path("app-a-src");
    env.git(
        &env.path(""),
        &[
            "clone",
            "-q",
            &tmpl.to_string_lossy(),
            &work.to_string_lossy(),
        ],
    );
    env.git(
        &work,
        &["remote", "set-url", "origin", &bare.to_string_lossy()],
    );
    env.commit(&work, &[("a.txt", "a\n")], "app-a: own work");
    env.git(&work, &["push", "-q", "origin", "HEAD:main"]);
    let down = env.path("down");
    env.git(
        &env.path(""),
        &[
            "clone",
            "-q",
            &tmpl.to_string_lossy(),
            &down.to_string_lossy(),
        ],
    );
    env.git(&down, &["remote", "remove", "origin"]);
    let up = common::Upstream { bare, work };
    let cfg = format!(
        "integration = \"main\"\nchecks = [\"test -f apps/a/a.txt\"]\n\n[[upstream]]\nname = \"app-a\"\nremote = \"app-a\"\nbranch = \"main\"\nurl = {:?}\npath = \"apps/a\"\n",
        up.url()
    );
    env.commit(&down, &[(".fork-sync.toml", &cfg)], "mono: configuration");
    (env, up, down)
}

#[test]
fn upstream_sharing_a_template_ancestor_is_imported() {
    let (env, _up, down) = template_pair();
    env.tool(&down, &["fetch"]).success();
    let status = env.tool_json(&down, &["status", "--json"], 0);
    assert_eq!(status["upstreams"][0]["state"], "not_imported");
    env.tool(&down, &["doctor"])
        .success()
        .stdout(contains("not imported yet; path 'apps/a' will be created"));

    env.tool(&down, &["start"]).success();
    env.tool(&down, &["verify"]).success();
    env.tool(&down, &["finish"]).success();
    assert!(down.join("apps/a/a.txt").exists());
    assert!(down.join("apps/a/template.txt").exists());
    let status = env.tool_json(&down, &["status", "--json"], 0);
    assert_eq!(status["upstreams"][0]["state"], "current");
}

#[test]
fn imported_path_removed_downstream_is_refused() {
    let (env, up, down) = template_pair();
    env.tool(&down, &["start"]).success();
    env.tool(&down, &["verify"]).success();
    env.tool(&down, &["finish"]).success();
    env.git(&down, &["rm", "-q", "-r", "apps/a"]);
    env.git(&down, &["commit", "-q", "-m", "mono: remove apps/a"]);
    env.push_upstream(&up, &[("b.txt", "b\n")], "app-a: more");
    env.tool(&down, &["start"]).code(2).stderr(contains(
        "was imported before, but path 'apps/a' was removed on the integration branch",
    ));
    env.tool(&down, &["doctor"]).code(1).stdout(contains(
        "upstream is imported but path 'apps/a' does not exist",
    ));
}

#[test]
fn rerere_autoupdate_still_reports_the_conflict() {
    let f = Fork::new();
    let env = &f.env;
    env.git(&f.down, &["config", "rerere.autoupdate", "true"]);
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
    fs::write(wt.join(FORK_FILE), "one\nTWO-both\nthree\n").unwrap();
    env.git(&wt, &["add", FORK_FILE]);
    f.tool(&["continue"]).success();
    f.tool(&["abort"]).success();

    let again = f.tool(&["start"]).code(1);
    let out = stdout(&again);
    assert!(
        out.contains("app.txt  (resolved by a recorded resolution)"),
        "{out}"
    );
    assert!(!out.contains("stopped without conflicted files"), "{out}");
    let report = env.tool_json(&f.down, &["report", "--json"], 0);
    assert_eq!(report["upstreams"][0]["conflicts"][0]["path"], "app.txt");
    assert_eq!(
        report["upstreams"][0]["conflicts"][0]["resolution"],
        "recorded"
    );
    f.tool(&["continue"]).success();
}

#[test]
fn continue_recognises_a_merge_completed_before_the_state_was_saved() {
    let m = Mono::new();
    m.tool(&["start"]).success();
    // As if the process stopped right after app-b's import commit.
    edit_state(&m.down, |s| {
        s["upstreams"][1]["step"] = "pending".into();
        s["upstreams"][1]["merge_commit"] = serde_json::Value::Null;
    });
    m.tool(&["continue"])
        .success()
        .stdout(contains("app-b: merged as"))
        .stdout(contains("next: git fork-sync verify"));
    m.tool(&["verify"]).success();
}

#[test]
fn verify_rejects_an_invalid_merged_configuration() {
    let f = Fork::new();
    let env = &f.env;
    env.push_upstream(&f.up, &[("n.txt", "n\n")], "upstream: n");
    f.tool(&["start"]).success();
    let wt = env.update_worktree(&f.down);
    let good = fs::read_to_string(wt.join(".fork-sync.toml")).unwrap();

    env.commit(
        &wt,
        &[(".fork-sync.toml", &format!("{good}\nbogus = 1\n"))],
        "bad config",
    );
    let v = f.tool(&["verify"]).code(1);
    assert!(
        stdout(&v).contains("the merged .fork-sync.toml is invalid"),
        "{}",
        stdout(&v)
    );
    assert!(
        stdout(&v).contains("skipped  test -f app.txt"),
        "{}",
        stdout(&v)
    );

    env.commit(
        &wt,
        &[(
            ".fork-sync.toml",
            &good.replace("integration = \"main\"", "integration = \"develop\""),
        )],
        "other integration",
    );
    f.tool(&["verify"]).code(1).stdout(contains(
        "names 'develop' as the integration branch instead of 'main'",
    ));
    f.tool(&["finish"]).code(2);

    env.commit(&wt, &[(".fork-sync.toml", &good)], "restore config");
    f.tool(&["verify"]).success();
    let _ = stderr(&f.tool(&["finish"]).success());
}

fn reject_pushes(bare: &Path) {
    let dir = bare.join("hooks");
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("pre-receive");
    fs::write(&path, "#!/bin/sh\necho 'protected branch' >&2\nexit 1\n").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
fn rerun_finish_publishes_only_the_planned_commits() {
    let f = Fork::new();
    let env = &f.env;
    env.push_upstream(&f.up, &[("n.txt", "n\n")], "upstream: n");
    f.tool(&["start"]).success();
    f.tool(&["verify"]).success();
    let planned = env.rev(&env.update_worktree(&f.down), "HEAD");

    reject_pushes(&f.publish);
    f.tool(&["finish"]).code(2).stderr(contains("was rejected"));
    // A local commit lands on the integration branch while finishing.
    let local = env.commit(
        &f.down,
        &[("local.txt", "unverified\n")],
        "fork: unverified work",
    );
    fs::remove_file(f.publish.join("hooks/pre-receive")).unwrap();

    f.tool(&["finish"])
        .success()
        .stdout(contains(format!(
            "main: at {}, which contains",
            &local[..12]
        )))
        .stdout(contains("pushed main, upstream-main to 'origin'"));
    assert_eq!(
        env.rev(&f.publish, "main"),
        planned,
        "only the planned commit is published"
    );
    assert_eq!(
        env.rev(&f.down, "main"),
        local,
        "the local commit stays local"
    );
    env.assert_no_update(&f.down);
}

#[test]
fn finish_no_publish_closes_an_update_the_remote_always_rejects() {
    let f = Fork::new();
    let env = &f.env;
    let target = env.push_upstream(&f.up, &[("n.txt", "n\n")], "upstream: n");
    f.tool(&["start"]).success();
    f.tool(&["verify"]).success();
    let planned = env.rev(&env.update_worktree(&f.down), "HEAD");
    let published = env.rev(&f.publish, "main");

    reject_pushes(&f.publish);
    f.tool(&["finish"])
        .code(2)
        .stderr(contains("finish --no-publish"));
    f.tool(&["finish"]).code(2);
    f.tool(&["finish", "--no-publish"])
        .success()
        .stdout(contains(
            "not published to 'origin' (advanced locally only): main, upstream-main",
        ))
        .stdout(contains("update finished"));
    assert_eq!(env.rev(&f.down, "main"), planned);
    assert_eq!(env.rev(&f.down, "upstream-main"), target);
    assert_eq!(
        env.rev(&f.publish, "main"),
        published,
        "the remote is unchanged"
    );
    assert!(!env.has_rev(&f.publish, "refs/heads/upstream-main"));
    env.assert_no_update(&f.down);
    let report = env.tool_json(&f.down, &["report", "--json"], 0);
    assert_eq!(report["in_progress"], false);
    assert_eq!(
        report["unpublished"],
        serde_json::json!(["main", "upstream-main"])
    );
    f.tool(&["report"]).success().stdout(contains(
        "not published (advanced locally only): main, upstream-main",
    ));
}

#[test]
fn finish_no_publish_works_before_any_push_attempt() {
    let f = Fork::new();
    let env = &f.env;
    env.push_upstream(&f.up, &[("n.txt", "n\n")], "upstream: n");
    f.tool(&["start"]).success();
    f.tool(&["finish", "--no-publish"])
        .code(2)
        .stderr(contains("not verified"));
    f.tool(&["verify"]).success();
    let planned = env.rev(&env.update_worktree(&f.down), "HEAD");
    let published = env.rev(&f.publish, "main");
    f.tool(&["finish", "--no-publish"])
        .success()
        .stdout(contains("not published to 'origin'"));
    assert_eq!(env.rev(&f.down, "main"), planned);
    assert_eq!(env.rev(&f.publish, "main"), published);
    env.assert_no_update(&f.down);
}

#[test]
fn path_imported_by_subtree_add_is_adopted() {
    let env = Env::new();
    let up = env.upstream("app-a", &[("a.txt", "a1\na2\n")]);
    let down = env.work("mono");
    let cfg = format!(
        "integration = \"main\"\nchecks = [\"test -f apps/a/a.txt\"]\n\n[[upstream]]\nname = \"app-a\"\nremote = \"app-a\"\nbranch = \"main\"\nurl = {:?}\npath = \"apps/a\"\n",
        up.url()
    );
    env.commit(&down, &[(".fork-sync.toml", &cfg)], "mono: configuration");
    env.tool(&down, &["fetch"]).success();
    let first = env.rev(&down, "refs/remotes/app-a/main");
    // What `git subtree add --prefix=apps/a` does: no sync record.
    env.git(
        &down,
        &[
            "merge",
            "-q",
            "-s",
            "ours",
            "--no-commit",
            "--allow-unrelated-histories",
            "app-a/main",
        ],
    );
    env.git(
        &down,
        &["read-tree", "--prefix=apps/a/", "-u", "app-a/main"],
    );
    env.git(&down, &["commit", "-q", "-m", "Add 'apps/a/' from app-a"]);

    let status = env.tool_json(&down, &["status", "--json"], 0);
    let u = &status["upstreams"][0];
    assert_eq!(u["state"], "current");
    assert_eq!(u["last_synced"], first.as_str());
    assert_eq!(u["last_synced_source"], "history");

    let second = env.push_upstream(&up, &[("a.txt", "a1\na2-up\n")], "app-a: change");
    env.tool(&down, &["fetch"]).success();
    let status = env.tool_json(&down, &["status", "--json"], 0);
    let u = &status["upstreams"][0];
    assert_eq!(u["state"], "behind");
    assert_eq!(u["commits_behind"], 1);
    assert_eq!(u["last_synced"], first.as_str());
    assert_eq!(u["last_synced_source"], "history");
    env.tool(&down, &["status"])
        .success()
        .stdout(contains("app-a: no sync record yet"));
    let check = env.tool_json(&down, &["status", "--check", "--json"], 1);
    assert_eq!(check["upstreams"][0]["conflicts"], serde_json::json!([]));
    env.tool(&down, &["doctor"])
        .success()
        .stdout(contains("path 'apps/a' was imported without a sync record"));

    env.tool(&down, &["start"]).success();
    env.tool(&down, &["verify"]).success();
    env.tool(&down, &["finish"]).success();
    assert_eq!(
        fs::read_to_string(down.join("apps/a/a.txt")).unwrap(),
        "a1\na2-up\n"
    );
    assert!(!down.join("a.txt").exists());
    let status = env.tool_json(&down, &["status", "--json"], 0);
    let u = &status["upstreams"][0];
    assert_eq!(u["state"], "current");
    assert_eq!(u["last_synced"], second.as_str());
    assert_eq!(u["last_synced_source"], "record");
}

#[test]
fn conflicts_are_labelled_recorded_only_when_rerere_reports_them() {
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
    // rerere is switched off for this run, so nothing can be resolved by it.
    let start = env.tool_env(
        &f.down,
        &["start"],
        &[
            ("GIT_CONFIG_COUNT", "1"),
            ("GIT_CONFIG_KEY_0", "rerere.enabled"),
            ("GIT_CONFIG_VALUE_0", "false"),
        ],
    );
    let out = stdout(&start.code(1));
    assert!(out.contains("app.txt  (unresolved)"), "{out}");
    let report = env.tool_json(&f.down, &["report", "--json"], 0);
    assert_eq!(
        report["upstreams"][0]["conflicts"][0]["resolution"],
        "unresolved"
    );
}

/// Two upstreams created from one template, and a monorepo that is not.
struct Twins {
    env: Env,
    a: common::Upstream,
    b: common::Upstream,
    down: std::path::PathBuf,
}

fn twins() -> Twins {
    let env = Env::new();
    let tmpl = env.work("template");
    env.commit(&tmpl, &[("template.txt", "t\n")], "template");
    let mut ups = Vec::new();
    for name in ["app-a", "app-b"] {
        let bare = env.bare(name);
        let work = env.path(&format!("{name}-src"));
        env.git(
            &env.path(""),
            &[
                "clone",
                "-q",
                &tmpl.to_string_lossy(),
                &work.to_string_lossy(),
            ],
        );
        env.git(
            &work,
            &["remote", "set-url", "origin", &bare.to_string_lossy()],
        );
        env.commit(
            &work,
            &[(&format!("{name}.txt") as &str, "own\n")],
            &format!("{name}: own work"),
        );
        env.git(&work, &["push", "-q", "origin", "HEAD:main"]);
        ups.push(common::Upstream { bare, work });
    }
    let b = ups.pop().unwrap();
    let a = ups.pop().unwrap();
    let down = env.work("mono");
    let cfg = format!(
        "integration = \"main\"\n\n\
         [[upstream]]\nname = \"app-a\"\nremote = \"app-a\"\nbranch = \"main\"\nurl = {:?}\npath = \"apps/a\"\n\n\
         [[upstream]]\nname = \"app-b\"\nremote = \"app-b\"\nbranch = \"main\"\nurl = {:?}\npath = \"apps/b\"\n",
        a.url(),
        b.url()
    );
    env.commit(&down, &[(".fork-sync.toml", &cfg)], "mono: configuration");
    Twins { env, a, b, down }
}

fn assert_b_unrelated(t: &Twins) {
    let env = &t.env;
    let status = env.tool_json(&t.down, &["status", "--json"], 0);
    let b = &status["upstreams"][1];
    assert_eq!(b["state"], "not_imported", "{b}");
    assert_eq!(b["last_synced"], serde_json::Value::Null, "{b}");
    let check = env.tool_json(&t.down, &["status", "--check", "--json"], 1);
    assert!(
        check["upstreams"][1]["detail"]
            .as_str()
            .unwrap()
            .contains("path 'apps/b' holds content unrelated to the history of app-b/main"),
        "{check}"
    );
    env.tool(&t.down, &["start", "app-b"])
        .code(2)
        .stderr(contains(
            "upstream 'app-b': path 'apps/b' holds content unrelated",
        ));
    env.assert_no_update(&t.down);
    env.tool(&t.down, &["doctor"]).code(1).stdout(contains(
        "[FAIL] app-b: upstream 'app-b': path 'apps/b' holds content unrelated",
    ));
}

#[test]
fn ancestry_shared_only_through_a_template_is_not_an_import() {
    let t = twins();
    let env = &t.env;
    env.commit(
        &t.down,
        &[("apps/b/own.txt", "the monorepo's own code\n")],
        "mono: own apps/b",
    );
    env.tool(&t.down, &["fetch"]).success();

    // Before app-a is imported.
    assert_b_unrelated(&t);

    // After app-a, which shares the template with app-b, is imported.
    env.tool(&t.down, &["start", "app-a"]).success();
    env.tool(&t.down, &["verify"]).success();
    env.tool(&t.down, &["finish"]).success();
    assert!(t.down.join("apps/a/template.txt").exists());
    assert_b_unrelated(&t);
    assert_eq!(
        fs::read_to_string(t.down.join("apps/b/own.txt")).unwrap(),
        "the monorepo's own code\n"
    );
}

#[test]
fn subtree_add_import_sharing_a_template_is_adopted() {
    let t = twins();
    let env = &t.env;
    env.tool(&t.down, &["fetch"]).success();
    env.tool(&t.down, &["start", "app-a"]).success();
    env.tool(&t.down, &["verify"]).success();
    env.tool(&t.down, &["finish"]).success();
    // app-b imported by other means, as `git subtree add` does.
    let b1 = env.rev(&t.down, "refs/remotes/app-b/main");
    env.git(
        &t.down,
        &[
            "merge",
            "-q",
            "-s",
            "ours",
            "--no-commit",
            "--allow-unrelated-histories",
            "app-b/main",
        ],
    );
    env.git(
        &t.down,
        &["read-tree", "--prefix=apps/b/", "-u", "app-b/main"],
    );
    env.git(&t.down, &["commit", "-q", "-m", "Add 'apps/b/' from app-b"]);

    let status = env.tool_json(&t.down, &["status", "--json"], 0);
    let b = &status["upstreams"][1];
    assert_eq!(b["state"], "current");
    assert_eq!(b["last_synced"], b1.as_str());
    assert_eq!(b["last_synced_source"], "history");

    let b2 = env.push_upstream(&t.b, &[("app-b.txt", "own\nmore\n")], "app-b: more");
    env.tool(&t.down, &["fetch"]).success();
    let status = env.tool_json(&t.down, &["status", "--json"], 0);
    assert_eq!(status["upstreams"][1]["state"], "behind");
    assert_eq!(status["upstreams"][1]["last_synced"], b1.as_str());
    env.tool(&t.down, &["start"]).success();
    env.tool(&t.down, &["verify"]).success();
    env.tool(&t.down, &["finish"]).success();
    assert_eq!(
        fs::read_to_string(t.down.join("apps/b/app-b.txt")).unwrap(),
        "own\nmore\n"
    );
    let status = env.tool_json(&t.down, &["status", "--json"], 0);
    assert_eq!(status["upstreams"][1]["last_synced"], b2.as_str());
    assert_eq!(status["upstreams"][1]["last_synced_source"], "record");
    let _ = &t.a;
}

#[test]
fn unusual_file_names_are_listed_once_and_raw() {
    let f = Fork::new();
    let env = &f.env;
    let names = ["café.txt", "q\"uote.txt"];
    env.push_upstream(
        &f.up,
        &[(names[0], "1\n2\n3\n"), (names[1], "1\n2\n3\n")],
        "upstream: add files",
    );
    f.tool(&["start"]).success();
    f.tool(&["verify"]).success();
    f.tool(&["finish"]).success();
    env.commit(
        &f.down,
        &[(names[0], "1\nfork\n3\n"), (names[1], "1\nfork\n3\n")],
        "fork: edit",
    );
    env.git(&f.down, &["push", "-q", "origin", "main"]);
    env.push_upstream(
        &f.up,
        &[(names[0], "1\nup\n3\n"), (names[1], "1\nup\n3\n")],
        "upstream: edit",
    );

    let first = stdout(&f.tool(&["start"]).code(1));
    for n in names {
        assert!(first.contains(&format!("  {n}  (unresolved)")), "{first}");
    }
    let wt = env.update_worktree(&f.down);
    for n in names {
        fs::write(wt.join(n), "1\nboth\n3\n").unwrap();
        env.git(&wt, &["add", n]);
    }
    f.tool(&["continue"]).success();
    f.tool(&["abort"]).success();

    let again = stdout(&f.tool(&["start"]).code(1));
    for n in names {
        assert_eq!(again.matches(n).count(), 1, "{again}");
        assert!(
            again.contains(&format!("  {n}  (resolved by a recorded resolution)")),
            "{again}"
        );
    }
    let report = env.tool_json(&f.down, &["report", "--json"], 0);
    let conflicts = report["upstreams"][0]["conflicts"].as_array().unwrap();
    assert_eq!(conflicts.len(), 2, "{report}");
    let mut paths: Vec<&str> = conflicts
        .iter()
        .map(|c| c["path"].as_str().unwrap())
        .collect();
    paths.sort();
    assert_eq!(paths, vec!["café.txt", "q\"uote.txt"]);
    assert!(
        conflicts.iter().all(|c| c["resolution"] == "recorded"),
        "{report}"
    );
}

#[test]
fn recovered_conflict_keeps_recorded_resolution_labels() {
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
    fs::write(wt.join(FORK_FILE), "one\nTWO-both\nthree\n").unwrap();
    env.git(&wt, &["add", FORK_FILE]);
    f.tool(&["continue"]).success();
    f.tool(&["abort"]).success();
    f.tool(&["start"]).code(1);

    // As if the process stopped after the merge but before saving the conflict.
    let forget = |s: &mut serde_json::Value| {
        s["upstreams"][0]["step"] = "pending".into();
        s["upstreams"][0]["conflicts"] = serde_json::json!([]);
    };
    edit_state(&f.down, forget);
    f.tool(&["continue"])
        .code(1)
        .stdout(contains("app.txt  (resolved by a recorded resolution)"));
    let report = env.tool_json(&f.down, &["report", "--json"], 0);
    assert_eq!(
        report["upstreams"][0]["conflicts"][0]["resolution"],
        "recorded"
    );

    // Without the saved output, nothing is claimed as a recorded resolution.
    edit_state(&f.down, forget);
    fs::remove_file(
        env.fork_sync_dir(&f.down)
            .join("logs")
            .join("merge-upstream.out"),
    )
    .unwrap();
    f.tool(&["continue"])
        .code(1)
        .stdout(contains("app.txt  (unresolved)"));
}

/// Imports an upstream at `prefix` the way `git subtree add` does: an `ours`
/// merge plus `read-tree`, with no sync record.
fn subtree_add(env: &Env, repo: &Path, prefix: &str, rev: &str) {
    env.git(
        repo,
        &[
            "merge",
            "-q",
            "-s",
            "ours",
            "--no-commit",
            "--allow-unrelated-histories",
            rev,
        ],
    );
    env.git(
        repo,
        &["read-tree", &format!("--prefix={prefix}/"), "-u", rev],
    );
    env.git(
        repo,
        &["commit", "-q", "-m", &format!("Add '{prefix}/' from {rev}")],
    );
}

#[test]
fn unrelated_path_is_refused_whichever_upstreams_are_fetched() {
    let t = twins();
    let env = &t.env;
    env.commit(
        &t.down,
        &[("apps/b/own.txt", "the monorepo's own code\n")],
        "mono: own apps/b",
    );
    env.tool(&t.down, &["start", "app-a"]).success();
    env.tool(&t.down, &["verify"]).success();
    env.tool(&t.down, &["finish"]).success();

    // A fresh clone in which only app-b has been fetched.
    let clone = env.path("clone");
    env.git(
        &env.path(""),
        &[
            "clone",
            "-q",
            &t.down.to_string_lossy(),
            &clone.to_string_lossy(),
        ],
    );
    env.tool(&clone, &["fetch", "app-b"]).success();
    assert!(!env.has_rev(&clone, "refs/remotes/app-a/main"));
    let status = env.tool_json(&clone, &["status", "--json"], 0);
    let b = &status["upstreams"][1];
    assert_eq!(b["state"], "not_imported", "{b}");
    assert_eq!(b["last_synced"], serde_json::Value::Null);
    env.tool(&clone, &["start", "app-b"])
        .code(2)
        .stderr(contains(
            "upstream 'app-b': path 'apps/b' holds content unrelated",
        ));
    env.assert_no_update(&clone);
}

#[test]
fn import_without_record_survives_another_upstream_merging_it() {
    let t = twins();
    let env = &t.env;
    env.tool(&t.down, &["fetch"]).success();
    let a1 = env.rev(&t.down, "refs/remotes/app-a/main");
    subtree_add(env, &t.down, "apps/a", "app-a/main");
    env.tool(&t.down, &["start", "app-b"]).success();
    env.tool(&t.down, &["verify"]).success();
    env.tool(&t.down, &["finish"]).success();

    // app-b merges app-a's history, and the monorepo takes app-b in.
    env.git(&t.b.work, &["fetch", "-q", &t.a.url(), "main"]);
    env.git(&t.b.work, &["merge", "-q", "--no-edit", "FETCH_HEAD"]);
    env.git(&t.b.work, &["push", "-q", "origin", "HEAD:main"]);
    env.tool(&t.down, &["fetch"]).success();
    env.tool(&t.down, &["start", "app-b"]).success();
    env.tool(&t.down, &["verify"]).success();
    env.tool(&t.down, &["finish"]).success();

    let status = env.tool_json(&t.down, &["status", "--json"], 0);
    let a = &status["upstreams"][0];
    assert_eq!(a["state"], "current", "{a}");
    assert_eq!(a["last_synced"], a1.as_str());
    assert_eq!(a["last_synced_source"], "history");

    let a2 = env.push_upstream(&t.a, &[("app-a.txt", "own\nmore\n")], "app-a: more");
    env.tool(&t.down, &["fetch"]).success();
    let status = env.tool_json(&t.down, &["status", "--json"], 0);
    let a = &status["upstreams"][0];
    assert_eq!(a["state"], "behind", "{a}");
    assert_eq!(a["last_synced"], a1.as_str());
    assert_eq!(a["last_synced_source"], "history");
    env.tool(&t.down, &["doctor"])
        .success()
        .stdout(contains("path 'apps/a' was imported without a sync record"));
    env.tool(&t.down, &["start", "app-a"]).success();
    env.tool(&t.down, &["verify"]).success();
    env.tool(&t.down, &["finish"]).success();
    assert_eq!(
        fs::read_to_string(t.down.join("apps/a/app-a.txt")).unwrap(),
        "own\nmore\n"
    );
    let status = env.tool_json(&t.down, &["status", "--json"], 0);
    assert_eq!(status["upstreams"][0]["last_synced"], a2.as_str());
    assert_eq!(status["upstreams"][0]["last_synced_source"], "record");
}

#[test]
fn subtree_pull_merge_without_record_is_the_newest_sync() {
    let t = twins();
    let env = &t.env;
    env.tool(&t.down, &["fetch"]).success();
    subtree_add(env, &t.down, "apps/a", "app-a/main");
    // A later `git subtree pull`-style merge, again without a record.
    let a2 = env.push_upstream(&t.a, &[("app-a.txt", "own\n2\n")], "app-a: 2");
    env.tool(&t.down, &["fetch"]).success();
    env.git(
        &t.down,
        &[
            "merge",
            "-q",
            "--no-edit",
            "-X",
            "subtree=apps/a",
            "app-a/main",
        ],
    );
    assert_eq!(
        fs::read_to_string(t.down.join("apps/a/app-a.txt")).unwrap(),
        "own\n2\n"
    );

    let status = env.tool_json(&t.down, &["status", "--json"], 0);
    let a = &status["upstreams"][0];
    assert_eq!(a["state"], "current", "{a}");
    assert_eq!(a["last_synced"], a2.as_str());
    assert_eq!(a["last_synced_source"], "history");

    env.push_upstream(&t.a, &[("app-a.txt", "own\n2\n3\n")], "app-a: 3");
    env.tool(&t.down, &["fetch"]).success();
    let status = env.tool_json(&t.down, &["status", "--json"], 0);
    assert_eq!(status["upstreams"][0]["state"], "behind");
    assert_eq!(status["upstreams"][0]["commits_behind"], 1);
    assert_eq!(status["upstreams"][0]["last_synced"], a2.as_str());
}
