//! Upstreams in `port` mode: the journal of decisions, the pending commits,
//! and the plan an update carries out.
//!
//! A taken commit is recorded by the copy that took it, in its trailers. A
//! skipped commit, or one the product already has, is recorded in the
//! journal `.git-intake/<name>.toml` on the integration branch, with the
//! upstream commit the port started from as its base. The pending commits
//! are computed each time: the first-parent commits of the fetched upstream
//! branch after the base, minus those taken and those decided.

use std::collections::{HashMap, HashSet};

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

use crate::config::{Config, Upstream};
use crate::drift;
use crate::git::{Git, abbrev};
use crate::sync_record::{self, Record};

/// The directory of the journals at the repository root.
pub const JOURNAL_DIR: &str = ".git-intake";

/// The journal of upstream `name`, relative to the repository root.
pub fn journal_path(name: &str) -> String {
    format!("{JOURNAL_DIR}/{name}.toml")
}

/// What a decision says about an upstream commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// The product does not take it.
    Skip,
    /// The product already has it.
    Have,
}

impl Verdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Verdict::Skip => "skip",
            Verdict::Have => "have",
        }
    }
}

/// One decision of the journal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Decision {
    pub commit: String,
    pub verdict: Verdict,
    pub note: Option<String>,
}

/// A port upstream's journal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Journal {
    /// The upstream commit the port started from; it never moves.
    pub base: String,
    pub decisions: Vec<Decision>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawJournal {
    base: String,
    #[serde(default, rename = "decision")]
    decisions: Vec<RawDecision>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawDecision {
    commit: String,
    verdict: String,
    note: Option<String>,
}

/// Reads and validates the journal of upstream `name` at `rev`, or `None`
/// when there is none.
pub fn read_journal(git: &Git, rev: &str, name: &str) -> Result<Option<Journal>> {
    let path = journal_path(name);
    match git.show_file(rev, &path)? {
        None => Ok(None),
        Some(text) => parse_journal(&text).map(Some).map_err(|problems| {
            anyhow::anyhow!(
                "invalid journal {path} of upstream '{name}':\n{}",
                problems
                    .iter()
                    .map(|p| format!("  - {p}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            )
        }),
    }
}

/// Parses journal text. All problems are reported at once.
pub fn parse_journal(text: &str) -> std::result::Result<Journal, Vec<String>> {
    let raw: RawJournal = toml::from_str(text).map_err(|e| vec![e.message().to_string()])?;
    let mut problems = Vec::new();
    let base = raw.base.trim().to_string();
    if !is_full_id(&base) {
        problems.push(format!("base: '{base}' is not a full commit id"));
    }
    let mut decisions: Vec<Decision> = Vec::new();
    for (i, d) in raw.decisions.into_iter().enumerate() {
        let commit = d.commit.trim().to_string();
        if !is_full_id(&commit) {
            problems.push(format!("decision[{i}]: '{commit}' is not a full commit id"));
        }
        let verdict = match d.verdict.as_str() {
            "skip" => Verdict::Skip,
            "have" => Verdict::Have,
            other => {
                problems.push(format!(
                    "decision[{i}]: the verdict '{other}' is neither `skip` nor `have`"
                ));
                continue;
            }
        };
        if decisions.iter().any(|seen| seen.commit == commit) {
            problems.push(format!(
                "decision[{i}]: commit {commit} is decided more than once"
            ));
            continue;
        }
        let note = d
            .note
            .map(|n| n.trim().to_string())
            .filter(|n| !n.is_empty());
        decisions.push(Decision {
            commit,
            verdict,
            note,
        });
    }
    if problems.is_empty() {
        Ok(Journal { base, decisions })
    } else {
        Err(problems)
    }
}

/// The journal in the form the tool writes it.
pub fn render_journal(journal: &Journal) -> String {
    let quote = |s: &str| toml::Value::String(s.to_string()).to_string();
    let mut out = format!(
        "# Decisions on the upstream's commits, kept by git intake. A commit taken into the\n\
         # product is recorded by its copy; a commit decided here is skipped (`skip`) or\n\
         # already present in the product (`have`).\n\
         base = {}\n",
        quote(&journal.base)
    );
    for d in &journal.decisions {
        out.push_str(&format!(
            "\n[[decision]]\ncommit = {}\nverdict = {}\n",
            quote(&d.commit),
            quote(d.verdict.as_str())
        ));
        if let Some(note) = &d.note {
            out.push_str(&format!("note = {}\n", quote(note)));
        }
    }
    out
}

/// A full hexadecimal commit id, SHA-1 or SHA-256.
pub fn is_full_id(s: &str) -> bool {
    (s.len() == 40 || s.len() == 64) && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// One commit of the first-parent line of the upstream branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineCommit {
    pub commit: String,
    pub subject: String,
    /// Whether it is a merge, copied as its change against its first parent.
    pub merge: bool,
}

/// Where a port upstream stands.
#[derive(Debug, Clone)]
pub enum Standing {
    /// Every commit after the base is accounted for, or some are pending.
    Known(Accounting),
    /// The base, a decided commit or a taken commit is no longer on the
    /// first-parent line of the fetched branch.
    Rewritten(String),
    /// No journal, and no last synced commit to start one from.
    NoBase,
}

/// The commits after a port upstream's base, accounted for.
#[derive(Debug, Clone)]
pub struct Accounting {
    pub base: String,
    /// Whether the base comes from the journal, or from the last synced
    /// commit because the journal does not exist yet.
    pub journal: bool,
    /// The first-parent commits after the base, oldest first.
    pub line: Vec<LineCommit>,
    /// Commits of the line that copies name as taken.
    pub taken: HashSet<String>,
    /// Commits of the line the journal decides.
    pub decided: HashMap<String, Verdict>,
}

impl Accounting {
    /// The pending commits, oldest first.
    pub fn pending(&self) -> Vec<&LineCommit> {
        self.line
            .iter()
            .filter(|c| !self.taken.contains(&c.commit) && !self.decided.contains_key(&c.commit))
            .collect()
    }
}

/// Where a port upstream stands against the integration branch at
/// `integration_tip`: its journal there, or without one its last synced
/// commit from the sync record `last` or the history, and the commits the
/// integration branch's copies name as taken. An invalid journal is an
/// error.
pub fn standing(
    git: &Git,
    config: &Config,
    integration_tip: &str,
    up: &Upstream,
    last: Option<&Record>,
    tip: &str,
) -> Result<Standing> {
    let journal = read_journal(git, integration_tip, &up.name)?;
    let last_synced = match (&journal, last) {
        (Some(_), _) => None,
        (None, Some(record)) => Some(record.target.clone()),
        (None, None) => drift::adopted_base(git, integration_tip, up, tip)?,
    };
    let taken = sync_record::taken(git, integration_tip, config, &up.name)?;
    account(git, tip, journal.as_ref(), last_synced.as_deref(), &taken)
}

/// Accounts for the commits of a port upstream's fetched branch at `tip`
/// after its base. `journal` is the journal on the integration branch;
/// without one, `last_synced` is the base a new journal would start from.
/// `taken` are the upstream commits the integration branch's copies name.
pub fn account(
    git: &Git,
    tip: &str,
    journal: Option<&Journal>,
    last_synced: Option<&str>,
    taken: &[String],
) -> Result<Standing> {
    let (base, from_journal) = match (journal, last_synced) {
        (Some(j), _) => (j.base.clone(), true),
        (None, Some(last)) => (last.to_string(), false),
        (None, None) => return Ok(Standing::NoBase),
    };
    let what = if from_journal {
        "the journal's base"
    } else {
        "the last synced commit"
    };
    let Some(line) = line_after(git, &base, tip)? else {
        return Ok(Standing::Rewritten(format!(
            "{what} {} is no longer on the first-parent line of the upstream branch",
            abbrev(&base)
        )));
    };
    let on_line: HashSet<&str> = line.iter().map(|c| c.commit.as_str()).collect();
    // A commit at or before the base is on the line too; it only does not
    // count, since the port starts after the base.
    let before_base =
        |c: &str| -> Result<bool> { Ok(git.object_exists(c)? && git.is_ancestor(c, &base)?) };

    let mut decided = HashMap::new();
    for d in journal.map(|j| j.decisions.as_slice()).unwrap_or_default() {
        if on_line.contains(d.commit.as_str()) {
            decided.insert(d.commit.clone(), d.verdict);
        } else if !before_base(&d.commit)? {
            return Ok(Standing::Rewritten(format!(
                "the journal decides commit {}, which is no longer on the first-parent line of the upstream branch",
                abbrev(&d.commit)
            )));
        }
    }
    let mut taken_on_line = HashSet::new();
    for named in taken {
        let commit = if is_full_id(named) {
            Some(named.clone())
        } else {
            git.commit(named)?
        };
        match commit {
            Some(c) if on_line.contains(c.as_str()) => {
                taken_on_line.insert(c);
            }
            Some(c) if before_base(&c)? => {}
            _ => {
                return Ok(Standing::Rewritten(format!(
                    "a copy on the integration branch names commit {} as taken, which is no longer on the \
                     first-parent line of the upstream branch",
                    abbrev(named)
                )));
            }
        }
    }
    Ok(Standing::Known(Accounting {
        base,
        journal: from_journal,
        line,
        taken: taken_on_line,
        decided,
    }))
}

/// The first-parent commits of `tip` after `base`, oldest first, or `None`
/// when `base` is not on the first-parent line of `tip`.
fn line_after(git: &Git, base: &str, tip: &str) -> Result<Option<Vec<LineCommit>>> {
    if !git.object_exists(base)? || !git.is_ancestor(base, tip)? {
        return Ok(None);
    }
    let out = git.run(&[
        "log",
        "--first-parent",
        "--reverse",
        "--format=%H%x1f%P%x1f%s%x1e",
        &format!("{base}..{tip}"),
        "--",
    ])?;
    let mut line = Vec::new();
    let mut first_parent_of_oldest = None;
    for entry in out.split('\u{1e}') {
        let mut fields = entry.trim_matches(['\n', '\r']).split('\u{1f}');
        let (Some(commit), Some(parents), Some(subject)) =
            (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        if commit.is_empty() {
            continue;
        }
        let parents: Vec<&str> = parents.split_whitespace().collect();
        if line.is_empty() {
            first_parent_of_oldest = parents.first().map(|p| p.to_string());
        }
        line.push(LineCommit {
            commit: commit.to_string(),
            subject: subject.to_string(),
            merge: parents.len() > 1,
        });
    }
    // The walk stops at the first commit `base` contains: it reaches `base`
    // itself only when `base` is on the line.
    let on_line = match first_parent_of_oldest {
        None => line.is_empty() && git.line(&["rev-parse", tip])? == base,
        Some(p) => p == base,
    };
    Ok(on_line.then_some(line))
}

/// What a plan line says to do with a commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verb {
    Take,
    Skip,
    Have,
    Later,
}

impl Verb {
    pub fn as_str(self) -> &'static str {
        match self {
            Verb::Take => "take",
            Verb::Skip => "skip",
            Verb::Have => "have",
            Verb::Later => "later",
        }
    }

    fn parse(word: &str) -> Option<Self> {
        match word {
            "take" => Some(Verb::Take),
            "skip" => Some(Verb::Skip),
            "have" => Some(Verb::Have),
            "later" => Some(Verb::Later),
            _ => None,
        }
    }
}

/// The plan `start` writes: every pending commit, oldest first, with the
/// verb `later`.
pub fn render_plan(name: &str, commits: &[(String, String)]) -> String {
    let n = commits.len();
    let mut out = format!(
        "# git intake plan for {name}: {n} pending commit{}, oldest first.\n\
         # take  copy the commit onto the update branch\n\
         # skip  record that the product does not take it\n\
         # have  record that the product already has it\n\
         # later leave it pending\n\
         # Takes are copied in the order listed here. Text after the subject and ' # ' is the\n\
         # note of a skip or have decision. A commit left out of the plan stays pending.\n",
        if n == 1 { "" } else { "s" }
    );
    for (commit, subject) in commits {
        out.push_str(&format!("later {} {subject}\n", abbrev(commit)));
    }
    out
}

/// One decided line of a plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanLine {
    /// Index of the commit in the planned list.
    pub index: usize,
    pub verb: Verb,
    pub note: Option<String>,
}

/// Reads an edited plan against the planned commits (`(commit, subject)`,
/// oldest first). Returns its lines in plan order, or why the plan is
/// refused, naming the line.
pub fn parse_plan(
    text: &str,
    commits: &[(String, String)],
) -> std::result::Result<Vec<PlanLine>, String> {
    let mut lines: Vec<PlanLine> = Vec::new();
    for (no, raw) in text.lines().enumerate() {
        let no = no + 1;
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let refuse = |why: String| format!("plan line {no} ({line}): {why}");
        let (word, rest) = split_word(line);
        let Some(verb) = Verb::parse(word) else {
            return Err(refuse(format!(
                "unknown verb '{word}'; use take, skip, have or later"
            )));
        };
        let (id, rest) = split_word(rest);
        if id.is_empty() {
            return Err(refuse("the line names no commit".to_string()));
        }
        let matches: Vec<usize> = if id.len() >= 4 && id.bytes().all(|b| b.is_ascii_hexdigit()) {
            let id = id.to_ascii_lowercase();
            commits
                .iter()
                .enumerate()
                .filter(|(_, (c, _))| c.starts_with(&id))
                .map(|(i, _)| i)
                .collect()
        } else {
            Vec::new()
        };
        let index = match matches.as_slice() {
            [one] => *one,
            [] => {
                return Err(refuse(format!(
                    "'{id}' is not one of the pending commits this plan lists"
                )));
            }
            _ => {
                return Err(refuse(format!(
                    "'{id}' names more than one pending commit; give more of its id"
                )));
            }
        };
        if lines.iter().any(|l| l.index == index) {
            return Err(refuse(format!(
                "commit {} is named more than once",
                abbrev(&commits[index].0)
            )));
        }
        let note = match verb {
            Verb::Skip | Verb::Have => note_of(rest, &commits[index].1),
            Verb::Take | Verb::Later => None,
        };
        lines.push(PlanLine { index, verb, note });
    }
    if lines.iter().all(|l| l.verb == Verb::Later) {
        return Err(
            "the plan decides nothing: mark at least one commit take, skip or have".to_string(),
        );
    }
    Ok(lines)
}

fn split_word(s: &str) -> (&str, &str) {
    let s = s.trim_start();
    match s.find(char::is_whitespace) {
        Some(i) => (&s[..i], &s[i..]),
        None => (s, ""),
    }
}

/// The note on a plan line: the text after the subject and a ` # `.
fn note_of(rest: &str, subject: &str) -> Option<String> {
    let rest = rest.trim_start();
    let after = rest.strip_prefix(subject).unwrap_or(rest);
    let note = if let Some(note) = after.trim_start().strip_prefix("# ") {
        note
    } else {
        after.split_once(" # ")?.1
    };
    let note = note.trim();
    (!note.is_empty()).then(|| note.to_string())
}

/// Refuses an upstream whose standing cannot be planned.
pub fn require_plannable(name: &str, standing: Standing) -> Result<Accounting> {
    match standing {
        Standing::Known(a) => Ok(a),
        Standing::Rewritten(why) => bail!("upstream '{name}' was rewritten: {why}"),
        Standing::NoBase => bail!(
            "upstream '{name}' is in port mode but has no journal, and the integration branch has no sync record \
             of it and shares no history with it; write the journal {} by hand, with the upstream commit the port \
             starts from as its `base`",
            journal_path(name)
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const C: &str = "cccccccccccccccccccccccccccccccccccccccc";

    fn commits() -> Vec<(String, String)> {
        vec![
            (A.into(), "fix: keep # in names".into()),
            (B.into(), "feat: add a page".into()),
            (C.into(), "chore: update".into()),
        ]
    }

    #[test]
    fn journal_round_trips() {
        let journal = Journal {
            base: A.into(),
            decisions: vec![
                Decision {
                    commit: B.into(),
                    verdict: Verdict::Skip,
                    note: Some("not \"ours\"".into()),
                },
                Decision {
                    commit: C.into(),
                    verdict: Verdict::Have,
                    note: None,
                },
            ],
        };
        assert_eq!(parse_journal(&render_journal(&journal)).unwrap(), journal);
    }

    #[test]
    fn invalid_journals_name_the_problem() {
        let text = format!(
            "base = \"{A}\"\n[[decision]]\ncommit = \"{B}\"\nverdict = \"drop\"\n\
             [[decision]]\ncommit = \"{C}\"\nverdict = \"skip\"\n[[decision]]\ncommit = \"{C}\"\nverdict = \"have\"\n"
        );
        let problems = parse_journal(&text).unwrap_err().join("\n");
        assert!(
            problems.contains("the verdict 'drop' is neither"),
            "{problems}"
        );
        assert!(problems.contains("is decided more than once"), "{problems}");
        let problems = parse_journal("base = \"abc\"\n").unwrap_err().join("\n");
        assert!(
            problems.contains("'abc' is not a full commit id"),
            "{problems}"
        );
        assert!(parse_journal("bases = 1\n").is_err());
    }

    #[test]
    fn plan_lists_every_commit_as_later() {
        let plan = render_plan("example-app", &commits());
        assert!(
            plan.starts_with("# git intake plan for example-app: 3 pending commits, oldest first.\n")
        );
        assert!(plan.contains("later aaaaaaaaaaaa fix: keep # in names\n"));
        assert!(plan.ends_with("later cccccccccccc chore: update\n"));
    }

    #[test]
    fn plan_lines_are_read_in_order_with_notes() {
        let text = "# comment\n\ntake cccc chore: update\nskip aaaaaaa fix: keep # in names # not ours\n\
                    have bbbbbbbbbbbb feat: add a page\n";
        let lines = parse_plan(text, &commits()).unwrap();
        assert_eq!(
            lines,
            vec![
                PlanLine {
                    index: 2,
                    verb: Verb::Take,
                    note: None
                },
                PlanLine {
                    index: 0,
                    verb: Verb::Skip,
                    note: Some("not ours".into())
                },
                PlanLine {
                    index: 1,
                    verb: Verb::Have,
                    note: None
                },
            ]
        );
        let edited = parse_plan("skip bbbb # upstream only\n", &commits()).unwrap();
        assert_eq!(edited[0].note.as_deref(), Some("upstream only"));
    }

    #[test]
    fn invalid_plans_name_the_line() {
        let e = parse_plan("take aaaa x\ndrop bbbb y\n", &commits()).unwrap_err();
        assert!(
            e.starts_with("plan line 2 (drop bbbb y): unknown verb 'drop'"),
            "{e}"
        );
        let e = parse_plan("take dddd x\n", &commits()).unwrap_err();
        assert!(
            e.contains("plan line 1 (take dddd x): 'dddd' is not one of the pending commits"),
            "{e}"
        );
        let e = parse_plan("take aaaa\nskip aaaaaa\n", &commits()).unwrap_err();
        assert!(
            e.contains("plan line 2 (skip aaaaaa): commit aaaaaaaaaaaa is named more than once"),
            "{e}"
        );
        let e = parse_plan("later aaaa\nlater bbbb\n", &commits()).unwrap_err();
        assert!(e.contains("the plan decides nothing"), "{e}");
        let e = parse_plan("# nothing\n", &commits()).unwrap_err();
        assert!(e.contains("the plan decides nothing"), "{e}");
        let e = parse_plan("take\n", &commits()).unwrap_err();
        assert!(e.contains("the line names no commit"), "{e}");
    }
}
