//! Updates that take an upstream at a revision named on the command line
//! instead of its fetched tip.

mod common;

use std::fs;

use common::{FORK_FILE, Fork, Mono, stderr, stdout};
use predicates::str::contains;

/// Pushes three commits to the fork's upstream and returns them, oldest first.
fn three_upstream_commits(f: &Fork) -> [String; 3] {
    let env = &f.env;
    [
        env.push_upstream(&f.up, &[("c1.txt", "1\n")], "upstream: c1"),
        env.push_upstream(&f.up, &[("c2.txt", "2\n")], "upstream: c2"),
        env.push_upstream(&f.up, &[("c3.txt", "3\n")], "upstream: c3"),
    ]
}

#[test]
fn commit_target_is_merged_published_and_shown_as_behind() {
    let f = Fork::new();
    let env = &f.env;
    let [_, c2, c3] = three_upstream_commits(&f);
    let base = env.rev(&f.down, "main");
    let short = &c2[..10];

    let start = f.tool(&["start", &format!("upstream={short}")]).success();
    let out = stdout(&start);
    assert!(
        out.contains(&format!(
            "upstream: queued at {} ({short}) (2 new commits)",
            &c2[..12]
        )),
        "{out}"
    );
    let wt = env.update_worktree(&f.down);
    assert!(wt.join("c2.txt").exists());
    assert!(!wt.join("c3.txt").exists());

    let status = env.tool_json(&f.down, &["status", "--json"], 0);
    assert_eq!(status["update"]["upstreams"][0]["target"], c2.as_str());
    assert_eq!(status["update"]["upstreams"][0]["rev"], short);
    f.tool(&["status"]).success().stdout(contains(format!(
        "upstream: target {} ({short}), merged",
        &c2[..12]
    )));

    f.tool(&["verify"]).success();
    f.tool(&["finish"]).success();
    env.assert_no_update(&f.down);
    assert_eq!(env.rev(&f.down, "main^1"), base);
    assert_eq!(env.rev(&f.down, "main^2"), c2);
    let msg = env.git(&f.down, &["log", "-1", "--format=%B", "main"]);
    assert!(msg.contains(&format!("Intake-Target: {c2}")), "{msg}");
    assert_eq!(env.rev(&f.down, "upstream-main"), c2);
    assert_eq!(env.rev(&f.publish, "upstream-main"), c2);

    let report = env.tool_json(&f.down, &["report", "--json"], 0);
    assert_eq!(report["upstreams"][0]["target"], c2.as_str());
    assert_eq!(report["upstreams"][0]["rev"], short);
    assert_eq!(report["upstreams"][0]["commits"], 2);
    f.tool(&["report"])
        .success()
        .stdout(contains(format!("target named by: {short}")));

    // The upstream is behind its tip by the commits the update left out.
    let status = env.tool_json(&f.down, &["status", "--json"], 0);
    let up = &status["upstreams"][0];
    assert_eq!(up["last_synced"], c2.as_str());
    assert_eq!(up["last_synced_source"], "record");
    assert_eq!(up["tip"], c3.as_str());
    assert_eq!(up["state"], "behind");
    assert_eq!(up["commits_behind"], 1);
    f.tool(&["status", "--check"])
        .code(1)
        .stdout(contains("upstream: 1 commit behind"));

    // A later update without a revision takes the rest.
    f.tool(&["start"])
        .success()
        .stdout(contains("(1 new commit)"));
    f.tool(&["verify"]).success();
    f.tool(&["finish"]).success();
    assert_eq!(env.rev(&f.down, "upstream-main"), c3);
    let status = env.tool_json(&f.down, &["status", "--json"], 0);
    assert_eq!(status["upstreams"][0]["state"], "current");
    let report = env.tool_json(&f.down, &["report", "--json"], 0);
    assert!(report["upstreams"][0]["rev"].is_null());
}

