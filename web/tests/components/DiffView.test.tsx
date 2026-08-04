import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render } from "vitest-browser-react";
import { page } from "vitest/browser";

import DiffView from "@/components/repo/DiffView";
import { ApiError } from "@/lib/api/client";
import { getDiff, getRefs } from "@/lib/api/repos";
import type { RefsInfo, RevDiff } from "@/lib/api/schemas";

vi.mock("@/lib/api/repos", () => ({
  getDiff: vi.fn(),
  getRefs: vi.fn(),
  // Link-only builders (never fetched) — real implementations.
  compareRawDiffUrl: (name: string, params: Record<string, unknown>) =>
    `/api/v1/repos/${name}/rawdiff?${new URLSearchParams(params as Record<string, string>).toString()}`,
  comparePatchUrl: (name: string, params: Record<string, unknown>) =>
    `/api/v1/repos/${name}/patch?${new URLSearchParams(params as Record<string, string>).toString()}`,
}));

const mockedGetDiff = vi.mocked(getDiff);
const mockedGetRefs = vi.mocked(getRefs);

const NO_REFS: RefsInfo = { branches: [], tags: [] };

const FROM_SHA = "aaa000111222aaa000111222aaa000111222aaa";
const TO_SHA = "bbb333444555bbb333444555bbb333444555bbb";

const REV_DIFF: RevDiff = {
  from: FROM_SHA,
  to: TO_SHA,
  truncated: false,
  diffstat: {
    files: [{ path: "a.txt", old_path: null, status: "modified", additions: 1, deletions: 1, binary: false }],
    files_changed: 1,
    total_additions: 1,
    total_deletions: 1,
  },
  files: [
    {
      path: "a.txt",
      old_path: null,
      status: "modified",
      additions: 1,
      deletions: 1,
      binary: false,
      truncated: false,
      hunks: [
        {
          header: "@@ -1,2 +1,2 @@",
          old_start: 1,
          old_lines: 2,
          new_start: 1,
          new_lines: 2,
          lines: [
            { origin: " ", content: "one", old_lineno: 1, new_lineno: 1 },
            { origin: "-", content: "two", old_lineno: 2, new_lineno: null },
            { origin: "+", content: "three", old_lineno: null, new_lineno: 2 },
          ],
        },
      ],
    },
  ],
};

describe("DiffView", () => {
  beforeEach(() => {
    mockedGetDiff.mockReset();
    mockedGetRefs.mockReset();
    mockedGetRefs.mockResolvedValue(NO_REFS);
  });

  afterEach(() => {
    cleanup();
  });

  it("shows the revision picker and an idle message when neither side is given", async () => {
    render(<DiffView repo="git-compose" />);

    await expect.element(page.getByText("Pick two revisions to compare.")).toBeVisible();
    expect(mockedGetDiff).not.toHaveBeenCalled();
  });

  it("forwards from/to to the api and renders the resulting diff", async () => {
    mockedGetDiff.mockResolvedValue(REV_DIFF);
    render(<DiffView repo="git-compose" from={FROM_SHA} to={TO_SHA} />);

    await expect.element(page.getByText("one")).toBeVisible();
    await expect.element(page.getByText("two")).toBeVisible();
    await expect.element(page.getByText("three")).toBeVisible();
    expect(mockedGetDiff).toHaveBeenCalledWith("git-compose", {
      from: FROM_SHA,
      to: TO_SHA,
      path: undefined,
    });
  });

  it("fetches as soon as only one side is given", async () => {
    mockedGetDiff.mockResolvedValue(REV_DIFF);
    render(<DiffView repo="git-compose" to={TO_SHA} />);

    await expect.element(page.getByText("one")).toBeVisible();
    expect(mockedGetDiff).toHaveBeenCalledWith("git-compose", { from: undefined, to: TO_SHA, path: undefined });
  });

  it("shows a not-found message for an unknown revision", async () => {
    mockedGetDiff.mockRejectedValue(new ApiError("ref_not_found", "ref 'nope' not found", 404));
    render(<DiffView repo="git-compose" from="nope" to={TO_SHA} />);

    await expect.element(page.getByRole("alert")).toHaveTextContent("Revision not found.");
  });

  it("shows the api error message for other failures", async () => {
    mockedGetDiff.mockRejectedValue(new ApiError("internal", "boom", 500));
    render(<DiffView repo="git-compose" from={FROM_SHA} to={TO_SHA} />);

    await expect.element(page.getByRole("alert")).toHaveTextContent("boom");
  });

  it("populates the revision datalist from getRefs", async () => {
    mockedGetDiff.mockResolvedValue(REV_DIFF);
    mockedGetRefs.mockResolvedValue({
      branches: [{ name: "main", target: TO_SHA, committed_at: null }],
      tags: [{ name: "v1.0.0", target: TO_SHA, annotation: null, tagged_at: null }],
    });
    render(<DiffView repo="git-compose" from={FROM_SHA} to={TO_SHA} />);

    await expect.element(page.getByText("one")).toBeVisible();
    const options = document.querySelectorAll("datalist option");
    const values = Array.from(options).map((option) => option.getAttribute("value"));
    expect(values).toEqual(["main", "v1.0.0"]);
  });

  it("stays functional (no error) when getRefs fails — decoration only", async () => {
    mockedGetDiff.mockResolvedValue(REV_DIFF);
    mockedGetRefs.mockRejectedValue(new ApiError("internal", "boom", 500));
    render(<DiffView repo="git-compose" from={FROM_SHA} to={TO_SHA} />);

    await expect.element(page.getByText("one")).toBeVisible();
    expect(page.getByRole("alert").elements().length).toBe(0);
  });
});
