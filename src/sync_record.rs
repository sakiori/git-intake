//! The sync record: trailers on the merge commits the tool creates and on
//! the commits it copies, the last synced commit of each upstream derived
//! from the merges, and the upstream commits the copies took.

use std::collections::HashMap;

use anyhow::Result;

use crate::git::Git;
use crate::integration_line;

pub const UPSTREAM_TRAILER: &str = "Intake-Upstream";
pub const TARGET_TRAILER: &str = "Intake-Target";
/// Names the upstream tag that points at the target of a sync merge.
pub const TAG_TRAILER: &str = "Intake-Tag";
/// Names the upstream commit a copy took.
pub const TAKEN_TRAILER: &str = "Intake-Taken";

const FIELD: char = '\u{1f}';
const RECORD: char = '\u{1e}';

/// One sync merge found in history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    pub commit: String,
    pub upstream: String,
    pub target: String,
    /// The upstream tag that points at the target, when the merge names one.
    pub tag: Option<String>,
}

/// The commit message of a sync merge, naming the upstream tag `tag` of the
/// target when one points at it.
pub fn message(
    name: &str,
    source: &str,
    short_target: &str,
    target: &str,
    tag: Option<&str>,
) -> String {
    match tag {
        None => format!(
            "sync({name}): merge {source} at {short_target}\n\n{UPSTREAM_TRAILER}: {name}\n{TARGET_TRAILER}: {target}\n"
        ),
        Some(tag) => format!(
            "sync({name}): merge {source} at {short_target} ({tag})\n\n{UPSTREAM_TRAILER}: {name}\n{TARGET_TRAILER}: {target}\n{TAG_TRAILER}: {tag}\n"
        ),
    }
}

/// The trailers a copy of `taken` from upstream `name` carries, as
/// `git commit --trailer` takes them.
pub fn copy_trailers(name: &str, taken: &str) -> [String; 2] {
    [
        format!("{UPSTREAM_TRAILER}: {name}"),
        format!("{TAKEN_TRAILER}: {taken}"),
    ]
}

fn log_format() -> String {
    format!(
        "--format=%H%x1f%(trailers:key={UPSTREAM_TRAILER},valueonly,separator=%x2C)%x1f%(trailers:key={TARGET_TRAILER},valueonly,separator=%x2C)%x1f%(trailers:key={TAG_TRAILER},valueonly,separator=%x2C)%x1e"
    )
}

fn trailer_format(second: &str) -> String {
    format!(
        "--format=%H%x1f%(trailers:key={UPSTREAM_TRAILER},valueonly,separator=%x2C)%x1f%(trailers:key={second},valueonly,separator=%x2C)%x1e"
    )
}

/// This repository's own sync records on the integration line ending at
/// `rev`, newest first.
///
/// The integration line (see [`integration_line`]) holds the tool's merges
/// however the update landed: on the first-parent line when the integration
/// branch was fast-forwarded to the update branch, and right after the merge
/// commit that brought the update in when a merge request was merged with
/// one. Commits contained in a configured upstream's fetched branch are
/// excluded: such records were inherited from that upstream (for example a
/// stacked branch that starts from another fork's integration branch), while
/// a merge this repository creates is never part of its upstream.
/// `upstream_tips` are the fetched tips of the configured upstreams.
pub fn records(git: &Git, rev: &str, upstream_tips: &[String]) -> Result<Vec<Record>> {
    Ok(integration_line::walk(git, rev, upstream_tips)?.records)
}

/// The sync records that `commits` carry, in their order.
pub fn read<'a>(git: &Git, commits: impl IntoIterator<Item = &'a str>) -> Result<Vec<Record>> {
    let input: String = commits.into_iter().map(|c| format!("{c}\n")).collect();
    if input.is_empty() {
        return Ok(Vec::new());
    }
    // `--no-walk=unsorted` keeps the order given and only filters it.
    let out = git.run_with_input(
        &[
            "log".to_string(),
            "--no-walk=unsorted".into(),
            "--stdin".into(),
            "--fixed-strings".into(),
            format!("--grep={UPSTREAM_TRAILER}: "),
            log_format(),
            "--".into(),
        ],
        input,
    )?;
    Ok(parse_log(&out))
}

