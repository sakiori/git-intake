//! `release`: releases of the integration branch, cut, checked, versioned
//! and described from the publish remote. The forge is a fake one; see
//! `common::forge`.

mod common;

use std::fs;
use std::path::PathBuf;

use common::forge::branch_tip;
use common::{Fork, stderr, stdout};
use predicates::str::contains;

/// The instant the tests run at, and its UTC day.
const NOW: &str = "2026-11-01T12:00:00Z";
const DAY: &str = "20261101";

/// Tags `commit` of the upstream as `tag` and pushes the tag.
fn upstream_tag(f: &Fork, tag: &str, commit: &str) {
    f.env.git(&f.up.work, &["tag", tag, commit]);
    f.env.git(&f.up.work, &["push", "-q", "origin", tag]);
}

/// Appends `text` to the fork's configuration and publishes it.
fn append_config(f: &Fork, text: &str) -> String {
    let path = f.down.join(".git-intake.toml");
    let cfg = fs::read_to_string(&path).unwrap() + text;
    let c = f.env.commit(
        &f.down,
        &[(".git-intake.toml", &cfg)],
        "fork: make the integration branch releasable",
    );
    f.env.git(&f.down, &["push", "-q", "origin", "main"]);
    c
}

/// Makes `f` releasable from its upstream's `v*` tags, with the upstream's
/// initial commit released as `v0.11.0`, at the tests' instant.
fn releasable(f: &Fork, release: &str) -> String {
    f.env.set_var("GIT_INTAKE_TEST_NOW", NOW);
    let initial = f.env.rev(&f.up.work, "HEAD");
    upstream_tag(f, "v0.11.0", &initial);
    append_config(f, release)
}

const RELEASE: &str = "\n[release]\nupstream = \"upstream\"\ntags = \"v*\"\n";

fn release_fork() -> Fork {
    let f = Fork::new();
    releasable(&f, RELEASE);
    f
}

/// A fork that publishes by push to a fake GitLab, releasable.
fn forge_fork() -> Fork {
    let f = Fork::push_gitlab();
    releasable(&f, RELEASE);
    f
}

/// Commits on the fork's integration branch and publishes it.
fn land(f: &Fork, name: &str, message: &str) -> String {
    let c = f
        .env
        .commit(&f.down, &[(&format!("{name}.txt"), "x\n")], message);
    f.env.git(&f.down, &["push", "-q", "origin", "main"]);
    c
}

/// Tags `commit` of the fork as `tag` and pushes the tag, as a release
/// does.
fn release_tag(f: &Fork, tag: &str, commit: &str) {
    f.env.git(
        &f.down,
        &["tag", "-a", "-m", &format!("Release {tag}"), tag, commit],
    );
    f.env.git(&f.down, &["push", "-q", "origin", tag]);
}

fn next_json(f: &Fork) -> serde_json::Value {
    f.env.tool_json(&f.down, &["release", "next", "--json"], 0)
}

/// The tags of the publish remote.
fn remote_tags(f: &Fork) -> Vec<String> {
    f.env
        .git(&f.publish, &["tag", "--list"])
        .lines()
        .map(str::to_string)
        .collect()
}

/// A clone of the publish remote as a CI job makes it: `HEAD` detached at
/// `rev`, no local integration branch.
fn ci_clone(f: &Fork, rev: &str) -> PathBuf {
    let env = &f.env;
    let dir = env.path("ci");
    env.git(
        &env.path(""),
        &[
            "clone",
            "-q",
            "--no-checkout",
            &f.publish.to_string_lossy(),
            &dir.to_string_lossy(),
        ],
    );
    env.git(&dir, &["checkout", "-q", "--detach", rev]);
    env.git(&dir, &["branch", "-D", "main"]);
    if let Some(forge) = &f.forge {
        env.git(&dir, &["remote", "set-url", "origin", &forge.remote_url()]);
    }
    dir
}

#[test]
fn the_base_is_the_next_patch_after_the_newest_contained_upstream_release() {
    let f = release_fork();
    let doc = next_json(&f);
    assert_eq!(doc["base"], "0.11.1");
    assert_eq!(doc["upstream_release"], "v0.11.0");
    assert_eq!(doc["release"], serde_json::Value::Null);
    assert_eq!(doc["published"], serde_json::Value::Null);
    assert_eq!(doc["next"], format!("v0.11.1-custom.{DAY}.1"));
    assert_eq!(doc["tip"], f.env.rev(&f.publish, "main"));
    assert_eq!(doc["forge_error"], serde_json::Value::Null);
    assert_eq!(doc["versioning"], "dated");
    assert_eq!(doc["bump"], serde_json::Value::Null);
}

#[test]
fn a_branch_past_a_release_takes_the_next_patch() {
    let f = release_fork();
    // Twelve upstream commits after v0.11.0, and v0.12.0 on a commit the
    // tip does not take.
    for i in 0..12 {
        f.env.push_upstream(
            &f.up,
            &[(&format!("u{i}.txt"), "u\n")],
            &format!("upstream: {i}"),
        );
    }
    f.tool(&["sync"]).success();
    let future = f
        .env
        .push_upstream(&f.up, &[("future.txt", "f\n")], "upstream: future");
    upstream_tag(&f, "v0.12.0", &future);
    let doc = next_json(&f);
    assert_eq!(doc["base"], "0.11.1");
    assert_eq!(doc["upstream_release"], "v0.11.0");
}

#[test]
fn an_upstream_prerelease_is_no_upstream_release() {
    let f = Fork::new();
    f.env.set_var("GIT_INTAKE_TEST_NOW", NOW);
    let initial = f.env.rev(&f.up.work, "HEAD");
    upstream_tag(&f, "v0.1.6", &initial);
    let c = f
        .env
        .push_upstream(&f.up, &[("n.txt", "n\n")], "upstream: n");
    upstream_tag(&f, "v0.1.7-nightly.20261031.3991", &c);
    f.tool(&["sync"]).success();
    append_config(&f, RELEASE);
    assert_eq!(next_json(&f)["base"], "0.1.7");

    // An upstream taken at its prereleases still releases from its releases.
    let d = f
        .env
        .push_upstream(&f.up, &[("d.txt", "d\n")], "upstream: d");
    upstream_tag(&f, "v0.1.8-nightly.20261101.4000", &d);
    f.reconfigure(
        "mode = \"follow\"\n",
        "mode = \"follow\"\ntags = \"v*\"\nprereleases = true\n",
    );
    f.tool(&["sync"]).success();
    assert_eq!(f.env.git(&f.publish, &["rev-parse", "main^2"]), d);
    let doc = next_json(&f);
    assert_eq!(doc["base"], "0.1.7");
    assert_eq!(doc["upstream_release"], "v0.1.6");
}

#[test]
fn prefixed_release_tags_carry_their_version_after_the_prefix() {
    let f = Fork::new();
    f.env.set_var("GIT_INTAKE_TEST_NOW", NOW);
    let initial = f.env.rev(&f.up.work, "HEAD");
    upstream_tag(&f, "expo-device-hub@0.19.1", &initial);
    upstream_tag(&f, "v9.0.0", &initial);
    append_config(
        &f,
        "\n[release]\nupstream = \"upstream\"\ntags = \"expo-device-hub@*\"\n",
    );
    let doc = next_json(&f);
    assert_eq!(doc["base"], "0.19.2");
    assert_eq!(doc["upstream_release"], "expo-device-hub@0.19.1");
}

#[test]
fn release_tags_without_following_them_leave_updates_at_the_branch_tip() {
    let f = release_fork();
    let tip = f
        .env
        .push_upstream(&f.up, &[("n.txt", "n\n")], "upstream: n");
    let status = f.env.tool_json(&f.down, &["status", "--json"], 0);
    let up = &status["upstreams"][0];
    assert_eq!(up["tags"], serde_json::Value::Null);
    assert_eq!(up["tip"], tip);
    assert_eq!(up["state"], "behind");
    assert_eq!(status["release"]["base"], "0.11.1");
    f.tool(&["sync"]).success();
    assert_eq!(f.env.git(&f.publish, &["rev-parse", "main^2"]), tip);
}

