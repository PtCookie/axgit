import { expect, test, type Page } from "@playwright/test";

/**
 * `AXGIT_SYNTAX_THEME_LIGHT`/`_DARK` reach the frontend as `<meta>`s injected
 * into every served shell (`api/src/shell.rs::site_head_meta`,
 * docs/DECISIONS.md #95). `astro dev` injects none of its own, so these tests
 * add them with `addInitScript` — before any page script runs, which is what
 * matters: the highlighter reads them once, when its singleton is built.
 *
 * A fresh page load per case is the point. The same coverage can't live in
 * `tests/format/highlight.test.ts`, where every case shares one already-built
 * singleton.
 */

const SUMMARY = {
  name: "git-compose",
  section: "infra",
  owner: "PtCookie",
  description: "Compose project of Git server",
  homepage: null,
  default_branch: "main",
  last_modified: "2026-07-24T13:06:00+09:00",
  head: "abc123def456",
  branch_count: 1,
  tag_count: 1,
  clone_url: "git@git.ptcookie.net:git-compose.git",
};

const README = {
  path: "README.md",
  format: "markdown",
  content: "# Getting started\n\n```sh\necho hello\n```\n",
};

async function stubRepo(page: Page) {
  await page.route("**/api/v1/repos/git-compose", async (route) => {
    await route.fulfill({ json: SUMMARY });
  });
  await page.route("**/api/v1/repos/git-compose/readme*", async (route) => {
    await route.fulfill({ json: README });
  });
}

async function injectThemeMetas(page: Page, themes: { light?: string; dark?: string }) {
  await page.addInitScript((configured: { light?: string; dark?: string }) => {
    // Runs at document start, before <head> exists — hence the wait.
    document.addEventListener("DOMContentLoaded", () => {
      for (const [mode, id] of Object.entries(configured)) {
        const meta = document.createElement("meta");
        meta.setAttribute("name", `axgit:syntax-theme-${mode}`);
        meta.setAttribute("content", id);
        document.head.appendChild(meta);
      }
    });
  }, themes);
}

/** The inline `color` and `--shiki-dark` of the README fence's first
 *  non-whitespace token, once highlighting has run. */
async function fenceTokenStyle(page: Page) {
  await stubRepo(page);
  await page.goto("/git-compose");
  await expect(page.getByText("echo hello")).toBeVisible();

  const token = page.locator("pre.shiki-code span").filter({ hasText: /\S/ }).first();
  await expect(token).toHaveAttribute("style", /color/);

  return token.evaluate((element: HTMLElement) => ({
    color: element.style.color,
    dark: element.style.getPropertyValue("--shiki-dark"),
  }));
}

test("highlights with the configured light and dark themes", async ({ page }) => {
  const unconfigured = await fenceTokenStyle(page);

  await injectThemeMetas(page, { light: "one-light", dark: "dracula" });
  const configured = await fenceTokenStyle(page);

  expect(configured.color).not.toBe(unconfigured.color);
  expect(configured.dark).not.toBe(unconfigured.dark);
  // Both themes still travel on the token, so the mode toggle stays a pure
  // CSS switch with no re-highlight.
  expect(configured.color).not.toBe("");
  expect(configured.dark).not.toBe("");
});

test("falls back to the default theme and warns for an unknown theme id", async ({ page }) => {
  const unconfigured = await fenceTokenStyle(page);

  const warnings: string[] = [];
  page.on("console", (message) => {
    if (message.type() === "warning") warnings.push(message.text());
  });

  await injectThemeMetas(page, { dark: "githbu-dark" });
  const misconfigured = await fenceTokenStyle(page);

  // The API passes the value through unvalidated (docs/DECISIONS.md #95), so
  // this warning is the only place a typo surfaces.
  expect(warnings.some((text) => text.includes("githbu-dark"))).toBe(true);
  expect(misconfigured.dark).toBe(unconfigured.dark);
});
