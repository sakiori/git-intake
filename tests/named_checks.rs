//! Named checks: the configuration's table form, `verify --only` and
//! `--skip`, partial runs beside the verification, and their logs.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::{Fork, checks_line, stderr, stdout};
use serde_json::json;

/// A check that records that it ran in `$HOME/ran` and fails while
/// `$HOME/fail-<name>` exists.
fn marked(name: &str) -> String {
    format!("echo {name} >> \"$HOME/ran\"; test ! -f \"$HOME/fail-{name}\"")
}

/// A fork with the checks `lint` and `test` and an install command, each
/// recording that it ran, and an update in progress whose upstreams are
/// merged.
fn started() -> Fork {
    let f = Fork::with_checks(&[("lint", &marked("lint")), ("test", &marked("test"))]);
    f.reconfigure(
        "install = \"true\"",
        "install = \"echo install >> \\\"$HOME/ran\\\"\"",
    );
    f.env
        .push_upstream(&f.up, &[("n.txt", "n\n")], "upstream: n");
    f.tool(&["start"]).success();
    f
}

/// What ran since the last call, in order.
fn ran(f: &Fork) -> Vec<String> {
    let path = f.env.path("home").join("ran");
    let text = fs::read_to_string(&path).unwrap_or_default();
    let _ = fs::remove_file(&path);
    text.lines().map(str::to_string).collect()
}

fn fail(f: &Fork, name: &str) {
    fs::write(f.env.path("home").join(format!("fail-{name}")), "").unwrap();
}

fn head(f: &Fork) -> String {
    f.env.rev(&f.env.update_worktree(&f.down), "HEAD")
}

fn logs(f: &Fork, run: &str) -> PathBuf {
    f.env.intake_dir(&f.down).join("logs").join(run)
}

fn update(f: &Fork) -> serde_json::Value {
    f.env.tool_json(&f.down, &["status", "--json"], 0)["update"].clone()
}

fn state_text(f: &Fork) -> String {
    fs::read_to_string(f.env.intake_dir(&f.down).join("state.json")).unwrap()
}

/// Other work lands on the publish remote's integration branch.
fn land(f: &Fork, edit: impl FnOnce(&Path)) -> String {
    let env = &f.env;
    let dir = env.path("other");
    env.git(
        &env.path(""),
        &[
            "clone",
            "-q",
            &f.publish.to_string_lossy(),
            &dir.to_string_lossy(),
        ],
    );
    edit(&dir);
    env.git(&dir, &["add", "-A"]);
    env.git(&dir, &["commit", "-q", "-m", "other: work"]);
    env.git(&dir, &["push", "-q", "origin", "HEAD:main"]);
    env.rev(&dir, "HEAD")
}

#[test]
fn checks_as_plain_commands_are_refused_with_the_table_form() {
    let f = Fork::new();
    f.plain_checks();
    for args in [&["status"][..], &["start"], &["doctor"], &["sync"]] {
        let a = f.tool(args);
        let out = format!("{}{}", stdout(&a), stderr(&a));
        assert!(
            out.contains(
                "checks: each check is a table with a name and the command it runs, not a plain command; \
                 write `checks = [{ name = \"...\", run = \"test -f app.txt\" }]`, naming each check"
            ),
            "{args:?}: {out}"
        );
    }
}

#[test]
fn duplicate_names_and_unknown_keys_are_refused() {
    let f = Fork::new();
    f.reconfigure(
        &checks_line(&[common::FORK_CHECK]),
        "checks = [{ name = \"test\", run = \"true\" }, { name = \"Test\", run = \"true\" }, \
         { name = \"lint\", cmd = \"make lint\" }]",
    );
    let err = stderr(&f.tool(&["status"]).code(2));
    assert!(
        err.contains("checks 'test' and 'Test' share a name"),
        "{err}"
    );
    assert!(err.contains("check 'lint': unknown key `cmd`"), "{err}");
}

