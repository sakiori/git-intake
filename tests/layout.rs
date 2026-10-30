//! Repository layouts and the directory a command runs in: every command
//! behaves the same from the main checkout, a linked worktree, the update
//! worktree, a submodule, or a repository whose git directory lies outside
//! its checkout.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::{Env, FORK_FILE, Fork, Mono, Upstream, stderr};
use predicates::str::contains;

/// Note printed when a command removes the update worktree it was run from.
const LEFT_REMOVED_DIRECTORY: &str =
    "note: the current directory was in the update worktree, which is now removed";

/// The update worktree of a repository whose common git directory is `git_dir`.
fn update_worktree_in(git_dir: &Path) -> PathBuf {
    git_dir.join("git-intake").join("worktree")
}

/// A configuration with one upstream at the root, reached by `url`, without
/// a publish remote.
fn root_config(url: &str) -> String {
    format!(
        "integration = \"main\"\nchecks = [\"test -f {FORK_FILE}\"]\n\n\
         [[upstream]]\nname = \"upstream\"\nremote = \"upstream\"\nbranch = \"main\"\nurl = {url:?}\n"
    )
}

/// A downstream repository `name`, cloned from the upstream at `bare`, that
/// carries [`root_config`] with `url`. Returns its checkout.
fn root_downstream(env: &Env, up: &Upstream, name: &str, url: &str) -> PathBuf {
    let down = env.path(name);
    env.git(
        &env.path(""),
        &["clone", "-q", &up.url(), &down.to_string_lossy()],
    );
    env.git(&down, &["remote", "remove", "origin"]);
    env.commit(
        &down,
        &[(".git-intake.toml", &root_config(url))],
        "down: add git-intake configuration",
    );
    down
}

/// Adds a linked worktree at `path` on a new branch from `HEAD`.
fn add_worktree(env: &Env, repo: &Path, branch: &str, path: &Path) {
    env.git(
        repo,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            branch,
            &path.to_string_lossy(),
        ],
    );
}

#[test]
fn finish_run_from_the_update_worktree_completes() {
    let m = Mono::new();
    let env = &m.env;
    m.tool(&["start"]).success();
    m.tool(&["verify"]).success();
    let wt = env.update_worktree(&m.down);
    let verified = env.rev(&wt, "HEAD");

    env.tool(&wt, &["finish"])
        .success()
        .stdout(contains(
            "no publish remote is configured; nothing was pushed",
        ))
        .stdout(contains("update finished"))
        .stderr(contains(LEFT_REMOVED_DIRECTORY));
    assert_eq!(env.rev(&m.down, "main"), verified);
    env.assert_no_update(&m.down);
    m.tool(&["doctor"]).success();
}

#[test]
fn push_and_no_publish_finish_run_from_the_update_worktree_complete() {
    let f = Fork::new();
    let env = &f.env;
    env.push_upstream(&f.up, &[("n.txt", "n\n")], "upstream: n");
    f.tool(&["start"]).success();
    f.tool(&["verify"]).success();
    let wt = env.update_worktree(&f.down);
    let verified = env.rev(&wt, "HEAD");
    env.tool(&wt, &["finish"])
        .success()
        .stdout(contains("pushed main, upstream-main to 'origin'"))
        .stderr(contains(LEFT_REMOVED_DIRECTORY));
    assert_eq!(env.rev(&f.publish, "main"), verified);
    env.assert_no_update(&f.down);

    env.push_upstream(&f.up, &[("m.txt", "m\n")], "upstream: m");
    f.tool(&["start"]).success();
    f.tool(&["verify"]).success();
    let verified = env.rev(&wt, "HEAD");
    env.tool(&wt, &["finish", "--no-publish"])
        .success()
        .stdout(contains("update finished"))
        .stderr(contains(LEFT_REMOVED_DIRECTORY));
    assert_eq!(env.rev(&f.down, "main"), verified);
    env.assert_no_update(&f.down);
}

