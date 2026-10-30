//! `report`: what an update took in from each upstream.

use anyhow::{Result, bail};
use serde::Serialize;

use crate::Exit;
use crate::commands::update::plural;
use crate::config::Mode;
use crate::git::{Git, abbrev};
use crate::output::{Render, emit};
use crate::port::{Verb, Verdict};
use crate::repo::Repo;
use crate::state::{
    self, ConflictFile, CopyStep, FollowMerge, MergeKind, PortPlan, Resolution, UpdateState,
    UpstreamStep, Work,
};

#[derive(Serialize)]
pub struct ReportDoc {
    pub in_progress: bool,
    pub integration: String,
    pub base: String,
    /// Branches `finish --no-publish` advanced locally without publishing.
    pub unpublished: Vec<String>,
    pub upstreams: Vec<UpstreamReport>,
}

#[derive(Serialize)]
pub struct UpstreamReport {
    pub name: String,
    pub path: Option<String>,
    pub source: String,
    pub mode: Mode,
    pub step: UpstreamStep,
    /// The revision `start` was given: the target of a follow upstream, the
    /// limit of a port plan; `null` for the fetched tip.
    pub rev: Option<String>,
    /// Follow mode: how the upstream is merged.
    pub kind: Option<MergeKind>,
    /// Follow mode: start of the merged range; `null` for a first import
    /// (full history).
    pub range_start: Option<String>,
    /// Follow mode: the target.
    pub target: Option<String>,
    /// Follow mode: upstream commits merged.
    pub commits: Option<u64>,
    pub merge_commit: Option<String>,
    /// Follow mode: the merge's conflicted files.
    pub conflicts: Vec<ConflictFile>,
    /// Follow mode: the repository's own changes relative to the upstream
    /// before the merge.
    pub own_changes_before: Option<DiffStat>,
    /// Follow mode: the repository's own changes relative to the upstream
    /// after the merge.
    pub own_changes_after: Option<DiffStat>,
    /// Port mode: what the plan did.
    pub port: Option<PortReport>,
}

#[derive(Serialize)]
pub struct PortReport {
    /// The taken commits in the order they were copied, or are to be.
    pub taken: Vec<TakenReport>,
    /// Commits marked `take` whose copy changed nothing, recorded as `have`.
    pub unchanged: Vec<UnchangedReport>,
    /// The `skip` and `have` decisions of the plan.
    pub decisions: Vec<DecisionReport>,
    /// Commits left pending: those marked `later`, and those a revision
    /// named to `start` left out of the plan.
    pub left_pending: u64,
    /// The commit that recorded the decisions in the journal.
    pub journal_commit: Option<String>,
}

#[derive(Serialize)]
pub struct TakenReport {
    pub commit: String,
    pub subject: String,
    /// `none` while not copied yet, `conflict` or `copied`.
    pub copy: CopyStep,
    pub copied: Option<String>,
    pub conflicts: Vec<ConflictFile>,
    /// Paths the copy left out: `.git-intake.toml` and `.git-intake/`.
    pub left_out: Vec<String>,
    /// Earlier upstream commits still pending that changed some of the same
    /// files.
    pub earlier_pending: Vec<CommitRef>,
}

#[derive(Serialize)]
pub struct UnchangedReport {
    pub commit: String,
    pub subject: String,
    /// Paths the copy left out: `.git-intake.toml` and `.git-intake/`.
    pub left_out: Vec<String>,
}

#[derive(Serialize)]
pub struct CommitRef {
    pub commit: String,
    pub subject: String,
}

#[derive(Serialize)]
pub struct DecisionReport {
    pub commit: String,
    pub subject: String,
    pub verdict: Verdict,
    pub note: Option<String>,
}

#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct DiffStat {
    pub files: u64,
    pub insertions: u64,
    pub deletions: u64,
}

pub fn run(repo: &Repo, json: bool) -> Result<Exit> {
    let (state, in_progress) = match repo.state()? {
        Some(s) => (s, true),
        None => match state::load_finished(&repo.paths.last_update)? {
            Some(s) => (s, false),
            None => bail!("no update is in progress or finished; nothing to report"),
        },
    };
    let doc = build(&repo.git, &state, in_progress)?;
    emit(&doc, json)?;
    Ok(Exit::Success)
}

