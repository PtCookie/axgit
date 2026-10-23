import * as React from "react";
import { Menu as MenuPrimitive } from "@base-ui/react/menu";
import { CaretRightIcon, CheckIcon } from "@phosphor-icons/react";
import { cva } from "class-variance-authority";

import { cn } from "cn";

// A solid menu with full-width rows: an opaque `popover` surface, rounded to `radius-2xl` and clipped, with no
// padding. Rows run edge to edge with no radius, so the first and last highlight meets the rounded corners.
// Keep at most one destructive item, and put it last.
const menuRowVariants = cva(
  "relative flex cursor-default items-center gap-2.5 px-3 py-2 text-sm font-medium outline-hidden select-none data-disabled:pointer-events-none data-disabled:opacity-50 [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-4",
  {
    variants: {
      variant: {
        default: "data-highlighted:bg-accent data-highlighted:text-accent-foreground",
        destructive:
          "text-destructive data-highlighted:bg-destructive-muted data-highlighted:text-destructive [&_svg]:text-destructive",
      },
    },
    defaultVariants: {
      variant: "default",
    },
  },
);

function DropdownMenu({ ...props }: MenuPrimitive.Root.Props) {
  return <MenuPrimitive.Root data-slot="dropdown-menu" {...props} />;
}

function DropdownMenuPortal({ ...props }: MenuPrimitive.Portal.Props) {
  return <MenuPrimitive.Portal data-slot="dropdown-menu-portal" {...props} />;
}

function DropdownMenuTrigger({ ...props }: MenuPrimitive.Trigger.Props) {
  return <MenuPrimitive.Trigger data-slot="dropdown-menu-trigger" data-testid="dropdown-menu-trigger" {...props} />;
}

function DropdownMenuContent({
  align = "start",
  alignOffset = 0,
  side = "bottom",
  sideOffset = 4,
  className,
  ...props
}: MenuPrimitive.Popup.Props & Pick<MenuPrimitive.Positioner.Props, "align" | "alignOffset" | "side" | "sideOffset">) {
  return (
    <MenuPrimitive.Portal>
      <MenuPrimitive.Positioner
        className="isolate z-50 outline-none"
        align={align}
        alignOffset={alignOffset}
        side={side}
        sideOffset={sideOffset}
      >
        <MenuPrimitive.Popup
          data-slot="dropdown-menu-content"
          data-testid="dropdown-menu-content"
          className={cn(
            "bg-popover text-popover-foreground z-50 max-h-(--available-height) min-w-48 origin-(--transform-origin) overflow-x-hidden overflow-y-auto rounded-2xl p-0 shadow-(--shadow-menu) outline-none",
            className,
          )}
          {...props}
        />
      </MenuPrimitive.Positioner>
    </MenuPrimitive.Portal>
  );
}

function DropdownMenuGroup({ ...props }: MenuPrimitive.Group.Props) {
  return <MenuPrimitive.Group data-slot="dropdown-menu-group" data-testid="dropdown-menu-group" {...props} />;
}

function DropdownMenuLabel({ className, ...props }: MenuPrimitive.GroupLabel.Props) {
  return (
    <MenuPrimitive.GroupLabel
      data-slot="dropdown-menu-label"
      data-testid="dropdown-menu-label"
      className={cn("text-muted-foreground px-3 pt-2.5 pb-1 text-xs", className)}
      {...props}
    />
  );
}

function DropdownMenuItem({
  className,
  variant = "default",
  ...props
}: MenuPrimitive.Item.Props & { variant?: "default" | "destructive" }) {
  return (
    <MenuPrimitive.Item
      data-slot="dropdown-menu-item"
      data-testid="dropdown-menu-item"
      data-variant={variant}
      className={cn(menuRowVariants({ variant }), className)}
      {...props}
    />
  );
}

function DropdownMenuSub({ ...props }: MenuPrimitive.SubmenuRoot.Props) {
  return <MenuPrimitive.SubmenuRoot data-slot="dropdown-menu-sub" {...props} />;
}