#[test]
fn merge_request_opened_from_the_update_worktree_waits() {
    let f = Fork::merge_request();
    let env = &f.env;
    env.push_upstream(&f.up, &[("n.txt", "n\n")], "upstream: n");
    f.tool(&["start"]).success();
    f.tool(&["verify"]).success();
    let wt = env.update_worktree(&f.down);
    let verified = env.rev(&wt, "HEAD");
    env.tool(&wt, &["finish"])
        .success()
        .stdout(contains("the update waits for the merge request"))
        .stderr(contains(LEFT_REMOVED_DIRECTORY));
    assert!(!wt.exists(), "the update worktree remains");
    let branch = format!("git-intake/update-{}", &verified[..12]);
    assert_eq!(env.rev(&f.publish, &branch), verified);
    let status = env.tool_json(&f.down, &["status", "--json"], 0);
    assert_eq!(status["update"]["step"], "waiting_for_merge_request");
}

#[test]
fn abort_run_from_the_update_worktree_removes_the_update() {
    let m = Mono::new();
    let env = &m.env;
    m.tool(&["start"]).success();
    let wt = env.update_worktree(&m.down);
    env.tool(&wt.join("apps").join("a"), &["abort"])
        .success()
        .stdout(contains("update aborted"))
        .stderr(contains(LEFT_REMOVED_DIRECTORY));
    env.assert_no_update(&m.down);
    m.tool(&["doctor"]).success();
}

#[test]
fn integration_branch_in_a_linked_worktree_follows_a_finish_from_the_update_worktree() {
    let f = Fork::new();
    let env = &f.env;
    env.git(&f.down, &["checkout", "-q", "-b", "feature"]);
    let linked = env.path("integration");
    env.git(
        &f.down,
        &["worktree", "add", "-q", &linked.to_string_lossy(), "main"],
    );
    env.push_upstream(&f.up, &[("n.txt", "n\n")], "upstream: n");
    f.tool(&["start"]).success();
    f.tool(&["verify"]).success();
    let wt = env.update_worktree(&f.down);
    let verified = env.rev(&wt, "HEAD");

    env.tool(&wt, &["finish"]).success();
    assert_eq!(env.rev(&linked, "HEAD"), verified);
    assert!(linked.join("n.txt").exists(), "the checkout did not follow");
    assert!(env.git(&linked, &["status", "--porcelain"]).is_empty());
    assert_eq!(env.git(&f.down, &["branch", "--show-current"]), "feature");
}

#[test]
fn bare_downstream_finishes_from_the_update_worktree() {
    let m = Mono::new();
    let env = &m.env;
    let bare = env.path("mono-bare.git");
    env.git(
        &env.path(""),
        &[
            "clone",
            "-q",
            "--bare",
            &m.down.to_string_lossy(),
            &bare.to_string_lossy(),
        ],
    );
    env.tool(&bare, &["start"]).success();
    let wt = update_worktree_in(&bare);
    env.tool(&wt, &["verify"]).success();
    let verified = env.rev(&wt, "HEAD");
    env.tool(&wt, &["finish"])
        .success()
        .stdout(contains("update finished"))
        .stderr(contains(LEFT_REMOVED_DIRECTORY));
    assert_eq!(env.rev(&bare, "main"), verified);
    assert!(!wt.exists(), "the update worktree remains");
    assert!(!env.has_rev(&bare, "refs/heads/git-intake/update"));
}

#[test]
fn relative_remote_urls_mean_the_same_from_every_worktree() {
    let m = Mono::new();
    let env = &m.env;
    let cfg = common::mono_config(&m.a, &m.b, "")
        .replace(&format!("{:?}", m.a.url()), "\"../app-a.git\"")
        .replace(&format!("{:?}", m.b.url()), "\"../app-b.git\"");
    env.commit(
        &m.down,
        &[(".git-intake.toml", &cfg)],
        "mono: relative urls",
    );
    // A linked worktree one level deeper than the main checkout.
    let linked = env.path("nested").join("side");
    add_worktree(env, &m.down, "side", &linked);
    env.tool(&linked, &["start"])
        .success()
        .stdout(contains("app-a: queued"))
        .stdout(contains("app-b: queued"));
    assert_eq!(
        env.git(&m.down, &["config", "remote.app-a.url"]),
        "../app-a.git"
    );

    let wt = env.update_worktree(&m.down);
    env.push_upstream(&m.a, &[("n.txt", "n\n")], "app-a: n");
    env.tool(&wt, &["fetch"])
        .success()
        .stdout(contains("app-a: fetched"))
        .stdout(contains("app-b: fetched"));
    assert_eq!(env.rev(&m.down, "app-a/main"), env.rev(&m.a.bare, "main"));
}

