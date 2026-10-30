//! `migrate`: an update in progress started by a release of state format 3
//! is carried over in place, with its worktree, branch, merge, index and
//! files.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::{Env, FORK_FILE, Fork, Mono, stderr, stdout};
use predicates::str::contains;

/// The advice every command but `migrate` gives on a state in format 3.
const MIGRATE_ADVICE: &str = "the update in progress was started by a release that keeps its state in format 3; \
     this release (git-intake 0.0.0) understands format 5. Run `git intake migrate` to carry the update over \
     to this release, or finish or abort it with a release that keeps format 3 (git-intake v0.3.0 to v0.3.1)";

/// What `migrate` says once the directory has moved.
const STOP_PROCESSES: &str = "processes, shells and editors that used";

fn state_path(git_dir: &Path) -> PathBuf {
    git_dir.join("git-intake").join("state.json")
}

fn read_state(git_dir: &Path) -> serde_json::Value {
    serde_json::from_str(&fs::read_to_string(state_path(git_dir)).unwrap()).unwrap()
}

/// Rewrites the state of the update in progress in `git_dir` as the
/// previous release wrote it: format 3, with its worktree at `worktree`.
/// Returns the text written.
fn write_v3_state(git_dir: &Path, worktree: &Path) -> String {
    let state = read_state(git_dir);
    // The previous format recorded no release.
    let mut state = without_release(state);
    state["version"] = 3.into();
    state["worktree"] = worktree.to_string_lossy().into_owned().into();
    let text = serde_json::to_string_pretty(&state).unwrap();
    fs::write(state_path(git_dir), &text).unwrap();
    text
}

/// A state as `migrate` leaves it from the previous format, which recorded
/// no release: `state` without its release fields.
fn without_release(mut state: serde_json::Value) -> serde_json::Value {
    let fields = state.as_object_mut().unwrap();
    fields.remove("release");
    fields.remove("release_warned");
    state
}

/// Turns the update in progress in the main worktree `down`, whose git
/// directory is `git_dir`, into one the previous release left: the update
/// worktree in the git directory, no `.worktrees` in the checkout, no
/// exclude line, and the state in format 3. Returns the old worktree.
fn as_previous_release(env: &Env, down: &Path, git_dir: &Path) -> PathBuf {
    let old = fs::canonicalize(git_dir)
        .unwrap()
        .join("git-intake")
        .join("worktree");
    let new = env.update_worktree(down);
    env.git(
        down,
        &[
            "worktree",
            "move",
            &new.to_string_lossy(),
            &old.to_string_lossy(),
        ],
    );
    fs::remove_dir_all(down.join(".worktrees")).unwrap();
    let exclude = env.info_exclude(down);
    let kept: String = fs::read_to_string(&exclude)
        .unwrap()
        .lines()
        .filter(|l| *l != "/.worktrees/")
        .map(|l| format!("{l}\n"))
        .collect();
    fs::write(&exclude, kept).unwrap();
    write_v3_state(git_dir, &old);
    old
}

/// What git records of a worktree's changes: status, staged and unstaged
/// diffs, unmerged entries, and the merge in progress.
fn changes(env: &Env, wt: &Path) -> Vec<String> {
    let merge_head = env.git_output(wt, &["rev-parse", "-q", "--verify", "MERGE_HEAD"]);
    vec![
        env.git(
            wt,
            &[
                "status",
                "--porcelain=v2",
                "--branch",
                "--untracked-files=all",
                "--ignored",
            ],
        ),
        env.git(wt, &["diff"]),
        env.git(wt, &["diff", "--cached"]),
        env.git(wt, &["ls-files", "--stage", "--unmerged"]),
        String::from_utf8_lossy(&merge_head.stdout).into_owned(),
    ]
}

