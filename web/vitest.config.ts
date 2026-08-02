/// <reference types="vitest/config" />
import { getViteConfig } from "astro/config";
import { playwright } from "@vitest/browser-playwright";

export default getViteConfig({
  test: {
    include: ["tests/**/*.test.{ts,tsx}"],
    browser: {
      enabled: true,
      provider: playwright(),
      headless: true,
      instances: [{ browser: "chromium" }, { browser: "firefox" }, { browser: "webkit" }],
    },
  },
});
