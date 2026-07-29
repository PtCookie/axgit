use git2::{Commit, Repository};

use crate::error::ApiError;

/// Resolves a user-supplied ref (branch, tag, or commit sha) to a commit.
/// Both resolution and peeling failures map to [`ApiError::RefNotFound`] —
/// never `?`-propagate git2 errors here, that would turn a bad ref into a 500.
/// Callers handle the "no ref given" default (usually HEAD) themselves.
pub fn resolve_commit<'r>(repo: &'r Repository, refname: &str) -> Result<Commit<'r>, ApiError> {
    repo.revparse_single(refname)
        .and_then(|object| object.peel_to_commit())
        .map_err(|_| ApiError::RefNotFound(refname.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_repo() -> (tempfile::TempDir, Repository) {
        let dir = tempfile::tempdir().expect("failed to create tempdir");
        let repo = Repository::init(dir.path()).expect("failed to init repo");
        {
            let mut index = repo.index().unwrap();
            let tree = index.write_tree().unwrap();
            let tree = repo.find_tree(tree).unwrap();
            let sig = git2::Signature::now("Test", "test@example.com").unwrap();
            repo.commit(Some("HEAD"), &sig, &sig, "initial", &tree, &[])
                .unwrap();
        }
        (dir, repo)
    }

    #[test]
    fn resolve_commit_should_accept_branch_and_sha() {
        let (_dir, repo) = fixture_repo();
        let head_ref = repo.head().unwrap();
        // The default branch name depends on the environment; read it from HEAD.
        let branch = head_ref.shorthand().unwrap().to_owned();
        let head = head_ref.target().unwrap();
        drop(head_ref);
        let by_branch = resolve_commit(&repo, &branch).expect("branch did not resolve");
        assert_eq!(by_branch.id(), head);
        let by_sha = resolve_commit(&repo, &head.to_string()).expect("sha did not resolve");
        assert_eq!(by_sha.id(), head);
    }

    #[test]
    fn resolve_commit_should_return_ref_not_found_for_unknown_ref() {
        let (_dir, repo) = fixture_repo();
        assert!(matches!(
            resolve_commit(&repo, "no-such-ref"),
            Err(ApiError::RefNotFound(name)) if name == "no-such-ref"
        ));
    }

    #[test]
    fn resolve_commit_should_return_ref_not_found_for_non_commit_object() {
        let (_dir, repo) = fixture_repo();
        let tree = repo.head().unwrap().peel_to_commit().unwrap().tree_id();
        assert!(matches!(
            resolve_commit(&repo, &tree.to_string()),
            Err(ApiError::RefNotFound(_))
        ));
    }
}
