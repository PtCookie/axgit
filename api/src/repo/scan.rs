use std::path::Path;

use anyhow::Context;
use git2::Repository;

use super::RepoInfo;
use super::meta;

/// Scans `root` for bare `*.git` directories (one level deep, cgit
/// `scan-path` equivalent) and returns them sorted by name.
///
/// Blocking (filesystem + libgit2) — call from `spawn_blocking`.
pub fn scan_repos(root: &Path) -> anyhow::Result<Vec<RepoInfo>> {
    let entries = std::fs::read_dir(root)
        .with_context(|| format!("failed to read repo root {}", root.display()))?;

    let mut repos = Vec::new();
    for entry in entries {
        let entry = entry.with_context(|| format!("failed to read entry in {}", root.display()))?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(name) = path
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.strip_suffix(".git"))
            .filter(|n| !n.is_empty())
        else {
            continue;
        };
        match Repository::open_bare(&path) {
            Ok(repo) => repos.push(meta::read_repo_info(&repo, name)),
            Err(err) => {
                tracing::warn!(repo = %path.display(), error = %err, "skipping unreadable repository");
            }
        }
    }
    repos.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(repos)
}