#[test]
fn only_one_check_runs_with_the_install_command_and_does_not_verify() {
    let f = started();
    let env = &f.env;
    let head = head(&f);
    let main = env.rev(&f.down, "main");
    let published = env.rev(&f.publish, "main");

    let out = stdout(&f.tool(&["verify", "--only", "test"]).success());
    assert_eq!(ran(&f), ["install", "test"]);
    assert!(out.contains("passed   install command"), "{out}");
    assert!(out.contains("passed   test"), "{out}");
    assert!(!out.contains("lint  ("), "{out}");
    assert!(out.contains("did not verify the update"), "{out}");
    assert!(out.contains("the update is not verified"), "{out}");
    assert!(out.contains("`git intake verify`"), "{out}");

    // Its logs are its own, named after the commands.
    assert!(logs(&f, "partial").join("install.log").exists());
    assert!(logs(&f, "partial").join("check-test.log").exists());
    assert!(!logs(&f, "partial").join("check-lint.log").exists());
    assert!(!logs(&f, "verify").exists());

    let update = update(&f);
    assert_eq!(update["checks"], json!(["lint", "test"]));
    assert_eq!(update["verified_commit"], serde_json::Value::Null);
    assert_eq!(
        update["partial_run"],
        json!({
            "commit": head,
            "install": "passed",
            "checks": [{"name": "test", "outcome": "passed"}],
            "left_out": ["lint"],
            "passed": true,
        })
    );

    // A head with only a partial run is not published.
    let err = stderr(&f.tool(&["finish"]).code(2));
    assert!(
        err.contains("has only a partial run, which left out 'lint'"),
        "{err}"
    );
    assert!(err.contains("run `git intake verify`"), "{err}");
    assert_eq!(env.rev(&f.down, "main"), main);
    assert_eq!(env.rev(&f.publish, "main"), published);
    assert!(ran(&f).is_empty());
}

#[test]
fn a_failed_partial_run_after_a_full_verification_keeps_it() {
    let f = started();
    let env = &f.env;
    let head = head(&f);
    f.tool(&["verify"]).success();
    assert_eq!(ran(&f), ["install", "lint", "test"]);
    let full_log = logs(&f, "verify").join("check-test.log");
    assert!(full_log.exists());

    fail(&f, "lint");
    let out = stdout(&f.tool(&["verify", "--only", "lint"]).code(1));
    assert_eq!(ran(&f), ["install", "lint"]);
    assert!(out.contains("FAILED   lint"), "{out}");
    assert!(out.contains("did not verify the update"), "{out}");
    assert!(
        out.contains(&format!("the update stays verified at {}", &head[..12])),
        "{out}"
    );
    // The logs of the full verification are kept; the partial run writes
    // its own apart from them.
    assert!(full_log.exists());
    assert!(logs(&f, "verify").join("check-lint.log").exists());
    assert!(logs(&f, "partial").join("check-lint.log").exists());

    let text = stdout(&f.tool(&["status"]).success());
    assert!(
        text.contains(&format!("  verified: {}", &head[..12])),
        "{text}"
    );
    assert!(
        text.contains(&format!(
            "  partial run at {}, which does not verify the update: failed; install command passed, lint failed; \
             left out: test",
            &head[..12]
        )),
        "{text}"
    );
    assert_eq!(update(&f)["verified_commit"], head.as_str());

    // The update stays verified, so `finish` publishes it.
    f.tool(&["finish"]).success();
    assert_eq!(env.rev(&f.publish, "main"), head);
    env.assert_no_update(&f.down);
    assert!(!env.intake_dir(&f.down).join("logs").exists());
    assert!(
        ran(&f).is_empty(),
        "finish runs no check on a verified head"
    );
}

#[test]
fn a_passing_partial_run_on_a_verified_head_is_shown_in_json() {
    let f = started();
    let head = head(&f);
    f.tool(&["verify"]).success();
    ran(&f);
    let out = stdout(&f.tool(&["verify", "--skip", "test"]).success());
    assert_eq!(ran(&f), ["install", "lint"]);
    assert!(
        out.contains("this partial run passed and did not verify the update"),
        "{out}"
    );
    assert!(
        out.contains(&format!("the update stays verified at {}", &head[..12])),
        "{out}"
    );
    let update = update(&f);
    assert_eq!(update["verified_commit"], head.as_str());
    assert_eq!(
        update["partial_run"],
        json!({
            "commit": head,
            "install": "passed",
            "checks": [{"name": "lint", "outcome": "passed"}],
            "left_out": ["test"],
            "passed": true,
        })
    );
}