fn build(git: &Git, state: &UpdateState, in_progress: bool) -> Result<ReportDoc> {
    let wt = Git::worktree(&state.worktree);
    let stopped = state
        .upstreams
        .iter()
        .any(|q| q.step == UpstreamStep::Conflict);
    let live_unmerged = if in_progress && stopped && state.worktree.exists() {
        Some(wt.unmerged_files()?)
    } else {
        None
    };
    let mut upstreams = Vec::new();
    for q in &state.upstreams {
        let up = state.config.upstream(&q.name);
        let path = up.and_then(|u| u.path.clone());
        let source = up.map(|u| u.source()).unwrap_or_default();
        let mut report = UpstreamReport {
            name: q.name.clone(),
            path: path.clone(),
            source,
            mode: Mode::Follow,
            step: q.step,
            rev: q.rev.clone(),
            kind: None,
            range_start: None,
            target: None,
            commits: None,
            merge_commit: None,
            conflicts: Vec::new(),
            own_changes_before: None,
            own_changes_after: None,
            port: None,
        };
        match &q.work {
            Work::Follow(f) => follow_report(
                git,
                state,
                f,
                q.step,
                path.as_deref(),
                &live_unmerged,
                &mut report,
            )?,
            Work::Port(p) => {
                report.mode = Mode::Port;
                report.port = Some(port_report(p, q.step, &live_unmerged));
            }
        }
        upstreams.push(report);
    }
    Ok(ReportDoc {
        in_progress,
        integration: state.config.integration.clone(),
        base: state.base.clone(),
        unpublished: state
            .finishing
            .as_ref()
            .map(|f| f.unpublished.clone())
            .unwrap_or_default(),
        upstreams,
    })
}

/// Conflicts as they stand: a file recorded unresolved that is no longer
/// unmerged in the stopped merge or copy was resolved by hand.
fn live(
    conflicts: &[ConflictFile],
    stopped: bool,
    live_unmerged: &Option<Vec<String>>,
) -> Vec<ConflictFile> {
    let mut conflicts = conflicts.to_vec();
    if stopped && let Some(unmerged) = live_unmerged {
        for c in &mut conflicts {
            if c.resolution == Resolution::Unresolved && !unmerged.contains(&c.path) {
                c.resolution = Resolution::Manual;
            }
        }
    }
    conflicts
}

fn follow_report(
    git: &Git,
    state: &UpdateState,
    f: &FollowMerge,
    step: UpstreamStep,
    path: Option<&str>,
    live_unmerged: &Option<Vec<String>>,
    report: &mut UpstreamReport,
) -> Result<()> {
    let range_start = match f.kind {
        MergeKind::Import => None,
        _ => match &f.previous {
            Some(p) => Some(p.clone()),
            None => git.merge_base(&state.base, &f.target)?,
        },
    };
    report.own_changes_before = match (&range_start, f.kind) {
        (Some(start), MergeKind::Root) => Some(diff_stat(git, start, &state.base)?),
        (Some(start), MergeKind::Subtree) => Some(diff_stat(
            git,
            start,
            &format!("{}:{}", state.base, path.unwrap_or_default()),
        )?),
        _ => None,
    };
    report.own_changes_after = match (&f.merge_commit, f.kind) {
        (Some(m), MergeKind::Root) => Some(diff_stat(git, &f.target, m)?),
        (Some(m), _) => Some(diff_stat(
            git,
            &f.target,
            &format!("{m}:{}", path.unwrap_or_default()),
        )?),
        (None, _) => None,
    };
    report.kind = Some(f.kind);
    report.range_start = range_start;
    report.target = Some(f.target.clone());
    report.commits = Some(f.commits);
    report.merge_commit = f.merge_commit.clone();
    report.conflicts = live(&f.conflicts, step == UpstreamStep::Conflict, live_unmerged);
    Ok(())
}

