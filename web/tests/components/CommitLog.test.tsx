import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render } from "vitest-browser-react";
import { page } from "vitest/browser";

import CommitLog from "@/components/repo/CommitLog";
import { ApiError } from "@/lib/api/client";
import { listCommits } from "@/lib/api/repos";
import type { CommitsPage } from "@/lib/api/schemas";

vi.mock("@/lib/api/repos", () => ({
  listCommits: vi.fn(),
}));

const mockedListCommits = vi.mocked(listCommits);

const PAGE: CommitsPage = {
  commits: [
    {
      sha: "abc123def456abc123def456abc123def456abc",
      summary: "fix: update readme",
      author: { name: "Ada Lovelace", email_hash: "deadbeef" },
      authored_at: "2026-07-24T13:06:00+09:00",
      parents: [],
    },
  ],
  next_cursor: null,
};

describe("CommitLog", () => {
  beforeEach(() => {
    mockedListCommits.mockReset();
  });

  afterEach(() => {
    cleanup();
  });

  it("lists commits", async () => {
    mockedListCommits.mockResolvedValue(PAGE);
    render(<CommitLog repo="git-compose" />);

    await expect.element(page.getByText("fix: update readme")).toBeVisible();
    await expect.element(page.getByText("Ada Lovelace")).toBeVisible();
    expect(mockedListCommits).toHaveBeenCalledWith("git-compose", {
      ref: undefined,
      path: undefined,
      cursor: undefined,
    });
  });

  it("links each summary to the commit page", async () => {
    mockedListCommits.mockResolvedValue(PAGE);
    render(<CommitLog repo="git-compose" />);

    const link = page.getByRole("link", { name: "fix: update readme" });
    await expect.element(link).toHaveAttribute("href", "/git-compose/commit/abc123def456abc123def456abc123def456abc");
  });

  it("falls back to a placeholder for a null summary", async () => {
    mockedListCommits.mockResolvedValue({
      commits: [{ ...PAGE.commits[0], summary: null, authored_at: null }],
      next_cursor: null,
    });
    render(<CommitLog repo="git-compose" />);

    await expect.element(page.getByText("(no commit message)")).toBeVisible();
  });

  it("shows an empty-repository message", async () => {
    mockedListCommits.mockResolvedValue({ commits: [], next_cursor: null });
    render(<CommitLog repo="git-compose" />);

    await expect.element(page.getByText("No commits yet.")).toBeVisible();
  });

  it("shows an error message when the request fails", async () => {
    mockedListCommits.mockRejectedValue(new ApiError("internal", "boom", 500));
    render(<CommitLog repo="git-compose" />);

    await expect.element(page.getByRole("alert")).toHaveTextContent("boom");
  });

  it("renders an Older link that carries the cursor and preserves ref/path", async () => {
    mockedListCommits.mockResolvedValue({ ...PAGE, next_cursor: "def456" });
    render(<CommitLog repo="git-compose" ref="main" path="src" />);

    const older = page.getByRole("link", { name: "Older →" });
    await expect.element(older).toBeVisible();
    const href = await older.element().getAttribute("href");
    const url = new URL(href ?? "", "http://localhost");
    expect(url.pathname).toBe("/git-compose/log");
    expect(url.searchParams.get("cursor")).toBe("def456");
    expect(url.searchParams.get("ref")).toBe("main");
    expect(url.searchParams.get("path")).toBe("src");
  });

  it("does not render an Older link on the last page", async () => {
    mockedListCommits.mockResolvedValue(PAGE);
    render(<CommitLog repo="git-compose" />);

    await expect.element(page.getByText("fix: update readme")).toBeVisible();
    expect(page.getByRole("link", { name: "Older →" }).elements().length).toBe(0);
  });

  it("shows a path filter banner with a clear-filter link", async () => {
    mockedListCommits.mockResolvedValue(PAGE);
    render(<CommitLog repo="git-compose" path="src/main.rs" />);

    await expect.element(page.getByText("src/main.rs")).toBeVisible();
    const clear = page.getByRole("link", { name: "clear filter" });
    await expect.element(clear).toHaveAttribute("href", "/git-compose/log");
  });

  it("renders one graph cell per commit", async () => {
    const twoCommits: CommitsPage = {
      commits: [
        PAGE.commits[0],
        {
          sha: "def456abc123def456abc123def456abc123def",
          summary: "feat: add graph",
          author: { name: "Ada Lovelace", email_hash: "deadbeef" },
          authored_at: "2026-07-23T13:06:00+09:00",
          parents: [],
        },
      ],
      next_cursor: null,
    };
    mockedListCommits.mockResolvedValue(twoCommits);
    render(<CommitLog repo="git-compose" />);

    await expect.element(page.getByText("fix: update readme")).toBeVisible();
    expect(page.getByTestId("commit-graph").elements().length).toBe(2);
  });

  it("marks a merge commit's node as a merge", async () => {
    mockedListCommits.mockResolvedValue({
      commits: [{ ...PAGE.commits[0], parents: ["p1", "p2"] }],
      next_cursor: null,
    });
    render(<CommitLog repo="git-compose" />);

    await expect.element(page.getByText("fix: update readme")).toBeVisible();
    expect(page.getByTestId("commit-graph").element().querySelectorAll("[data-merge='true']").length).toBe(1);
  });

  it("hides the graph column when a path filter is active", async () => {
    mockedListCommits.mockResolvedValue(PAGE);
    render(<CommitLog repo="git-compose" path="src/main.rs" />);

    await expect.element(page.getByText("fix: update readme")).toBeVisible();
    expect(page.getByTestId("commit-graph").elements().length).toBe(0);
  });
});
