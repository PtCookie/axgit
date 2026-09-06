import { afterEach, describe, expect, it } from "vitest";
import { cleanup, render } from "vitest-browser-react";
import { page } from "vitest/browser";

import SplitHunk from "@/components/repo/diff/SplitHunk";
import type { Hunk } from "@/lib/api/schemas";

afterEach(cleanup);

describe("SplitHunk", () => {
  it("renders both content columns for a context line", async () => {
    const hunk: Hunk = {
      header: "@@ -1,2 +1,2 @@",
      old_start: 1,
      old_lines: 2,
      new_start: 1,
      new_lines: 2,
      lines: [{ origin: " ", content: "unchanged", old_lineno: 1, new_lineno: 1 }],
    };
    await render(<SplitHunk hunk={hunk} />);

    await expect.element(page.getByText("unchanged").first()).toBeVisible();
    const cells = page.getByText("unchanged").elements();
    expect(cells.length).toBe(2);
  });

  it("renders a filler cell (no content) on the side missing a line", async () => {
    const hunk: Hunk = {
      header: "@@ -1,3 +1,1 @@",
      old_start: 1,
      old_lines: 3,
      new_start: 1,
      new_lines: 1,
      lines: [
        { origin: "-", content: "removed one", old_lineno: 1, new_lineno: null },
        { origin: "-", content: "removed two", old_lineno: 2, new_lineno: null },
      ],
    };
    await render(<SplitHunk hunk={hunk} />);

    await expect.element(page.getByText("removed one")).toBeVisible();
    await expect.element(page.getByText("removed two")).toBeVisible();
    // Both rows exist and each has exactly one populated content cell (the
    // old side) — the new-side cell is present but empty (a filler), not
    // absent, so the two columns stay aligned.
    const rows = document.querySelectorAll("tbody tr");
    // 1 header row + 2 change rows.
    expect(rows.length).toBe(3);
  });

  it("highlights only the changed portion of a paired line via intra-line spans", async () => {
    const hunk: Hunk = {
      header: "@@ -1,1 +1,1 @@",
      old_start: 1,
      old_lines: 1,
      new_start: 1,
      new_lines: 1,
      lines: [
        { origin: "-", content: "let value = oldName;", old_lineno: 1, new_lineno: null },
        { origin: "+", content: "let value = newName;", old_lineno: null, new_lineno: 1 },
      ],
    };
    await render(<SplitHunk hunk={hunk} />);

    // Common prefix ("let value = ") and suffix ("Name;") are trimmed off by
    // tier 1 before word-diffing ever runs, so the highlighted span is the
    // precise "old"/"new" difference, not the whole "oldName"/"newName" word.
    await expect.element(page.getByText("old", { exact: true })).toBeVisible();
    await expect.element(page.getByText("new", { exact: true })).toBeVisible();
  });
});
