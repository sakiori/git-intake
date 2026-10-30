//! `contribute`: a local branch at an upstream's fetched tip, from which a
//! change can be offered to the upstream.

use anyhow::{Result, bail};

use crate::Exit;
use crate::cli::UpstreamTarget;
use crate::git::{abbrev, branch_ref};
use crate::repo::Repo;
use crate::tags::UpstreamTags;
use crate::target;
use crate::upstreams::Fetches;

/// Creates `branch` at the upstream's fetched tip, or at the commit its
/// revision names on the fetched branch. Sets no tracking branch, changes no
/// checkout, and pushes nothing.
pub fn run(repo: &Repo, upstream: &UpstreamTarget, branch: &str) -> Result<Exit> {
    let config = repo.load_config()?.config;
    let Some(up) = config.upstream(&upstream.name) else {
        let known: Vec<&str> = config.upstreams.iter().map(|u| u.name.as_str()).collect();
        bail!(
            "unknown upstream '{}'; configured upstreams: {}",
            upstream.name,
            known.join(", ")
        );
    };
    let git = &repo.git;
    let valid = git.raw(&["check-ref-format", "--branch", branch])?;
    if !valid.success() || valid.stdout.trim() != branch {
        bail!("'{branch}' is not a valid branch name");
    }
    let full = branch_ref(branch);
    if let Some(at) = git.commit(&full)? {
        bail!(
            "the branch '{branch}' already exists at {}; name a new branch",
            abbrev(&at)
        );
    }

    let mut fetches = Fetches::new(repo);
    let tip = match fetches.upstream(up)? {
        Ok(tip) => tip,
        Err(why) => bail!("cannot fetch upstream '{}': {why}", up.name),
    };
    let (commit, named) = match &upstream.rev {
        None => (tip, "its fetched tip".to_string()),
        Some(rev) => {
            let tags = match UpstreamTags::list(git, up)? {
                Ok(tags) => tags,
                Err(why) => bail!("{why}"),
            };
            match target::resolve(git, &repo.here, up, &tags, rev, &tip)? {
                Ok(resolved) => (resolved.commit, format!("'{rev}'")),
                Err(why) => bail!("{why}"),
            }
        }
    };
    // An empty old value creates the branch only where none exists.
    git.run(&[
        "update-ref",
        "--create-reflog",
        "-m",
        &format!("git-intake: contribute to {}", up.name),
        &full,
        &commit,
        "",
    ])?;
    drop(fetches);
    println!(
        "created branch '{branch}' at {} of upstream '{}' ({})",
        named,
        up.name,
        abbrev(&commit)
    );
    println!(
        "it holds only upstream history and has no tracking branch; copy the change onto it, then push it to where \
         pull requests to {} come from, such as your fork on the upstream's host. The tool does not push it",
        up.url
    );
    Ok(Exit::Success)
}
