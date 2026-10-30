//! Follow upstreams taken at their release tags, and the `Intake-Tag`
//! trailer that names the upstream tag of a merged target.

mod common;

use common::{Fork, stdout};
use predicates::str::contains;

/// Tags `commit` in the fork's upstream as `tag` and pushes the tag.
fn tag(f: &Fork, tag: &str, commit: &str) {
    f.env.git(&f.up.work, &["tag", tag, commit]);
    f.env.git(&f.up.work, &["push", "-q", "origin", tag]);
}

/// Tags `commit` with an annotated tag and pushes it.
fn annotated(f: &Fork, tag: &str, commit: &str) {
    f.env.git(
        &f.up.work,
        &["tag", "-a", "-m", &format!("release {tag}"), tag, commit],
    );
    f.env.git(&f.up.work, &["push", "-q", "origin", tag]);
}

/// A fork that takes its upstream at the release tags matching `v*`.
fn release_fork() -> Fork {
    let f = Fork::new();
    f.reconfigure("mode = \"follow\"\n", "mode = \"follow\"\ntags = \"v*\"\n");
    f
}

fn push(f: &Fork, n: &str) -> String {
    f.env.push_upstream(
        &f.up,
        &[(&format!("{n}.txt"), "x\n")],
        &format!("upstream: {n}"),
    )
}

/// The message of the integration tip, ending with a line break.
fn tip_message(f: &Fork) -> String {
    f.env.git(&f.down, &["log", "-1", "--format=%B", "main"]) + "\n"
}

fn cycle(f: &Fork, args: &[&str]) {
    f.tool(args).success();
    f.tool(&["verify"]).success();
    f.tool(&["finish"]).success();
    f.env.assert_no_update(&f.down);
}

#[test]
fn start_takes_the_newest_release_tag_on_the_branch() {
    let f = release_fork();
    let env = &f.env;
    let c1 = push(&f, "c1");
    let c2 = push(&f, "c2");
    let c3 = push(&f, "c3");
    let dev = push(&f, "dev");
    // Version order, not commit order: 0.10.0 is newer than 0.9.0.
    annotated(&f, "v0.10.0", &c2);
    tag(&f, "v0.9.0", &c3);
    tag(&f, "v0.2.0", &c1);
    // A prerelease, a name that is not a version and a tag outside the
    // pattern are never release targets, whatever they point at.
    tag(&f, "v0.11.0-rc.1", &dev);
    // Build metadata marks a build, not a release.
    tag(&f, "v0.20.0+build.1", &c3);
    tag(&f, "vnext", &dev);
    tag(&f, "release-9.0.0", &dev);
    // A newer release on a branch the upstream's main branch lacks.
    env.git(&f.up.work, &["switch", "-q", "-c", "maint", &c3]);
    let side = env.commit(&f.up.work, &[("side.txt", "s\n")], "upstream: side");
    env.git(&f.up.work, &["push", "-q", "origin", "maint"]);
    tag(&f, "v0.12.0", &side);
    env.git(&f.up.work, &["switch", "-q", "main"]);
    // The repository's own tags stay as they are.
    env.git(&f.down, &["tag", "own-tag", "main"]);

    let start = f.tool(&["start"]).success();
    let out = stdout(&start);
    assert!(
        out.contains(&format!("upstream: queued at {} (v0.10.0)", &c2[..12])),
        "{out}"
    );
    let status = env.tool_json(&f.down, &["status", "--json"], 0);
    let queued = &status["update"]["upstreams"][0];
    assert_eq!(queued["target"], c2.as_str());
    assert_eq!(queued["tag"], "v0.10.0");
    assert!(queued["rev"].is_null());
    f.tool(&["verify"]).success();
    f.tool(&["finish"]).success();
    env.assert_no_update(&f.down);

    assert_eq!(env.rev(&f.down, "main^2"), c2);
    let msg = tip_message(&f);
    assert!(
        msg.contains(&format!(
            "Intake-Upstream: upstream\nIntake-Target: {c2}\nIntake-Tag: v0.10.0\n"
        )),
        "{msg}"
    );
    assert!(
        msg.starts_with(&format!(
            "sync(upstream): merge upstream/main at {} (v0.10.0)",
            &c2[..7]
        )),
        "{msg}"
    );
    // Release tooling reads the merged release from the history alone.
    let merged = env.git(
        &f.down,
        &[
            "log",
            "-1",
            "--format=%(trailers:key=Intake-Tag,valueonly)",
            "--grep=^Intake-Upstream: upstream$",
            "main",
        ],
    );
    assert_eq!(merged, "v0.10.0");
    // No upstream tag entered the repository.
    assert_eq!(env.git(&f.down, &["tag", "--list"]), "own-tag");
    let report = env.tool_json(&f.down, &["report", "--json"], 0);
    assert_eq!(report["upstreams"][0]["tag"], "v0.10.0");
    f.tool(&["report"])
        .success()
        .stdout(contains("target tag: v0.10.0"));

    // The release is taken: the commits after it do not count as behind.
    let status = env.tool_json(&f.down, &["status", "--json"], 0);
    let up = &status["upstreams"][0];
    assert_eq!(up["state"], "current");
    assert_eq!(up["tags"], "v*");
    assert_eq!(up["release"], c2.as_str());
    assert_eq!(up["release_tag"], "v0.10.0");
    assert_eq!(up["tip"], dev.as_str());
    assert_eq!(up["commits_behind"], 0);
    assert_eq!(up["last_synced"], c2.as_str());
    assert_eq!(up["last_synced_tag"], "v0.10.0");
    f.tool(&["status", "--check"]).success();
    f.tool(&["start"])
        .success()
        .stdout(contains(format!(
            "upstream: current at {} (v0.10.0)",
            &c2[..12]
        )))
        .stdout(contains("nothing to update"));
    env.assert_no_update(&f.down);
    assert_eq!(env.git(&f.down, &["tag", "--list"]), "own-tag");
}

