import { expect, test } from "@playwright/test";

// `/{repo}/blob` with no path segment is an unmatched route shape
// (`lib/shell.ts::shellFor` — there's nothing to show for a bare blob URL),
// so it serves `pages/404.astro` — the only page in the app that renders the
// real `Layout.astro` with no islands other than the theme toggle itself,
// and therefore no API calls to stub. (`/{repo}/stats` used to serve this
// role too, until DECISIONS.md #29 gave it a real page.)
const PAGE = "/git-compose/blob";
const STORAGE_KEY = "axgit:theme";

const preference = (page: import("@playwright/test").Page) =>
  page.evaluate(() => document.documentElement.dataset.theme);
const isDark = (page: import("@playwright/test").Page) =>
  page.evaluate(() => document.documentElement.classList.contains("dark"));
const stored = (page: import("@playwright/test").Page) =>
  page.evaluate((key) => localStorage.getItem(key), STORAGE_KEY);

async function choose(page: import("@playwright/test").Page, label: "System" | "Light" | "Dark") {
  await page.getByRole("button", { name: "Theme" }).click();
  await page.getByRole("menuitemradio", { name: label }).click();
}

test.describe("with a dark OS preference", () => {
  test.use({ colorScheme: "dark" });

  test("defaults to system and resolves dark", async ({ page }) => {
    await page.goto(PAGE);

    expect(await preference(page)).toBe("system");
    expect(await isDark(page)).toBe(true);
    expect(await stored(page)).toBeNull(); // nothing written until a choice is made
  });

  test("an explicit Light choice overrides the OS preference", async ({ page }) => {
    await page.goto(PAGE);
    await choose(page, "Light");

    expect(await preference(page)).toBe("light");
    expect(await isDark(page)).toBe(false);
    expect(await stored(page)).toBe("light");
  });
});

test.describe("with a light OS preference", () => {
  test.use({ colorScheme: "light" });

  test("defaults to system and resolves light", async ({ page }) => {
    await page.goto(PAGE);

    expect(await preference(page)).toBe("system");
    expect(await isDark(page)).toBe(false);
  });

  // The regression test for the head script: a stored value applied late,
  // after paint, is exactly the flash this design exists to prevent.
  test("an explicit Dark choice survives a reload", async ({ page }) => {
    await page.goto(PAGE);
    await choose(page, "Dark");

    await page.reload();

    expect(await preference(page)).toBe("dark");
    expect(await isDark(page)).toBe(true);
    await page.getByRole("button", { name: "Theme" }).click();
    await expect(page.getByRole("menuitemradio", { name: "Dark" })).toHaveAttribute("aria-checked", "true");
  });

  test("tracks OS changes only while set to System", async ({ page }) => {
    // Chromium's CDP-based `colorScheme` emulation updates
    // `matchMedia(...).matches` immediately (confirmed manually) but does
    // not reliably dispatch the MediaQueryList "change" event afterwards —
    // a known CDP/Playwright limitation, not an app bug. Firefox and WebKit
    // dispatch it natively, so the manual dispatch below is a harmless
    // no-op duplicate there. Browsers also mint a fresh `MediaQueryList`
    // object per `matchMedia()` call, so a "change" event dispatched on one
    // from here wouldn't reach a listener the app attached to a different
    // one. Pinning the query to a single, shared object works around both:
    // `emulateMedia` supplies the `.matches` value the app's listener
    // reads, and dispatching on the shared object reaches that exact
    // listener — the same notification a real OS-level toggle sends.
    await page.addInitScript(() => {
      const query = "(prefers-color-scheme: dark)";
      const shared = matchMedia(query);
      const original = window.matchMedia.bind(window);
      window.matchMedia = (q) => (q === query ? shared : original(q));
      Object.assign(window, { __darkQuery: shared });
    });
    const dispatchDarkChange = () =>
      page.evaluate(() =>
        (window as unknown as { __darkQuery: MediaQueryList }).__darkQuery.dispatchEvent(new Event("change")),
      );

    await page.goto(PAGE);
    // The listener this test exercises is registered by the toggle island's
    // `useEffect`, which only runs once it hydrates — wait for that so the
    // dispatch below isn't racing it (the fallback button stays `disabled`
    // until then).
    await expect(page.getByRole("button", { name: "Theme" })).toBeEnabled();

    await page.emulateMedia({ colorScheme: "dark" });
    await dispatchDarkChange();
    await expect.poll(() => isDark(page)).toBe(true);

    await choose(page, "Light");
    await dispatchDarkChange();
    await expect.poll(() => isDark(page)).toBe(false);
  });
});

// The page shells are prerendered once per route shape and served to every
// visitor with `Cache-Control: no-cache` (`api/src/shell.rs`,
// docs/DECISIONS.md #17), so the theme must never be baked into the HTML —
// only applied by script. Checked against just the `<html>` tag itself:
// `astro dev` inlines the compiled stylesheet as a literal `<style>` block
// (production serves it via `<link>`), and that stylesheet's own
// `html[data-theme="…"]` selectors (the icon-visibility rule in
// `global.css`) would otherwise false-positive a whole-document substring
// check.
test("the served shell carries no theme of its own", async ({ page }) => {
  const html = await (await page.request.get(PAGE)).text();
  const openingTag = html.match(/<html[^>]*>/)?.[0];

  expect(openingTag).toBeDefined();
  expect(openingTag).not.toMatch(/\bdata-theme=/);
  expect(openingTag).not.toMatch(/\bclass=/);
});

// Regression test for the `<ClientRouter />` swap (docs/DECISIONS.md #24):
// Astro's swap replaces every attribute on <html> with the incoming
// document's, and the served shell carries none of its own (asserted right
// below), so a same-shell navigation resets the theme unless
// `Layout.astro`'s theme script re-applies it from `astro:after-swap`. This
// page (`PAGE`, the 404 shell) has a "Back to repository list" link to `/`
// to navigate with, with no API stub needed for either endpoint.
test("an explicit Dark choice survives a client-side navigation", async ({ page }) => {
  await page.goto(PAGE);
  await choose(page, "Dark");
  // Selecting a radio item doesn't close the menu (DECISIONS.md #23's
  // documented Base UI default), and the open menu's portal overlay
  // otherwise intercepts the click below.
  await page.keyboard.press("Escape");

  await page.getByRole("link", { name: "Back to repository list" }).click();
  await expect(page).toHaveURL("/");

  expect(await preference(page)).toBe("dark");
  expect(await isDark(page)).toBe(true);
});

// No-flash guard. There is no API that observes "the class was set before
// first paint" — anything queryable from `page.evaluate` already runs after
// it — so what's asserted instead is the structural property that
// guarantees it: the theme script is a synchronous, classic, in-head
// script. If it is ever converted to a bundled `<script>`, Astro turns it
// into a deferred `type="module"` in a separate file and this fails.
test("the theme script runs synchronously in the head", async ({ page }) => {
  await page.goto(PAGE);

  expect(
    await page.evaluate(() => {
      const script = document.head.querySelector("script[data-theme-init]");
      if (!script || !(script instanceof HTMLScriptElement)) return null;
      return { parent: script.parentElement?.tagName, type: script.type, defer: script.defer, src: script.src };
    }),
  ).toEqual({ parent: "HEAD", type: "", defer: false, src: "" });
});
