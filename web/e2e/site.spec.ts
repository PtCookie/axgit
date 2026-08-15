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
test("fillSiteChrome applies injected site metas to the brand, description, title, logo, and logo link", async ({
  page,
}) => {
  await page.route("**/api/v1/repos", async (route) => {
    await route.fulfill({ json: { repos: [], sort: "name" } });
  });
  await page.route("**/api/v1/site", async (route) => {
    await route.fulfill({ json: UNCONFIGURED });
  });
  // The image actually has to load for the `toBeVisible()` assertions below
  // to hold — `fillSiteChrome`'s `onerror` handler (docs/DECISIONS.md #85)
  // falls back to the default mark on a failed load, and nothing serves this
  // path in the `astro dev` e2e environment otherwise.
  await page.route("**/api/v1/site/logo", async (route) => {
    await route.fulfill({
      contentType: "image/svg+xml",
      body: '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><circle cx="12" cy="12" r="10"/></svg>',
    });
  });

  await page.goto("/");
  await expect(page.getByRole("link", { name: "Axgit", exact: true })).toBeVisible();
  // Unconfigured: the logo image is already visible at axgit's own default
  // mark, and the brand link still points at `/` (docs/DECISIONS.md #85).
  await expect(page.locator("[data-site-logo]")).toBeVisible();
  await expect(page.locator("[data-site-logo]")).toHaveAttribute("src", "/favicon.svg");
  await expect(page.locator("[data-site-brand]")).toHaveAttribute("href", "/");

  await page.evaluate(() => {
    const title = document.createElement("meta");
    title.setAttribute("name", "axgit:site-title");
    title.setAttribute("content", "PtCookie Git");
    document.head.appendChild(title);

    const desc = document.createElement("meta");
    desc.setAttribute("name", "axgit:site-desc");
    desc.setAttribute("content", "Self-hosted repositories");
    document.head.appendChild(desc);

    const logo = document.createElement("meta");
    logo.setAttribute("name", "axgit:logo");
    logo.setAttribute("content", "/api/v1/site/logo");
    document.head.appendChild(logo);

    const logoLink = document.createElement("meta");
    logoLink.setAttribute("name", "axgit:logo-link");
    logoLink.setAttribute("content", "https://example.net");
    document.head.appendChild(logoLink);

    (window as unknown as { __axgit: { fillSiteChrome: () => void } }).__axgit.fillSiteChrome();
  });

  await expect(page.getByRole("link", { name: "PtCookie Git", exact: true })).toBeVisible();
  await expect(page).toHaveTitle("PtCookie Git");
  await expect(page.locator('meta[name="description"]')).toHaveAttribute("content", "Self-hosted repositories");
  await expect(page.locator("[data-site-logo]")).toBeVisible();
  await expect(page.locator("[data-site-logo]")).toHaveAttribute("src", "/api/v1/site/logo");
  await expect(page.locator("[data-site-brand]")).toHaveAttribute("href", "https://example.net");
});

// docs/DECISIONS.md #85: `AXGIT_LOGO` being *configured* (the meta is
// present) doesn't guarantee the file actually loads — shell.rs can't
// re-verify that per shell response without defeating the point of serving
// it with `Cache-Control: no-cache`. This exercises the client-side
// recovery: a load failure falls back to axgit's own default mark instead of
// leaving a broken-image icon in the header.
test("a logo that fails to load falls back to axgit's own mark rather than shown broken", async ({ page }) => {
  await page.route("**/api/v1/repos", async (route) => {
    await route.fulfill({ json: { repos: [], sort: "name" } });
  });
  await page.route("**/api/v1/site", async (route) => {
    await route.fulfill({ json: UNCONFIGURED });
  });
  await page.route("**/api/v1/site/logo", async (route) => {
    await route.fulfill({ status: 404, json: { error: { code: "not_found", message: "not found" } } });
  });

  await page.goto("/");
  await expect(page.locator("[data-site-logo]")).toHaveAttribute("src", "/favicon.svg");

  await page.evaluate(() => {
    const logo = document.createElement("meta");
    logo.setAttribute("name", "axgit:logo");
    logo.setAttribute("content", "/api/v1/site/logo");
    document.head.appendChild(logo);

    (window as unknown as { __axgit: { fillSiteChrome: () => void } }).__axgit.fillSiteChrome();
  });

  await expect(page.locator("[data-site-logo]")).toBeVisible();
  await expect(page.locator("[data-site-logo]")).toHaveAttribute("src", "/favicon.svg");
});
