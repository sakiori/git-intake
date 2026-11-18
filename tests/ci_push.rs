//! Pushing from a GitLab CI job: the README's recipe against the git
//! configuration the runner writes, and what the tool does with a clone that
//! would push with the job token.

mod common;

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Output};

use common::{Fork, stderr};

const SERVER: &str = "https://gitlab.example.com:8443";
const JOB_TOKEN: &str = "job-token-of-the-pipeline";
const OWN_TOKEN: &str = "access-token-of-the-job";
/// What the runner's helper does, as its source writes it.
const RUNNER_HELPER: &str =
    r#"!f(){ if [ "$1" = "get" ] ; then echo "password=${CI_JOB_TOKEN}" ; fi ; } ; f"#;

/// The ways the runner configures a job's git, by its flag
/// `FF_GIT_URLS_WITHOUT_TOKENS`.
#[derive(Clone, Copy)]
enum Runner {
    /// A rewrite of the server's URLs into ones with the job token.
    Rewrite,
    /// The job token's helper, for the URLs as they are.
    Helper,
}

const README: &str = include_str!("../README.md");

/// The script of `.push-with-token` in the README, as the job's shell runs it.
fn recipe() -> String {
    let block = README
        .split_once(".push-with-token:\n  script:\n")
        .expect("the README shows .push-with-token")
        .1
        .split_once("\n\n")
        .expect("the job ends with a blank line")
        .0;
    let mut script = String::new();
    for line in block.lines() {
        let body = line.strip_prefix("    ").unwrap_or(line);
        let body = body.strip_prefix("- ").unwrap_or(body);
        if body.trim_start().starts_with('#') || body.trim() == "|" {
            continue;
        }
        script.push_str(body);
        script.push('\n');
    }
    script
}

/// A clone with the configuration the runner gives a job: a file with the
/// runner's settings, included from the clone's configuration.
fn job_clone(runner: Runner) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let job = dir.path().join("job");
    std::fs::create_dir(&job).unwrap();
    git(&job, &["init", "-q"]);
    // Not the URL the recipe sets.
    let origin = format!("{SERVER}/group/as-cloned.git");
    git(&job, &["remote", "add", "origin", &origin]);
    let external = dir.path().join("runner.conf");
    let external = external.to_str().unwrap();
    match runner {
        Runner::Rewrite => git(
            &job,
            &[
                "config",
                "--file",
                external,
                &format!(
                    "url.https://gitlab-ci-token:{JOB_TOKEN}@gitlab.example.com:8443.insteadOf"
                ),
                SERVER,
            ],
        ),
        Runner::Helper => {
            for (key, value) in [("username", "gitlab-ci-token"), ("helper", RUNNER_HELPER)] {
                git(
                    &job,
                    &[
                        "config",
                        "--file",
                        external,
                        &format!("credential.{SERVER}.{key}"),
                        value,
                    ],
                );
            }
            String::new()
        }
    };
    git(&job, &["config", "include.path", external]);
    dir
}

