import { ALLOWED_CONTEXT, type DiffOptions, type DiffViewMode } from "@/lib/diff-options";
import { cn } from "@/lib/utils";

const NO_EXTRA_PARAMS: Record<string, string> = {};
const VIEW_MODES: { value: DiffViewMode; label: string }[] = [
  { value: "unified", label: "Unified" },
  { value: "split", label: "Split" },
  { value: "stat", label: "Stat only" },
];

interface DiffOptionsBarProps {
  options: DiffOptions;
  /**
   * Every other param currently set on the page (e.g. the compare page's
   * `from`/`to`) — carried as hidden inputs so submitting this form doesn't
   * drop them. Never `context`/`ignorews`/`view` themselves.
   */
  extraParams?: Record<string, string>;
  /**
   * Builds the href for a view pill — everything about the current page
   * except `view` stays as it is now. Supplied by the caller since the
   * commit page (`commitHref`) and the compare page (`compareHref`) build
   * that href differently.
   */
  hrefFor: (patch: Partial<DiffOptions>) => string;
}

/**
 * Display controls for a diff view: a fixed unified/split/stat choice as pill
 * links (`StatsView`'s period-switcher pattern — one click, no Apply), plus
 * context-line and ignore-whitespace controls as a plain `method="get"` form
 * (`SearchView`'s pattern): `<ClientRouter />` intercepts the same-origin
 * submit for a client-side transition, and a full page load works
 * identically if it doesn't — no client-side state is kept.
 *
 * Submitting the form always triggers a refetch, even though `context`/
 * `ignorews` are the only things that changed: the island remounts on every
 * navigation (DECISIONS.md #24). That's a deliberate cost, not something to
 * "fix" with component state — see the module doc on `lib/diff-options.ts`.
 * It's mitigated by always linking full commit shas (`commitHref`), so the
 * refetch on a full-sha URL is an immutable, browser-cached hit.
 */
export default function DiffOptionsBar({ options, extraParams = NO_EXTRA_PARAMS, hrefFor }: DiffOptionsBarProps) {
  return (
    <div className="flex flex-wrap items-center gap-3">
      <nav className="flex gap-2" aria-label="Diff view">
        {VIEW_MODES.map((mode) => (
          <a
            key={mode.value}
            href={hrefFor({ view: mode.value })}
            aria-current={mode.value === options.view ? "page" : undefined}
            className={cn(
              "rounded-3xl border px-3 py-1 text-sm font-medium",
              mode.value === options.view
                ? "border-primary bg-primary/10 text-foreground"
                : "border-input text-muted-foreground hover:text-foreground",
            )}
          >
            {mode.label}
          </a>
        ))}
      </nav>
      <form method="get" className="flex flex-wrap items-center gap-3 text-sm">
        {Object.entries(extraParams).map(([key, value]) => (
          <input key={key} type="hidden" name={key} value={value} />
        ))}
        {options.view !== "unified" && <input type="hidden" name="view" value={options.view} />}
        <label className="flex items-center gap-1.5">
          <span className="text-muted-foreground">Context</span>
          <select
            name="context"
            defaultValue={String(options.context)}
            aria-label="Context lines"
            className="border-input bg-input/50 h-8 rounded-md border px-2 text-sm"
          >
            {ALLOWED_CONTEXT.map((value) => (
              <option key={value} value={value}>
                {value}
              </option>
            ))}
          </select>
        </label>
        <label className="flex items-center gap-1.5">
          <input type="checkbox" name="ignorews" value="1" defaultChecked={options.ignorews} className="size-4" />
          <span>Ignore whitespace</span>
        </label>
        <button
          type="submit"
          className="border-input bg-input/50 hover:bg-accent h-8 rounded-md border px-3 text-sm font-medium"
        >
          Apply
        </button>
      </form>
    </div>
  );
}
