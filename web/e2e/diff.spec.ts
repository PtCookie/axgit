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

const FROM_SHA = "aaa000111222aaa000111222aaa000111222aaa";
const TO_SHA = "bbb333444555bbb333444555bbb333444555bbb";

const REFS = {
  branches: [{ name: "main", target: TO_SHA, committed_at: "2026-07-24T13:06:00+09:00" }],
  tags: [{ name: "v1.0.0", target: FROM_SHA, annotation: "First release", tagged_at: "2026-01-01T00:00:00+09:00" }],
};

const REV_DIFF = {
  from: FROM_SHA,
  to: TO_SHA,
  truncated: false,
  diffstat: {
    files: [{ path: "a.txt", old_path: null, status: "modified", additions: 1, deletions: 1, binary: false }],
    files_changed: 1,
    total_additions: 1,
    total_deletions: 1,
  },
  files: [
    {
      path: "a.txt",
      old_path: null,
      status: "modified",
      additions: 1,
      deletions: 1,
      binary: false,
      truncated: false,
      hunks: [
        {
          header: "@@ -1,2 +1,2 @@",
          old_start: 1,
          old_lines: 2,
          new_start: 1,
          new_lines: 2,
          lines: [
            { origin: " ", content: "one", old_lineno: 1, new_lineno: 1 },
            { origin: "-", content: "two", old_lineno: 2, new_lineno: null },
            { origin: "+", content: "three", old_lineno: null, new_lineno: 2 },
          ],
        },
      ],
    },
  ],
};

test("navigates from the summary to the diff tab, showing the idle picker", async ({ page }) => {
  await page.route("**/api/v1/repos/git-compose", async (route) => {
    await route.fulfill({ json: SUMMARY });
  });
  await page.route("**/api/v1/repos/git-compose/readme*", async (route) => {
    await route.fulfill({ status: 404, json: NO_README });
  });
  await page.route("**/api/v1/repos/git-compose/refs", async (route) => {
    await route.fulfill({ json: REFS });
  });

  await page.goto("/git-compose");
  await page.getByRole("link", { name: "Diff" }).click();

  await expect(page).toHaveURL("/git-compose/diff");
  await expect(page.getByText("Pick two revisions to compare.")).toBeVisible();
});

// A link click navigating via `<ClientRouter />` (docs/DECISIONS.md #24) never
// tears down the JS realm — `window` survives. A form GET submit is a
// different code path from a link click (the browser, not an anchor,
// initiates the navigation), so it needs its own marker check rather than
// assuming the same interception applies (same pattern as repo.spec.ts).
test("filling in a comparison and submitting stays client-side and renders the diff", async ({ page }) => {
  await page.route("**/api/v1/repos/git-compose", async (route) => {
    await route.fulfill({ json: SUMMARY });
  });
  await page.route("**/api/v1/repos/git-compose/readme*", async (route) => {
    await route.fulfill({ status: 404, json: NO_README });
  });
  await page.route("**/api/v1/repos/git-compose/refs", async (route) => {
    await route.fulfill({ json: REFS });
  });
  await page.route("**/api/v1/repos/git-compose/diff*", async (route) => {
    await route.fulfill({ json: REV_DIFF });
  });

  await page.goto("/git-compose/diff");
  await page.evaluate(() => {
    (window as unknown as { __navMarker?: number }).__navMarker = 1;
  });

  await page.getByLabel("Compare from revision").fill(FROM_SHA);
  await page.getByLabel("Compare to revision").fill(TO_SHA);
  await page.getByRole("button", { name: "Compare" }).click();

  await expect(page).toHaveURL(`/git-compose/diff?from=${FROM_SHA}&to=${TO_SHA}`);
  await expect(page.getByText("Comparing")).toBeVisible();
  // Scoped to the diff table's own cells: a plain `getByText` for a short,
  // common word like "one" can also match the Astro dev toolbar's overlay
  // (dev-server only, not present in production), which lists the page's
  // islands by source path.
  await expect(page.getByRole("cell", { name: "one", exact: true })).toBeVisible();
  await expect(page.getByRole("cell", { name: "three", exact: true })).toBeVisible();
  expect(await page.evaluate(() => (window as unknown as { __navMarker?: number }).__navMarker)).toBe(1);
});

