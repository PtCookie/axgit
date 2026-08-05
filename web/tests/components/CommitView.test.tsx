import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render } from "vitest-browser-react";
import { page } from "vitest/browser";

import CommitView from "@/components/repo/CommitView";
import { ApiError } from "@/lib/api/client";
import { getCommit, getCommitDiff, getRefs } from "@/lib/api/repos";
import type { CommitDetail, CommitDiff, RefsInfo } from "@/lib/api/schemas";

vi.mock("@/lib/api/repos", () => ({
  getCommit: vi.fn(),
  getCommitDiff: vi.fn(),
  getRefs: vi.fn(),
  // Link-only builders (never fetched) — real implementations, so the
  // rendered hrefs reflect the actual URL shape rather than a stub.
  commitPatchUrl: (name: string, sha: string) => `/api/v1/repos/${name}/patch?to=${sha}`,
  commitRawDiffUrl: (name: string, sha: string) => `/api/v1/repos/${name}/rawdiff?to=${sha}`,
}));

const mockedGetCommit = vi.mocked(getCommit);
const mockedGetCommitDiff = vi.mocked(getCommitDiff);
const mockedGetRefs = vi.mocked(getRefs);

const NO_REFS: RefsInfo = { branches: [], tags: [] };

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
    mockedGetRefs.mockReset();
    mockedGetRefs.mockResolvedValue(NO_REFS);
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
    expect(mockedGetCommitDiff).toHaveBeenCalledWith("git-compose", DETAIL.sha, {});
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
      .element(
        page.getByText(
          "This is a merge commit — the diff below is shown against the first parent only. Use the (diff) links above to compare against another parent.",
        ),
      )
      .toBeVisible();
  });

  it("shows an error message when the request fails", async () => {
    mockedGetCommit.mockRejectedValue(new ApiError("internal", "boom", 500));
    mockedGetCommitDiff.mockResolvedValue(DIFF);
    render(<CommitView repo="git-compose" sha={DETAIL.sha} />);

    await expect.element(page.getByRole("alert")).toHaveTextContent("boom");
  });

  it("shows every ref badge pointing at the commit, uncapped", async () => {
    mockedGetCommit.mockResolvedValue(DETAIL);
    mockedGetCommitDiff.mockResolvedValue(DIFF);
    mockedGetRefs.mockResolvedValue({
      branches: [
        { name: "main", target: DETAIL.sha, committed_at: null },
        { name: "dev", target: DETAIL.sha, committed_at: null },
      ],
      tags: [{ name: "v1.0.0", target: DETAIL.sha, annotation: null, tagged_at: null }],
    });
    render(<CommitView repo="git-compose" sha={DETAIL.sha} />);

    await expect.element(page.getByText("main")).toBeVisible();
    await expect.element(page.getByText("dev")).toBeVisible();
    await expect.element(page.getByText("v1.0.0")).toBeVisible();
  });

  it("renders no badges and no error when the refs request fails", async () => {
    mockedGetCommit.mockResolvedValue(DETAIL);
    mockedGetCommitDiff.mockResolvedValue(DIFF);
    mockedGetRefs.mockRejectedValue(new ApiError("internal", "boom", 500));
    render(<CommitView repo="git-compose" sha={DETAIL.sha} />);

    await expect.element(page.getByRole("heading", { name: "fix: update a" })).toBeVisible();
    expect(page.getByRole("alert").elements().length).toBe(0);
  });

  it("forwards context and ignorews to the diff API", async () => {
    mockedGetCommit.mockResolvedValue(DETAIL);
    mockedGetCommitDiff.mockResolvedValue(DIFF);
    render(<CommitView repo="git-compose" sha={DETAIL.sha} context={10} ignorews={true} />);

    await expect.element(page.getByRole("heading", { name: "fix: update a" })).toBeVisible();
    expect(mockedGetCommitDiff).toHaveBeenCalledWith("git-compose", DETAIL.sha, { context: 10, ignorews: 1 });
  });

  it("links each parent to a (diff) comparison against this commit", async () => {
    mockedGetCommit.mockResolvedValue(DETAIL);
    mockedGetCommitDiff.mockResolvedValue(DIFF);
    render(<CommitView repo="git-compose" sha={DETAIL.sha} />);

    const parent = DETAIL.parents[0];
    await expect
      .element(page.getByRole("link", { name: `Diff against parent ${parent.slice(0, 12)}` }))
      .toHaveAttribute("href", `/git-compose/diff?from=${parent}&to=${DETAIL.sha}`);
  });

  it("links to the tree, raw diff, and patch views for this commit", async () => {
    mockedGetCommit.mockResolvedValue(DETAIL);
    mockedGetCommitDiff.mockResolvedValue(DIFF);
    render(<CommitView repo="git-compose" sha={DETAIL.sha} />);

    await expect
      .element(page.getByRole("link", { name: "Browse the tree at this commit" }))
      .toHaveAttribute("href", `/git-compose/tree?ref=${DETAIL.sha}`);
    await expect
      .element(page.getByRole("link", { name: "Raw diff" }))
      .toHaveAttribute("href", `/api/v1/repos/git-compose/rawdiff?to=${DETAIL.sha}`);
    await expect
      .element(page.getByRole("link", { name: "Patch" }))
      .toHaveAttribute("href", `/api/v1/repos/git-compose/patch?to=${DETAIL.sha}`);
  });

  it("skips the diff fetch and hunk rendering entirely in stat-only mode", async () => {
    mockedGetCommit.mockResolvedValue(DETAIL);
    render(<CommitView repo="git-compose" sha={DETAIL.sha} view="stat" />);

    await expect.element(page.getByText("a.txt").first()).toBeVisible();
    expect(mockedGetCommitDiff).not.toHaveBeenCalled();
    expect(page.getByText("one").elements().length).toBe(0);
  });

  it("links a stat row to that file's own single-file diff", async () => {
    mockedGetCommit.mockResolvedValue(DETAIL);
    render(<CommitView repo="git-compose" sha={DETAIL.sha} view="stat" />);

    await expect
      .element(page.getByRole("link", { name: "a.txt" }))
      .toHaveAttribute("href", `/git-compose/commit/${DETAIL.sha}?path=a.txt`);
  });

  it("shows a path banner with a link back to the full diff when path is set", async () => {
    mockedGetCommit.mockResolvedValue(DETAIL);
    mockedGetCommitDiff.mockResolvedValue(DIFF);
    render(<CommitView repo="git-compose" sha={DETAIL.sha} path="a.txt" />);

    await expect.element(page.getByText("Showing only")).toBeVisible();
    await expect.element(page.getByText("a.txt").first()).toBeVisible();
    await expect
      .element(page.getByRole("link", { name: "Show all files" }))
      .toHaveAttribute("href", `/git-compose/commit/${DETAIL.sha}`);
    expect(mockedGetCommitDiff).toHaveBeenCalledWith("git-compose", DETAIL.sha, { path: "a.txt" });
  });
});
