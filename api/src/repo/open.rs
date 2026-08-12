use std::path::Path;

use git2::Repository;

use super::meta;
use crate::error::ApiError;

/// Opens `{root}/{name}.git` as a bare repository. `name` is the `{repo}`
/// URL parameter (no `.git` suffix), i.e. untrusted input.
///
/// Also enforces the `ignore` config flag (docs/DECISIONS.md #66): an
/// ignored repository reports `repo_not_found` here, the single choke point
/// every per-repo handler and Smart HTTP go through, so it's unreachable by
/// any means — not just absent from the index (`hide`'s narrower effect,
/// enforced only in `scan.rs`).
pub fn open_named(root: &Path, name: &str) -> Result<Repository, ApiError> {
    // Reject separators and leading dots so the joined path cannot escape
    // the repo root (`..`, absolute-ish names, hidden directories).
    if name.is_empty() || name.starts_with('.') || name.contains(['/', '\\']) {
        return Err(ApiError::InvalidParam(format!(
            "invalid repository name '{name}'"
        )));
    }
    let repo = Repository::open_bare(root.join(format!("{name}.git")))
        .map_err(|_| ApiError::RepoNotFound(name.to_owned()))?;
    if meta::is_ignored(&repo) {
        return Err(ApiError::RepoNotFound(name.to_owned()));
    }
    Ok(repo)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open_in_empty_root(name: &str) -> Result<Repository, ApiError> {
        let root = tempfile::tempdir().expect("failed to create tempdir");
        open_named(root.path(), name)
    }

    #[test]
    fn open_named_should_reject_traversal_names() {
        for name in ["", ".", "..", "../etc", "a/b", "a\\b", ".hidden"] {
            assert!(
                matches!(open_in_empty_root(name), Err(ApiError::InvalidParam(_))),
                "name {name:?} was not rejected as invalid"
            );
        }
    }

    #[test]
    fn open_named_should_return_not_found_for_missing_repo() {
        assert!(matches!(
            open_in_empty_root("missing"),
            Err(ApiError::RepoNotFound(name)) if name == "missing"
        ));
    }
}
