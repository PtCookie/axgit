import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render } from "vitest-browser-react";
import { page } from "vitest/browser";

import DiffView from "@/components/repo/DiffView";
import { ApiError } from "@/lib/api/client";
import { getDiff, getRefs, getRepo } from "@/lib/api/repos";
import type { RefsInfo, RepoSummary, RevDiff } from "@/lib/api/schemas";

vi.mock("@/lib/api/repos", () => ({
  getDiff: vi.fn(),
  getRefs: vi.fn(),
  getRepo: vi.fn(),
  // Link-only builders (never fetched) — real implementations.
  compareRawDiffUrl: (name: string, params: Record<string, unknown>) =>
    `/api/v1/repos/${name}/rawdiff?${new URLSearchParams(params as Record<string, string>).toString()}`,
  comparePatchUrl: (name: string, params: Record<string, unknown>) =>
    `/api/v1/repos/${name}/patch?${new URLSearchParams(params as Record<string, string>).toString()}`,
}));

const mockedGetDiff = vi.mocked(getDiff);
const mockedGetRefs = vi.mocked(getRefs);
const mockedGetRepo = vi.mocked(getRepo);

const NO_REFS: RefsInfo = { branches: [], remote_branches: [], tags: [] };

const SUMMARY: RepoSummary = {
  name: "git-compose",
  section: "infra",
  owner: "PtCookie",
  description: "Compose project of Git server",
  default_branch: "main",
  last_modified: "2026-07-24T13:06:00+09:00",
  head: "abc123def456",
  branch_count: 1,
  tag_count: 1,
  clone_url: "git@git.ptcookie.net:git-compose.git",
};

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
    mockedGetRepo.mockReset();
    // Powers the idle `to` prefill only — most tests don't care about the
    // value, so default it here and override per test where it matters
    // (same convention as RefsView.test.tsx's default-branch mock).
    mockedGetRepo.mockResolvedValue(SUMMARY);
  });

  afterEach(() => {
    cleanup();
  });

  it("prefills `to` with the default branch and names it in the idle message", async () => {
    render(<DiffView repo="git-compose" />);

    await expect.element(page.getByText("Pick a revision to compare against main.")).toBeVisible();
    await expect.element(page.getByLabelText("Compare to revision")).toHaveValue("main");
    expect(mockedGetDiff).not.toHaveBeenCalled();
  });

  it("shows the generic idle message and leaves `to` empty when the default-branch fetch fails", async () => {
    mockedGetRepo.mockRejectedValue(new ApiError("internal", "boom", 500));
    render(<DiffView repo="git-compose" />);

    await expect.element(page.getByText("Pick two revisions to compare.")).toBeVisible();
    await expect.element(page.getByLabelText("Compare to revision")).toHaveValue("");
    expect(mockedGetDiff).not.toHaveBeenCalled();
  });

  it("does not fetch the default branch once a comparison is already given", async () => {
    mockedGetDiff.mockResolvedValue(REV_DIFF);
    render(<DiffView repo="git-compose" from={FROM_SHA} to={TO_SHA} />);

    await expect.element(page.getByText("one")).toBeVisible();
    await expect.element(page.getByLabelText("Compare to revision")).toHaveValue(TO_SHA);
    expect(mockedGetRepo).not.toHaveBeenCalled();
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
      remote_branches: [],
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

  it("requests stat=1 without context/ignorews and skips hunk rendering in stat-only mode", async () => {
    mockedGetDiff.mockResolvedValue({ ...REV_DIFF, files: [] });
    render(<DiffView repo="git-compose" from={FROM_SHA} to={TO_SHA} view="stat" context={10} ignorews={true} />);

    await expect.element(page.getByText("a.txt").first()).toBeVisible();
    expect(mockedGetDiff).toHaveBeenCalledWith("git-compose", {
      from: FROM_SHA,
      to: TO_SHA,
      path: undefined,
      stat: 1,
    });
    expect(page.getByText("one").elements().length).toBe(0);
  });

  it("links a stat row to that file's own single-file comparison", async () => {
    mockedGetDiff.mockResolvedValue({ ...REV_DIFF, files: [] });
    render(<DiffView repo="git-compose" from={FROM_SHA} to={TO_SHA} view="stat" />);

    await expect
      .element(page.getByRole("link", { name: "a.txt" }))
      .toHaveAttribute("href", `/git-compose/diff?from=${FROM_SHA}&to=${TO_SHA}&path=a.txt`);
  });

  it("shows a path banner with a link back to the full comparison when path is set", async () => {
    mockedGetDiff.mockResolvedValue(REV_DIFF);
    render(<DiffView repo="git-compose" from={FROM_SHA} to={TO_SHA} path="a.txt" />);

    await expect.element(page.getByText("Showing only")).toBeVisible();
    await expect
      .element(page.getByRole("link", { name: "Show all files" }))
      .toHaveAttribute("href", `/git-compose/diff?from=${FROM_SHA}&to=${TO_SHA}`);
    expect(mockedGetDiff).toHaveBeenCalledWith("git-compose", { from: FROM_SHA, to: TO_SHA, path: "a.txt" });
  });
});
