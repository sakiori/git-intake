//! The environment `verify` gives the install command and the checks: the
//! update's references, as full commit ids that stay in the repository while
//! verification runs.

mod common;

use std::fs;

use common::{FORK_CHECK, FORK_FILE, Fork, Mono, checks_line};

/// A check that writes the tool's variables, sorted, to `$HOME/<file>`. The
/// variables are read by name, so a multi-line value of another variable
/// cannot add lines.
fn dump(file: &str) -> String {
    format!(
        "awk 'BEGIN {{ for (k in ENVIRON) if (k ~ /^GIT_INTAKE_/ && k !~ /^GIT_INTAKE_TEST_/) print k \"=\" ENVIRON[k] }}' \
         | LC_ALL=C sort > \"$HOME/{file}\""
    )
}

/// A check that fails unless every commit the environment names exists and
/// is reachable from a reference.
const REACHABLE: &str = "for v in $(awk 'BEGIN { for (k in ENVIRON) \
     if (k ~ /^GIT_INTAKE_(BASE|HEAD|UPSTREAM_.*_(TARGET|PREVIOUS))$/) print ENVIRON[k] }'); do \
     git cat-file -e \"$v^{commit}\" || exit 1; \
     test -n \"$(git for-each-ref --contains \"$v\")\" || exit 1; \
     done";

fn dumped(env: &common::Env, file: &str) -> Vec<String> {
    fs::read_to_string(env.path("home").join(file))
        .unwrap_or_else(|e| panic!("no {file}: {e}"))
        .lines()
        .map(str::to_string)
        .collect()
}

fn sorted(mut lines: Vec<String>) -> Vec<String> {
    lines.sort();
    lines
}

#[test]
fn follow_upstream_names_the_base_head_target_and_previous_commit() {
    let f = Fork::with_checks(&[("env", &dump("check-env")), ("reachable", REACHABLE)]);
    let env = &f.env;
    f.reconfigure(
        "install = \"true\"",
        &format!("install = {:?}", dump("install-env")),
    );
    let initial = env.rev(&f.up.bare, "main");

    // First update: the fork shares the upstream's first commit.
    let t1 = env.push_upstream(&f.up, &[("new.txt", "x\n")], "upstream: new");
    let base = env.rev(&f.down, "main");
    f.tool(&["start"]).success();
    let head = env.rev(&env.update_worktree(&f.down), "HEAD");
    // Values inherited from the caller's environment are not passed on.
    env.tool_env(
        &f.down,
        &["verify"],
        &[
            ("GIT_INTAKE_BASE", "inherited"),
            ("GIT_INTAKE_STALE", "x"),
            // A value whose second line looks like one of the variables, as
            // a commit message a CI system exports can.
            (
                "CI_COMMIT_MESSAGE",
                "fix: something\nGIT_INTAKE_UPSTREAMS=not-an-upstream\nGIT_INTAKE_HEAD=0000",
            ),
        ],
    )
    .success();
    let expected = sorted(vec![
        format!("GIT_INTAKE_BASE={base}"),
        format!("GIT_INTAKE_HEAD={head}"),
        "GIT_INTAKE_INTEGRATION=main".to_string(),
        "GIT_INTAKE_UPSTREAMS=upstream".to_string(),
        "GIT_INTAKE_UPSTREAM_UPSTREAM_MODE=follow".to_string(),
        format!("GIT_INTAKE_UPSTREAM_UPSTREAM_PREVIOUS={initial}"),
        format!("GIT_INTAKE_UPSTREAM_UPSTREAM_TARGET={t1}"),
    ]);
    assert_eq!(dumped(env, "check-env"), expected);
    assert_eq!(
        dumped(env, "install-env"),
        expected,
        "install gets the same"
    );
    f.tool(&["finish"]).success();

    // Second update: the previous commit is the last target.
    let t2 = env.push_upstream(&f.up, &[(FORK_FILE, "one\n")], "upstream: shorter");
    let base = env.rev(&f.down, "main");
    f.tool(&["start"]).success();
    let head = env.rev(&env.update_worktree(&f.down), "HEAD");
    f.tool(&["verify"]).success();
    assert_eq!(
        dumped(env, "check-env"),
        sorted(vec![
            format!("GIT_INTAKE_BASE={base}"),
            format!("GIT_INTAKE_HEAD={head}"),
            "GIT_INTAKE_INTEGRATION=main".to_string(),
            "GIT_INTAKE_UPSTREAMS=upstream".to_string(),
            "GIT_INTAKE_UPSTREAM_UPSTREAM_MODE=follow".to_string(),
            format!("GIT_INTAKE_UPSTREAM_UPSTREAM_PREVIOUS={t1}"),
            format!("GIT_INTAKE_UPSTREAM_UPSTREAM_TARGET={t2}"),
        ])
    );
}

