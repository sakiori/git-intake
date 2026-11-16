//! An upstream taken in port mode: a plan of its pending commits, copies of
//! the commits taken, and a journal of the others.

mod common;

use std::fs;

use common::{FORK_FILE, Fork, How, merge_on_fake, open_number, stderr, stdout};
use predicates::str::contains;

const UPSTREAM_AUTHOR: &str = "Upstream Dev <dev@upstream.example>";
const JOURNAL: &str = ".git-intake/upstream.toml";

/// Pushes `n` upstream commits, each adding its own file, by an author of
/// the upstream's; returns them oldest first.
fn upstream_commits(f: &Fork, n: usize) -> Vec<String> {
    (1..=n)
        .map(|i| {
            upstream_change(
                f,
                &format!("p{i}.txt"),
                &format!("{i}\n"),
                &format!("upstream: change {i}"),
            )
        })
        .collect()
}

/// Pushes one upstream commit that writes `file`.
fn upstream_change(f: &Fork, file: &str, content: &str, message: &str) -> String {
    let env = &f.env;
    let path = f.up.work.join(file);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
    env.git(&f.up.work, &["add", "-A"]);
    env.git(
        &f.up.work,
        &["commit", "-q", "--author", UPSTREAM_AUTHOR, "-m", message],
    );
    env.git(&f.up.work, &["push", "-q", "origin", "HEAD:main"]);
    env.rev(&f.up.work, "HEAD")
}

fn plan_path(f: &Fork) -> std::path::PathBuf {
    f.env.intake_dir(&f.down).join("plan")
}

/// Writes the plan: one `verb commit` line per entry, with ` # note` when a
/// note is given.
fn write_plan(f: &Fork, lines: &[(&str, &str, &str)]) {
    let mut text = String::from("# edited\n");
    for (verb, commit, note) in lines {
        text.push_str(&format!("{verb} {}", &commit[..12]));
        if !note.is_empty() {
            text.push_str(&format!(" # {note}"));
        }
        text.push('\n');
    }
    fs::write(plan_path(f), text).unwrap();
}

/// The journal at `rev` of the publish remote, where the integration
/// branch is.
fn journal_at(f: &Fork, rev: &str) -> String {
    f.env
        .git(&f.publish, &["show", &format!("{rev}:{JOURNAL}")])
}

/// The upstream commit a copy names as taken.
fn taken_by(f: &Fork, rev: &str) -> String {
    f.env.git(
        &f.down,
        &[
            "log",
            "-1",
            "--format=%(trailers:key=Intake-Taken,valueonly)",
            rev,
        ],
    )
}

fn port_status(f: &Fork, code: i32) -> serde_json::Value {
    let status = f.env.tool_json(&f.down, &["status", "--json"], code);
    status["upstreams"][0].clone()
}

#[test]
fn plan_is_written_edited_and_carried_out() {
    let f = Fork::port();
    let env = &f.env;
    let initial = env.rev(&f.up.bare, "main");
    let c = upstream_commits(&f, 4);
    let base = env.rev(&f.down, "main");

    // Before any update the port has no journal; its base would be the last
    // synced commit the history gives.
    let up = port_status(&f, 0);
    assert_eq!(up["mode"], "port");
    assert_eq!(up["state"], "pending");
    assert_eq!(up["base"], initial.as_str());
    assert_eq!(up["journal"], false);
    assert_eq!(up["pending"], 4);
    assert_eq!(up["taken"], 0);
    assert_eq!(up["last_synced"], serde_json::Value::Null);
    f.tool(&["status", "--check"]).code(1);

    let start = stdout(&f.tool(&["start"]).success());
    assert!(
        start.contains("upstream: 4 pending commits planned"),
        "{start}"
    );
    assert!(start.contains("upstream: the plan is in "), "{start}");
    assert!(start.contains("/git-intake/plan\n"), "{start}");
    let plan = fs::read_to_string(plan_path(&f)).unwrap();
    assert!(
        plan.starts_with("# git intake plan for upstream: 4 pending commits, oldest first.\n"),
        "{plan}"
    );
    let listed: Vec<&str> = plan.lines().filter(|l| !l.starts_with('#')).collect();
    assert_eq!(
        listed,
        (0..4)
            .map(|i| format!("later {} upstream: change {}", &c[i][..12], i + 1))
            .collect::<Vec<_>>()
    );
    let wt = env.update_worktree(&f.down);
    assert_eq!(
        env.rev(&wt, "HEAD"),
        base,
        "nothing is copied before the plan is read"
    );

    let status = env.tool_json(&f.down, &["status", "--json"], 0);
    let update = &status["update"];
    assert_eq!(update["step"], "plan");
    assert_eq!(update["upstreams"][0]["mode"], "port");
    assert_eq!(update["upstreams"][0]["step"], "plan");
    let plan_shown = update["upstreams"][0]["plan"].as_str().unwrap();
    assert!(plan_shown.ends_with("/git-intake/plan"), "{plan_shown}");
    let next = update["next"].as_str().unwrap();
    assert!(next.contains(plan_shown), "{next}");
    assert!(next.contains("git intake continue"), "{next}");
    f.tool(&["verify"])
        .code(2)
        .stderr(contains("upstream (at its plan)"));

    // Two takes, a skip with its note, and one commit left for later.
    write_plan(
        &f,
        &[
            ("take", &c[0], ""),
            ("skip", &c[1], "not for this product"),
            ("take", &c[2], ""),
            ("later", &c[3], ""),
        ],
    );
    let out = stdout(&f.tool(&["continue"]).success());
    assert!(
        out.contains(
            "upstream: the plan takes 2, skips 1, records 0 as present and leaves 1 pending"
        ),
        "{out}"
    );
    assert!(out.contains("next: git intake verify"), "{out}");

    // Two copies, then the journal commit.
    let head = env.rev(&wt, "HEAD");
    assert_eq!(env.rev(&wt, "HEAD~3"), base);
    assert_eq!(taken_by(&f, &env.rev(&wt, "HEAD~2")), c[0]);
    assert_eq!(taken_by(&f, &env.rev(&wt, "HEAD~1")), c[2]);
    let copy = env.git(&wt, &["log", "-1", "--format=%an <%ae>%n%B", "HEAD~1"]);
    assert!(
        copy.starts_with(&format!("{UPSTREAM_AUTHOR}\nupstream: change 3\n")),
        "{copy}"
    );
    assert!(copy.contains("Intake-Upstream: upstream\n"), "{copy}");
    assert!(copy.contains(&format!("Intake-Taken: {}", c[2])), "{copy}");
    assert!(wt.join("p1.txt").exists() && wt.join("p3.txt").exists());
    assert!(!wt.join("p2.txt").exists() && !wt.join("p4.txt").exists());
    let changed = env.git(&wt, &["show", "--name-only", "--format=", "HEAD"]);
    assert_eq!(changed, JOURNAL, "the last commit changes only the journal");
    let journal = fs::read_to_string(wt.join(JOURNAL)).unwrap();
    assert!(
        journal.contains(&format!("base = \"{initial}\"")),
        "{journal}"
    );
    assert!(
        journal.contains(&format!(
            "[[decision]]\ncommit = \"{}\"\nverdict = \"skip\"\nnote = \"not for this product\"\n",
            c[1]
        )),
        "{journal}"
    );
    assert!(!journal.contains(&c[3]), "{journal}");

    let report = env.tool_json(&f.down, &["report", "--json"], 0);
    let port = &report["upstreams"][0]["port"];
    assert_eq!(report["upstreams"][0]["mode"], "port");
    assert_eq!(port["taken"][0]["commit"], c[0].as_str());
    assert_eq!(port["taken"][1]["commit"], c[2].as_str());
    assert_eq!(port["decisions"][0]["verdict"], "skip");
    assert_eq!(port["decisions"][0]["note"], "not for this product");
    assert_eq!(port["left_pending"], 1);

    f.tool(&["verify"]).success();
    f.tool(&["finish"]).success();
    env.assert_no_update(&f.down);
    assert_eq!(env.rev(&f.down, "main"), base, "the local `main` stays");
    assert_eq!(env.rev(&f.publish, "main"), head);
    assert_eq!(journal_at(&f, "main"), journal.trim_end());

    // Taken, decided and pending as the history and the journal say.
    let up = port_status(&f, 0);
    assert_eq!(up["journal"], true);
    assert_eq!(up["base"], initial.as_str());
    assert_eq!(up["taken"], 2);
    assert_eq!(up["decided"], 1);
    assert_eq!(up["pending"], 1);
    assert_eq!(up["state"], "pending");
    let text = stdout(&f.tool(&["status"]).success());
    assert!(
        text.contains(&format!(
            "upstream: base {}, 2 taken, 1 decided, 1 pending",
            &initial[..12]
        )),
        "{text}"
    );
    let check = env.tool_json(&f.down, &["status", "--check", "--json"], 1);
    assert_eq!(check["all_current"], false);
    let log = env.tool_json(&f.down, &["log", "--json"], 0);
    let commits = log["upstreams"][0]["commits"].as_array().unwrap();
    assert_eq!(commits.len(), 1);
    assert_eq!(commits[0]["commit"], c[3].as_str());
    assert!(
        stdout(&f.tool(&["log"]).success())
            .contains("upstream (/, upstream/main): 1 pending commit, oldest first"),
    );
    // Every branch of the upstream holds what the integration branch lacks,
    // except the commits copies took.
    let branches = env.tool_json(&f.down, &["branches", "--json"], 0);
    assert_eq!(
        branches["upstreams"][0]["branches"][0]["unmerged_commits"],
        2
    );
    env.assert_no_update(&f.down);

    // The last pending commit taken, nothing is left.
    f.tool(&["start"]).success();
    write_plan(&f, &[("take", &c[3], "")]);
    f.tool(&["continue"]).success();
    // A plan without decisions on an existing journal makes no journal commit.
    let wt_head = env.rev(&wt, "HEAD");
    assert_eq!(taken_by(&f, &wt_head), c[3]);
    f.tool(&["verify"]).success();
    f.tool(&["finish"]).success();
    let up = port_status(&f, 0);
    assert_eq!(up["state"], "current");
    assert_eq!(up["pending"], 0);
    f.tool(&["status", "--check"]).success();
    f.tool(&["start"])
        .success()
        .stdout(contains("upstream: current; every commit after the base"))
        .stdout(contains("nothing to update"));
    env.assert_no_update(&f.down);
}

