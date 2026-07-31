/** Escapes one path segment for use in an API (or web) URL. */
export function encodeSegment(value: string): string {
  return encodeURIComponent(value);
}