#[test]
fn follow_target_named_by_a_revision_is_not_the_fetched_tip() {
    let f = Fork::with_checks(&[("env", &dump("check-env")), ("reachable", REACHABLE)]);
    let env = &f.env;
    let initial = env.rev(&f.up.bare, "main");
    let t1 = env.push_upstream(&f.up, &[("one.txt", "1\n")], "upstream: one");
    let tip = env.push_upstream(&f.up, &[("two.txt", "2\n")], "upstream: two");
    let base = env.rev(&f.down, "main");
    f.tool(&["start", &format!("upstream={t1}")]).success();
    let head = env.rev(&env.update_worktree(&f.down), "HEAD");
    f.tool(&["verify"]).success();
    assert_eq!(
        dumped(env, "check-env"),
        sorted(vec![
            format!("GIT_INTAKE_BASE={base}"),
            format!("GIT_INTAKE_HEAD={head}"),
            "GIT_INTAKE_INTEGRATION=main".to_string(),
            "GIT_INTAKE_UPSTREAMS=upstream".to_string(),
            "GIT_INTAKE_UPSTREAM_UPSTREAM_MODE=follow".to_string(),
            format!("GIT_INTAKE_UPSTREAM_UPSTREAM_PREVIOUS={initial}"),
            format!("GIT_INTAKE_UPSTREAM_UPSTREAM_TARGET={t1}"),
        ])
    );
    assert!(
        !env.is_ancestor(&f.down, &tip, &head),
        "the tip is not merged"
    );
}

#[test]
fn every_queued_upstream_gets_its_own_variables() {
    let m = Mono::new();
    let env = &m.env;
    m.env.commit(
        &m.down,
        &[(
            ".git-intake.toml",
            &common::mono_config(&m.a, &m.b, "").replace(
                "checks = [",
                &format!(
                    "checks = [{{ name = \"env\", run = {:?} }}, {{ name = \"reachable\", run = {:?} }}, ",
                    dump("check-env"),
                    REACHABLE
                ),
            ),
        )],
        "mono: dump the check environment",
    );

    // A first import has no previous commit.
    let a1 = env.rev(&m.a.bare, "main");
    let b1 = env.rev(&m.b.bare, "main");
    let base = env.rev(&m.down, "main");
    m.tool(&["start"]).success();
    let head = env.rev(&env.update_worktree(&m.down), "HEAD");
    m.tool(&["verify"]).success();
    assert_eq!(
        dumped(env, "check-env"),
        sorted(vec![
            format!("GIT_INTAKE_BASE={base}"),
            format!("GIT_INTAKE_HEAD={head}"),
            "GIT_INTAKE_INTEGRATION=main".to_string(),
            "GIT_INTAKE_UPSTREAMS=app-a app-b".to_string(),
            "GIT_INTAKE_UPSTREAM_APP_A_MODE=follow".to_string(),
            format!("GIT_INTAKE_UPSTREAM_APP_A_TARGET={a1}"),
            "GIT_INTAKE_UPSTREAM_APP_B_MODE=follow".to_string(),
            format!("GIT_INTAKE_UPSTREAM_APP_B_TARGET={b1}"),
        ])
    );
    m.tool(&["finish"]).success();

    // Only the upstreams the update takes are named, in queue order.
    let b2 = env.push_upstream(&m.b, &[("b.txt", "b1\n")], "app-b: shorter");
    let base = env.rev(&m.down, "main");
    m.tool(&["start"]).success();
    let head = env.rev(&env.update_worktree(&m.down), "HEAD");
    m.tool(&["verify"]).success();
    assert_eq!(
        dumped(env, "check-env"),
        sorted(vec![
            format!("GIT_INTAKE_BASE={base}"),
            format!("GIT_INTAKE_HEAD={head}"),
            "GIT_INTAKE_INTEGRATION=main".to_string(),
            "GIT_INTAKE_UPSTREAMS=app-b".to_string(),
            "GIT_INTAKE_UPSTREAM_APP_B_MODE=follow".to_string(),
            format!("GIT_INTAKE_UPSTREAM_APP_B_PREVIOUS={b1}"),
            format!("GIT_INTAKE_UPSTREAM_APP_B_TARGET={b2}"),
        ])
    );
}

