import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render } from "vitest-browser-react";
import { page } from "vitest/browser";

import CommitView from "@/components/repo/CommitView";
import { ApiError } from "@/lib/api/client";
import { getCommit, getCommitDiff } from "@/lib/api/repos";
import type { CommitDetail, CommitDiff } from "@/lib/api/schemas";

vi.mock("@/lib/api/repos", () => ({
  getCommit: vi.fn(),
  getCommitDiff: vi.fn(),
}));

const mockedGetCommit = vi.mocked(getCommit);
const mockedGetCommitDiff = vi.mocked(getCommitDiff);

const AUTHOR = { name: "Ada Lovelace", email_hash: "deadbeef" };

const DETAIL: CommitDetail = {
  sha: "abc123def456abc123def456abc123def456abc",
  summary: "fix: update a",
  message: "fix: update a\n\nfull body\n",
  author: AUTHOR,
  committer: AUTHOR,
  authored_at: "2026-07-01T14:00:00+09:00",
  committed_at: "2026-07-01T14:00:00+09:00",
  parents: ["parent00000000000000000000000000000000"],
  diffstat: {
    files: [{ path: "a.txt", old_path: null, status: "modified", additions: 3, deletions: 1, binary: false }],
    files_changed: 1,
    total_additions: 3,
    total_deletions: 1,
  },
};

const DIFF: CommitDiff = {
  sha: DETAIL.sha,
  parent: "parent00000000000000000000000000000000",
  truncated: false,
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

describe("CommitView", () => {
  beforeEach(() => {
    mockedGetCommit.mockReset();
    mockedGetCommitDiff.mockReset();
  });

  afterEach(() => {
    cleanup();
  });

  it("shows the commit header, message, diffstat and diff lines", async () => {
    mockedGetCommit.mockResolvedValue(DETAIL);
    mockedGetCommitDiff.mockResolvedValue(DIFF);
    render(<CommitView repo="git-compose" sha={DETAIL.sha} />);

    await expect.element(page.getByRole("heading", { name: "fix: update a" })).toBeVisible();
    await expect.element(page.getByText("full body")).toBeVisible();
    await expect.element(page.getByText(AUTHOR.name).first()).toBeVisible();
    await expect.element(page.getByText("a.txt").first()).toBeVisible();
    await expect.element(page.getByText("one")).toBeVisible();
    await expect.element(page.getByText("two")).toBeVisible();
    await expect.element(page.getByText("three")).toBeVisible();

    expect(mockedGetCommit).toHaveBeenCalledWith("git-compose", DETAIL.sha);
    expect(mockedGetCommitDiff).toHaveBeenCalledWith("git-compose", DETAIL.sha);
  });

  it("shows a binary-file message instead of hunks", async () => {
    mockedGetCommit.mockResolvedValue(DETAIL);
    mockedGetCommitDiff.mockResolvedValue({
      ...DIFF,
      files: [{ ...DIFF.files[0], binary: true, hunks: [] }],
    });
    render(<CommitView repo="git-compose" sha={DETAIL.sha} />);

    await expect.element(page.getByText("Binary file not shown.")).toBeVisible();
  });

  it("shows a truncated-file banner", async () => {
    mockedGetCommit.mockResolvedValue(DETAIL);
    mockedGetCommitDiff.mockResolvedValue({
      ...DIFF,
      files: [{ ...DIFF.files[0], truncated: true }],
    });
    render(<CommitView repo="git-compose" sha={DETAIL.sha} />);

    await expect.element(page.getByText("Diff truncated (1000 lines max).")).toBeVisible();
  });

  it("shows a top-level truncated banner when files were omitted", async () => {
    mockedGetCommit.mockResolvedValue({
      ...DETAIL,
      diffstat: { ...DETAIL.diffstat, files_changed: 400 },
    });
    mockedGetCommitDiff.mockResolvedValue({ ...DIFF, truncated: true });
    render(<CommitView repo="git-compose" sha={DETAIL.sha} />);

    await expect
      .element(page.getByText("Some files were omitted (300 files max) — see the table above for the full file list."))
      .toBeVisible();
  });

  it("handles a root commit (no parents, null diff parent)", async () => {
    mockedGetCommit.mockResolvedValue({ ...DETAIL, parents: [] });
    mockedGetCommitDiff.mockResolvedValue({ ...DIFF, parent: null });
    render(<CommitView repo="git-compose" sha={DETAIL.sha} />);

    await expect.element(page.getByRole("heading", { name: "fix: update a" })).toBeVisible();
    expect(page.getByText("Parents").elements().length).toBe(0);
  });

  it("notes that the diff is against the first parent for a merge commit", async () => {
    mockedGetCommit.mockResolvedValue({
      ...DETAIL,
      parents: ["parent1000000000000000000000000000000", "parent2000000000000000000000000000000"],
    });
    mockedGetCommitDiff.mockResolvedValue(DIFF);
    render(<CommitView repo="git-compose" sha={DETAIL.sha} />);

    await expect
      .element(page.getByText("This is a merge commit — the diff below is shown against the first parent only."))
      .toBeVisible();
  });

  it("shows an error message when the request fails", async () => {
    mockedGetCommit.mockRejectedValue(new ApiError("internal", "boom", 500));
    mockedGetCommitDiff.mockResolvedValue(DIFF);
    render(<CommitView repo="git-compose" sha={DETAIL.sha} />);

    await expect.element(page.getByRole("alert")).toHaveTextContent("boom");
  });
});
