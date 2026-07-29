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

/// A `{ref}/{path...}` wildcard split into its resolved parts.
pub struct RefPath<'r> {
    pub commit: Commit<'r>,
    /// The ref part exactly as it appeared in the URL (for immutable-header checks).
    pub refname: String,
    /// Tree path relative to the root; empty for the root itself.
    pub path: String,
}

/// Splits a `{ref}/{path...}` wildcard where the ref may itself contain `/`
/// (branch and tag names). The longest leading segment sequence matching an
/// existing branch or tag wins — unique, since git forbids a ref `a` next to
/// a ref `a/b`. Without a match the first segment is taken as the ref (commit
/// shas, `HEAD`).
pub fn resolve_ref_path<'r>(repo: &'r Repository, rest: &str) -> Result<RefPath<'r>, ApiError> {
    let rest = rest.trim_matches('/');
    let segments: Vec<&str> = rest.split('/').collect();
    if segments
        .iter()
        .any(|segment| segment.is_empty() || *segment == "." || *segment == "..")
    {
        return Err(ApiError::InvalidParam(format!("invalid path '{rest}'")));
    }

    let refnames = ref_shorthands(repo)?;
    let mut split = 1; // fallback: first segment is the ref
    for end in 1..=segments.len() {
        if refnames.contains(&segments[..end].join("/")) {
            split = end;
        }
    }

    let refname = segments[..split].join("/");
    let commit = resolve_commit(repo, &refname)?;
    Ok(RefPath {
        commit,
        refname,
        path: segments[split..].join("/"),
    })
}

/// Shorthand names of all local branches and tags.
fn ref_shorthands(repo: &Repository) -> Result<std::collections::HashSet<String>, ApiError> {
    let mut names = std::collections::HashSet::new();
    for reference in repo.references()? {
        let Ok(reference) = reference else { continue };
        let Some(name) = reference.name() else {
            continue; // non-utf8 ref name
        };
        if let Some(short) = name
            .strip_prefix("refs/heads/")
            .or_else(|| name.strip_prefix("refs/tags/"))
        {
            names.insert(short.to_owned());
        }
    }
    Ok(names)
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
    fn resolve_ref_path_should_prefer_longest_ref_match() {
        let (_dir, repo) = fixture_repo();
        let head = repo.head().unwrap().peel_to_commit().unwrap().id();
        repo.branch("feature/x", &repo.find_commit(head).unwrap(), false)
            .unwrap();
        let split = resolve_ref_path(&repo, "feature/x/src/main.rs").unwrap();
        assert_eq!(split.refname, "feature/x");
        assert_eq!(split.path, "src/main.rs");
        assert_eq!(split.commit.id(), head);
    }

    #[test]
    fn resolve_ref_path_should_fall_back_to_first_segment_for_shas() {
        let (_dir, repo) = fixture_repo();
        let head = repo.head().unwrap().peel_to_commit().unwrap().id();
        let split = resolve_ref_path(&repo, &format!("{head}/a.txt")).unwrap();
        assert_eq!(split.refname, head.to_string());
        assert_eq!(split.path, "a.txt");
        // Ref-only input yields an empty path (tree root).
        let root = resolve_ref_path(&repo, &head.to_string()).unwrap();
        assert_eq!(root.path, "");
    }

    #[test]
    fn resolve_ref_path_should_reject_dot_segments() {
        let (_dir, repo) = fixture_repo();
        for rest in ["main/../etc", "main/./a", "main//a"] {
            assert!(
                matches!(
                    resolve_ref_path(&repo, rest),
                    Err(ApiError::InvalidParam(_))
                ),
                "rest {rest:?} was not rejected"
            );
        }
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
