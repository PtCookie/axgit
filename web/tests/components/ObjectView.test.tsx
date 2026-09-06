import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render } from "vitest-browser-react";
import { page } from "vitest/browser";

import ObjectView from "@/components/repo/ObjectView";
import { ApiError } from "@/lib/api/client";
import { fetchRawBytes, getObject } from "@/lib/api/repos";
import type { ObjectDetail } from "@/lib/api/schemas";

vi.mock("@/lib/api/repos", async (importOriginal) => {
  // `objectRawUrl` is a pure link builder (no network) — kept real via
  // `importOriginal` so the Raw link assertions below exercise the actual
  // implementation, same reasoning as `TagView.test.tsx`. `fetchRawBytes`
  // does hit the network, so it's mocked like `getObject`.
  const actual = await importOriginal<typeof import("@/lib/api/repos")>();
  return { ...actual, getObject: vi.fn(), fetchRawBytes: vi.fn() };
});

const mockedGetObject = vi.mocked(getObject);
const mockedFetchRawBytes = vi.mocked(fetchRawBytes);

const TREE_SHA = "1111111111111111111111111111111111111a";
const BLOB_SHA = "1111111111111111111111111111111111111b";
const COMMIT_SHA = "1111111111111111111111111111111111111c";
const TAG_SHA = "1111111111111111111111111111111111111d";

const TREE: ObjectDetail = {
  sha: TREE_SHA,
  type: "tree",
  tree: {
    entries: [
      {
        name: "src",
        type: "tree",
        mode: "040000",
        sha: "2222222222222222222222222222222222222a",
        size: null,
        target: null,
        module_link: null,
      },
      {
        name: "README.md",
        type: "blob",
        mode: "100644",
        sha: BLOB_SHA,
        size: 16,
        target: null,
        module_link: null,
      },
      {
        name: "vendor",
        type: "commit",
        mode: "160000",
        sha: "3333333333333333333333333333333333333a",
        size: null,
        target: null,
        // Always null under the by-oid tree — no path context to resolve a
        // `module_link` template against (docs/DECISIONS.md #72).
        module_link: null,
      },
    ],
  },
  blob: null,
  tag: null,
};

const BLOB: ObjectDetail = {
  sha: BLOB_SHA,
  type: "blob",
  tree: null,
  blob: { size: 6, binary: false, too_large: false, content: "hello\n" },
  tag: null,
};

const COMMIT: ObjectDetail = {
  sha: COMMIT_SHA,
  type: "commit",
  tree: null,
  blob: null,
  tag: null,
};

const TAG: ObjectDetail = {
  sha: TAG_SHA,
  type: "tag",
  tree: null,
  blob: null,
  tag: {
    object: { sha: TREE_SHA, type: "tree" },
    target: null,
    message: "a tree tag",
    tagger: { name: "Ada Lovelace", email_hash: "deadbeef" },
    tagged_at: "2026-07-22T19:00:00+09:00",
  },
};

