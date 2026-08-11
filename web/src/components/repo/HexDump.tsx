import { useEffect, useState } from "react";

import { fetchRawBytes } from "@/lib/api/repos";
import { HEX_BYTES_PER_ROW, HEX_DUMP_LIMIT, hexRows } from "@/lib/format/hex";
import { formatSize } from "@/lib/format/size";

type State = { status: "loading" } | { status: "error" } | { status: "data"; bytes: Uint8Array };

interface HexDumpProps {
  /** A `rawUrl`/`objectRawUrl` link — fetched directly, not through
   *  `apiFetch` (the response isn't JSON). */
  url: string;
  /** The blob's full size (`BlobInfo.size`/`ObjectBlob.size`), used only for
   *  the truncation note — independent of how many bytes actually arrive. */
  size: number;
}

/** Joins one row's hex byte strings with an extra space every 8 bytes
 *  (`xxd`'s grouping), left-aligned — a short final row is shorter than a
 *  full one, which the caller pads for column alignment. */
function joinHexBytes(bytes: string[]): string {
  return bytes.map((byte, index) => (index > 0 && index % 8 === 0 ? ` ${byte}` : byte)).join(" ");
}

/** Width (in characters) of a fully padded row, used to pad a short final
 *  row so the ASCII column still lines up underneath it. */
const FULL_ROW_WIDTH = joinHexBytes(Array<string>(HEX_BYTES_PER_ROW).fill("00")).length;

/** Binary blob content as a hex dump (docs/DECISIONS.md #58, cgit's
 *  `<table class='bin-blob'>`). Self-fetches the raw endpoint — the JSON
 *  blob/object responses never carry binary content inline. Rendered
 *  immediately in place of the old "Binary file not shown" notice, matching
 *  cgit; a failed fetch falls back to that same notice rather than leaving a
 *  blank page. */
export default function HexDump({ url, size }: HexDumpProps) {
  const [state, setState] = useState<State>({ status: "loading" });

  useEffect(() => {
    let cancelled = false;

    fetchRawBytes(url)
      .then((bytes) => {
        if (!cancelled) {
          setState({ status: "data", bytes });
        }
      })
      .catch(() => {
        if (!cancelled) {
          setState({ status: "error" });
        }
      });

    return () => {
      cancelled = true;
    };
  }, [url]);

  if (state.status === "loading") {
    return <p className="text-muted-foreground text-sm">Loading hex dump…</p>;
  }

  if (state.status === "error") {
    return (
      <p className="text-muted-foreground text-sm">
        Binary file not shown —{" "}
        <a className="underline" href={url}>
          view raw
        </a>
        .
      </p>
    );
  }

  const rows = hexRows(state.bytes);
  const truncated = state.bytes.length > HEX_DUMP_LIMIT;

  return (
    <div className="space-y-2">
      <div className="border-border overflow-x-auto rounded-md border">
        <table className="w-full border-collapse font-mono text-sm">
          <tbody>
            {rows.map((row) => (
              <tr key={row.offset}>
                <td className="text-muted-foreground w-20 shrink-0 px-2 text-right align-top select-none">
                  {row.offset.toString(16).padStart(8, "0")}
                </td>
                <td className="px-2 align-top whitespace-pre">{joinHexBytes(row.bytes).padEnd(FULL_ROW_WIDTH)}</td>
                <td className="text-muted-foreground px-2 align-top whitespace-pre">{row.ascii}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      {truncated && (
        <p className="text-muted-foreground text-sm">
          Showing the first {formatSize(HEX_DUMP_LIMIT)} of {formatSize(size)} —{" "}
          <a className="underline" href={url}>
            view raw
          </a>
          .
        </p>
      )}
    </div>
  );
}