#[test]
fn port_upstream_names_the_newest_commit_it_takes_or_decides() {
    let f = Fork::port();
    let env = &f.env;
    f.reconfigure(
        &checks_line(&[FORK_CHECK]),
        &checks_line(&[("env", &dump("check-env")), ("reachable", REACHABLE)]),
    );
    let c: Vec<String> = (1..=3)
        .map(|i| {
            env.push_upstream(
                &f.up,
                &[(&format!("p{i}.txt"), &format!("{i}\n"))],
                &format!("upstream: change {i}"),
            )
        })
        .collect();
    let base = env.rev(&f.down, "main");
    f.tool(&["start"]).success();
    let plan = env.intake_dir(&f.down).join("plan");
    fs::write(
        &plan,
        format!(
            "take {}\nskip {} # not here\nlater {}\n",
            &c[0][..12],
            &c[1][..12],
            &c[2][..12]
        ),
    )
    .unwrap();
    f.tool(&["continue"]).success();
    let head = env.rev(&env.update_worktree(&f.down), "HEAD");
    f.tool(&["verify"]).success();
    assert_eq!(
        dumped(env, "check-env"),
        sorted(vec![
            format!("GIT_INTAKE_BASE={base}"),
            format!("GIT_INTAKE_HEAD={head}"),
            "GIT_INTAKE_INTEGRATION=main".to_string(),
            "GIT_INTAKE_UPSTREAMS=upstream".to_string(),
            "GIT_INTAKE_UPSTREAM_UPSTREAM_MODE=port".to_string(),
            // The commit left `later` is not what the update moves the
            // upstream to.
            format!("GIT_INTAKE_UPSTREAM_UPSTREAM_TARGET={}", c[1]),
        ])
    );
}

#[test]
fn upstream_names_that_share_variables_are_refused() {
    let m = Mono::new();
    m.env.commit(
        &m.down,
        &[(
            ".git-intake.toml",
            &common::mono_config(&m.a, &m.b, "").replace("name = \"app-b\"", "name = \"APP.a\""),
        )],
        "mono: clashing names",
    );
    let clash =
        "upstreams 'app-a' and 'APP.a' share the environment variables GIT_INTAKE_UPSTREAM_APP_A_*";
    m.tool(&["start"])
        .code(2)
        .stderr(predicates::str::contains(clash));
    m.tool(&["status"])
        .code(2)
        .stderr(predicates::str::contains(clash));
    m.tool(&["doctor"])
        .code(1)
        .stdout(predicates::str::contains(clash));
    m.env.assert_no_update(&m.down);
}

#[test]
fn a_catch_up_names_the_tip_it_caught_up_with_as_the_base() {
    let f = Fork::with_checks(&[("env", &dump("check-env")), ("reachable", REACHABLE)]);
    let env = &f.env;
    env.push_upstream(&f.up, &[("new.txt", "x\n")], "upstream: new");
    f.tool(&["start"]).success();
    f.tool(&["verify"]).success();
    let before = dumped(env, "check-env");
    let local = env.commit(&f.down, &[("local.txt", "l\n")], "fork: local work");

    f.tool(&["finish"]).success();
    let merge = env.rev(&f.down, "main");
    let after = dumped(env, "check-env");
    assert!(
        after.contains(&format!("GIT_INTAKE_BASE={local}")),
        "{after:?}"
    );
    assert!(
        after.contains(&format!("GIT_INTAKE_HEAD={merge}")),
        "{after:?}"
    );
    // Every other variable is as before the catch-up.
    let rest = |lines: &[String]| -> Vec<String> {
        lines
            .iter()
            .filter(|l| !l.starts_with("GIT_INTAKE_BASE=") && !l.starts_with("GIT_INTAKE_HEAD="))
            .cloned()
            .collect()
    };
    assert_eq!(rest(&after), rest(&before));
}
