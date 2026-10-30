//! `fetch`: add missing remotes and fetch the selected upstreams.

use anyhow::Result;

use crate::Exit;
use crate::remotes;
use crate::repo::Repo;

/// Fetches each selected upstream on its own. An upstream whose remote is
/// unusable (missing without a URL, or pointing elsewhere) is refused and
/// reported; the others are still fetched. Any refusal or failure exits 1.
pub fn run(repo: &Repo, names: &[String]) -> Result<Exit> {
    let loaded = repo.load_config()?;
    let selected = loaded.config.select(names)?;
    let mut failed = false;
    for up in &selected {
        match remotes::ensure_one(&repo.git, up)? {
            Err(why) => {
                failed = true;
                println!("{}: refused: {why}", up.name);
                continue;
            }
            Ok(true) => println!(
                "{}: added remote '{}' ({})",
                up.name,
                up.remote,
                up.url.as_deref().unwrap_or_default()
            ),
            Ok(false) => {}
        }
        match remotes::fetch(&repo.git, up)? {
            Ok(()) => println!("{}: fetched {}", up.name, up.source()),
            Err(why) => {
                failed = true;
                println!("{}: fetch failed: {why}", up.name);
            }
        }
    }
    Ok(if failed {
        Exit::Negative
    } else {
        Exit::Success
    })
}
