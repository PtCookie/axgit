import { ALLOWED_CONTEXT, type DiffOptions } from "@/lib/diff-options";

const NO_EXTRA_PARAMS: Record<string, string> = {};

interface DiffOptionsBarProps {
  options: DiffOptions;
  /**
   * Every other param currently set on the page (e.g. a future compare
   * page's `from`/`to`) — carried as hidden inputs so submitting this form
   * doesn't drop them. Never `context`/`ignorews`/`view` themselves.
   */
  extraParams?: Record<string, string>;
}

/**
 * Context-line and ignore-whitespace controls for a diff view. A plain
 * `method="get"` form (same pattern as `SearchView`'s query controls):
 * `<ClientRouter />` intercepts the same-origin submit for a client-side
 * transition, and a full page load works identically if it doesn't — no
 * client-side state is kept.
 *
 * Submitting this form always triggers a refetch, even though `context`/
 * `ignorews` are the only things that changed: the island remounts on every
 * navigation (DECISIONS.md #24). That's a deliberate cost, not something to
 * "fix" with component state — see the module doc on `lib/diff-options.ts`.
 * It's mitigated by always linking full commit shas (`commitHref`), so the
 * refetch on a full-sha URL is an immutable, browser-cached hit.
 */
export default function DiffOptionsBar({ options, extraParams = NO_EXTRA_PARAMS }: DiffOptionsBarProps) {
  return (
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
  );
}