function DropdownMenuSubTrigger({ className, children, ...props }: MenuPrimitive.SubmenuTrigger.Props) {
  return (
    <MenuPrimitive.SubmenuTrigger
      data-slot="dropdown-menu-sub-trigger"
      data-testid="dropdown-menu-sub-trigger"
      className={cn(
        menuRowVariants({ variant: "default" }),
        "data-popup-open:bg-accent data-popup-open:text-accent-foreground",
        className,
      )}
      {...props}
    >
      {children}
      <CaretRightIcon className="ml-auto" aria-hidden />
    </MenuPrimitive.SubmenuTrigger>
  );
}

function DropdownMenuSubContent({
  align = "start",
  alignOffset = 0,
  side = "right",
  sideOffset = 0,
  className,
  ...props
}: React.ComponentProps<typeof DropdownMenuContent>) {
  return (
    <DropdownMenuContent
      data-slot="dropdown-menu-sub-content"
      data-testid="dropdown-menu-sub-content"
      className={className}
      align={align}
      alignOffset={alignOffset}
      side={side}
      sideOffset={sideOffset}
      {...props}
    />
  );
}

function DropdownMenuCheckboxItem({ className, children, checked, ...props }: MenuPrimitive.CheckboxItem.Props) {
  return (
    <MenuPrimitive.CheckboxItem
      data-slot="dropdown-menu-checkbox-item"
      data-testid="dropdown-menu-checkbox-item"
      className={cn(menuRowVariants({ variant: "default" }), className)}
      checked={checked}
      {...props}
    >
      {children}
      <span
        className="pointer-events-none ml-auto flex items-center"
        data-slot="dropdown-menu-checkbox-item-indicator"
        data-testid="dropdown-menu-checkbox-item-indicator"
      >
        <MenuPrimitive.CheckboxItemIndicator>
          <CheckIcon aria-hidden />
        </MenuPrimitive.CheckboxItemIndicator>
      </span>
    </MenuPrimitive.CheckboxItem>
  );
}

function DropdownMenuRadioGroup({ ...props }: MenuPrimitive.RadioGroup.Props) {
  return (
    <MenuPrimitive.RadioGroup
      data-slot="dropdown-menu-radio-group"
      data-testid="dropdown-menu-radio-group"
      {...props}
    />
  );
}

function DropdownMenuRadioItem({ className, children, ...props }: MenuPrimitive.RadioItem.Props) {
  return (
    <MenuPrimitive.RadioItem
      data-slot="dropdown-menu-radio-item"
      data-testid="dropdown-menu-radio-item"
      className={cn(menuRowVariants({ variant: "default" }), className)}
      {...props}
    >
      {children}
      <span
        className="pointer-events-none ml-auto flex items-center"
        data-slot="dropdown-menu-radio-item-indicator"
        data-testid="dropdown-menu-radio-item-indicator"
      >
        <MenuPrimitive.RadioItemIndicator>
          <CheckIcon aria-hidden />
        </MenuPrimitive.RadioItemIndicator>
      </span>
    </MenuPrimitive.RadioItem>
  );
}

function DropdownMenuSeparator({ className, ...props }: MenuPrimitive.Separator.Props) {
  return (
    <MenuPrimitive.Separator
      data-slot="dropdown-menu-separator"
      data-testid="dropdown-menu-separator"
      className={cn("bg-border my-1 h-px", className)}
      {...props}
    />
  );
}

function DropdownMenuShortcut({ className, ...props }: React.ComponentProps<"span">) {
  return (
    <span
      data-slot="dropdown-menu-shortcut"
      data-testid="dropdown-menu-shortcut"
      className={cn("text-muted-foreground ml-auto text-xs leading-4 tracking-widest", className)}
      {...props}
    />
  );
}

export {
  DropdownMenu,
  DropdownMenuPortal,
  DropdownMenuTrigger,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuLabel,
  DropdownMenuItem,
  DropdownMenuCheckboxItem,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuShortcut,
  DropdownMenuSub,
  DropdownMenuSubTrigger,
  DropdownMenuSubContent,
};
