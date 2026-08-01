import { expect, test } from "@playwright/test";

import fixture from "../tests/fixtures/repos.json" with { type: "json" };

test("shows repositories grouped by section", async ({ page }) => {
  await page.route("**/api/v1/repos", async (route) => {
    await route.fulfill({ json: fixture });
  });

  await page.goto("/");

  await expect(page.getByRole("heading", { name: "infra", level: 2 })).toBeVisible();
  await expect(page.getByText("git-compose")).toBeVisible();
  await expect(page.getByRole("heading", { name: "Other", level: 2 })).toBeVisible();
});
