import { useEffect, useState } from "react";
import { MonitorIcon } from "@phosphor-icons/react/dist/ssr/Monitor";
import { MoonIcon } from "@phosphor-icons/react/dist/ssr/Moon";
import { SunIcon } from "@phosphor-icons/react/dist/ssr/Sun";

import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";

type Theme = "system" | "light" | "dark";

const STORAGE_KEY = "axgit:theme";

const OPTIONS: { value: Theme; label: string; Icon: typeof MonitorIcon }[] = [
  { value: "system", label: "System", Icon: MonitorIcon },
  { value: "light", label: "Light", Icon: SunIcon },
  { value: "dark", label: "Dark", Icon: MoonIcon },
];

function isTheme(value: string | null | undefined): value is Theme {
  return value === "system" || value === "light" || value === "dark";
}

/** Applies `theme` to <html>: `data-theme` carries the *preference*
 *  (system/light/dark), the `dark` class carries the *resolved* value that
 *  `global.css`'s `@custom-variant dark` and `.dark {}` token block key off.
 *  Mirrors the resolve expression in `Layout.astro`'s head script — kept
 *  duplicated rather than shared, since that one is a separate `is:inline`
 *  script with no import of its own. */
function apply(theme: Theme, media: MediaQueryList) {
  document.documentElement.dataset.theme = theme;
  document.documentElement.classList.toggle("dark", theme === "dark" || (theme === "system" && media.matches));
}

function ThemeIcons() {
  return (
    <>
      {OPTIONS.map(({ value, Icon }) => (
        // `global.css` shows only the icon matching `<html data-theme>`, so
        // all three are always in the DOM — including here, before the
        // island hydrates.
        <Icon key={value} data-theme-icon={value} className="size-4" aria-hidden="true" />
      ))}
    </>
  );
}

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

export default function ThemeToggle() {
  // Safe to read `document` synchronously: this island is `client:only`, so
  // it never runs during the static build, and `Layout.astro`'s head script
  // has already set `data-theme` by the time any island hydrates.
  const [theme, setTheme] = useState<Theme>(() => {
    const current = document.documentElement.dataset.theme;
    return isTheme(current) ? current : "system";
  });

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

  return (
    <DropdownMenu>
      <DropdownMenuTrigger render={<Button variant="ghost" size="icon-sm" aria-label="Theme" />}>
        <ThemeIcons />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        <DropdownMenuRadioGroup value={theme} onValueChange={handleChange}>
          {OPTIONS.map(({ value, label, Icon }) => (
            <DropdownMenuRadioItem key={value} value={value}>
              <Icon className="size-4" aria-hidden="true" />
              {label}
            </DropdownMenuRadioItem>
          ))}
        </DropdownMenuRadioGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