/// A command as a job runs it, with nothing of the environment the tests run in.
fn job_command(dir: &Path, program: &str) -> Command {
    let mut cmd = Command::new(program);
    cmd.current_dir(dir)
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", dir.join("global"))
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("HOME", dir)
        .env("CI_SERVER_URL", SERVER)
        .env("CI_PROJECT_URL", format!("{SERVER}/group/app"))
        .env("CI_JOB_TOKEN", JOB_TOKEN)
        .env("GITLAB_TOKEN", OWN_TOKEN);
    cmd
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = job_command(dir, "git").args(args).output().unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// The recipe, followed by `then`, in one shell, as a job's script runs.
fn in_job(mut sh: Command, then: &str) -> Output {
    sh.arg("-ec")
        .arg(format!("{}\n{then}", recipe()))
        .output()
        .unwrap()
}

fn fill(url: &str) -> String {
    format!("printf 'url={url}\\n\\n' | git credential fill")
}

fn text(out: &Output) -> String {
    format!(
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

#[test]
fn the_recipe_pushes_with_the_access_token_whichever_way_the_runner_is_set() {
    for runner in [Runner::Rewrite, Runner::Helper] {
        let dir = job_clone(runner);
        let job = dir.path().join("job");
        let project = format!("{SERVER}/group/app.git");
        // What the runner leaves: the job token, one way or the other.
        let left = match runner {
            Runner::Rewrite => git(&job, &["remote", "get-url", "--push", "origin"]),
            Runner::Helper => {
                let sh = fill(&project);
                text(&job_command(&job, "sh").args(["-c", &sh]).output().unwrap())
            }
        };
        assert!(left.contains(JOB_TOKEN), "{left}");
        let out = in_job(
            job_command(&job, "sh"),
            &format!(
                "git remote get-url --push origin\n{}\ngit config user.name\ngit config user.email",
                fill(&project)
            ),
        );
        let all = text(&out);
        assert!(out.status.success(), "{all}");
        assert!(all.starts_with(&format!("{project}\n")), "{all}");
        assert!(all.contains("\nSakiori\noss@sakiori.app\n"), "{all}");
        assert!(all.contains("username=oauth2"), "{all}");
        assert!(all.contains(&format!("password={OWN_TOKEN}")), "{all}");
        assert!(!all.contains(JOB_TOKEN), "{all}");
    }
}

#[test]
fn the_recipe_drops_a_rewrite_of_the_global_configuration() {
    let dir = job_clone(Runner::Helper);
    let job = dir.path().join("job");
    git(
        &job,
        &[
            "config",
            "--global",
            &format!("url.https://gitlab-ci-token:{JOB_TOKEN}@gitlab.example.com:8443.insteadOf"),
            SERVER,
        ],
    );
    let out = in_job(job_command(&job, "sh"), "git remote get-url --push origin");
    let all = text(&out);
    assert!(out.status.success(), "{all}");
    assert!(
        all.starts_with(&format!("{SERVER}/group/app.git\n")),
        "{all}"
    );
}

#[test]
fn the_recipe_clears_a_helper_of_the_system_for_the_server_alone() {
    let dir = job_clone(Runner::Rewrite);
    let job = dir.path().join("job");
    let system = dir.path().join("system");
    git(
        &job,
        &[
            "config",
            "--file",
            system.to_str().unwrap(),
            "credential.helper",
            "!f() { echo username=image; echo password=of-the-image; }; f",
        ],
    );
    let mut sh = job_command(&job, "sh");
    sh.env_remove("GIT_CONFIG_NOSYSTEM")
        .env("GIT_CONFIG_SYSTEM", &system);
    let out = in_job(
        sh,
        &format!(
            "{}\n{}",
            fill(&format!("{SERVER}/group/app.git")),
            fill("https://github.com/gone/upstream.git")
        ),
    );
    let all = text(&out);
    assert!(out.status.success(), "{all}");
    let (server, other) = all.split_once("username=image").expect(&all);
    assert!(server.contains(&format!("password={OWN_TOKEN}")), "{all}");
    assert!(other.contains("password=of-the-image"), "{all}");
    assert!(!other.contains(OWN_TOKEN), "{all}");
}

#[test]
fn the_recipe_gives_no_other_host_the_token() {
    let dir = job_clone(Runner::Rewrite);
    let job = dir.path().join("job");
    // An unbound helper a job left in a checkout the runner reuses.
    git(
        &job,
        &[
            "config",
            "credential.helper",
            "!f() { echo username=oauth2; echo \"password=${GITLAB_TOKEN}\"; }; f",
        ],
    );
    for elsewhere in [
        "https://github.com/gone/upstream.git",
        "https://gitlab.example.com/group/app.git",
        "http://gitlab.example.com:8443/group/app.git",
    ] {
        let out = in_job(job_command(&job, "sh"), &fill(elsewhere));
        let all = text(&out);
        assert!(!all.contains(OWN_TOKEN), "{elsewhere}: {all}");
        assert!(!out.status.success(), "{elsewhere}: {all}");
    }
}

#[test]
fn the_guard_of_the_recipe_fails_on_a_rewrite_it_did_not_drop() {
    let dir = job_clone(Runner::Rewrite);
    let job = dir.path().join("job");
    git(
        &job,
        &[
            "config",
            "url.https://oauth2:left-over@gitlab.example.com:8443.insteadOf",
            SERVER,
        ],
    );
    let out = in_job(job_command(&job, "sh"), "echo reached");
    assert!(!out.status.success());
    let all = text(&out);
    assert!(all.contains("git rewrites the push URL"), "{all}");
    assert!(!all.contains("reached"), "{all}");
    assert!(!all.contains("left-over"), "{all}");
}

#[test]
fn the_guard_of_the_recipe_fails_when_git_cannot_read_the_push_url() {
    let dir = job_clone(Runner::Rewrite);
    let job = dir.path().join("job");
    let recipe = recipe();
    let guard = &recipe[recipe.find("push_url=").expect("the guard reads the URL")..];
    // Git refuses the checkout: no URL is no proof of a URL without credentials.
    let out = job_command(&job, "sh")
        .env("GIT_TEST_ASSUME_DIFFERENT_OWNER", "1")
        .arg("-ec")
        .arg(format!("{guard}\necho reached"))
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(!text(&out).contains("reached"));
}

#[test]
fn the_recipe_stops_with_git_s_own_message_when_git_refuses_the_checkout() {
    let dir = job_clone(Runner::Rewrite);
    let job = dir.path().join("job");
    let mut sh = job_command(&job, "sh");
    sh.env("GIT_TEST_ASSUME_DIFFERENT_OWNER", "1");
    let out = in_job(sh, "echo reached");
    let all = text(&out);
    assert!(!out.status.success(), "{all}");
    assert!(all.contains("dubious ownership"), "{all}");
    // Nothing the recipe tolerates speaks first.
    assert!(!all.contains("--local can only be used"), "{all}");
    assert!(!all.contains("not in a git directory"), "{all}");
    assert!(!all.contains("reached"), "{all}");
}

#[test]
fn only_the_sync_pipeline_of_the_readme_pushes_through_the_recipe() {
    // `release tag` creates its tag through the API and configures no git
    // credential.
    assert_eq!(
        README
            .matches("- !reference [.push-with-token, script]")
            .count(),
        1
    );
    let release = README
        .split_once("release-tag:\n")
        .expect("the README shows the release-tag job")
        .1
        .split_once("\n\n")
        .expect("the job ends with a blank line")
        .0;
    assert!(!release.contains(".push-with-token"), "{release}");
}

#[test]
fn the_variables_of_the_pipelines_name_the_checkout_for_every_job() {
    let block = "  GIT_CONFIG_COUNT: \"1\"\n  GIT_CONFIG_KEY_0: safe.directory\n  GIT_CONFIG_VALUE_0: $CI_PROJECT_DIR\n";
    assert_eq!(README.matches(block).count(), 2, "both pipelines set them");
    let dir = job_clone(Runner::Helper);
    let job = dir.path().join("job");
    let checkout = std::fs::canonicalize(&job).unwrap();
    // What git tests its ownership check with.
    let owned_by_another = |cmd: &mut Command| {
        cmd.env("GIT_TEST_ASSUME_DIFFERENT_OWNER", "1");
    };
    // Git accepts the checkout for a job that does not push, and for one that
    // does. `a_checkout_of_another_owner` in the layout tests runs the tool.
    let mut plain = job_command(&job, "git");
    owned_by_another(&mut plain);
    plain
        .env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", "safe.directory")
        .env("GIT_CONFIG_VALUE_0", &checkout)
        .args(["rev-parse", "--show-toplevel"]);
    let out = plain.output().unwrap();
    assert!(out.status.success(), "{}", text(&out));
    let mut sh = job_command(&job, "sh");
    owned_by_another(&mut sh);
    sh.env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", "safe.directory")
        .env("GIT_CONFIG_VALUE_0", &checkout);
    let out = in_job(sh, "git remote get-url --push origin\ngit config user.name");
    let all = text(&out);
    assert!(out.status.success(), "{all}");
    assert!(
        all.starts_with(&format!("{SERVER}/group/app.git\n")),
        "{all}"
    );
}

/// A fork in merge-request mode, on a GitLab, in one of its CI jobs, with
/// every upstream current.
fn fork_in_a_job() -> Fork {
    let f = Fork::merge_request();
    f.env.set_var("CI_JOB_TOKEN", JOB_TOKEN);
    f
}

fn add_push_url(f: &Fork, url: &str) {
    f.env.git(
        &f.down,
        &["remote", "set-url", "--add", "--push", "origin", url],
    );
}

fn refused(f: &Fork, args: &[&str], how: &str) {
    f.forge().clear_requests();
    let a = f.env.tool(&f.down, args).code(2);
    let err = stderr(&a);
    assert!(
        err.contains("would push to 'origin' with its job token"),
        "{err}"
    );
    assert!(err.contains(how), "{err}");
    assert!(err.contains("Running in CI"), "{err}");
    assert!(!err.contains(JOB_TOKEN), "{err}");
    assert!(f.forge().changes().is_empty());
}

fn job_token_url() -> String {
    format!("https://gitlab-ci-token:{JOB_TOKEN}@gitlab.example.com/team/app.git")
}

#[test]
fn sync_refuses_a_push_url_with_the_job_token_though_nothing_moved() {
    let f = fork_in_a_job();
    add_push_url(&f, &job_token_url());
    refused(&f, &["sync"], "the push URL carries it");
}

#[test]
fn sync_refuses_the_job_token_on_any_push_url() {
    let f = fork_in_a_job();
    add_push_url(&f, "ssh://git@gitlab.example.com/team/app.git");
    add_push_url(&f, &job_token_url());
    refused(&f, &["sync"], "the push URL carries it");
}

#[test]
fn sync_reads_a_percent_encoded_job_token_from_the_push_url() {
    let f = fork_in_a_job();
    f.env.set_var("CI_JOB_TOKEN", "job+token/1");
    add_push_url(
        &f,
        "https://gitlab-ci-token:job%2Btoken%2F1@gitlab.example.com/team/app.git",
    );
    let a = f.env.tool(&f.down, &["sync"]).code(2);
    let err = stderr(&a);
    assert!(err.contains("the push URL carries it"), "{err}");
    assert!(!err.contains("job+token/1"), "{err}");
}

#[test]
fn sync_refuses_a_helper_that_answers_with_the_job_token() {
    let f = fork_in_a_job();
    add_push_url(&f, "https://gitlab.example.com/team/app.git");
    for (key, value) in [("username", "gitlab-ci-token"), ("helper", RUNNER_HELPER)] {
        f.env.git(
            &f.down,
            &[
                "config",
                &format!("credential.https://gitlab.example.com.{key}"),
                value,
            ],
        );
    }
    refused(&f, &["sync"], "git's credential helpers answer with it");
}

#[test]
fn sync_goes_on_with_a_credential_of_the_job_s_own() {
    let f = fork_in_a_job();
    // A password that is not the job token, under the user name the job
    // token has: an access token works there as well.
    add_push_url(
        &f,
        &format!("https://gitlab-ci-token:{OWN_TOKEN}@gitlab.example.com/team/app.git"),
    );
    f.env.tool(&f.down, &["sync"]).success();
    f.env
        .git(&f.down, &["config", "--unset-all", "remote.origin.pushurl"]);
    add_push_url(&f, "https://gitlab.example.com/team/app.git");
    f.env.git(
        &f.down,
        &[
            "config",
            "credential.https://gitlab.example.com.helper",
            &format!("!f() {{ echo username=oauth2; echo password={OWN_TOKEN}; }}; f"),
        ],
    );
    f.env.tool(&f.down, &["sync"]).success();
}

#[test]
fn only_a_password_counts_where_git_sends_one() {
    let f = fork_in_a_job();
    // An SSH URL's password is not what the push authenticates with.
    add_push_url(
        &f,
        &format!("ssh://gitlab-ci-token:{JOB_TOKEN}@gitlab.example.com/team/app.git"),
    );
    f.env.tool(&f.down, &["sync"]).success();
}

#[test]
fn asking_the_helpers_prompts_nobody() {
    let f = fork_in_a_job();
    add_push_url(&f, "https://gitlab.example.com/team/app.git");
    let marker = f.env.path("asked");
    let askpass = f.env.path("askpass.sh");
    std::fs::write(
        &askpass,
        format!("#!/bin/sh\ntouch '{}'\necho answer\n", marker.display()),
    )
    .unwrap();
    std::fs::set_permissions(&askpass, std::fs::Permissions::from_mode(0o755)).unwrap();
    f.env.set_var("GIT_ASKPASS", askpass.to_str().unwrap());
    f.env.set_var("SSH_ASKPASS", askpass.to_str().unwrap());
    f.env.tool(&f.down, &["sync"]).success();
    assert!(!marker.exists(), "git asked the askpass program");
}

#[test]
fn outside_a_job_nothing_is_checked() {
    let f = Fork::merge_request();
    add_push_url(&f, &job_token_url());
    f.env.tool(&f.down, &["sync"]).success();
}

#[test]
fn sync_by_push_warns_of_the_job_token_and_goes_on() {
    let f = Fork::new();
    f.env.set_var("CI_JOB_TOKEN", JOB_TOKEN);
    add_push_url(&f, &job_token_url());
    let a = f.env.tool(&f.down, &["sync"]).success();
    let err = stderr(&a);
    assert!(
        err.contains("warning: this GitLab CI job would push"),
        "{err}"
    );
    assert!(!err.contains(JOB_TOKEN), "{err}");
}