#[test]
fn start_resolves_a_revision_in_the_worktree_it_runs_in() {
    let f = Fork::new();
    let env = &f.env;
    let c1 = env.push_upstream(&f.up, &[("c1.txt", "1\n")], "upstream: c1");
    env.push_upstream(&f.up, &[("c2.txt", "2\n")], "upstream: c2");
    f.tool(&["fetch"]).success();
    // In a linked worktree, HEAD^2 is c1; the main checkout's HEAD has no
    // second parent.
    let linked = env.path("side");
    add_worktree(env, &f.down, "side", &linked);
    env.git(&linked, &["merge", "-q", "--no-ff", "--no-edit", &c1]);

    env.tool(&linked, &["start", "upstream=HEAD^2"]).success();
    let status = env.tool_json(&f.down, &["status", "--json"], 0);
    assert_eq!(status["update"]["upstreams"][0]["target"], c1.as_str());
    assert_eq!(status["update"]["upstreams"][0]["rev"], "HEAD^2");
}

/// A downstream that is a submodule of a superproject, with its upstream
/// reached by a URL relative to the submodule's checkout. Returns the
/// environment, the upstream and the submodule's checkout.
fn submodule_downstream() -> (Env, Upstream, PathBuf) {
    let env = Env::new();
    let up = env.upstream("upstream", &[(FORK_FILE, "one\n")]);
    // `super/sub` reaches the upstream at `../../upstream.git`; the
    // submodule's git directory, `super/.git/modules/sub`, does not.
    let src = root_downstream(&env, &up, "down-src", "../../upstream.git");
    let sup = env.work("super");
    env.commit(&sup, &[("README.md", "super\n")], "super: initial");
    env.git(
        &sup,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            "-q",
            &src.to_string_lossy(),
            "sub",
        ],
    );
    let sub = sup.join("sub");
    env.git(&sub, &["checkout", "-q", "main"]);
    (env, up, sub)
}

#[test]
fn submodule_downstream_resolves_relative_urls_from_its_checkout() {
    let (env, up, sub) = submodule_downstream();
    env.push_upstream(&up, &[("n.txt", "n\n")], "upstream: n");
    env.tool(&sub, &["start"])
        .success()
        .stdout(contains("upstream: queued"));
    let git_dir = PathBuf::from(env.git(
        &sub,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    ));
    let wt = update_worktree_in(&git_dir);
    env.push_upstream(&up, &[("m.txt", "m\n")], "upstream: m");
    env.tool(&wt, &["fetch"])
        .success()
        .stdout(contains("upstream: fetched"));
    env.tool(&wt, &["verify"]).success();
    let verified = env.rev(&wt, "HEAD");
    env.tool(&wt, &["finish"])
        .success()
        .stderr(contains(LEFT_REMOVED_DIRECTORY));
    assert_eq!(env.rev(&sub, "main"), verified);
    assert!(sub.join("n.txt").exists(), "the checkout did not follow");
    env.tool(&sub, &["doctor"]).success();
}

/// A downstream whose git directory lies outside its checkout, at `git_dir`
/// in the environment, with its upstream reached by a URL relative to the
/// checkout. Returns the environment, the upstream, the checkout and the git
/// directory.
fn separate_git_dir_downstream(git_dir: &str) -> (Env, Upstream, PathBuf, PathBuf) {
    let env = Env::new();
    let up = env.upstream("upstream", &[(FORK_FILE, "one\n")]);
    let down = env.path("down");
    // `down` reaches the upstream at `../upstream.git`; a git directory one
    // level deeper, or the directory holding it, does not.
    let git_dir = env.path(git_dir);
    fs::create_dir_all(git_dir.parent().unwrap()).unwrap();
    env.git(
        &env.path(""),
        &[
            "clone",
            "-q",
            "--separate-git-dir",
            &git_dir.to_string_lossy(),
            &up.url(),
            &down.to_string_lossy(),
        ],
    );
    env.git(&down, &["remote", "remove", "origin"]);
    env.commit(
        &down,
        &[(".git-intake.toml", &root_config("../upstream.git"))],
        "down: add git-intake configuration",
    );
    let git_dir = fs::canonicalize(git_dir).unwrap();
    (env, up, down, git_dir)
}

