import type { FileDiff, Line } from "@/lib/api/schemas";
import { highlightCode, type HighlightedLine } from "@/lib/format/highlight";

interface SideText {
  text: string;
  /** `lineRefs[i]` is the `Line` object whose content became line `i` of
   *  `text` — the mapping back from a tokenized line to the diff line it
   *  belongs to, since only a subset of the real file's lines are shown. */
  lineRefs: Line[];
}

/** Concatenates the lines actually shown for one side of a file's diff —
 *  context + deletion for `"old"`, context + addition for `"new"` — across
 *  every hunk, in order. */
function buildSideText(file: FileDiff, side: "old" | "new"): SideText {
  const parts: string[] = [];
  const lineRefs: Line[] = [];
  for (const hunk of file.hunks) {
    for (const line of hunk.lines) {
      const include = side === "old" ? line.origin !== "+" : line.origin !== "-";
      if (include) {
        parts.push(line.content);
        lineRefs.push(line);
      }
    }
  }
  return { text: parts.join("\n"), lineRefs };
}

/**
 * Highlights a file's diff by reconstructing each side's shown lines into
 * one string per side and tokenizing it once (`highlightCode`, same
 * threshold/language rules as `BlobView`), rather than tokenizing every line
 * in isolation — a multi-line construct (an unterminated string, a block
 * comment) is far less likely to be mis-highlighted with more surrounding
 * context. Only the lines actually present in the diff are included, so a
 * hunk boundary can still land mid-construct; that residual inaccuracy is
 * the accepted cost of not fetching and tokenizing the full blob.
 *
 * Returns a `Line` object identity → token array map covering both sides —
 * a context `Line` is literally the same object on both sides
 * (`pair-lines.ts`), so it only ever needs one entry — or `null` if neither
 * side's language is supported (`file.binary`, or an extension `highlight.ts`
 * doesn't recognize).
 */
export async function highlightFileDiff(file: FileDiff): Promise<Map<Line, HighlightedLine> | null> {
  if (file.binary) return null;

  const oldSide = buildSideText(file, "old");
  const newSide = buildSideText(file, "new");
  const [oldTokens, newTokens] = await Promise.all([
    oldSide.lineRefs.length > 0 ? highlightCode(oldSide.text, file.old_path ?? file.path) : Promise.resolve(null),
    newSide.lineRefs.length > 0 ? highlightCode(newSide.text, file.path) : Promise.resolve(null),
  ]);

  if (!oldTokens && !newTokens) return null;

  const map = new Map<Line, HighlightedLine>();
  oldTokens?.forEach((tokens, index) => {
    const line = oldSide.lineRefs[index];
    if (line) map.set(line, tokens);
  });
  newTokens?.forEach((tokens, index) => {
    const line = newSide.lineRefs[index];
    if (line) map.set(line, tokens);
  });
  return map;
}