#[test]
fn partial_runs_do_not_add_up() {
    let f = started();
    f.tool(&["verify", "--only", "lint"]).success();
    assert!(logs(&f, "partial").join("check-lint.log").exists());
    f.tool(&["verify", "--only", "test"]).success();
    assert_eq!(ran(&f), ["install", "lint", "install", "test"]);
    // The later run replaced the earlier one with its logs.
    assert!(!logs(&f, "partial").join("check-lint.log").exists());
    let update = update(&f);
    assert_eq!(update["verified_commit"], serde_json::Value::Null);
    assert_eq!(update["step"], "merged");
    assert_eq!(
        update["partial_run"]["checks"],
        json!([{"name": "test", "outcome": "passed"}])
    );
    assert_eq!(update["partial_run"]["left_out"], json!(["lint"]));
    let err = stderr(&f.tool(&["finish"]).code(2));
    assert!(err.contains("left out 'lint'"), "{err}");
}

#[test]
fn a_failed_partial_run_on_an_unverified_head_in_status() {
    let f = started();
    let head = head(&f);
    fail(&f, "lint");
    let out = stdout(&f.tool(&["verify", "--only", "lint"]).code(1));
    assert!(out.contains("the update is not verified"), "{out}");
    let text = stdout(&f.tool(&["status"]).success());
    assert!(text.contains("checks:\n  lint: "), "{text}");
    assert!(text.contains("\n  test: "), "{text}");
    assert!(text.contains("  checks: lint, test\n"), "{text}");
    assert!(
        text.contains(&format!(
            "  partial run at {}, which does not verify the update: failed; install command passed, lint failed; \
             left out: test",
            &head[..12]
        )),
        "{text}"
    );
    assert!(!text.contains("  verified:"), "{text}");
}

#[test]
fn a_selection_of_every_check_is_a_full_run() {
    let f = started();
    let head = head(&f);
    f.tool(&["verify", "--only", "lint"]).success();
    assert!(logs(&f, "partial").exists());
    let out = stdout(
        &f.tool(&["verify", "--only", "test", "--only", "lint"])
            .success(),
    );
    assert!(out.contains(&format!("verified {}", &head[..12])), "{out}");
    assert_eq!(ran(&f), ["install", "lint", "install", "lint", "test"]);
    let update = update(&f);
    assert_eq!(update["verified_commit"], head.as_str());
    assert_eq!(update["partial_run"], serde_json::Value::Null);
    assert!(!logs(&f, "partial").exists());
}

#[test]
fn a_name_given_twice_counts_once_and_skipping_every_check_runs_the_install_command() {
    let f = started();
    f.tool(&["verify", "--only", "lint", "--only", "lint"])
        .success();
    assert_eq!(ran(&f), ["install", "lint"]);
    assert_eq!(update(&f)["partial_run"]["left_out"], json!(["test"]));

    f.tool(&["verify", "--skip", "lint", "--skip", "test"])
        .success();
    assert_eq!(ran(&f), ["install"]);
    let update = update(&f);
    assert_eq!(update["partial_run"]["checks"], json!([]));
    assert_eq!(update["partial_run"]["left_out"], json!(["lint", "test"]));
    assert_eq!(update["verified_commit"], serde_json::Value::Null);
}

#[test]
fn unknown_names_and_both_selections_are_refused_running_nothing() {
    let f = started();
    let before = state_text(&f);
    let err = stderr(&f.tool(&["verify", "--skip", "tests"]).code(2));
    assert!(
        err.contains("unknown check 'tests'; the update's checks are 'lint', 'test'"),
        "{err}"
    );
    let err = stderr(
        &f.tool(&["verify", "--only", "lint", "--skip", "test"])
            .code(2),
    );
    assert!(err.contains("--skip"), "{err}");
    assert!(ran(&f).is_empty());
    assert_eq!(state_text(&f), before);
    assert!(!logs(&f, "partial").exists());
}

#[test]
fn a_check_named_install_has_a_log_of_its_own() {
    let f = Fork::with_checks(&[("install", "echo check")]);
    f.reconfigure("install = \"true\"", "install = \"echo install\"");
    f.env
        .push_upstream(&f.up, &[("n.txt", "n\n")], "upstream: n");
    f.tool(&["start"]).success();
    let out = stdout(&f.tool(&["verify"]).success());
    assert!(out.contains("passed   install command"), "{out}");
    assert!(out.contains("passed   install  (log:"), "{out}");
    let dir = logs(&f, "verify");
    assert_eq!(
        fs::read_to_string(dir.join("install.log")).unwrap(),
        "install\n"
    );
    assert_eq!(
        fs::read_to_string(dir.join("check-install.log")).unwrap(),
        "check\n"
    );
}

