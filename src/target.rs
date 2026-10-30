//! The commit of an upstream that `start` or `contribute` names with a
//! revision.

use anyhow::Result;

use crate::config::Upstream;
use crate::git::{Git, abbrev};
use crate::tags::UpstreamTags;

/// The commit a revision names, with the upstream tag it named, if any.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub commit: String,
    /// The tag of the upstream's repository the revision named.
    pub tag: Option<String>,
}

/// Resolves `rev` to a commit of the fetched upstream branch at `tip`, or
/// returns why it cannot be the commit named. `tags` are the tags of the
/// upstream's repository, and `git` checks the ancestry; `here`, bound to
/// the directory the tool was started in, resolves the revision as the user
/// typed it there, so that `HEAD~1`, `@{-1}` or `ORIG_HEAD` mean that
/// worktree's.
///
/// A tag of the upstream's repository takes precedence. Any other revision
/// is resolved in the repository, except one that goes through one of the
/// repository's own tags: those belong to the repository, not to the
/// upstream, so they cannot stand in for an upstream tag. Either way the
/// commit must be contained in the fetched upstream branch.
pub fn resolve(
    git: &Git,
    here: &Git,
    up: &Upstream,
    tags: &UpstreamTags,
    rev: &str,
    tip: &str,
) -> Result<std::result::Result<Resolved, String>> {
    let tag = tag_name(rev);
    let (oid, named, upstream_tag) = match tags.get(tag) {
        Some(oid) => (
            oid.to_string(),
            format!("tag '{tag}' of {}", up.url),
            Some(tag.to_string()),
        ),
        None => {
            let Some(commit) = here.commit(rev)? else {
                return Ok(Err(format!(
                    "upstream '{}': '{rev}' names no commit and no tag of {}",
                    up.name, up.url
                )));
            };
            if let Some(local) = own_tag(here, rev)? {
                return Ok(Err(format!(
                    "upstream '{}': {} has no tag '{tag}', and '{rev}' resolves through \
                     the repository's own tag {local}; name a tag of the upstream's repository or a commit",
                    up.name, up.url
                )));
            }
            (commit, format!("'{rev}'"), None)
        }
    };
    // A tagged commit that is not in the repository is not on the fetched
    // branch either; tags are never fetched.
    let on_branch = match git.commit(&oid)? {
        Some(commit) if git.is_ancestor(&commit, tip)? => Some(commit),
        _ => None,
    };
    Ok(on_branch
        .map(|commit| Resolved {
            commit,
            tag: upstream_tag,
        })
        .ok_or_else(|| {
            format!(
                "upstream '{}': {named} at {} is not in the history of {} as fetched; \
                 name a commit of the upstream branch the tool takes from",
                up.name,
                abbrev(&oid),
                up.source()
            )
        }))
}

/// The tag name a revision would name in the upstream's repository: the
/// revision without a leading `refs/tags/` or `tags/`.
fn tag_name(rev: &str) -> &str {
    rev.strip_prefix("refs/tags/")
        .or_else(|| rev.strip_prefix("tags/"))
        .unwrap_or(rev)
}

/// The repository's own tag that `rev` resolves through, as a full ref name:
/// the revision itself, or the name an expression such as `v2.3~1` starts
/// from. The tag ref is looked up directly, so that a name that is also a
/// branch, which git resolves through the tag, is still found.
fn own_tag(git: &Git, rev: &str) -> Result<Option<String>> {
    let start = rev.split(['~', '^', '@', ':']).next().unwrap_or_default();
    for name in [rev, start] {
        if name.is_empty() {
            continue;
        }
        let full = format!("refs/tags/{}", tag_name(name));
        let out = git.raw(&["rev-parse", "-q", "--verify", "--end-of-options", &full])?;
        if out.success() {
            return Ok(Some(full));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_prefixes_are_stripped() {
        assert_eq!(tag_name("refs/tags/v2.3"), "v2.3");
        assert_eq!(tag_name("tags/v2.3"), "v2.3");
        assert_eq!(tag_name("v2.3"), "v2.3");
    }
}