/// A fork whose update, started by the previous release, merged its
/// upstream. Returns the fork and the old worktree.
fn merged_fork() -> (Fork, PathBuf) {
    let f = Fork::new();
    f.env
        .push_upstream(&f.up, &[("n.txt", "n\n")], "upstream: n");
    f.tool(&["start"]).success();
    let old = as_previous_release(&f.env, &f.down, &f.down.join(".git"));
    (f, old)
}

#[test]
fn every_other_command_names_migrate_for_a_state_of_the_previous_release() {
    let (f, old) = merged_fork();
    let env = &f.env;
    let written = fs::read_to_string(state_path(&f.down.join(".git"))).unwrap();
    let main = env.rev(&f.down, "main");
    for args in [
        vec!["status"],
        vec!["continue"],
        vec!["verify"],
        vec!["finish"],
        vec!["abort"],
        vec!["start"],
        vec!["report"],
    ] {
        for dir in [&f.down, &old] {
            let err = stderr(&env.tool(dir, &args).code(2));
            assert!(
                err.contains(MIGRATE_ADVICE),
                "{args:?} in {}: {err}",
                dir.display()
            );
        }
    }
    assert_eq!(
        fs::read_to_string(state_path(&f.down.join(".git"))).unwrap(),
        written
    );
    assert!(
        old.join("n.txt").exists(),
        "the update worktree was touched"
    );
    assert_eq!(env.rev(&f.down, "main"), main);
    assert!(!f.down.join(".worktrees").exists());
    let doctor = stdout(&f.tool(&["doctor"]).code(1));
    assert!(doctor.contains("git intake migrate"), "{doctor}");
}

#[test]
fn merged_update_moves_with_its_uncommitted_and_untracked_files() {
    let (f, old) = merged_fork();
    let env = &f.env;
    let branch = env.rev(&f.down, "refs/heads/git-intake/update");
    let refs = env.intake_refs(&f.down);
    fs::write(old.join(FORK_FILE), "one\ntwo\nthree\nunstaged\n").unwrap();
    fs::write(old.join("staged.txt"), "staged\n").unwrap();
    env.git(&old, &["add", "staged.txt"]);
    fs::create_dir_all(old.join("untracked")).unwrap();
    fs::write(old.join("untracked").join("u.txt"), "untracked\n").unwrap();
    // Ignored content, such as installed dependencies, moves too.
    let exclude = env.info_exclude(&f.down);
    let mut text = fs::read_to_string(&exclude).unwrap();
    text.push_str("node_modules/\n");
    fs::write(&exclude, text).unwrap();
    fs::create_dir_all(old.join("node_modules").join("dep")).unwrap();
    fs::write(
        old.join("node_modules").join("dep").join("index.js"),
        "ok\n",
    )
    .unwrap();
    let before = changes(env, &old);
    assert!(before[0].contains("! node_modules/"), "{}", before[0]);

    let migrate = f.tool(&["migrate"]).success();
    let out = stdout(&migrate);
    let new = env.update_worktree(&f.down);
    let new = fs::canonicalize(&new).unwrap();
    assert!(
        out.contains(&format!(
            "moved the update worktree from {} to {}",
            old.display(),
            new.display()
        )),
        "{out}"
    );
    assert!(out.contains(STOP_PROCESSES), "{out}");
    assert!(
        out.contains("migrated the update in progress to state format 5"),
        "{out}"
    );
    assert!(!old.exists(), "the old worktree remains");
    assert_eq!(changes(env, &new), before);
    assert_eq!(env.rev(&f.down, "refs/heads/git-intake/update"), branch);
    assert_eq!(env.intake_refs(&f.down), refs);
    let state = read_state(&f.down.join(".git"));
    assert_eq!(state["version"], 5);
    assert_eq!(state["worktree"], new.to_string_lossy().as_ref());
    assert!(
        fs::read_to_string(env.info_exclude(&f.down))
            .unwrap()
            .lines()
            .any(|l| l == "/.worktrees/")
    );
    assert!(env.git(&f.down, &["status", "--porcelain"]).is_empty());

    let status = env.tool_json(&f.down, &["status", "--json"], 0);
    assert_eq!(status["update"]["worktree"], new.to_string_lossy().as_ref());
    assert_eq!(status["update"]["step"], "merged");
    f.tool(&["continue"])
        .success()
        .stdout(contains("every queued upstream is already merged"));
    f.tool(&["verify"])
        .code(2)
        .stderr(contains("has uncommitted changes"));
    env.git(&new, &["reset", "-q", "--hard"]);
    fs::remove_dir_all(new.join("untracked")).unwrap();
    f.tool(&["verify"]).success();
    f.tool(&["finish"]).success();
    assert!(f.down.join("n.txt").exists());
    env.assert_no_update(&f.down);
}