describe("ObjectView", () => {
  beforeEach(() => {
    mockedGetObject.mockReset();
    mockedFetchRawBytes.mockReset();
  });

  afterEach(cleanup);

  it("lists a tree's entries, linking onward by their own sha except a gitlink", async () => {
    mockedGetObject.mockResolvedValue(TREE);
    await render(<ObjectView repo="git-compose" oid={TREE_SHA} />);

    await expect.element(page.getByText("src/")).toBeVisible();
    await expect
      .element(page.getByRole("link", { name: "src/" }))
      .toHaveAttribute("href", `/git-compose/object/2222222222222222222222222222222222222a`);
    await expect
      .element(page.getByRole("link", { name: "README.md" }))
      .toHaveAttribute("href", `/git-compose/object/${BLOB_SHA}`);
    // A gitlink's sha is a commit in another repository — no link.
    await expect.element(page.getByText("vendor")).toBeVisible();
    await expect.element(page.getByRole("link", { name: "vendor" })).not.toBeInTheDocument();
  });

  it("shows a blob's content and a Raw link", async () => {
    mockedGetObject.mockResolvedValue(BLOB);
    await render(<ObjectView repo="git-compose" oid={BLOB_SHA} />);

    await expect.element(page.getByText("hello")).toBeVisible();
    await expect
      .element(page.getByRole("link", { name: "Raw" }))
      .toHaveAttribute("href", `/api/v1/repos/git-compose/objects/${BLOB_SHA}/raw`);
  });

  it("renders a hex dump for a binary blob", async () => {
    mockedGetObject.mockResolvedValue({
      ...BLOB,
      blob: { size: 3, binary: true, too_large: false, content: null },
    });
    mockedFetchRawBytes.mockResolvedValue(Uint8Array.from([0x89, 0x50, 0x4e]));
    await render(<ObjectView repo="git-compose" oid={BLOB_SHA} />);

    await expect.element(page.getByText("00000000")).toBeVisible();
    await expect.element(page.getByText("89 50 4e")).toBeVisible();
    expect(mockedFetchRawBytes).toHaveBeenCalledWith(`/api/v1/repos/git-compose/objects/${BLOB_SHA}/raw`);
  });

  it("falls back to a binary notice when the raw fetch fails", async () => {
    mockedGetObject.mockResolvedValue({
      ...BLOB,
      blob: { size: 6, binary: true, too_large: false, content: null },
    });
    mockedFetchRawBytes.mockRejectedValue(new ApiError("internal", "network request failed", 0));
    await render(<ObjectView repo="git-compose" oid={BLOB_SHA} />);

    await expect.element(page.getByText(/Binary file not shown/)).toBeVisible();
  });

  it("links a commit oid to the commit page", async () => {
    mockedGetObject.mockResolvedValue(COMMIT);
    await render(<ObjectView repo="git-compose" oid={COMMIT_SHA} />);

    await expect
      .element(page.getByRole("link", { name: "View commit" }))
      .toHaveAttribute("href", `/git-compose/commit/${COMMIT_SHA}`);
  });

  it("shows a tag's dereference, tagger, and message, linking a non-commit target onward by oid", async () => {
    mockedGetObject.mockResolvedValue(TAG);
    await render(<ObjectView repo="git-compose" oid={TAG_SHA} />);

    await expect.element(page.getByText("Ada Lovelace")).toBeVisible();
    await expect.element(page.getByText(/a tree tag/)).toBeVisible();
    await expect
      .element(page.getByRole("link", { name: TREE_SHA }))
      .toHaveAttribute("href", `/git-compose/object/${TREE_SHA}`);
    // A tag on a tree never reaches a commit — no separate Commit row.
    await expect.element(page.getByText("Commit", { exact: true })).not.toBeInTheDocument();
  });

  it("links a nested tag's commit-typed dereference to the commit page", async () => {
    mockedGetObject.mockResolvedValue({
      ...TAG,
      tag: {
        object: { sha: COMMIT_SHA, type: "commit" },
        target: COMMIT_SHA,
        message: "a tree tag",
        tagger: { name: "Ada Lovelace", email_hash: "deadbeef" },
        tagged_at: "2026-07-22T19:00:00+09:00",
      },
    });
    await render(<ObjectView repo="git-compose" oid={TAG_SHA} />);

    await expect
      .element(page.getByRole("link", { name: COMMIT_SHA }))
      .toHaveAttribute("href", `/git-compose/commit/${COMMIT_SHA}`);
  });

  it("shows an error message when the object is not found", async () => {
    mockedGetObject.mockRejectedValue(new ApiError("object_not_found", "not found", 404));
    await render(<ObjectView repo="git-compose" oid="deadbeef" />);

    await expect.element(page.getByRole("alert")).toHaveTextContent("Object not found.");
  });
});
