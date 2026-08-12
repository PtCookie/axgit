//! Resolves a link for a submodule (gitlink, mode `160000`) tree row,
//! cgit's `module-link` / `repo.module-link.<path>` parity (docs/DECISIONS.md
//! #72).
//!
//! Two sources, tried in order, per gitlink path:
//!
//! 1. A configured template: `axgit.<path>.module-link` →
//!    `cgit.<path>.module-link` → `axgit.module-link` → `cgit.module-link`
//!    ([`meta::module_link_template`]). The first level that yields a value
//!    stops the search — an empty, malformed, or rejected template means
//!    "no link", it never falls through to `.gitmodules`. The template's two
//!    `%s` placeholders are the gitlink's full repo-relative path and its
//!    recorded commit sha, expanded by [`expand_template`] and then filtered
//!    through [`is_link_href`].
//! 2. `.gitmodules` in the resolved commit, an axgit extension cgit itself
//!    doesn't have: [`parse_gitmodules`] maps each stanza's `path` to its
//!    `url`, and an http(s) `url` is used verbatim (no substitution — it's
//!    already a URL) after passing the stricter [`meta::is_http_url`].
//!
//! Every failure degrades this one field to `None` rather than erroring the
//! whole listing (the same posture as `meta::homepage`/`defbranch`).

use git2::{Commit, Repository};

use super::meta::{self, is_http_url};
use super::tree::{EntryKind, TreeEntryInfo};

/// Cap on the `.gitmodules` blob read inline — generous even for a very
/// large superproject (roughly 600 `[submodule]` stanzas), and checked
/// against the object header before loading, same pattern as
/// `tree.rs::SYMLINK_TARGET_LIMIT`. An oversized file yields no links at
/// all rather than a truncated (and therefore wrong) parse.
const GITMODULES_LIMIT: u64 = 64 * 1024;

/// Fills `module_link` on every [`EntryKind::Commit`] row in `entries`
/// in place. A no-op scan (one bool check) when the listing has no gitlink
/// at all, which is the common case. `dir` is the directory being listed
/// (empty for the root), used to build each entry's full repo-relative path.
pub(crate) fn fill_module_links(
    repo: &Repository,
    commit: &Commit<'_>,
    dir: &str,
    entries: &mut [TreeEntryInfo],
) {
    if !entries.iter().any(|entry| entry.kind == EntryKind::Commit) {
        return;
    }

    let config = repo.config().and_then(|mut cfg| cfg.snapshot()).ok();
    // `.gitmodules` is read at most once, and only if a gitlink actually
    // needs it (a fully config-driven deployment never touches the odb).
    let mut gitmodules: Option<Vec<(String, String)>> = None;

    for entry in entries.iter_mut() {
        if entry.kind != EntryKind::Commit {
            continue;
        }
        let path = if dir.is_empty() {
            entry.name.clone()
        } else {
            format!("{dir}/{}", entry.name)
        };

        let from_config = config
            .as_ref()
            .and_then(|cfg| meta::module_link_template(cfg, &path))
            .map(|template| {
                expand_template(&template, &path, &entry.sha).filter(|url| is_link_href(url))
            });

        entry.module_link = match from_config {
            // A template applied (even if it resolved to no usable link) —
            // stop here, never fall through to `.gitmodules`.
            Some(link) => link,
            None => gitmodules
                .get_or_insert_with(|| read_gitmodules(repo, commit).unwrap_or_default())
                .iter()
                .find(|(p, _)| p == &path)
                .map(|(_, url)| url.clone())
                .filter(|url| is_http_url(url)),
        };
    }
}

/// Expands a cgitrc-style `module-link` template: the first `%s` becomes
/// `path`, the second becomes `sha`, and `%%` is a literal `%`. Anything
/// else involving `%` — a third `%s`, an unrecognized specifier, or a
/// trailing lone `%` — makes the template unusable and returns `None`
/// rather than guessing: a silently broken link is worse than no link, and
/// unlike C `printf`, there is no third argument to read here anyway.
fn expand_template(template: &str, path: &str, sha: &str) -> Option<String> {
    let mut out = String::with_capacity(template.len());
    let mut seen_s = 0u8;
    let mut chars = template.chars();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('s') => {
                seen_s += 1;
                match seen_s {
                    1 => out.push_str(path),
                    2 => out.push_str(sha),
                    _ => return None,
                }
            }
            Some('%') => out.push('%'),
            _ => return None,
        }
    }
    Some(out)
}

