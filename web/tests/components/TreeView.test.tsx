import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render } from "vitest-browser-react";
import { page } from "vitest/browser";

import TreeView from "@/components/repo/TreeView";
import { ApiError } from "@/lib/api/client";
import { getTree } from "@/lib/api/repos";
import type { TreeEntryInfo, TreeListing } from "@/lib/api/schemas";

vi.mock("@/lib/api/repos", async (importOriginal) => {
  // `rawUrl` is a pure link builder (no network) — kept real via
  // `importOriginal` so the per-row Raw link assertions below exercise the
  // actual implementation, only `getTree` needs mocking.
  const actual = await importOriginal<typeof import("@/lib/api/repos")>();
  return { ...actual, getTree: vi.fn() };
});

const mockedGetTree = vi.mocked(getTree);

// Deliberately not a target any other row's name matches — the target renders
// as its own link, so a colliding value would make every non-exact
// `getByRole("link", { name })` lookup below ambiguous.
const SYMLINK: TreeEntryInfo = {
  name: "link",
  type: "symlink",
  mode: "120000",
  sha: "1111111111111111111111111111111111111c",
  size: 4,
  target: "docs/guide.md",
  module_link: null,
};

const ROOT_TREE: TreeListing = {
  sha: "abc123def456abc123def456abc123def456abc",
  path: "",
  entries: [
    {
      name: "src",
      type: "tree",
      mode: "040000",
      sha: "1111111111111111111111111111111111111a",
      size: null,
      target: null,
      module_link: null,
    },
    {
      name: "README.md",
      type: "blob",
      mode: "100644",
      sha: "1111111111111111111111111111111111111b",
      size: 16,
      target: null,
      module_link: null,
    },
    SYMLINK,
    {
      name: "vendor",
      type: "commit",
      mode: "160000",
      sha: "1111111111111111111111111111111111111d",
      size: null,
      target: null,
      module_link: null,
    },
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
      .element(page.getByRole("link", { name: "README.md", exact: true }))
      .toHaveAttribute("href", "/git-compose/blob/README.md");
    await expect
      .element(page.getByRole("link", { name: "link", exact: true }))
      .toHaveAttribute("href", "/git-compose/blob/link");
  });

  it("shows a symlink's target and links it through the normalized path", async () => {
    mockedGetTree.mockResolvedValue({ ...ROOT_TREE, path: "src", entries: [SYMLINK] });
    render(<TreeView repo="git-compose" path="src" />);

    // Displayed verbatim, linked resolved against the listed directory.
    await expect
      .element(page.getByRole("link", { name: "docs/guide.md" }))
      .toHaveAttribute("href", "/git-compose/blob/src/docs/guide.md");
  });

  it("resolves a `..` target against the listed directory, not the entry", async () => {
    const entry = { ...SYMLINK, target: "../README.md" };
    mockedGetTree.mockResolvedValue({ ...ROOT_TREE, path: "src/lib", entries: [entry] });
    render(<TreeView repo="git-compose" path="src/lib" />);

    await expect
      .element(page.getByRole("link", { name: "../README.md" }))
      .toHaveAttribute("href", "/git-compose/blob/src/README.md");
  });

  it("renders a root-escaping symlink target as plain text, not a link", async () => {
    const entry = { ...SYMLINK, target: "../../outside" };
    mockedGetTree.mockResolvedValue({ ...ROOT_TREE, entries: [entry] });
    render(<TreeView repo="git-compose" path="" />);

    await expect.element(page.getByText("../../outside")).toBeVisible();
    expect(page.getByRole("link", { name: "../../outside" }).elements().length).toBe(0);
  });

  it("renders a submodule entry without a name link or any row actions", async () => {
    mockedGetTree.mockResolvedValue(ROOT_TREE);
    render(<TreeView repo="git-compose" path="" />);

    await expect.element(page.getByText("vendor")).toBeVisible();
    expect(page.getByRole("link", { name: "vendor" }).elements().length).toBe(0);
    expect(page.getByRole("link", { name: "Log for vendor" }).elements().length).toBe(0);
    expect(page.getByRole("link", { name: "Stats for vendor" }).elements().length).toBe(0);
  });

  it("links a submodule entry to its module_link, with a full-load opt-out, but no row actions", async () => {
    const vendor = {
      ...ROOT_TREE.entries[3],
      module_link: "https://example.com/dep",
    };
    mockedGetTree.mockResolvedValue({ ...ROOT_TREE, entries: [vendor] });
    render(<TreeView repo="git-compose" path="" />);

    const link = page.getByRole("link", { name: "vendor" });
    await expect.element(link).toHaveAttribute("href", "https://example.com/dep");
    await expect.element(link).toHaveAttribute("rel", "noopener noreferrer");
    await expect.element(link).toHaveAttribute("data-astro-reload");
    expect(page.getByRole("link", { name: "Log for vendor" }).elements().length).toBe(0);
    expect(page.getByRole("link", { name: "Stats for vendor" }).elements().length).toBe(0);
  });

  it("renders a root-relative module_link verbatim", async () => {
    const vendor = {
      ...ROOT_TREE.entries[3],
      module_link: "/git/dep.git/commit/?id=abc",
    };
    mockedGetTree.mockResolvedValue({ ...ROOT_TREE, entries: [vendor] });
    render(<TreeView repo="git-compose" path="" />);

    await expect
      .element(page.getByRole("link", { name: "vendor" }))
      .toHaveAttribute("href", "/git/dep.git/commit/?id=abc");
  });

  it("orders the columns as Mode, Name, Size and shows symbolic modes", async () => {
    mockedGetTree.mockResolvedValue(ROOT_TREE);
    render(<TreeView repo="git-compose" path="" />);

    await expect.element(page.getByText("d---------")).toBeVisible();
    await expect.element(page.getByText("-rw-r--r--")).toBeVisible();

    const headers = page.getByRole("row").elements()[0].textContent;
    expect(headers).toEqual("ModeNameSizeLinks");
  });

  it("gives a file row Log, Stats, Raw, and Blame quick links", async () => {
    mockedGetTree.mockResolvedValue(ROOT_TREE);
    render(<TreeView repo="git-compose" path="" />);

    await expect
      .element(page.getByRole("link", { name: "Log for README.md" }))
      .toHaveAttribute("href", "/git-compose/log?path=README.md");
    await expect
      .element(page.getByRole("link", { name: "Stats for README.md" }))
      .toHaveAttribute("href", "/git-compose/stats?path=README.md");
    await expect
      .element(page.getByRole("link", { name: "Raw for README.md" }))
      .toHaveAttribute("href", "/api/v1/repos/git-compose/raw/HEAD/README.md");
    await expect
      .element(page.getByRole("link", { name: "Blame for README.md" }))
      .toHaveAttribute("href", "/git-compose/blame/README.md");
  });

  it("gives a directory row only Log and Stats quick links", async () => {
    mockedGetTree.mockResolvedValue(ROOT_TREE);
    render(<TreeView repo="git-compose" path="" />);

    await expect
      .element(page.getByRole("link", { name: "Log for src" }))
      .toHaveAttribute("href", "/git-compose/log?path=src");
    await expect
      .element(page.getByRole("link", { name: "Stats for src" }))
      .toHaveAttribute("href", "/git-compose/stats?path=src");
    expect(page.getByRole("link", { name: "Raw for src" }).elements().length).toBe(0);
    expect(page.getByRole("link", { name: "Blame for src" }).elements().length).toBe(0);
  });

  it("carries ref through to the row action links", async () => {
    mockedGetTree.mockResolvedValue(ROOT_TREE);
    render(<TreeView repo="git-compose" path="" ref="v1.0.0" />);

    await expect
      .element(page.getByRole("link", { name: "Log for README.md" }))
      .toHaveAttribute("href", "/git-compose/log?path=README.md&ref=v1.0.0");
    await expect
      .element(page.getByRole("link", { name: "Stats for README.md" }))
      .toHaveAttribute("href", "/git-compose/stats?path=README.md&ref=v1.0.0");
    await expect
      .element(page.getByRole("link", { name: "Blame for README.md" }))
      .toHaveAttribute("href", "/git-compose/blame/README.md?ref=v1.0.0");
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
