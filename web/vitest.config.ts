/// <reference types="vitest/config" />
import { getViteConfig } from "astro/config";
import { playwright } from "@vitest/browser-playwright";

export default getViteConfig({
  test: {
    include: ["tests/**/*.test.{ts,tsx}"],
    // Jenkins runs the `web` and `api` Test stages in parallel on one agent, so vitest's
    // browser-mode tests (3 browser instances, each re-fetching any lazy-loaded chunk per
    // test — see ReadmeView.test.tsx) compete for CPU with cargo/clippy. That occasionally
    // pushes a chunk fetch past even a generously bumped `expect.element` timeout. Retrying
    // only in CI re-runs just the failed test under (usually) less contention, without
    // masking a genuinely broken assertion during local development.
    retry: process.env.CI ? 2 : 0,
    browser: {
      enabled: true,
      provider: playwright(),
      headless: true,
      instances: [{ browser: "chromium" }, { browser: "firefox" }, { browser: "webkit" }],
    },
  },
});
