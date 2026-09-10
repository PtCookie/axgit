import { test as base, expect } from "@playwright/test";

/**
 * Matches the app's own API calls — and nothing else. A double-star glob
 * around `/api/` looks equivalent but is not: `astro dev` serves the app's
 * unbundled source, so it would also swallow `/src/lib/api/client.ts` and
 * leave the page unhydrated (which it did, on the first run of this guard).
 */
const isApiRequest = (url: URL) => url.pathname.startsWith("/api/");

/**
 * The e2e suite runs against `astro dev` with no axgit binary behind it
 * (docs/DECISIONS.md #92, #96): every `/api` response a spec relies on is
 * stubbed with `page.route`. Anything a spec *forgets* to stub used to fall
 * through to the dev server's `/api` proxy and out to `127.0.0.1:8080`,
 * where nothing is listening — the island then rendered its error fallback
 * and the test passed anyway, as long as it didn't assert on that data.
 * That made a missing stub invisible, and it made a local run (where the
 * developer may well have an axgit serving `fixtures/repos` on 8080) pass
 * for a different reason than CI's.
 *
 * This fixture closes both: a catch-all route answers every unstubbed `/api`
 * request with a 503 — so nothing reaches the proxy, and the ECONNREFUSED
 * stack traces vite logs for it disappear at the source — and the test fails
 * with the offending URLs.
 *
 * Two ordering facts it rests on, both verified against the installed
 * packages rather than the docs:
 *
 *   * `page.route` registers with `_routes.unshift(...)`
 *     (playwright-core 1.62.1, `lib/coreBundle.js`), i.e. the *last* route
 *     registered is matched *first*. An auto fixture is set up before
 *     `beforeEach` hooks and before the test body, so this catch-all is
 *     always the last resort — a spec's own stubs win without any of them
 *     having to know it exists.
 *   * The part of a fixture after `await use()` runs after the `afterEach`
 *     hooks and before the `page` fixture disposes. Requests that arrive in
 *     that window are answered but not counted: a fetch still in flight when
 *     the body ended is a teardown race, not a missing stub — the fix for
 *     those is awaiting the page's islands before the test returns, which
 *     the navigation specs do.
 */
// eslint-disable-next-line @typescript-eslint/no-invalid-void-type -- Playwright's own spelling for a fixture that yields no value
export const test = base.extend<{ strictApi: void }>({
  strictApi: [
    async ({ page }, use, testInfo) => {
      const unstubbed = new Set<string>();
      let inTestBody = true;

      await page.route(isApiRequest, async (route) => {
        if (inTestBody) {
          const url = new URL(route.request().url());
          unstubbed.add(`${url.pathname}${url.search}`);
        }
        await route.fulfill({
          status: 503,
          json: { error: { code: "unavailable", message: "unmocked API request" } },
        });
      });

      await use();

      inTestBody = false;
      // Don't pile onto a test that already failed — the missing stub is
      // usually a symptom of that failure, not a second finding.
      if (unstubbed.size > 0 && testInfo.errors.length === 0) {
        throw new Error(
          `The page requested ${unstubbed.size} API endpoint(s) this test never stubbed:\n` +
            [...unstubbed].map((url) => `  ${url}`).join("\n") +
            `\nAdd a page.route() stub for each — the e2e suite runs with no API behind it, ` +
            `so an unstubbed endpoint means the assertions ran against an error fallback.`,
        );
      }
    },
    { auto: true },
  ],
});

export { expect };
