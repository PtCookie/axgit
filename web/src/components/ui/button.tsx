import { Button as ButtonPrimitive } from "@base-ui/react/button";
import { cva, type VariantProps } from "class-variance-authority";

import { cn } from "cn";

// Each class group is set by exactly one variant, so the result does not depend on `cn` merging conflicts.
const buttonVariants = cva(
  "focus-visible:outline-ring inline-flex shrink-0 cursor-pointer items-center justify-center border bg-clip-padding font-medium whitespace-nowrap transition-colors duration-150 select-none focus-visible:outline-2 focus-visible:outline-offset-2 active:not-aria-[haspopup]:translate-y-px disabled:pointer-events-none disabled:opacity-50 [&_svg]:pointer-events-none [&_svg]:shrink-0",
  {
    variants: {
      variant: {
        default: "bg-primary text-primary-foreground hover:bg-primary-hover border-transparent",
        outline: "border-border bg-outline-fill text-foreground hover:bg-outline-hover",
        secondary: "bg-secondary text-secondary-foreground hover:bg-secondary-hover border-transparent",
        ghost: "text-foreground hover:bg-ghost-hover border-transparent bg-transparent",
        destructive: "bg-destructive-muted text-destructive hover:bg-destructive-muted-hover border-transparent",
        link: "text-link border-transparent bg-transparent underline-offset-4 hover:underline",
      },
      size: {
        default: "h-(--control-md) gap-1.5 px-3 text-sm [&_svg:not([class*='size-'])]:size-4",
        xs: "h-(--control-xs) gap-1 px-2.5 text-xs [&_svg:not([class*='size-'])]:size-3",
        sm: "h-(--control-sm) gap-1 px-3 text-sm [&_svg:not([class*='size-'])]:size-4",
        lg: "h-(--control-lg) gap-1.5 px-4 text-sm [&_svg:not([class*='size-'])]:size-4",
        icon: "size-(--control-md) gap-1.5 text-sm [&_svg:not([class*='size-'])]:size-4",
        "icon-xs": "size-(--control-xs) gap-1 text-xs [&_svg:not([class*='size-'])]:size-3",
        "icon-sm": "size-(--control-sm) gap-1 text-sm [&_svg:not([class*='size-'])]:size-4",
        "icon-lg": "size-(--control-lg) gap-1.5 text-sm [&_svg:not([class*='size-'])]:size-4",
      },
      // The pill is opt-in: switch every control of a group or screen together.
      shape: {
        default: "rounded-lg",
        pill: "rounded-full",
      },
    },
    defaultVariants: {
      variant: "default",
      size: "default",
      shape: "default",
    },
  },
);

function Button({
  className,
  variant,
  size,
  shape,
  ...props
}: ButtonPrimitive.Props & VariantProps<typeof buttonVariants>) {
  return (
    <ButtonPrimitive
      data-slot="button"
      data-testid="button"
      className={cn(buttonVariants({ variant, size, shape, className }))}
      {...props}
    />
  );
}

export { Button, buttonVariants };