#[test]
fn a_commit_without_an_upstream_release_is_refused() {
    let f = Fork::new();
    append_config(&f, RELEASE);
    let err = stderr(&f.tool(&["release", "next"]).code(2));
    assert!(
        err.contains("contains no release of upstream 'upstream'") && err.contains("'v*'"),
        "{err}"
    );
}

#[test]
fn an_upstream_that_does_not_follow_is_refused() {
    let f = Fork::port();
    let cfg = fs::read_to_string(f.down.join(".git-intake.toml")).unwrap()
        + "\n[release]\nupstream = \"upstream\"\ntags = \"v*\"\n";
    f.env
        .commit(&f.down, &[(".git-intake.toml", &cfg)], "fork: release");
    let err = stderr(&f.tool(&["status"]).code(2));
    assert!(err.contains("release.upstream"), "{err}");
}

#[test]
fn the_second_release_of_a_day_takes_the_next_number() {
    let f = release_fork();
    let c = f.env.rev(&f.down, "main");
    release_tag(&f, &format!("v0.11.1-custom.{DAY}.1"), &c);
    land(&f, "feature", "feat: a feature");
    assert_eq!(next_json(&f)["next"], format!("v0.11.1-custom.{DAY}.2"));
}

#[test]
fn a_base_older_than_a_release_is_refused() {
    let f = release_fork();
    let c = f.env.rev(&f.down, "main");
    release_tag(&f, "v0.12.1-custom.20261031.1", &c);
    land(&f, "feature", "feat: a feature");
    let err = stderr(&f.tool(&["release", "next"]).code(2));
    assert!(
        err.contains("v0.12.1-custom.20261031.1") && err.contains("does not sort after"),
        "{err}"
    );
}

#[test]
fn a_label_that_sorts_first_is_refused() {
    let f = release_fork();
    let c = f.env.rev(&f.down, "main");
    f.reconfigure("tags = \"v*\"\n", "tags = \"v*\"\nlabels = [\"nightly\"]\n");
    release_tag(&f, "v0.11.1-nightly.20261031.1", &c);
    f.reconfigure(
        "labels = [\"nightly\"]",
        "labels = [\"custom\", \"nightly\"]",
    );
    let err = stderr(&f.tool(&["release", "next"]).code(2));
    assert!(
        err.contains(&format!(
            "v0.11.1-custom.{DAY}.1 does not sort after the release v0.11.1-nightly.20261031.1"
        )),
        "{err}"
    );
}

#[test]
fn releases_of_earlier_labels_count() {
    let f = release_fork();
    let c = f.env.rev(&f.down, "main");
    release_tag(&f, &format!("v0.1.6-custom.{DAY}.3"), &c);
    f.reconfigure(
        "tags = \"v*\"\n",
        "tags = \"v*\"\nlabels = [\"nightly\", \"custom\"]\n",
    );
    assert_eq!(next_json(&f)["next"], format!("v0.11.1-nightly.{DAY}.4"));
}

#[test]
fn upstream_tags_of_release_form_below_the_downstream_are_no_releases() {
    let f = release_fork();
    // The upstream's own build tag on the upstream commit the fork was
    // cloned from, carried in the publish remote.
    let initial = f.env.rev(&f.up.work, "HEAD");
    release_tag(&f, "v9.0.21-custom.20260419.73", &initial);
    assert_eq!(next_json(&f)["next"], format!("v0.11.1-custom.{DAY}.1"));
}

#[test]
fn a_tag_off_the_first_parent_line_is_no_release() {
    let f = release_fork();
    let env = &f.env;
    env.git(&f.down, &["switch", "-q", "-c", "topic"]);
    let side = env.commit(&f.down, &[("side.txt", "s\n")], "feat: side");
    env.git(&f.down, &["switch", "-q", "main"]);
    env.git(
        &f.down,
        &["merge", "-q", "--no-ff", "-m", "Merge topic", "topic"],
    );
    env.git(&f.down, &["push", "-q", "origin", "main"]);
    release_tag(&f, "v0.12.1-custom.20261031.5", &side);
    assert_eq!(next_json(&f)["next"], format!("v0.11.1-custom.{DAY}.1"));
}

#[test]
fn release_tag_pushes_the_next_release_at_the_remote_tip() {
    let f = release_fork();
    let env = &f.env;
    env.git(&f.down, &["tag", "own-tag", "main"]);
    env.git(&f.down, &["push", "-q", "origin", "own-tag"]);
    // The local branch lags the publish remote's.
    let before = env.rev(&f.down, "main");
    let dir = common::forge(env, &f.publish);
    let tip = env.commit(&dir, &[("other.txt", "o\n")], "feat: landed elsewhere");
    env.git(&dir, &["push", "-q", "origin", "HEAD:main"]);

    let out = stdout(&f.tool(&["release", "tag"]).success());
    let name = format!("v0.11.1-custom.{DAY}.1");
    assert!(
        out.contains(&format!("pushed the release tag {name}")),
        "{out}"
    );
    assert_eq!(remote_tags(&f), vec!["own-tag".to_string(), name.clone()]);
    assert_eq!(env.rev(&f.publish, &format!("{name}^{{commit}}")), tip);
    assert_eq!(env.git(&f.publish, &["cat-file", "-t", &name]), "tag");
    let message = env.git(&f.publish, &["tag", "-l", "--format=%(contents)", &name]);
    assert_eq!(message, format!("Release 0.11.1-custom.{DAY}.1"));
    assert_eq!(
        env.git(&f.publish, &["tag", "-l", "--format=%(taggername)", &name]),
        "Test Committer"
    );
    // The local branch stays; the tag is created locally as well.
    assert_eq!(env.rev(&f.down, "main"), before);
    assert_eq!(env.rev(&f.down, &format!("{name}^{{commit}}")), tip);
}

#[test]
fn release_next_creates_no_tag() {
    let f = release_fork();
    f.tool(&["release", "next"])
        .success()
        .stdout(contains(format!("next release     v0.11.1-custom.{DAY}.1")));
    assert!(remote_tags(&f).is_empty());
    assert_eq!(f.env.git(&f.down, &["tag", "--list"]), "");
}

#[test]
fn a_published_release_at_the_tip_is_left_alone() {
    let f = forge_fork();
    let name = format!("v0.11.1-custom.{DAY}.1");
    f.tool(&["release", "tag"]).success();
    f.forge().publish_release(&name);
    let doc = next_json(&f);
    assert_eq!(doc["release"], name.as_str());
    assert_eq!(doc["published"], true);
    assert_eq!(doc["next"], serde_json::Value::Null);
    let out = stdout(&f.tool(&["release", "tag"]).success());
    assert!(
        out.contains(&format!("the published release {name}")),
        "{out}"
    );
    assert_eq!(remote_tags(&f), vec![name]);
    assert!(f.forge().changes().is_empty());
}

#[test]
fn an_unpublished_release_at_the_tip_fails_the_scheduled_run() {
    let f = forge_fork();
    let name = format!("v0.11.1-custom.{DAY}.1");
    f.tool(&["release", "tag"]).success();
    let doc = next_json(&f);
    assert_eq!(doc["release"], name.as_str());
    assert_eq!(doc["published"], false);
    // The next day's scheduled run.
    f.env.set_var("GIT_INTAKE_TEST_NOW", "2026-11-02T03:00:00Z");
    let out = stdout(&f.tool(&["release", "tag"]).code(1));
    assert!(
        out.contains(&format!("the release {name}, which is not published"))
            && out.contains("running its pipeline's failed jobs again"),
        "{out}"
    );
    assert_eq!(remote_tags(&f), vec![name]);
}

