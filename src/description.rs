//! The title and description of the update's merge request, which belong to
//! the tool.
//!
//! The description says what the update takes in, how to merge it, and what
//! it changes of its own checks. It ends with a block, hidden when rendered,
//! that records each follow upstream's target: `sync` reads it back from a
//! merge request closed without merging to tell the targets a person
//! declined, and from a merged one to tell whether the merge kept the
//! update's history. A description without a readable block, as one written
//! by an earlier release or edited by hand, or the note a replaced merge
//! request gets, records nothing.

use std::collections::BTreeMap;

/// How the hidden block starts.
const BLOCK_START: &str = "<!-- git-intake targets";

/// The first line of the description of a merge request a replacement
/// closed.
const REPLACED: &str = "Upstream update by git intake, replaced.";
/// How it ends.
const BLOCK_END: &str = "-->";

/// The follow targets an update's merge request records, by upstream name.
pub type Targets = BTreeMap<String, String>;

/// What the description of an update's merge request says.
#[derive(Debug, Clone, Default)]
pub struct Content {
    /// One line per upstream: its merged range, or for a port its taken
    /// commits and decisions.
    pub upstreams: Vec<String>,
    /// The forge's term, `merge request` or `pull request`.
    pub term: &'static str,
    pub integration: String,
    /// Changes the update makes to its own checks: the forge's CI definition,
    /// or the install command and checks of the configuration.
    pub check_changes: Vec<String>,
    /// The merge request merged without its history whose targets this one
    /// proposes again, as `!5 (<address>)`.
    pub retry_of: Option<String>,
    /// The merge request this one replaces, and the releases that caused
    /// the replacement.
    pub replaces: Option<Replaces>,
    pub targets: Targets,
}

/// What the description of a merge request that replaces another says of
/// it.
#[derive(Debug, Clone, Default)]
pub struct Replaces {
    /// The replaced merge request, as `!5 (<address>)`.
    pub merge_request: String,
    /// Each newer release, such as `` `app` v1.3.0 ``.
    pub releases: Vec<String>,
}

/// The title of the merge request of an update of `names`.
pub fn title(names: &[String]) -> String {
    format!(
        "Merge upstream update{}: {}",
        if names.len() == 1 { "" } else { "s" },
        names.join(", ")
    )
}

/// The description, no longer than `limit` characters: a description too
/// long for the forge is shortened before the hidden block.
pub fn render(content: &Content, limit: usize) -> String {
    let term = content.term;
    let mut body = String::from("Upstream update by git intake.\n\n");
    for line in &content.upstreams {
        body.push_str(&format!("- {line}\n"));
    }
    body.push_str(&format!(
        "\nMerge this {term} without squashing it and without rebasing its branch, including with the forge's \
         rebase or update-with-rebase buttons: either drops the update's merge commits, whose trailers record what \
         it merged. Keep it current with `git intake sync`, which catches it up with `{}` by a merge.\n",
        content.integration
    ));
    if !content.check_changes.is_empty() {
        body.push_str(&format!(
            "\nThis update changes its own checks, so the pipeline that verifies it runs what the update brings: {}.\n",
            content.check_changes.join(", ")
        ));
    }
    if let Some(retry) = &content.retry_of {
        body.push_str(&format!(
            "\nThe {term} {retry} was merged without the update's history, which a squash or rebase drops, so its \
             sync records never landed; this {term} proposes the same targets again, and merging it records them.\n"
        ));
    }
    if let Some(replaces) = &content.replaces {
        body.push_str(&format!(
            "\nThis {term} replaces the {term} {}, on which, as far as the forge reported, nobody had approved, \
             reviewed, commented or otherwise acted: git intake closed it without merging when {} came out, so \
             that the review starts from the newest release. Nothing of it was merged.\n",
            replaces.merge_request,
            match replaces.releases.is_empty() {
                true => "newer releases".to_string(),
                false => replaces.releases.join(", "),
            }
        ));
    }
    let block = block(&content.targets);
    let total = body.chars().count() + 1 + block.chars().count();
    if total > limit {
        const NOTE: &str = "\n\n(The description was shortened to fit the forge; `git intake report` lists the \
                            whole update.)\n";
        let keep = limit.saturating_sub(block.chars().count() + 1 + NOTE.chars().count());
        body = body.chars().take(keep).collect();
        body.push_str(NOTE);
    }
    format!("{body}\n{block}")
}

/// The description a merge request gets when `sync` closes it to replace it
/// with an update at `releases`. It records no targets, so it declines
/// nothing.
pub fn replaced(term: &str, releases: &[String]) -> String {
    format!(
        "{REPLACED}\n\nThis {term} was untouched when {} came out, so git intake \
         closed it without merging and proposed an update at the newer release{} in a new {term}. Nothing of it was \
         merged; its changes are part of the new {term}, which is reviewed from scratch.\n",
        releases.join(", "),
        if releases.len() == 1 { "" } else { "s" }
    )
}

/// Whether `description` is the one [`replaced`] writes.
pub fn is_replaced(description: &str) -> bool {
    description.starts_with(REPLACED)
}