fn port_report(
    p: &PortPlan,
    step: UpstreamStep,
    live_unmerged: &Option<Vec<String>>,
) -> PortReport {
    let reference = |i: usize| CommitRef {
        commit: p.commits[i].commit.clone(),
        subject: p.commits[i].subject.clone(),
    };
    let find = |commit: &str| p.commits.iter().position(|c| c.commit == commit);
    let taken = p
        .takes
        .iter()
        .filter(|t| p.commits[**t].copy != CopyStep::Empty)
        .map(|&t| {
            let c = &p.commits[t];
            TakenReport {
                commit: c.commit.clone(),
                subject: c.subject.clone(),
                copy: c.copy,
                copied: c.copied.clone(),
                conflicts: live(
                    &c.conflicts,
                    step == UpstreamStep::Conflict && c.copy == CopyStep::Conflict,
                    live_unmerged,
                ),
                left_out: c.left_out.clone(),
                earlier_pending: c
                    .overlaps
                    .iter()
                    .filter_map(|o| find(o))
                    .map(reference)
                    .collect(),
            }
        })
        .collect();
    let unchanged = p
        .commits
        .iter()
        .filter(|c| c.copy == CopyStep::Empty)
        .map(|c| UnchangedReport {
            commit: c.commit.clone(),
            subject: c.subject.clone(),
            left_out: c.left_out.clone(),
        })
        .collect();
    let decisions = p
        .commits
        .iter()
        .filter_map(|c| {
            let verdict = match c.verb {
                Some(Verb::Skip) => Verdict::Skip,
                Some(Verb::Have) => Verdict::Have,
                _ => return None,
            };
            Some(DecisionReport {
                commit: c.commit.clone(),
                subject: c.subject.clone(),
                verdict,
                note: c.note.clone(),
            })
        })
        .collect();
    let later = p
        .commits
        .iter()
        .filter(|c| matches!(c.verb, None | Some(Verb::Later)))
        .count() as u64;
    PortReport {
        taken,
        unchanged,
        decisions,
        left_pending: later + p.left_out,
        journal_commit: p.journal_commit.clone(),
    }
}

/// Size of the difference between two tree-ish objects.
fn diff_stat(git: &Git, from: &str, to: &str) -> Result<DiffStat> {
    let out = git.run(&["diff", "--numstat", "-z", "--no-renames", from, to])?;
    Ok(parse_numstat(&out))
}

fn parse_numstat(out: &str) -> DiffStat {
    let mut stat = DiffStat::default();
    // With -z and no renames each entry is `<ins>\t<del>\t<path>\0`.
    for line in out.split('\0').filter(|l| !l.trim().is_empty()) {
        let mut parts = line.split('\t');
        let ins = parts
            .next()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0);
        let del = parts
            .next()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0);
        stat.files += 1;
        stat.insertions += ins;
        stat.deletions += del;
    }
    stat
}

fn fmt_stat(s: &DiffStat) -> String {
    format!(
        "{} file{}, +{} -{}",
        s.files,
        plural(s.files),
        s.insertions,
        s.deletions
    )
}

impl Render for ReportDoc {
    fn render(&self) -> String {
        let mut out = format!(
            "update of {} from {} ({})\n",
            self.integration,
            abbrev(&self.base),
            if self.in_progress {
                "in progress"
            } else {
                "finished"
            }
        );
        if !self.unpublished.is_empty() {
            out.push_str(&format!(
                "not published (advanced locally only): {}\n",
                self.unpublished.join(", ")
            ));
        }
        for u in &self.upstreams {
            out.push_str(&format!(
                "\n{} ({}, {}): {}\n",
                u.name,
                u.path.as_deref().unwrap_or("/"),
                u.source,
                u.step.describe(u.mode == Mode::Port)
            ));
            match &u.port {
                Some(p) => render_port(u, p, &mut out),
                None => render_follow(u, &mut out),
            }
        }
        out
    }
}

