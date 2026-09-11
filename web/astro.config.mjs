// @ts-check
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { defineConfig } from "astro/config";
import tailwindcss from "@tailwindcss/vite";
import react from "@astrojs/react";

import { redirectFor } from "./src/lib/cgit-compat.ts";
import { shellFor } from "./src/lib/shell.ts";
import { REPO_SHELL_PARAM, shellRoutesManifest } from "./src/lib/shell-routes.ts";

const PUBLIC_DIR = path.join(fileURLToPath(new URL(".", import.meta.url)), "public");

/**
 * True when `pathname` resolves to a real file under `public/` — those are
 * served as-is (favicon, etc.) and must never be rewritten onto a page
 * shell. `path.normalize` + a prefix check keeps a `..`-laden pathname from
 * escaping `public/` (this only decides whether to skip a rewrite, so a
 * false negative just falls through to `shellFor`, not a filesystem read of
 * arbitrary content).
 *
 * @param {string} pathname
 */
function isPublicAsset(pathname) {
  const resolved = path.normalize(path.join(PUBLIC_DIR, pathname));
  if (!resolved.startsWith(PUBLIC_DIR + path.sep)) return false;
  return fs.existsSync(resolved) && fs.statSync(resolved).isFile();
}

/**
 * Rewrites HTML navigations onto the prerendered page shell for their route
 * shape, mirroring the production static fallback (`api/src/shell.rs`,
 * docs/DECISIONS.md #17). `astro dev` serves `/{repo}` from the placeholder
 * route it actually built (`src/pages/[repo]/`), so dev and production
 * render the same file. Only affects `astro dev` / Playwright against it —
 * the static build itself is untouched.
 *
 * The mapping lives in `src/lib/shell.ts` so it has one JS definition and a
 * unit test; only the request plumbing is here.
 *
 * A request path's *shape* (not its extension) decides whether it's an app
 * route — a blob path like `/{repo}/blob/src/main.rs` has a dot in it but is
 * still an app route, so an `extname`-based check would wrongly skip it (the
 * bug this replaced). Only an actual `public/` file is left alone.
 *
 * @returns {NonNullable<import("astro").ViteUserConfig["plugins"]>[number]}
 */
function shellFallback() {
  return {
    name: "axgit-shell-fallback",
    configureServer(server) {
      server.middlewares.use((req, res, next) => {
        const [pathname, query] = (req.url ?? "/").split("?");
        const accept = req.headers.accept ?? "";
        // A real browser navigation asks for `text/html`. Astro's
        // `<ClientRouter />` (docs/DECISIONS.md #24) instead calls plain
        // `fetch(href)` with no adapter-supplied headers, which Chrome/Firefox
        // send as `Accept: */*` with `Sec-Fetch-Dest: empty` — so relying on
        // `accept` alone would make every client-side navigation 404 in dev
        // (and under Playwright, which runs against `astro dev`) and silently
        // fall back to a full reload. `sec-fetch-dest` isn't sent by non-browser
        // clients (e.g. `curl`), but those fall through to `next()` untouched
        // either way, so this only ever widens which *browser* requests match.
        const isNavigation =
          (req.method === "GET" || req.method === "HEAD") &&
          (accept.includes("text/html") || req.headers["sec-fetch-dest"] === "empty");
        const isAppRoute =
          !pathname.startsWith("/api") &&
          !pathname.startsWith("/@") &&
          !pathname.startsWith("/_astro") &&
          !pathname.startsWith("/src") &&
          !pathname.startsWith("/node_modules") &&
          !isPublicAsset(pathname);

        if (isNavigation && isAppRoute) {
          // cgit-compatibility redirects (docs/DECISIONS.md #35) run first —
          // a `.git`-suffixed or cgit-query-shaped path gets a real redirect,
          // mirroring `api/src/shell.rs::serve_shell_or_redirect` — before
          // falling through to the shell rewrite below.
          const redirect = redirectFor(pathname, query ?? "");
          if (redirect !== null) {
            res.writeHead(308, { Location: redirect });
            res.end();
            return;
          }

          const target = shellFor(pathname);
          // `shellFor` is idempotent on its own targets, so this also leaves
          // a direct request to `/` or `/__repo__` alone.
          if (target !== pathname) {
            req.url = query ? `${target}?${query}` : target;
          }
        }
        next();
      });
    },
  };
}

/**
 * Emits the route-shape table to `dist/shell-routes.json` so the production
 * server can read the same mapping the dev middleware uses, instead of
 * mirroring it in a second hand-written matcher (`api/src/shell.rs`,
 * docs/DECISIONS.md #88).
 *
 * It also cross-checks the table against what was actually built, in both
 * directions — a declared route with no shell, or a built shell nobody
 * declared, fails the build. That's what keeps `src/pages/[repo]/` and
 * `src/lib/shell-routes.ts` from drifting apart: forgetting the table entry
 * for a new page is a build error, not a 404 discovered later.
 *
 * @returns {import("astro").AstroIntegration}
 */
