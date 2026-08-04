import type { HighlightedLine } from "@/lib/format/highlight";

/** Renders one already-tokenized diff line as inline-styled spans — Shiki's
 *  foreground color only, applied directly as a React `style` prop, never
 *  through `dangerouslySetInnerHTML` (repository content is untrusted input,
 *  same rule `highlight.ts`'s own doc comment states). Shared by `UnifiedHunk`
 *  and `SplitHunk` (whose own tokens+intra-line merge builds a superset,
 *  `MergedFragment`, rendered by its own inline map instead of this). */
export default function TokenSpans({ tokens }: { tokens: HighlightedLine }) {
  return tokens.map((token, index) => (
    // eslint-disable-next-line @eslint-react/no-array-index-key -- tokens have no stable identity
    <span key={index} style={token.style}>
      {token.content}
    </span>
  ));
}
