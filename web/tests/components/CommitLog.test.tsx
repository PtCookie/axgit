import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render } from "vitest-browser-react";
import { page } from "vitest/browser";

import CommitLog from "@/components/repo/CommitLog";
import { ApiError } from "@/lib/api/client";
import { getRefs, listCommits } from "@/lib/api/repos";
import type { CommitsPage, RefsInfo } from "@/lib/api/schemas";

vi.mock("@/lib/api/repos", () => ({
  listCommits: vi.fn(),
  getRefs: vi.fn(),
}));

const mockedListCommits = vi.mocked(listCommits);
const mockedGetRefs = vi.mocked(getRefs);

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

const NO_REFS: RefsInfo = { branches: [], remote_branches: [], tags: [] };

describe("CommitLog", () => {
  beforeEach(() => {
    mockedListCommits.mockReset();
    mockedGetRefs.mockReset();
    mockedGetRefs.mockResolvedValue(NO_REFS);
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
      msg: undefined,
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

  it("shows a badge for a branch pointing at the commit", async () => {
    mockedListCommits.mockResolvedValue(PAGE);
    mockedGetRefs.mockResolvedValue({
      branches: [{ name: "main", target: PAGE.commits[0].sha, committed_at: null }],
      remote_branches: [],
      tags: [],
    });
    render(<CommitLog repo="git-compose" />);

    const badge = page.getByText("main");
    await expect.element(badge).toBeVisible();
    await expect.element(badge.element().closest("a")).toHaveAttribute("href", "/git-compose/log?ref=main");
  });

  it("shows a badge for a tag pointing at the commit", async () => {
    mockedListCommits.mockResolvedValue(PAGE);
    mockedGetRefs.mockResolvedValue({
      branches: [],
      remote_branches: [],
      tags: [{ name: "v1.0.0", target: PAGE.commits[0].sha, annotation: null, tagged_at: null }],
    });
    render(<CommitLog repo="git-compose" />);

    await expect.element(page.getByText("v1.0.0")).toBeVisible();
  });

  it("shows no badge for a commit no ref points at", async () => {
    mockedListCommits.mockResolvedValue(PAGE);
    mockedGetRefs.mockResolvedValue({
      branches: [{ name: "main", target: "someothersha", committed_at: null }],
      remote_branches: [],
      tags: [],
    });
    render(<CommitLog repo="git-compose" />);

    await expect.element(page.getByText("fix: update readme")).toBeVisible();
    expect(page.getByText("main").elements().length).toBe(0);
  });

  it("caps badges at 3 with a +N overflow link", async () => {
    mockedListCommits.mockResolvedValue(PAGE);
    mockedGetRefs.mockResolvedValue({
      branches: [
        { name: "main", target: PAGE.commits[0].sha, committed_at: null },
        { name: "dev", target: PAGE.commits[0].sha, committed_at: null },
        { name: "release", target: PAGE.commits[0].sha, committed_at: null },
        { name: "hotfix", target: PAGE.commits[0].sha, committed_at: null },
      ],
      remote_branches: [],
      tags: [],
    });
    render(<CommitLog repo="git-compose" />);

    await expect.element(page.getByText("main")).toBeVisible();
    expect(page.getByText("hotfix").elements().length).toBe(0);
    const overflow = page.getByRole("link", { name: "+1" });
    await expect.element(overflow).toHaveAttribute("href", "/git-compose/refs");
  });

  it("renders no badges and no error when the refs request fails", async () => {
    mockedListCommits.mockResolvedValue(PAGE);
    mockedGetRefs.mockRejectedValue(new ApiError("internal", "boom", 500));
    render(<CommitLog repo="git-compose" />);

    await expect.element(page.getByText("fix: update readme")).toBeVisible();
    expect(page.getByRole("alert").elements().length).toBe(0);
  });

  it("requests msg=1 and shows the body when expanded", async () => {
    mockedListCommits.mockResolvedValue({
      commits: [{ ...PAGE.commits[0], body: "with a body" }],
      next_cursor: null,
    });
    render(<CommitLog repo="git-compose" msg="1" />);

    await expect.element(page.getByText("with a body")).toBeVisible();
    expect(mockedListCommits).toHaveBeenCalledWith("git-compose", {
      ref: undefined,
      path: undefined,
      cursor: undefined,
      msg: 1,
    });
  });

  it("shows no message row for a commit with no body even when expanded", async () => {
    mockedListCommits.mockResolvedValue(PAGE);
    render(<CommitLog repo="git-compose" msg="1" />);

    await expect.element(page.getByText("fix: update readme")).toBeVisible();
    expect(page.getByTestId("commit-graph-spacer").elements().length).toBe(0);
  });

  it("does not show the body when collapsed, even if the api sent one", async () => {
    mockedListCommits.mockResolvedValue({
      commits: [{ ...PAGE.commits[0], body: "with a body" }],
      next_cursor: null,
    });
    render(<CommitLog repo="git-compose" />);

    await expect.element(page.getByText("fix: update readme")).toBeVisible();
    expect(page.getByText("with a body").elements().length).toBe(0);
  });

  it("shows an Expand messages link that carries msg=1 and preserves ref/path/cursor", async () => {
    mockedListCommits.mockResolvedValue(PAGE);
    render(<CommitLog repo="git-compose" ref="main" path="src" cursor="abc123.50" />);

    const expandLink = page.getByRole("link", { name: "Expand messages" });
    await expect.element(expandLink).toBeVisible();
    const expandHref = await expandLink.element().getAttribute("href");
    const expandUrl = new URL(expandHref ?? "", "http://localhost");
    expect(expandUrl.pathname).toBe("/git-compose/log");
    expect(expandUrl.searchParams.get("msg")).toBe("1");
    expect(expandUrl.searchParams.get("ref")).toBe("main");
    expect(expandUrl.searchParams.get("path")).toBe("src");
    expect(expandUrl.searchParams.get("cursor")).toBe("abc123.50");
  });

  it("shows a Collapse messages link that drops msg and preserves ref/path", async () => {
    mockedListCommits.mockResolvedValue(PAGE);
    render(<CommitLog repo="git-compose" ref="main" path="src" msg="1" />);

    const collapseLink = page.getByRole("link", { name: "Collapse messages" });
    await expect.element(collapseLink).toBeVisible();
    const collapseHref = await collapseLink.element().getAttribute("href");
    const collapseUrl = new URL(collapseHref ?? "", "http://localhost");
    expect(collapseUrl.searchParams.get("msg")).toBeNull();
    expect(collapseUrl.searchParams.get("ref")).toBe("main");
  });

  it("renders one graph spacer per expanded commit with a body", async () => {
    const twoCommits: CommitsPage = {
      commits: [
        { ...PAGE.commits[0], body: "first body" },
        {
          sha: "def456abc123def456abc123def456abc123def",
          summary: "feat: add graph",
          body: undefined,
          author: { name: "Ada Lovelace", email_hash: "deadbeef" },
          authored_at: "2026-07-23T13:06:00+09:00",
          parents: [],
        },
      ],
      next_cursor: null,
    };
    mockedListCommits.mockResolvedValue(twoCommits);
    render(<CommitLog repo="git-compose" msg="1" />);

    await expect.element(page.getByText("first body")).toBeVisible();
    expect(page.getByTestId("commit-graph-spacer").elements().length).toBe(1);
  });

  it("keeps the Older link's msg=1 when expanded", async () => {
    mockedListCommits.mockResolvedValue({ ...PAGE, next_cursor: "def456" });
    render(<CommitLog repo="git-compose" msg="1" />);

    const older = page.getByRole("link", { name: "Older →" });
    await expect.element(older).toBeVisible();
    const href = await older.element().getAttribute("href");
    const url = new URL(href ?? "", "http://localhost");
    expect(url.searchParams.get("msg")).toBe("1");
  });
});
