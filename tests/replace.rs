//! `sync` replacing an untouched update merge request with an update at a
//! newer release, with `replace_after` set. The forge is a fake one; see
//! `common::forge`.

mod common;

use std::fs;
use std::path::Path;

use common::forge::{Action, FakeForge, Kind, Step, branch_tip};
use common::{
    Fork, How, MERGE_REQUEST_BRANCH, Mono, PAUSE_REF, dead_pause, merge_on_fake, stderr, stdout,
    survives_each_failed_request,
};

fn on_remote(f: &Fork, branch: &str) -> Option<String> {
    branch_tip(&f.publish, branch)
}

/// Lands a commit on the publish remote's integration branch, as other work
/// does; returns it.
fn land(f: &Fork, files: &[(&str, &str)], message: &str) -> String {
    let dir = common::forge(&f.env, &f.publish);
    let c = f.env.commit(&dir, files, message);
    f.env.git(&dir, &["push", "-q", "origin", "HEAD:main"]);
    c
}

/// Commits `files` on the upstream and tags the commit `tag`; returns it.
fn release(f: &Fork, tag: &str, files: &[(&str, &str)]) -> String {
    let c = f
        .env
        .push_upstream(&f.up, files, &format!("upstream: {tag}"));
    f.env.git(&f.up.work, &["tag", tag, &c]);
    f.env.git(&f.up.work, &["push", "-q", "origin", tag]);
    c
}

/// A fork on `kind` whose upstream is taken at its `v*` releases, with
/// `replace_after = "7d"`.
fn replacing(kind: Kind) -> Fork {
    let f = match kind {
        Kind::GitLab => Fork::merge_request(),
        Kind::GitHub => Fork::github(),
    };
    f.reconfigure("mode = \"follow\"\n", "mode = \"follow\"\ntags = \"v*\"\n");
    f.reconfigure(
        "via = \"merge-request\"\n",
        "via = \"merge-request\"\nreplace_after = \"7d\"\n",
    );
    // A run that fails on what may pass goes again at once.
    f.env.set_var("GIT_INTAKE_TEST_RETRY_GAP_MS", "0");
    f
}

/// [`replacing`] with the update's merge request !1 opened by `sync` at the
/// release `v1.2.0`; returns that release.
fn opened(kind: Kind) -> (Fork, String) {
    let f = replacing(kind);
    let v12 = release(&f, "v1.2.0", &[("v12.txt", "1.2\n")]);
    f.tool(&["sync"]).success();
    assert_eq!(common::open_number(f.forge()), 1);
    f.forge().clear_requests();
    (f, v12)
}

/// The merge request as messages name it on `kind`.
fn mr(kind: Kind, n: u64) -> String {
    match kind {
        Kind::GitLab => format!("merge request !{n}"),
        Kind::GitHub => format!("pull request #{n}"),
    }
}

