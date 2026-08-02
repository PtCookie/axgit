/**
 * cgit-style URL compatibility redirects (docs/DECISIONS.md #35).
 *
 * Mirrors `api/src/cgit_compat.rs::redirect_for` — the two must change
 * together, same as `shellFor` mirrors `api/src/shell.rs::shell_for`. Used
 * only by `astro.config.mjs`'s dev-server middleware, ahead of `shellFor`,
 * so `astro dev` matches the production static fallback (`api/src/routes.rs`).
 *
 * Query values are never percent-decoded: matching/validation happens on the
 * raw (still-encoded) text, and anything copied into the redirect target is
 * copied verbatim from the request — see the Rust module's doc comment for
 * the full rationale.
 */

/** cgit's `id=` is always a full or abbreviated hex object id. */
function looksLikeSha(value: string): boolean {
  return value.length >= 4 && value.length <= 64 && /^[0-9a-fA-F]+$/.test(value);
}

/** Finds `key=value` in a raw (still percent-encoded) query string. */
function queryValue(query: string, key: string): string | undefined {
  for (const pair of query.split("&")) {
    const eq = pair.indexOf("=");
    if (eq === -1) continue;
    if (pair.slice(0, eq) === key) return pair.slice(eq + 1);
  }
  return undefined;
}

/**
 * Returns the absolute path (+ query) to redirect a cgit-shaped request to,
 * or `null` if `pathname`/`query` doesn't match a recognized shape —
 * including when it already *is* a valid axgit route, so redirecting would
 * fight or loop with `shellFor`.
 *
 * `query` is the raw query string with no leading `?` (or `""`), matching
 * how `astro.config.mjs`'s middleware already splits `req.url`.
 */
export function redirectFor(pathname: string, query: string): string | null {
  const segments = pathname.split("/").filter((segment) => segment.length > 0);
  if (segments.length === 0) return null;

  const [repoRaw, ...rest] = segments;
  const stripped = repoRaw.endsWith(".git") && repoRaw.length > 4 ? repoRaw.slice(0, -4) : undefined;
  const gitStripped = stripped !== undefined;
  const repo = stripped ?? repoRaw;

  const sha = queryValue(query, "id");
  const validSha = sha !== undefined && looksLikeSha(sha) ? sha : undefined;
  const refName = queryValue(query, "h");

  const isCommitOrDiff = rest.length === 1 && (rest[0] === "commit" || rest[0] === "diff");
  const isLog = rest.length === 1 && rest[0] === "log";
  const isRefs = rest.length === 1 && rest[0] === "refs";

  // cgit's changeset/diff/log query shapes redirect regardless of `.git` —
  // `/{repo}/commit` (2 segments) is never a valid axgit shape either way,
  // so there's no native route to conflict with.
  let target: string | null = null;
  if (isCommitOrDiff && validSha !== undefined) {
    target = `/${repo}/commit/${validSha}`;
  } else if (isLog && refName !== undefined) {
    target = `/${repo}/log?ref=${refName}`;
  } else if (gitStripped) {
    // Everything past this point only fires once `.git` was actually
    // stripped — without it, the request already matches a native axgit
    // shape (`shellFor`), and redirecting would just loop back.
    if (rest.length === 0) {
      target = `/${repo}`;
    } else if (isRefs) {
      target = `/${repo}/refs`;
    } else {
      const suffix = rest.join("/");
      target = query.length === 0 ? `/${repo}/${suffix}` : `/${repo}/${suffix}?${query}`;
    }
  }

  if (target === null) return null;

  const current = query.length === 0 ? pathname : `${pathname}?${query}`;
  return target !== current ? target : null;
}
