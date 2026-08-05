import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render } from "vitest-browser-react";
import { page } from "vitest/browser";

import SearchView from "@/components/repo/SearchView";
import { ApiError } from "@/lib/api/client";
import { searchRepo } from "@/lib/api/repos";
import type { SearchResults } from "@/lib/api/schemas";

vi.mock("@/lib/api/repos", () => ({
  searchRepo: vi.fn(),
}));

const mockedSearchRepo = vi.mocked(searchRepo);

const CONTENT_RESULTS: SearchResults = {
  sha: "abc123def456abc123def456abc123def456abc",
  type: "content",
  truncated: false,
  files: [
    {
      path: "src/main.rs",
      lines: [{ line: 12, text: 'fn main() { println!("hello"); }' }],
    },
  ],
  commits: [],
};

const PATH_RESULTS: SearchResults = {
  sha: "abc123def456abc123def456abc123def456abc",
  type: "path",
  truncated: true,
  files: [{ path: "src/main.rs", lines: [] }],
  commits: [],
};

const MESSAGE_RESULTS: SearchResults = {
  sha: "abc123def456abc123def456abc123def456abc",
  type: "message",
  truncated: false,
  files: [],
  commits: [
    {
      sha: "def456abc123def456abc123def456abc123def",
      summary: "fix: add a helper function",
      author: { name: "Ada Lovelace", email_hash: "deadbeef" },
      authored_at: "2026-07-24T13:06:00+09:00",
      parents: [],
    },
  ],
};

const AUTHOR_RESULTS: SearchResults = {
  ...MESSAGE_RESULTS,
  type: "author",
};

const RANGE_RESULTS: SearchResults = {
  ...MESSAGE_RESULTS,
  type: "range",
};

describe("SearchView", () => {
  beforeEach(() => {
    mockedSearchRepo.mockReset();
  });

  afterEach(() => {
    cleanup();
  });

  it("shows a prompt and does not fetch when there is no query", async () => {
    render(<SearchView repo="git-compose" q="" />);

    await expect.element(page.getByText("Enter a search query above.")).toBeVisible();
    expect(mockedSearchRepo).not.toHaveBeenCalled();
  });

  it("renders content matches linking into the matching line", async () => {
    mockedSearchRepo.mockResolvedValue(CONTENT_RESULTS);
    render(<SearchView repo="git-compose" q="hello" type="content" />);

    await expect.element(page.getByText("src/main.rs")).toBeVisible();
    await expect.element(page.getByText(/fn main/)).toBeVisible();
    const link = page.getByRole("link", { name: /12:/ });
    await expect.element(link).toHaveAttribute("href", "/git-compose/blob/src/main.rs#L12");
    expect(mockedSearchRepo).toHaveBeenCalledWith("git-compose", {
      q: "hello",
      type: "content",
      ref: undefined,
    });
  });

  it("renders path matches without line-level detail", async () => {
    mockedSearchRepo.mockResolvedValue(PATH_RESULTS);
    render(<SearchView repo="git-compose" q="main" type="path" />);

    const link = page.getByRole("link", { name: "src/main.rs" });
    await expect.element(link).toHaveAttribute("href", "/git-compose/blob/src/main.rs");
  });

  it("renders message matches as commit rows", async () => {
    mockedSearchRepo.mockResolvedValue(MESSAGE_RESULTS);
    render(<SearchView repo="git-compose" q="helper" type="message" />);

    await expect.element(page.getByText("Ada Lovelace")).toBeVisible();
    const link = page.getByRole("link", { name: "fix: add a helper function" });
    await expect.element(link).toHaveAttribute("href", "/git-compose/commit/def456abc123def456abc123def456abc123def");
  });

  it("renders author matches as commit rows, reusing the message-row shape", async () => {
    mockedSearchRepo.mockResolvedValue(AUTHOR_RESULTS);
    render(<SearchView repo="git-compose" q="Ada" type="author" />);

    await expect.element(page.getByText("Ada Lovelace")).toBeVisible();
    const link = page.getByRole("link", { name: "fix: add a helper function" });
    await expect.element(link).toHaveAttribute("href", "/git-compose/commit/def456abc123def456abc123def456abc123def");
    expect(mockedSearchRepo).toHaveBeenCalledWith("git-compose", {
      q: "Ada",
      type: "author",
      ref: undefined,
    });
  });

  it("renders range matches as commit rows and shows the range hint", async () => {
    mockedSearchRepo.mockResolvedValue(RANGE_RESULTS);
    render(<SearchView repo="git-compose" q="v1.0..main" type="range" />);

    await expect.element(page.getByText("Ada Lovelace")).toBeVisible();
    await expect.element(page.getByText(/rev-list expression/)).toBeVisible();
    expect(mockedSearchRepo).toHaveBeenCalledWith("git-compose", {
      q: "v1.0..main",
      type: "range",
      ref: undefined,
    });
  });

  it("shows a no-matches message for an empty result", async () => {
    mockedSearchRepo.mockResolvedValue({ ...CONTENT_RESULTS, files: [] });
    render(<SearchView repo="git-compose" q="nope" />);

    await expect.element(page.getByText("No matches found.")).toBeVisible();
  });

  it("shows a truncation notice when the response is truncated", async () => {
    mockedSearchRepo.mockResolvedValue(PATH_RESULTS);
    render(<SearchView repo="git-compose" q="main" type="path" />);

    await expect.element(page.getByRole("status")).toHaveTextContent("partial result");
  });

  it("shows an error message when the request fails", async () => {
    mockedSearchRepo.mockRejectedValue(new ApiError("internal", "boom", 500));
    render(<SearchView repo="git-compose" q="hello" />);

    await expect.element(page.getByRole("alert")).toHaveTextContent("boom");
  });

  it("defaults to type=content when omitted", async () => {
    mockedSearchRepo.mockResolvedValue(CONTENT_RESULTS);
    render(<SearchView repo="git-compose" q="hello" />);

    await expect.element(page.getByText("src/main.rs")).toBeVisible();
    expect(mockedSearchRepo).toHaveBeenCalledWith("git-compose", {
      q: "hello",
      type: "content",
      ref: undefined,
    });
  });
});