#[test]
fn tag_target_is_the_upstream_tag_and_no_tag_is_fetched() {
    let f = Fork::new();
    let env = &f.env;
    let [c1, c2, _] = three_upstream_commits(&f);
    env.git(&f.up.work, &["tag", "-a", "-m", "release 1.1", "v1.1", &c1]);
    env.git(&f.up.work, &["tag", "v1.2", &c2]);
    env.git(&f.up.work, &["push", "-q", "origin", "v1.1", "v1.2"]);
    // The repository's own tag of the same name does not stand in for the
    // upstream's.
    env.git(&f.down, &["tag", "v1.1", "main"]);

    f.tool(&["start", "upstream=v1.1"])
        .success()
        .stdout(contains(format!(
            "upstream: queued at {} (v1.1)",
            &c1[..12]
        )));
    f.tool(&["verify"]).success();
    f.tool(&["finish"]).success();
    assert_eq!(env.rev(&f.down, "main^2"), c1);
    assert_eq!(env.rev(&f.down, "upstream-main"), c1);

    // A lightweight tag works the same way.
    f.tool(&["start", "upstream=v1.2"]).success();
    f.tool(&["verify"]).success();
    f.tool(&["finish"]).success();
    assert_eq!(env.rev(&f.down, "upstream-main"), c2);

    // Upstream tags never enter the repository.
    let tags = env.git(&f.down, &["tag", "--list"]);
    assert_eq!(tags, "v1.1", "{tags}");
    assert!(!env.has_rev(&f.down, "refs/tags/v1.2"));
}

#[test]
fn revision_outside_the_upstream_branch_is_refused() {
    let f = Fork::new();
    let env = &f.env;
    let [c1, _, _] = three_upstream_commits(&f);
    // A side branch of the upstream that its main branch never merged.
    env.git(&f.up.work, &["switch", "-q", "-c", "feature", &c1]);
    let side = env.commit(&f.up.work, &[("side.txt", "s\n")], "upstream: side");
    env.git(&f.up.work, &["tag", "side-tag"]);
    env.git(&f.up.work, &["push", "-q", "origin", "feature", "side-tag"]);
    let local = env.commit(&f.down, &[("local.txt", "l\n")], "fork: local");
    env.git(&f.down, &["push", "-q", "origin", "main"]);

    let refused = f.tool(&["start", "upstream=upstream/feature"]).code(2);
    let err = stderr(&refused);
    assert!(
        err.contains(&format!(
            "upstream 'upstream': 'upstream/feature' at {} is not in the history of upstream/main as fetched",
            &side[..12]
        )),
        "{err}"
    );
    f.tool(&["start", "upstream=side-tag"])
        .code(2)
        .stderr(contains("tag 'side-tag' of remote 'upstream' at"))
        .stderr(contains("is not in the history of upstream/main"));
    f.tool(&["start", &format!("upstream={local}")])
        .code(2)
        .stderr(contains("is not in the history of upstream/main"));
    f.tool(&["start", "upstream=no-such-rev"])
        .code(2)
        .stderr(contains(
            "upstream 'upstream': 'no-such-rev' names no commit and no tag of remote 'upstream'",
        ));
    env.assert_no_update(&f.down);
    assert_eq!(env.rev(&f.down, "main"), local);
}

#[test]
fn target_already_taken_in_is_current_and_an_older_one_is_refused() {
    let f = Fork::new();
    let env = &f.env;
    let [c1, c2, _] = three_upstream_commits(&f);
    f.tool(&["start", &format!("upstream={c2}")]).success();
    f.tool(&["verify"]).success();
    f.tool(&["finish"]).success();
    let main = env.rev(&f.down, "main");

    f.tool(&["start", &format!("upstream={c2}")])
        .success()
        .stdout(contains(format!(
            "upstream: current at {} ({c2})",
            &c2[..12]
        )))
        .stdout(contains(
            "every selected upstream is current; nothing to update",
        ));
    env.assert_no_update(&f.down);

    f.tool(&["start", &format!("upstream={c1}")])
        .code(2)
        .stderr(contains(format!(
            "the target {} ({c1}) is older than the last synced commit {}; an update never moves an upstream back",
            &c1[..12],
            &c2[..12]
        )));
    env.assert_no_update(&f.down);
    assert_eq!(env.rev(&f.down, "main"), main);
}

