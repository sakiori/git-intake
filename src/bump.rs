//! The change a commit makes to a semver release, by semantic-release's
//! default rules for Conventional Commits.

use git_conventional::{Commit, Type};
use semver::{Comparator, Op, Version};
use serde::Serialize;

/// A change of a semantic version, smallest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Bump {
    Patch,
    Minor,
    Major,
}

impl Bump {
    /// The change of a commit message: a breaking change is major, `feat`
    /// minor, `fix`, `perf` and `revert` patch, other types none, and a
    /// subject that is not a Conventional Commit patch.
    pub fn of_message(message: &str) -> Option<Self> {
        let Some(commit) = parse(message) else {
            return Some(Self::Patch);
        };
        if commit.breaking() {
            return Some(Self::Major);
        }
        let ty = commit.type_();
        if ty == Type::FEAT {
            Some(Self::Minor)
        } else if [Type::FIX, Type::PERF, Type::REVERT].contains(&ty) {
            Some(Self::Patch)
        } else {
            None
        }
    }

    /// `version` raised by this change.
    pub fn apply(self, version: &Version) -> Version {
        let v = version;
        match self {
            Self::Major => Version::new(v.major + 1, 0, 0),
            Self::Minor => Version::new(v.major, v.minor + 1, 0),
            Self::Patch => Version::new(v.major, v.minor, v.patch + 1),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Patch => "patch",
            Self::Minor => "minor",
            Self::Major => "major",
        }
    }
}

/// Whether `next` satisfies the caret range `^previous`.
pub fn compatible(previous: &Version, next: &Version) -> bool {
    let caret = Comparator {
        op: Op::Caret,
        major: previous.major,
        minor: Some(previous.minor),
        patch: Some(previous.patch),
        pre: previous.pre.clone(),
    };
    caret.matches(next)
}

/// A breaking change of a commit message, with the text of its
/// `BREAKING CHANGE` footer when it has one.
pub fn breaking(message: &str) -> Option<Option<String>> {
    let commit = parse(message)?;
    if !commit.breaking() {
        return None;
    }
    Some(
        commit
            .footers()
            .iter()
            .find(|f| f.breaking())
            .map(|f| f.value().trim().to_string()),
    )
}

/// The message as a Conventional Commit, or else its subject alone, so that
/// a body the parser refuses does not hide the subject's type.
fn parse(message: &str) -> Option<Commit<'_>> {
    let message = message.trim();
    Commit::parse(message).ok().or_else(|| {
        let subject = message.lines().next().unwrap_or_default();
        Commit::parse(subject).ok()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_take_semantic_release_s_default_rules() {
        for (message, bump) in [
            ("feat: a", Some(Bump::Minor)),
            ("Feat(scope): a", Some(Bump::Minor)),
            ("fix: a", Some(Bump::Patch)),
            ("perf: a", Some(Bump::Patch)),
            ("revert: a", Some(Bump::Patch)),
            ("feat!: a", Some(Bump::Major)),
            ("docs(x)!: a", Some(Bump::Major)),
            ("refactor: a\n\nBREAKING CHANGE: b", Some(Bump::Major)),
            ("chore: a\n\nBREAKING-CHANGE: b", Some(Bump::Major)),
            ("docs: a", None),
            ("ci: a\n\nSigned-off-by: A <a@example.com>\n", None),
            ("chore(deps): a", None),
            (
                "Revert \"feat: a\"\n\nThis reverts commit 0123.",
                Some(Bump::Patch),
            ),
            ("Add the widget", Some(Bump::Patch)),
            ("fixup! fix: a", Some(Bump::Patch)),
        ] {
            assert_eq!(Bump::of_message(message), bump, "{message}");
        }
    }

    #[test]
    fn changes_raise_versions() {
        let v = Version::new(1, 2, 3);
        assert_eq!(Bump::Major.apply(&v), Version::new(2, 0, 0));
        assert_eq!(Bump::Minor.apply(&v), Version::new(1, 3, 0));
        assert_eq!(Bump::Patch.apply(&v), Version::new(1, 2, 4));
        assert!(Bump::Major > Bump::Minor && Bump::Minor > Bump::Patch);
    }

    #[test]
    fn upstream_releases_are_compatible_by_the_caret_rule() {
        let v = |s: &str| Version::parse(s).unwrap();
        assert!(compatible(&v("1.2.0"), &v("1.9.4")));
        assert!(!compatible(&v("1.2.0"), &v("2.0.0")));
        assert!(!compatible(&v("1.2.0"), &v("1.1.0")));
        assert!(compatible(&v("0.11.0"), &v("0.11.3")));
        assert!(!compatible(&v("0.11.0"), &v("0.12.0")));
        assert!(compatible(&v("0.0.3"), &v("0.0.3")));
        assert!(!compatible(&v("0.0.3"), &v("0.0.4")));
    }

    #[test]
    fn breaking_changes_carry_their_footer() {
        assert_eq!(
            breaking("feat!: a\n\nBREAKING CHANGE: clients need\n protocol 3\n"),
            Some(Some("clients need\n protocol 3".to_string()))
        );
        assert_eq!(breaking("feat!: a"), Some(None));
        assert_eq!(breaking("feat: a"), None);
        assert_eq!(breaking("Add the widget"), None);
    }
}
