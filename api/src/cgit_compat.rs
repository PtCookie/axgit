//! cgit-style URL compatibility redirects (docs/DECISIONS.md #35).
//!
//! Historical/external links point at cgit's URL shapes
//! (`/{repo}.git/commit/?id={sha}`, `/{repo}.git/log/?h={ref}`, a bare
//! `/{repo}.git`, …) — Jenkins' `cgit` Repository browser is the case that
//! prompted this — which don't correspond to any axgit route today. This
//! module recognizes a handful of core shapes and answers with a permanent
//! redirect to the matching axgit URL. Everything else, including any shape
//! that's already a valid axgit route, is left alone so `shell::shell_for`
//! keeps answering exactly as before (this also rules out redirect loops).
//!
//! Out of scope (deliberately): cgit's `tree/{path}?id=` can't be split into
//! `tree` vs `blob` without a git lookup to tell whether `{path}` is a
//! directory or a file, so it isn't handled here — only the `.git` suffix on
//! it gets stripped, via the generic fallback branch below. `plain/`,
//! `atom/`, `snapshot/` are out of scope too.
//!
//! Query values are never percent-decoded: matching/validation happens on
//! the raw (still-encoded) text, and anything copied into the `Location` is
//! copied verbatim from the request, so there's no separate re-encoding step
//! and nothing built here can smuggle in characters the request didn't
//! already contain.

use axum::http::Uri;

/// Returns the absolute path (+ query) to redirect a cgit-shaped request to,
/// or `None` if `uri` doesn't match a recognized shape — including when it
/// already *is* a valid axgit route, so redirecting would fight or loop with
/// `shell::shell_for`.
pub fn redirect_for(uri: &Uri) -> Option<String> {
    let path = uri.path();
    let segments: Vec<&str> = path
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect();
    let (repo_raw, rest) = segments.split_first()?;

    let stripped = repo_raw
        .strip_suffix(".git")
        .filter(|name| !name.is_empty());
    let git_stripped = stripped.is_some();
    let repo = stripped.unwrap_or(repo_raw);

    let query = uri.query().unwrap_or("");
    let sha = query_value(query, "id").filter(|value| looks_like_sha(value));
    let ref_name = query_value(query, "h");

    let is_commit_or_diff = matches!(rest, ["commit"] | ["diff"]);
    let is_log = matches!(rest, ["log"]);
    let is_refs = matches!(rest, ["refs"]);

    // cgit's changeset/diff/log query shapes redirect regardless of `.git` —
    // `/{repo}/commit` (2 segments) is never a valid axgit shape either way,
    // so there's no native route to conflict with.
    let specific = if is_commit_or_diff {
        sha.map(|sha| format!("/{repo}/commit/{sha}"))
    } else if is_log {
        ref_name.map(|ref_name| format!("/{repo}/log?ref={ref_name}"))
    } else {
        None
    };

    let target = specific.or_else(|| {
        // Everything past this point only fires once `.git` was actually
        // stripped — without it, the request already matches a native axgit
        // shape (`shell::shell_for`), and redirecting would just loop back.
        if !git_stripped {
            return None;
        }
        Some(if rest.is_empty() {
            format!("/{repo}")
        } else if is_refs {
            format!("/{repo}/refs")
        } else {
            let suffix = rest.join("/");
            if query.is_empty() {
                format!("/{repo}/{suffix}")
            } else {
                format!("/{repo}/{suffix}?{query}")
            }
        })
    })?;

    let current = match uri.query() {
        Some(q) => format!("{path}?{q}"),
        None => path.to_owned(),
    };
    (target != current).then_some(target)
}

/// Finds `key=value` in a raw (still percent-encoded) query string. The
/// value is returned undecoded — see the module doc comment.
fn query_value<'a>(query: &'a str, key: &str) -> Option<&'a str> {
    query.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=')?;
        (k == key).then_some(v)
    })
}