/// The hidden block recording each follow target.
fn block(targets: &Targets) -> String {
    let mut out = format!("{BLOCK_START}\n");
    for (name, target) in targets {
        out.push_str(&format!("{name} {target}\n"));
    }
    out.push_str(BLOCK_END);
    out.push('\n');
    out
}

/// The follow targets the hidden block at the end of `description`
/// records, or `None` when it has no readable block.
pub fn targets(description: &str) -> Option<Targets> {
    let start = description.rfind(BLOCK_START)?;
    let rest = &description[start + BLOCK_START.len()..];
    let end = rest.find(BLOCK_END)?;
    let mut targets = Targets::new();
    for line in rest[..end].lines().map(str::trim).filter(|l| !l.is_empty()) {
        let (name, target) = line.split_once(' ')?;
        let target = target.trim().to_ascii_lowercase();
        let valid_name = !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
        if !valid_name || !crate::port::is_full_id(&target) || targets.contains_key(name) {
            return None;
        }
        targets.insert(name.to_string(), target);
    }
    Some(targets)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn content() -> Content {
        let mut targets = Targets::new();
        targets.insert("app-a".into(), "a".repeat(40));
        targets.insert("app.b".into(), "b".repeat(40));
        Content {
            upstreams: vec!["app-a from app-a/main: 1111..aaaa (3 commits, tag v1.2.0)".into()],
            term: "pull request",
            integration: "main".into(),
            check_changes: vec!["`.github/workflows/ci.yml`".into()],
            retry_of: Some("#5 (https://github.com/team/app/pull/5)".into()),
            replaces: None,
            targets,
        }
    }

    #[test]
    fn the_block_records_the_targets_and_reads_back() {
        let text = render(&content(), 65_536);
        assert!(
            text.contains("without squashing it and without rebasing its branch"),
            "{text}"
        );
        assert!(text.contains("`git intake sync`"), "{text}");
        assert!(text.contains("`.github/workflows/ci.yml`"), "{text}");
        assert!(
            text.contains("#5 (https://github.com/team/app/pull/5) was merged without"),
            "{text}"
        );
        assert!(text.ends_with(&format!(
            "app-a {}\napp.b {}\n-->\n",
            "a".repeat(40),
            "b".repeat(40)
        )));
        assert_eq!(targets(&text), Some(content().targets));
    }

    #[test]
    fn descriptions_without_a_readable_block_record_nothing() {
        assert_eq!(targets("Upstream changes verified by git intake."), None);
        assert_eq!(targets("<!-- git-intake targets\napp-a nothex\n-->"), None);
        assert_eq!(targets("<!-- git-intake targets\napp-a deadbeef\n"), None);
        // An update of port upstreams alone records no follow target.
        assert_eq!(
            targets("x\n<!-- git-intake targets\n-->\n"),
            Some(Targets::new())
        );
        // The block a person quoted above the tool's does not count.
        let quoted = format!(
            "<!-- git-intake targets\napp-a {}\n-->\n{}",
            "c".repeat(40),
            render(&content(), 65_536)
        );
        assert_eq!(targets(&quoted), Some(content().targets));
    }

    #[test]
    fn a_long_description_is_shortened_before_the_block() {
        let mut c = content();
        c.upstreams = (0..2000)
            .map(|i| format!("upstream {i} with a long line"))
            .collect();
        let text = render(&c, 4000);
        assert!(text.chars().count() <= 4000, "{}", text.chars().count());
        assert!(text.contains("The description was shortened"), "{text}");
        assert_eq!(targets(&text), Some(c.targets));
    }

    #[test]
    fn a_replacement_names_the_replaced_merge_request_and_the_releases() {
        let mut c = content();
        c.retry_of = None;
        c.replaces = Some(Replaces {
            merge_request: "#5 (https://github.com/team/app/pull/5)".into(),
            releases: vec!["`app-a` v1.3.0".into()],
        });
        let text = render(&c, 65_536);
        assert!(
            text.contains(
                "This pull request replaces the pull request #5 (https://github.com/team/app/pull/5)"
            ),
            "{text}"
        );
        assert!(text.contains("when `app-a` v1.3.0 came out"), "{text}");
        assert_eq!(targets(&text), Some(c.targets));
    }

    #[test]
    fn a_replaced_merge_request_records_no_targets() {
        let text = replaced(
            "merge request",
            &["`app-a` v1.4.0".into(), "`app.b` v2.0.0".into()],
        );
        assert!(text.contains("closed it without merging"), "{text}");
        assert!(text.contains("Nothing of it was merged"), "{text}");
        assert!(
            text.contains("`app-a` v1.4.0, `app.b` v2.0.0 came out"),
            "{text}"
        );
        assert!(
            text.contains("at the newer releases in a new merge request"),
            "{text}"
        );
        assert!(!text.contains(BLOCK_START), "{text}");
        assert_eq!(targets(&text), None);
        assert!(is_replaced(&text));
        assert!(!is_replaced(&render(&content(), 65_536)));
    }

    #[test]
    fn titles_name_the_upstreams() {
        assert_eq!(title(&["a".into()]), "Merge upstream update: a");
        assert_eq!(
            title(&["a".into(), "b".into()]),
            "Merge upstream updates: a, b"
        );
    }
}