#[test]
fn target_beside_the_last_sync_is_refused() {
    let f = Fork::new();
    let env = &f.env;
    let fork_point = env.rev(&f.up.work, "HEAD");
    let c1 = env.push_upstream(&f.up, &[("c1.txt", "1\n")], "upstream: c1");
    f.tool(&["start"]).success();
    f.tool(&["verify"]).success();
    f.tool(&["finish"]).success();

    // A side branch that forked before the last sync is merged afterwards.
    env.git(&f.up.work, &["switch", "-q", "-c", "side", &fork_point]);
    let side = env.commit(&f.up.work, &[("side.txt", "s\n")], "upstream: side");
    env.git(&f.up.work, &["switch", "-q", "main"]);
    env.git(&f.up.work, &["merge", "-q", "--no-ff", "--no-edit", "side"]);
    env.git(&f.up.work, &["push", "-q", "origin", "main"]);

    f.tool(&["start", &format!("upstream={side}")])
        .code(2)
        .stderr(contains(format!(
            "does not contain the last synced commit {}",
            &c1[..12]
        )));
    env.assert_no_update(&f.down);
}

#[test]
fn conflict_at_a_chosen_target_resumes_and_records_that_target() {
    let f = Fork::new();
    let env = &f.env;
    env.commit(
        &f.down,
        &[(FORK_FILE, "one\nTWO-fork\nthree\n")],
        "fork: change two",
    );
    env.git(&f.down, &["push", "-q", "origin", "main"]);
    let c1 = env.push_upstream(
        &f.up,
        &[(FORK_FILE, "one\nTWO-upstream\nthree\n")],
        "upstream: change two",
    );
    env.git(&f.up.work, &["tag", "v2.0"]);
    env.git(&f.up.work, &["push", "-q", "origin", "v2.0"]);
    let later = env.push_upstream(&f.up, &[("later.txt", "l\n")], "upstream: later");

    let start = f.tool(&["start", "upstream=v2.0"]).code(1);
    assert!(
        stdout(&start).contains("upstream: conflicts merging upstream/main"),
        "{}",
        stdout(&start)
    );
    // A new shell sees the chosen target.
    f.tool(&["status"]).success().stdout(contains(format!(
        "upstream: target {} (v2.0), in conflict",
        &c1[..12]
    )));

    // The tag moves on the remote during the update; the target stays the
    // commit `start` resolved.
    env.git(&f.up.work, &["tag", "-f", "v2.0", &later]);
    env.git(&f.up.work, &["push", "-q", "-f", "origin", "v2.0"]);

    let wt = env.update_worktree(&f.down);
    std::fs::write(wt.join(FORK_FILE), "one\nTWO-both\nthree\n").unwrap();
    env.git(&wt, &["add", FORK_FILE]);
    f.tool(&["continue"])
        .success()
        .stdout(contains("upstream: merged as"));
    assert!(!wt.join("later.txt").exists());
    f.tool(&["verify"]).success();
    f.tool(&["finish"]).success();
    env.assert_no_update(&f.down);
    assert_eq!(env.rev(&f.down, "main^2"), c1);
    let msg = env.git(&f.down, &["log", "-1", "--format=%B", "main"]);
    assert!(msg.contains(&format!("Intake-Target: {c1}")), "{msg}");
    assert_eq!(env.rev(&f.down, "upstream-main"), c1);
    assert!(!env.is_ancestor(&f.down, &later, "main"));
    let report = env.tool_json(&f.down, &["report", "--json"], 0);
    assert_eq!(report["upstreams"][0]["rev"], "v2.0");
    assert_eq!(
        report["upstreams"][0]["conflicts"][0]["resolution"],
        "manual"
    );
}

#[test]
fn abort_and_retry_keep_the_chosen_target() {
    let f = Fork::new();
    let env = &f.env;
    let [c1, _, _] = three_upstream_commits(&f);
    f.tool(&["start", &format!("upstream={c1}")]).success();
    f.tool(&["abort"]).success();
    env.assert_no_update(&f.down);
    assert!(!env.has_rev(&f.down, "refs/heads/upstream-main"));
    f.tool(&["start", &format!("upstream={c1}")]).success();
    f.tool(&["verify"]).success();
    f.tool(&["finish"]).success();
    assert_eq!(env.rev(&f.down, "upstream-main"), c1);
}

