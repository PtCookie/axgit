// @ts-check
import path from "node:path";

import { defineConfig } from "astro/config";
import tailwindcss from "@tailwindcss/vite";
import react from "@astrojs/react";

/**
 * Rewrites `/{repo}/...` HTML navigations to `/` in dev, mirroring the
 * production SPA fallback (`api/src/routes.rs`, DECISIONS.md #16): the api
 * server serves `index.html` for any unmatched path so the client router
 * can take over, but Astro's dev server has no such fallback and 404s.
 * Only affects `astro dev` / Playwright against it — the static build is
 * untouched.
 */
function spaFallback() {
  return {
    name: "axgit-spa-fallback",
    configureServer(server) {
      server.middlewares.use((req, res, next) => {
        const url = req.url ?? "/";
        const accept = req.headers.accept ?? "";
        const pathname = url.split("?")[0];
        const isNavigation = (req.method === "GET" || req.method === "HEAD") && accept.includes("text/html");
        const isAppRoute =
          !pathname.startsWith("/api") &&
          !pathname.startsWith("/@") &&
          !pathname.startsWith("/_astro") &&
          !pathname.startsWith("/src") &&
          !pathname.startsWith("/node_modules") &&
          path.extname(pathname) === "";

        if (isNavigation && isAppRoute && pathname !== "/") {
          req.url = "/";
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
    plugins: [tailwindcss(), spaFallback()],
    server: {
      proxy: {
        "/api": process.env.AXGIT_API_URL ?? "http://127.0.0.1:8080",
      },
    },
  },
});