function shellRoutes() {
  return {
    name: "axgit-shell-routes",
    hooks: {
      "astro:build:done": ({ dir, logger }) => {
        const outDir = fileURLToPath(dir);
        const manifest = shellRoutesManifest();

        /** Built shell directory for a route, relative to the build root. */
        const shellFile = (/** @type {string | null} */ shell) =>
          shell === null ? path.join(REPO_SHELL_PARAM, "index.html") : path.join(REPO_SHELL_PARAM, shell, "index.html");

        const declared = new Set();
        for (const route of manifest.routes) {
          const relative = shellFile(route.shell);
          if (!fs.existsSync(path.join(outDir, relative))) {
            throw new Error(
              `shell-routes: route ${JSON.stringify(route.segment)} declares shell ` +
                `${relative}, which the build did not produce — add the matching page ` +
                `under src/pages/[repo]/ or drop the entry from src/lib/shell-routes.ts`,
            );
          }
          if (route.shell !== null) declared.add(route.shell);
        }

        const shellRoot = path.join(outDir, REPO_SHELL_PARAM);
        for (const entry of fs.readdirSync(shellRoot, { withFileTypes: true })) {
          if (!entry.isDirectory() || declared.has(entry.name)) continue;
          throw new Error(
            `shell-routes: built shell ${path.join(REPO_SHELL_PARAM, entry.name)} is not ` +
              `declared in src/lib/shell-routes.ts — add an entry for it, or the server ` +
              `will never route to it`,
          );
        }

        fs.writeFileSync(path.join(outDir, "shell-routes.json"), `${JSON.stringify(manifest, null, 2)}\n`);
        logger.info(`emitted shell-routes.json (${manifest.routes.length} routes)`);
      },
    },
  };
}

/**
 * True while the Playwright suite is running (`playwright.config.ts` sets it
 * on the `webServer` it spawns). A dev-server-only flag — nothing in
 * `api/src/config/` reads it.
 */
const E2E = process.env.AXGIT_E2E === "1";

/**
 * Answers `/api` with a 503 instead of proxying it, for the e2e run only.
 *
 * `e2e/fixtures.ts` already stubs or 503s every `/api` request a test makes,
 * but it cannot cover all of them: once a page starts closing, Playwright's
 * `_onRoute` returns early without consulting a single handler
 * (playwright-core 1.62.1, `lib/coreBundle.js`) and lets the request go to
 * the network. A fetch that lands in that window reached the proxy, and
 * vite logged `http proxy error … ECONNREFUSED` for it — a run that passed
 * still looked broken in CI's log. Removing the proxy for the duration of
 * the suite is the only place that race can be closed (docs/DECISIONS.md
 * #96).
 *
 * @returns {NonNullable<import("astro").ViteUserConfig["plugins"]>[number]}
 */
function apiUnavailable() {
  return {
    name: "axgit-api-unavailable",
    configureServer(server) {
      server.middlewares.use((req, res, next) => {
        if (!(req.url ?? "").startsWith("/api/")) return next();
        res.writeHead(503, { "Content-Type": "application/json" });
        res.end(JSON.stringify({ error: { code: "unavailable", message: "api unavailable (e2e)" } }));
      });
    },
  };
}

// https://astro.build/config
export default defineConfig({
  output: "static",
  integrations: [react(), shellRoutes()],
  // `<ClientRouter />` (docs/DECISIONS.md #24) defaults `prefetchAll` to
  // `true`. That's a net loss here: every `/{repo}/blob/*` (etc.) maps to one
  // byte-identical shell served `Cache-Control: no-cache` (`api/src/shell.rs`,
  // DECISIONS.md #17), so hovering a file listing would refetch that same
  // shell over and over for no benefit — the real per-page data always comes
  // from a separate `/api` call after the island mounts, which prefetching
  // the shell does nothing to speed up.
  prefetch: { prefetchAll: false },
  vite: {
    plugins: [tailwindcss(), shellFallback(), ...(E2E ? [apiUnavailable()] : [])],
    server: {
      // Only `astro dev` proxies `/api`; the production binary serves it
      // itself. Under `AXGIT_E2E` there is nothing to proxy to, so the
      // proxy is dropped entirely and `apiUnavailable()` answers instead.
      proxy: E2E ? {} : { "/api": process.env.AXGIT_API_URL ?? "http://127.0.0.1:8080" },
    },
  },
});