#[test]
fn several_upstreams_each_at_their_own_point() {
    let m = Mono::new();
    let env = &m.env;
    m.import_all();
    let a1 = env.push_upstream(&m.a, &[("a.txt", "a1\na2\na3\na4\n")], "app-a: a4");
    let a2 = env.push_upstream(&m.a, &[("a.txt", "a1\na2\na3\na4\na5\n")], "app-a: a5");
    let b1 = env.push_upstream(&m.b, &[("b.txt", "b1\nb2\nb3\nb4\n")], "app-b: b4");

    m.tool(&["start", &format!("app-a={a1}"), "app-b"])
        .success();
    m.tool(&["verify"]).success();
    m.tool(&["finish"]).success();
    let status = env.tool_json(&m.down, &["status", "--json"], 0);
    let ups = status["upstreams"].as_array().unwrap();
    assert_eq!(ups[0]["last_synced"], a1.as_str());
    assert_eq!(ups[0]["tip"], a2.as_str());
    assert_eq!(ups[0]["state"], "behind");
    assert_eq!(ups[0]["commits_behind"], 1);
    assert_eq!(ups[1]["last_synced"], b1.as_str());
    assert_eq!(ups[1]["state"], "current");
    assert_eq!(env.rev(&m.down, "mirror-a"), a1);
    assert_eq!(
        std::fs::read_to_string(m.down.join("apps/a/a.txt")).unwrap(),
        "a1\na2\na3\na4\n"
    );
}

#[test]
fn malformed_or_repeated_upstream_operands_are_refused() {
    let f = Fork::new();
    f.tool(&["start", "upstream="])
        .code(2)
        .stderr(contains("the revision after '=' is empty"));
    f.tool(&["start", "=v1"])
        .code(2)
        .stderr(contains("the upstream name before '=' is empty"));
    f.tool(&["start", "upstream", "upstream=v1"])
        .code(2)
        .stderr(contains("named more than once: upstream"));
    f.tool(&["start", "nope=v1"])
        .code(2)
        .stderr(contains("unknown upstream 'nope'"));
    f.env.assert_no_update(&f.down);
}

#[test]
fn tag_prefixes_name_the_upstream_tag() {
    let f = Fork::new();
    let env = &f.env;
    let [c1, c2, _] = three_upstream_commits(&f);
    env.git(&f.up.work, &["tag", "-a", "-m", "release 1.1", "v1.1", &c1]);
    env.git(&f.up.work, &["tag", "v1.2", &c2]);
    env.git(&f.up.work, &["push", "-q", "origin", "v1.1", "v1.2"]);
    // The repository's own tags of the same names point elsewhere.
    env.git(&f.down, &["tag", "v1.1", "main"]);
    env.git(&f.down, &["tag", "v1.2", "main"]);

    f.tool(&["start", "upstream=refs/tags/v1.1"])
        .success()
        .stdout(contains(format!(
            "upstream: queued at {} (refs/tags/v1.1)",
            &c1[..12]
        )));
    f.tool(&["abort"]).success();
    f.tool(&["start", "upstream=tags/v1.2"])
        .success()
        .stdout(contains(format!(
            "upstream: queued at {} (tags/v1.2)",
            &c2[..12]
        )));
}

#[test]
fn own_tag_does_not_stand_in_for_a_missing_upstream_tag() {
    let f = Fork::new();
    let env = &f.env;
    let [c1, c2, _] = three_upstream_commits(&f);
    // The repository tags an upstream commit; the upstream has no such tag.
    env.git(&f.down, &["fetch", "-q", "upstream"]);
    env.git(&f.down, &["tag", "v2.3", &c2]);
    env.git(&f.down, &["tag", "-a", "-m", "own", "own-1", &c1]);

    for rev in ["v2.3", "refs/tags/v2.3", "tags/v2.3", "v2.3~1", "own-1"] {
        let refused = f.tool(&["start", &format!("upstream={rev}")]).code(2);
        let err = stderr(&refused);
        assert!(
            err.contains("resolves through the repository's own tag refs/tags/"),
            "{rev}: {err}"
        );
        assert!(err.contains("remote 'upstream' has no tag"), "{rev}: {err}");
    }
    env.assert_no_update(&f.down);
}

