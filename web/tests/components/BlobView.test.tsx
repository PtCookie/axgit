import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render } from "vitest-browser-react";
import { page } from "vitest/browser";

import BlobView from "@/components/repo/BlobView";
import { ApiError } from "@/lib/api/client";
import { fetchRawBytes, getBlob, rawUrl } from "@/lib/api/repos";
import type { BlobInfo } from "@/lib/api/schemas";
import { HEX_DUMP_LIMIT } from "@/lib/format/hex";

vi.mock("@/lib/api/repos", () => ({
  getBlob: vi.fn(),
  rawUrl: vi.fn(() => "/api/v1/repos/git-compose/raw/HEAD/README.md"),
  fetchRawBytes: vi.fn(),
}));

const mockedGetBlob = vi.mocked(getBlob);
const mockedFetchRawBytes = vi.mocked(fetchRawBytes);

const TEXT_BLOB: BlobInfo = {
  sha: "abc123def456abc123def456abc123def456abc",
  path: "README.md",
  mode: "100644",
  size: 16,
  binary: false,
  too_large: false,
  content: "# Axgit\nHello\n",
};

describe("BlobView", () => {
  beforeEach(() => {
    mockedGetBlob.mockReset();
    mockedFetchRawBytes.mockReset();
    vi.mocked(rawUrl).mockClear();
  });

  afterEach(cleanup);

  it("renders text content with line numbers", async () => {
    mockedGetBlob.mockResolvedValue(TEXT_BLOB);
    render(<BlobView repo="git-compose" path="README.md" />);

    await expect.element(page.getByText("# Axgit")).toBeVisible();
    await expect.element(page.getByText("Hello")).toBeVisible();
    expect(mockedGetBlob).toHaveBeenCalledWith("git-compose", undefined, "README.md");
  });

  it("links to the raw endpoint, blame, and the path-filtered log", async () => {
    mockedGetBlob.mockResolvedValue(TEXT_BLOB);
    render(<BlobView repo="git-compose" path="README.md" />);

    await expect
      .element(page.getByRole("link", { name: "Raw" }))
      .toHaveAttribute("href", "/api/v1/repos/git-compose/raw/HEAD/README.md");
    await expect
      .element(page.getByRole("link", { name: "Blame" }))
      .toHaveAttribute("href", "/git-compose/blame/README.md");
    await expect
      .element(page.getByRole("link", { name: "History" }))
      .toHaveAttribute("href", "/git-compose/log?path=README.md");
  });

  it("renders a hex dump for a binary blob", async () => {
    mockedGetBlob.mockResolvedValue({ ...TEXT_BLOB, binary: true, content: null, size: 3 });
    mockedFetchRawBytes.mockResolvedValue(Uint8Array.from([0x89, 0x50, 0x4e]));
    render(<BlobView repo="git-compose" path="image.png" />);

    await expect.element(page.getByText("00000000")).toBeVisible();
    await expect.element(page.getByText("89 50 4e")).toBeVisible();
    expect(mockedFetchRawBytes).toHaveBeenCalledWith("/api/v1/repos/git-compose/raw/HEAD/README.md");
  });

  it("shows a truncation notice past the hex dump render cap", async () => {
    mockedGetBlob.mockResolvedValue({ ...TEXT_BLOB, binary: true, content: null, size: HEX_DUMP_LIMIT + 1 });
    mockedFetchRawBytes.mockResolvedValue(new Uint8Array(HEX_DUMP_LIMIT + 1));
    render(<BlobView repo="git-compose" path="image.png" />);

    await expect.element(page.getByText("Showing the first", { exact: false })).toBeVisible();
  });

  it("falls back to a binary notice when the raw fetch fails", async () => {
    mockedGetBlob.mockResolvedValue({ ...TEXT_BLOB, binary: true, content: null });
    mockedFetchRawBytes.mockRejectedValue(new ApiError("internal", "network request failed", 0));
    render(<BlobView repo="git-compose" path="image.png" />);

    await expect.element(page.getByText("Binary file not shown —", { exact: false })).toBeVisible();
  });

  it("shows a too-large message instead of content", async () => {
    mockedGetBlob.mockResolvedValue({ ...TEXT_BLOB, too_large: true, content: null });
    render(<BlobView repo="git-compose" path="huge.bin" />);

    await expect.element(page.getByText("File too large to display", { exact: false })).toBeVisible();
  });

  it("shows the link target for a symlink", async () => {
    mockedGetBlob.mockResolvedValue({ ...TEXT_BLOB, mode: "120000", content: "../target.txt" });
    render(<BlobView repo="git-compose" path="link" />);

    await expect.element(page.getByText("../target.txt")).toBeVisible();
    await expect.element(page.getByText("Symlink to", { exact: false })).toBeVisible();
  });

  it("shows a path-not-found message", async () => {
    mockedGetBlob.mockRejectedValue(new ApiError("path_not_found", "path 'nope' not found", 404));
    render(<BlobView repo="git-compose" path="nope" />);

    await expect.element(page.getByRole("alert")).toHaveTextContent("Path not found.");
  });
});
