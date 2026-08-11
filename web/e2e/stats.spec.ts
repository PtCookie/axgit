import { expect, test } from "@playwright/test";

const SUMMARY = {
  name: "git-compose",
  section: "infra",
  owner: "PtCookie",
  description: "Compose project of Git server",
  default_branch: "main",
  last_modified: "2026-07-24T13:06:00+09:00",
  head: "abc123def456",
  branch_count: 1,
  tag_count: 1,
  clone_url: "git@git.ptcookie.net:git-compose.git",
};

const NO_README = { error: { code: "path_not_found", message: "no readme found" } };

const COMMIT_SHA = "abc123def456abc123def456abc123def456abc";

/** 12 ascending buckets (fixed `BUCKET_COUNT`, docs/DECISIONS.md #28), all
 *  empty except the last (`commits`), which is where the fixture author's
 *  activity lands regardless of `period`. */
function buckets(commits: number) {
  return Array.from({ length: 12 }, (_, index) => ({
    start: `2026-${String((index % 12) + 1).padStart(2, "0")}-01T00:00:00+00:00`,
    commits: index === 11 ? commits : 0,
  }));
}

const MONTH_STATS = {
  sha: COMMIT_SHA,
  period: "month",
  truncated: false,
  author_count: 1,
  buckets: buckets(3),
  authors: [
    {
      author: { name: "Ada Lovelace", email_hash: "deadbeef" },
      commits: 3,
      buckets: buckets(3).map((bucket) => bucket.commits),
    },
  ],
};

const QUARTER_STATS = { ...MONTH_STATS, period: "quarter" };

/** One handler for both `period=month` (the tab's default landing URL) and
 *  `period=quarter` (after clicking the period link) — `getStats` always
 *  sends an explicit `?period=`, so the query param alone decides the
 *  fixture. Avoids relying on Playwright's route-registration-order
 *  semantics for two overlapping glob patterns. */
async function routeStats(page: import("@playwright/test").Page) {
  await page.route("**/api/v1/repos/git-compose/stats*", async (route) => {
    const period = new URL(route.request().url()).searchParams.get("period");
    await route.fulfill({ json: period === "quarter" ? QUARTER_STATS : MONTH_STATS });
  });
}

test("shows commit statistics and switches the bucket period via a link", async ({ page }) => {
  await page.route("**/api/v1/repos/git-compose", async (route) => {
    await route.fulfill({ json: SUMMARY });
  });
  await page.route("**/api/v1/repos/git-compose/readme*", async (route) => {
    await route.fulfill({ status: 404, json: NO_README });
  });
  await routeStats(page);

  await page.goto("/git-compose");
  await page.getByRole("link", { name: "Stats" }).click();

  await expect(page).toHaveURL("/git-compose/stats");
  await expect(page.getByText("Ada Lovelace")).toBeVisible();
  await expect(page.getByRole("link", { name: "Month" })).toHaveAttribute("aria-current", "page");

  await page.getByRole("link", { name: "Quarter" }).click();

  await expect(page).toHaveURL("/git-compose/stats?period=quarter");
  await expect(page.getByRole("link", { name: "Quarter" })).toHaveAttribute("aria-current", "page");
  await expect(page.getByText("Ada Lovelace")).toBeVisible();
});

test("shows a path-filter banner and sends path to the api", async ({ page }) => {
  await page.route("**/api/v1/repos/git-compose", async (route) => {
    await route.fulfill({ json: SUMMARY });
  });
  await page.route("**/api/v1/repos/git-compose/readme*", async (route) => {
    await route.fulfill({ status: 404, json: NO_README });
  });
  let requestedUrl: string | undefined;
  await page.route("**/api/v1/repos/git-compose/stats*", async (route) => {
    requestedUrl = route.request().url();
    await route.fulfill({ json: MONTH_STATS });
  });

  await page.goto("/git-compose/stats?path=src%2Fmain.rs");

  await expect(page.getByText("Filtered by path")).toBeVisible();
  await expect(page.getByText("src/main.rs")).toBeVisible();
  expect(requestedUrl).toBeDefined();
  expect(new URL(requestedUrl ?? "").searchParams.get("path")).toBe("src/main.rs");

  // The clear-filter link carries the resolved period forward (same as every
  // other `statsHref` call on this page) — it only drops `path`.
  await page.getByRole("link", { name: "clear filter" }).click();
  await expect(page).toHaveURL("/git-compose/stats?period=month");
  await expect(page.getByText("Filtered by path")).not.toBeVisible();
});

// A link click navigating via `<ClientRouter />` (docs/DECISIONS.md #24) never
// tears down the JS realm — `window` survives. A silently downgraded full
// reload would still land on the same URL and pass every other assertion in
// this file, so this marker is the only signal that a test is actually
// exercising client-side routing (same pattern as repo.spec.ts).
test("stats tab navigation is client-side, not a full page reload", async ({ page }) => {
  await page.route("**/api/v1/repos/git-compose", async (route) => {
    await route.fulfill({ json: SUMMARY });
  });
  await page.route("**/api/v1/repos/git-compose/readme*", async (route) => {
    await route.fulfill({ status: 404, json: NO_README });
  });
  await routeStats(page);

  await page.goto("/git-compose");
  await page.evaluate(() => {
    (window as unknown as { __navMarker?: number }).__navMarker = 1;
  });

  await page.getByRole("link", { name: "Stats" }).click();

  await expect(page).toHaveURL("/git-compose/stats");
  await expect(page.getByText("Ada Lovelace")).toBeVisible();
  expect(await page.evaluate(() => (window as unknown as { __navMarker?: number }).__navMarker)).toBe(1);
});