#[test]
fn takes_are_copied_in_plan_order_and_a_plan_may_only_decide() {
    let f = Fork::port();
    let env = &f.env;
    let c = upstream_commits(&f, 3);
    let base = env.rev(&f.down, "main");
    f.tool(&["start"]).success();
    write_plan(&f, &[("take", &c[2], ""), ("take", &c[0], "")]);
    f.tool(&["continue"]).success();
    let wt = env.update_worktree(&f.down);
    // The newer commit first, then the older one, then the journal that
    // the first plan creates.
    assert_eq!(taken_by(&f, &env.rev(&wt, "HEAD~2")), c[2]);
    assert_eq!(taken_by(&f, &env.rev(&wt, "HEAD~1")), c[0]);
    assert_eq!(env.rev(&wt, "HEAD~3"), base);
    let subject = env.git(&wt, &["log", "-1", "--format=%s"]);
    assert!(
        subject.starts_with("sync(upstream): start the port journal at "),
        "{subject}"
    );
    f.tool(&["abort"]).success();
    env.assert_no_update(&f.down);

    // Only decisions: one journal commit.
    f.tool(&["start"]).success();
    write_plan(
        &f,
        &[
            ("skip", &c[0], "upstream only"),
            ("have", &c[1], "ported earlier"),
            ("later", &c[2], ""),
        ],
    );
    f.tool(&["continue"])
        .success()
        .stdout(contains("next: git intake verify"));
    assert_eq!(env.rev(&wt, "HEAD~1"), base);
    let message = env.git(&wt, &["log", "-1", "--format=%B"]);
    assert!(
        message.starts_with("sync(upstream): record 2 port decisions\n"),
        "{message}"
    );
    assert!(
        message.contains(&format!(
            "skip {} upstream: change 1 # upstream only",
            &c[0][..12]
        )),
        "{message}"
    );
    let journal = fs::read_to_string(wt.join(JOURNAL)).unwrap();
    assert!(
        journal.contains("verdict = \"have\"\nnote = \"ported earlier\""),
        "{journal}"
    );
    f.tool(&["verify"]).success();
    f.tool(&["finish"]).success();
    let up = port_status(&f, 0);
    assert_eq!(
        (
            up["taken"].as_u64(),
            up["decided"].as_u64(),
            up["pending"].as_u64()
        ),
        (Some(0), Some(2), Some(1))
    );
}

#[test]
fn invalid_plans_are_refused_and_change_nothing() {
    let f = Fork::port();
    let env = &f.env;
    let c = upstream_commits(&f, 2);
    let base = env.rev(&f.down, "main");
    f.tool(&["start"]).success();
    let wt = env.update_worktree(&f.down);
    let state_before = fs::read_to_string(env.intake_dir(&f.down).join("state.json")).unwrap();

    let not_pending = env.commit(&f.down, &[], "fork: not upstream");
    env.git(&f.down, &["reset", "-q", "--hard", "HEAD~1"]);
    let cases: Vec<(String, String)> = vec![
        (
            format!("take {}\n", &not_pending[..12]),
            format!(
                "plan line 1 (take {}): '{}' is not one of the pending commits",
                &not_pending[..12],
                &not_pending[..12]
            ),
        ),
        (
            format!("take {}\nskip {}\n", &c[0][..12], &c[0][..8]),
            format!(
                "plan line 2 (skip {}): commit {} is named more than once",
                &c[0][..8],
                &c[0][..12]
            ),
        ),
        (
            format!("pick {}\n", &c[0][..12]),
            "plan line 1".to_string() + &format!(" (pick {}): unknown verb 'pick'", &c[0][..12]),
        ),
        (
            format!("later {}\nlater {}\n", &c[0][..12], &c[1][..12]),
            "the plan decides nothing".to_string(),
        ),
    ];
    for (plan, why) in cases {
        fs::write(plan_path(&f), &plan).unwrap();
        let err = stderr(&f.tool(&["continue"]).code(2));
        assert!(err.contains(&why), "{plan}: {err}");
        assert!(
            err.contains("nothing was changed; correct the plan"),
            "{err}"
        );
        assert_eq!(env.rev(&wt, "HEAD"), base);
        assert_eq!(
            fs::read_to_string(env.intake_dir(&f.down).join("state.json")).unwrap(),
            state_before
        );
        assert_eq!(
            fs::read_to_string(plan_path(&f)).unwrap(),
            plan,
            "the plan is kept"
        );
    }
    let status = env.tool_json(&f.down, &["status", "--json"], 0);
    assert_eq!(status["update"]["step"], "plan");
}

#[test]
fn abort_at_the_plan_step_leaves_nothing() {
    let f = Fork::port();
    let env = &f.env;
    upstream_commits(&f, 2);
    let main = env.rev(&f.down, "main");
    f.tool(&["start"]).success();
    assert!(plan_path(&f).exists());
    assert_eq!(
        env.intake_refs(&f.down),
        vec!["refs/git-intake/upstreams/upstream".to_string()]
    );
    let out = stdout(&f.tool(&["abort"]).success());
    assert!(out.contains("removed plan "), "{out}");
    assert!(
        out.contains("removed reference refs/git-intake/upstreams/upstream"),
        "{out}"
    );
    env.assert_no_update(&f.down);
    assert_eq!(env.rev(&f.down, "main"), main);
    assert!(
        !env.has_rev(&f.down, &format!("main:{JOURNAL}")),
        "the journal is unchanged"
    );
}