#[test]
fn a_check_added_by_a_catch_up_is_selected_and_finish_runs_every_check() {
    let f = started();
    let env = &f.env;
    f.tool(&["verify"]).success();
    f.tool(&["verify", "--only", "lint"]).success();
    ran(&f);
    // The integration branch adds the check `audit` and a commit that
    // conflicts with the update.
    land(&f, |dir| {
        let path = dir.join(".git-intake.toml");
        let cfg = fs::read_to_string(&path).unwrap().replace(
            "}]",
            &format!("}}, {{ name = \"audit\", run = {:?} }}]", marked("audit")),
        );
        fs::write(path, cfg).unwrap();
        fs::write(dir.join("n.txt"), "theirs\n").unwrap();
    });
    f.tool(&["finish"]).code(1);
    let wt = env.update_worktree(&f.down);
    fs::write(wt.join("n.txt"), "both\n").unwrap();
    env.git(&wt, &["add", "n.txt"]);
    f.tool(&["continue"]).success();
    // The catch-up merge dropped the partial run with its logs.
    let caught = update(&f);
    assert_eq!(caught["partial_run"], serde_json::Value::Null);
    assert_eq!(caught["checks"], json!(["lint", "test", "audit"]));
    assert!(!logs(&f, "partial").exists());

    let merge = head(&f);
    f.tool(&["verify", "--only", "audit"]).success();
    assert_eq!(ran(&f), ["install", "audit"]);
    f.tool(&["verify", "--only", "lint", "--only", "test"])
        .success();
    assert_eq!(ran(&f), ["install", "lint", "test"]);
    assert_eq!(update(&f)["partial_run"]["left_out"], json!(["audit"]));

    // `finish` verifies the catch-up merge it made itself, running every
    // check, and publishes it.
    let out = stdout(&f.tool(&["finish"]).success());
    assert_eq!(ran(&f), ["install", "lint", "test", "audit"]);
    assert!(out.contains(&format!("verified {}", &merge[..12])), "{out}");
    assert_eq!(env.rev(&f.publish, "main"), merge);
    env.assert_no_update(&f.down);
}

#[test]
fn a_failed_full_run_is_reported_beside_a_partial_run_on_the_same_head() {
    let f = started();
    fail(&f, "test");
    f.tool(&["verify"]).code(1);
    f.tool(&["verify", "--only", "lint"]).success();
    let err = stderr(&f.tool(&["finish"]).code(2));
    assert!(
        err.contains(&format!("verification of {} failed", &head(&f)[..12])),
        "{err}"
    );
    assert!(!err.contains("only a partial run"), "{err}");
}

#[test]
fn a_refused_selection_writes_no_warning_and_restores_no_reference() {
    let f = started();
    let env = &f.env;
    // Another release started the update, and its upstream reference is
    // missing: carrying the update on would warn and restore it.
    let path = env.intake_dir(&f.down).join("state.json");
    let mut state: serde_json::Value = serde_json::from_str(&state_text(&f)).unwrap();
    state["release"] = "0.0.1".into();
    fs::write(&path, serde_json::to_string_pretty(&state).unwrap()).unwrap();
    env.git(
        &f.down,
        &["update-ref", "-d", "refs/git-intake/upstreams/upstream"],
    );
    let before = state_text(&f);

    let err = stderr(&f.tool(&["verify", "--only", "tests"]).code(2));
    assert!(err.contains("unknown check 'tests'"), "{err}");
    assert!(!err.contains("warning:"), "{err}");
    assert!(!err.contains("restored"), "{err}");
    assert_eq!(state_text(&f), before);
    assert!(env.intake_refs(&f.down).is_empty());
    assert!(ran(&f).is_empty());

    // A valid selection carries the update on as usual.
    let err = stderr(&f.tool(&["verify", "--only", "lint"]).success());
    assert!(
        err.contains("warning: the update was started by git-intake 0.0.1"),
        "{err}"
    );
    assert!(err.contains("restored the update's reference"), "{err}");
}
