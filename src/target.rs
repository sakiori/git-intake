//! The target of an upstream that `start` names with a revision.

use anyhow::{Context, Result};

use crate::config::Upstream;
use crate::git::{Git, abbrev};

/// Resolves `rev` to a commit of the fetched upstream branch at `tip`, or
/// returns why it cannot be the upstream's target.
///
/// A tag of the upstream's remote takes precedence, so that a tag of the
/// repository with the same name cannot stand in for it; any other revision
/// is resolved in the repository. Either way the commit must be contained in
/// the fetched upstream branch.
pub fn resolve(
    git: &Git,
    up: &Upstream,
    rev: &str,
    tip: &str,
) -> Result<std::result::Result<String, String>> {
    let (oid, named) = match remote_tag(git, up, rev)? {
        Some(oid) => (oid, format!("tag '{rev}' of remote '{}'", up.remote)),
        None => match git.commit(rev)? {
            Some(commit) => (commit, format!("'{rev}'")),
            None => {
                return Ok(Err(format!(
                    "upstream '{}': '{rev}' names no commit and no tag of remote '{}'",
                    up.name, up.remote
                )));
            }
        },
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
             a target must be a commit of the upstream branch the tool syncs",
            up.name,
            abbrev(&oid),
            up.source()
        )
    }))
}

/// The object a tag of the upstream's remote points at, peeled to the object
/// an annotated tag names, or `None` when the remote has no such tag. Only
/// the remote's tag list is read; no tag is fetched.
fn remote_tag(git: &Git, up: &Upstream, rev: &str) -> Result<Option<String>> {
    let name = format!("refs/tags/{rev}");
    if !git.raw(&["check-ref-format", &name])?.success() {
        return Ok(None);
    }
    // A pattern matches only the ref it names, so the peeled entry of an
    // annotated tag is asked for by its own name.
    let peeled = format!("{name}^{{}}");
    let listing = git
        .run(&["ls-remote", "--tags", &up.remote, &name, &peeled])
        .with_context(|| {
            format!(
                "cannot list the tags of remote '{}' for upstream '{}'",
                up.remote, up.name
            )
        })?;
    Ok(peeled_tag(&listing, &name))
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
