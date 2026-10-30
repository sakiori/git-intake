//! `report`: what an update took in from each upstream.

use anyhow::{Result, bail};
use serde::Serialize;

use crate::Exit;
use crate::commands::update::plural;
use crate::git::{Git, abbrev};
use crate::output::{Render, emit};
use crate::repo::Repo;
use crate::state::{self, ConflictFile, MergeKind, Resolution, UpdateState, UpstreamStep};

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
    pub kind: MergeKind,
    pub step: UpstreamStep,
    /// Start of the merged range; `None` for a first import (full history).
    pub range_start: Option<String>,
    pub target: String,
    pub commits: u64,
    pub merge_commit: Option<String>,
    pub conflicts: Vec<ConflictFile>,
    /// The repository's own changes relative to the upstream before the merge.
    pub own_changes_before: Option<DiffStat>,
    /// The repository's own changes relative to the upstream after the merge.
    pub own_changes_after: Option<DiffStat>,
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
    let live_unmerged = if in_progress && state.worktree.exists() && wt.merge_in_progress()? {
        Some(wt.unmerged_files()?)
    } else {
        None
    };
    let mut upstreams = Vec::new();
    for q in &state.upstreams {
        let up = state.config.upstream(&q.name);
        let path = up.and_then(|u| u.path.clone());
        let source = up.map(|u| u.source()).unwrap_or_default();

        let mut conflicts = q.conflicts.clone();
        if q.step == UpstreamStep::Conflict
            && let Some(unmerged) = &live_unmerged
        {
            for c in &mut conflicts {
                if c.resolution == Resolution::Unresolved && !unmerged.contains(&c.path) {
                    c.resolution = Resolution::Manual;
                }
            }
        }

        let range_start = match q.kind {
            MergeKind::Import => None,
            _ => match &q.previous {
                Some(p) => Some(p.clone()),
                None => git.merge_base(&state.base, &q.target)?,
            },
        };
        let own_changes_before = match (&range_start, q.kind) {
            (Some(start), MergeKind::Root) => Some(diff_stat(git, start, &state.base)?),
            (Some(start), MergeKind::Subtree) => Some(diff_stat(
                git,
                start,
                &format!("{}:{}", state.base, path.as_deref().unwrap_or_default()),
            )?),
            _ => None,
        };
        let own_changes_after = match (&q.merge_commit, q.kind) {
            (Some(m), MergeKind::Root) => Some(diff_stat(git, &q.target, m)?),
            (Some(m), _) => Some(diff_stat(
                git,
                &q.target,
                &format!("{m}:{}", path.as_deref().unwrap_or_default()),
            )?),
            (None, _) => None,
        };
        upstreams.push(UpstreamReport {
            name: q.name.clone(),
            path,
            source,
            kind: q.kind,
            step: q.step,
            range_start,
            target: q.target.clone(),
            commits: q.commits,
            merge_commit: q.merge_commit.clone(),
            conflicts,
            own_changes_before,
            own_changes_after,
        });
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
            let step = match u.step {
                UpstreamStep::Pending => "pending",
                UpstreamStep::Conflict => "in conflict",
                UpstreamStep::Merged => "merged",
            };
            out.push_str(&format!(
                "\n{} ({}, {}): {step}\n",
                u.name,
                u.path.as_deref().unwrap_or("/"),
                u.source
            ));
            let range = match &u.range_start {
                Some(s) => format!("{}..{}", abbrev(s), abbrev(&u.target)),
                None => format!("full history up to {}", abbrev(&u.target)),
            };
            out.push_str(&format!(
                "  range: {range} ({} commit{})\n",
                u.commits,
                plural(u.commits)
            ));
            if let Some(m) = &u.merge_commit {
                out.push_str(&format!("  merge commit: {}\n", abbrev(m)));
            }
            if u.conflicts.is_empty() {
                out.push_str("  conflicts: none\n");
            } else {
                out.push_str("  conflicts:\n");
                for c in &u.conflicts {
                    out.push_str(&format!("    {}  ({})\n", c.path, c.resolution.describe()));
                }
            }
            let before = u.own_changes_before.as_ref().map(fmt_stat);
            let after = u.own_changes_after.as_ref().map(fmt_stat);
            out.push_str(&format!(
                "  own changes against the upstream: before {}, after {}\n",
                before.as_deref().unwrap_or("none (first import)"),
                after.as_deref().unwrap_or("not merged yet")
            ));
        }
        out
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
