import { describe, expect, it } from "vitest";

import { HEX_BYTES_PER_ROW, HEX_DUMP_LIMIT, hexRows } from "@/lib/format/hex";

describe("hexRows", () => {
  it("returns no rows for empty input", () => {
    expect(hexRows(new Uint8Array())).toEqual([]);
  });

  it("splits a full row and reports offsets advancing by HEX_BYTES_PER_ROW", () => {
    const bytes = Uint8Array.from({ length: HEX_BYTES_PER_ROW * 2 }, (_, i) => i);
    const rows = hexRows(bytes);

    expect(rows).toHaveLength(2);
    expect(rows[0].offset).toBe(0);
    expect(rows[0].bytes).toHaveLength(HEX_BYTES_PER_ROW);
    expect(rows[1].offset).toBe(HEX_BYTES_PER_ROW);
  });

  it("leaves a short final row unpadded", () => {
    const bytes = Uint8Array.from([0x41, 0x42, 0x43]);
    const rows = hexRows(bytes);

    expect(rows).toHaveLength(1);
    expect(rows[0].bytes).toEqual(["41", "42", "43"]);
    expect(rows[0].ascii).toBe("ABC");
  });

  it("formats bytes as 2-char lowercase hex", () => {
    const rows = hexRows(Uint8Array.from([0x0a, 0xff, 0x41]));
    expect(rows[0].bytes).toEqual(["0a", "ff", "41"]);
  });

  it("maps printable ASCII to itself and everything else to '.'", () => {
    // 'A' (printable), NUL and DEL-adjacent control bytes, a high byte.
    const rows = hexRows(Uint8Array.from([0x41, 0x00, 0x1f, 0x7f, 0x80]));
    expect(rows[0].ascii).toBe("A....");
  });

  it("maps the full printable range (0x20-0x7e) to characters", () => {
    const rows = hexRows(Uint8Array.from([0x20, 0x7e]));
    expect(rows[0].ascii).toBe(" ~");
  });

  it("truncates at HEX_DUMP_LIMIT, dropping anything past it", () => {
    const bytes = new Uint8Array(HEX_DUMP_LIMIT + HEX_BYTES_PER_ROW);
    const rows = hexRows(bytes);

    const totalBytes = rows.reduce((sum, row) => sum + row.bytes.length, 0);
    expect(totalBytes).toBe(HEX_DUMP_LIMIT);
  });
});