/// Starts an update in the checkout `down`, then fetches, verifies and
/// finishes it from the update worktree.
fn update_from_checkout_and_update_worktree(env: &Env, up: &Upstream, down: &Path, git_dir: &Path) {
    env.push_upstream(up, &[("n.txt", "n\n")], "upstream: n");
    env.tool(down, &["start"])
        .success()
        .stdout(contains("upstream: queued"));
    let wt = update_worktree_in(git_dir);
    env.push_upstream(up, &[("m.txt", "m\n")], "upstream: m");
    env.tool(&wt, &["fetch"])
        .success()
        .stdout(contains("upstream: fetched"));
    env.tool(&wt, &["verify"]).success();
    let verified = env.rev(&wt, "HEAD");
    env.tool(&wt, &["finish"])
        .success()
        .stderr(contains(LEFT_REMOVED_DIRECTORY));
    assert_eq!(env.rev(down, "main"), verified);
    assert!(down.join("n.txt").exists(), "the checkout did not follow");
    assert!(env.git(down, &["status", "--porcelain"]).is_empty());
    env.tool(down, &["doctor"]).success();
}

#[test]
fn separate_git_dir_downstream_updates_from_its_checkout_and_update_worktree() {
    let (env, up, down, git_dir) = separate_git_dir_downstream("git-dirs/down.git");
    update_from_checkout_and_update_worktree(&env, &up, &down, &git_dir);
}

#[test]
fn separate_git_dir_named_dot_git_is_not_taken_for_its_parent() {
    let (env, up, down, git_dir) = separate_git_dir_downstream("store/down/.git");
    update_from_checkout_and_update_worktree(&env, &up, &down, &git_dir);
    let beside: Vec<_> = fs::read_dir(git_dir.parent().unwrap())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(
        beside,
        [".git"],
        "files were written beside the git directory"
    );
}

#[test]
fn unreadable_state_stops_finish_from_the_update_worktree() {
    let (env, up, down, git_dir) = separate_git_dir_downstream("git-dirs/down.git");
    env.push_upstream(&up, &[("n.txt", "n\n")], "upstream: n");
    env.tool(&down, &["start"]).success();
    env.tool(&down, &["verify"]).success();
    let main = env.rev(&down, "main");
    fs::write(git_dir.join("git-intake").join("state.json"), "{").unwrap();
    let finish = env.tool(&update_worktree_in(&git_dir), &["finish"]).code(2);
    assert!(
        stderr(&finish).contains("is unreadable"),
        "{}",
        stderr(&finish)
    );
    assert_eq!(env.rev(&down, "main"), main);
}

#[test]
fn separate_git_dir_main_worktree_unknown_from_a_linked_worktree_is_named() {
    let (env, up, down, _) = separate_git_dir_downstream("git-dirs/down.git");
    env.push_upstream(&up, &[("n.txt", "n\n")], "upstream: n");
    env.tool(&down, &["fetch"]).success();
    let linked = env.path("side");
    add_worktree(&env, &down, "side", &linked);
    let reason = "the main worktree cannot be located from here";
    let start = env.tool(&linked, &["start"]).code(2);
    assert!(stderr(&start).contains(reason), "{}", stderr(&start));
    // Nothing is wrong with the repository, so doctor only warns.
    let doctor = env.tool_json(&linked, &["doctor", "--json"], 0);
    let check = doctor["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["check"] == "main_checkout_clean")
        .expect("main checkout check");
    assert_eq!(check["status"], "warn");
    assert!(check["detail"].as_str().unwrap().contains(reason));
}