#[test]
fn merge_in_progress_moves_with_its_staged_and_unstaged_conflicts() {
    let f = Fork::new();
    let env = &f.env;
    env.commit(
        &f.down,
        &[(FORK_FILE, "one\nTWO-fork\nthree\n"), ("b.txt", "fork\n")],
        "fork: change two, add b",
    );
    env.git(&f.down, &["push", "-q", "origin", "main"]);
    env.push_upstream(
        &f.up,
        &[
            (FORK_FILE, "one\nTWO-upstream\nthree\n"),
            ("b.txt", "upstream\n"),
        ],
        "upstream: change two, add b",
    );
    f.tool(&["start"]).code(1);
    let old = as_previous_release(env, &f.down, &f.down.join(".git"));
    // One conflict resolved and staged, the other left in the worktree.
    fs::write(old.join(FORK_FILE), "one\nTWO-both\nthree\n").unwrap();
    env.git(&old, &["add", FORK_FILE]);
    fs::write(old.join("b.txt"), "both\n").unwrap();
    let before = changes(env, &old);
    assert!(!before[4].is_empty(), "no merge in progress");

    f.tool(&["migrate"]).success();
    let new = fs::canonicalize(env.update_worktree(&f.down)).unwrap();
    assert!(!old.exists());
    assert_eq!(changes(env, &new), before);
    let status = env.tool_json(&f.down, &["status", "--json"], 0);
    assert_eq!(status["update"]["step"], "conflict");
    f.tool(&["continue"])
        .code(2)
        .stderr(contains("still has unresolved conflicts"));
    env.git(&new, &["add", "b.txt"]);
    f.tool(&["continue"])
        .success()
        .stdout(contains("upstream: merged as"));
    assert_eq!(fs::read_to_string(new.join("b.txt")).unwrap(), "both\n");
    f.tool(&["verify"]).success();
    f.tool(&["finish"]).success();
    env.assert_no_update(&f.down);
}

#[test]
fn verification_run_in_the_old_location_is_dropped_with_the_move() {
    let (f, _) = merged_fork();
    let env = &f.env;
    // The previous release verified the update in its old location.
    let git_dir = f.down.join(".git");
    let old = PathBuf::from(read_state(&git_dir)["worktree"].as_str().unwrap());
    env.tool(&f.down, &["migrate"]).success();
    f.tool(&["verify"]).success();
    let new = env.update_worktree(&f.down);
    env.git(
        &f.down,
        &[
            "worktree",
            "move",
            &new.to_string_lossy(),
            &old.to_string_lossy(),
        ],
    );
    fs::remove_dir_all(f.down.join(".worktrees")).unwrap();
    write_v3_state(&git_dir, &old);
    let verified = read_state(&git_dir)["verification"]["commit"].clone();
    assert!(verified.is_string());

    let out = stdout(&f.tool(&["migrate"]).success());
    assert!(out.contains("dropped the verification of"), "{out}");
    assert!(out.contains("next: git intake verify"), "{out}");
    assert!(read_state(&git_dir)["verification"].is_null());
    f.tool(&["finish"])
        .code(2)
        .stderr(contains("the update is not verified"));
    f.tool(&["verify"]).success();
    f.tool(&["finish"]).success();
}