#[test]
fn a_conflicted_copy_stops_resumes_and_its_resolution_is_recorded() {
    let f = Fork::port();
    let env = &f.env;
    env.commit(
        &f.down,
        &[(FORK_FILE, "one\nTWO-fork\nthree\n")],
        "fork: change two",
    );
    env.git(&f.down, &["push", "-q", "origin", "main"]);
    let c1 = upstream_change(
        &f,
        FORK_FILE,
        "one\nTWO-upstream\nthree\n",
        "upstream: change two",
    );
    let c2 = upstream_change(&f, "later.txt", "l\n", "upstream: later");

    f.tool(&["start"]).success();
    write_plan(&f, &[("take", &c1, ""), ("take", &c2, "")]);
    let out = stdout(&f.tool(&["continue"]).code(1));
    assert!(
        out.contains(&format!(
            "upstream: conflicts copying {} upstream: change two",
            &c1[..12]
        )),
        "{out}"
    );
    assert!(out.contains("app.txt  (unresolved)"), "{out}");
    assert!(
        out.contains(&format!("still to copy: {}", &c2[..12])),
        "{out}"
    );
    let status = env.tool_json(&f.down, &["status", "--json"], 0);
    assert_eq!(status["update"]["step"], "conflict");
    assert_eq!(
        status["update"]["upstreams"][0]["conflicts"][0]["path"],
        "app.txt"
    );
    f.tool(&["doctor"]).success().stdout(contains(
        "[pass] the update in progress matches its worktree and branch",
    ));

    // The update keeps what it needs: with the upstream gone and the
    // repository garbage-collected, it goes on without fetching.
    let moved = env.path("upstream-away.git");
    fs::rename(&f.up.bare, &moved).unwrap();
    env.git(&f.down, &["gc", "-q", "--prune=now"]);
    assert!(env.has_rev(&f.down, &format!("{c2}^{{commit}}")));
    let wt = env.update_worktree(&f.down);
    f.tool(&["continue"])
        .code(2)
        .stderr(contains("still has unresolved conflicts"));
    let resolved = "one\nTWO-both\nthree\n";
    fs::write(wt.join(FORK_FILE), resolved).unwrap();
    env.git(&wt, &["add", FORK_FILE]);
    let out = stdout(&f.tool(&["continue"]).success());
    assert!(
        out.contains(&format!("upstream: copied {} as", &c1[..12])),
        "{out}"
    );
    assert!(
        out.contains(&format!("upstream: copied {} as", &c2[..12])),
        "{out}"
    );
    assert_eq!(taken_by(&f, &env.rev(&wt, "HEAD~2")), c1);
    assert_eq!(taken_by(&f, &env.rev(&wt, "HEAD~1")), c2);
    assert_eq!(fs::read_to_string(wt.join(FORK_FILE)).unwrap(), resolved);
    let report = env.tool_json(&f.down, &["report", "--json"], 0);
    let taken = &report["upstreams"][0]["port"]["taken"][0];
    assert_eq!(taken["conflicts"][0]["path"], "app.txt");
    assert_eq!(taken["conflicts"][0]["resolution"], "manual");
    fs::rename(&moved, &f.up.bare).unwrap();

    // The resolution is reapplied when the same copy is made again.
    f.tool(&["abort"]).success();
    env.assert_no_update(&f.down);
    f.tool(&["start"]).success();
    write_plan(&f, &[("take", &c1, ""), ("take", &c2, "")]);
    let out = stdout(&f.tool(&["continue"]).code(1));
    assert!(
        out.contains("app.txt  (resolved by a recorded resolution)"),
        "{out}"
    );
    assert_eq!(fs::read_to_string(wt.join(FORK_FILE)).unwrap(), resolved);
    env.git(&wt, &["add", FORK_FILE]);
    f.tool(&["continue"]).success();
    f.tool(&["report"])
        .success()
        .stdout(contains("app.txt  (resolved by a recorded resolution)"));
    f.tool(&["verify"]).success();
    f.tool(&["finish"]).success();
    env.assert_no_update(&f.down);
}

#[test]
fn a_copy_that_changes_nothing_is_recorded_as_present() {
    let f = Fork::port();
    let env = &f.env;
    // The product made the same change on its own.
    env.commit(&f.down, &[("same.txt", "same\n")], "fork: same change");
    env.git(&f.down, &["push", "-q", "origin", "main"]);
    let same = upstream_change(&f, "same.txt", "same\n", "upstream: same change");
    let other = upstream_change(&f, "other.txt", "o\n", "upstream: other");
    let base = env.rev(&f.down, "main");

    f.tool(&["start"]).success();
    write_plan(&f, &[("take", &same, ""), ("take", &other, "")]);
    let out = stdout(&f.tool(&["continue"]).success());
    assert!(
        out.contains(&format!(
            "upstream: the copy of {} changes nothing; it is recorded as present",
            &same[..12]
        )),
        "{out}"
    );
    let wt = env.update_worktree(&f.down);
    // One copy, then the journal; nothing for the empty copy.
    assert_eq!(env.rev(&wt, "HEAD~2"), base);
    assert_eq!(taken_by(&f, &env.rev(&wt, "HEAD~1")), other);
    let journal = fs::read_to_string(wt.join(JOURNAL)).unwrap();
    assert!(
        journal.contains(&format!(
            "commit = \"{same}\"\nverdict = \"have\"\nnote = \"taken, but its copy changed nothing\""
        )),
        "{journal}"
    );
    let report = env.tool_json(&f.down, &["report", "--json"], 0);
    let port = &report["upstreams"][0]["port"];
    assert_eq!(port["unchanged"][0]["commit"], same.as_str());
    assert_eq!(port["taken"].as_array().unwrap().len(), 1);
    f.tool(&["report"]).success().stdout(contains(
        "recorded as present because the copy changed nothing",
    ));
    f.tool(&["verify"]).success();
    f.tool(&["finish"]).success();
    let up = port_status(&f, 0);
    assert_eq!(up["state"], "current");
    assert_eq!(up["decided"], 1);
    assert_eq!(up["taken"], 1);
}

#[test]
fn report_points_at_earlier_pending_commits_that_change_the_same_files() {
    let f = Fork::port();
    let env = &f.env;
    let helper = upstream_change(&f, "lib.txt", "helper\n", "upstream: add a helper");
    let unrelated = upstream_change(&f, "u.txt", "u\n", "upstream: unrelated");
    let user = upstream_change(&f, "lib.txt", "helper\nuser\n", "upstream: use the helper");
    f.tool(&["start"]).success();
    write_plan(
        &f,
        &[
            ("later", &helper, ""),
            ("later", &unrelated, ""),
            ("take", &user, ""),
        ],
    );
    // The copy conflicts, since the helper is missing; resolved by hand.
    f.tool(&["continue"]).code(1);
    let wt = env.update_worktree(&f.down);
    fs::write(wt.join("lib.txt"), "user\n").unwrap();
    env.git(&wt, &["add", "lib.txt"]);
    let out = stdout(&f.tool(&["continue"]).success());
    assert!(
        out.contains("1 taken commit changes files that earlier pending commits change too"),
        "{out}"
    );
    let report = env.tool_json(&f.down, &["report", "--json"], 0);
    let taken = &report["upstreams"][0]["port"]["taken"][0];
    assert_eq!(taken["commit"], user.as_str());
    let earlier = taken["earlier_pending"].as_array().unwrap();
    assert_eq!(earlier.len(), 1);
    assert_eq!(earlier[0]["commit"], helper.as_str());
    let text = stdout(&f.tool(&["report"]).success());
    assert!(
        text.contains(&format!(
            "earlier pending commits that change the same files:\n        {} upstream: add a helper",
            &helper[..12]
        )),
        "{text}"
    );
}

#[test]
fn switching_from_follow_to_port_starts_the_journal_at_the_last_merge() {
    let f = Fork::new();
    let env = &f.env;
    let c = upstream_commits(&f, 3);
    f.tool(&["start", &format!("upstream={}", c[0])]).success();
    f.tool(&["verify"]).success();
    f.tool(&["finish"]).success();
    f.pull();
    f.reconfigure("mode = \"follow\"", "mode = \"port\"");

    let up = port_status(&f, 0);
    assert_eq!(up["base"], c[0].as_str());
    assert_eq!(up["journal"], false);
    assert_eq!(up["pending"], 2);
    f.tool(&["start"]).success();
    write_plan(&f, &[("skip", &c[1], "")]);
    f.tool(&["continue"]).success();
    f.tool(&["verify"]).success();
    f.tool(&["finish"]).success();

    assert!(journal_at(&f, "main").contains(&format!("base = \"{}\"", c[0])));
    let up = port_status(&f, 0);
    assert_eq!(up["journal"], true);
    assert_eq!(up["base"], c[0].as_str());
    assert_eq!(up["decided"], 1);
    assert_eq!(up["pending"], 1);
    let log = env.tool_json(&f.down, &["log", "--json"], 0);
    assert_eq!(log["upstreams"][0]["commits"][0]["commit"], c[2].as_str());

    // Back in follow mode the journal no longer means anything.
    f.pull();
    f.reconfigure("mode = \"port\"", "mode = \"follow\"");
    let doctor = stdout(&f.tool(&["doctor"]).code(1));
    assert!(
        doctor.contains(&format!(
            "[FAIL] upstream: the journal {JOURNAL} belongs to upstream 'upstream', which is in follow mode"
        )),
        "{doctor}"
    );
}

