/** Layout for the binary blob hex dump view (docs/DECISIONS.md #58, cgit
 *  parity: `<table class='bin-blob'>`). Pure function of its input, same
 *  split as `commit-graph.ts` — the component only renders rows this
 *  produces. */

/** 16, not cgit's 32 — `offset + 3*16 hex chars + 16 ascii` fits a narrow
 *  screen without horizontal scroll and matches `xxd`/`hexdump -C`. */
export const HEX_BYTES_PER_ROW = 16;

/** Render cap, independent of the fetch itself: a binary blob is fetched in
 *  full up to the api's own `too_large` boundary (1 MiB), but only the first
 *  64 KiB (4096 rows) is ever turned into DOM rows. */
export const HEX_DUMP_LIMIT = 64 * 1024;

export interface HexRow {
  /** Byte offset of this row's first byte into the (possibly truncated)
   *  slice handed to `hexRows`. */
  offset: number;
  /** One 2-char lowercase hex string per byte. Shorter than
   *  `HEX_BYTES_PER_ROW` on the final row — not padded, the caller aligns. */
  bytes: string[];
  /** Same bytes as printable ASCII (`0x20`-`0x7e`), `.` for everything else,
   *  same length as `bytes`. */
  ascii: string;
}

function toAscii(byte: number): string {
  return byte >= 0x20 && byte <= 0x7e ? String.fromCharCode(byte) : ".";
}

/** Splits `bytes` into `HEX_BYTES_PER_ROW`-wide rows, first truncating to
 *  `HEX_DUMP_LIMIT`. Truncation is the caller's to detect (compare
 *  `bytes.length` against `HEX_DUMP_LIMIT`) — this only ever renders what it's
 *  given, capped. */
export function hexRows(bytes: Uint8Array): HexRow[] {
  const limited = bytes.subarray(0, HEX_DUMP_LIMIT);
  const rows: HexRow[] = [];
  for (let offset = 0; offset < limited.length; offset += HEX_BYTES_PER_ROW) {
    const chunk = limited.subarray(offset, offset + HEX_BYTES_PER_ROW);
    const rowBytes: string[] = [];
    let ascii = "";
    for (const byte of chunk) {
      rowBytes.push(byte.toString(16).padStart(2, "0"));
      ascii += toAscii(byte);
    }
    rows.push({ offset, bytes: rowBytes, ascii });
  }
  return rows;
}