#[test]
fn bare_repository_state_is_rewritten_and_its_verification_kept() {
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
    let bare = fs::canonicalize(bare).unwrap();
    env.tool(&bare, &["start"]).success();
    let wt = bare.join("git-intake").join("worktree");
    env.tool(&wt, &["verify"]).success();
    let before = without_release(read_state(&bare));
    write_v3_state(&bare, &wt);
    fs::write(wt.join("untracked.txt"), "u\n").unwrap();
    let files = changes(env, &wt);

    let out = stdout(&env.tool(&bare, &["migrate"]).success());
    assert!(
        out.contains(&format!("the update worktree stays at {}", wt.display())),
        "{out}"
    );
    assert!(!out.contains(STOP_PROCESSES), "{out}");
    assert_eq!(read_state(&bare), before, "only the format changed");
    assert_eq!(changes(env, &wt), files);
    fs::remove_file(wt.join("untracked.txt")).unwrap();
    env.tool(&wt, &["finish"]).success();
    assert!(!wt.exists());
}

#[test]
fn interrupted_migrate_finishes_the_state_once_the_worktree_moved() {
    let (f, old) = merged_fork();
    let env = &f.env;
    fs::write(old.join("untracked.txt"), "u\n").unwrap();
    let before = changes(env, &old);
    // The move happened; writing the state did not.
    let new = env.update_worktree(&f.down);
    fs::create_dir_all(new.parent().unwrap()).unwrap();
    env.git(
        &f.down,
        &[
            "worktree",
            "move",
            &old.to_string_lossy(),
            &new.to_string_lossy(),
        ],
    );
    assert_eq!(read_state(&f.down.join(".git"))["version"], 3);

    let out = stdout(&f.tool(&["migrate"]).success());
    let new = fs::canonicalize(&new).unwrap();
    assert!(
        out.contains(&format!(
            "the update worktree is already at {}",
            new.display()
        )),
        "{out}"
    );
    let state = read_state(&f.down.join(".git"));
    assert_eq!(state["version"], 5);
    assert_eq!(state["worktree"], new.to_string_lossy().as_ref());
    assert_eq!(changes(env, &new), before);
    assert!(
        fs::read_to_string(env.info_exclude(&f.down))
            .unwrap()
            .lines()
            .any(|l| l == "/.worktrees/")
    );
    f.tool(&["status"]).success();
}

/// Asserts that a refused `migrate` changed nothing: the state text, the
/// old worktree and its files.
fn assert_unchanged(env: &Env, git_dir: &Path, state: &str, old: &Path, files: &[String]) {
    assert_eq!(fs::read_to_string(state_path(git_dir)).unwrap(), state);
    assert!(old.exists(), "the old worktree moved");
    assert_eq!(changes(env, old), files);
}

#[test]
fn existing_destination_is_refused_before_anything_changes() {
    let (f, old) = merged_fork();
    let env = &f.env;
    let state = fs::read_to_string(state_path(&f.down.join(".git"))).unwrap();
    let files = changes(env, &old);
    let dest = env.update_worktree(&f.down);
    fs::create_dir_all(&dest).unwrap();
    let err = stderr(&f.tool(&["migrate"]).code(2));
    let dest_shown = fs::canonicalize(&dest).unwrap();
    assert!(
        err.contains(&format!(
            "{} already exists, and the update worktree would be moved there",
            dest_shown.display()
        )),
        "{err}"
    );
    assert!(
        err.contains(&format!("git worktree repair {}", dest_shown.display())),
        "{err}"
    );
    assert!(!err.contains("remove"), "{err}");
    assert_unchanged(env, &f.down.join(".git"), &state, &old, &files);
    assert_eq!(fs::read_dir(&dest).unwrap().count(), 0);
}

