import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { OPTIONS, ThemeIcons, type Theme } from "@/components/theme";

interface ThemeMenuProps {
  theme: Theme;
  onThemeChange: (value: unknown) => void;
}

/** The real theme menu — lazy-loaded by `ThemeToggle.tsx` on first
 *  interaction. This is the only module in the app that imports
 *  `@/components/ui/dropdown-menu` (base-ui's `Menu`, which pulls in
 *  floating-ui for positioning); keeping that import out of `ThemeToggle.tsx`
 *  is what keeps the ~137 KB floating-ui/menu chunk from shipping on every
 *  page. Never import from `ThemeToggle.tsx` here — see `theme.tsx`'s
 *  comment on why the two stay decoupled. */
export default function ThemeMenu({ theme, onThemeChange }: ThemeMenuProps) {
  return (
    // `defaultOpen` (not controlled `open`/`onOpenChange`): base-ui
    // initializes open on first render and then owns every close path itself
    // (Escape, outside-press, trigger toggle, scroll lock) — this component
    // is mounted precisely when the toggle wants the menu open, so there's
    // nothing to observe or resync from the eager side.
    <DropdownMenu defaultOpen>
      <DropdownMenuTrigger render={<Button variant="ghost" size="icon-sm" aria-label="Theme" />}>
        <ThemeIcons />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        <DropdownMenuRadioGroup value={theme} onValueChange={onThemeChange}>
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