#[test]
fn mirror_ahead_of_the_target_is_refused_at_start() {
    let f = Fork::new();
    let env = &f.env;
    let [_, c2, c3] = three_upstream_commits(&f);
    env.git(&f.down, &["fetch", "-q", "upstream"]);
    env.git(&f.down, &["branch", "upstream-main", &c3]);

    f.tool(&["start", &format!("upstream={c2}")])
        .code(2)
        .stderr(contains(format!(
            "upstream 'upstream': the mirror branch 'upstream-main' at {} is not contained in the target {}",
            &c3[..12],
            &c2[..12]
        )));
    env.assert_no_update(&f.down);
    assert_eq!(env.rev(&f.down, "upstream-main"), c3);

    // A target that contains the mirror is taken.
    f.tool(&["start", &format!("upstream={c3}")]).success();
}

#[test]
fn revision_expression_on_the_fetched_branch_is_resolved() {
    let f = Fork::new();
    let env = &f.env;
    let [_, c2, _] = three_upstream_commits(&f);
    // `start` fetches first, so the expression sees the new tip.
    f.tool(&["start", "upstream=upstream/main~1"])
        .success()
        .stdout(contains(format!(
            "upstream: queued at {} (upstream/main~1)",
            &c2[..12]
        )));
    f.tool(&["verify"]).success();
    f.tool(&["finish"]).success();
    assert_eq!(env.rev(&f.down, "main^2"), c2);
}

#[test]
fn first_import_of_a_path_upstream_at_a_chosen_revision() {
    let m = Mono::new();
    let env = &m.env;
    // Only app-a is imported, so only its check runs.
    let cfg = fs::read_to_string(m.down.join(".git-intake.toml"))
        .unwrap()
        .replace(", \"test -f apps/b/b.txt\"", "");
    env.commit(
        &m.down,
        &[(".git-intake.toml", &cfg)],
        "mono: check app-a only",
    );
    let a0 = env.rev(&m.a.work, "HEAD");
    let a1 = env.push_upstream(&m.a, &[("a.txt", "a1\na2\na3\na4\n")], "app-a: a4");

    m.tool(&["start", &format!("app-a={a0}")])
        .success()
        .stdout(contains(format!("app-a: queued at {} ({a0})", &a0[..12])));
    let status = env.tool_json(&m.down, &["status", "--json"], 0);
    assert_eq!(status["update"]["upstreams"][0]["kind"], "import");
    m.tool(&["verify"]).success();
    m.tool(&["finish"]).success();
    assert_eq!(
        fs::read_to_string(m.down.join("apps/a/a.txt")).unwrap(),
        "a1\na2\na3\n"
    );
    assert_eq!(env.rev(&m.down, "mirror-a"), a0);
    let status = env.tool_json(&m.down, &["status", "--json"], 0);
    let a = &status["upstreams"][0];
    assert_eq!(a["last_synced"], a0.as_str());
    assert_eq!(a["tip"], a1.as_str());
    assert_eq!(a["state"], "behind");
    assert_eq!(a["commits_behind"], 1);
}

#[test]
fn unreachable_remote_during_tag_lookup_is_refused_clearly() {
    let f = Fork::new();
    let env = &f.env;
    let [c1, _, _] = three_upstream_commits(&f);
    // The upstream's remote serves the fetch of `start`, then fails.
    let count = env.path("upload-pack-count");
    let script = env.path("upload-pack.sh");
    fs::write(
        &script,
        format!(
            "#!/bin/sh\nn=$(cat {count:?} 2>/dev/null || echo 0)\necho $((n + 1)) > {count:?}\n\
             [ \"$n\" -ge 1 ] && {{ echo 'remote unavailable' >&2; exit 1; }}\nexec git-upload-pack \"$@\"\n"
        ),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    env.git(
        &f.down,
        &[
            "config",
            "remote.upstream.uploadpack",
            &script.to_string_lossy(),
        ],
    );

    let refused = f.tool(&["start", &format!("upstream={c1}")]).code(2);
    let err = stderr(&refused);
    assert!(
        err.contains(&format!(
            "upstream 'upstream': cannot ask remote 'upstream' whether it has a tag '{c1}'"
        )),
        "{err}"
    );
    env.assert_no_update(&f.down);
}
