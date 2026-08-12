import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render } from "vitest-browser-react";
import { page } from "vitest/browser";

import SiteIntro from "@/components/SiteIntro";
import { ApiError } from "@/lib/api/client";
import { getSite } from "@/lib/api/site";
import type { SiteInfo } from "@/lib/api/schemas";

vi.mock("@/lib/api/site", () => ({
  getSite: vi.fn(),
}));

const mockedGetSite = vi.mocked(getSite);

const UNCONFIGURED: SiteInfo = { title: "Axgit", description: null, readme: null };

describe("SiteIntro", () => {
  beforeEach(() => {
    mockedGetSite.mockReset();
  });

  afterEach(() => {
    cleanup();
  });

  it("renders nothing when nothing is configured", async () => {
    mockedGetSite.mockResolvedValue(UNCONFIGURED);
    const { container } = await render(<SiteIntro />);

    // Wait for the skeleton (`aria-busy="true"`) to clear before asserting
    // emptiness — otherwise this would trivially pass during the loading
    // state too, without ever exercising the "fetch resolved, nothing
    // configured" branch this test is actually about.
    await vi.waitFor(() => {
      expect(container.querySelector('[aria-busy="true"]')).toBeNull();
    });
    expect(container.childElementCount).toBe(0);
  });

  it("shows the configured title and description", async () => {
    mockedGetSite.mockResolvedValue({ title: "PtCookie Git", description: "Self-hosted repositories", readme: null });
    render(<SiteIntro />);

    await expect.element(page.getByRole("heading", { name: "PtCookie Git", level: 1 })).toBeVisible();
    await expect.element(page.getByText("Self-hosted repositories")).toBeVisible();
  });

  it("shows only the title when the description is unset", async () => {
    mockedGetSite.mockResolvedValue({ title: "PtCookie Git", description: null, readme: null });
    render(<SiteIntro />);

    await expect.element(page.getByRole("heading", { name: "PtCookie Git", level: 1 })).toBeVisible();
  });

  it("renders a markdown readme without rewriting its relative links", async () => {
    mockedGetSite.mockResolvedValue({
      title: "Axgit",
      description: null,
      readme: { format: "markdown", content: "# Welcome\n\nSee [the guide](./guide.md).\n" },
    });
    render(<SiteIntro />);

    // The <h1> from `getSite`'s own `title` still renders even though it's
    // the default — a readme was configured, so `configured` is true.
    await expect.element(page.getByRole("heading", { name: "Axgit", level: 1 })).toBeVisible();
    await expect.element(page.getByRole("heading", { name: "Welcome", level: 1 })).toBeVisible();
    // No `repo` means `ReadmeMarkdown` leaves relative hrefs untouched
    // rather than rewriting them into a `/{repo}/blob/...` page URL.
    await expect.element(page.getByRole("link", { name: "the guide" })).toHaveAttribute("href", "./guide.md");
  });

  it("renders a plain-text readme as preformatted text", async () => {
    mockedGetSite.mockResolvedValue({
      title: "Axgit",
      description: null,
      readme: { format: "plain", content: "Just text.\n" },
    });
    render(<SiteIntro />);

    await expect.element(page.getByText("Just text.")).toBeVisible();
  });

  it("shows an error message when the request fails", async () => {
    mockedGetSite.mockRejectedValue(new ApiError("internal", "boom", 500));
    render(<SiteIntro />);

    await expect.element(page.getByRole("alert")).toHaveTextContent("boom");
  });
});
