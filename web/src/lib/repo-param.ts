/**
 * The `{repo}` segment of a page URL, decoded. Used by data islands mounted
 * on a `/{repo}/…` shell (built under a placeholder param, see
 * `lib/shell.ts`) to recover the real repository name from `location`.
 */
export function repoFromPathname(pathname: string): string {
  const [segment] = pathname.split("/").filter((part) => part.length > 0);
  if (segment === undefined) return "";
  try {
    return decodeURIComponent(segment);
  } catch {
    // A malformed escape means a hand-typed URL — show it verbatim rather
    // than throwing.
    return segment;
  }
}