#[test]
fn worktree_moved_by_hand_is_repaired_and_kept() {
    let (f, old) = merged_fork();
    let env = &f.env;
    fs::write(old.join("untracked.txt"), "u\n").unwrap();
    let before = changes(env, &old);
    // Moved without git, as by `mv`, or by a `git worktree move` stopped
    // before it updated its records: git still lists the old path.
    let new = env.update_worktree(&f.down);
    fs::create_dir_all(new.parent().unwrap()).unwrap();
    fs::rename(&old, &new).unwrap();
    let new = fs::canonicalize(&new).unwrap();

    let out = stdout(&f.tool(&["migrate"]).success());
    assert!(
        out.contains(&format!(
            "the update worktree is already at {}",
            new.display()
        )),
        "{out}"
    );
    let list = env.git(&f.down, &["worktree", "list", "--porcelain"]);
    assert!(
        list.contains(&format!("worktree {}", new.display())),
        "{list}"
    );
    assert!(!list.contains("prunable"), "{list}");
    assert_eq!(changes(env, &new), before);
    let state = read_state(&f.down.join(".git"));
    assert_eq!(state["version"], 5);
    assert_eq!(state["worktree"], new.to_string_lossy().as_ref());
    fs::remove_file(new.join("untracked.txt")).unwrap();
    f.tool(&["verify"]).success();
    f.tool(&["finish"]).success();
    env.assert_no_update(&f.down);
}

#[test]
fn worktrees_directory_that_is_a_link_is_refused_before_anything_changes() {
    let (f, old) = merged_fork();
    let env = &f.env;
    let state = fs::read_to_string(state_path(&f.down.join(".git"))).unwrap();
    let files = changes(env, &old);
    let elsewhere = env.path("elsewhere");
    fs::create_dir_all(&elsewhere).unwrap();
    std::os::unix::fs::symlink(&elsewhere, f.down.join(".worktrees")).unwrap();
    let err = stderr(&f.tool(&["migrate"]).code(2));
    assert!(err.contains("symbolic link"), "{err}");
    assert_unchanged(env, &f.down.join(".git"), &state, &old, &files);
    assert_eq!(fs::read_dir(&elsewhere).unwrap().count(), 0);
}

#[test]
fn move_git_refuses_is_relayed_before_anything_changes() {
    let (f, old) = merged_fork();
    let env = &f.env;
    env.git(
        &f.down,
        &[
            "worktree",
            "lock",
            "--reason",
            "kept for a test",
            &old.to_string_lossy(),
        ],
    );
    let state = fs::read_to_string(state_path(&f.down.join(".git"))).unwrap();
    let files = changes(env, &old);
    let err = stderr(&f.tool(&["migrate"]).code(2));
    assert!(
        err.contains("git refused to move the update worktree"),
        "{err}"
    );
    assert!(err.contains("locked"), "{err}");
    assert_unchanged(env, &f.down.join(".git"), &state, &old, &files);
    assert!(
        !f.down.join(".worktrees").exists(),
        "the directories made for it remain"
    );
    let exclude = fs::read_to_string(env.info_exclude(&f.down)).unwrap();
    assert!(!exclude.lines().any(|l| l == "/.worktrees/"), "{exclude}");
}

/// A downstream whose git directory `down.git` lies outside its checkout,
/// with an update the previous release started from the checkout. Returns
/// the fork-like parts: environment, checkout, git directory and old
/// worktree.
fn separate_git_dir_previous_release() -> (Env, PathBuf, PathBuf, PathBuf) {
    let env = Env::new();
    let up = env.upstream("upstream", &[(FORK_FILE, "one\n")]);
    let down = env.path("down");
    let git_dir = env.path("git-dirs").join("down.git");
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
    let config = format!(
        "integration = \"main\"\nchecks = [\"test -f {FORK_FILE}\"]\n\n\
         [[upstream]]\nname = \"upstream\"\nurl = \"../upstream.git\"\nbranch = \"main\"\n"
    );
    env.commit(
        &down,
        &[(".git-intake.toml", &config)],
        "down: configuration",
    );
    env.push_upstream(&up, &[("n.txt", "n\n")], "upstream: n");
    env.tool(&down, &["start"]).success();
    let git_dir = fs::canonicalize(git_dir).unwrap();
    let old = as_previous_release(&env, &down, &git_dir);
    (env, down, git_dir, old)
}

