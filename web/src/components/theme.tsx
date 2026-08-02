import { MonitorIcon } from "@phosphor-icons/react/dist/ssr/Monitor";
import { MoonIcon } from "@phosphor-icons/react/dist/ssr/Moon";
import { SunIcon } from "@phosphor-icons/react/dist/ssr/Sun";

export type Theme = "system" | "light" | "dark";

export const STORAGE_KEY = "axgit:theme";

export const OPTIONS: { value: Theme; label: string; Icon: typeof MonitorIcon }[] = [
  { value: "system", label: "System", Icon: MonitorIcon },
  { value: "light", label: "Light", Icon: SunIcon },
  { value: "dark", label: "Dark", Icon: MoonIcon },
];

export function isTheme(value: string | null | undefined): value is Theme {
  return value === "system" || value === "light" || value === "dark";
}

/** Applies `theme` to <html>: `data-theme` carries the *preference*
 *  (system/light/dark), the `dark` class carries the *resolved* value that
 *  `global.css`'s `@custom-variant dark` and `.dark {}` token block key off.
 *  Mirrors the resolve expression in `Layout.astro`'s head script — kept
 *  duplicated rather than shared, since that one is a separate `is:inline`
 *  script with no import of its own. */
export function apply(theme: Theme, media: MediaQueryList) {
  document.documentElement.dataset.theme = theme;
  document.documentElement.classList.toggle("dark", theme === "dark" || (theme === "system" && media.matches));
}

/** Leaf module shared by the eager `ThemeToggle` and the lazy `ThemeMenu` —
 *  neither of those two imports the other, so this is the only place their
 *  dependency graphs meet. Keep it that way: a cross-import between them
 *  would pull `ThemeMenu`'s base-ui/floating-ui weight back into the eager
 *  chunk that ships on every page. */
export function ThemeIcons() {
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
