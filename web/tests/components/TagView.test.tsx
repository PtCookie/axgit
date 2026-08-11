import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render } from "vitest-browser-react";
import { page } from "vitest/browser";

import TagView from "@/components/repo/TagView";
import { ApiError } from "@/lib/api/client";
import { ARCHIVE_FORMATS, getTag } from "@/lib/api/repos";
import type { TagDetail } from "@/lib/api/schemas";

vi.mock("@/lib/api/repos", async (importOriginal) => {
  // `archiveUrl` is a pure link builder (no network) — kept real via
  // `importOriginal` so the action row's href assertions below exercise the
  // actual implementation, same reasoning as `RepoSummary.test.tsx`.
  const actual = await importOriginal<typeof import("@/lib/api/repos")>();
  return { ...actual, getTag: vi.fn() };
});

const mockedGetTag = vi.mocked(getTag);

const TAGGER = { name: "Ada Lovelace", email_hash: "deadbeef" };

const ANNOTATED: TagDetail = {
  name: "v1.0.0",
  tag_object: "tagobject0000000000000000000000000000000",
  object: { sha: "abc123def456abc123def456abc123def456abc", type: "commit" },
  target: "abc123def456abc123def456abc123def456abc",
  message: "Release v1.0.0\n\nSecond line.",
  tagger: TAGGER,
  tagged_at: "2026-07-22T19:00:00+09:00",
};

describe("TagView", () => {
  beforeEach(() => {
    mockedGetTag.mockReset();
  });

  afterEach(() => {
    cleanup();
  });

  it("shows the tag object, tagger, message, and a linked commit object", async () => {
    mockedGetTag.mockResolvedValue(ANNOTATED);
    render(<TagView repo="git-compose" name="v1.0.0" />);

    await expect.element(page.getByRole("heading", { name: "v1.0.0" })).toBeVisible();
    await expect.element(page.getByText("tagobject0000000000000000000000000000000")).toBeVisible();
    await expect.element(page.getByText("Ada Lovelace")).toBeVisible();
    await expect.element(page.getByText(/Release v1\.0\.0/)).toBeVisible();
    await expect.element(page.getByText(/Second line\./)).toBeVisible();

    const objectLink = page.getByRole("link", { name: ANNOTATED.object.sha });
    await expect
      .element(objectLink)
      .toHaveAttribute("href", "/git-compose/commit/abc123def456abc123def456abc123def456abc");
    // object.type === "commit" and target === object.sha: no separate
    // "Commit" row, since it would just repeat the Object row above it.
    await expect.element(page.getByText("Commit", { exact: true })).not.toBeInTheDocument();
  });

  it("offers Tree/Log/download links when the tag reaches a commit", async () => {
    mockedGetTag.mockResolvedValue(ANNOTATED);
    render(<TagView repo="git-compose" name="v1.0.0" />);

    for (const format of ARCHIVE_FORMATS) {
      await expect
        .element(page.getByRole("link", { name: format }))
        .toHaveAttribute("href", `/api/v1/repos/git-compose/archive/v1.0.0.${format}`);
    }
    await expect.element(page.getByRole("link", { name: "Browse the tree at this tag" })).toBeVisible();
  });

  it("shows a separate Commit row for a nested tag (object differs from the peeled commit)", async () => {
    mockedGetTag.mockResolvedValue({
      ...ANNOTATED,
      object: { sha: "innertag00000000000000000000000000000000", type: "tag" },
      target: "commit000000000000000000000000000000000000",
    });
    render(<TagView repo="git-compose" name="outer" />);

    await expect.element(page.getByText("innertag00000000000000000000000000000000")).toBeVisible();
    const commitLink = page.getByRole("link", { name: "commit000000000000000000000000000000000000" });
    await expect
      .element(commitLink)
      .toHaveAttribute("href", "/git-compose/commit/commit000000000000000000000000000000000000");
  });

  it("links a blob target to the by-oid object page, with no Tree/Log/download links", async () => {
    mockedGetTag.mockResolvedValue({
      ...ANNOTATED,
      object: { sha: "blob00000000000000000000000000000000000000", type: "blob" },
      target: null,
    });
    render(<TagView repo="git-compose" name="blob-tag" />);

    await expect
      .element(page.getByRole("link", { name: "blob00000000000000000000000000000000000000" }))
      .toHaveAttribute("href", "/git-compose/object/blob00000000000000000000000000000000000000");
    // A tag with no target still can't be archived or browsed as a tree —
    // only the object row's link changed, not the `canBrowse` gate.
    await expect.element(page.getByRole("link", { name: "tar.gz" })).not.toBeInTheDocument();
    await expect.element(page.getByRole("link", { name: "Browse the tree at this tag" })).not.toBeInTheDocument();
  });

  it("renders a lightweight tag with no tag object or tagger row", async () => {
    mockedGetTag.mockResolvedValue({
      ...ANNOTATED,
      tag_object: null,
      message: null,
      tagger: null,
      tagged_at: null,
    });
    render(<TagView repo="git-compose" name="snapshot" />);

    await expect.element(page.getByText("Tag object", { exact: true })).not.toBeInTheDocument();
    await expect.element(page.getByText("Tagger", { exact: true })).not.toBeInTheDocument();
  });

  it("shows an error message when the tag is not found", async () => {
    mockedGetTag.mockRejectedValue(new ApiError("ref_not_found", "not found", 404));
    render(<TagView repo="git-compose" name="no-such-tag" />);

    await expect.element(page.getByRole("alert")).toHaveTextContent("Tag not found.");
  });
});
