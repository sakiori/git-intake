//! `fetch`: add missing remotes and fetch the selected upstreams.

use anyhow::Result;

use crate::Exit;
use crate::remotes;
use crate::repo::Repo;

pub fn run(repo: &Repo, names: &[String]) -> Result<Exit> {
    let loaded = repo.load_config()?;
    let selected = loaded.config.select(names)?;
    let added = remotes::ensure_all(&repo.git, &selected)?;
    let mut failed = false;
    for up in &selected {
        if added.contains(&up.name) {
            println!(
                "{}: added remote '{}' ({})",
                up.name,
                up.remote,
                up.url.as_deref().unwrap_or_default()
            );
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