test("the (diff) link on a commit's parent leads to the compare page", async ({ page }) => {
  const COMMIT_SHA = TO_SHA;
  await page.route("**/api/v1/repos/git-compose", async (route) => {
    await route.fulfill({ json: SUMMARY });
  });
  await page.route("**/api/v1/repos/git-compose/readme*", async (route) => {
    await route.fulfill({ status: 404, json: NO_README });
  });
  await page.route("**/api/v1/repos/git-compose/refs", async (route) => {
    await route.fulfill({ json: REFS });
  });
  await page.route(`**/api/v1/repos/git-compose/commits/${COMMIT_SHA}`, async (route) => {
    await route.fulfill({
      json: {
        sha: COMMIT_SHA,
        summary: "fix: update a",
        message: "fix: update a\n",
        author: { name: "Ada Lovelace", email_hash: "deadbeef" },
        committer: { name: "Ada Lovelace", email_hash: "deadbeef" },
        authored_at: "2026-07-24T13:06:00+09:00",
        committed_at: "2026-07-24T13:06:00+09:00",
        parents: [FROM_SHA],
        diffstat: REV_DIFF.diffstat,
      },
    });
  });
  await page.route(`**/api/v1/repos/git-compose/commits/${COMMIT_SHA}/diff`, async (route) => {
    await route.fulfill({ json: { sha: COMMIT_SHA, parent: FROM_SHA, truncated: false, files: REV_DIFF.files } });
  });
  await page.route("**/api/v1/repos/git-compose/diff*", async (route) => {
    await route.fulfill({ json: REV_DIFF });
  });

  await page.goto(`/git-compose/commit/${COMMIT_SHA}`);
  await page.getByRole("link", { name: `Diff against parent ${FROM_SHA.slice(0, 12)}` }).click();

  await expect(page).toHaveURL(`/git-compose/diff?from=${FROM_SHA}&to=${COMMIT_SHA}`);
  await expect(page.getByText("Comparing")).toBeVisible();
});

test("toggling unified/split updates the URL and the rendered table shape", async ({ page }) => {
  await page.route("**/api/v1/repos/git-compose", async (route) => {
    await route.fulfill({ json: SUMMARY });
  });
  await page.route("**/api/v1/repos/git-compose/readme*", async (route) => {
    await route.fulfill({ status: 404, json: NO_README });
  });
  await page.route("**/api/v1/repos/git-compose/refs", async (route) => {
    await route.fulfill({ json: REFS });
  });
  await page.route("**/api/v1/repos/git-compose/diff*", async (route) => {
    await route.fulfill({ json: REV_DIFF });
  });

  await page.goto(`/git-compose/diff?from=${FROM_SHA}&to=${TO_SHA}`);
  await expect(page.getByRole("link", { name: "Unified" })).toHaveAttribute("aria-current", "page");
  // Unified: one origin-character column between the two line-number columns.
  await expect(page.getByRole("cell", { name: "-", exact: true })).toBeVisible();

  await page.getByRole("link", { name: "Split" }).click();

  await expect(page).toHaveURL(`/git-compose/diff?from=${FROM_SHA}&to=${TO_SHA}&view=split`);
  await expect(page.getByRole("link", { name: "Split" })).toHaveAttribute("aria-current", "page");
  // Split: the deleted line and the added line render in the same row,
  // each in its own column — both visible at once, unlike unified's single
  // interleaved column.
  await expect(page.getByRole("cell", { name: "two", exact: true })).toBeVisible();
  await expect(page.getByRole("cell", { name: "three", exact: true })).toBeVisible();
  // The origin-character column is dropped in split view.
  await expect(page.getByRole("cell", { name: "-", exact: true })).not.toBeVisible();
});

test("the refs page's Compare links lead to the diff page (branch and tag rows)", async ({ page }) => {
  await page.route("**/api/v1/repos/git-compose", async (route) => {
    await route.fulfill({ json: SUMMARY });
  });
  await page.route("**/api/v1/repos/git-compose/refs", async (route) => {
    await route.fulfill({ json: REFS });
  });
  await page.route("**/api/v1/repos/git-compose/diff*", async (route) => {
    await route.fulfill({ json: REV_DIFF });
  });

  await page.goto("/git-compose/refs");

  // The default branch (`main`, from `SUMMARY.default_branch`) has no
  // Compare link on its own row — only `—`.
  await expect(page.getByRole("link", { name: "Compare main with main" })).toHaveCount(0);

  await page.getByRole("link", { name: "Compare v1.0.0 with main" }).click();

  await expect(page).toHaveURL("/git-compose/diff?from=v1.0.0&to=main");
  await expect(page.getByText("Comparing")).toBeVisible();
});
