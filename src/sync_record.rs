//! The sync record: trailers on the merge commits the tool creates, and the
//! last synced commit of each upstream derived from them.

use std::collections::HashMap;

use anyhow::Result;

use crate::config::Config;
use crate::git::{Git, tracking_ref};

pub const UPSTREAM_TRAILER: &str = "Intake-Upstream";
pub const TARGET_TRAILER: &str = "Intake-Target";

const FIELD: char = '\u{1f}';
const RECORD: char = '\u{1e}';

/// One sync merge found in history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    pub commit: String,
    pub upstream: String,
    pub target: String,
}

/// The commit message of a sync merge.
pub fn message(name: &str, source: &str, short_target: &str, target: &str) -> String {
    format!(
        "sync({name}): merge {source} at {short_target}\n\n{UPSTREAM_TRAILER}: {name}\n{TARGET_TRAILER}: {target}\n"
    )
}

fn log_format() -> String {
    format!(
        "--format=%H%x1f%(trailers:key={UPSTREAM_TRAILER},valueonly,separator=%x2C)%x1f%(trailers:key={TARGET_TRAILER},valueonly,separator=%x2C)%x1e"
    )
}

/// This repository's own sync records on the integration line ending at
/// `rev`, newest first.
///
/// Only the first-parent line is walked: the tool's merges are always on it,
/// because `finish` fast-forwards the integration branch to the update branch,
/// whose merges take the previous head as first parent. Commits contained in a
/// configured upstream's fetched branch are excluded as well: such records
/// were inherited from that upstream (for example a stacked branch that starts
/// from another fork's integration line), while a merge this repository
/// creates is never part of its upstream.
pub fn records(git: &Git, rev: &str, config: &Config) -> Result<Vec<Record>> {
    let mut args: Vec<String> = vec![
        "log".into(),
        "--first-parent".into(),
        "--fixed-strings".into(),
        format!("--grep={UPSTREAM_TRAILER}: "),
        log_format(),
        rev.into(),
    ];
    let mut excluded = Vec::new();
    for up in &config.upstreams {
        let r = tracking_ref(&up.remote, &up.branch);
        if git.commit(&r)?.is_some() {
            excluded.push(r);
        }
    }
    if !excluded.is_empty() {
        args.push("--not".into());
        args.extend(excluded);
    }
    args.push("--".into());
    Ok(parse_log(&git.run(&args)?))
}

/// The sync record carried by one commit, as git parses its trailers.
pub fn record_of(git: &Git, commit: &str) -> Result<Option<Record>> {
    let out = git.run(&["log", "-1", &log_format(), commit, "--"])?;
    Ok(parse_log(&out).into_iter().next())
}

/// Parses the output of the `git log` format used by [`records`].
pub fn parse_log(out: &str) -> Vec<Record> {
    out.split(RECORD)
        .filter_map(|entry| {
            let mut fields = entry.trim_matches(['\n', '\r']).split(FIELD);
            let commit = fields.next()?.trim();
            let upstream = first_value(fields.next()?);
            let target = first_value(fields.next()?);
            if commit.is_empty() || upstream.is_empty() || target.is_empty() {
                return None;
            }
            Some(Record {
                commit: commit.to_string(),
                upstream: upstream.to_string(),
                target: target.to_string(),
            })
        })
        .collect()
}

fn first_value(field: &str) -> &str {
    field.split(',').next().unwrap_or("").trim()
}

/// The newest record per upstream name.
pub fn latest(records: &[Record]) -> HashMap<String, Record> {
    let mut map = HashMap::new();
    for r in records {
        map.entry(r.upstream.clone()).or_insert_with(|| r.clone());
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_has_subject_and_trailers() {
        let m = message("app-a", "app-a/main", "abc1234", "abc1234ffff");
        assert_eq!(
            m,
            "sync(app-a): merge app-a/main at abc1234\n\nIntake-Upstream: app-a\nIntake-Target: abc1234ffff\n"
        );
    }

    #[test]
    fn parses_records_and_keeps_newest() {
        let out = "c3\u{1f}a\u{1f}t3\u{1e}\nc2\u{1f}b\u{1f}t2\u{1e}\nc1\u{1f}a\u{1f}t1\u{1e}\nc0\u{1f}\u{1f}\u{1e}\n";
        let recs = parse_log(out);
        assert_eq!(recs.len(), 3);
        let latest = latest(&recs);
        assert_eq!(latest["a"].target, "t3");
        assert_eq!(latest["a"].commit, "c3");
        assert_eq!(latest["b"].target, "t2");
    }
}
