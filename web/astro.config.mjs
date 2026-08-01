// @ts-check
import path from "node:path";

import { defineConfig } from "astro/config";
import tailwindcss from "@tailwindcss/vite";
import react from "@astrojs/react";

import { shellFor } from "./src/lib/shell.ts";

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
          path.extname(pathname) === "";

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