#[test]
fn migrate_from_the_old_worktree_finds_the_main_worktree_in_the_state() {
    let (env, down, git_dir, old) = separate_git_dir_previous_release();
    let migrate = env.tool(&old, &["migrate"]).success();
    let new = fs::canonicalize(env.update_worktree(&down)).unwrap();
    assert!(
        stderr(&migrate).contains(&format!(
            "note: the current directory was in the update worktree, which moved to {}",
            new.display()
        )),
        "{}",
        stderr(&migrate)
    );
    assert!(!old.exists());
    assert_eq!(
        read_state(&git_dir)["worktree"],
        new.to_string_lossy().as_ref()
    );
    env.tool(&new, &["verify"]).success();
    env.tool(&new, &["finish"]).success();
    assert!(down.join("n.txt").exists());
}

#[test]
fn main_worktree_that_cannot_be_located_is_refused() {
    let (env, _, git_dir, old) = separate_git_dir_previous_release();
    let mut state = read_state(&git_dir);
    state["main_worktree"] = serde_json::Value::Null;
    let text = serde_json::to_string_pretty(&state).unwrap();
    fs::write(state_path(&git_dir), &text).unwrap();
    let files = changes(&env, &old);
    let err = stderr(&env.tool(&old, &["migrate"]).code(2));
    assert!(
        err.contains("the main worktree cannot be located from here"),
        "{err}"
    );
    assert_unchanged(&env, &git_dir, &text, &old, &files);
}

#[test]
fn current_state_and_no_update_are_refused() {
    let f = Fork::new();
    let env = &f.env;
    f.tool(&["migrate"]).code(2).stderr(contains(
        "no update is in progress; there is nothing to migrate",
    ));
    env.push_upstream(&f.up, &[("n.txt", "n\n")], "upstream: n");
    f.tool(&["start"]).success();
    let state = fs::read_to_string(state_path(&f.down.join(".git"))).unwrap();
    f.tool(&["migrate"]).code(2).stderr(contains(
        "the update in progress already keeps its state in format 5; there is nothing to migrate",
    ));
    assert_eq!(
        fs::read_to_string(state_path(&f.down.join(".git"))).unwrap(),
        state
    );
}

#[test]
fn state_older_than_the_previous_release_keeps_its_advice() {
    let f = Fork::new();
    let dir = f.env.intake_dir(&f.down);
    fs::create_dir_all(&dir).unwrap();
    let text =
        r#"{"version": 2, "config": {"integration": "main", "upstreams": []}, "upstreams": []}"#;
    fs::write(dir.join("state.json"), text).unwrap();
    let err = stderr(&f.tool(&["migrate"]).code(2));
    assert!(
        err.contains(
            "keeps its state in format 2; this release (git-intake 0.0.0) understands format 5. \
             Finish or abort the update with a release that keeps format 2 (git-intake v0.2.0 to v0.2.2)"
        ),
        "{err}"
    );
    assert_eq!(fs::read_to_string(dir.join("state.json")).unwrap(), text);
}

