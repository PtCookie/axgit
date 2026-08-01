import type { ReactNode } from "react";
import { useEffect, useState } from "react";

import { highlightCode, type HighlightedLine } from "@/lib/format/highlight";

/** One entry of an optional attribution gutter (blame). `rowSpan` merges it
 *  down over the following lines that belong to the same commit range —
 *  those lines' own entries must be `null` (covered by the cell above). */
export interface GutterCell {
  node: ReactNode;
  rowSpan: number;
}

interface CodeBlockProps {
  content: string;
  /** Repository-relative path — used only to pick a language by extension. */
  path: string;
  /** Per-line attribution column, indexed 0-based like `content`'s lines.
   *  Omitted entirely by callers that don't need one (`BlobView`) — leaving
   *  this `undefined` renders exactly as before the gutter was added. */
  gutter?: (GutterCell | null)[];
}

/**
 * Line-numbered code viewer. Renders `content` as plain text immediately
 * (no loading flash), then swaps in Shiki's tokenized spans once
 * highlighting finishes — `highlightCode` resolves to `null` for an
 * unsupported language or an over-threshold file, in which case it stays
 * plain. Each row gets an `id="L{n}"` anchor with a matching gutter link,
 * cgit's line-linking UX.
 */
export default function CodeBlock({ content, path, gutter }: CodeBlockProps) {
  const [highlighted, setHighlighted] = useState<HighlightedLine[] | null>(null);
  const lines = content.split("\n");

  useEffect(() => {
    let cancelled = false;

    highlightCode(content, path).then((result) => {
      if (!cancelled) {
        setHighlighted(result);
      }
    });

    return () => {
      cancelled = true;
    };
  }, [content, path]);

  return (
    <div className="border-border overflow-x-auto rounded-md border">
      <table className="shiki-code w-full border-collapse font-mono text-sm">
        <tbody>
          {lines.map((line, index) => {
            const lineNumber = index + 1;
            const anchor = `L${lineNumber}`;
            const tokens = highlighted?.[index];
            const cell = gutter?.[index];
            return (
              <tr key={lineNumber} id={anchor} className="target:bg-primary/10">
                {gutter && cell && (
                  <td
                    rowSpan={cell.rowSpan}
                    className="border-border text-muted-foreground border-r px-2 align-top whitespace-nowrap"
                  >
                    {cell.node}
                  </td>
                )}
                <td className="text-muted-foreground w-10 shrink-0 px-2 text-right align-top select-none">
                  <a href={`#${anchor}`} className="hover:text-foreground">
                    {lineNumber}
                  </a>
                </td>
                <td className="px-2 whitespace-pre">
                  {tokens
                    ? tokens.map((token, tokenIndex) => (
                        // eslint-disable-next-line @eslint-react/no-array-index-key -- tokens have no stable identity
                        <span key={tokenIndex} style={token.style}>
                          {token.content}
                        </span>
                      ))
                    : line}
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}