#[test]
fn status_counts_behind_against_the_newest_release_and_shows_the_branch_tip() {
    let f = release_fork();
    let env = &f.env;
    let c1 = push(&f, "c1");
    tag(&f, "v1.0.0", &c1);
    cycle(&f, &["start"]);
    let c2 = push(&f, "c2");
    let c3 = push(&f, "c3");
    let dev = push(&f, "dev");
    tag(&f, "v1.1.0", &c3);

    let status = env.tool_json(&f.down, &["status", "--json"], 0);
    let up = &status["upstreams"][0];
    assert_eq!(up["state"], "behind");
    assert_eq!(up["commits_behind"], 2);
    assert_eq!(up["release"], c3.as_str());
    assert_eq!(up["release_tag"], "v1.1.0");
    assert_eq!(up["tip"], dev.as_str());
    assert_eq!(up["last_synced_tag"], "v1.0.0");
    let text = stdout(&f.tool(&["status"]).success());
    assert!(text.contains(&format!("{} (v1.0.0)", &c1[..12])), "{text}");
    assert!(text.contains(&format!("{} (v1.1.0)", &c3[..12])), "{text}");
    assert!(
        text.contains(&format!(
            "upstream: takes release tags 'v*'; the branch tip {} is 1 commit past v1.1.0",
            &dev[..12]
        )),
        "{text}"
    );
    f.tool(&["status", "--check"])
        .code(1)
        .stdout(contains("upstream: 2 commits behind"));

    let log = env.tool_json(&f.down, &["log", "--json"], 0);
    let commits: Vec<&str> = log["upstreams"][0]["commits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["commit"].as_str().unwrap())
        .collect();
    assert_eq!(commits, [c3.as_str(), c2.as_str()]);
    assert_eq!(log["upstreams"][0]["release_tag"], "v1.1.0");
    f.tool(&["log"])
        .success()
        .stdout(contains("2 commits not in main up to v1.1.0"));
}

#[test]
fn without_a_release_tag_start_refuses_and_status_says_so() {
    let f = release_fork();
    let env = &f.env;
    let c1 = push(&f, "c1");
    tag(&f, "v1.0.0-rc.1", &c1);
    tag(&f, "other-1.0.0", &c1);
    let main = env.rev(&f.down, "main");

    f.tool(&["start"]).code(2).stderr(contains(format!(
        "upstream 'upstream': no tag of {} matching 'v*' is a release version on upstream/main as fetched; \
         name the target with `git intake start upstream=<rev>`",
        f.up.url()
    )));
    env.assert_no_update(&f.down);
    assert_eq!(env.rev(&f.down, "main"), main);

    let status = env.tool_json(&f.down, &["status", "--json"], 0);
    let up = &status["upstreams"][0];
    assert_eq!(up["state"], "no_release");
    assert!(up["release"].is_null());
    assert_eq!(up["tip"], c1.as_str());
    f.tool(&["status", "--check"])
        .code(1)
        .stdout(contains("no release"));
}

#[test]
fn a_named_revision_overrides_the_release_tags() {
    let f = release_fork();
    let env = &f.env;
    let c1 = push(&f, "c1");
    tag(&f, "v1.0.0", &c1);
    let dev = push(&f, "dev");

    cycle(&f, &["start", &format!("upstream={dev}")]);
    assert_eq!(env.rev(&f.down, "main^2"), dev);
    let msg = tip_message(&f);
    assert!(!msg.contains("Intake-Tag"), "{msg}");

    // The newest release is older than what the branch holds: nothing to take.
    f.tool(&["start"]).success().stdout(contains(format!(
        "upstream: current at {} (v1.0.0)",
        &c1[..12]
    )));
    env.assert_no_update(&f.down);
}

#[test]
fn a_named_tag_is_recorded_without_the_tags_setting() {
    let f = Fork::new();
    let env = &f.env;
    let c1 = push(&f, "c1");
    let c2 = push(&f, "c2");
    let c3 = push(&f, "c3");
    annotated(&f, "v2.0.0", &c1);
    tag(&f, "v2.1.0", &c2);
    tag(&f, "stable", &c2);

    // A tag named on the command line is the one recorded.
    cycle(&f, &["start", "upstream=stable"]);
    let msg = tip_message(&f);
    assert!(msg.contains("Intake-Tag: stable\n"), "{msg}");

    // A commit that only tags without a version name gets no trailer.
    tag(&f, "archive/c3", &c3);
    let c4 = push(&f, "c4");
    cycle(&f, &["start", &format!("upstream={c3}")]);
    let msg = tip_message(&f);
    assert!(!msg.contains("Intake-Tag"), "{msg}");
    let status = env.tool_json(&f.down, &["status", "--json"], 0);
    assert!(status["upstreams"][0]["last_synced_tag"].is_null());
    assert!(status["upstreams"][0]["release"].is_null());

    // A target taken at the fetched tip that an upstream tag names gets it.
    tag(&f, "v3.0.0", &c4);
    cycle(&f, &["start"]);
    let msg = tip_message(&f);
    assert!(msg.contains("Intake-Tag: v3.0.0\n"), "{msg}");
    let status = env.tool_json(&f.down, &["status", "--json"], 0);
    assert_eq!(status["upstreams"][0]["last_synced_tag"], "v3.0.0");
}

#[test]
fn a_commit_with_several_tags_records_the_newest_release() {
    let f = Fork::new();
    let c1 = push(&f, "c1");
    tag(&f, "latest", &c1);
    tag(&f, "v1.9.0", &c1);
    tag(&f, "v1.10.0", &c1);
    cycle(&f, &["start", &format!("upstream={}", &c1[..10])]);
    let msg = tip_message(&f);
    assert!(msg.contains("Intake-Tag: v1.10.0\n"), "{msg}");
    assert_eq!(msg.matches("Intake-Tag").count(), 1, "{msg}");
}

#[test]
fn the_release_tags_of_a_port_upstream_are_refused() {
    let f = Fork::port();
    f.reconfigure("mode = \"port\"\n", "mode = \"port\"\ntags = \"v*\"\n");
    f.tool(&["status"]).code(2).stderr(contains(
        "upstream 'upstream': `tags` selects the target of a follow upstream; an upstream in `port` mode cannot have it",
    ));
}

#[test]
fn releases_outside_the_branch_cost_no_git_command_each() {
    let f = release_fork();
    let env = &f.env;
    let c1 = push(&f, "c1");
    tag(&f, "v0.1.0", &c1);
    // Forty newer releases on a branch the upstream's main branch lacks,
    // so their commits are never fetched.
    env.git(&f.up.work, &["switch", "-q", "-c", "maint", &c1]);
    for n in 1..=40 {
        let c = env.commit(
            &f.up.work,
            &[("m.txt", &format!("{n}\n"))],
            "upstream: maint",
        );
        env.git(&f.up.work, &["tag", &format!("v1.{n}.0"), &c]);
    }
    env.git(&f.up.work, &["push", "-q", "origin", "maint", "--tags"]);
    env.git(&f.up.work, &["switch", "-q", "main"]);

    let trace = env.path("git-trace");
    let trace_path = trace.to_string_lossy().into_owned();
    let assert = env
        .tool_env(
            &f.down,
            &["status", "--json"],
            &[("GIT_TRACE", &trace_path)],
        )
        .success();
    let status: serde_json::Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(status["upstreams"][0]["release_tag"], "v0.1.0");
    let log = std::fs::read_to_string(&trace).unwrap();
    let commands = log
        .lines()
        .filter(|l| l.contains("trace: built-in: git "))
        .count();
    assert!(commands < 40, "{commands} git commands:\n{log}");
    let ancestry = log.lines().filter(|l| l.contains("--is-ancestor")).count();
    assert!(ancestry <= 2, "{ancestry} ancestry checks:\n{log}");
}
