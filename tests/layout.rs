//! Repository layouts and the directory a command runs in: every command
//! behaves the same from the main checkout, a linked worktree, the update
//! worktree, a submodule, or a repository whose git directory lies outside
//! its checkout.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::{Env, FORK_FILE, Fork, Mono, Upstream, stderr, stdout};
use predicates::str::contains;

/// Note printed when a command removes the update worktree it was run from.
const LEFT_REMOVED_DIRECTORY: &str =
    "note: the current directory was in the update worktree, which is now removed";

/// A configuration with one upstream at the root, reached by `url`, without
/// a publish remote.
fn root_config(url: &str) -> String {
    format!(
        "integration = \"main\"\nchecks = [{{ name = \"app\", run = \"test -f {FORK_FILE}\" }}]\n\n\
         [[upstream]]\nname = \"upstream\"\nurl = {url:?}\nbranch = \"main\"\n"
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
        .stdout(contains("pushed main to 'origin'"))
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
fn merge_request_opened_from_the_update_worktree_ends_the_update() {
    let f = Fork::merge_request();
    let env = &f.env;
    env.push_upstream(&f.up, &[("n.txt", "n\n")], "upstream: n");
    f.tool(&["start"]).success();
    let wt = env.update_worktree(&f.down);
    let head = env.rev(&wt, "HEAD");
    env.tool(&wt, &["finish"])
        .success()
        .stdout(contains("opened merge request !1"))
        .stderr(contains(LEFT_REMOVED_DIRECTORY));
    assert!(!wt.exists(), "the update worktree remains");
    let branch = common::MERGE_REQUEST_BRANCH;
    assert_eq!(env.rev(&f.publish, branch), head);
    env.assert_no_update(&f.down);
    let status = env.tool_json(&f.down, &["status", "--json"], 0);
    assert_eq!(status["update"], serde_json::Value::Null);
    assert_eq!(status["merge_request"]["number"], 1);
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

/// The update worktree of a bare repository whose git directory is `git_dir`.
fn bare_update_worktree(git_dir: &Path) -> PathBuf {
    git_dir.join("git-intake").join("worktree")
}

/// A bare clone of `m`'s downstream at `path` in the environment.
fn bare_downstream(m: &Mono, path: &str) -> PathBuf {
    let bare = m.env.path(path);
    m.env.git(
        &m.env.path(""),
        &[
            "clone",
            "-q",
            "--bare",
            &m.down.to_string_lossy(),
            &bare.to_string_lossy(),
        ],
    );
    fs::canonicalize(bare).unwrap()
}

#[test]
fn bare_downstream_finishes_from_the_update_worktree() {
    let m = Mono::new();
    let env = &m.env;
    let bare = bare_downstream(&m, "mono-bare.git");
    env.tool(&bare, &["start"]).success();
    let wt = bare_update_worktree(&bare);
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
fn bare_downstream_whose_update_worktree_path_holds_dot_git_is_refused() {
    let m = Mono::new();
    let env = &m.env;
    let bare = bare_downstream(&m, "bare-store/.git");
    let start = env.tool(&bare, &["start"]).code(2);
    let err = stderr(&start);
    assert!(err.contains("/.git"), "{err}");
    assert!(err.contains("ignore"), "{err}");
    assert!(err.contains("clone with a checkout"), "{err}");
    assert!(!bare.join("git-intake").exists(), "the update was started");
    assert!(!env.has_rev(&bare, "refs/heads/git-intake/update"));
    assert_eq!(
        env.git_output(&bare, &["config", "rerere.enabled"]).stdout,
        b""
    );
}

/// The worktree a status reports for the update in progress in `repo`.
fn reported_update_worktree(env: &Env, repo: &Path) -> PathBuf {
    let status = env.tool_json(repo, &["status", "--json"], 0);
    PathBuf::from(
        status["update"]["worktree"]
            .as_str()
            .expect("update worktree"),
    )
}

#[test]
fn update_worktree_lives_in_the_main_worktree_outside_the_git_directory() {
    let f = Fork::new();
    let env = &f.env;
    env.push_upstream(&f.up, &[("n.txt", "n\n")], "upstream: n");
    f.tool(&["start"])
        .success()
        .stdout(contains(".worktrees/git-intake/update"));
    let wt = reported_update_worktree(env, &f.down);
    let main = fs::canonicalize(&f.down).unwrap();
    assert_eq!(
        wt,
        main.join(".worktrees").join("git-intake").join("update")
    );
    assert!(
        !wt.components().any(|c| c.as_os_str() == ".git"),
        "the update worktree is inside a .git directory: {}",
        wt.display()
    );
    assert!(
        wt.join("n.txt").exists(),
        "the merge is not in the worktree"
    );
    let listed = env.git(&f.down, &["worktree", "list", "--porcelain"]);
    assert!(
        listed.contains(&format!("worktree {}", wt.display())),
        "{listed}"
    );
    assert!(
        !env.intake_dir(&f.down).join("worktree").exists(),
        "a worktree was created in the git directory"
    );

    // The main checkout shows nothing of it, and doctor finds it clean.
    assert_eq!(
        env.git(&f.down, &["status", "--porcelain", "--untracked-files=all"]),
        ""
    );
    let doctor = env.tool_json(&f.down, &["doctor", "--json"], 0);
    let clean = doctor["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["check"] == "main_checkout_clean")
        .expect("main checkout check");
    assert_eq!(clean["status"], "pass", "{clean}");

    f.tool(&["verify"]).success();
    f.tool(&["finish"]).success();
    env.assert_no_update(&f.down);
    assert!(
        !f.down.join(".worktrees").exists(),
        "the directories that held the update worktree remain"
    );
}

/// The lines of `path` that equal `line`.
fn count_lines(path: &Path, line: &str) -> usize {
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter(|l| *l == line)
        .count()
}

#[test]
fn start_excludes_the_update_worktrees_once() {
    let f = Fork::new();
    let env = &f.env;
    let exclude = env.info_exclude(&f.down);
    // Without the file, start creates it.
    let _ = fs::remove_file(&exclude);
    env.push_upstream(&f.up, &[("n.txt", "n\n")], "upstream: n");
    f.tool(&["start"]).success();
    assert_eq!(count_lines(&exclude, "/.worktrees/"), 1);
    assert_eq!(
        env.git(&f.down, &["status", "--porcelain", "--untracked-files=all"]),
        ""
    );
    f.tool(&["abort"]).success();
    env.assert_no_update(&f.down);

    // With its own lines and no final newline, start adds the line once.
    fs::write(&exclude, "# own\n*.log").unwrap();
    f.tool(&["start"]).success();
    f.tool(&["abort"]).success();
    f.tool(&["start"]).success();
    assert_eq!(
        fs::read_to_string(&exclude).unwrap(),
        "# own\n*.log\n/.worktrees/\n"
    );
    assert_eq!(
        env.git(&f.down, &["status", "--porcelain", "--untracked-files=all"]),
        ""
    );
}

#[test]
fn start_leaves_the_exclude_file_alone_when_the_update_worktree_is_ignored() {
    let f = Fork::new();
    let env = &f.env;
    env.commit(
        &f.down,
        &[(".gitignore", ".worktrees/\n")],
        "fork: ignore worktrees",
    );
    env.git(&f.down, &["push", "-q", "origin", "main"]);
    let exclude = env.info_exclude(&f.down);
    let before = fs::read_to_string(&exclude).ok();
    env.push_upstream(&f.up, &[("n.txt", "n\n")], "upstream: n");
    f.tool(&["start"]).success();
    assert_eq!(fs::read_to_string(&exclude).ok(), before);
    assert_eq!(
        fs::read_to_string(f.down.join(".gitignore")).unwrap(),
        ".worktrees/\n"
    );
    assert_eq!(
        env.git(&f.down, &["status", "--porcelain", "--untracked-files=all"]),
        ""
    );
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
    assert_eq!(env.git(&m.down, &["remote"]), "", "no remote was added");
    let held = env.rev(&m.down, "refs/git-intake/upstreams/app-a");

    // From the update worktree, status fetches the relative URLs as the
    // main worktree means them, and gives the update its references back.
    let wt = env.update_worktree(&m.down);
    let a2 = env.push_upstream(&m.a, &[("n.txt", "n\n")], "app-a: n");
    let status = env.tool_json(&wt, &["status", "--json"], 0);
    assert_eq!(status["upstreams"][0]["tip"], a2.as_str());
    assert_eq!(
        status["upstreams"][1]["tip"],
        env.rev(&m.b.bare, "main").as_str()
    );
    assert_eq!(env.rev(&m.down, "refs/git-intake/upstreams/app-a"), held);
}

#[test]
fn start_resolves_a_revision_in_the_worktree_it_runs_in() {
    let f = Fork::new();
    let env = &f.env;
    let c1 = env.push_upstream(&f.up, &[("c1.txt", "1\n")], "upstream: c1");
    env.push_upstream(&f.up, &[("c2.txt", "2\n")], "upstream: c2");
    env.git(&f.down, &["fetch", "-q", &f.up.url(), "main"]);
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
    let wt = env.update_worktree(&sub);
    let m = env.push_upstream(&up, &[("m.txt", "m\n")], "upstream: m");
    let status = env.tool_json(&wt, &["status", "--json"], 0);
    assert_eq!(status["upstreams"][0]["tip"], m.as_str());
    env.tool(&wt, &["verify"]).success();
    let verified = env.rev(&wt, "HEAD");
    env.tool(&wt, &["finish"])
        .success()
        .stderr(contains(LEFT_REMOVED_DIRECTORY));
    assert_eq!(env.rev(&sub, "main"), verified);
    assert!(sub.join("n.txt").exists(), "the checkout did not follow");
    env.tool(&sub, &["doctor"]).success();
}

#[test]
fn update_worktree_of_a_submodule_shows_in_neither_status() {
    let (env, up, sub) = submodule_downstream();
    let sup = sub.parent().unwrap();
    env.git(sup, &["commit", "-q", "-m", "super: add sub"]);
    env.push_upstream(&up, &[("n.txt", "n\n")], "upstream: n");
    env.tool(&sub, &["start"]).success();
    assert!(env.update_worktree(&sub).join("n.txt").exists());
    assert_eq!(count_lines(&env.info_exclude(&sub), "/.worktrees/"), 1);
    for dir in [&sub, sup] {
        assert_eq!(
            env.git(dir, &["status", "--porcelain", "--untracked-files=all"]),
            "",
            "{}",
            dir.display()
        );
    }
}

#[test]
fn start_from_a_linked_worktree_excludes_the_update_worktree_in_the_repository() {
    let f = Fork::new();
    let env = &f.env;
    let linked = env.path("side");
    add_worktree(env, &f.down, "side", &linked);
    env.push_upstream(&f.up, &[("n.txt", "n\n")], "upstream: n");
    env.tool(&linked, &["start"]).success();
    assert!(env.update_worktree(&f.down).join("n.txt").exists());
    assert_eq!(env.info_exclude(&linked), env.info_exclude(&f.down));
    assert_eq!(count_lines(&env.info_exclude(&f.down), "/.worktrees/"), 1);
    for dir in [&f.down, &linked] {
        assert_eq!(
            env.git(dir, &["status", "--porcelain", "--untracked-files=all"]),
            "",
            "{}",
            dir.display()
        );
    }
}

#[test]
fn own_worktrees_beside_the_update_worktree_are_kept() {
    let f = Fork::new();
    let env = &f.env;
    let other = f.down.join(".worktrees").join("other");
    add_worktree(env, &f.down, "other", &other);
    let notes = f.down.join(".worktrees").join("git-intake").join("notes");
    fs::create_dir_all(&notes).unwrap();
    fs::write(notes.join("n.md"), "mine\n").unwrap();
    env.push_upstream(&f.up, &[("n.txt", "n\n")], "upstream: n");

    f.tool(&["start"]).success();
    f.tool(&["abort"]).success();
    assert!(!env.update_worktree(&f.down).exists());
    assert!(notes.join("n.md").exists(), "own files were removed");
    assert_eq!(env.git(&other, &["branch", "--show-current"]), "other");

    f.tool(&["start"]).success();
    f.tool(&["verify"]).success();
    f.tool(&["finish"]).success();
    assert!(!env.update_worktree(&f.down).exists());
    assert!(notes.join("n.md").exists(), "own files were removed");
    assert_eq!(env.git(&other, &["branch", "--show-current"]), "other");
    assert!(!env.has_rev(&f.down, "refs/heads/git-intake/update"));
}

#[test]
fn worktrees_directory_that_is_a_symbolic_link_is_refused_before_anything_changes() {
    let f = Fork::new();
    let env = &f.env;
    let elsewhere = env.path("elsewhere");
    fs::create_dir_all(&elsewhere).unwrap();
    std::os::unix::fs::symlink(&elsewhere, f.down.join(".worktrees")).unwrap();
    env.push_upstream(&f.up, &[("n.txt", "n\n")], "upstream: n");
    let start = f.tool(&["start"]).code(2);
    let err = stderr(&start);
    assert!(err.contains(".worktrees"), "{err}");
    assert!(err.contains("symbolic link"), "{err}");
    assert!(!env.has_rev(&f.down, "refs/heads/git-intake/update"));
    assert!(!env.intake_dir(&f.down).join("state.json").exists());
    assert_eq!(
        env.git_output(&f.down, &["config", "rerere.enabled"])
            .stdout,
        b""
    );
    assert_eq!(fs::read_dir(&elsewhere).unwrap().count(), 0);
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
    let wt = env.update_worktree(down);
    assert!(
        !git_dir.join("git-intake").join("worktree").exists(),
        "a worktree was created in the git directory"
    );
    let m = env.push_upstream(up, &[("m.txt", "m\n")], "upstream: m");
    let status = env.tool_json(&wt, &["status", "--json"], 0);
    assert_eq!(status["upstreams"][0]["tip"], m.as_str());
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
fn abort_from_a_linked_worktree_removes_an_update_whose_start_was_interrupted() {
    let (env, _up, down, git_dir) = separate_git_dir_downstream("git-dirs/down.git");
    // `start` stopped after adding the update worktree, before it saved the
    // state that records the main worktree.
    let wt = env.update_worktree(&down);
    env.git(
        &down,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "git-intake/update",
            &wt.to_string_lossy(),
            "main",
        ],
    );
    let linked = env.path("side");
    add_worktree(&env, &down, "side", &linked);
    env.tool(&linked, &["abort"])
        .success()
        .stdout(contains("removed worktree"))
        .stdout(contains("removed branch git-intake/update"));
    assert!(!wt.exists(), "the update worktree remains");
    assert!(!env.has_rev(&down, "refs/heads/git-intake/update"));
    assert!(!git_dir.join("git-intake").join("worktree").exists());
}

#[test]
fn own_worktree_on_the_update_branch_is_left_alone() {
    let f = Fork::new();
    let env = &f.env;
    let mine = env.path("mine");
    env.git(
        &f.down,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "git-intake/update",
            &mine.to_string_lossy(),
            "main",
        ],
    );
    fs::write(mine.join("draft.txt"), "uncommitted\n").unwrap();
    let mine = fs::canonicalize(&mine).unwrap();
    let advice = "remove that worktree or switch it to another branch";
    for args in [vec!["abort"], vec!["start"]] {
        let err = stderr(&f.tool(&args).code(2));
        assert!(err.contains(&mine.display().to_string()), "{args:?}: {err}");
        assert!(err.contains(advice), "{args:?}: {err}");
    }
    let doctor = stdout(&f.tool(&["doctor"]).code(1));
    assert!(doctor.contains(advice), "{doctor}");
    f.tool(&["status"]).success();
    assert_eq!(
        fs::read_to_string(mine.join("draft.txt")).unwrap(),
        "uncommitted\n"
    );
    assert_eq!(
        env.git(&mine, &["branch", "--show-current"]),
        "git-intake/update"
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
    let finish = env.tool(&env.update_worktree(&down), &["finish"]).code(2);
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
    let linked = env.path("side");
    add_worktree(&env, &down, "side", &linked);
    let reason = "the main worktree cannot be located from here";
    let start = env.tool(&linked, &["start"]).code(2);
    assert!(stderr(&start).contains(reason), "{}", stderr(&start));
    // Nothing is wrong with the repository, so doctor only warns about the
    // main checkout. The relative URL resolves against the git directory
    // from here, where it reaches no upstream, so the fetch check fails.
    let doctor = env.tool_json(&linked, &["doctor", "--json"], 1);
    let fetch = doctor["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["check"] == "fetch")
        .expect("fetch check");
    assert_eq!(fetch["status"], "fail");
    let check = doctor["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["check"] == "main_checkout_clean")
        .expect("main checkout check");
    assert_eq!(check["status"], "warn");
    assert!(check["detail"].as_str().unwrap().contains(reason));
}
