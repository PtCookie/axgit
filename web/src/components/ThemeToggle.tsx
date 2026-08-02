import { lazy, Suspense, startTransition, useEffect, useState } from "react";

import { Button } from "@/components/ui/button";
import { apply, isTheme, STORAGE_KEY, ThemeIcons, type Theme } from "@/components/theme";

// Module scope, not inside the component — otherwise every render would mint
// a new lazy type and remount `ThemeMenu`. `importThemeMenu` is also called
// directly (outside `lazy()`) to warm the chunk on hover/focus, ahead of an
// actual click.
const importThemeMenu = () => import("./ThemeMenu");
const ThemeMenu = lazy(importThemeMenu);

/** Rendered with no client directive, so Astro inlines it as static SVG at
 *  build time — the prerendered shell's `slot="fallback"` for the island
 *  below, matching `RepoList.tsx`'s `RepoListSkeleton` pattern. Since the
 *  icon shown is chosen by CSS rather than script, this already displays the
 *  right one, and is otherwise inert (disabled, no menu) until hydration. */
export function ThemeToggleFallback() {
  return (
    <Button variant="ghost" size="icon-sm" aria-label="Theme" disabled>
      <ThemeIcons />
    </Button>
  );
}

/** The plain, pre-interaction trigger — also `<ThemeMenu>`'s `Suspense`
 *  fallback, so the two renders are the literal same element and swapping
 *  between them (should the fallback ever actually commit) has nothing to
 *  flash. `aria-haspopup="menu"` matters beyond a11y: `ui/button.tsx`'s cva
 *  has `active:not-aria-[haspopup]:translate-y-px`, so without it this
 *  button's press animation would visibly change the instant it's replaced
 *  by the real trigger. */
function PlainTrigger({ onClick }: { onClick?: () => void }) {
  return (
    <Button
      variant="ghost"
      size="icon-sm"
      aria-label="Theme"
      aria-haspopup="menu"
      onClick={onClick}
      onPointerEnter={() => void importThemeMenu()}
      onFocus={() => void importThemeMenu()}
    >
      <ThemeIcons />
    </Button>
  );
}

export default function ThemeToggle() {
  // Safe to read `document` synchronously: this island is `client:only`, so
  // it never runs during the static build, and `Layout.astro`'s head script
  // has already set `data-theme` by the time any island hydrates.
  const [theme, setTheme] = useState<Theme>(() => {
    const current = document.documentElement.dataset.theme;
    return isTheme(current) ? current : "system";
  });
  // Set once the toggle has been interacted with (click, or a warmed hover/
  // focus that resolved before a click) — mounts the lazy `ThemeMenu`, which
  // then owns its own open state via `defaultOpen` (see `ThemeMenu.tsx`).
  // `transition:persist`ing `<header>` (docs/DECISIONS.md #24) means this
  // survives client-side navigations, which is a feature: once requested on
  // one page, the toggle is already the real menu on the next.
  const [menuRequested, setMenuRequested] = useState(false);

  useEffect(() => {
    const media = matchMedia("(prefers-color-scheme: dark)");

    // Only "System" tracks the OS; an explicit choice must stay put.
    const onMediaChange = () => {
      if (document.documentElement.dataset.theme === "system") {
        apply("system", media);
      }
    };
    media.addEventListener("change", onMediaChange);

    // `storage` fires only in *other* tabs/documents per spec, so there's no
    // feedback loop with `handleChange` below.
    const onStorage = (event: StorageEvent) => {
      if (event.key !== STORAGE_KEY) {
        return;
      }
      const next = isTheme(event.newValue) ? event.newValue : "system";
      apply(next, media);
      setTheme(next);
    };
    window.addEventListener("storage", onStorage);

    return () => {
      media.removeEventListener("change", onMediaChange);
      window.removeEventListener("storage", onStorage);
    };
  }, []);

  function handleChange(value: unknown) {
    if (typeof value !== "string" || !isTheme(value)) {
      return;
    }
    setTheme(value);
    apply(value, matchMedia("(prefers-color-scheme: dark)"));
    try {
      localStorage.setItem(STORAGE_KEY, value);
    } catch {
      // Storage disabled (private mode, some embedded webviews) — the choice
      // still applies for this page view, it just won't survive a reload.
    }
  }

  if (!menuRequested) {
    return (
      <PlainTrigger
        onClick={() => {
          // A synchronous setState that causes `ThemeMenu` to suspend would
          // otherwise make React commit the Suspense fallback immediately
          // (and warn in dev) — `startTransition` keeps this button on
          // screen until the chunk resolves, then swaps straight to the open
          // menu.
          startTransition(() => setMenuRequested(true));
        }}
      />
    );
  }

  return (
    <Suspense fallback={<PlainTrigger />}>
      <ThemeMenu theme={theme} onThemeChange={handleChange} />
    </Suspense>
  );
}