/// Whether an expanded `module-link` template result may be used as an
/// `<a href>`. Deliberately looser than [`meta::is_http_url`] — cgit's own
/// documented example is root-relative (`/git/%s/commit/?id=%s`), the
/// self-hosting case an operator behind the same reverse proxy actually
/// wants — but a *relative* template (cgit's other documented example,
/// `./?repo=%s&page=commit&id=%s`) is rejected: it would resolve against
/// the current tree path, so its meaning would change with directory depth,
/// which is not something a single config value can mean consistently.
///
/// Runs on the *expanded* result, not the template itself — a gitlink
/// literally named `javascript:alert(1)` under a bare `%s` template is
/// still rejected, because the scheme only comes from the template and this
/// check sees the substituted string.
fn is_link_href(url: &str) -> bool {
    if is_http_url(url) {
        return true;
    }
    // A leading `/` is root-relative, *except* when immediately followed by
    // another `/` or a `\` — both are folded into "protocol-relative" by
    // browser URL parsers (`//evil.com/x`, `/\evil.com/x`), which would
    // navigate off-site despite starting with `/`.
    let mut chars = url.chars();
    match (chars.next(), chars.next()) {
        (Some('/'), None) => true,
        (Some('/'), Some(next)) => next != '/' && next != '\\',
        _ => false,
    }
}

/// Reads and parses `.gitmodules` from the commit's root tree, if present,
/// readable as a blob, and within [`GITMODULES_LIMIT`]. Any failure — not
/// found, a symlink/submodule sharing the name, oversized, non-UTF-8 —
/// yields `None`, same posture as `readme.rs::find_readme`'s per-candidate
/// skips.
fn read_gitmodules(repo: &Repository, commit: &Commit<'_>) -> Option<Vec<(String, String)>> {
    let tree = commit.tree().ok()?;
    let entry = tree.get_name(".gitmodules")?;
    if entry.filemode() == 0o120000 {
        return None; // symlink, not a real .gitmodules
    }
    let odb = repo.odb().ok()?;
    let size = odb
        .read_header(entry.id())
        .ok()
        .map(|(size, _)| size as u64)?;
    if size > GITMODULES_LIMIT {
        return None;
    }
    let blob = entry.to_object(repo).ok()?.into_blob().ok()?;
    let content = std::str::from_utf8(blob.content()).ok()?;
    Some(parse_gitmodules(content))
}

