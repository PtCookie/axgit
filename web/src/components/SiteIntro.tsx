import { useEffect, useState } from "react";

import { ApiError } from "@/lib/api/client";
import { getSite } from "@/lib/api/site";
import type { SiteInfo } from "@/lib/api/schemas";
import ReadmeBody from "@/components/repo/ReadmeBody";
import { Skeleton } from "@/components/ui/skeleton";

/** The `title` the api falls back to when `AXGIT_ROOT_TITLE` is unset
 *  (`api/src/site.rs::DEFAULT_TITLE`) — used below to tell "nothing
 *  configured" apart from "title configured, coincidentally to this same
 *  value" (indistinguishable either way, same collapse the api itself
 *  already makes). */
const DEFAULT_TITLE = "Axgit";

type State = { status: "loading" } | { status: "error"; error: ApiError } | { status: "data"; site: SiteInfo };

/** Also rendered statically into the page shell as the island's
 *  `slot="fallback"`. Most deployments configure none of
 *  `AXGIT_ROOT_TITLE`/`AXGIT_ROOT_DESC`/`AXGIT_ROOT_README`, so most of the
 *  time this briefly appears and then collapses to nothing (see the
 *  `!configured` check below) rather than settling into a final shape. */
export function SiteIntroSkeleton() {
  return (
    <div className="space-y-2" aria-busy="true">
      <Skeleton className="h-7 w-48" />
      <Skeleton className="h-4 w-72" />
    </div>
  );
}

/** The site's own title/description/readme (`GET /api/v1/site`,
 *  docs/DECISIONS.md #70/#71) — cgit's `root-title`/`root-desc`/
 *  `root-readme`. Renders nothing at all when none of the three are
 *  configured, so an unconfigured deployment's index page is unchanged; the
 *  header brand (`Layout.astro`'s `fillSiteChrome`) already shows the
 *  title, so this only adds an `<h1>` when there's something beyond that
 *  default to say. */
export default function SiteIntro() {
  const [state, setState] = useState<State>({ status: "loading" });

  useEffect(() => {
    let cancelled = false;

    getSite()
      .then((site) => {
        if (!cancelled) {
          setState({ status: "data", site });
        }
      })
      .catch((error: unknown) => {
        if (cancelled) {
          return;
        }
        setState({
          status: "error",
          error: error instanceof ApiError ? error : new ApiError("internal", "unknown error", 0),
        });
      });

    return () => {
      cancelled = true;
    };
  }, []);

  if (state.status === "loading") {
    return <SiteIntroSkeleton />;
  }

  if (state.status === "error") {
    return (
      <p role="alert" className="text-destructive text-sm">
        Failed to load site metadata: {state.error.message}
      </p>
    );
  }

  const { site } = state;
  const configured = site.title !== DEFAULT_TITLE || site.description !== null || site.readme !== null;
  if (!configured) {
    return null;
  }

  return (
    <div className="space-y-4">
      <h1 className="font-heading text-2xl font-medium">{site.title}</h1>
      {site.description && <p className="text-muted-foreground text-sm">{site.description}</p>}
      {site.readme && <ReadmeBody format={site.readme.format} content={site.readme.content} />}
    </div>
  );
}
