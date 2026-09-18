import { defineConfig, devices } from "@playwright/test";

/**
 * Read environment variables from file.
 * https://github.com/motdotla/dotenv
 */
// import dotenv from 'dotenv';
// import path from 'path';
// dotenv.config({ path: path.resolve(__dirname, '.env') });

/**
 * See https://playwright.dev/docs/test-configuration.
 */
export default defineConfig({
  testDir: "./e2e",
  /* Run tests in files in parallel */
  fullyParallel: true,
  /* Fail the build on CI if you accidentally left test.only in the source code. */
  forbidOnly: !!process.env.CI,
  /* Retry on CI only */
  retries: process.env.CI ? 2 : 0,
  /* Opt out of parallel tests on CI. */
  workers: process.env.CI ? 1 : undefined,
  /* Reporter to use. See https://playwright.dev/docs/test-reporters */
  reporter: "html",
  /* Shared settings for all the projects below. See https://playwright.dev/docs/api/class-testoptions. */
  use: {
    /* Base URL to use in actions like `await page.goto('')`. */
    baseURL: "http://localhost:4321",
    /* Collect trace when retrying the failed test. See https://playwright.dev/docs/trace-viewer */
    trace: "on-first-retry",
  },

  /* Configure projects for major browsers */
  projects: [
    {
      name: "chromium",
      use: { ...devices["Desktop Chrome"] },
    },
    {
      name: "Mobile Chrome",
      use: { ...devices["Pixel 5"] },
    },

    /* Test against minor browsers on CI. */
    ...(process.env.CI
      ? [
          {
            name: "firefox",
            use: { ...devices["Desktop Firefox"] },
          },
          {
            name: "webkit",
            use: { ...devices["Desktop Safari"] },
          },
          {
            name: "Mobile Safari",
            use: { ...devices["iPhone 12"] },
          },
        ]
      : []),

    /* Test against branded browsers. */
    // {
    //   name: 'Microsoft Edge',
    //   use: { ...devices['Desktop Edge'], channel: 'msedge' },
    // },
    // {
    //   name: 'Google Chrome',
    //   use: { ...devices['Desktop Chrome'], channel: 'chrome' },
    // },
  ],

  /* Two modes, chosen by whether `AXGIT_E2E_SERVER` is set (docs/DECISIONS.md
   * #97):
   *
   *   * Locally: nothing sets it, so this spawns `astro dev` as before — no
   *     axgit runs behind that server (docs/DECISIONS.md #92), the specs
   *     stub `/api` themselves, and `AXGIT_E2E=1` makes `astro.config.mjs`
   *     drop the `/api` proxy for it, so a request that escapes interception
   *     while a page is closing gets a local 503 instead of an ECONNREFUSED
   *     stack trace in the run's log (docs/DECISIONS.md #96).
   *   * On CI: the `e2e` workflow job sets `AXGIT_E2E_SERVER` to a real axgit
   *     binary invocation and this spawns that instead. Not `astro preview`:
   *     its static-output preview server discards every user Vite plugin
   *     wholesale, which is where the `/{repo}/…` → `__repo__` shell
   *     rewrite and cgit redirects live (`astro.config.mjs`'s
   *     `shellFallback()`) — every `/{repo}/*` navigation this suite makes
   *     would 404 under it. The binary implements that routing for real
   *     (`api/src/shell.rs`), and still has no `/api` behind it either, so
   *     the same `page.route` stubs apply unchanged.
   */
  webServer: {
    command: process.env.AXGIT_E2E_SERVER ?? "pnpm run dev",
    url: "http://localhost:4321",
    // Not just a CI/local split: a binary invocation with no `AXGIT_E2E_SERVER`
    // fallback should fail loudly rather than quietly reuse (or spawn) a dev
    // server that skips the very routing this mode exists to exercise.
    reuseExistingServer: !process.env.CI,
    // Only meaningful for the `astro dev` invocation above — an axgit binary
    // reads its own `AXGIT_*` settings from CLI flags, not these.
    env: process.env.AXGIT_E2E_SERVER ? {} : { ASTRO_DEV_BACKGROUND: "0", AXGIT_E2E: "1" },
  },
});
