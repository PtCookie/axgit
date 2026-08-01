/**
 * Relative-URL handling for README rendering (`ReadmeView.tsx`,
 * DECISIONS.md #21). A README's `<a href>`/`<img src>` are written relative
 * to the repository's file tree (e.g. `./docs/x.md`, `images/logo.png`), but
 * the browser resolves them against the *page* URL (`/{repo}`) — so every
 * relative link/image would 404 unless rewritten to the matching repository
 * URL first.
 */

/** True for a URL that already points somewhere outside the repository's
 *  own tree and must be left untouched: absolute (`scheme:`), protocol-
 *  relative (`//host/...`), a same-page anchor (`#...`), or empty. */
export function isExternalUrl(url: string): boolean {
  if (url === "") return false;
  if (url.startsWith("#")) return true;
  if (url.startsWith("//")) return true;
  // A leading `scheme:` per RFC 3986 (letter, then letters/digits/+/-/.) —
  // covers `http:`, `https:`, `mailto:`, `data:`, etc. A Windows-style
  // drive letter or a bare `a:b` path segment doesn't occur in git trees.
  return /^[a-z][a-z0-9+.-]*:/i.test(url);
}

/** Resolves a README-relative `url` against `base` (the README's own
 *  directory within the repository tree — currently always `""`, since the
 *  readme endpoint only searches the root tree, but this stays honest about
 *  where it would generalize) into a repository-tree path with no leading
 *  `/`, `.`, or `..` segments.
 *
 *  Returns `null` when the reference escapes the repository root (e.g.
 *  `../../etc`) or is itself an external/absolute URL — callers should
 *  leave those untouched rather than passing them here. */
export function resolveRepoPath(base: string, url: string): string | null {
  if (isExternalUrl(url)) return null;

  // A repo-root-relative link (`/docs/x.md`) — treat the leading `/` as the
  // tree root, same effect as an empty `base`.
  const fromRoot = url.startsWith("/");
  const baseSegments = fromRoot ? [] : base.split("/").filter((segment) => segment.length > 0);

  // Strip a query/hash suffix before splitting — no query/hash exists in a
  // git tree path, but a README may still write `img.png?raw=true`.
  const path = url.split(/[?#]/, 1)[0] ?? "";
  const segments = fromRoot ? path.slice(1).split("/") : path.split("/");

  const resolved = [...baseSegments]; // base is already a directory (no filename component)

  for (const segment of segments) {
    if (segment === "" || segment === ".") continue;
    if (segment === "..") {
      if (resolved.length === 0) return null; // escapes the repository root
      resolved.pop();
      continue;
    }
    resolved.push(segment);
  }

  return resolved.join("/");
}
