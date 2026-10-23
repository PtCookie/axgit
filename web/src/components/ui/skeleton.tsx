import { cva, type VariantProps } from "class-variance-authority";
import * as React from "react";

import { cn } from "cn";

// A `muted` placeholder that pulses at 2s and stays still under prefers-reduced-motion. Text lines use the
// default radius, blocks `block`, avatars `circle`. Give it the width and height of the real content.
const skeletonVariants = cva("bg-muted animate-pulse motion-reduce:animate-none", {
  variants: {
    shape: {
      default: "rounded-md",
      block: "rounded-lg",
      circle: "rounded-full",
    },
  },
  defaultVariants: {
    shape: "default",
  },
});

function Skeleton({ className, shape, ...props }: React.ComponentProps<"div"> & VariantProps<typeof skeletonVariants>) {
  return (
    <div
      data-slot="skeleton"
      data-testid="skeleton"
      className={cn(skeletonVariants({ shape }), className)}
      {...props}
    />
  );
}

export { Skeleton, skeletonVariants };
