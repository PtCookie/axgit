/** Escapes one path segment for use in an API (or web) URL. */
export function encodeSegment(value: string): string {
  return encodeURIComponent(value);
}

/** Escapes a `/`-separated file path for use in an API (or web) URL,
 *  segment-by-segment — a `/` inside a single path component never appears
 *  (git rejects it in a tree entry name), so splitting on it is safe. */
export function encodePath(path: string): string {
  return path.split("/").map(encodeSegment).join("/");
}
