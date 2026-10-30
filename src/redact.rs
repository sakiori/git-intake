//! The one path every message takes before it is shown, in text or JSON:
//! credentials written into URLs are removed, and so is every token the tool
//! holds, wherever it appears, as in a forge's message that quotes it.

use std::sync::{Mutex, PoisonError};

/// What a token the tool holds is replaced with in a message.
const HIDDEN: &str = "[hidden]";

/// The tokens the tool holds, which no message shows.
static HELD: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Keeps every later message from showing `secret`.
pub fn hold(secret: &str) {
    if secret.is_empty() {
        return;
    }
    let mut held = HELD.lock().unwrap_or_else(PoisonError::into_inner);
    if !held.iter().any(|s| s == secret) {
        held.push(secret.to_string());
    }
}

/// `text` as it may be shown: without the credentials of its URLs (see
/// [`without_credentials`]) and without the tokens the tool holds.
pub fn redacted(text: &str) -> String {
    let held = HELD.lock().unwrap_or_else(PoisonError::into_inner);
    without_secrets(&without_credentials(text), &held)
}

/// `text` with each of `secrets` replaced, the longest first, so that a
/// token containing another is hidden whole.
fn without_secrets(text: &str, secrets: &[String]) -> String {
    let mut secrets: Vec<&String> = secrets.iter().filter(|s| !s.is_empty()).collect();
    secrets.sort_by_key(|s| std::cmp::Reverse(s.len()));
    let mut out = text.to_string();
    for secret in secrets {
        if out.contains(secret.as_str()) {
            out = out.replace(secret.as_str(), HIDDEN);
        }
    }
    out
}

/// `text` with the credentials removed from every URL in it: a password
/// always, and for `http` and `https` a user without one, which is where a
/// token is usually written (`https://<token>@host/path`). Another scheme's
/// user, as `git` in `ssh://git@host/path`, names an account rather than a
/// credential and stays, as it does in the scp-like form `git@host:path`. A
/// credential written into a remote's or an upstream's URL thus never
/// reaches a message.
fn without_credentials(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find("://") {
        let (head, tail) = rest.split_at(at + 3);
        out.push_str(head);
        let scheme: String = head[..at]
            .chars()
            .rev()
            .take_while(|c| c.is_ascii_alphanumeric() || "+.-".contains(*c))
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<String>()
            .to_ascii_lowercase();
        let end = tail
            .find(|c: char| "/?#".contains(c) || c.is_whitespace() || "'\"`<>()[]".contains(c))
            .unwrap_or(tail.len());
        let authority = &tail[..end];
        match authority.rsplit_once('@') {
            None => out.push_str(authority),
            Some((user, host)) => {
                let user = user.split_once(':').map_or(user, |(name, _)| name);
                if !(scheme == "http" || scheme == "https") && !user.is_empty() {
                    out.push_str(user);
                    out.push('@');
                }
                out.push_str(host);
            }
        }
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credentials_in_urls_are_removed() {
        assert_eq!(
            without_credentials("push to https://user:s3cret@gitlab.example.com/g/app.git failed"),
            "push to https://gitlab.example.com/g/app.git failed"
        );
        assert_eq!(
            without_credentials("https://ghp_token@github.com/team/app.git"),
            "https://github.com/team/app.git"
        );
        // Another scheme keeps its user, and loses only a password.
        assert_eq!(
            without_credentials("'ssh://git@host:22/x' and ssh://deploy:s3cret@host/y"),
            "'ssh://git@host:22/x' and ssh://deploy@host/y"
        );
        // The authority ends at a path, a query or a fragment.
        assert_eq!(
            without_credentials("https://host?u=a@b and https://host#a@b"),
            "https://host?u=a@b and https://host#a@b"
        );
        assert_eq!(
            without_credentials("git@example.com:team/app.git"),
            "git@example.com:team/app.git"
        );
        assert_eq!(without_credentials("no url here"), "no url here");
        // In a JSON document as well.
        assert_eq!(
            without_credentials(r#"{"api": "https://user:s3cret@host/api/v4"}"#),
            r#"{"api": "https://host/api/v4"}"#
        );
    }

    #[test]
    fn held_tokens_are_hidden_wherever_they_appear() {
        let held = vec!["glpat-abc".to_string(), "glpat-abcdef".to_string()];
        assert_eq!(
            without_secrets("401: token glpat-abcdef rejected; glpat-abc too", &held),
            "401: token [hidden] rejected; [hidden] too"
        );
        assert_eq!(
            without_secrets("nothing to hide", &[String::new()]),
            "nothing to hide"
        );
    }
}
