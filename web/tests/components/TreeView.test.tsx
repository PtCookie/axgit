import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render } from "vitest-browser-react";
import { page } from "vitest/browser";

import TreeView from "@/components/repo/TreeView";
import { ApiError } from "@/lib/api/client";
import { getTree } from "@/lib/api/repos";
import type { TreeListing } from "@/lib/api/schemas";

vi.mock("@/lib/api/repos", () => ({
  getTree: vi.fn(),
}));

const mockedGetTree = vi.mocked(getTree);

const ROOT_TREE: TreeListing = {
  sha: "abc123def456abc123def456abc123def456abc",
  path: "",
  entries: [
    { name: "src", type: "tree", mode: "040000", size: null },
    { name: "README.md", type: "blob", mode: "100644", size: 16 },
    { name: "link", type: "symlink", mode: "120000", size: 4 },
    { name: "vendor", type: "commit", mode: "160000", size: null },
  ],
};

describe("TreeView", () => {
  beforeEach(() => {
    mockedGetTree.mockReset();
  });

  afterEach(() => {
    cleanup();
  });

  it("lists directory and file entries", async () => {
    mockedGetTree.mockResolvedValue(ROOT_TREE);
    render(<TreeView repo="git-compose" path="" />);

    await expect.element(page.getByText("src/")).toBeVisible();
    await expect.element(page.getByText("README.md")).toBeVisible();
    expect(mockedGetTree).toHaveBeenCalledWith("git-compose", undefined, "");
  });

  it("links a directory entry into the tree and a file entry to blob", async () => {
    mockedGetTree.mockResolvedValue(ROOT_TREE);
    render(<TreeView repo="git-compose" path="" />);

    await expect.element(page.getByRole("link", { name: "src/" })).toHaveAttribute("href", "/git-compose/tree/src");
    await expect
      .element(page.getByRole("link", { name: "README.md" }))
      .toHaveAttribute("href", "/git-compose/blob/README.md");
    await expect.element(page.getByRole("link", { name: "link" })).toHaveAttribute("href", "/git-compose/blob/link");
  });

  it("renders a submodule entry without a link", async () => {
    mockedGetTree.mockResolvedValue(ROOT_TREE);
    render(<TreeView repo="git-compose" path="" />);

    await expect.element(page.getByText("vendor")).toBeVisible();
    expect(page.getByRole("link", { name: "vendor" }).elements().length).toBe(0);
  });

  it("shows a parent-directory link when not at the root", async () => {
    mockedGetTree.mockResolvedValue({ ...ROOT_TREE, path: "src" });
    render(<TreeView repo="git-compose" path="src" />);

    await expect.element(page.getByRole("link", { name: ".." })).toHaveAttribute("href", "/git-compose/tree");
    expect(mockedGetTree).toHaveBeenCalledWith("git-compose", undefined, "src");
  });

  it("passes ref through to the tree link and the request", async () => {
    mockedGetTree.mockResolvedValue({ ...ROOT_TREE, path: "src" });
    render(<TreeView repo="git-compose" path="src" ref="v1.0.0" />);

    await expect
      .element(page.getByRole("link", { name: ".." }))
      .toHaveAttribute("href", "/git-compose/tree?ref=v1.0.0");
    expect(mockedGetTree).toHaveBeenCalledWith("git-compose", "v1.0.0", "src");
  });

  it("shows an empty-repository message at the root", async () => {
    mockedGetTree.mockResolvedValue({ ...ROOT_TREE, entries: [] });
    render(<TreeView repo="scratch" path="" />);

    await expect.element(page.getByText("This repository is empty.")).toBeVisible();
  });

  it("shows a path-not-found message", async () => {
    mockedGetTree.mockRejectedValue(new ApiError("path_not_found", "path 'nope' not found", 404));
    render(<TreeView repo="git-compose" path="nope" />);

    await expect.element(page.getByRole("alert")).toHaveTextContent("Path not found.");
  });

  it("shows a repository-not-found message", async () => {
    mockedGetTree.mockRejectedValue(new ApiError("repo_not_found", "repository 'nope' not found", 404));
    render(<TreeView repo="nope" path="" />);

    await expect.element(page.getByRole("alert")).toHaveTextContent("Repository not found.");
  });
});
