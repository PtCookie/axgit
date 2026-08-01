// @ts-check
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { defineConfig } from "astro/config";
import tailwindcss from "@tailwindcss/vite";
import react from "@astrojs/react";

import { shellFor } from "./src/lib/shell.ts";

const PUBLIC_DIR = path.join(fileURLToPath(new URL(".", import.meta.url)), "public");

/**
 * True when `pathname` resolves to a real file under `public/` — those are
 * served as-is (favicon, etc.) and must never be rewritten onto a page
 * shell. `path.normalize` + a prefix check keeps a `..`-laden pathname from
 * escaping `public/` (this only decides whether to skip a rewrite, so a
 * false negative just falls through to `shellFor`, not a filesystem read of
 * arbitrary content).
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
        const isNavigation = (req.method === "GET" || req.method === "HEAD") && accept.includes("text/html");
        const isAppRoute =
          !pathname.startsWith("/api") &&
          !pathname.startsWith("/@") &&
          !pathname.startsWith("/_astro") &&
          !pathname.startsWith("/src") &&
          !pathname.startsWith("/node_modules") &&
          !isPublicAsset(pathname);

        if (isNavigation && isAppRoute) {
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

// https://astro.build/config
export default defineConfig({
  output: "static",
  integrations: [react()],
  vite: {
    plugins: [tailwindcss(), shellFallback()],
    server: {
      proxy: {
        "/api": process.env.AXGIT_API_URL ?? "http://127.0.0.1:8080",
      },
    },
  },
});