fn render_follow(u: &UpstreamReport, out: &mut String) {
    let target = u.target.as_deref().unwrap_or_default();
    let range = match &u.range_start {
        Some(s) => format!("{}..{}", abbrev(s), abbrev(target)),
        None => format!("full history up to {}", abbrev(target)),
    };
    let commits = u.commits.unwrap_or_default();
    out.push_str(&format!(
        "  range: {range} ({commits} commit{})\n",
        plural(commits)
    ));
    if let Some(rev) = &u.rev {
        out.push_str(&format!("  target named by: {rev}\n"));
    }
    if let Some(m) = &u.merge_commit {
        out.push_str(&format!("  merge commit: {}\n", abbrev(m)));
    }
    render_conflicts(&u.conflicts, "  ", out);
    let before = u.own_changes_before.as_ref().map(fmt_stat);
    let after = u.own_changes_after.as_ref().map(fmt_stat);
    out.push_str(&format!(
        "  own changes against the upstream: before {}, after {}\n",
        before.as_deref().unwrap_or("none (first import)"),
        after.as_deref().unwrap_or("not merged yet")
    ));
}

fn render_port(u: &UpstreamReport, p: &PortReport, out: &mut String) {
    if let Some(rev) = &u.rev {
        out.push_str(&format!("  plan limited to: {rev}\n"));
    }
    if p.taken.is_empty() {
        out.push_str("  taken: none\n");
    } else {
        out.push_str("  taken, in the order copied:\n");
    }
    for t in &p.taken {
        let outcome = match (&t.copy, &t.copied) {
            (CopyStep::Copied, Some(c)) => format!("copied as {}", abbrev(c)),
            (CopyStep::Conflict, _) => "in conflict".to_string(),
            (CopyStep::CommitFailed, _) => "its commit failed".to_string(),
            _ => "not copied yet".to_string(),
        };
        out.push_str(&format!(
            "    {} {}  ({outcome})\n",
            abbrev(&t.commit),
            t.subject
        ));
        render_conflicts(&t.conflicts, "      ", out);
        if !t.left_out.is_empty() {
            out.push_str(&format!("      left out: {}\n", t.left_out.join(", ")));
        }
        if !t.earlier_pending.is_empty() {
            out.push_str("      earlier pending commits that change the same files:\n");
            for e in &t.earlier_pending {
                out.push_str(&format!("        {} {}\n", abbrev(&e.commit), e.subject));
            }
        }
    }
    if !p.unchanged.is_empty() {
        out.push_str("  recorded as present because the copy changed nothing:\n");
        for c in &p.unchanged {
            out.push_str(&format!("    {} {}\n", abbrev(&c.commit), c.subject));
            if !c.left_out.is_empty() {
                out.push_str(&format!("      left out: {}\n", c.left_out.join(", ")));
            }
        }
    }
    if p.decisions.is_empty() {
        out.push_str("  decisions: none\n");
    } else {
        out.push_str("  decisions:\n");
    }
    for d in &p.decisions {
        out.push_str(&format!(
            "    {} {} {}{}\n",
            d.verdict.as_str(),
            abbrev(&d.commit),
            d.subject,
            d.note
                .as_deref()
                .map(|n| format!(" # {n}"))
                .unwrap_or_default()
        ));
    }
    out.push_str(&format!(
        "  left pending: {} commit{}\n",
        p.left_pending,
        plural(p.left_pending)
    ));
    if let Some(j) = &p.journal_commit {
        out.push_str(&format!("  journal commit: {}\n", abbrev(j)));
    }
}

fn render_conflicts(conflicts: &[ConflictFile], indent: &str, out: &mut String) {
    if conflicts.is_empty() {
        out.push_str(&format!("{indent}conflicts: none\n"));
    } else {
        out.push_str(&format!("{indent}conflicts:\n"));
        for c in conflicts {
            out.push_str(&format!(
                "{indent}  {}  ({})\n",
                c.path,
                c.resolution.describe()
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numstat_counts_text_and_binary_files() {
        let s = parse_numstat("3\t1\ta.txt\u{0}-\t-\timg.png\u{0}10\t0\tb.txt\u{0}");
        assert_eq!(
            s,
            DiffStat {
                files: 3,
                insertions: 13,
                deletions: 1
            }
        );
    }
}