/// The upstream commits, oldest copy first, that copies on the history
/// ending at `rev` name as taken from upstream `name`, as written in their
/// trailers. The whole
/// history counts, so that a copy made by hand on a merged branch counts as
/// well; commits contained in a configured upstream's fetched branch are
/// excluded, as in [`records`].
pub fn taken(git: &Git, rev: &str, upstream_tips: &[String], name: &str) -> Result<Vec<String>> {
    let args = log_args(
        upstream_tips,
        &["--reverse".into(), trailer_format(TAKEN_TRAILER)],
        TAKEN_TRAILER,
        rev,
    );
    let out = git.run(&args)?;
    let mut found = Vec::new();
    for entry in out.split(RECORD) {
        let mut fields = entry.trim_matches(['\n', '\r']).split(FIELD);
        let (Some(_), Some(upstreams), Some(commits)) =
            (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        if !upstreams.split(',').any(|u| u.trim() == name) {
            continue;
        }
        for c in commits
            .split(',')
            .map(|c| c.trim().to_ascii_lowercase())
            .filter(|c| !c.is_empty())
        {
            if !found.contains(&c) {
                found.push(c);
            }
        }
    }
    Ok(found)
}

/// `git log` arguments that list the commits of `rev` whose message holds
/// `trailer`, without those of the configured upstreams' fetched branches.
fn log_args(upstream_tips: &[String], options: &[String], trailer: &str, rev: &str) -> Vec<String> {
    let mut args: Vec<String> = vec!["log".into(), "--fixed-strings".into()];
    args.push(format!("--grep={trailer}: "));
    args.extend(options.iter().cloned());
    args.push(rev.into());
    if !upstream_tips.is_empty() {
        args.push("--not".into());
        args.extend(upstream_tips.iter().cloned());
    }
    args.push("--".into());
    args
}

/// The sync record carried by one commit, as git parses its trailers.
pub fn record_of(git: &Git, commit: &str) -> Result<Option<Record>> {
    let out = git.run(&["log", "-1", &log_format(), commit, "--"])?;
    Ok(parse_log(&out).into_iter().next())
}

/// The upstream commit `commit` names as taken from upstream `name`, if it
/// is a copy from that upstream.
pub fn copy_of(git: &Git, commit: &str, name: &str) -> Result<Option<String>> {
    let out = git.run(&["log", "-1", &trailer_format(TAKEN_TRAILER), commit, "--"])?;
    let mut fields = out.trim_matches(['\n', '\r', RECORD]).split(FIELD);
    let (Some(_), Some(upstreams), Some(taken)) = (fields.next(), fields.next(), fields.next())
    else {
        return Ok(None);
    };
    if !upstreams.split(',').any(|u| u.trim() == name) {
        return Ok(None);
    }
    Ok(taken
        .split(',')
        .map(str::trim)
        .find(|t| !t.is_empty())
        .map(str::to_ascii_lowercase))
}

/// Parses the output of the `git log` format used by [`records`].
pub fn parse_log(out: &str) -> Vec<Record> {
    out.split(RECORD)
        .filter_map(|entry| {
            let mut fields = entry.trim_matches(['\n', '\r']).split(FIELD);
            let commit = fields.next()?.trim();
            let upstream = first_value(fields.next()?);
            let target = first_value(fields.next()?);
            let tag = fields.next().map(first_value).unwrap_or_default();
            if commit.is_empty() || upstream.is_empty() || target.is_empty() {
                return None;
            }
            Some(Record {
                commit: commit.to_string(),
                upstream: upstream.to_string(),
                target: target.to_string(),
                tag: (!tag.is_empty()).then(|| tag.to_string()),
            })
        })
        .collect()
}

fn first_value(field: &str) -> &str {
    field.split(',').next().unwrap_or("").trim()
}

/// The record of each upstream's last synced commit, by upstream name:
/// of the upstream's records, newest first, the first whose target no other
/// of its records' targets contains. A record merged in later for an older
/// target, as from a branch that started before an update, therefore never
/// moves an upstream back. A target git does not have is compared with
/// none. One `cat-file` checks the targets and one `merge-base
/// --independent` compares those of each upstream with several.
pub fn latest(git: &Git, records: &[Record]) -> Result<HashMap<String, Record>> {
    let mut names: Vec<&str> = Vec::new();
    let mut targets: Vec<String> = Vec::new();
    for r in records {
        if !names.contains(&r.upstream.as_str()) {
            names.push(&r.upstream);
        }
        let t = r.target.to_ascii_lowercase();
        if !targets.contains(&t) {
            targets.push(t);
        }
    }
    let present = present_commits(git, &targets)?;
    let mut map = HashMap::new();
    for name in names {
        let own: Vec<&Record> = records.iter().filter(|r| r.upstream == name).collect();
        let mut compared: Vec<String> = Vec::new();
        for r in &own {
            let t = r.target.to_ascii_lowercase();
            if present.contains(&t) && !compared.contains(&t) {
                compared.push(t);
            }
        }
        let independent: Vec<String> = if compared.len() > 1 {
            let mut args = vec!["merge-base".to_string(), "--independent".into()];
            args.extend(compared.iter().cloned());
            crate::git::lines(&git.run(&args)?)
        } else {
            compared.clone()
        };
        let chosen = own.iter().find(|r| {
            let t = r.target.to_ascii_lowercase();
            !present.contains(&t) || independent.contains(&t)
        });
        if let Some(r) = chosen {
            map.insert(name.to_string(), (*r).clone());
        }
    }
    Ok(map)
}

/// Those of `ids`, full commit ids in lower case, that name commits git has.
fn present_commits(git: &Git, ids: &[String]) -> Result<Vec<String>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let input: String = ids.iter().map(|id| format!("{id}\n")).collect();
    let out = git.run_with_input(
        &["cat-file", "--batch-check=%(objectname) %(objecttype)"],
        input,
    )?;
    Ok(out
        .lines()
        .filter_map(|l| l.strip_suffix(" commit"))
        .map(str::to_string)
        .filter(|id| ids.contains(id))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_has_subject_and_trailers() {
        let m = message("app-a", "app-a/main", "abc1234", "abc1234ffff", None);
        assert_eq!(
            m,
            "sync(app-a): merge app-a/main at abc1234\n\nIntake-Upstream: app-a\nIntake-Target: abc1234ffff\n"
        );
        let m = message(
            "app-a",
            "app-a/main",
            "abc1234",
            "abc1234ffff",
            Some("v1.2.0"),
        );
        assert_eq!(
            m,
            "sync(app-a): merge app-a/main at abc1234 (v1.2.0)\n\nIntake-Upstream: app-a\nIntake-Target: abc1234ffff\nIntake-Tag: v1.2.0\n"
        );
    }

    #[test]
    fn parses_records_in_order() {
        let out = "c3\u{1f}a\u{1f}t3\u{1f}v3\u{1e}\nc2\u{1f}b\u{1f}t2\u{1f}\u{1e}\nc1\u{1f}a\u{1f}t1\u{1f}\u{1e}\nc0\u{1f}\u{1f}\u{1f}\u{1e}\n";
        let recs = parse_log(out);
        assert_eq!(recs.len(), 3);
        assert_eq!(recs[0].target, "t3");
        assert_eq!(recs[0].tag.as_deref(), Some("v3"));
        assert_eq!(recs[1].upstream, "b");
        assert_eq!(recs[1].tag, None);
    }
}
