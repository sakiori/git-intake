//! The commit of an upstream that `start` or `contribute` names with a
//! revision.

use anyhow::Result;

use crate::config::Upstream;
use crate::git::{Git, abbrev};

/// Resolves `rev` to a commit of the fetched upstream branch at `tip`, or
/// returns why it cannot be the commit named. `git` asks the upstream's URL
/// and checks the ancestry; `here`, bound to the directory the tool was
/// started in, resolves the revision as the user typed it there, so that
/// `HEAD~1`, `@{-1}` or `ORIG_HEAD` mean that worktree's.
///
/// A tag of the upstream's repository takes precedence; its URL is asked
/// for it. Any other revision is resolved in the repository, except one that
/// goes through one of the repository's own tags: those belong to the
/// repository, not to the upstream, so they cannot stand in for an upstream
/// tag. Either way the commit must be contained in the fetched upstream
/// branch.
pub fn resolve(
    git: &Git,
    here: &Git,
    up: &Upstream,
    rev: &str,
    tip: &str,
) -> Result<std::result::Result<String, String>> {
    let tag = tag_name(rev);
    let upstream_tag = match upstream_tag(git, up, tag)? {
        Ok(found) => found,
        Err(why) => return Ok(Err(why)),
    };
    let (oid, named) = match upstream_tag {
        Some(oid) => (oid, format!("tag '{tag}' of {}", up.url)),
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
            (commit, format!("'{rev}'"))
        }
    };
    // A tagged commit that is not in the repository is not on the fetched
    // branch either; tags are never fetched.
    let on_branch = match git.commit(&oid)? {
        Some(commit) if git.is_ancestor(&commit, tip)? => Some(commit),
        _ => None,
    };
    Ok(on_branch.ok_or_else(|| {
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

/// The object the tag `tag` of the upstream's repository points at, peeled
/// to the object an annotated tag names, or `None` when it has no such tag.
/// Only the tag list at the upstream's URL is read; no tag is fetched. When
/// the URL cannot be asked, returns why.
fn upstream_tag(
    git: &Git,
    up: &Upstream,
    tag: &str,
) -> Result<std::result::Result<Option<String>, String>> {
    let name = format!("refs/tags/{tag}");
    if !git.raw(&["check-ref-format", &name])?.success() {
        return Ok(Ok(None));
    }
    // A pattern matches only the ref it names, so the peeled entry of an
    // annotated tag is asked for by its own name.
    let peeled = format!("{name}^{{}}");
    let out = git.raw(&[
        "ls-remote",
        "--tags",
        "--end-of-options",
        &up.url,
        &name,
        &peeled,
    ])?;
    if !out.success() {
        return Ok(Err(format!(
            "upstream '{}': cannot ask {} whether it has a tag '{tag}': {}",
            up.name,
            up.url,
            out.stderr.trim()
        )));
    }
    Ok(Ok(peeled_tag(&out.stdout, &name)))
}

/// Picks the tag `name` from `git ls-remote` output: the peeled entry
/// (`<name>^{}`) of an annotated tag, or the entry itself for a lightweight
/// tag.
fn peeled_tag(listing: &str, name: &str) -> Option<String> {
    let peeled_name = format!("{name}^{{}}");
    let mut direct = None;
    for line in listing.lines() {
        let Some((oid, refname)) = line.split_once('\t') else {
            continue;
        };
        if refname == peeled_name {
            return Some(oid.to_string());
        }
        if refname == name {
            direct = Some(oid.to_string());
        }
    }
    direct
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

    #[test]
    fn annotated_tags_are_peeled_and_other_refs_ignored() {
        let listing = "aaaa\trefs/tags/v1\nbbbb\trefs/tags/v1^{}\ncccc\trefs/tags/x/refs/tags/v1\n";
        assert_eq!(peeled_tag(listing, "refs/tags/v1").as_deref(), Some("bbbb"));
        assert_eq!(
            peeled_tag("dddd\trefs/tags/v2\n", "refs/tags/v2").as_deref(),
            Some("dddd")
        );
        assert_eq!(
            peeled_tag("cccc\trefs/tags/x/refs/tags/v1\n", "refs/tags/v1"),
            None
        );
    }
}