#[test]
fn a_port_without_shared_history_needs_a_journal_written_by_hand() {
    let env = common::Env::new();
    let up = env.upstream("upstream", &[("u.txt", "u\n")]);
    let down = env.work("product");
    let publish = env.bare("product-publish");
    env.git(
        &down,
        &["remote", "add", "origin", &publish.to_string_lossy()],
    );
    let cfg = format!(
        "integration = \"main\"\n\n[publish]\nremote = \"origin\"\n\n[[upstream]]\nname = \"upstream\"\nurl = {:?}\nbranch = \"main\"\nmode = \"port\"\n",
        up.url()
    );
    env.commit(
        &down,
        &[(".git-intake.toml", &cfg)],
        "product: configuration",
    );
    env.git(&down, &["push", "-q", "origin", "main"]);
    let err = stderr(&env.tool(&down, &["start"]).code(2));
    assert!(
        err.contains("write the journal .git-intake/upstream.toml by hand, with the upstream commit the port starts from as its `base`"),
        "{err}"
    );
    env.assert_no_update(&down);
    let status = env.tool_json(&down, &["status", "--json"], 0);
    assert_eq!(status["upstreams"][0]["state"], "no_base");
    assert_eq!(status["upstreams"][0]["base"], serde_json::Value::Null);
    assert!(
        status["upstreams"][0]["detail"]
            .as_str()
            .unwrap()
            .contains("write the journal .git-intake/upstream.toml by hand")
    );
    let text = stdout(&env.tool(&down, &["status"]).success());
    assert!(text.contains("no base"), "{text}");
    env.tool(&down, &["status", "--check"]).code(1);
    env.tool(&down, &["doctor"])
        .code(1)
        .stdout(contains("write the journal"));

    // A journal written by hand starts the port.
    let root = env.rev(&up.bare, "main");
    env.push_upstream(&up, &[("v.txt", "v\n")], "upstream: v");
    env.commit(
        &down,
        &[(JOURNAL, &format!("base = \"{root}\"\n"))],
        "product: start the port",
    );
    env.git(&down, &["push", "-q", "origin", "main"]);
    let status = env.tool_json(&down, &["status", "--json"], 0);
    assert_eq!(status["upstreams"][0]["pending"], 1);
    assert_eq!(status["upstreams"][0]["journal"], true);
}

