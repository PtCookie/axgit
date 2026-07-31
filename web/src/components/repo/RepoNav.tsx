import { cn } from "@/lib/utils";
import { repoUrl } from "@/lib/router";

const TABS = [
  { sub: undefined, label: "요약" },
  { sub: "refs", label: "브랜치/태그" },
] as const;

interface RepoNavProps {
  repo: string;
  active: "summary" | "refs";
}

/**
 * Repository heading + tab navigation. Plain `<a>` links, not shadcn `Tabs`
 * (which is state-driven) — navigation here is a full page load
 * (DECISIONS.md #16), so an anchor's native behavior (modifier-click,
 * back/forward, prefetch) is what we want.
 */
export default function RepoNav({ repo, active }: RepoNavProps) {
  return (
    <div className="mb-6 space-y-3">
      <h1 className="font-heading text-2xl font-medium break-all">{repo}</h1>
      <nav className="border-border flex gap-4 border-b">
        {TABS.map((tab) => {
          const isActive = (tab.sub === undefined ? "summary" : "refs") === active;
          return (
            <a
              key={tab.label}
              href={repoUrl(repo, tab.sub)}
              aria-current={isActive ? "page" : undefined}
              className={cn(
                "-mb-px border-b-2 px-1 pb-2 text-sm font-medium",
                isActive
                  ? "border-primary text-foreground"
                  : "text-muted-foreground hover:text-foreground border-transparent",
              )}
            >
              {tab.label}
            </a>
          );
        })}
      </nav>
    </div>
  );
}
