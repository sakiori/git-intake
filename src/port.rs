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

use crate::config::Upstream;
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
    /// Upstream commits that were taken or decided but are no longer on the
    /// upstream's first-parent line, acknowledged by the owner after the
    /// upstream was rewritten. They count as neither taken nor decided.
    pub retired: Vec<String>,
    pub decisions: Vec<Decision>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawJournal {
    base: String,
    #[serde(default)]
    retired: Vec<String>,
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

/// Parses journal text. All problems are reported at once. Commit ids are
/// read in lower case, as git writes them.
pub fn parse_journal(text: &str) -> std::result::Result<Journal, Vec<String>> {
    let raw: RawJournal = toml::from_str(text).map_err(|e| vec![e.message().to_string()])?;
    let mut problems = Vec::new();
    let base = commit_id(&raw.base);
    if !is_full_id(&base) {
        problems.push(format!("base: '{base}' is not a full commit id"));
    }
    let mut retired: Vec<String> = Vec::new();
    for (i, r) in raw.retired.iter().enumerate() {
        let commit = commit_id(r);
        if !is_full_id(&commit) {
            problems.push(format!("retired[{i}]: '{commit}' is not a full commit id"));
        } else if retired.contains(&commit) {
            problems.push(format!(
                "retired[{i}]: commit {commit} is retired more than once"
            ));
        } else {
            retired.push(commit);
        }
    }
    let mut decisions: Vec<Decision> = Vec::new();
    for (i, d) in raw.decisions.into_iter().enumerate() {
        let commit = commit_id(&d.commit);
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
        Ok(Journal {
            base,
            retired,
            decisions,
        })
    } else {
        Err(problems)
    }
}

/// A commit id as the journal and the trailers compare it: trimmed and in
/// lower case.
pub fn commit_id(s: &str) -> String {
    s.trim().to_ascii_lowercase()
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
    if !journal.retired.is_empty() {
        out.push_str(
            "# Taken or decided commits the upstream no longer has, acknowledged after a rewrite.\n\
             retired = [\n",
        );
        for r in &journal.retired {
            out.push_str(&format!("    {},\n", quote(r)));
        }
        out.push_str("]\n");
    }
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
    /// The base, or a taken or decided commit that is not retired, is no
    /// longer on the first-parent line of the fetched branch.
    Rewritten(Rewrite),
    /// No journal, and no last synced commit to start one from.
    NoBase,
}

/// What a rewrite of a port upstream left off its first-parent line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rewrite {
    /// The journal's base, or the last synced commit a new journal would
    /// start from, when it is no longer on the line.
    pub base: Option<String>,
    /// Whether that base comes from the journal.
    pub journal: bool,
    /// Taken or decided commits off the line that the journal does not
    /// retire.
    pub commits: Vec<OffLine>,
}

/// A taken or decided commit that is no longer on the upstream's line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OffLine {
    pub commit: String,
    /// `taken`, `skip` or `have`.
    pub recorded_as: &'static str,
}