#[test]
fn journal_and_history_edited_by_hand_count() {
    let f = Fork::port();
    let env = &f.env;
    let c = upstream_commits(&f, 3);
    f.tool(&["start"]).success();
    write_plan(&f, &[("skip", &c[0], ""), ("skip", &c[1], "")]);
    f.tool(&["continue"]).success();
    f.tool(&["verify"]).success();
    f.tool(&["finish"]).success();
    assert_eq!(port_status(&f, 0)["pending"], 1);

    // A skip decision removed in an ordinary commit makes its commit pending again.
    let journal = journal_at(&f, "main");
    let cut = journal.find("\n[[decision]]").unwrap();
    let rest = &journal[cut + 1..];
    let second = rest[1..].find("[[decision]]").unwrap() + 1;
    let edited = format!("{}\n{}", &journal[..cut], &rest[second..]);
    f.pull();
    f.land(&[(JOURNAL, &edited)], "fork: reconsider change 1");
    let log = env.tool_json(&f.down, &["log", "--json"], 0);
    let pending: Vec<&str> = log["upstreams"][0]["commits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["commit"].as_str().unwrap())
        .collect();
    assert_eq!(pending, vec![c[0].as_str(), c[2].as_str()]);

    // A commit copied by hand with the trailers counts as taken.
    env.git(&f.down, &["fetch", "-q", &f.up.url(), "main"]);
    env.git(&f.down, &["cherry-pick", "--no-commit", &c[2]]);
    env.git(
        &f.down,
        &[
            "commit",
            "-q",
            "-C",
            &c[2],
            "--trailer",
            "Intake-Upstream: upstream",
            "--trailer",
            &format!("Intake-Taken: {}", c[2]),
        ],
    );
    env.git(&f.down, &["push", "-q", "origin", "main"]);
    let up = port_status(&f, 0);
    assert_eq!(up["taken"], 1);
    assert_eq!(up["decided"], 1);
    assert_eq!(up["pending"], 1);
}

#[test]
fn a_rewritten_port_upstream_is_reported_and_not_planned() {
    let f = Fork::port();
    let env = &f.env;
    let c = upstream_commits(&f, 2);
    f.tool(&["start"]).success();
    write_plan(&f, &[("skip", &c[1], "")]);
    f.tool(&["continue"]).success();
    f.tool(&["verify"]).success();
    f.tool(&["finish"]).success();

    // The upstream drops the skipped commit.
    env.git(&f.up.work, &["reset", "-q", "--hard", &c[0]]);
    env.commit(&f.up.work, &[("r.txt", "r\n")], "upstream: rewritten");
    env.git(
        &f.up.work,
        &["push", "-q", "--force", "origin", "HEAD:main"],
    );

    let up = port_status(&f, 0);
    assert_eq!(up["state"], "rewritten");
    assert_eq!(up["off_line"]["base"], serde_json::Value::Null);
    assert_eq!(
        up["off_line"]["commits"],
        serde_json::json!([{"commit": c[1], "recorded_as": "skip"}])
    );
    let detail = up["detail"].as_str().unwrap();
    assert!(detail.contains(&format!("{} (skip)", c[1])), "{detail}");
    assert!(
        detail
            .contains("list the commits under `retired` in the journal .git-intake/upstream.toml"),
        "{detail}"
    );
    f.tool(&["start"])
        .code(2)
        .stderr(contains("upstream 'upstream' was rewritten"))
        .stderr(contains(c[1].as_str()));
    env.assert_no_update(&f.down);
    f.tool(&["doctor"])
        .code(1)
        .stdout(contains("the upstream was rewritten"))
        .stdout(contains(format!("{} (skip)", c[1])));
}

#[test]
fn an_invalid_journal_is_refused_by_name() {
    let f = Fork::port();
    let env = &f.env;
    let c = upstream_commits(&f, 1);
    let base = env.rev(&f.up.bare, "main~1");
    f.land(
        &[(
            JOURNAL,
            &format!(
                "base = \"{base}\"\n[[decision]]\ncommit = \"{}\"\nverdict = \"drop\"\n",
                c[0]
            ),
        )],
        "fork: a broken journal",
    );
    let err = stderr(&f.tool(&["start"]).code(2));
    assert!(
        err.contains(&format!("invalid journal {JOURNAL} of upstream 'upstream'")),
        "{err}"
    );
    assert!(
        err.contains("the verdict 'drop' is neither `skip` nor `have`"),
        "{err}"
    );
    env.assert_no_update(&f.down);
    let up = port_status(&f, 0);
    assert_eq!(up["state"], "unavailable");
    assert!(up["detail"].as_str().unwrap().contains("invalid journal"));
    f.tool(&["doctor"])
        .code(1)
        .stdout(contains("invalid journal"));

    // A commit decided twice.
    f.land(
        &[(
            JOURNAL,
            &format!(
                "base = \"{base}\"\n[[decision]]\ncommit = \"{c0}\"\nverdict = \"skip\"\n\
                 [[decision]]\ncommit = \"{c0}\"\nverdict = \"have\"\n",
                c0 = c[0]
            ),
        )],
        "fork: a commit decided twice",
    );
    f.tool(&["start"])
        .code(2)
        .stderr(contains(format!(
            "invalid journal {JOURNAL} of upstream 'upstream'"
        )))
        .stderr(contains(format!(
            "decision[1]: commit {} is decided more than once",
            c[0]
        )));
    env.assert_no_update(&f.down);
    assert_eq!(port_status(&f, 0)["state"], "unavailable");
}

#[test]
fn a_plan_is_limited_to_a_release() {
    let f = Fork::port();
    let env = &f.env;
    let c = upstream_commits(&f, 3);
    env.git(&f.up.work, &["tag", "v1", &c[1]]);
    env.git(&f.up.work, &["push", "-q", "origin", "v1"]);
    f.tool(&["start", "upstream=v1"]).success().stdout(contains(
        "upstream: 2 pending commits planned up to v1; 1 later pending commit left out",
    ));
    let plan = fs::read_to_string(plan_path(&f)).unwrap();
    assert!(
        plan.contains(&c[0][..12]) && plan.contains(&c[1][..12]),
        "{plan}"
    );
    assert!(!plan.contains(&c[2][..12]), "{plan}");
    assert!(
        env.git(&f.down, &["tag", "--list"]).is_empty(),
        "no tag is fetched"
    );
    write_plan(&f, &[("take", &c[0], ""), ("take", &c[1], "")]);
    f.tool(&["continue"]).success();
    let report = env.tool_json(&f.down, &["report", "--json"], 0);
    assert_eq!(report["upstreams"][0]["rev"], "v1");
    assert_eq!(report["upstreams"][0]["port"]["left_pending"], 1);
    f.tool(&["verify"]).success();
    f.tool(&["finish"]).success();
    f.tool(&["start", "upstream=v1"]).success().stdout(contains(
        "upstream: current up to v1; 1 later commit pending",
    ));
    env.assert_no_update(&f.down);
}

#[test]
fn merge_request_describes_the_port() {
    let f = Fork::port_merge_request();
    let c = upstream_commits(&f, 3);
    f.tool(&["start"]).success();
    write_plan(
        &f,
        &[
            ("take", &c[0], ""),
            ("skip", &c[1], ""),
            ("have", &c[2], ""),
        ],
    );
    f.tool(&["continue"]).success();
    f.tool(&["finish"]).success();
    let description = f.forge().mr(1).description;
    assert!(
        description.contains(&format!(
            "`upstream` from upstream/main, ported: took {}; skipped {}; already present {}",
            &c[0][..12],
            &c[1][..12],
            &c[2][..12]
        )),
        "{description}"
    );
}

#[test]
fn doctor_reports_stray_references_and_journals_of_no_upstream() {
    let f = Fork::port();
    let env = &f.env;
    let tip = env.rev(&f.down, "main");
    env.git(
        &f.down,
        &["update-ref", "refs/git-intake/upstreams/upstream", &tip],
    );
    let doctor = stdout(&f.tool(&["doctor"]).code(1));
    assert!(
        doctor.contains(&format!(
            "[FAIL] references under refs/git-intake/ are left without an update: refs/git-intake/upstreams/upstream at {}",
            &tip[..12]
        )),
        "{doctor}"
    );
    // Reading commands never touch it; removing it by hand clears the check.
    f.tool(&["status"]).success();
    assert_eq!(
        env.intake_refs(&f.down),
        vec!["refs/git-intake/upstreams/upstream".to_string()]
    );
    env.git(
        &f.down,
        &["update-ref", "-d", "refs/git-intake/upstreams/upstream"],
    );
    // A namespace an interrupted command left behind is reported too.
    let left = "refs/git-intake/read/1-a/upstreams/upstream";
    env.git(&f.down, &["update-ref", left, &tip]);
    let doctor = stdout(&f.tool(&["doctor"]).code(1));
    assert!(
        doctor.contains(&format!("left without an update: {left} at")),
        "{doctor}"
    );
    env.git(&f.down, &["update-ref", "-d", left]);
    f.tool(&["doctor"]).success();

    f.land(
        &[(".git-intake/gone.toml", "base = \"x\"\n")],
        "fork: stale journal",
    );
    let doctor = stdout(&f.tool(&["doctor"]).code(1));
    assert!(
        doctor.contains(
            "[FAIL] gone: the journal .git-intake/gone.toml belongs to no configured upstream"
        ),
        "{doctor}"
    );
}

#[test]
fn contribute_creates_a_branch_of_upstream_history_only() {
    let f = Fork::port();
    let env = &f.env;
    let c = upstream_commits(&f, 2);
    env.git(&f.up.work, &["tag", "v1", &c[0]]);
    env.git(&f.up.work, &["push", "-q", "origin", "v1"]);
    let head = env.rev(&f.down, "HEAD");

    let out = stdout(
        &f.tool(&["contribute", "upstream", "fix-upstream"])
            .success(),
    );
    assert!(
        out.contains("created branch 'fix-upstream' at its fetched tip of upstream 'upstream'"),
        "{out}"
    );
    assert!(out.contains("The tool does not push it"), "{out}");
    assert_eq!(env.rev(&f.down, "fix-upstream"), c[1]);
    assert_eq!(
        env.git(
            &f.down,
            &["rev-list", "--count", "fix-upstream", "--not", &c[1]]
        ),
        "0",
        "the branch holds only upstream history"
    );
    assert!(
        !env.git_output(&f.down, &["config", "--get", "branch.fix-upstream.merge"])
            .status
            .success(),
        "no tracking branch"
    );
    assert_eq!(env.rev(&f.down, "HEAD"), head, "no checkout changed");
    assert_eq!(env.git(&f.down, &["branch", "--show-current"]), "main");
    assert!(
        !env.has_rev(&f.publish, "refs/heads/fix-upstream"),
        "nothing was pushed"
    );
    assert!(
        !env.has_rev(&f.up.bare, "refs/heads/fix-upstream"),
        "nothing was pushed"
    );
    assert!(env.intake_refs(&f.down).is_empty());

    f.tool(&["contribute", "upstream=v1", "from-release"])
        .success()
        .stdout(contains("at 'v1' of upstream 'upstream'"));
    assert_eq!(env.rev(&f.down, "from-release"), c[0]);

    f.tool(&["contribute", "upstream", "fix-upstream"])
        .code(2)
        .stderr(contains("the branch 'fix-upstream' already exists"));
    assert_eq!(env.rev(&f.down, "fix-upstream"), c[1]);
    f.tool(&["contribute", "upstream", "bad..name"])
        .code(2)
        .stderr(contains("'bad..name' is not a valid branch name"));
    f.tool(&["contribute", "nope", "x"])
        .code(2)
        .stderr(contains("unknown upstream 'nope'"));
    assert!(env.intake_refs(&f.down).is_empty());
}

/// Rewrites the upstream's whole history: a new root with the same files as
/// its first commit, and the given changes on top, force-pushed. Returns the
/// new commits, the root first.
fn rewrite_upstream(f: &Fork, changes: &[(&str, &str, &str)]) -> Vec<String> {
    let env = &f.env;
    env.git(&f.up.work, &["checkout", "-q", "--orphan", "rewritten"]);
    env.git(&f.up.work, &["rm", "-q", "-r", "-f", "--cached", "."]);
    for entry in fs::read_dir(&f.up.work).unwrap() {
        let path = entry.unwrap().path();
        if path.file_name().unwrap() != ".git" {
            if path.is_dir() {
                fs::remove_dir_all(path).unwrap();
            } else {
                fs::remove_file(path).unwrap();
            }
        }
    }
    let mut commits = vec![env.commit(
        &f.up.work,
        &[(FORK_FILE, common::FORK_CONTENT)],
        "upstream: initial, rewritten",
    )];
    for (file, content, message) in changes {
        commits.push(env.commit(&f.up.work, &[(file, content)], message));
    }
    env.git(
        &f.up.work,
        &["push", "-q", "--force", "origin", "HEAD:main"],
    );
    commits
}

#[test]
fn retiring_commits_recovers_a_rewritten_port_upstream() {
    let f = Fork::port();
    let env = &f.env;
    let c = upstream_commits(&f, 3);
    f.tool(&["start"]).success();
    write_plan(&f, &[("take", &c[0], ""), ("take", &c[1], "")]);
    f.tool(&["continue"]).success();
    f.tool(&["verify"]).success();
    f.tool(&["finish"]).success();
    let base = env.rev(&f.up.bare, &format!("{}~1", c[0]));

    // The upstream rewrites the commits after the base: the taken commits
    // are no longer on its line, and once garbage-collected no longer in
    // the repository either.
    env.git(&f.up.work, &["reset", "-q", "--hard", &base]);
    let mut again = Vec::new();
    for i in 1..=3 {
        again.push(env.commit(
            &f.up.work,
            &[(&format!("p{i}.txt") as &str, &format!("{i}\n") as &str)],
            &format!("upstream: change {i}, rewritten"),
        ));
    }
    env.git(
        &f.up.work,
        &["push", "-q", "--force", "origin", "HEAD:main"],
    );
    env.git(&f.down, &["gc", "-q", "--prune=now"]);
    assert!(
        !env.has_rev(&f.down, &format!("{}^{{commit}}", c[0])),
        "pruned"
    );

    let up = port_status(&f, 0);
    assert_eq!(up["state"], "rewritten");
    assert_eq!(
        up["off_line"]["commits"],
        serde_json::json!([
            {"commit": c[0], "recorded_as": "taken"},
            {"commit": c[1], "recorded_as": "taken"}
        ])
    );
    let text = stdout(&f.tool(&["status"]).success());
    assert!(text.contains(&format!("{} (taken)", c[1])), "{text}");
    assert!(text.contains("list the commits under `retired`"), "{text}");
    f.tool(&["start"]).code(2).stderr(contains("was rewritten"));
    f.tool(&["doctor"])
        .code(1)
        .stdout(contains(format!("{} (taken)", c[0])));

    // The owner acknowledges them, in upper case as some tools print ids.
    let journal = journal_at(&f, "main");
    assert!(journal.contains(&format!("base = \"{base}\"")), "{journal}");
    let journal = journal.replace(
        &format!("base = \"{base}\""),
        &format!(
            "base = \"{base}\"\nretired = [\"{}\", \"{}\"]",
            c[0].to_uppercase(),
            c[1].to_uppercase()
        ),
    );
    f.pull();
    f.land(&[(JOURNAL, &journal)], "fork: retire the rewritten commits");
    let up = port_status(&f, 0);
    assert_eq!(up["state"], "pending");
    assert_eq!(up["taken"], 0);
    assert_eq!(up["pending"], 3);
    f.tool(&["doctor"]).success();

    // The rewritten versions are pending and can be marked present.
    f.tool(&["start"]).success();
    write_plan(
        &f,
        &[
            ("have", &again[0], "taken before the rewrite"),
            ("have", &again[1], ""),
            ("take", &again[2], ""),
        ],
    );
    f.tool(&["continue"]).success();
    f.tool(&["verify"]).success();
    f.tool(&["finish"]).success();
    let up = port_status(&f, 0);
    assert_eq!(up["state"], "current");
    let journal = journal_at(&f, "main");
    assert!(
        journal.contains(&format!(
            "retired = [\n    \"{}\",\n    \"{}\",\n]",
            c[0], c[1]
        )),
        "{journal}"
    );

    // A rewrite that drops the base as well: the base has to move.
    let root = rewrite_upstream(&f, &[("q.txt", "q\n", "upstream: q")]);
    env.git(&f.down, &["gc", "-q", "--prune=now"]);
    let up = port_status(&f, 0);
    assert_eq!(up["state"], "rewritten");
    assert_eq!(up["off_line"]["base"], base.as_str());
    let off: Vec<&str> = up["off_line"]["commits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["commit"].as_str().unwrap())
        .collect();
    assert_eq!(
        off,
        vec![again[0].as_str(), again[1].as_str(), again[2].as_str()]
    );
    let detail = up["detail"].as_str().unwrap();
    assert!(detail.contains("move the base in the journal"), "{detail}");
    // Retiring alone is not enough while the base is off the line.
    let retired = format!(
        "retired = [\"{}\", \"{}\", \"{}\", \"{}\", \"{}\"]\n",
        c[0], c[1], again[0], again[1], again[2]
    );
    let journal = journal_at(&f, "main");
    let start = journal.find("retired = [").unwrap();
    let end = start + journal[start..].find("]\n").unwrap() + 2;
    let retired_only = format!("{}{retired}{}", &journal[..start], &journal[end..]);
    f.pull();
    f.land(&[(JOURNAL, &retired_only)], "fork: retire again");
    assert_eq!(port_status(&f, 0)["state"], "rewritten");
    let moved = retired_only.replace(
        &format!("base = \"{base}\""),
        &format!("base = \"{}\"", root[0]),
    );
    f.land(&[(JOURNAL, &moved)], "fork: move the base");
    let up = port_status(&f, 0);
    assert_eq!(up["state"], "pending");
    assert_eq!(up["base"], root[0].as_str());
    assert_eq!(up["pending"], 1);
    f.tool(&["start"]).success();
    f.tool(&["abort"]).success();
    env.assert_no_update(&f.down);
}

#[test]
fn a_failed_copy_commit_stops_and_continue_commits_the_fix() {
    let f = Fork::port();
    let env = &f.env;
    let bad = upstream_change(&f, "bad.txt", "FORBIDDEN\n", "upstream: add bad.txt");
    let ok = upstream_change(&f, "ok.txt", "ok\n", "upstream: add ok.txt");
    // A hook of this repository rejects some content.
    let hook = f.down.join(".git").join("hooks").join("pre-commit");
    fs::create_dir_all(hook.parent().unwrap()).unwrap();
    fs::write(
        &hook,
        "#!/bin/sh\nif git diff --cached | grep -q FORBIDDEN; then echo 'forbidden content' >&2; exit 1; fi\nexit 0\n",
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();

    f.tool(&["start"]).success();
    write_plan(&f, &[("take", &bad, ""), ("take", &ok, "")]);
    let out = stdout(&f.tool(&["continue"]).code(1));
    assert!(
        out.contains(&format!(
            "upstream: the copy of {} upstream: add bad.txt could not be committed",
            &bad[..12]
        )),
        "{out}"
    );
    assert!(out.contains("forbidden content"), "{out}");
    assert!(out.contains("the copy stays staged"), "{out}");
    let status = env.tool_json(&f.down, &["status", "--json"], 0);
    assert_eq!(status["update"]["step"], "commit_failed");
    let planned = &status["update"]["upstreams"][0]["commits"][0];
    assert_eq!(planned["copy"], "commit_failed");
    assert!(
        planned["failure"]
            .as_str()
            .unwrap()
            .contains("forbidden content")
    );
    assert!(
        status["update"]["next"]
            .as_str()
            .unwrap()
            .contains("stage the result")
    );

    // Continue without a fix fails again and keeps the staged copy.
    let wt = env.update_worktree(&f.down);
    f.tool(&["continue"]).code(1);
    assert_eq!(
        fs::read_to_string(wt.join("bad.txt")).unwrap(),
        "FORBIDDEN\n"
    );

    // The fix is staged, and an unrelated edit is left unstaged.
    fs::write(wt.join("bad.txt"), "fine\n").unwrap();
    env.git(&wt, &["add", "bad.txt"]);
    fs::write(wt.join(FORK_FILE), "edited\n").unwrap();
    let out = stdout(&f.tool(&["continue"]).success());
    assert!(
        out.contains(&format!("upstream: copied {} as", &bad[..12])),
        "{out}"
    );
    assert!(
        out.contains(&format!("upstream: copied {} as", &ok[..12])),
        "{out}"
    );
    assert_eq!(taken_by(&f, &env.rev(&wt, "HEAD~2")), bad);
    assert_eq!(
        env.git(&wt, &["show", "HEAD~2:bad.txt"]),
        "fine",
        "the staged fix is the copy"
    );
    assert_eq!(
        env.git(&wt, &["log", "-1", "--format=%an <%ae>", "HEAD~2"]),
        UPSTREAM_AUTHOR
    );
    assert_eq!(
        fs::read_to_string(wt.join(FORK_FILE)).unwrap(),
        "edited\n",
        "unstaged work is kept"
    );
}

#[test]
fn a_copy_never_changes_the_git_intake_files() {
    let f = Fork::port();
    let env = &f.env;
    let config_before = fs::read_to_string(f.down.join(".git-intake.toml")).unwrap();
    // The upstream carries git-intake files of its own.
    env.git(&f.up.work, &["add", "-A"]);
    fs::create_dir_all(f.up.work.join(".git-intake")).unwrap();
    fs::write(
        f.up.work.join(".git-intake.toml"),
        "integration = \"theirs\"\n",
    )
    .unwrap();
    fs::write(
        f.up.work.join(".git-intake").join("x.toml"),
        "base = \"x\"\n",
    )
    .unwrap();
    fs::write(f.up.work.join("real.txt"), "real\n").unwrap();
    env.git(&f.up.work, &["add", "-A"]);
    env.git(
        &f.up.work,
        &["commit", "-q", "-m", "upstream: real change and own files"],
    );
    env.git(&f.up.work, &["push", "-q", "origin", "HEAD:main"]);
    let mixed = env.rev(&f.up.work, "HEAD");
    let only_own = upstream_change(
        &f,
        ".git-intake/y.toml",
        "base = \"y\"\n",
        "upstream: own files only",
    );

    f.tool(&["start"]).success();
    write_plan(&f, &[("take", &mixed, ""), ("take", &only_own, "")]);
    let out = stdout(&f.tool(&["continue"]).success());
    assert!(
        out.contains(&format!(
            "upstream: left out of the copy of {}: .git-intake.toml, .git-intake/x.toml",
            &mixed[..12]
        )),
        "{out}"
    );
    assert!(
        out.contains(&format!(
            "upstream: the copy of {} changes nothing",
            &only_own[..12]
        )),
        "{out}"
    );
    let wt = env.update_worktree(&f.down);
    assert_eq!(
        fs::read_to_string(wt.join(".git-intake.toml")).unwrap(),
        config_before
    );
    assert!(wt.join("real.txt").exists());
    assert!(!wt.join(".git-intake").join("x.toml").exists());
    assert!(!wt.join(".git-intake").join("y.toml").exists());
    let journal = fs::read_to_string(wt.join(JOURNAL)).unwrap();
    assert!(
        journal.contains("note = \"taken, but its copy changed nothing outside .git-intake.toml and .git-intake/\""),
        "{journal}"
    );
    let report = env.tool_json(&f.down, &["report", "--json"], 0);
    let port = &report["upstreams"][0]["port"];
    assert_eq!(
        port["taken"][0]["left_out"],
        serde_json::json!([".git-intake.toml", ".git-intake/x.toml"])
    );
    assert_eq!(
        port["unchanged"][0]["left_out"],
        serde_json::json!([".git-intake/y.toml"])
    );
    f.tool(&["report"])
        .success()
        .stdout(contains("left out: .git-intake.toml, .git-intake/x.toml"));
    f.tool(&["verify"]).success();
}

#[test]
fn a_merge_commit_is_taken_as_its_change_against_its_first_parent() {
    let f = Fork::port();
    let env = &f.env;
    env.git(&f.up.work, &["checkout", "-q", "-b", "feature"]);
    env.commit(&f.up.work, &[("f1.txt", "1\n")], "feature: one");
    env.commit(&f.up.work, &[("f2.txt", "2\n")], "feature: two");
    env.git(&f.up.work, &["checkout", "-q", "main"]);
    env.git(
        &f.up.work,
        &[
            "merge",
            "-q",
            "--no-ff",
            "-m",
            "Merge pull request: feature",
            "feature",
        ],
    );
    env.git(&f.up.work, &["push", "-q", "origin", "main"]);
    let merge = env.rev(&f.up.work, "HEAD");

    f.tool(&["start"]).success();
    let plan = fs::read_to_string(plan_path(&f)).unwrap();
    let listed: Vec<&str> = plan.lines().filter(|l| !l.starts_with('#')).collect();
    assert_eq!(
        listed,
        vec![format!(
            "later {} Merge pull request: feature",
            &merge[..12]
        )]
    );
    write_plan(&f, &[("take", &merge, "")]);
    f.tool(&["continue"]).success();
    let wt = env.update_worktree(&f.down);
    let copy = env.rev(&wt, "HEAD~1");
    assert_eq!(taken_by(&f, &copy), merge);
    assert_eq!(
        env.git(&wt, &["rev-list", "--parents", "-n", "1", &copy])
            .split(' ')
            .count(),
        2,
        "one parent"
    );
    assert!(wt.join("f1.txt").exists() && wt.join("f2.txt").exists());
    f.tool(&["verify"]).success();
    f.tool(&["finish"]).success();
    assert_eq!(port_status(&f, 0)["state"], "current");
}

#[test]
fn reading_commands_never_move_the_update_references() {
    let f = Fork::port();
    let env = &f.env;
    upstream_commits(&f, 1);
    f.tool(&["start"]).success();
    let held = env.rev(&f.down, "refs/git-intake/upstreams/upstream");
    let moved = upstream_change(&f, "later.txt", "l\n", "upstream: later");
    let up = port_status(&f, 0);
    assert_eq!(up["tip"], moved.as_str());
    env.tool(&f.down, &["log"]).success();
    env.tool(&f.down, &["branches"]).success();
    env.tool(&f.down, &["doctor"]).success();
    assert_eq!(env.rev(&f.down, "refs/git-intake/upstreams/upstream"), held);
    assert_eq!(
        env.intake_refs(&f.down),
        vec!["refs/git-intake/upstreams/upstream".to_string()],
        "nothing else is left"
    );
}

/// Edits the update state as an interrupted command would have left it.
fn edit_state(f: &Fork, edit: impl FnOnce(&mut serde_json::Value)) {
    let path = f.env.intake_dir(&f.down).join("state.json");
    let mut state: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    edit(&mut state);
    fs::write(&path, serde_json::to_string_pretty(&state).unwrap()).unwrap();
}

#[test]
fn a_start_interrupted_before_holding_its_references_is_repaired() {
    let f = Fork::port();
    let env = &f.env;
    let c = upstream_commits(&f, 2);
    f.tool(&["start"]).success();
    let held = "refs/git-intake/upstreams/upstream";
    let fetched = env.rev(&f.down, held);
    // The window between saving the state and holding the references: the
    // fetched commits live only in the namespace of the interrupted start.
    let left = "refs/git-intake/read/7-1f/upstreams/upstream";
    env.git(&f.down, &["update-ref", left, &fetched]);
    env.git(&f.down, &["update-ref", "-d", held]);
    env.git(&f.down, &["gc", "-q", "--prune=now"]);

    let doctor = stdout(&f.tool(&["doctor"]).code(1));
    assert!(
        doctor.contains(&format!(
            "[FAIL] the update's reference {held} is missing; `git intake status`, `continue`, `verify` or `finish` \
             restores it at {} from the update state",
            &fetched[..12]
        )),
        "{doctor}"
    );
    assert!(
        doctor.contains(&format!(
            "[warn] {left} at {} still holds commits the update in progress needs; keep it",
            &fetched[..12]
        )),
        "{doctor}"
    );
    assert!(!doctor.contains("are left without an update"), "{doctor}");

    // `continue` restores the reference and carries the plan out.
    write_plan(&f, &[("take", &c[0], ""), ("take", &c[1], "")]);
    let out = f.tool(&["continue"]).success();
    assert!(
        stderr(&out).contains(&format!("note: restored the update's reference {held}")),
        "{}",
        stderr(&out)
    );
    assert_eq!(env.rev(&f.down, held), fetched);
    // The leftover namespace no longer protects anything and is stray now.
    f.tool(&["doctor"])
        .code(1)
        .stdout(contains(format!("left without an update: {left} at")));
    env.git(&f.down, &["update-ref", "-d", left]);
    f.tool(&["doctor"]).success();

    // status restores it as well, and doctor names a reference moved by hand.
    env.git(&f.down, &["update-ref", "-d", held]);
    f.tool(&["status"])
        .success()
        .stderr(contains("note: restored the update's reference"));
    assert_eq!(env.rev(&f.down, held), fetched);
    env.git(&f.down, &["update-ref", held, &env.rev(&f.down, "main")]);
    f.tool(&["doctor"]).code(1).stdout(contains(format!(
        "the update's reference {held} is at {} instead of {}",
        &env.rev(&f.down, "main")[..12],
        &fetched[..12]
    )));
}

#[test]
fn an_interrupted_copy_left_unstaged_stops_until_it_is_staged() {
    let f = Fork::port();
    let env = &f.env;
    let c = upstream_commits(&f, 1);
    f.tool(&["start"]).success();
    let wt = env.update_worktree(&f.down);
    let head = env.rev(&wt, "HEAD");
    // The plan was read and the copy began; what it applied sits in the
    // worktree, unstaged.
    edit_state(&f, |s| {
        let q = &mut s["upstreams"][0];
        q["step"] = "pending".into();
        q["takes"] = serde_json::json!([0]);
        q["commits"][0]["verb"] = "take".into();
        q["commits"][0]["head_before"] = head.clone().into();
    });
    fs::write(wt.join("p1.txt"), "1\n").unwrap();
    env.git(&wt, &["add", "-N", "p1.txt"]);
    let out = stdout(&f.tool(&["continue"]).code(1));
    assert!(
        out.contains(
            "nothing of the copy is staged, but files in the worktree have unstaged changes"
        ),
        "{out}"
    );
    assert!(out.contains("stage what belongs to the copy"), "{out}");
    assert_eq!(env.rev(&wt, "HEAD"), head, "nothing is committed");
    let status = env.tool_json(&f.down, &["status", "--json"], 0);
    assert_eq!(status["update"]["step"], "commit_failed");
    assert!(wt.join("p1.txt").exists(), "the worktree is kept");

    env.git(&wt, &["add", "p1.txt"]);
    f.tool(&["continue"])
        .success()
        .stdout(contains(format!("upstream: copied {} as", &c[0][..12])));
    assert_eq!(taken_by(&f, &env.rev(&wt, "HEAD~1")), c[0]);
    let journal = fs::read_to_string(wt.join(JOURNAL)).unwrap();
    assert!(!journal.contains("verdict = \"have\""), "{journal}");
}

#[test]
fn a_copy_whose_added_file_was_unstaged_is_not_recorded_as_present() {
    let f = Fork::port();
    let env = &f.env;
    let added = upstream_change(&f, "q.txt", "FORBIDDEN\n", "upstream: add q.txt");
    let hook = f.down.join(".git").join("hooks").join("pre-commit");
    fs::create_dir_all(hook.parent().unwrap()).unwrap();
    fs::write(
        &hook,
        "#!/bin/sh\nif git diff --cached | grep -q FORBIDDEN; then echo 'forbidden content' >&2; exit 1; fi\nexit 0\n",
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();

    f.tool(&["start"]).success();
    write_plan(&f, &[("take", &added, "")]);
    f.tool(&["continue"]).code(1);
    let wt = env.update_worktree(&f.down);
    let head = env.rev(&wt, "HEAD");
    // The added file is unstaged and becomes untracked.
    env.git(&wt, &["restore", "--staged", "q.txt"]);
    let out = stdout(&f.tool(&["continue"]).code(1));
    assert!(
        out.contains(
            "nothing of the copy is staged, but files in the worktree have unstaged changes"
        ),
        "{out}"
    );
    assert_eq!(env.rev(&wt, "HEAD"), head, "nothing is committed");
    assert!(!wt.join(JOURNAL).exists(), "no decision is recorded");

    fs::write(wt.join("q.txt"), "fine\n").unwrap();
    env.git(&wt, &["add", "q.txt"]);
    f.tool(&["continue"])
        .success()
        .stdout(contains(format!("upstream: copied {} as", &added[..12])));
    // The copy, then the journal that the first plan starts.
    assert_eq!(taken_by(&f, &env.rev(&wt, "HEAD~1")), added);
}

/// Opens the merge request of the update in progress, has the forge merge
/// it with a merge commit, and brings the local integration branch up to
/// date.
fn land_with_a_merge_commit(f: &Fork) {
    let env = &f.env;
    let commit = env.rev(&env.update_worktree(&f.down), "HEAD");
    f.tool(&["finish"]).success();
    merge_on_fake(env, f.forge(), open_number(f.forge()), How::MergeCommit);
    env.git(&f.down, &["pull", "-q", "--ff-only", "origin", "main"]);
    assert_eq!(env.rev(&f.down, "main^2"), commit);
}

#[test]
fn a_port_starts_at_a_record_merged_with_a_merge_commit() {
    let f = Fork::merge_request();
    let c = upstream_commits(&f, 3);
    f.tool(&["start", &format!("upstream={}", c[0])]).success();
    land_with_a_merge_commit(&f);
    // c1 merged by hand, without a record: the history would say c1, the
    // record says c0.
    let env = &f.env;
    env.git(&f.down, &["fetch", "-q", &f.up.url(), &c[1]]);
    env.git(
        &f.down,
        &["merge", "-q", "--no-ff", "-m", "Merge c1 by hand", &c[1]],
    );
    env.git(&f.down, &["push", "-q", "origin", "main"]);
    f.reconfigure("mode = \"follow\"", "mode = \"port\"");

    let up = port_status(&f, 0);
    assert_eq!(up["base"], c[0].as_str());
    assert_eq!(up["journal"], false);
    assert_eq!(up["pending"], 2);

    // The port update lands the same way; its journal starts at the record,
    // and its copy counts as taken.
    f.tool(&["start"]).success();
    write_plan(&f, &[("have", &c[1], ""), ("take", &c[2], "")]);
    f.tool(&["continue"]).success();
    land_with_a_merge_commit(&f);
    assert!(journal_at(&f, "main").contains(&format!("base = \"{}\"", c[0])));
    let up = port_status(&f, 0);
    assert_eq!(up["state"], "current");
    assert_eq!(up["journal"], true);
    assert_eq!(up["base"], c[0].as_str());
    assert_eq!(up["decided"], 1);
    assert_eq!(up["taken"], 1);
    assert_eq!(up["pending"], 0);
}

#[test]
fn a_caught_up_port_merged_with_a_merge_commit_counts_its_takes() {
    let f = Fork::port_merge_request();
    let env = &f.env;
    let c = upstream_commits(&f, 2);
    f.tool(&["start"]).success();
    write_plan(&f, &[("take", &c[0], ""), ("skip", &c[1], "")]);
    f.tool(&["continue"]).success();
    let commit = env.rev(&env.update_worktree(&f.down), "HEAD");
    f.tool(&["finish"]).success();
    // Other work lands first; `sync` catches the merge request up and
    // pushes the result.
    let forge = common::forge(env, &f.publish);
    env.commit(&forge, &[("other.txt", "o\n")], "other: lands first");
    env.git(&forge, &["push", "-q", "origin", "HEAD:main"]);
    f.tool(&["sync"]).success();
    let branch = format!("refs/heads/{}", common::MERGE_REQUEST_BRANCH);
    let caught = env.rev(&f.publish, &branch);
    assert_ne!(caught, commit);
    assert_eq!(env.rev(&f.publish, &format!("{caught}^2")), commit);

    merge_on_fake(env, f.forge(), open_number(f.forge()), How::MergeCommit);
    env.git(&f.down, &["pull", "-q", "--ff-only", "origin", "main"]);
    assert_eq!(env.rev(&f.down, "main^2"), caught);
    let up = port_status(&f, 0);
    assert_eq!(up["state"], "current");
    assert_eq!(up["journal"], true);
    assert_eq!(up["taken"], 1);
    assert_eq!(up["decided"], 1);
    assert_eq!(up["pending"], 0);
}

#[test]
fn a_catch_up_refuses_a_changed_journal() {
    let f = Fork::port();
    let env = &f.env;
    let c = upstream_commits(&f, 2);
    f.tool(&["start"]).success();
    write_plan(&f, &[("take", &c[0], ""), ("skip", &c[1], "")]);
    f.tool(&["continue"]).success();
    f.tool(&["verify"]).success();
    let verified = env.rev(&env.update_worktree(&f.down), "HEAD");
    let tip = f.land(
        &[(JOURNAL, "# written by hand\n")],
        "fork: edit the journal",
    );

    let err = stderr(&f.tool(&["finish"]).code(2));
    assert!(
        err.contains("the journal .git-intake/upstream.toml of port upstream 'upstream' changed"),
        "{err}"
    );
    assert!(err.contains("Run `git intake abort`"), "{err}");
    assert_eq!(env.rev(&env.update_worktree(&f.down), "HEAD"), verified);
    assert_eq!(f.tip(), tip, "nothing was pushed");
}
