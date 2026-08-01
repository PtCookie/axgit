import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render } from "vitest-browser-react";
import { page } from "vitest/browser";

import RefsView from "@/components/repo/RefsView";
import { ApiError } from "@/lib/api/client";
import { getRefs } from "@/lib/api/repos";
import type { RefsInfo } from "@/lib/api/schemas";

vi.mock("@/lib/api/repos", () => ({
  getRefs: vi.fn(),
}));

const mockedGetRefs = vi.mocked(getRefs);

const REFS: RefsInfo = {
  branches: [{ name: "main", target: "abc123def456", committed_at: "2026-07-24T13:06:00+09:00" }],
  tags: [
    { name: "v1.0.0", target: "def456abc123", annotation: "First release", tagged_at: "2026-01-01T00:00:00+09:00" },
  ],
};

describe("RefsView", () => {
  beforeEach(() => {
    mockedGetRefs.mockReset();
  });

  afterEach(() => {
    cleanup();
  });

  it("lists branches and tags", async () => {
    mockedGetRefs.mockResolvedValue(REFS);
    render(<RefsView repo="git-compose" />);

    await expect.element(page.getByText("main")).toBeVisible();
    await expect.element(page.getByText("v1.0.0")).toBeVisible();
    await expect.element(page.getByText("First release")).toBeVisible();
  });

  it("shows empty-state messages when there are no branches or tags", async () => {
    mockedGetRefs.mockResolvedValue({ branches: [], tags: [] });
    render(<RefsView repo="scratch" />);

    await expect.element(page.getByText("No branches.")).toBeVisible();
    await expect.element(page.getByText("No tags.")).toBeVisible();
  });

  it("shows an error message when the request fails", async () => {
    mockedGetRefs.mockRejectedValue(new ApiError("internal", "boom", 500));
    render(<RefsView repo="git-compose" />);

    await expect.element(page.getByRole("alert")).toHaveTextContent("boom");
  });
});