impl Rewrite {
    /// What was rewritten and the way out, naming every commit.
    pub fn describe(&self, name: &str) -> String {
        let mut parts = Vec::new();
        if let Some(base) = &self.base {
            parts.push(format!(
                "{} {} is no longer on the first-parent line of the upstream branch",
                if self.journal {
                    "the journal's base"
                } else {
                    "the last synced commit"
                },
                abbrev(base)
            ));
        }
        if !self.commits.is_empty() {
            let listed: Vec<String> = self
                .commits
                .iter()
                .map(|c| format!("{} ({})", c.commit, c.recorded_as))
                .collect();
            parts.push(format!(
                "taken or decided commits are no longer on it: {}",
                listed.join(", ")
            ));
        }
        let journal = journal_path(name);
        let mut steps = Vec::new();
        match (&self.base, self.journal) {
            (Some(_), true) => steps.push(format!(
                "move the base in the journal {journal} to a commit of the upstream's first-parent line"
            )),
            (Some(_), false) => steps.push(format!(
                "write the journal {journal} by hand, with a commit of the upstream's first-parent line as its `base`"
            )),
            (None, true) => {}
            (None, false) => steps.push(format!(
                "write the journal {journal} with the last synced commit as its `base`"
            )),
        }
        if !self.commits.is_empty() {
            steps.push(format!(
                "list the commits under `retired` in the journal {journal}; their rewritten versions then show as \
                 pending and can be marked `have`"
            ));
        }
        let way_out = steps.join(", and ");
        format!("{}; {way_out}", parts.join("; "))
    }
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
/// integration branch's copies name as taken. `upstream_tips` are the
/// fetched tips of the configured upstreams, whose own history holds no
/// copies of this repository. An invalid journal is an error.
pub fn standing(
    git: &Git,
    integration_tip: &str,
    up: &Upstream,
    last: Option<&Record>,
    tip: &str,
    upstream_tips: &[String],
) -> Result<Standing> {
    let journal = read_journal(git, integration_tip, &up.name)?;
    let last_synced = match (&journal, last) {
        (Some(_), _) => None,
        (None, Some(record)) => Some(record.target.clone()),
        (None, None) => drift::adopted_base(git, integration_tip, up, tip)?,
    };
    let taken = sync_record::taken(git, integration_tip, upstream_tips, &up.name)?;
    account(git, tip, journal.as_ref(), last_synced.as_deref(), &taken)
}

/// Accounts for the commits of a port upstream's fetched branch at `tip`
/// after its base. `journal` is the journal on the integration branch;
/// without one, `last_synced` is the base a new journal would start from.
/// `taken` are the upstream commits the integration branch's copies name.
/// Commits the journal retires count as neither taken nor decided.
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
    let retired: HashSet<&str> = journal
        .map(|j| j.retired.iter().map(String::as_str).collect())
        .unwrap_or_default();
    let decisions = journal.map(|j| j.decisions.as_slice()).unwrap_or_default();
    // The commits to place on the line, as recorded.
    let mut recorded: Vec<(String, &'static str, Option<Verdict>)> = Vec::new();
    for d in decisions {
        if !retired.contains(d.commit.as_str()) {
            recorded.push((d.commit.clone(), d.verdict.as_str(), Some(d.verdict)));
        }
    }
    for named in taken {
        let commit = match commit_id(named) {
            c if is_full_id(&c) => c,
            c => git.commit(&c)?.unwrap_or(c),
        };
        if !retired.contains(commit.as_str()) && !recorded.iter().any(|r| r.0 == commit) {
            recorded.push((commit, "taken", None));
        }
    }

    let Some(line) = line_after(git, &base, tip)? else {
        // Without the base, the commits are judged against the whole line.
        let whole: HashSet<String> =
            crate::git::lines(&git.run(&["rev-list", "--first-parent", tip, "--"])?)
                .into_iter()
                .collect();
        return Ok(Standing::Rewritten(Rewrite {
            base: Some(base),
            journal: from_journal,
            commits: recorded
                .iter()
                .filter(|(c, _, _)| !whole.contains(c))
                .map(|(c, as_, _)| OffLine {
                    commit: c.clone(),
                    recorded_as: as_,
                })
                .collect(),
        }));
    };
    let on_line: HashSet<&str> = line.iter().map(|c| c.commit.as_str()).collect();
    let mut decided = HashMap::new();
    let mut taken_on_line = HashSet::new();
    let mut off_line = Vec::new();
    for (commit, as_, verdict) in recorded {
        if on_line.contains(commit.as_str()) {
            match verdict {
                Some(v) => {
                    decided.insert(commit, v);
                }
                None => {
                    taken_on_line.insert(commit);
                }
            }
        } else if !(git.object_exists(&commit)? && git.is_ancestor(&commit, &base)?) {
            // A commit at or before the base is on the line too; it only
            // does not count, since the port starts after the base.
            off_line.push(OffLine {
                commit,
                recorded_as: as_,
            });
        }
    }
    if !off_line.is_empty() {
        return Ok(Standing::Rewritten(Rewrite {
            base: None,
            journal: from_journal,
            commits: off_line,
        }));
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

/// Why a port upstream has no base, and the way out.
pub fn no_base_advice(name: &str) -> String {
    format!(
        "the upstream is in port mode but has no journal, and the integration branch has no sync record of it and \
         shares no history with it; write the journal {} by hand, with the upstream commit the port starts from \
         as its `base`",
        journal_path(name)
    )
}

/// Refuses an upstream whose standing cannot be planned.
pub fn require_plannable(name: &str, standing: Standing) -> Result<Accounting> {
    match standing {
        Standing::Known(a) => Ok(a),
        Standing::Rewritten(rewrite) => bail!(
            "upstream '{name}' was rewritten: {}",
            rewrite.describe(name)
        ),
        Standing::NoBase => bail!("upstream '{name}': {}", no_base_advice(name)),
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
            retired: vec![C.into()],
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
    fn commit_ids_are_read_in_lower_case() {
        let text = format!(
            "base = \"{}\"\nretired = [\"{}\"]\n[[decision]]\ncommit = \" {} \"\nverdict = \"skip\"\n",
            A.to_uppercase(),
            B.to_uppercase(),
            C.to_uppercase()
        );
        let journal = parse_journal(&text).unwrap();
        assert_eq!(journal.base, A);
        assert_eq!(journal.retired, vec![B.to_string()]);
        assert_eq!(journal.decisions[0].commit, C);
        let lines = parse_plan("take AAAAAAAA\n", &commits()).unwrap();
        assert_eq!(lines[0].index, 0);
    }

    #[test]
    fn invalid_retired_commits_are_refused() {
        let text = format!("base = \"{A}\"\nretired = [\"{B}\", \"{B}\", \"abc\"]\n");
        let problems = parse_journal(&text).unwrap_err().join("\n");
        assert!(problems.contains("is retired more than once"), "{problems}");
        assert!(
            problems.contains("retired[2]: 'abc' is not a full commit id"),
            "{problems}"
        );
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
            plan.starts_with(
                "# git intake plan for example-app: 3 pending commits, oldest first.\n"
            )
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