#[test]
fn update_waiting_for_its_merge_request_keeps_its_verification() {
    let f = Fork::merge_request();
    let env = &f.env;
    env.push_upstream(&f.up, &[("n.txt", "n\n")], "upstream: n");
    f.tool(&["start"]).success();
    f.tool(&["verify"]).success();
    f.tool(&["finish"]).success();
    let git_dir = f.down.join(".git");
    let old = fs::canonicalize(&git_dir)
        .unwrap()
        .join("git-intake")
        .join("worktree");
    let before = without_release(read_state(&git_dir));
    assert!(!env.update_worktree(&f.down).exists());
    write_v3_state(&git_dir, &old);

    let out = stdout(&f.tool(&["migrate"]).success());
    assert!(
        out.contains("the update has no worktree checked out"),
        "{out}"
    );
    assert!(!out.contains("dropped the verification"), "{out}");
    let mut after = read_state(&git_dir);
    assert_eq!(
        after["worktree"],
        fs::canonicalize(&f.down)
            .unwrap()
            .join(".worktrees/git-intake/update")
            .to_string_lossy()
            .as_ref()
    );
    after["worktree"] = before["worktree"].clone();
    assert_eq!(after, before);
    let status = env.tool_json(&f.down, &["status", "--json"], 0);
    assert_eq!(status["update"]["step"], "waiting_for_merge_request");
    f.tool(&["abort"]).success();
    env.assert_no_update(&f.down);
}

#[test]
fn verification_is_kept_once_finish_has_begun() {
    let (f, _) = merged_fork();
    let env = &f.env;
    let git_dir = f.down.join(".git");
    let old = PathBuf::from(read_state(&git_dir)["worktree"].as_str().unwrap());
    f.tool(&["migrate"]).success();
    f.tool(&["verify"]).success();
    // The publish remote rejects the push: the update stays finishing.
    let hook = f.publish.join("hooks").join("pre-receive");
    fs::create_dir_all(hook.parent().unwrap()).unwrap();
    fs::write(&hook, "#!/bin/sh\nexit 1\n").unwrap();
    let mut perms = fs::metadata(&hook).unwrap().permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o755);
    fs::set_permissions(&hook, perms).unwrap();
    f.tool(&["finish"]).code(2);
    assert!(read_state(&git_dir)["finishing"].is_object());
    let new = env.update_worktree(&f.down);
    env.git(
        &f.down,
        &[
            "worktree",
            "move",
            &new.to_string_lossy(),
            &old.to_string_lossy(),
        ],
    );
    fs::remove_dir_all(f.down.join(".worktrees")).unwrap();
    write_v3_state(&git_dir, &old);
    let verification = read_state(&git_dir)["verification"].clone();

    let out = stdout(&f.tool(&["migrate"]).success());
    assert!(!out.contains("dropped the verification"), "{out}");
    assert_eq!(read_state(&git_dir)["verification"], verification);
    fs::remove_file(&hook).unwrap();
    f.tool(&["finish"]).success();
    env.assert_no_update(&f.down);
}

#[test]
fn bare_repository_at_a_path_containing_dot_git_drops_its_verification() {
    let m = Mono::new();
    let env = &m.env;
    let first = env.path("mono-bare.git");
    env.git(
        &env.path(""),
        &[
            "clone",
            "-q",
            "--bare",
            &m.down.to_string_lossy(),
            &first.to_string_lossy(),
        ],
    );
    let first = fs::canonicalize(first).unwrap();
    env.tool(&first, &["start"]).success();
    env.tool(&first.join("git-intake").join("worktree"), &["verify"])
        .success();
    // The previous release placed the update worktree of a bare repository
    // that is a directory named `.git` in that directory too.
    let store = env.path("store");
    fs::create_dir_all(&store).unwrap();
    let bare = fs::canonicalize(&store).unwrap().join(".git");
    fs::rename(&first, &bare).unwrap();
    let wt = bare.join("git-intake").join("worktree");
    env.git(&bare, &["worktree", "repair", &wt.to_string_lossy()]);
    write_v3_state(&bare, &wt);
    assert!(read_state(&bare)["verification"].is_object());

    let out = stdout(&env.tool(&bare, &["migrate"]).success());
    assert!(
        out.contains(&format!("the update worktree stays at {}", wt.display())),
        "{out}"
    );
    assert!(out.contains("dropped the verification of"), "{out}");
    assert!(out.contains("`/.git`"), "{out}");
    assert!(read_state(&bare)["verification"].is_null());
    env.tool(&wt, &["finish"])
        .code(2)
        .stderr(contains("the update is not verified"));
}