#[test]
fn a_tag_another_run_pushed_first_is_reported() {
    let f = release_fork();
    let env = &f.env;
    let name = format!("v0.11.1-custom.{DAY}.1");
    let tip = env.rev(&f.publish, "main");
    // The other run pushes the same tag just before this run's push.
    let hook = f.down.join(".git/hooks/pre-push");
    fs::write(
        &hook,
        format!(
            "#!/bin/sh\ngit --git-dir='{}' tag {name} {tip}\n",
            f.publish.display()
        ),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
    let err = stderr(&f.tool(&["release", "tag"]).code(2));
    assert!(
        err.contains(&format!("another run created the tag {name}")) && err.contains("again"),
        "{err}"
    );
    // The remote's tag is the other run's lightweight tag.
    assert_eq!(env.git(&f.publish, &["cat-file", "-t", &name]), "commit");
    assert!(!env.has_rev(&f.down, &format!("refs/tags/{name}")));
}

#[test]
fn a_publish_remote_that_pushes_to_the_upstream_is_refused() {
    let f = release_fork();
    let env = &f.env;
    env.git(
        &f.down,
        &["remote", "set-url", "--push", "origin", &f.up.url()],
    );
    let err = stderr(&f.tool(&["release", "tag"]).code(2));
    assert!(err.contains("never pushes to an upstream"), "{err}");
    assert_eq!(env.git(&f.up.bare, &["tag", "--list"]), "v0.11.0");
}

#[test]
fn release_check_passes_an_unpublished_release_in_its_pipeline() {
    let f = forge_fork();
    let name = format!("v0.11.1-custom.{DAY}.1");
    f.tool(&["release", "tag"]).success();
    let ci = ci_clone(&f, &name);
    let env = &f.env;
    let out = stdout(
        &env.tool_env(&ci, &["release", "check"], &[("CI_COMMIT_TAG", &name)])
            .success(),
    );
    assert!(out.contains(&format!("{name} may be released")), "{out}");
    // A pipeline run again passes the same way, and creates no branch.
    env.tool_env(&ci, &["release", "check"], &[("CI_COMMIT_TAG", &name)])
        .success();
    let refs = env.git(
        &ci,
        &[
            "for-each-ref",
            "--format=%(refname)",
            "refs/heads/",
            "refs/tags/",
        ],
    );
    assert_eq!(refs, format!("refs/tags/{name}"));
    // Once published, the release is never written again.
    f.forge().publish_release(&name);
    let out = stdout(&env.tool(&ci, &["release", "check", &name]).code(1));
    assert!(out.contains("is published already"), "{out}");
}

#[test]
fn release_check_reads_the_forge_with_the_job_token() {
    let f = forge_fork();
    let name = format!("v0.11.1-custom.{DAY}.1");
    f.tool(&["release", "tag"]).success();
    let ci = ci_clone(&f, &name);
    let env = &f.env;
    env.unset_var("GITLAB_TOKEN");
    f.forge().world().private = true;
    f.forge().clear_requests();
    env.tool_env(
        &ci,
        &["release", "check"],
        &[
            ("CI_COMMIT_TAG", &name),
            ("CI_JOB_TOKEN", "job-token-of-the-pipeline"),
        ],
    )
    .success();
    let requests = f.forge().requests();
    assert!(!requests.is_empty());
    for r in &requests {
        assert_eq!(r.method, "GET");
        assert_eq!(
            r.header("job-token"),
            Some("job-token-of-the-pipeline"),
            "{r:?}"
        );
        assert_eq!(r.header("private-token"), None);
    }
    assert!(
        requests
            .iter()
            .any(|r| r.path.ends_with(&format!("/releases/{name}")))
    );
}

#[test]
fn release_tag_refuses_to_push_with_the_job_token() {
    let f = forge_fork();
    f.env.set_var("CI_JOB_TOKEN", "job-token-of-the-pipeline");
    f.env.git(
        &f.down,
        &[
            "remote",
            "set-url",
            "--push",
            "origin",
            "https://gitlab-ci-token:job-token-of-the-pipeline@gitlab.example.com/team/app.git",
        ],
    );
    let err = stderr(&f.tool(&["release", "tag"]).code(2));
    assert!(
        err.contains("would push to 'origin' with its job token")
            && err.contains("the push URL carries it")
            && !err.contains("job-token-of-the-pipeline"),
        "{err}"
    );
    assert_eq!(f.env.git(&f.down, &["tag", "--list"]), "");
}

#[test]
fn a_private_project_without_a_token_is_refused() {
    let f = forge_fork();
    f.tool(&["release", "tag"]).success();
    f.env.unset_var("GITLAB_TOKEN");
    f.forge().world().private = true;
    let a = f.tool(&["release", "next", "--json"]).code(2);
    assert_eq!(stdout(&a), "");
    let err = stderr(&a);
    assert!(
        err.contains("without a token")
            && err.contains("GITLAB_TOKEN")
            && err.contains("CI_JOB_TOKEN"),
        "{err}"
    );
}

#[test]
fn release_check_reports_a_tag_off_the_line() {
    let f = release_fork();
    let env = &f.env;
    env.git(&f.down, &["switch", "-q", "-c", "feature"]);
    let side = env.commit(&f.down, &[("side.txt", "s\n")], "feat: side");
    env.git(&f.down, &["push", "-q", "origin", "feature"]);
    let name = format!("v0.11.1-custom.{DAY}.1");
    release_tag(&f, &name, &side);
    let out = stdout(
        &env.tool_env(&f.down, &["release", "check"], &[("CI_COMMIT_TAG", &name)])
            .code(1),
    );
    assert!(
        out.contains(&format!(
            "the checked-out commit {} is not on the downstream's line of 'main'",
            &side[..12]
        )),
        "{out}"
    );
}

#[test]
fn release_check_reports_every_problem_of_a_tag() {
    let f = release_fork();
    let env = &f.env;
    let tip = env.rev(&f.down, "main");
    release_tag(&f, &format!("v0.11.1-custom.{DAY}.4"), &tip);
    let name = format!("v0.10.1-custom.{DAY}.2");
    release_tag(&f, &name, &tip);
    let out = stdout(&env.tool(&f.down, &["release", "check", &name]).code(1));
    assert!(
        out.contains("carries the base 0.10.1, but the release base"),
        "{out}"
    );
    assert!(
        out.contains(&format!(
            "does not sort after the release v0.11.1-custom.{DAY}.4"
        )),
        "{out}"
    );
    let out = stdout(
        &env.tool(&f.down, &["release", "check", "v0.11.1-other.20261101.1"])
            .code(1),
    );
    assert!(out.contains("with the current label 'custom'"), "{out}");
    assert!(
        out.contains("has no tag 'v0.11.1-other.20261101.1'"),
        "{out}"
    );
}

#[test]
fn release_version_gives_builds_the_version() {
    let f = release_fork();
    let env = &f.env;
    let name = "v0.11.1-custom.20261101.2";
    let tip = env.rev(&f.down, "main");
    release_tag(&f, name, &tip);
    let ci = ci_clone(&f, name);
    let ci_tag = [("CI_COMMIT_TAG", name)];
    let out = stdout(
        &env.tool_env(&ci, &["release", "version"], &ci_tag)
            .success(),
    );
    assert_eq!(out, "0.11.1-custom.20261101.2\n");
    let out = stdout(
        &env.tool_env(&ci, &["release", "version", "--dotenv"], &ci_tag)
            .success(),
    );
    assert_eq!(
        out,
        "RELEASE_VERSION=0.11.1-custom.20261101.2\nRELEASE_TAG=v0.11.1-custom.20261101.2\n"
    );
    let a = env
        .tool_env(&ci, &["release", "version", "--json"], &ci_tag)
        .success();
    let doc: serde_json::Value = serde_json::from_slice(&a.get_output().stdout).unwrap();
    assert_eq!(doc["version"], "0.11.1-custom.20261101.2");
    assert_eq!(doc["tag"], name);
    assert_eq!(doc["rehearsal"], false);
    assert!(doc.get("upstream").is_none(), "{doc}");
    // A tag whose base is not the commit's is refused.
    release_tag(&f, "v0.12.1-custom.20261101.3", &tip);
    let err = stderr(
        &env.tool(
            &f.down,
            &["release", "version", "--tag", "v0.12.1-custom.20261101.3"],
        )
        .code(2),
    );
    assert!(err.contains("carries the base 0.12.1"), "{err}");
}

#[test]
fn release_version_rehearses_without_a_tag() {
    let f = release_fork();
    let env = &f.env;
    env.unset_var("GIT_INTAKE_TEST_NOW");
    let ci = ci_clone(&f, "origin/main");
    let out = stdout(
        &env.tool_env(
            &ci,
            &["release", "version"],
            &[("CI_PIPELINE_CREATED_AT", "2026-11-01T08:15:00Z")],
        )
        .success(),
    );
    assert_eq!(out, "0.11.1-custom.20261101.0\n");
    let out = stdout(
        &env.tool_env(
            &ci,
            &["release", "version", "--dotenv"],
            &[("CI_PIPELINE_CREATED_AT", "2026-11-01T08:15:00Z")],
        )
        .success(),
    );
    assert_eq!(out, "RELEASE_VERSION=0.11.1-custom.20261101.0\n");
}

#[test]
fn a_tag_pipeline_run_again_uses_the_tagged_configuration() {
    let f = forge_fork();
    let env = &f.env;
    let name = format!("v0.11.1-custom.{DAY}.1");
    f.tool(&["release", "tag"]).success();
    f.reconfigure("tags = \"v*\"\n", "tags = \"v*\"\nlabels = [\"nightly\"]\n");
    let ci = ci_clone(&f, &name);
    let ci_tag = [("CI_COMMIT_TAG", name.as_str())];
    env.tool_env(&ci, &["release", "check"], &ci_tag).success();
    let out = stdout(
        &env.tool_env(&ci, &["release", "version"], &ci_tag)
            .success(),
    );
    assert_eq!(out, format!("0.11.1-custom.{DAY}.1\n"));
}

#[test]
fn notes_list_the_downstream_s_changes_and_updates() {
    let f = release_fork();
    let env = &f.env;
    let first = format!("v0.11.1-custom.{DAY}.1");
    release_tag(&f, &first, &env.rev(&f.down, "main"));
    let out = stdout(&f.tool(&["release", "notes", &first]).success());
    assert_eq!(out, "This is the first release.\n");

    land(&f, "widget", "feat: add the widget");
    let target = env.push_upstream(&f.up, &[("two.txt", "2\n")], "upstream: two");
    f.tool(&["sync"]).success();
    env.git(&f.down, &["pull", "-q", "--ff-only", "origin", "main"]);
    let second = format!("v0.11.1-custom.{DAY}.2");
    release_tag(&f, &second, &env.rev(&f.down, "main"));
    let out = stdout(
        &env.tool_env(
            &f.down,
            &["release", "notes"],
            &[("CI_COMMIT_TAG", &second)],
        )
        .success(),
    );
    assert_eq!(
        out,
        format!(
            "Changes since {first}:\n\n- Update upstream to `{}`\n- feat: add the widget\n",
            &target[..12]
        )
    );
    assert!(!out.contains("upstream: two"));

    // A release without changes of its own.
    let third = format!("v0.11.1-custom.{DAY}.3");
    release_tag(&f, &third, &env.rev(&f.down, "main"));
    let out = stdout(&f.tool(&["release", "notes", &third]).success());
    assert_eq!(
        out,
        format!("No changes since the previous release, {second}.\n")
    );
}

#[test]
fn notes_name_the_upstream_tag_an_update_took() {
    let f = Fork::new();
    f.reconfigure("mode = \"follow\"\n", "mode = \"follow\"\ntags = \"v*\"\n");
    releasable(&f, RELEASE);
    let env = &f.env;
    let first = format!("v0.11.1-custom.{DAY}.1");
    release_tag(&f, &first, &env.rev(&f.down, "main"));
    let c = env.push_upstream(&f.up, &[("two.txt", "2\n")], "upstream: two");
    upstream_tag(&f, "v0.12.0", &c);
    f.tool(&["sync"]).success();
    env.git(&f.down, &["pull", "-q", "--ff-only", "origin", "main"]);
    let second = format!("v0.12.1-custom.{DAY}.2");
    release_tag(&f, &second, &env.rev(&f.down, "main"));
    let out = stdout(&f.tool(&["release", "notes", &second]).success());
    assert_eq!(
        out,
        format!("Changes since {first}:\n\n- Update upstream to v0.12.0\n")
    );
}

#[test]
fn status_shows_the_release_state() {
    let f = release_fork();
    let doc = f.env.tool_json(&f.down, &["status", "--json"], 0);
    assert_eq!(doc["release"]["next"], format!("v0.11.1-custom.{DAY}.1"));
    assert_eq!(doc["release_error"], serde_json::Value::Null);
    let out = stdout(&f.tool(&["status"]).success());
    assert!(
        out.contains("release base     0.11.1 (upstream 'upstream' at v0.11.0)"),
        "{out}"
    );
    // Without [release], `release` is null.
    let plain = Fork::new();
    let doc = plain.env.tool_json(&plain.down, &["status", "--json"], 0);
    assert_eq!(doc["release"], serde_json::Value::Null);
}

#[test]
fn status_without_the_forge_leaves_publication_unknown() {
    // The publish remote is a local path, on no forge.
    let f = release_fork();
    f.tool(&["release", "tag"]).success();
    let doc = f.env.tool_json(&f.down, &["status", "--json"], 0);
    let release = &doc["release"];
    assert_eq!(release["release"], format!("v0.11.1-custom.{DAY}.1"));
    assert_eq!(release["published"], serde_json::Value::Null);
    assert!(
        release["forge_error"]
            .as_str()
            .unwrap()
            .contains("names no host"),
        "{release}"
    );
}

#[test]
fn the_release_commands_never_change_the_upstream() {
    let f = forge_fork();
    f.tool(&["release", "tag"]).success();
    assert_eq!(f.env.git(&f.up.bare, &["tag", "--list"]), "v0.11.0");
    assert_eq!(
        branch_tip(&f.up.bare, "main"),
        Some(f.env.rev(&f.up.work, "HEAD"))
    );
}

#[test]
fn status_names_a_release_state_it_cannot_read() {
    let f = Fork::new();
    append_config(&f, RELEASE);
    let doc = f.env.tool_json(&f.down, &["status", "--json"], 0);
    assert_eq!(doc["release"], serde_json::Value::Null);
    let why = doc["release_error"].as_str().unwrap();
    assert!(
        why.contains("no release of upstream 'upstream'") && why.contains("'v*'"),
        "{why}"
    );
}

#[test]
fn releases_are_read_from_github_in_merge_request_mode() {
    let f = Fork::github();
    releasable(&f, RELEASE);
    let name = format!("v0.11.1-custom.{DAY}.1");
    f.tool(&["release", "tag"]).success();
    assert_eq!(next_json(&f)["published"], false);
    f.forge().publish_release(&name);
    f.forge().clear_requests();
    assert_eq!(next_json(&f)["published"], true);
    let requests = f.forge().requests();
    assert!(
        requests
            .iter()
            .any(|r| r.path.ends_with(&format!("/releases/tags/{name}")))
    );
    for r in &requests {
        assert_eq!(r.method, "GET");
        assert_eq!(
            r.header("authorization"),
            Some(format!("Bearer {}", common::forge::TOKEN).as_str())
        );
    }
}

#[test]
fn release_tag_pushes_only_the_release_tag_whatever_push_follow_tags_says() {
    let f = release_fork();
    let env = &f.env;
    env.git(&f.down, &["config", "push.followTags", "true"]);
    env.git(
        &f.down,
        &["tag", "-a", "-m", "a note", "local-note", "main"],
    );
    f.tool(&["release", "tag"]).success();
    assert_eq!(remote_tags(&f), vec![format!("v0.11.1-custom.{DAY}.1")]);
}

#[test]
fn the_release_fetch_leaves_the_repository_s_tags_and_branches() {
    let f = release_fork();
    let env = &f.env;
    let name = format!("v0.11.1-custom.{DAY}.1");
    let local = env.rev(&f.down, "main");
    let landed = land(&f, "feature", "feat: a feature");
    env.git(&f.down, &["reset", "-q", "--hard", &local]);
    release_tag(&f, &name, &landed);
    // The repository's own tag of the same name points elsewhere.
    env.git(&f.down, &["tag", "-f", &name, &local]);
    for spec in [
        "+refs/tags/*:refs/tags/*",
        "+refs/heads/*:refs/heads/mirror/*",
    ] {
        env.git(&f.down, &["config", "--add", "remote.origin.fetch", spec]);
    }
    f.tool(&["release", "next"]).success();
    f.tool(&["status"]).success();
    assert_eq!(env.rev(&f.down, &format!("{name}^{{commit}}")), local);
    assert!(!env.has_rev(&f.down, "refs/heads/mirror/main"));
    assert_eq!(env.rev(&f.down, "main"), local);
}

#[test]
fn a_release_form_tag_off_the_line_takes_its_number_of_the_day() {
    let f = release_fork();
    let env = &f.env;
    env.git(&f.down, &["switch", "-q", "-c", "feature"]);
    let side = env.commit(&f.down, &[("side.txt", "s\n")], "feat: side");
    env.git(&f.down, &["switch", "-q", "main"]);
    release_tag(&f, &format!("v0.11.1-custom.{DAY}.1"), &side);
    assert_eq!(next_json(&f)["next"], format!("v0.11.1-custom.{DAY}.2"));
    f.tool(&["release", "tag"]).success();
    assert!(remote_tags(&f).contains(&format!("v0.11.1-custom.{DAY}.2")));
}

#[test]
fn an_unpublished_release_below_the_tip_is_shown_and_does_not_stop_the_next() {
    let f = forge_fork();
    let first = format!("v0.11.1-custom.{DAY}.1");
    f.tool(&["release", "tag"]).success();
    land(&f, "feature", "feat: a feature");
    let doc = next_json(&f);
    assert_eq!(doc["release"], serde_json::Value::Null);
    assert_eq!(doc["unpublished_previous"], first.as_str());
    assert_eq!(doc["next"], format!("v0.11.1-custom.{DAY}.2"));
    let status = f.env.tool_json(&f.down, &["status", "--json"], 0);
    assert_eq!(status["release"]["unpublished_previous"], first.as_str());
    let a = f.tool(&["release", "tag"]).code(0);
    assert!(
        stderr(&a).contains(&format!(
            "warning: the release {first} below the tip is not published"
        )),
        "{}",
        stderr(&a)
    );
    assert!(remote_tags(&f).contains(&format!("v0.11.1-custom.{DAY}.2")));
    // Once the releases are published, nothing is shown.
    f.forge().publish_release(&first);
    f.forge()
        .publish_release(&format!("v0.11.1-custom.{DAY}.2"));
    land(&f, "more", "feat: more");
    assert_eq!(
        next_json(&f)["unpublished_previous"],
        serde_json::Value::Null
    );
}

const SEMVER: &str =
    "\n[release]\nupstream = \"upstream\"\ntags = \"v*\"\nversioning = \"semver\"\n";

/// A fork releasable under `semver`, with a feature of its own: the
/// fixture's `fork:` commits ask for no release.
fn semver_fork() -> Fork {
    let f = Fork::new();
    releasable(&f, SEMVER);
    land(&f, "own", "feat: the fork's own feature");
    f
}

fn semver_forge_fork() -> Fork {
    let f = Fork::push_gitlab();
    releasable(&f, SEMVER);
    land(&f, "own", "feat: the fork's own feature");
    f
}

/// Takes the publish remote's integration branch into the fork's checkout.
fn pull(f: &Fork) {
    f.env
        .git(&f.down, &["pull", "-q", "--ff-only", "origin", "main"]);
}

#[test]
fn the_first_semver_release_is_1_0_0() {
    let f = Fork::new();
    releasable(&f, SEMVER);
    // The fixture's `fork:` and `upstream:` commits ask for none.
    assert_eq!(next_json(&f)["next"], serde_json::Value::Null);
    land(&f, "fix", "fix: a fix of the fork");
    let doc = next_json(&f);
    assert_eq!(doc["versioning"], "semver");
    assert_eq!(doc["base"], serde_json::Value::Null);
    assert_eq!(doc["upstream_release"], "v0.11.0");
    assert_eq!(doc["bump"], "major");
    assert_eq!(doc["next"], "v1.0.0");
    let out = stdout(&f.tool(&["status"]).success());
    assert!(
        out.contains("upstream release v0.11.0 (upstream 'upstream')")
            && out.contains("next release     v1.0.0, a major change")
            && !out.contains("release base"),
        "{out}"
    );

    f.tool(&["release", "tag"]).success();
    assert_eq!(remote_tags(&f), vec!["v1.0.0".to_string()]);
    let message = f
        .env
        .git(&f.publish, &["tag", "-l", "--format=%(contents)", "v1.0.0"]);
    let released = f.env.rev(&f.up.work, "v0.11.0");
    assert_eq!(
        message,
        format!(
            "Release 1.0.0\n\nIntake-Upstream: upstream\nIntake-Target: {released}\n\
             Intake-Tag: v0.11.0"
        )
    );
    let doc = next_json(&f);
    assert_eq!(doc["release"], "v1.0.0");
    assert_eq!(doc["bump"], serde_json::Value::Null);
    assert_eq!(doc["next"], serde_json::Value::Null);
}

#[test]
fn semver_releases_follow_the_conventional_commits_since_the_previous_one() {
    let f = semver_fork();
    f.tool(&["release", "tag"]).success();
    let next = |f: &Fork| {
        let doc = next_json(f);
        (doc["bump"].clone(), doc["next"].clone())
    };
    let none = (serde_json::Value::Null, serde_json::Value::Null);

    land(&f, "docs", "docs: describe the widget");
    land(
        &f,
        "ci",
        "ci: cache the build\n\nBREAKING: not a footer of a breaking change",
    );
    assert_eq!(next(&f), none);
    let out = stdout(&f.tool(&["release", "tag"]).success());
    assert!(
        out.contains("the commits since the release v1.0.0 ask for no release; nothing to release"),
        "{out}"
    );
    assert_eq!(remote_tags(&f), vec!["v1.0.0".to_string()]);
    let out = stdout(&f.tool(&["release", "next"]).success());
    assert!(
        out.contains(
            "next release     none: the commits since the release v1.0.0 ask for no release"
        ),
        "{out}"
    );
    let status = f.env.tool_json(&f.down, &["status", "--json"], 0);
    assert_eq!(status["release"]["next"], serde_json::Value::Null);
    assert_eq!(status["release"]["bump"], serde_json::Value::Null);

    land(&f, "revert", "Revert \"docs: describe the widget\"");
    assert_eq!(next(&f), ("patch".into(), "v1.0.1".into()));
    land(&f, "fix", "fix: a fix");
    assert_eq!(next(&f), ("patch".into(), "v1.0.1".into()));
    land(&f, "feat", "Feat(widget): a feature");
    assert_eq!(next(&f), ("minor".into(), "v1.1.0".into()));
    f.tool(&["release", "tag"]).success();

    land(&f, "perf", "perf: faster");
    assert_eq!(next(&f), ("patch".into(), "v1.1.1".into()));
    land(
        &f,
        "rename",
        "refactor: rename the service\n\nBREAKING CHANGE: clients use the new name",
    );
    assert_eq!(next(&f), ("major".into(), "v2.0.0".into()));
    f.tool(&["release", "tag"]).success();
    land(&f, "bang", "chore!: drop the old protocol");
    assert_eq!(next(&f), ("major".into(), "v3.0.0".into()));
}

#[test]
fn semver_releases_count_the_upstream_commits_an_update_brought() {
    let f = semver_fork();
    f.tool(&["release", "tag"]).success();
    f.env
        .push_upstream(&f.up, &[("d.txt", "d\n")], "docs: upstream docs");
    f.tool(&["sync"]).success();
    // The update's merge is no commit of the release.
    assert_eq!(next_json(&f)["next"], serde_json::Value::Null);
    f.env
        .push_upstream(&f.up, &[("w.txt", "w\n")], "feat: upstream widget");
    f.tool(&["sync"]).success();
    assert_eq!(next_json(&f)["next"], "v1.1.0");
}

#[test]
fn an_incompatible_upstream_release_raises_the_major_version() {
    let f = semver_fork();
    f.tool(&["release", "tag"]).success();
    let c = f.env.push_upstream(&f.up, &[("a.txt", "a\n")], "fix: a");
    upstream_tag(&f, "v0.11.3", &c);
    f.tool(&["sync"]).success();
    let doc = next_json(&f);
    assert_eq!(
        (doc["upstream_release"].clone(), doc["next"].clone()),
        ("v0.11.3".into(), "v1.0.1".into())
    );
    f.tool(&["release", "tag"]).success();

    let c = f.env.push_upstream(&f.up, &[("b.txt", "b\n")], "fix: b");
    upstream_tag(&f, "v0.12.0", &c);
    f.tool(&["sync"]).success();
    let doc = next_json(&f);
    assert_eq!(
        (doc["bump"].clone(), doc["next"].clone()),
        ("major".into(), "v2.0.0".into())
    );
}

#[test]
fn dated_releases_count_before_the_first_semver_release() {
    let f = release_fork();
    f.tool(&["release", "tag"]).success();
    let dated = format!("v0.11.1-custom.{DAY}.1");
    f.reconfigure(
        "tags = \"v*\"\n",
        "tags = \"v*\"\nversioning = \"semver\"\n",
    );
    land(&f, "fix", "fix: a");
    let doc = next_json(&f);
    assert_eq!(
        (doc["bump"].clone(), doc["next"].clone()),
        ("major".into(), "v1.0.0".into())
    );
    f.tool(&["release", "tag"]).success();
    assert_eq!(remote_tags(&f), vec![dated, "v1.0.0".to_string()]);

    // A dated release at 1.0.0 or above.
    let g = release_fork();
    release_tag(&g, "v1.4.1-custom.20261031.1", &g.env.rev(&g.down, "main"));
    g.reconfigure(
        "tags = \"v*\"\n",
        "tags = \"v*\"\nversioning = \"semver\"\n",
    );
    land(&g, "fix", "fix: a");
    assert_eq!(next_json(&g)["next"], "v2.0.0");
}

#[test]
fn a_plain_version_tag_is_no_dated_release() {
    let f = release_fork();
    release_tag(&f, "v1.0.0", &f.env.rev(&f.down, "main"));
    let doc = next_json(&f);
    assert_eq!(doc["versioning"], "dated");
    assert_eq!(doc["release"], serde_json::Value::Null);
    assert_eq!(doc["bump"], serde_json::Value::Null);
    assert_eq!(doc["next"], format!("v0.11.1-custom.{DAY}.1"));
}

#[test]
fn a_semver_release_below_a_higher_one_is_refused() {
    let f = semver_fork();
    let first = f.env.rev(&f.down, "main");
    release_tag(&f, "v5.0.0", &first);
    let second = land(&f, "fix", "fix: a");
    release_tag(&f, "v1.0.0", &second);
    land(&f, "more", "fix: b");
    let err = stderr(&f.tool(&["release", "next"]).code(2));
    assert!(
        err.contains("the next release would be v1.0.1")
            && err.contains("does not sort after the release v5.0.0"),
        "{err}"
    );
}

#[test]
fn release_check_recomputes_the_version_of_a_semver_tag() {
    let f = semver_forge_fork();
    f.tool(&["release", "tag"]).success();
    let env = &f.env;
    let ci = ci_clone(&f, "v1.0.0");
    let out = stdout(
        &env.tool_env(&ci, &["release", "check"], &[("CI_COMMIT_TAG", "v1.0.0")])
            .success(),
    );
    assert!(out.contains("v1.0.0 may be released"), "{out}");

    let fix = land(&f, "fix", "fix: a");
    release_tag(&f, "v2.0.0", &fix);
    let out = stdout(&env.tool(&f.down, &["release", "check", "v2.0.0"]).code(1));
    assert!(
        out.contains(&format!(
            "'v2.0.0' carries the version 2.0.0, but {} takes 1.0.1, a patch change since the release v1.0.0",
            &fix[..12]
        )),
        "{out}"
    );
    let docs = land(&f, "docs", "docs: a");
    release_tag(&f, "v2.0.1", &docs);
    let out = stdout(&env.tool(&f.down, &["release", "check", "v2.0.1"]).code(1));
    assert!(
        out.contains(&format!(
            "'v2.0.1' is on {}, which takes no release: its commits since the release v2.0.0 ask for none",
            &docs[..12]
        )),
        "{out}"
    );
    let out = stdout(
        &env.tool(&f.down, &["release", "check", "v2.0.2-rc.1"])
            .code(1),
    );
    assert!(
        out.contains("'v2.0.2-rc.1' is not a release tag v<x.y.z>"),
        "{out}"
    );
}

#[test]
fn release_tag_warns_of_an_unpublished_release_when_nothing_is_due() {
    let f = semver_forge_fork();
    f.tool(&["release", "tag"]).success();
    land(&f, "docs", "docs: a");
    let a = f.tool(&["release", "tag"]).code(0);
    assert!(
        stderr(&a).contains("warning: the release v1.0.0 below the tip is not published"),
        "{}",
        stderr(&a)
    );
    assert!(stdout(&a).contains("nothing to release"), "{}", stdout(&a));
    assert_eq!(remote_tags(&f), vec!["v1.0.0".to_string()]);
    assert_eq!(next_json(&f)["unpublished_previous"], "v1.0.0");
}

#[test]
fn release_version_of_a_semver_release_names_its_upstream() {
    let f = semver_fork();
    f.tool(&["release", "tag"]).success();
    let env = &f.env;
    let ci = ci_clone(&f, "v1.0.0");
    let ci_tag = [("CI_COMMIT_TAG", "v1.0.0")];
    let out = stdout(
        &env.tool_env(&ci, &["release", "version"], &ci_tag)
            .success(),
    );
    assert_eq!(out, "1.0.0\n");
    let out = stdout(
        &env.tool_env(&ci, &["release", "version", "--dotenv"], &ci_tag)
            .success(),
    );
    assert_eq!(
        out,
        "RELEASE_VERSION=1.0.0\nRELEASE_TAG=v1.0.0\nRELEASE_UPSTREAM=upstream-0.11.0\n"
    );
    let a = env
        .tool_env(&ci, &["release", "version", "--json"], &ci_tag)
        .success();
    let doc: serde_json::Value = serde_json::from_slice(&a.get_output().stdout).unwrap();
    assert_eq!(doc["version"], "1.0.0");
    assert_eq!(doc["tag"], "v1.0.0");
    assert_eq!(doc["rehearsal"], false);
    assert_eq!(doc["upstream"], "upstream-0.11.0");
    // A tag whose version is not the commit's is refused.
    release_tag(&f, "v3.0.0", &env.rev(&f.down, "main"));
    let err = stderr(
        &env.tool(&f.down, &["release", "version", "--tag", "v3.0.0"])
            .code(2),
    );
    assert!(
        err.contains("'v3.0.0' carries the version 3.0.0, but the tagged commit"),
        "{err}"
    );
}

#[test]
fn release_version_rehearses_the_next_semver_release() {
    let f = semver_fork();
    f.tool(&["release", "tag"]).success();
    let env = &f.env;
    let created = [("CI_PIPELINE_CREATED_AT", "2026-11-01T08:15:00Z")];
    land(&f, "docs", "docs: a");
    let out = stdout(
        &env.tool_env(&f.down, &["release", "version"], &created)
            .success(),
    );
    assert_eq!(out, "1.0.1-rehearsal.20261101\n");
    land(&f, "feat", "feat: a");
    let out = stdout(
        &env.tool_env(&f.down, &["release", "version", "--dotenv"], &created)
            .success(),
    );
    assert_eq!(
        out,
        "RELEASE_VERSION=1.1.0-rehearsal.20261101\nRELEASE_UPSTREAM=upstream-0.11.0\n"
    );
}

#[test]
fn semver_notes_name_the_upstream_and_put_breaking_changes_first() {
    let f = semver_fork();
    let env = &f.env;
    f.tool(&["release", "tag"]).success();
    let out = stdout(&f.tool(&["release", "notes", "v1.0.0"]).success());
    assert_eq!(
        out,
        "Carries upstream v0.11.0.\n\nThis is the first release.\n"
    );

    env.push_upstream(&f.up, &[("one.txt", "1\n")], "upstream: one");
    let target = env.push_upstream(&f.up, &[("two.txt", "2\n")], "upstream: two");
    f.tool(&["sync"]).success();
    pull(&f);
    land(&f, "fix", "fix: a");
    land(
        &f,
        "protocol",
        "feat!: speak protocol 3\n\nBREAKING CHANGE: clients need\nprotocol 3",
    );
    f.tool(&["release", "tag"]).success();
    let out = stdout(&f.tool(&["release", "notes", "v2.0.0"]).success());
    assert_eq!(
        out,
        format!(
            "Carries upstream v0.11.0 and 2 upstream commits after it.\n\n\
             Breaking changes since v1.0.0:\n\n\
             - feat!: speak protocol 3\n\n  clients need\n  protocol 3\n\n\
             Other changes since v1.0.0:\n\n\
             - fix: a\n- Update upstream to `{}`\n",
            &target[..12]
        )
    );

    land(&f, "bang", "fix!: drop the flag");
    f.tool(&["release", "tag"]).success();
    let out = stdout(&f.tool(&["release", "notes", "v3.0.0"]).success());
    assert_eq!(
        out,
        "Carries upstream v0.11.0 and 2 upstream commits after it.\n\n\
         Breaking changes since v2.0.0:\n\n- fix!: drop the flag\n"
    );
}

/// Commits `message` signed with an SSH key on the fork's integration
/// branch, with `log.showSignature` set, and publishes it.
fn land_signed(f: &Fork, message: &str) -> String {
    let env = &f.env;
    let key = env.path("signing-key");
    if !key.exists() {
        let out = std::process::Command::new("ssh-keygen")
            .args(["-q", "-t", "ed25519", "-N", "", "-f"])
            .arg(&key)
            .output()
            .expect("ssh-keygen runs");
        assert!(out.status.success(), "{out:?}");
    }
    for (k, v) in [
        ("log.showSignature", "true"),
        ("gpg.format", "ssh"),
        ("user.signingkey", &key.to_string_lossy()),
    ] {
        env.git(&f.down, &["config", k, v]);
    }
    fs::write(f.down.join("signed.txt"), message).unwrap();
    env.git(&f.down, &["add", "signed.txt"]);
    env.git(&f.down, &["commit", "-q", "-S", "-m", message]);
    env.git(&f.down, &["push", "-q", "origin", "main"]);
    env.rev(&f.down, "main")
}

#[test]
fn signed_commits_read_alike_whatever_log_show_signature_says() {
    let f = release_fork();
    let first = format!("v0.11.1-custom.{DAY}.1");
    f.tool(&["release", "tag"]).success();
    land_signed(&f, "feat: a signed feature");
    f.tool(&["release", "tag"]).success();
    let second = format!("v0.11.1-custom.{DAY}.2");
    let out = stdout(&f.tool(&["release", "notes", &second]).success());
    assert_eq!(
        out,
        format!("Changes since {first}:\n\n- feat: a signed feature\n")
    );

    let g = semver_fork();
    g.tool(&["release", "tag"]).success();
    land_signed(&g, "feat: a signed feature");
    assert_eq!(next_json(&g)["next"], "v1.1.0");
}

#[test]
fn a_late_upstream_tag_changes_no_semver_release() {
    let f = semver_forge_fork();
    let env = &f.env;
    f.tool(&["release", "tag"]).success();
    f.forge().publish_release("v1.0.0");
    let late = env.push_upstream(&f.up, &[("u.txt", "u\n")], "fix: upstream fix");
    f.tool(&["sync"]).success();
    f.tool(&["release", "tag"]).success();
    // The upstream tags a commit that v1.0.1 already contains.
    upstream_tag(&f, "v0.12.0", &late);

    let ci = ci_clone(&f, "v1.0.1");
    let ci_tag = [("CI_COMMIT_TAG", "v1.0.1")];
    env.tool_env(&ci, &["release", "check"], &ci_tag).success();
    let out = stdout(
        &env.tool_env(&ci, &["release", "version", "--dotenv"], &ci_tag)
            .success(),
    );
    assert_eq!(
        out,
        "RELEASE_VERSION=1.0.1\nRELEASE_TAG=v1.0.1\nRELEASE_UPSTREAM=upstream-0.11.0\n"
    );
    let notes = stdout(&env.tool_env(&ci, &["release", "notes"], &ci_tag).success());
    assert!(
        notes.starts_with("Carries upstream v0.11.0 and 1 upstream commit after it.\n"),
        "{notes}"
    );

    // The next release takes the major version the upstream release asks for.
    pull(&f);
    land(&f, "fix", "fix: a");
    let doc = next_json(&f);
    assert_eq!(
        (doc["bump"].clone(), doc["next"].clone()),
        ("major".into(), "v2.0.0".into())
    );
}

#[test]
fn release_check_refuses_a_recorded_upstream_release_the_commit_lacks() {
    let f = semver_fork();
    let env = &f.env;
    let ahead = env.push_upstream(&f.up, &[("a.txt", "a\n")], "feat: ahead");
    upstream_tag(&f, "v0.12.0", &ahead);
    let tip = env.rev(&f.down, "main");
    env.git(
        &f.down,
        &[
            "tag",
            "-a",
            "-m",
            &format!(
                "Release 1.0.0\n\nIntake-Upstream: upstream\nIntake-Target: {ahead}\n\
                 Intake-Tag: v0.12.0"
            ),
            "v1.0.0",
            &tip,
        ],
    );
    env.git(&f.down, &["push", "-q", "origin", "v1.0.0"]);
    let out = stdout(&f.tool(&["release", "check", "v1.0.0"]).code(1));
    assert!(
        out.contains(&format!(
            "'v1.0.0' records an upstream release that does not hold: {} does not contain the upstream \
             release upstream v0.12.0 at {}",
            &tip[..12],
            &ahead[..12]
        )),
        "{out}"
    );
}

#[test]
fn a_semver_tip_without_an_upstream_release_is_refused() {
    let f = Fork::new();
    append_config(&f, SEMVER);
    let err = stderr(&f.tool(&["release", "next"]).code(2));
    assert!(
        err.contains("contains no release of upstream 'upstream'")
            && err
                .contains("a semver release names the newest upstream release its commit contains"),
        "{err}"
    );
    let g = Fork::new();
    append_config(&g, RELEASE);
    let err = stderr(&g.tool(&["release", "next"]).code(2));
    assert!(
        err.contains("the release base comes from the newest upstream release a commit contains"),
        "{err}"
    );
}

#[test]
fn a_plain_upstream_subject_is_a_patch() {
    let f = semver_fork();
    f.tool(&["release", "tag"]).success();
    f.env
        .push_upstream(&f.up, &[("w.txt", "w\n")], "Add the widget");
    f.tool(&["sync"]).success();
    let doc = next_json(&f);
    assert_eq!(
        (doc["bump"].clone(), doc["next"].clone()),
        ("patch".into(), "v1.0.1".into())
    );
}

#[test]
fn a_semver_prerelease_tag_is_no_release() {
    let f = semver_fork();
    f.tool(&["release", "tag"]).success();
    let fix = land(&f, "fix", "fix: a");
    release_tag(&f, "v1.2.0-rc.1", &fix);
    let doc = next_json(&f);
    assert_eq!(doc["release"], serde_json::Value::Null);
    assert_eq!(doc["next"], "v1.0.1");
}

#[test]
fn a_lower_upstream_release_raises_the_major_version() {
    let f = semver_fork();
    let env = &f.env;
    let c = env.push_upstream(&f.up, &[("a.txt", "a\n")], "fix: a");
    upstream_tag(&f, "v0.11.3", &c);
    f.tool(&["sync"]).success();
    f.tool(&["release", "tag"]).success();
    // The upstream withdraws v0.11.3; the tip's newest release is v0.11.0.
    env.git(&f.up.work, &["push", "-q", "origin", ":refs/tags/v0.11.3"]);
    pull(&f);
    land(&f, "docs", "docs: a");
    let doc = next_json(&f);
    assert_eq!(doc["upstream_release"], "v0.11.0");
    assert_eq!(
        (doc["bump"].clone(), doc["next"].clone()),
        ("major".into(), "v2.0.0".into())
    );
}

#[test]
fn an_incompatible_upstream_release_makes_a_major_release_of_chores() {
    let f = semver_fork();
    let env = &f.env;
    f.tool(&["release", "tag"]).success();
    let c = env.push_upstream(&f.up, &[("d.txt", "d\n")], "docs: upstream docs");
    upstream_tag(&f, "v0.12.0", &c);
    f.tool(&["sync"]).success();
    let doc = next_json(&f);
    assert_eq!(
        (doc["bump"].clone(), doc["next"].clone()),
        ("major".into(), "v2.0.0".into())
    );
    f.tool(&["release", "tag"]).success();
    let out = stdout(&f.tool(&["release", "notes", "v2.0.0"]).success());
    assert_eq!(
        out,
        "Carries upstream v0.12.0.\n\n\
         Breaking changes since v1.0.0:\n\n\
         - Major: upstream upstream moved from v0.11.0 to v0.12.0\n\n\
         Other changes since v1.0.0:\n\n\
         - Update upstream to v0.12.0\n"
    );
}

#[test]
fn notes_count_no_upstream_commits_without_a_last_synced_commit() {
    let f = semver_fork();
    let env = &f.env;
    // The fork carries an upstream commit after v0.11.0, merged by hand
    // without a sync record.
    env.push_upstream(&f.up, &[("one.txt", "1\n")], "upstream: one");
    env.git(&f.down, &["fetch", "-q", &f.up.url(), "main"]);
    env.git(
        &f.down,
        &[
            "merge",
            "-q",
            "--no-ff",
            "-m",
            "Merge upstream",
            "FETCH_HEAD",
        ],
    );
    env.git(&f.down, &["push", "-q", "origin", "main"]);
    let out = stdout(&f.tool(&["release", "next"]).success());
    assert!(out.contains("next release     v1.0.0"), "{out}");
    // The upstream's branch then starts over on a history of its own, which
    // shares no commit with the fork.
    env.git(&f.up.work, &["switch", "-q", "--orphan", "restart"]);
    env.commit(&f.up.work, &[("new.txt", "n\n")], "restart");
    env.git(&f.up.work, &["push", "-q", "-f", "origin", "restart:main"]);
    f.tool(&["release", "tag"]).success();
    let out = stdout(&f.tool(&["release", "notes", "v1.0.0"]).success());
    assert_eq!(
        out,
        "Carries upstream v0.11.0.\n\nThis is the first release.\n"
    );
}

#[test]
fn release_version_fetches_no_upstream_that_has_a_sync_record() {
    let f = semver_fork();
    let env = &f.env;
    f.tool(&["release", "tag"]).success();
    env.push_upstream(&f.up, &[("u.txt", "u\n")], "fix: upstream fix");
    f.tool(&["sync"]).success();
    pull(&f);
    // The upstream's branch can no longer be fetched; its tags still list.
    env.git(&f.up.bare, &["update-ref", "-d", "refs/heads/main"]);
    let out = stdout(
        &env.tool_env(
            &f.down,
            &["release", "version"],
            &[("CI_PIPELINE_CREATED_AT", "2026-11-01T08:15:00Z")],
        )
        .success(),
    );
    assert_eq!(out, "1.0.1-rehearsal.20261101\n");
    f.tool(&["release", "next"]).code(2);
}

#[test]
fn a_withdrawn_or_moved_upstream_tag_changes_no_semver_release() {
    let f = semver_forge_fork();
    let env = &f.env;
    let released = env.push_upstream(&f.up, &[("a.txt", "a\n")], "fix: a");
    upstream_tag(&f, "v0.11.3", &released);
    f.tool(&["sync"]).success();
    f.tool(&["release", "tag"]).success();
    let ci = ci_clone(&f, "v1.0.0");
    let ci_tag = [("CI_COMMIT_TAG", "v1.0.0")];
    let stable = |ci: &std::path::Path| {
        let out = stdout(
            &env.tool_env(ci, &["release", "version", "--dotenv"], &ci_tag)
                .success(),
        );
        assert_eq!(
            out,
            "RELEASE_VERSION=1.0.0\nRELEASE_TAG=v1.0.0\nRELEASE_UPSTREAM=upstream-0.11.3\n"
        );
        let notes = stdout(&env.tool_env(ci, &["release", "notes"], &ci_tag).success());
        assert!(notes.starts_with("Carries upstream v0.11.3.\n"), "{notes}");
    };

    // The upstream withdraws the release: the record holds.
    env.git(&f.up.work, &["push", "-q", "origin", ":refs/tags/v0.11.3"]);
    env.tool_env(&ci, &["release", "check"], &ci_tag).success();
    stable(&ci);

    // The upstream moves the tag elsewhere: the check names it.
    let elsewhere = env.push_upstream(&f.up, &[("b.txt", "b\n")], "fix: b");
    env.git(&f.up.work, &["tag", "-f", "v0.11.3", &elsewhere]);
    env.git(&f.up.work, &["push", "-q", "-f", "origin", "v0.11.3"]);
    let out = stdout(&env.tool_env(&ci, &["release", "check"], &ci_tag).code(1));
    assert!(
        out.contains(&format!(
            "upstream 'upstream' now has v0.11.3 at {}, not at the recorded {}",
            &elsewhere[..12],
            &released[..12]
        )),
        "{out}"
    );
    stable(&ci);
}

#[test]
fn a_dated_tag_s_trailers_record_nothing() {
    let f = forge_fork();
    let env = &f.env;
    let ahead = env.push_upstream(&f.up, &[("a.txt", "a\n")], "feat: ahead");
    upstream_tag(&f, "v0.12.0", &ahead);
    let name = format!("v0.11.1-custom.{DAY}.1");
    let tip = env.rev(&f.down, "main");
    env.git(
        &f.down,
        &[
            "tag",
            "-a",
            "-m",
            &format!(
                "Release 0.11.1\n\nIntake-Upstream: upstream\nIntake-Target: {ahead}\n\
                 Intake-Tag: v0.12.0"
            ),
            &name,
            &tip,
        ],
    );
    env.git(&f.down, &["push", "-q", "origin", &name]);
    let out = stdout(&f.tool(&["release", "check", &name]).success());
    assert!(out.contains(&format!("{name} may be released")), "{out}");
}

#[test]
fn release_version_reads_the_own_sync_records_of_a_fork_of_a_fork() {
    // A follows its upstream and carries a sync record of it.
    let a = Fork::new();
    let env = &a.env;
    env.set_var("GIT_INTAKE_TEST_NOW", NOW);
    env.push_upstream(&a.up, &[("u.txt", "u\n")], "fix: upstream of a");
    a.tool(&["sync"]).success();
    pull(&a);
    release_tag(&a, "v5.0.0", &env.rev(&a.down, "main"));

    // B is cloned from A and follows it under the same upstream name, with
    // no sync record of its own yet.
    let b_publish = env.path("b-publish");
    let b = env.path("b");
    let root = env.path("");
    env.git(
        &root,
        &[
            "clone",
            "-q",
            "--bare",
            &a.publish.to_string_lossy(),
            &b_publish.to_string_lossy(),
        ],
    );
    env.git(
        &root,
        &[
            "clone",
            "-q",
            &b_publish.to_string_lossy(),
            &b.to_string_lossy(),
        ],
    );
    let cfg = fs::read_to_string(b.join(".git-intake.toml"))
        .unwrap()
        .replace(
            &format!("{:?}", a.up.url()),
            &format!("{:?}", a.publish.to_string_lossy()),
        )
        + SEMVER;
    env.commit(&b, &[(".git-intake.toml", &cfg)], "feat: release b");
    env.git(&b, &["push", "-q", "origin", "main"]);
    let created = [("CI_PIPELINE_CREATED_AT", "2026-11-01T08:15:00Z")];
    // A's record is not B's: B's line starts above A's tip, with no release.
    let out = stdout(
        &env.tool_env(&b, &["release", "version"], &created)
            .success(),
    );
    assert_eq!(out, "1.0.0-rehearsal.20261101\n");

    // Once B syncs A, its own record leaves A's records below it out, and
    // A's branch need not be fetched.
    land(&a, "a2", "feat: a2");
    env.tool(&b, &["sync"]).success();
    env.git(&b, &["pull", "-q", "--ff-only", "origin", "main"]);
    env.git(&a.publish, &["update-ref", "-d", "refs/heads/main"]);
    let out = stdout(
        &env.tool_env(&b, &["release", "version"], &created)
            .success(),
    );
    assert_eq!(out, "1.0.0-rehearsal.20261101\n");
    env.tool(&b, &["release", "next"]).code(2);
}