/// Pure, line-oriented `.gitmodules` parser: maps each `[submodule "name"]`
/// stanza's `path` to its `url`, keyed on `path` (not the stanza name —
/// git allows the two to differ, and only `path` addresses the tree)  in
/// file order. A stanza is emitted only if it has both `path` and `url`.
///
/// Deliberately not handled: line continuations (`\` at end of line),
/// multi-line quoted values, `[include]`/`includeIf`, subsection escapes
/// beyond `\"`/`\\`, and relative `url` values (`../dep.git` is only
/// meaningful relative to the superproject's own remote, which a bare
/// repository doesn't have).
fn parse_gitmodules(content: &str) -> Vec<(String, String)> {
    let mut result = Vec::new();
    let mut in_submodule = false;
    let mut path: Option<String> = None;
    let mut url: Option<String> = None;

    let flush = |path: &mut Option<String>,
                 url: &mut Option<String>,
                 result: &mut Vec<(String, String)>| {
        if let (Some(p), Some(u)) = (path.take(), url.take()) {
            // Duplicate `path` across stanzas: first wins, resolved here
            // rather than left to the caller, so `parse_gitmodules`'s output
            // is already the answer, not raw data a caller must dedupe.
            if !result.iter().any(|(existing, _)| existing == &p) {
                result.push((p, u));
            }
        }
    };

    for raw_line in content.lines() {
        let line = raw_line.strip_suffix('\r').unwrap_or(raw_line).trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(rest) = line.strip_prefix('[') {
            // Entering a new section closes any open stanza.
            flush(&mut path, &mut url, &mut result);
            in_submodule = rest
                .trim_start()
                .to_ascii_lowercase()
                .starts_with("submodule");
            continue;
        }
        if !in_submodule {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim().to_ascii_lowercase();
        let value = parse_value(value.trim());
        match key.as_str() {
            "path" => path = Some(value.trim_end_matches('/').to_owned()),
            "url" => url = Some(value),
            _ => {}
        }
    }
    flush(&mut path, &mut url, &mut result);
    result
}

/// Unquotes a git-config-style value: a `"..."` value has `\"`, `\\`, `\n`,
/// `\t` unescaped; an unquoted value is truncated at the first `#`/`;`
/// (an inline comment) and trimmed.
fn parse_value(raw: &str) -> String {
    if let Some(inner) = raw.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
        let mut out = String::with_capacity(inner.len());
        let mut chars = inner.chars();
        while let Some(c) = chars.next() {
            if c != '\\' {
                out.push(c);
                continue;
            }
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some(other) => out.push(other), // `\"`, `\\`, or anything else literal
                None => {}
            }
        }
        return out;
    }
    let end = raw.find(['#', ';']).unwrap_or(raw.len());
    raw[..end].trim_end().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expand_template_substitutes_path_then_sha() {
        assert_eq!(
            expand_template("/git/%s/commit/?id=%s", "vendor/dep", "abc123"),
            Some("/git/vendor/dep/commit/?id=abc123".to_owned())
        );
    }

    #[test]
    fn expand_template_path_only() {
        assert_eq!(
            expand_template("https://example.com/%s", "vendor/dep", "abc123"),
            Some("https://example.com/vendor/dep".to_owned())
        );
    }

    #[test]
    fn expand_template_constant_url_needs_no_placeholder() {
        assert_eq!(
            expand_template("https://example.com/fixed", "vendor/dep", "abc123"),
            Some("https://example.com/fixed".to_owned())
        );
    }

    #[test]
    fn expand_template_percent_percent_is_literal_percent() {
        assert_eq!(
            expand_template("100%% at %s", "done", "sha"),
            Some("100% at done".to_owned())
        );
    }

    #[test]
    fn expand_template_a_third_percent_s_is_unusable() {
        assert_eq!(expand_template("%s/%s/%s", "p", "s"), None);
    }

    #[test]
    fn expand_template_unknown_specifier_is_unusable() {
        assert_eq!(expand_template("%d", "p", "s"), None);
    }

    #[test]
    fn expand_template_trailing_percent_is_unusable() {
        assert_eq!(expand_template("broken%", "p", "s"), None);
    }

    #[test]
    fn expand_template_never_swaps_path_and_sha() {
        let result = expand_template("%s-%s", "PATH", "SHA").unwrap();
        assert_eq!(result, "PATH-SHA");
    }

    #[test]
    fn is_link_href_accepts_http_and_https() {
        assert!(is_link_href("http://example.com"));
        assert!(is_link_href("https://example.com/x"));
    }

    #[test]
    fn is_link_href_accepts_root_relative() {
        assert!(is_link_href("/git/dep.git/commit/?id=abc"));
        assert!(is_link_href("/"));
    }

    #[test]
    fn is_link_href_rejects_protocol_relative() {
        assert!(!is_link_href("//evil.com/x"));
        assert!(!is_link_href("/\\evil.com/x"));
    }

    #[test]
    fn is_link_href_rejects_relative_and_other_schemes() {
        assert!(!is_link_href("./?repo=%s&page=commit&id=%s"));
        assert!(!is_link_href("../x"));
        assert!(!is_link_href("javascript:alert(1)"));
        assert!(!is_link_href("data:text/html,x"));
        assert!(!is_link_href("git@host:a/b.git"));
        assert!(!is_link_href("ssh://host/a"));
        assert!(!is_link_href("HTTPS://X"));
        assert!(!is_link_href(""));
    }

    #[test]
    fn parse_gitmodules_canonical_two_stanzas() {
        let content = r#"
[submodule "dep"]
	path = vendor/dep
	url = https://example.com/dep.git
[submodule "lib"]
	path = third_party/lib
	url = https://example.com/lib.git
"#;
        assert_eq!(
            parse_gitmodules(content),
            vec![
                (
                    "vendor/dep".to_owned(),
                    "https://example.com/dep.git".to_owned()
                ),
                (
                    "third_party/lib".to_owned(),
                    "https://example.com/lib.git".to_owned()
                ),
            ]
        );
    }

    #[test]
    fn parse_gitmodules_url_before_path() {
        let content = r#"
[submodule "dep"]
	url = https://example.com/dep.git
	path = vendor/dep
"#;
        assert_eq!(
            parse_gitmodules(content),
            vec![(
                "vendor/dep".to_owned(),
                "https://example.com/dep.git".to_owned()
            )]
        );
    }

    #[test]
    fn parse_gitmodules_quoted_value() {
        let content = r#"
[submodule "dep"]
	path = "vendor/dep"
	url = "https://example.com/a\"b.git"
"#;
        assert_eq!(
            parse_gitmodules(content),
            vec![(
                "vendor/dep".to_owned(),
                "https://example.com/a\"b.git".to_owned()
            )]
        );
    }

    #[test]
    fn parse_gitmodules_full_line_and_inline_comments() {
        let content = r#"
# a leading comment
[submodule "dep"]
	; another comment style
	path = vendor/dep # inline comment
	url = https://example.com/dep.git ; also inline
"#;
        assert_eq!(
            parse_gitmodules(content),
            vec![(
                "vendor/dep".to_owned(),
                "https://example.com/dep.git".to_owned()
            )]
        );
    }

    #[test]
    fn parse_gitmodules_tabs_and_crlf() {
        let content =
            "[submodule \"dep\"]\r\n\tpath = vendor/dep\r\n\turl = https://example.com/dep.git\r\n";
        assert_eq!(
            parse_gitmodules(content),
            vec![(
                "vendor/dep".to_owned(),
                "https://example.com/dep.git".to_owned()
            )]
        );
    }

    #[test]
    fn parse_gitmodules_unrelated_section_in_between() {
        let content = r#"
[submodule "dep"]
	path = vendor/dep
[core]
	filemode = true
[submodule "lib"]
	path = third_party/lib
	url = https://example.com/lib.git
"#;
        assert_eq!(
            parse_gitmodules(content),
            vec![(
                "third_party/lib".to_owned(),
                "https://example.com/lib.git".to_owned()
            )]
        );
    }

    #[test]
    fn parse_gitmodules_stanza_with_only_path_or_only_url_is_dropped() {
        let content = r#"
[submodule "path-only"]
	path = vendor/a
[submodule "url-only"]
	url = https://example.com/b.git
"#;
        assert_eq!(parse_gitmodules(content), Vec::<(String, String)>::new());
    }

    #[test]
    fn parse_gitmodules_name_may_differ_from_path() {
        let content = r#"
[submodule "totally-different-name"]
	path = vendor/dep
	url = https://example.com/dep.git
"#;
        assert_eq!(
            parse_gitmodules(content),
            vec![(
                "vendor/dep".to_owned(),
                "https://example.com/dep.git".to_owned()
            )]
        );
    }

    #[test]
    fn parse_gitmodules_duplicate_path_first_wins() {
        let content = r#"
[submodule "a"]
	path = vendor/dep
	url = https://example.com/first.git
[submodule "b"]
	path = vendor/dep
	url = https://example.com/second.git
"#;
        assert_eq!(
            parse_gitmodules(content),
            vec![(
                "vendor/dep".to_owned(),
                "https://example.com/first.git".to_owned()
            )]
        );
    }

    #[test]
    fn parse_gitmodules_duplicate_key_within_a_stanza_last_wins() {
        let content = r#"
[submodule "dep"]
	path = vendor/dep
	url = https://example.com/first.git
	url = https://example.com/second.git
"#;
        assert_eq!(
            parse_gitmodules(content),
            vec![(
                "vendor/dep".to_owned(),
                "https://example.com/second.git".to_owned()
            )]
        );
    }

    #[test]
    fn parse_gitmodules_trailing_slash_on_path_is_trimmed() {
        let content = r#"
[submodule "dep"]
	path = vendor/dep/
	url = https://example.com/dep.git
"#;
        assert_eq!(
            parse_gitmodules(content),
            vec![(
                "vendor/dep".to_owned(),
                "https://example.com/dep.git".to_owned()
            )]
        );
    }
}