/// cgit's `id=` is always a full or abbreviated hex object id. Rejecting
/// anything else keeps whatever is copied into the `Location` header
/// constrained to `[0-9a-fA-F]`, regardless of what a caller sent.
fn looks_like_sha(value: &str) -> bool {
    (4..=64).contains(&value.len()) && value.bytes().all(|b| b.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::redirect_for;

    fn redirect(uri: &str) -> Option<String> {
        redirect_for(&uri.parse().unwrap())
    }

    #[test]
    fn bare_dot_git_redirects_to_the_repo_page() {
        assert_eq!(redirect("/axgit.git"), Some("/axgit".to_owned()));
        assert_eq!(redirect("/axgit.git/"), Some("/axgit".to_owned()));
    }

    #[test]
    fn commit_query_shape_redirects_to_the_commit_page() {
        let sha = "abc123def456";
        assert_eq!(
            redirect(&format!("/axgit.git/commit/?id={sha}")),
            Some(format!("/axgit/commit/{sha}"))
        );
        assert_eq!(
            redirect(&format!("/axgit/commit?id={sha}")),
            Some(format!("/axgit/commit/{sha}"))
        );
    }

    #[test]
    fn diff_query_shape_redirects_to_the_commit_page() {
        let sha = "abc123def456";
        assert_eq!(
            redirect(&format!("/axgit.git/diff/?id={sha}")),
            Some(format!("/axgit/commit/{sha}"))
        );
    }

    #[test]
    fn log_with_h_redirects_to_the_log_page_with_ref_query() {
        assert_eq!(
            redirect("/axgit.git/log/?h=main"),
            Some("/axgit/log?ref=main".to_owned())
        );
        assert_eq!(
            redirect("/axgit/log?h=main"),
            Some("/axgit/log?ref=main".to_owned())
        );
    }

    #[test]
    fn refs_dot_git_redirects_dropping_the_query() {
        assert_eq!(
            redirect("/axgit.git/refs/?h=main"),
            Some("/axgit/refs".to_owned())
        );
    }

    #[test]
    fn other_dot_git_paths_are_stripped_and_the_query_is_preserved() {
        assert_eq!(
            redirect("/axgit.git/tree/src/main.rs?id=abc123"),
            Some("/axgit/tree/src/main.rs?id=abc123".to_owned())
        );
    }

    #[test]
    fn native_routes_are_left_alone() {
        for uri in [
            "/",
            "/axgit",
            "/axgit/",
            "/axgit/log",
            "/axgit/log/",
            "/axgit/refs",
            "/axgit/commit/abc123",
            "/axgit/tree/src",
        ] {
            assert_eq!(redirect(uri), None, "uri {uri}");
        }
    }

    #[test]
    fn commit_without_a_valid_id_is_not_redirected() {
        assert_eq!(redirect("/axgit/commit"), None);
        assert_eq!(redirect("/axgit/commit?id=not-a-sha"), None);
        assert_eq!(redirect("/axgit/commit?id="), None);
    }

    #[test]
    fn log_without_h_is_not_redirected_when_dot_git_is_absent() {
        assert_eq!(redirect("/axgit/log?other=1"), None);
    }

    #[test]
    fn log_without_h_but_with_dot_git_still_strips_the_suffix() {
        assert_eq!(redirect("/axgit.git/log"), Some("/axgit/log".to_owned()));
    }

    #[test]
    fn bare_dot_only_is_not_treated_as_a_dot_git_suffix() {
        // `.git` stripped down to an empty name must not count as
        // `git_stripped` — mirrors `smart_http::repo_name`'s guard against
        // the same edge case. This only matters for the branches gated on
        // `git_stripped` (the bare-repo and generic-fallback cases): the
        // commit/diff/log query shapes redirect regardless of the repo name,
        // exactly like `shell::shell_for` never validates it either.
        assert_eq!(redirect("/.git"), None);
        assert_eq!(redirect("/.git/tree/src"), None);
    }

    // Smart HTTP shapes (`/{repo}.git/info/refs`, `/{repo}.git/git-upload-pack`)
    // are not asserted here: `redirect_for` has no notion of registered
    // routes, and taken in isolation it *would* map them like any other
    // `.git`-suffixed path. In the real router those paths are matched by
    // `smart_http`'s routes before the fallback (where `redirect_for` runs)
    // is ever reached — that precedence is what
    // `cgit_compat_test.rs::smart_http_routes_take_precedence_over_cgit_redirects`
    // verifies, against the full router.
}
