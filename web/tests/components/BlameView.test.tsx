import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render } from "vitest-browser-react";
import { page } from "vitest/browser";

import BlameView from "@/components/repo/BlameView";
import { ApiError } from "@/lib/api/client";
import { getBlame, getBlob, rawUrl } from "@/lib/api/repos";
import type { BlameInfo, BlobInfo } from "@/lib/api/schemas";

vi.mock("@/lib/api/repos", () => ({
  getBlame: vi.fn(),
  getBlob: vi.fn(),
  rawUrl: vi.fn(() => "/api/v1/repos/git-compose/raw/HEAD/README.md"),
}));

const mockedGetBlame = vi.mocked(getBlame);
const mockedGetBlob = vi.mocked(getBlob);

const FULL_SHA = "abc123def456abc123def456abc123def456abc";

const TEXT_BLOB: BlobInfo = {
  sha: FULL_SHA,
  path: "README.md",
  mode: "100644",
  size: 16,
  binary: false,
  too_large: false,
  content: "# Axgit\nHello\n",
};

const BLAME: BlameInfo = {
  sha: FULL_SHA,
  path: "README.md",
  binary: false,
  too_large: false,
  lines: 2,
  ranges: [
    {
      start_line: 1,
      line_count: 2,
      sha: FULL_SHA,
      summary: "feat: initial",
      author: { name: "Ada Lovelace", email_hash: "deadbeef" },
      authored_at: "2026-07-24T13:06:00+09:00",
      orig_path: null,
    },
  ],
};

describe("BlameView", () => {
  beforeEach(() => {
    mockedGetBlame.mockReset();
    mockedGetBlob.mockReset();
    vi.mocked(rawUrl).mockClear();
  });

  afterEach(cleanup);

  it("renders content with a per-range gutter", async () => {
    mockedGetBlame.mockResolvedValue(BLAME);
    mockedGetBlob.mockResolvedValue(TEXT_BLOB);
    await render(<BlameView repo="git-compose" path="README.md" />);

    await expect.element(page.getByText("# Axgit")).toBeVisible();
    await expect.element(page.getByText("Hello")).toBeVisible();
    await expect.element(page.getByText("Ada Lovelace")).toBeVisible();
    expect(mockedGetBlame).toHaveBeenCalledWith("git-compose", undefined, "README.md");
    expect(mockedGetBlob).toHaveBeenCalledWith("git-compose", undefined, "README.md");
  });

  it("links the gutter's short sha to the commit page", async () => {
    mockedGetBlame.mockResolvedValue(BLAME);
    mockedGetBlob.mockResolvedValue(TEXT_BLOB);
    await render(<BlameView repo="git-compose" path="README.md" />);

    await expect
      .element(page.getByRole("link", { name: FULL_SHA.slice(0, 7) }))
      .toHaveAttribute("href", `/git-compose/commit/${FULL_SHA}`);
  });

  it("links to the raw endpoint, the file view, and the path-filtered log", async () => {
    mockedGetBlame.mockResolvedValue(BLAME);
    mockedGetBlob.mockResolvedValue(TEXT_BLOB);
    await render(<BlameView repo="git-compose" path="README.md" />);

    await expect
      .element(page.getByRole("link", { name: "Raw" }))
      .toHaveAttribute("href", "/api/v1/repos/git-compose/raw/HEAD/README.md");
    await expect
      .element(page.getByRole("link", { name: "View file" }))
      .toHaveAttribute("href", "/git-compose/blob/README.md");
    await expect
      .element(page.getByRole("link", { name: "History" }))
      .toHaveAttribute("href", "/git-compose/log?path=README.md");
  });

  it("marks a renamed range with a link to its previous path's blame", async () => {
    mockedGetBlame.mockResolvedValue({
      ...BLAME,
      ranges: [{ ...BLAME.ranges[0], orig_path: "old/README.md" }],
    });
    mockedGetBlob.mockResolvedValue(TEXT_BLOB);
    await render(<BlameView repo="git-compose" path="README.md" />);

    await expect
      .element(page.getByRole("link", { name: "Renamed from old/README.md" }))
      .toHaveAttribute("href", `/git-compose/blame/old/README.md?ref=${FULL_SHA}`);
  });

  it("shows no rename marker when orig_path is unset", async () => {
    mockedGetBlame.mockResolvedValue(BLAME);
    mockedGetBlob.mockResolvedValue(TEXT_BLOB);
    await render(<BlameView repo="git-compose" path="README.md" />);

    await expect.element(page.getByText("# Axgit")).toBeVisible();
    expect(page.getByRole("link", { name: /^Renamed from/ }).elements()).toHaveLength(0);
  });

  it("shows a binary-file message instead of a gutter", async () => {
    mockedGetBlame.mockResolvedValue({ ...BLAME, binary: true, lines: 0, ranges: [] });
    mockedGetBlob.mockResolvedValue({ ...TEXT_BLOB, binary: true, content: null });
    await render(<BlameView repo="git-compose" path="image.png" />);

    await expect.element(page.getByText("Binary file — blame not shown.")).toBeVisible();
  });

  it("shows a too-large message instead of a gutter", async () => {
    mockedGetBlame.mockResolvedValue({ ...BLAME, too_large: true, lines: 0, ranges: [] });
    mockedGetBlob.mockResolvedValue({ ...TEXT_BLOB, too_large: true, content: null });
    await render(<BlameView repo="git-compose" path="huge.bin" />);

    await expect.element(page.getByText("File too large — blame not shown.")).toBeVisible();
  });

  it("shows an empty-file message when there are no ranges", async () => {
    mockedGetBlame.mockResolvedValue({ ...BLAME, lines: 0, ranges: [] });
    mockedGetBlob.mockResolvedValue({ ...TEXT_BLOB, content: "" });
    await render(<BlameView repo="git-compose" path="empty.txt" />);

    await expect.element(page.getByText("This file is empty.")).toBeVisible();
  });

  it("shows a path-not-found message", async () => {
    mockedGetBlame.mockRejectedValue(new ApiError("path_not_found", "path 'nope' not found", 404));
    mockedGetBlob.mockResolvedValue(TEXT_BLOB);
    await render(<BlameView repo="git-compose" path="nope" />);

    await expect.element(page.getByRole("alert")).toHaveTextContent("Path not found.");
  });
});