/// Installs a hook of the repository at `repo` that runs `script` when git
/// runs it in the update worktree.
fn in_update_worktree_hook(repo: &Path, name: &str, script: &str) {
    let path = repo.join(".git").join("hooks").join(name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(
        &path,
        format!(
            "#!/bin/sh\ncase \"$(git rev-parse --git-dir)\" in\n*/worktrees/*) {script} ;;\nesac\n"
        ),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
}

/// Whether the tool asked the forge for a merge request's activity.
fn read_activity(f: &Fork) -> bool {
    f.forge().requests().iter().any(|r| {
        ["/approvals", "/notes", "/reviews", "/comments"]
            .iter()
            .any(|p| r.path.split('?').next().unwrap_or_default().ends_with(p))
    })
}

fn replaced_after_the_wait(kind: Kind) {
    let (f, v12) = opened(kind);
    let env = &f.env;
    let old_head = on_remote(&f, MERGE_REQUEST_BRANCH).unwrap();
    let v13 = release(&f, "v1.3.0", &[("v13.txt", "1.3\n")]);
    f.forge().age(1, 8);
    f.forge().clear_requests();

    let out = stdout(&f.tool(&["sync"]).code(0));
    assert!(
        out.contains(&format!("replacing the {} (http", mr(kind, 1)))
            && out.contains("with an update at `upstream` v1.3.0"),
        "{out}"
    );
    assert!(
        out.contains(&format!("closed the {}", mr(kind, 1))),
        "{out}"
    );
    assert!(out.contains(&format!("opened {}", mr(kind, 2))), "{out}");
    assert!(!out.contains("verifying"), "{out}");

    // One request closed it and replaced its description.
    let closes: Vec<_> = f
        .forge()
        .changes()
        .into_iter()
        .filter(|r| {
            let j = r.json();
            j["state_event"] == "close" || j["state"] == "closed"
        })
        .collect();
    assert_eq!(closes.len(), 1, "{closes:?}");
    let body = closes[0].json();
    let text = body["description"]
        .as_str()
        .or(body["body"].as_str())
        .unwrap();
    assert!(text.contains("closed it without merging"), "{text}");

    let old = f.forge().mr(1);
    assert_eq!(old.state, "closed");
    assert!(old.description.contains("Nothing of it was merged"));
    assert!(
        !old.description.contains("<!-- git-intake targets"),
        "{}",
        old.description
    );
    assert_eq!(old.head.as_deref(), Some(old_head.as_str()));

    let new = f.forge().mr(2);
    assert_eq!(
        (new.state, new.source.as_str(), new.target.as_str()),
        ("open", MERGE_REQUEST_BRANCH, "main")
    );
    let reference = match kind {
        Kind::GitLab => "!1",
        Kind::GitHub => "#1",
    };
    assert!(
        new.description
            .contains(&format!("replaces the {}", mr(kind, 1))),
        "{}",
        new.description
    );
    assert!(new.description.contains(reference), "{}", new.description);
    assert!(
        new.description.contains("when `upstream` v1.3.0 came out"),
        "{}",
        new.description
    );
    assert!(new.description.contains(&v13), "{}", new.description);
    let head = on_remote(&f, MERGE_REQUEST_BRANCH).unwrap();
    assert!(env.is_ancestor(&f.publish, &v13, &head));
    assert!(env.is_ancestor(&f.publish, &v12, &head));
    assert!(!env.is_ancestor(&f.publish, &old_head, &head));
    let log = env.git(&f.publish, &["log", "--format=%B", "-1", &head]);
    assert!(log.contains("Intake-Tag: v1.3.0"), "{log}");
    env.assert_no_update(&f.down);

    // Nothing newer: the new merge request waits.
    f.forge().age(2, 8);
    f.forge().clear_requests();
    let out = stdout(&f.tool(&["sync"]).success());
    assert!(
        out.contains(&format!("the update waits for the {}", mr(kind, 2))),
        "{out}"
    );
    assert!(!out.contains("not replaced"), "{out}");
    assert!(f.forge().changes().is_empty());
    assert!(!read_activity(&f));
}

#[test]
fn an_untouched_merge_request_is_replaced_after_the_wait_on_gitlab() {
    replaced_after_the_wait(Kind::GitLab);
}

#[test]
fn an_untouched_pull_request_is_replaced_after_the_wait_on_github() {
    replaced_after_the_wait(Kind::GitHub);
}

#[test]
fn status_shows_the_setting() {
    let f = replacing(Kind::GitLab);
    let out = stdout(&f.tool(&["status"]).success());
    assert!(
        out.contains("via merge request, replacing an untouched merge request after 7d"),
        "{out}"
    );
    let json = f.env.tool_json(&f.down, &["status", "--json"], 0);
    assert_eq!(json["replace_after"], "7d");
}

#[test]
fn several_releases_collapse_into_one_replacement_at_the_newest() {
    let (f, _) = opened(Kind::GitLab);
    release(&f, "v1.3.0", &[("v13.txt", "1.3\n")]);
    let v14 = release(&f, "v1.4.0", &[("v14.txt", "1.4\n")]);
    f.forge().age(1, 30);
    f.tool(&["sync"]).success();
    assert_eq!(f.forge().mrs().len(), 2);
    let new = f.forge().mr(2);
    assert!(
        new.description.contains("when `upstream` v1.4.0 came out"),
        "{}",
        new.description
    );
    assert!(!new.description.contains("v1.3.0"), "{}", new.description);
    assert!(new.description.contains(&v14), "{}", new.description);
}

#[test]
fn a_merge_request_too_young_is_kept_and_says_from_when() {
    let (f, _) = opened(Kind::GitLab);
    let head = on_remote(&f, MERGE_REQUEST_BRANCH);
    release(&f, "v1.3.0", &[("v13.txt", "1.3\n")]);
    f.forge().age(1, 6);
    f.forge().clear_requests();
    let out = stdout(&f.tool(&["sync"]).success());
    assert!(
        out.contains("the merge request !1 (http")
            && out.contains("is not replaced: `upstream` v1.3.0 came out"),
        "{out}"
    );
    assert!(out.contains("may be replaced from"), "{out}");
    assert!(out.contains("`replace_after` being 7d"), "{out}");
    assert!(
        out.contains("the update waits for the merge request !1"),
        "{out}"
    );
    assert!(out.contains("upstream: moved on to"), "{out}");
    assert!(f.forge().changes().is_empty());
    assert!(!read_activity(&f));
    assert_eq!(on_remote(&f, MERGE_REQUEST_BRANCH), head);
    f.env.assert_no_update(&f.down);
}

#[test]
fn each_trace_of_a_person_on_gitlab_keeps_the_merge_request() {
    let (f, _) = opened(Kind::GitLab);
    let fake = f.forge();
    // Catch-ups by the tool leave its merge request untouched: their
    // merges are on the branch, their description changes are system notes.
    land(&f, &[("other.txt", "o\n")], "other: lands");
    let out = stdout(&f.tool(&["sync"]).success());
    assert!(out.contains("caught up with 'main'"), "{out}");
    // More system notes than one page holds.
    for i in 0..150 {
        fake.add_event(1, &format!("added {i} commits"));
    }
    release(&f, "v1.3.0", &[("v13.txt", "1.3\n")]);
    fake.age(1, 8);
    let head = on_remote(&f, MERGE_REQUEST_BRANCH);

    type Change = fn(&FakeForge);
    let traces: [(Change, Change, &str); 8] = [
        (
            |fake| fake.edit(1, |m| m.reviewers.push(7)),
            |fake| fake.edit(1, |m| m.reviewers.clear()),
            "someone is asked to review it",
        ),
        (
            |fake| fake.edit(1, |m| m.reactions.push("thumbsup".into())),
            |fake| fake.edit(1, |m| m.reactions.clear()),
            "someone reacted to it",
        ),
        (
            |fake| {
                fake.edit(1, |m| {
                    m.merge_after = Some("2026-12-01T09:00:00.000Z".into())
                })
            },
            |fake| fake.edit(1, |m| m.merge_after = None),
            "someone scheduled its merge with `merge_after`",
        ),
        (
            |fake| fake.add_note(1, "Should we wait for 1.4?"),
            |fake| fake.remove_comments(1),
            "someone commented on it",
        ),
        (
            |fake| fake.edit(1, |m| m.approvals.push("reviewer".into())),
            |fake| fake.edit(1, |m| m.approvals.clear()),
            "someone approved it",
        ),
        (
            |fake| fake.edit(1, |m| m.assignees.push(7)),
            |fake| fake.edit(1, |m| m.assignees.clear()),
            "someone is assigned to it",
        ),
        (
            |fake| fake.edit(1, |m| m.draft = true),
            |fake| fake.edit(1, |m| m.draft = false),
            "it is marked as a draft",
        ),
        (
            |fake| fake.edit(1, |m| m.auto_merge = true),
            |fake| fake.edit(1, |m| m.auto_merge = false),
            "it is set to merge automatically",
        ),
    ];
    for (touch, untouch, why) in traces {
        touch(fake);
        fake.clear_requests();
        let out = stdout(&f.tool(&["sync"]).success());
        assert!(
            out.contains(&format!("is not replaced: {why}")),
            "{why}: {out}"
        );
        assert!(
            out.contains("the update waits for the merge request !1"),
            "{out}"
        );
        assert!(fake.changes().is_empty(), "{why}: {:?}", fake.changes());
        assert_eq!(on_remote(&f, MERGE_REQUEST_BRANCH), head);
        f.env.assert_no_update(&f.down);
        untouch(fake);
    }
    // The system notes span pages; with every trace gone it is replaced.
    fake.clear_requests();
    let out = stdout(&f.tool(&["sync"]).success());
    assert!(out.contains("opened merge request !2"), "{out}");
    assert!(
        fake.requests()
            .iter()
            .any(|r| r.path.contains("/notes") && r.path.contains("page=2")),
        "the notes were read page by page"
    );
    assert_eq!(fake.mr(1).state, "closed");
}

#[test]
fn each_trace_of_a_person_on_github_keeps_the_pull_request() {
    let (f, _) = opened(Kind::GitHub);
    let fake = f.forge();
    release(&f, "v1.3.0", &[("v13.txt", "1.3\n")]);
    fake.age(1, 8);
    type Change = fn(&FakeForge);
    let traces: [(Change, Change, &str); 9] = [
        (
            |fake| fake.edit(1, |m| m.reviewers.push(7)),
            |fake| fake.edit(1, |m| m.reviewers.clear()),
            "someone is asked to review it",
        ),
        (
            |fake| fake.edit(1, |m| m.requested_teams.push("maintainers".into())),
            |fake| fake.edit(1, |m| m.requested_teams.clear()),
            "someone is asked to review it",
        ),
        (
            |fake| fake.edit(1, |m| m.reactions.push("+1".into())),
            |fake| fake.edit(1, |m| m.reactions.clear()),
            "someone reacted to it",
        ),
        (
            |fake| fake.edit(1, |m| m.reviews.push("COMMENTED".into())),
            |fake| fake.edit(1, |m| m.reviews.clear()),
            "someone reviewed it",
        ),
        (
            |fake| fake.edit(1, |m| m.reviews.push("DISMISSED".into())),
            |fake| fake.edit(1, |m| m.reviews.clear()),
            "someone reviewed it",
        ),
        (
            |fake| fake.add_note(1, "ping"),
            |fake| fake.remove_comments(1),
            "someone commented on it",
        ),
        (
            |fake| fake.edit(1, |m| m.assignees.push(7)),
            |fake| fake.edit(1, |m| m.assignees.clear()),
            "someone is assigned to it",
        ),
        (
            |fake| fake.edit(1, |m| m.draft = true),
            |fake| fake.edit(1, |m| m.draft = false),
            "it is marked as a draft",
        ),
        (
            |fake| fake.edit(1, |m| m.auto_merge = true),
            |fake| fake.edit(1, |m| m.auto_merge = false),
            "it is set to merge automatically",
        ),
    ];
    for (touch, untouch, why) in traces {
        touch(fake);
        fake.clear_requests();
        let out = stdout(&f.tool(&["sync"]).success());
        assert!(
            out.contains(&format!("is not replaced: {why}")),
            "{why}: {out}"
        );
        assert!(fake.changes().is_empty(), "{why}: {:?}", fake.changes());
        untouch(fake);
    }
    let out = stdout(&f.tool(&["sync"]).success());
    assert!(out.contains("opened pull request #2"), "{out}");
}

#[test]
fn what_the_token_cannot_read_on_github_counts_as_touched() {
    let (f, _) = opened(Kind::GitHub);
    release(&f, "v1.3.0", &[("v13.txt", "1.3\n")]);
    f.forge().age(1, 8);
    for (suffix, what) in [
        ("/reviews", "its reviews could not be read"),
        ("/comments", "its comments could not be read"),
        ("/reactions", "its reactions could not be read"),
    ] {
        f.forge()
            .fail("GET", suffix, 403, "Resource not accessible by integration");
        let out = stdout(&f.tool(&["sync"]).success());
        assert!(out.contains(what), "{out}");
        assert!(
            out.contains("so the pull request counts as touched"),
            "{out}"
        );
        assert!(f.forge().changes().is_empty(), "{:?}", f.forge().changes());
        f.forge().world().failures.clear();
    }
}

#[test]
fn what_the_token_cannot_read_counts_as_touched() {
    let (f, _) = opened(Kind::GitLab);
    release(&f, "v1.3.0", &[("v13.txt", "1.3\n")]);
    f.forge().age(1, 8);
    for (suffix, status, what) in [
        ("/approvals", 403, "its approvals could not be read"),
        ("/notes", 401, "its comments could not be read"),
        ("/award_emoji", 403, "its reactions could not be read"),
    ] {
        f.forge().fail("GET", suffix, status, "insufficient scope");
        let out = stdout(&f.tool(&["sync"]).success());
        assert!(out.contains(what), "{out}");
        assert!(
            out.contains(&format!("with {status} insufficient scope")),
            "{out}"
        );
        assert!(
            out.contains("so the merge request counts as touched"),
            "{out}"
        );
        assert!(f.forge().changes().is_empty(), "{:?}", f.forge().changes());
        f.forge().world().failures.clear();
    }
}

/// Pushes `commit` made in a clone of the publish remote onto the merge
/// request's branch.
fn push_onto_branch(f: &Fork, files: &[(&str, &str)], message: &str) {
    let dir = common::forge(&f.env, &f.publish);
    f.env.git(
        &dir,
        &[
            "checkout",
            "-q",
            "-B",
            "fix",
            &format!("origin/{MERGE_REQUEST_BRANCH}"),
        ],
    );
    f.env.commit(&dir, files, message);
    f.env.git(
        &dir,
        &[
            "push",
            "-q",
            "origin",
            &format!("HEAD:{MERGE_REQUEST_BRANCH}"),
        ],
    );
    f.env.git(&dir, &["checkout", "-q", "main"]);
}

#[test]
fn commits_on_the_branch_keep_the_merge_request() {
    // A fix someone pushed, and a commit carrying the trailers of a port
    // upstream's copy, which only a port update makes.
    for (file, message) in [
        ("fix.txt", "fix: the update needs this".to_string()),
        (
            "copy.txt",
            format!(
                "port: copy\n\nIntake-Upstream: upstream\nIntake-Taken: {}\n",
                "a".repeat(40)
            ),
        ),
    ] {
        let (f, _) = opened(Kind::GitLab);
        push_onto_branch(&f, &[(file, "x\n")], &message);
        release(&f, "v1.3.0", &[("v13.txt", "1.3\n")]);
        f.forge().age(1, 8);
        f.forge().clear_requests();
        let out = stdout(&f.tool(&["sync"]).success());
        assert!(
            out.contains("is neither a sync merge nor a catch-up merge"),
            "{out}"
        );
        assert!(f.forge().changes().is_empty());
        assert!(!read_activity(&f));
        assert_eq!(f.forge().mrs().len(), 1);
    }
}

#[test]
fn a_merge_by_the_forge_s_update_button_keeps_the_merge_request() {
    let (f, _) = opened(Kind::GitLab);
    let env = &f.env;
    land(&f, &[("other.txt", "o\n")], "other: lands");
    // The forge merges the target branch into the source branch.
    let dir = common::forge(env, &f.publish);
    env.git(
        &dir,
        &[
            "checkout",
            "-q",
            "-B",
            "button",
            &format!("origin/{MERGE_REQUEST_BRANCH}"),
        ],
    );
    env.git(
        &dir,
        &[
            "merge",
            "-q",
            "--no-ff",
            "-m",
            "Merge branch 'main' into 'git-intake/update'",
            "origin/main",
        ],
    );
    env.git(
        &dir,
        &[
            "push",
            "-q",
            "origin",
            &format!("HEAD:{MERGE_REQUEST_BRANCH}"),
        ],
    );
    env.git(&dir, &["checkout", "-q", "main"]);
    release(&f, "v1.3.0", &[("v13.txt", "1.3\n")]);
    f.forge().age(1, 8);
    f.forge().clear_requests();
    let out = stdout(&f.tool(&["sync"]).success());
    assert!(
        out.contains("is neither a sync merge nor a catch-up merge"),
        "{out}"
    );
    assert!(
        out.contains("the update waits for the merge request !1"),
        "{out}"
    );
    assert!(f.forge().changes().is_empty());
}

/// Publishes, by `start` naming `rev` for the upstream, `verify` and
/// `finish`, merge request !1, then lets a newer release appear and the
/// merge request age past `replace_after`; returns what `sync` says.
fn chosen_by_a_person(f: &Fork, rev: &str) -> String {
    f.tool(&["start", &format!("upstream={rev}")]).success();
    f.tool(&["verify"]).success();
    f.tool(&["finish"]).success();
    assert_eq!(common::open_number(f.forge()), 1);
    f.forge().clear_requests();
    release(f, "v1.3.0", &[("v13.txt", "1.3\n")]);
    f.forge().age(1, 8);
    let out = stdout(&f.tool(&["sync"]).success());
    assert!(f.forge().changes().is_empty(), "{:?}", f.forge().changes());
    out
}

#[test]
fn a_revision_chosen_by_a_person_keeps_the_merge_request() {
    let f = replacing(Kind::GitLab);
    release(&f, "v1.2.0", &[("v12.txt", "1.2\n")]);
    let chosen = f
        .env
        .push_upstream(&f.up, &[("dev.txt", "d\n")], "upstream: dev");
    let out = chosen_by_a_person(&f, &chosen);
    assert!(
        out.contains("does not record that git intake chose its target"),
        "{out}"
    );
    let head = on_remote(&f, MERGE_REQUEST_BRANCH).unwrap();
    let log = f.env.git(&f.publish, &["log", "--format=%B", "-1", &head]);
    assert!(!log.contains("Intake-Chosen"), "{log}");
}

#[test]
fn a_release_a_person_named_keeps_the_merge_request() {
    // `start upstream=v1.2.0` names the release the rules would choose as
    // well, and its sync record names the tag, but a person chose it.
    let f = replacing(Kind::GitLab);
    release(&f, "v1.2.0", &[("v12.txt", "1.2\n")]);
    let out = chosen_by_a_person(&f, "v1.2.0");
    assert!(
        out.contains(
            "does not record that git intake chose its target, so a person named that revision"
        ),
        "{out}"
    );
    let head = on_remote(&f, MERGE_REQUEST_BRANCH).unwrap();
    let log = f.env.git(&f.publish, &["log", "--format=%B", "-1", &head]);
    assert!(log.contains("Intake-Tag: v1.2.0"), "{log}");
    assert!(!log.contains("Intake-Chosen"), "{log}");
}

#[test]
fn the_sync_record_names_the_rule_that_chose_the_target() {
    let (f, _) = opened(Kind::GitLab);
    let head = on_remote(&f, MERGE_REQUEST_BRANCH).unwrap();
    let log = f.env.git(&f.publish, &["log", "--format=%B", "-1", &head]);
    assert!(log.contains("Intake-Chosen: newest-release"), "{log}");
}

/// A monorepo whose `app-a` is taken at its `v*` releases and `app-b` at
/// its branch tip, with `replace_after = "7d"`, and the update's merge
/// request !1 opened by `sync`.
fn mono() -> Mono {
    let m = Mono::merge_request();
    let env = &m.env;
    let config = fs::read_to_string(m.down.join(".git-intake.toml"))
        .unwrap()
        .replace("path = \"apps/a\"\n", "path = \"apps/a\"\ntags = \"v*\"\n")
        .replace(
            "via = \"merge-request\"\n",
            "via = \"merge-request\"\nreplace_after = \"7d\"\n",
        );
    env.commit(&m.down, &[(".git-intake.toml", &config)], "mono: replace");
    env.git(&m.down, &["push", "-q", "origin", "main"]);
    let a = env.rev(&m.a.work, "HEAD");
    env.git(&m.a.work, &["tag", "v1.0.0", &a]);
    env.git(&m.a.work, &["push", "-q", "origin", "v1.0.0"]);
    m.tool(&["sync"]).success();
    assert_eq!(common::open_number(m.forge.as_ref().unwrap()), 1);
    m
}

#[test]
fn an_upstream_without_tags_never_triggers_a_replacement() {
    let m = mono();
    let env = &m.env;
    let fake = m.forge.as_ref().unwrap();
    env.push_upstream(&m.b, &[("b2.txt", "b\n")], "app-b: moves on");
    fake.age(1, 8);
    fake.clear_requests();
    let out = stdout(&m.tool(&["sync"]).success());
    assert!(out.contains("app-b: moved on to"), "{out}");
    assert!(
        out.contains("only a newer release of an upstream with `tags` replaces it"),
        "{out}"
    );
    assert!(fake.changes().is_empty());
    assert_eq!(fake.mrs().len(), 1);
}

#[test]
fn a_left_out_upstream_blocks_the_replacement() {
    let m = mono();
    let env = &m.env;
    let fake = m.forge.as_ref().unwrap();
    let c = env.push_upstream(&m.a, &[("a2.txt", "a\n")], "app-a: 1.1");
    env.git(&m.a.work, &["tag", "v1.1.0", &c]);
    env.git(&m.a.work, &["push", "-q", "origin", "v1.1.0"]);
    fs::rename(&m.b.bare, env.path("gone.git")).unwrap();
    fake.age(1, 8);
    fake.clear_requests();
    // An upstream left out makes the run's status 1, as it does without an
    // open merge request.
    let out = stdout(&m.tool(&["sync"]).code(1));
    assert!(
        out.contains("left out: upstream 'app-b' cannot be fetched"),
        "{out}"
    );
    assert!(
        out.contains("sync left out the follow upstreams named above"),
        "{out}"
    );
    assert!(fake.changes().is_empty());
    assert_eq!(fake.mrs().len(), 1);
}

/// [`opened`] on a fork whose integration branch changed `app.txt` before
/// the merge request was opened, and a release `v1.3.0` that changes the
/// same line, so that its merge into the integration tip conflicts.
fn conflicting() -> Fork {
    let f = replacing(Kind::GitLab);
    land(&f, &[("app.txt", "fork\ntwo\nthree\n")], "fork: change");
    release(&f, "v1.2.0", &[("v12.txt", "1.2\n")]);
    f.tool(&["sync"]).success();
    release(&f, "v1.3.0", &[("app.txt", "upstream\ntwo\nthree\n")]);
    f.forge().age(1, 8);
    f
}

#[test]
fn a_replacement_that_conflicts_replaces_nothing() {
    let f = conflicting();
    let head = on_remote(&f, MERGE_REQUEST_BRANCH);
    f.forge().clear_requests();
    let out = stdout(&f.tool(&["sync"]).code(1));
    assert!(
        out.contains("the replacement of the merge request !1"),
        "{out}"
    );
    assert!(out.contains("upstream at"), "{out}");
    assert!(out.contains("(v1.3.0)"), "{out}");
    assert!(out.contains("in `app.txt`"), "{out}");
    assert!(out.contains("`git intake abort --merge-request`"), "{out}");
    assert!(!out.contains("git intake continue"), "{out}");
    assert!(
        out.contains("the update waits for the merge request !1"),
        "{out}"
    );
    assert!(f.forge().changes().is_empty(), "{:?}", f.forge().changes());
    assert_eq!(on_remote(&f, MERGE_REQUEST_BRANCH), head);
    assert_eq!(f.forge().mr(1).state, "open");
    f.env.assert_no_update(&f.down);
}

#[test]
fn a_conflicting_replacement_still_catches_the_merge_request_up() {
    let f = conflicting();
    let before = on_remote(&f, MERGE_REQUEST_BRANCH).unwrap();
    let tip = land(&f, &[("other.txt", "o\n")], "other: lands");
    let out = stdout(&f.tool(&["sync"]).code(1));
    assert!(
        out.contains("the replacement of the merge request !1"),
        "{out}"
    );
    assert!(out.contains("caught up with 'main'"), "{out}");
    let after = on_remote(&f, MERGE_REQUEST_BRANCH).unwrap();
    assert_eq!(f.env.rev(&f.publish, &format!("{after}^1")), tip);
    assert_eq!(f.env.rev(&f.publish, &format!("{after}^2")), before);
    assert_eq!(f.forge().mrs().len(), 1);
    assert_eq!(f.forge().mr(1).state, "open");
    f.env.assert_no_update(&f.down);
}

#[test]
fn a_comment_during_the_replacement_stops_it() {
    let (f, _) = opened(Kind::GitLab);
    let head = on_remote(&f, MERGE_REQUEST_BRANCH);
    release(&f, "v1.3.0", &[("v13.txt", "1.3\n")]);
    f.forge().age(1, 8);
    let marker = f.env.path("commented-now");
    f.forge().world().comment_when = Some((1, marker.clone()));
    in_update_worktree_hook(
        &f.down,
        "post-merge",
        &format!("touch {}", marker.display()),
    );
    let out = stdout(&f.tool(&["sync"]).code(0));
    assert!(
        out.contains("is no longer untouched: someone commented on it"),
        "{out}"
    );
    assert!(out.contains("nothing was closed or pushed"), "{out}");
    assert!(f.forge().changes().is_empty(), "{:?}", f.forge().changes());
    assert_eq!(f.forge().mr(1).state, "open");
    assert_eq!(on_remote(&f, MERGE_REQUEST_BRANCH), head);
    f.env.assert_no_update(&f.down);
}

#[test]
fn a_merge_during_the_replacement_stops_it() {
    let (f, _) = opened(Kind::GitLab);
    release(&f, "v1.3.0", &[("v13.txt", "1.3\n")]);
    f.forge().age(1, 8);
    let marker = f.env.path("merged-now");
    f.forge().world().merge_when = Some((1, marker.clone()));
    in_update_worktree_hook(
        &f.down,
        "post-merge",
        &format!("touch {}", marker.display()),
    );
    let out = stdout(&f.tool(&["sync"]).code(0));
    assert!(
        out.contains("was merged while sync merged its replacement; nothing was closed or pushed"),
        "{out}"
    );
    assert!(f.forge().changes().is_empty(), "{:?}", f.forge().changes());
    assert_eq!(f.forge().mrs().len(), 1);
    f.env.assert_no_update(&f.down);
}

#[test]
fn a_merge_request_closed_by_a_replacement_declines_nothing() {
    let (f, _) = opened(Kind::GitLab);
    let env = &f.env;
    release(&f, "v1.3.0", &[("v13.txt", "1.3\n")]);
    f.forge().age(1, 8);
    f.tool(&["sync"]).success();
    let note = f.forge().mr(1).description;
    // The replacing merge request ends as one closed by a replacement
    // would, with its targets those the next update would merge.
    f.forge().describe(2, &note);
    f.forge().end(2, "closed");
    env.git(
        &f.publish,
        &[
            "update-ref",
            "-d",
            &format!("refs/heads/{MERGE_REQUEST_BRANCH}"),
        ],
    );
    let out = stdout(&f.tool(&["sync"]).success());
    assert!(!out.contains("declined"), "{out}");
    assert!(out.contains("opened merge request !3"), "{out}");
}

#[test]
fn a_replacement_interrupted_after_the_close_is_completed_by_the_next_sync() {
    let (f, _) = opened(Kind::GitLab);
    let env = &f.env;
    let old_head = on_remote(&f, MERGE_REQUEST_BRANCH).unwrap();
    let v13 = release(&f, "v1.3.0", &[("v13.txt", "1.3\n")]);
    f.forge().age(1, 8);
    // The publish remote refuses to delete the branch, as a run killed
    // right after the close leaves it.
    let hook = f.publish.join("hooks").join("pre-receive");
    fs::create_dir_all(hook.parent().unwrap()).unwrap();
    fs::write(
        &hook,
        "#!/bin/sh\nwhile read old new ref; do\n  case $new in 0000000000000000000000000000000000000000) exit 1 ;; esac\ndone\n",
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
    // Each further attempt meets the closed merge request and its branch,
    // as the next run would, and is refused the same way.
    let out = f.tool(&["sync"]).code(2);
    let (out, err) = (stdout(&out), stderr(&out));
    assert!(
        out.contains("remains on 'origin', so the update that replaces it was not pushed")
            && out.contains("trying again"),
        "{out}"
    );
    assert!(
        out.contains("holds nothing but what landed or what a closed or merged merge request held"),
        "{out}"
    );
    assert!(err.contains("Run `git intake sync` again"), "{err}");
    assert_eq!(f.forge().mr(1).state, "closed");
    assert_eq!(f.forge().mrs().len(), 1);
    assert_eq!(on_remote(&f, MERGE_REQUEST_BRANCH), Some(old_head));
    env.assert_no_update(&f.down);

    fs::remove_file(&hook).unwrap();
    let out = stdout(&f.tool(&["sync"]).success());
    assert!(out.contains("deleting it"), "{out}");
    assert!(!out.contains("declined"), "{out}");
    assert!(out.contains("opened merge request !2"), "{out}");
    let head = on_remote(&f, MERGE_REQUEST_BRANCH).unwrap();
    assert!(env.is_ancestor(&f.publish, &v13, &head));
    // The new merge request names the one the interrupted run closed.
    let new = f.forge().mr(2).description;
    assert!(
        new.contains("replaces the merge request !1")
            && new.contains("when `upstream` v1.3.0 came out"),
        "{new}"
    );
}

#[test]
fn a_merge_request_of_an_earlier_release_s_branch_is_replaced() {
    let f = replacing(Kind::GitLab);
    let env = &f.env;
    release(&f, "v1.2.0", &[("v12.txt", "1.2\n")]);
    f.tool(&["start"]).success();
    let head = env.rev(&env.update_worktree(&f.down), "HEAD");
    f.tool(&["abort"]).success();
    let earlier = format!("git-intake/update-{}", &head[..12]);
    env.git(
        &f.down,
        &[
            "push",
            "-q",
            &f.publish.to_string_lossy(),
            &format!("{head}:refs/heads/{earlier}"),
        ],
    );
    let n = f
        .forge()
        .add(&earlier, "main", "Upstream changes verified by git intake.");
    f.forge().age(n, 8);
    release(&f, "v1.3.0", &[("v13.txt", "1.3\n")]);

    let out = stdout(&f.tool(&["sync"]).success());
    assert!(
        out.contains(&format!("deleted the branch '{earlier}'")),
        "{out}"
    );
    assert_eq!(on_remote(&f, &earlier), None);
    assert_eq!(f.forge().mr(n).state, "closed");
    let new = f.forge().mr(n + 1);
    assert_eq!(
        (new.state, new.source.as_str()),
        ("open", MERGE_REQUEST_BRANCH)
    );
}

#[test]
fn the_setting_is_refused_where_it_does_nothing() {
    // Publication by push.
    let f = Fork::new();
    f.reconfigure("mode = \"follow\"\n", "mode = \"follow\"\ntags = \"v*\"\n");
    f.reconfigure(
        "via = \"push\"\n",
        "via = \"push\"\nreplace_after = \"7d\"\n",
    );
    let err = stderr(&f.tool(&["sync"]).code(2));
    assert!(err.contains("publish replace_after: '7d'"), "{err}");
    assert!(err.contains("needs merge-request mode"), "{err}");

    // No follow upstream with `tags`, then values that are no duration.
    let f = Fork::merge_request();
    f.reconfigure(
        "via = \"merge-request\"\n",
        "via = \"merge-request\"\nreplace_after = \"7d\"\n",
    );
    let err = stderr(&f.tool(&["sync"]).code(2));
    assert!(
        err.contains(
            "only a newer release of a follow upstream with `tags` triggers a replacement"
        ),
        "{err}"
    );
    f.reconfigure("mode = \"follow\"\n", "mode = \"follow\"\ntags = \"v*\"\n");
    let mut current = "7d".to_string();
    for bad in ["0d", "-7d", "1 month", "soon"] {
        f.reconfigure(
            &format!("replace_after = \"{current}\""),
            &format!("replace_after = \"{bad}\""),
        );
        current = bad.to_string();
        let err = stderr(&f.tool(&["sync"]).code(2));
        assert!(
            err.contains(&format!("publish replace_after: '{bad}'")),
            "{err}"
        );
        assert!(err.contains("`7d`, `36h`, `2w` or `1d 12h`"), "{err}");
        assert!(f.forge().requests().is_empty());
    }
}

#[test]
fn without_the_setting_nothing_is_replaced_and_no_activity_is_read() {
    let f = Fork::merge_request();
    f.reconfigure("mode = \"follow\"\n", "mode = \"follow\"\ntags = \"v*\"\n");
    release(&f, "v1.2.0", &[("v12.txt", "1.2\n")]);
    f.tool(&["sync"]).success();
    let head = on_remote(&f, MERGE_REQUEST_BRANCH);
    release(&f, "v1.3.0", &[("v13.txt", "1.3\n")]);
    f.forge().age(1, 365);
    f.forge().clear_requests();
    let out = stdout(&f.tool(&["sync"]).success());
    assert!(out.contains("upstream: moved on to"), "{out}");
    assert!(!out.contains("replace"), "{out}");
    assert!(f.forge().changes().is_empty());
    assert!(!read_activity(&f));
    assert_eq!(on_remote(&f, MERGE_REQUEST_BRANCH), head);

    // After the merge, the next sync takes the newer release as before.
    merge_on_fake(&f.env, f.forge(), 1, How::MergeCommit);
    let out = stdout(&f.tool(&["sync"]).success());
    assert!(out.contains("opened merge request !2"), "{out}");
}

#[test]
fn a_replacement_gets_through_any_one_failed_request() {
    survives_each_failed_request(|failing| {
        let (f, _) = opened(Kind::GitLab);
        let v13 = release(&f, "v1.3.0", &[("v13.txt", "1.3\n")]);
        let fake = f.forge();
        fake.age(1, 8);
        fake.clear_requests();
        if let Some((method, path)) = failing {
            fake.fail_times(method, path, 502, 1);
        }
        let out = f.tool(&["sync"]);
        assert_eq!(
            out.get_output().status.code(),
            Some(0),
            "{failing:?}:\n{}\n{}",
            stdout(&out),
            stderr(&out)
        );
        assert_eq!(fake.mr(1).state, "closed", "{failing:?}");
        let open = fake.open_mrs();
        assert_eq!(open.len(), 1, "{failing:?}: {open:?}");
        let head = on_remote(&f, MERGE_REQUEST_BRANCH).unwrap();
        assert!(f.env.is_ancestor(&f.publish, &v13, &head), "{failing:?}");
        f.env.assert_no_update(&f.down);
        fake.requests()
    });
}

#[test]
fn a_failure_after_the_close_is_taken_up_by_the_next_attempt() {
    let (f, _) = opened(Kind::GitLab);
    let v13 = release(&f, "v1.3.0", &[("v13.txt", "1.3\n")]);
    let fake = f.forge();
    fake.age(1, 8);
    // The forge fails the first request after the close: reading the
    // project's merge settings before the push.
    fake.on_request(
        "PUT",
        "/merge_requests/1",
        vec![Action::FailTimes(
            "GET".into(),
            format!(
                "/projects/{}%2F{}",
                common::forge::OWNER,
                common::forge::REPO
            ),
            502,
            1,
        )],
    );
    let out = stdout(&f.tool(&["sync"]).success());
    assert!(out.contains("closed the merge request !1"), "{out}");
    assert!(out.contains("trying again"), "{out}");
    assert!(!out.contains("declined"), "{out}");
    assert!(out.contains("opened merge request !2"), "{out}");
    let old = fake.mr(1);
    assert_eq!(old.state, "closed");
    assert!(old.description.contains("Nothing of it was merged"));
    assert_eq!(fake.open_mrs().len(), 1);
    let head = on_remote(&f, MERGE_REQUEST_BRANCH).unwrap();
    assert!(f.env.is_ancestor(&f.publish, &v13, &head));
    let new = fake.mr(2).description;
    assert!(
        new.contains("replaces the merge request !1")
            && new.contains("when `upstream` v1.3.0 came out"),
        "{new}"
    );
    f.env.assert_no_update(&f.down);
}

#[test]
fn a_newer_prerelease_replaces_with_prereleases_set() {
    let f = replacing(Kind::GitLab);
    f.reconfigure("tags = \"v*\"\n", "tags = \"v*\"\nprereleases = true\n");
    release(&f, "v1.2.0-rc.1", &[("rc1.txt", "rc1\n")]);
    f.tool(&["sync"]).success();
    assert_eq!(common::open_number(f.forge()), 1);
    let head = on_remote(&f, MERGE_REQUEST_BRANCH).unwrap();
    let log = f.env.git(&f.publish, &["log", "--format=%B", "-1", &head]);
    assert!(log.contains("Intake-Tag: v1.2.0-rc.1"), "{log}");
    // Its sync merge names a prerelease tag, which the upstream's rules
    // admit as a target, so the branch is untouched.
    let rc2 = release(&f, "v1.2.0-rc.2", &[("rc2.txt", "rc2\n")]);
    f.forge().age(1, 8);
    let out = stdout(&f.tool(&["sync"]).success());
    assert!(
        out.contains("with an update at `upstream` v1.2.0-rc.2"),
        "{out}"
    );
    assert!(out.contains("opened merge request !2"), "{out}");
    assert_eq!(f.forge().mr(1).state, "closed");
    let head = on_remote(&f, MERGE_REQUEST_BRANCH).unwrap();
    assert!(f.env.is_ancestor(&f.publish, &rc2, &head));
}

#[test]
fn the_replacement_takes_the_tag_the_conflict_limit_selects() {
    let f = replacing(Kind::GitLab);
    f.reconfigure("tags = \"v*\"\n", "tags = \"v*\"\nmax_conflicts = 1\n");
    land(
        &f,
        &[("app.txt", "fork\ntwo\nthree\n"), ("f2.txt", "ours\n")],
        "fork: change",
    );
    release(&f, "v1.2.0", &[("v12.txt", "1.2\n")]);
    f.tool(&["sync"]).success();
    assert_eq!(common::open_number(f.forge()), 1);
    let v13 = release(&f, "v1.3.0", &[("v13.txt", "1.3\n")]);
    let v14 = release(
        &f,
        "v1.4.0",
        &[
            ("app.txt", "upstream\ntwo\nthree\n"),
            ("f2.txt", "theirs\n"),
        ],
    );
    f.forge().age(1, 8);
    let out = stdout(&f.tool(&["sync"]).success());
    assert!(
        out.contains("passed over v1.4.0 (2 files) for the limit"),
        "{out}"
    );
    assert!(out.contains("with an update at `upstream` v1.3.0"), "{out}");
    assert!(out.contains("opened merge request !2"), "{out}");
    let head = on_remote(&f, MERGE_REQUEST_BRANCH).unwrap();
    assert!(f.env.is_ancestor(&f.publish, &v13, &head));
    assert!(!f.env.is_ancestor(&f.publish, &v14, &head));
}

#[test]
fn a_recorded_pause_keeps_the_merge_request() {
    let (f, _) = opened(Kind::GitLab);
    release(&f, "v1.3.0", &[("v13.txt", "1.3\n")]);
    f.forge().age(1, 8);
    dead_pause(&f, 1, &[]);
    f.forge().clear_requests();
    let out = f.tool(&["sync"]).success();
    let (out, err) = (stdout(&out), stderr(&out));
    assert!(
        err.contains("warning: a pause of the merge queue of @merge-bot is recorded on 'origin'"),
        "{err}"
    );
    assert!(
        out.contains(
            "the merge request !1 (http"
        ) && out.contains(
            "is not replaced: `upstream` v1.3.0 came out, but a pause of the merge queue is recorded"
        ),
        "{out}"
    );
    assert!(
        out.contains("the update waits for the merge request !1"),
        "{out}"
    );
    // The pause is told from the publish remote alone.
    assert!(!read_activity(&f));
    assert!(f.forge().changes().is_empty(), "{:?}", f.forge().changes());
    assert!(f.env.has_rev(&f.publish, PAUSE_REF));
}

#[test]
fn land_ends_on_a_merge_request_a_replacement_closed() {
    // A queue makes `land` pause it for the last pipeline, after a
    // catch-up, so that its end shows that the pause ends too.
    let (f, _) = opened(Kind::GitLab);
    f.reconfigure(
        "replace_after = \"7d\"\n",
        "replace_after = \"7d\"\nqueue = \"merge-bot\"\n",
    );
    let fake = f.forge();
    fake.add_user(common::BOT, "merge-bot");
    let head = on_remote(&f, MERGE_REQUEST_BRANCH).unwrap();
    fake.script(&head, vec![Step::is("success")]);
    // A scheduled `sync` replaces the merge request while `land` waits for
    // the pipeline of its catch-up: after the read that follows the push,
    // at the first read of a round.
    fake.script_branch(
        MERGE_REQUEST_BRANCH,
        vec![Step::is("running"), Step::is("running").after(Action::Close(
            1,
            "Upstream update by git intake, replaced.\n\nThis merge request was untouched when `upstream` \
             v1.3.0 came out."
                .into(),
        ))],
    );
    let out = stdout(&f.tool(&["land", "--poll", "0", "--timeout", "120"]).code(0));
    assert!(
        out.contains("paused the merge queue of @merge-bot"),
        "{out}"
    );
    assert!(
        out.contains("was closed without merging: `git intake sync` replaced it with an update at a newer release"),
        "{out}"
    );
    assert!(out.contains("`git intake land` lands the new one"), "{out}");
    assert!(!f.env.has_rev(&f.publish, PAUSE_REF));
    assert!(
        fake.requests().iter().all(|r| !r.path.ends_with("/merge")),
        "nothing was merged"
    );
    f.env.assert_no_update(&f.down);
}

#[test]
fn merge_settings_that_refuse_the_publication_close_nothing() {
    let (f, _) = opened(Kind::GitLab);
    let head = on_remote(&f, MERGE_REQUEST_BRANCH);
    release(&f, "v1.3.0", &[("v13.txt", "1.3\n")]);
    f.forge().age(1, 8);
    f.forge().set(
        "only_allow_merge_if_pipeline_succeeds",
        serde_json::json!(false),
    );
    f.forge().clear_requests();
    let err = stderr(&f.tool(&["sync"]).code(2));
    assert!(
        err.contains("Nothing was pushed. Change these settings")
            && err.contains("only_allow_merge_if_pipeline_succeeds"),
        "{err}"
    );
    assert!(f.forge().changes().is_empty(), "{:?}", f.forge().changes());
    assert_eq!(f.forge().mr(1).state, "open");
    assert_eq!(on_remote(&f, MERGE_REQUEST_BRANCH), head);
    f.env.assert_no_update(&f.down);
}

/// Lands a commit on the publish remote's integration branch of a
/// monorepo; returns it.
fn land_mono(m: &Mono, files: &[(&str, &str)], message: &str) -> String {
    let dir = common::forge(&m.env, &m.publish);
    let c = m.env.commit(&dir, files, message);
    m.env.git(&dir, &["push", "-q", "origin", "HEAD:main"]);
    c
}

/// Tags `commit` of `up` as `tag` and pushes the tag alone.
fn tag_upstream(env: &common::Env, up: &common::Upstream, tag: &str, commit: &str) {
    env.git(&up.work, &["tag", tag, commit]);
    env.git(&up.work, &["push", "-q", "origin", tag]);
}

#[test]
fn a_conflict_limit_that_would_move_an_upstream_back_keeps_the_merge_request() {
    // Both upstreams are taken at their releases, `app-a` within
    // `max_conflicts = 1`.
    let m = Mono::merge_request();
    let env = &m.env;
    let config = fs::read_to_string(m.down.join(".git-intake.toml"))
        .unwrap()
        .replace(
            "path = \"apps/a\"\n",
            "path = \"apps/a\"\ntags = \"v*\"\nmax_conflicts = 1\n",
        )
        .replace("path = \"apps/b\"\n", "path = \"apps/b\"\ntags = \"v*\"\n")
        .replace(
            "via = \"merge-request\"\n",
            "via = \"merge-request\"\nreplace_after = \"7d\"\n",
        );
    env.commit(&m.down, &[(".git-intake.toml", &config)], "mono: replace");
    env.git(&m.down, &["push", "-q", "origin", "main"]);
    env.set_var("GIT_INTAKE_TEST_RETRY_GAP_MS", "0");
    for up in [&m.a, &m.b] {
        let head = env.rev(&up.work, "HEAD");
        tag_upstream(env, up, "v1.0.0", &head);
    }
    let fake = m.forge.as_ref().unwrap();
    m.tool(&["sync"]).success();
    merge_on_fake(env, fake, 1, How::MergeCommit);
    let a11 = env.push_upstream(&m.a, &[("a11.txt", "1.1\n")], "app-a: 1.1");
    tag_upstream(env, &m.a, "v1.1.0", &a11);
    let a12 = env.push_upstream(
        &m.a,
        &[("f1.txt", "theirs\n"), ("f2.txt", "theirs\n")],
        "app-a: 1.2",
    );
    tag_upstream(env, &m.a, "v1.2.0", &a12);
    let out = stdout(&m.tool(&["sync"]).success());
    assert!(out.contains("opened merge request !2"), "{out}");
    // The fork's own files now make `v1.2.0` conflict past the limit, so a
    // plan takes `v1.1.0`, older than the merge request's, while `app-b`
    // releases.
    land_mono(
        &m,
        &[("apps/a/f1.txt", "ours\n"), ("apps/a/f2.txt", "ours\n")],
        "fork: a's files",
    );
    let b11 = env.push_upstream(&m.b, &[("b11.txt", "1.1\n")], "app-b: 1.1");
    tag_upstream(env, &m.b, "v1.1.0", &b11);
    fake.age(2, 8);
    fake.clear_requests();
    // The merge request is kept, and its catch-up stops on the same files.
    let out = stdout(&m.tool(&["sync"]).code(1));
    assert!(
        out.contains("is not replaced: `app-b` v1.1.0 came out, but the update planned from 'main' would take 'app-a'")
            && out.contains("which does not contain")
            && out.contains("a replacement never moves an upstream back"),
        "{out}"
    );
    assert_eq!(fake.mr(2).state, "open");
    assert_eq!(fake.mrs().len(), 2);
    assert!(
        fake.requests()
            .iter()
            .all(|r| !(r.method == "PUT" && r.json()["state_event"] == "close")),
        "nothing was closed"
    );
}

#[test]
fn a_maintenance_release_that_does_not_contain_the_target_keeps_the_merge_request() {
    let f = replacing(Kind::GitLab);
    f.reconfigure("tags = \"v*\"\n", "tags = \"v*\"\nmax_conflicts = 1\n");
    let env = &f.env;
    let v10 = release(&f, "v1.0.0", &[("base.txt", "b\n")]);
    f.tool(&["sync"]).success();
    merge_on_fake(env, f.forge(), 1, How::MergeCommit);
    // `v1.2.5` on a maintenance branch from `v1.0.0`, `v1.3.0` on the main
    // line.
    env.git(&f.up.work, &["checkout", "-q", "-b", "maint", &v10]);
    let v125 = env.commit(&f.up.work, &[("fix.txt", "fix\n")], "upstream: 1.2.5");
    tag_upstream(env, &f.up, "v1.2.5", &v125);
    env.git(&f.up.work, &["checkout", "-q", "-"]);
    release(
        &f,
        "v1.3.0",
        &[("f1.txt", "theirs\n"), ("f2.txt", "theirs\n")],
    );
    let out = stdout(&f.tool(&["sync"]).success());
    assert!(out.contains("opened merge request !2"), "{out}");
    // The maintenance branch joins the main line, and the fork's own files
    // make `v1.3.0` conflict past the limit, so a plan takes `v1.2.5`.
    env.git(&f.up.work, &["merge", "-q", "--no-edit", "maint"]);
    env.git(&f.up.work, &["push", "-q", "origin", "HEAD:main"]);
    land(
        &f,
        &[("f1.txt", "ours\n"), ("f2.txt", "ours\n")],
        "fork: own files",
    );
    f.forge().age(2, 8);
    f.forge().clear_requests();
    let out = stdout(&f.tool(&["sync"]).code(1));
    assert!(
        out.contains("is not replaced: `upstream` v1.2.5 came out, but the update planned from 'main' would take 'upstream' at")
            && out.contains("which does not contain")
            && out.contains("(v1.3.0)"),
        "{out}"
    );
    assert_eq!(f.forge().mr(2).state, "open");
    assert_eq!(f.forge().mrs().len(), 2);
}

#[test]
fn a_resolved_conflict_on_the_branch_keeps_the_merge_request() {
    let f = replacing(Kind::GitLab);
    let env = &f.env;
    land(&f, &[("app.txt", "fork\ntwo\nthree\n")], "fork: change");
    release(&f, "v1.2.0", &[("app.txt", "upstream\ntwo\nthree\n")]);
    // `sync` stops on the conflict; a person resolves it and publishes.
    f.tool(&["sync"]).code(1);
    let wt = env.update_worktree(&f.down);
    fs::write(wt.join("app.txt"), "both\ntwo\nthree\n").unwrap();
    env.git(&wt, &["add", "app.txt"]);
    f.tool(&["continue"]).success();
    f.tool(&["finish"]).success();
    assert_eq!(common::open_number(f.forge()), 1);
    release(&f, "v1.3.0", &[("v13.txt", "1.3\n")]);
    f.forge().age(1, 8);
    f.forge().clear_requests();
    let out = stdout(&f.tool(&["sync"]).success());
    assert!(
        out.contains("needed a resolution in `app.txt`, which a person made"),
        "{out}"
    );
    assert!(f.forge().changes().is_empty(), "{:?}", f.forge().changes());
    assert!(!read_activity(&f));
}

#[test]
fn a_merge_just_before_the_close_restores_the_description() {
    let (f, _) = opened(Kind::GitLab);
    release(&f, "v1.3.0", &[("v13.txt", "1.3\n")]);
    let fake = f.forge();
    fake.age(1, 8);
    let before = fake.mr(1).description;
    // The queue merges it after the last read, as the close arrives.
    fake.before_request("PUT", "/merge_requests/1", vec![Action::Merge(1)]);
    let out = stdout(&f.tool(&["sync"]).success());
    assert!(
        out.contains("was merged just as sync closed it; its description was restored, and nothing was pushed"),
        "{out}"
    );
    let mr = fake.mr(1);
    assert_eq!(mr.state, "merged");
    assert_eq!(mr.description, before);
    assert_eq!(fake.mrs().len(), 1);
    f.env.assert_no_update(&f.down);
}

#[test]
fn a_restored_description_is_written_again_through_a_failure() {
    let (f, _) = opened(Kind::GitLab);
    release(&f, "v1.3.0", &[("v13.txt", "1.3\n")]);
    let fake = f.forge();
    fake.age(1, 8);
    let before = fake.mr(1).description;
    fake.before_request("PUT", "/merge_requests/1", vec![Action::Merge(1)]);
    // The two requests after the close fail with a 5xx.
    fake.on_request(
        "PUT",
        "/merge_requests/1",
        vec![Action::FailTimes(
            "PUT".into(),
            "/merge_requests/1".into(),
            502,
            2,
        )],
    );
    let out = stdout(&f.tool(&["sync"]).success());
    assert!(out.contains("trying again"), "{out}");
    assert!(out.contains("its description was restored"), "{out}");
    let mr = fake.mr(1);
    assert_eq!(mr.state, "merged");
    assert_eq!(mr.description, before);
    assert_eq!(fake.mrs().len(), 1);
}

#[test]
fn a_description_that_cannot_be_restored_stops_the_run() {
    let (f, _) = opened(Kind::GitLab);
    release(&f, "v1.3.0", &[("v13.txt", "1.3\n")]);
    let fake = f.forge();
    fake.age(1, 8);
    fake.before_request("PUT", "/merge_requests/1", vec![Action::Merge(1)]);
    fake.on_request(
        "PUT",
        "/merge_requests/1",
        vec![Action::FailTimes(
            "PUT".into(),
            "/merge_requests/1".into(),
            502,
            100,
        )],
    );
    let out = f.tool(&["sync"]).code(2);
    let err = stderr(&out);
    assert!(
        err.contains("GitLab merged the merge request !1")
            && err.contains("as sync closed it, and its description could not be restored"),
        "{err}"
    );
    // The run did not go again: one close, no new merge request.
    let closes = fake
        .requests()
        .iter()
        .filter(|r| r.method == "PUT" && r.json()["state_event"] == "close")
        .count();
    assert_eq!(closes, 1);
    assert_eq!(fake.mrs().len(), 1);
    f.env.assert_no_update(&f.down);
}

#[test]
fn a_merge_request_gitlab_is_merging_is_kept() {
    let (f, _) = opened(Kind::GitLab);
    release(&f, "v1.3.0", &[("v13.txt", "1.3\n")]);
    let fake = f.forge();
    fake.age(1, 8);
    fake.edit(1, |m| m.locked = true);
    fake.clear_requests();
    let out = stdout(&f.tool(&["sync"]).success());
    assert!(
        out.contains(
            "is not replaced: `upstream` v1.3.0 came out, but GitLab is merging the merge request"
        ),
        "{out}"
    );
    assert!(fake.changes().is_empty(), "{:?}", fake.changes());
    assert!(!read_activity(&f));
}

#[test]
fn a_close_that_kept_the_description_is_followed_by_the_note() {
    let (f, _) = opened(Kind::GitLab);
    release(&f, "v1.3.0", &[("v13.txt", "1.3\n")]);
    let fake = f.forge();
    fake.age(1, 8);
    fake.world().close_keeps_description = true;
    fake.clear_requests();
    let out = stdout(&f.tool(&["sync"]).success());
    assert!(out.contains("opened merge request !2"), "{out}");
    let old = fake.mr(1);
    assert_eq!(old.state, "closed");
    assert!(
        !old.description.contains("<!-- git-intake targets")
            && old.description.contains("Nothing of it was merged"),
        "{}",
        old.description
    );
    let writes = fake
        .changes()
        .into_iter()
        .filter(|r| r.method == "PUT" && r.path.ends_with("/merge_requests/1"))
        .count();
    assert_eq!(writes, 2);
}

#[test]
fn a_pause_recorded_during_the_replacement_stops_it() {
    let (f, _) = opened(Kind::GitLab);
    let head = on_remote(&f, MERGE_REQUEST_BRANCH);
    release(&f, "v1.3.0", &[("v13.txt", "1.3\n")]);
    f.forge().age(1, 8);
    // `land` records a pause while `sync` merges.
    let record = common::record(&f, 1, &[], &[]);
    in_update_worktree_hook(
        &f.down,
        "post-merge",
        &format!(
            "git --git-dir={} update-ref {PAUSE_REF} {record}",
            f.publish.display()
        ),
    );
    f.forge().clear_requests();
    let out = stdout(&f.tool(&["sync"]).success());
    assert!(
        out.contains(
            "a pause of the merge queue was recorded on 'origin' while sync merged the replacement"
        ),
        "{out}"
    );
    assert!(f.forge().changes().is_empty(), "{:?}", f.forge().changes());
    assert_eq!(f.forge().mr(1).state, "open");
    assert_eq!(on_remote(&f, MERGE_REQUEST_BRANCH), head);
    f.env.assert_no_update(&f.down);
}

#[test]
fn an_unreadable_pause_record_keeps_the_merge_request() {
    let (f, _) = opened(Kind::GitLab);
    release(&f, "v1.3.0", &[("v13.txt", "1.3\n")]);
    f.forge().age(1, 8);
    // The reference points at a commit that is no record.
    let tree = f.env.git(
        &f.publish,
        &["hash-object", "-t", "tree", "-w", "/dev/null"],
    );
    let junk = f
        .env
        .git(&f.publish, &["commit-tree", &tree, "-m", "no record"]);
    f.env.git(&f.publish, &["update-ref", PAUSE_REF, &junk]);
    f.forge().clear_requests();
    let out = f.tool(&["sync"]).success();
    let (out, err) = (stdout(&out), stderr(&out));
    assert!(
        err.contains("warning: cannot tell whether the merge queue is paused"),
        "{err}"
    );
    assert!(
        out.contains("whether a pause of the merge queue is recorded cannot be read"),
        "{out}"
    );
    assert!(f.forge().changes().is_empty(), "{:?}", f.forge().changes());
    assert!(!read_activity(&f));
}

#[test]
fn a_merge_request_too_young_costs_no_prediction() {
    let f = replacing(Kind::GitLab);
    f.reconfigure("tags = \"v*\"\n", "tags = \"v*\"\nmax_conflicts = 5\n");
    release(&f, "v1.2.0", &[("v12.txt", "1.2\n")]);
    f.tool(&["sync"]).success();
    release(&f, "v1.3.0", &[("v13.txt", "1.3\n")]);
    f.forge().age(1, 6);
    f.forge().clear_requests();
    let out = stdout(&f.tool(&["sync"]).success());
    assert!(out.contains("may be replaced from"), "{out}");
    assert!(!out.contains("predicted to conflict"), "{out}");
    assert!(!read_activity(&f));
    // Old enough, the plan predicts within the limit before the reads.
    f.forge().age(1, 8);
    let out = stdout(&f.tool(&["sync"]).success());
    assert!(out.contains("predicted to conflict"), "{out}");
    assert!(out.contains("opened merge request !2"), "{out}");
}

/// Makes `actions` happen just before the second read of merge request !1
/// alone, the last one before the close.
fn before_the_last_read(fake: &FakeForge, actions: Vec<Action>) {
    fake.before_request(
        "GET",
        "/merge_requests/1",
        vec![Action::BeforeRequest(
            "GET".into(),
            "/merge_requests/1".into(),
            actions,
        )],
    );
}

#[test]
fn a_trace_the_last_read_shows_stops_the_replacement() {
    for (action, why) in [
        (Action::Assign(1, 7), "someone is assigned to it"),
        (Action::AutoMerge(1), "it is set to merge automatically"),
    ] {
        let (f, _) = opened(Kind::GitLab);
        let head = on_remote(&f, MERGE_REQUEST_BRANCH);
        release(&f, "v1.3.0", &[("v13.txt", "1.3\n")]);
        let fake = f.forge();
        fake.age(1, 8);
        before_the_last_read(fake, vec![action]);
        fake.clear_requests();
        let out = stdout(&f.tool(&["sync"]).success());
        assert!(
            out.contains(&format!("is no longer untouched: {why}")),
            "{why}: {out}"
        );
        assert!(
            fake.changes()
                .iter()
                .all(|r| !(r.method == "PUT" && r.path.ends_with("/merge_requests/1"))),
            "{why}: nothing was closed"
        );
        assert_eq!(fake.mr(1).state, "open");
        assert_eq!(on_remote(&f, MERGE_REQUEST_BRANCH), head);
        f.env.assert_no_update(&f.down);
    }
}

#[test]
fn a_merge_before_the_last_read_is_found_by_it() {
    let (f, _) = opened(Kind::GitLab);
    release(&f, "v1.3.0", &[("v13.txt", "1.3\n")]);
    let fake = f.forge();
    fake.age(1, 8);
    let before = fake.mr(1).description;
    // The first read sees it open; the queue merges it before the last.
    before_the_last_read(fake, vec![Action::Merge(1)]);
    fake.clear_requests();
    let out = stdout(&f.tool(&["sync"]).success());
    assert!(
        out.contains("was merged while sync merged its replacement; nothing was closed or pushed"),
        "{out}"
    );
    assert!(!out.contains("description was restored"), "{out}");
    let reads = fake
        .requests()
        .iter()
        .filter(|r| {
            r.method == "GET"
                && r.path
                    .split('?')
                    .next()
                    .unwrap()
                    .ends_with("/merge_requests/1")
        })
        .count();
    assert_eq!(reads, 2);
    assert!(
        fake.changes().iter().all(|r| r.method != "PUT"),
        "{:?}",
        fake.changes()
    );
    assert_eq!(fake.mr(1).state, "merged");
    assert_eq!(fake.mr(1).description, before);
}

#[test]
fn a_kept_description_is_written_again_through_a_failure() {
    let (f, _) = opened(Kind::GitLab);
    release(&f, "v1.3.0", &[("v13.txt", "1.3\n")]);
    let fake = f.forge();
    fake.age(1, 8);
    fake.world().close_keeps_description = true;
    // The two requests after the close fail with a 5xx.
    fake.on_request(
        "PUT",
        "/merge_requests/1",
        vec![Action::FailTimes(
            "PUT".into(),
            "/merge_requests/1".into(),
            502,
            2,
        )],
    );
    let out = stdout(&f.tool(&["sync"]).success());
    assert!(out.contains("trying again"), "{out}");
    assert!(out.contains("opened merge request !2"), "{out}");
    let old = fake.mr(1);
    assert!(
        !old.description.contains("<!-- git-intake targets"),
        "{}",
        old.description
    );
}

#[test]
fn a_kept_description_that_cannot_be_written_stops_the_run() {
    let (f, _) = opened(Kind::GitLab);
    release(&f, "v1.3.0", &[("v13.txt", "1.3\n")]);
    let fake = f.forge();
    fake.age(1, 8);
    fake.world().close_keeps_description = true;
    fake.on_request(
        "PUT",
        "/merge_requests/1",
        vec![Action::FailTimes(
            "PUT".into(),
            "/merge_requests/1".into(),
            502,
            100,
        )],
    );
    let out = f.tool(&["sync"]).code(2);
    let err = stderr(&out);
    assert!(
        err.contains("closed the merge request !1")
            && err.contains("kept its description, which still records its targets"),
        "{err}"
    );
    let old = fake.mr(1);
    assert_eq!(old.state, "closed");
    assert!(old.description.contains("<!-- git-intake targets"));
    // The run did not go again: no second close, no new merge request.
    let closes = fake
        .requests()
        .iter()
        .filter(|r| r.method == "PUT" && r.json()["state_event"] == "close")
        .count();
    assert_eq!(closes, 1);
    assert_eq!(fake.mrs().len(), 1);
    f.env.assert_no_update(&f.down);
}

#[test]
fn a_merge_changed_beyond_its_parents_keeps_the_merge_request() {
    let (f, _) = opened(Kind::GitLab);
    let env = &f.env;
    // A person amends the sync merge with a change of their own, keeping
    // its message and parents.
    let dir = common::forge(env, &f.publish);
    env.git(&dir, &["fetch", "-q", "origin", MERGE_REQUEST_BRANCH]);
    env.git(&dir, &["checkout", "-q", "FETCH_HEAD"]);
    fs::write(dir.join("extra.txt"), "mine\n").unwrap();
    env.git(&dir, &["add", "extra.txt"]);
    env.git(&dir, &["commit", "-q", "--amend", "--no-edit"]);
    env.git(
        &dir,
        &[
            "push",
            "-q",
            "-f",
            "origin",
            &format!("HEAD:refs/heads/{MERGE_REQUEST_BRANCH}"),
        ],
    );
    release(&f, "v1.3.0", &[("v13.txt", "1.3\n")]);
    f.forge().age(1, 8);
    f.forge().clear_requests();
    let out = stdout(&f.tool(&["sync"]).success());
    assert!(
        out.contains("holds changes beyond the clean merge of its parents, which a person made"),
        "{out}"
    );
    assert!(f.forge().changes().is_empty(), "{:?}", f.forge().changes());
    assert!(!read_activity(&f));
}
