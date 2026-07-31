import { useEffect, useState } from "react";

import NotFound from "@/components/NotFound";
import RepoList from "@/components/RepoList";
import RepoNav from "@/components/repo/RepoNav";
import RefsView from "@/components/repo/RefsView";
import RepoSummary from "@/components/repo/RepoSummary";
import { parseRoute, type Route } from "@/lib/router";

/**
 * Client-side route shell. There is no history router (DECISIONS.md #16):
 * every navigation is a full page load, so the route is read from
 * `location.pathname` once per mount and never changes underneath us.
 *
 * Mounted with `client:only="react"` (not `client:load`) — Astro's static
 * build only has one `index.html`, served at every `/{repo}/...` path by the
 * server's SPA fallback, so build-time HTML baked in for `/` would mismatch
 * the client render on any other route.
 */
export default function App() {
  const [route] = useState<Route>(() => parseRoute(window.location.pathname));

  useEffect(() => {
    document.title = titleFor(route);
  }, [route]);

  switch (route.name) {
    case "repos":
      return <RepoList />;
    case "repo":
      return (
        <>
          <RepoNav repo={route.repo} active="summary" />
          <RepoSummary repo={route.repo} />
        </>
      );
    case "refs":
      return (
        <>
          <RepoNav repo={route.repo} active="refs" />
          <RefsView repo={route.repo} />
        </>
      );
    case "not-found":
      return <NotFound />;
  }
}

function titleFor(route: Route): string {
  switch (route.name) {
    case "repos":
      return "Axgit";
    case "repo":
      return `${route.repo} — Axgit`;
    case "refs":
      return `${route.repo} 브랜치/태그 — Axgit`;
    case "not-found":
      return "페이지를 찾을 수 없습니다 — Axgit";
  }
}
