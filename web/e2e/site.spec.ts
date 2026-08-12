import { expect, test } from "@playwright/test";

import fixture from "../tests/fixtures/repos.json" with { type: "json" };

const UNCONFIGURED = { title: "Axgit", description: null, readme: null };

test("shows the configured site title, description, and readme on the index page", async ({ page }) => {
  await page.route("**/api/v1/repos", async (route) => {
    await route.fulfill({ json: fixture });
  });
  await page.route("**/api/v1/site", async (route) => {
    await route.fulfill({
      json: {
        title: "PtCookie Git",
        description: "Self-hosted repositories",
        readme: { format: "markdown", content: "# Welcome\n\nSee the repositories below.\n" },
      },
    });
  });

  await page.goto("/");

  await expect(page.getByRole("heading", { name: "PtCookie Git", level: 1 })).toBeVisible();
  await expect(page.getByText("Self-hosted repositories")).toBeVisible();
  await expect(page.getByRole("heading", { name: "Welcome", level: 1 })).toBeVisible();
  // The repository list below is unaffected.
  await expect(page.getByText("git-compose")).toBeVisible();
});

test("shows nothing extra on the index page when the site is unconfigured", async ({ page }) => {
  await page.route("**/api/v1/repos", async (route) => {
    await route.fulfill({ json: fixture });
  });
  await page.route("**/api/v1/site", async (route) => {
    await route.fulfill({ json: UNCONFIGURED });
  });

  await page.goto("/");

  await expect(page.getByText("git-compose")).toBeVisible();
  await expect(page.getByRole("heading", { level: 1 })).not.toBeAttached();
});

// `astro dev` (which the e2e webServer runs, `playwright.config.ts`) never
// exercises `api/src/shell.rs`'s server-side <meta> injection — same known
// limitation `<head>` Atom/vcs-git discovery (docs/DECISIONS.md #63) already
// documents, since that injection only happens in the production Rust
// binary. This test exercises `fillSiteChrome` directly instead: it injects
// the metas by hand and calls the function, verifying the client-side half
// of the feature in a real browser without needing the server half.
test("fillSiteChrome applies injected site metas to the brand, description, and title", async ({ page }) => {
  await page.route("**/api/v1/repos", async (route) => {
    await route.fulfill({ json: { repos: [], sort: "name" } });
  });
  await page.route("**/api/v1/site", async (route) => {
    await route.fulfill({ json: UNCONFIGURED });
  });

  await page.goto("/");
  await expect(page.getByRole("link", { name: "Axgit", exact: true })).toBeVisible();

  await page.evaluate(() => {
    const title = document.createElement("meta");
    title.setAttribute("name", "axgit:site-title");
    title.setAttribute("content", "PtCookie Git");
    document.head.appendChild(title);

    const desc = document.createElement("meta");
    desc.setAttribute("name", "axgit:site-desc");
    desc.setAttribute("content", "Self-hosted repositories");
    document.head.appendChild(desc);

    (window as unknown as { __axgit: { fillSiteChrome: () => void } }).__axgit.fillSiteChrome();
  });

  await expect(page.getByRole("link", { name: "PtCookie Git", exact: true })).toBeVisible();
  await expect(page).toHaveTitle("PtCookie Git");
  await expect(page.locator('meta[name="description"]')).toHaveAttribute("content", "Self-hosted repositories");
});
