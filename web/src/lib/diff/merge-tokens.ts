import type { Segment } from "@/lib/diff/intraline";
import type { HighlightedLine } from "@/lib/format/highlight";

/** One rendered fragment of a split-view line: Shiki's foreground `style`
 *  plus whether this span falls inside an intra-line change — the two
 *  independently-computed segmentations of the same string, sliced at each
 *  other's boundaries so neither has to know about the other. */
export interface MergedFragment {
  text: string;
  style?: Record<string, string>;
  changed: boolean;
}

/**
 * Splits a line's Shiki tokens and its intra-line diff segments at each
 * other's boundaries, producing one fragment list that carries both:
 * `style` (foreground color, from Shiki) and `changed` (background, from
 * the intra-line diff). Rendering both layers from one pass of fragments —
 * rather than nesting one segmentation's `<span>`s inside the other's —
 * avoids having to reconcile two independently-computed cut points.
 *
 * Assumes `tokens` and `segments` cover the exact same underlying string
 * (true here: both are derived from the same `Line.content`).
 */
export function mergeTokensWithSegments(tokens: HighlightedLine, segments: Segment[]): MergedFragment[] {
  const merged: MergedFragment[] = [];
  let tokenIndex = 0;
  let tokenOffset = 0;
  let segmentIndex = 0;
  let segmentOffset = 0;

  while (tokenIndex < tokens.length && segmentIndex < segments.length) {
    const token = tokens[tokenIndex];
    const segment = segments[segmentIndex];
    const tokenRemaining = token.content.length - tokenOffset;
    const segmentRemaining = segment.text.length - segmentOffset;
    const take = Math.min(tokenRemaining, segmentRemaining);

    merged.push({
      text: token.content.slice(tokenOffset, tokenOffset + take),
      style: token.style,
      changed: segment.changed,
    });

    tokenOffset += take;
    segmentOffset += take;
    if (tokenOffset >= token.content.length) {
      tokenIndex++;
      tokenOffset = 0;
    }
    if (segmentOffset >= segment.text.length) {
      segmentIndex++;
      segmentOffset = 0;
    }
  }

  return merged;
}
