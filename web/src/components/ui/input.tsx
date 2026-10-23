import * as React from "react";
import { Input as InputPrimitive } from "@base-ui/react/input";

import { cn } from "cn";

// A borderless field on a 50% `input` fill. Invalid state: a 3px ring (plus a border in dark mode) and a
// destructive description, set by the consumer with `aria-invalid`.
function Input({
  className,
  type,
  shape = "default",
  ...props
}: React.ComponentProps<"input"> & { shape?: "default" | "pill" }) {
  return (
    <InputPrimitive
      type={type}
      data-slot="input"
      data-testid="input"
      className={cn(
        "bg-control text-foreground file:text-foreground placeholder:text-muted-foreground focus-visible:outline-ring aria-invalid:border-invalid-border aria-invalid:ring-invalid-ring h-(--control-md) w-full min-w-0 border border-transparent px-3 py-1 text-base transition-colors duration-150 file:inline-flex file:h-7 file:border-0 file:bg-transparent file:text-sm file:font-medium focus-visible:outline-2 focus-visible:outline-offset-2 disabled:cursor-not-allowed disabled:opacity-50 aria-invalid:ring-3 md:text-sm",
        shape === "pill" ? "rounded-full" : "rounded-lg",
        className,
      )}
      {...props}
    />
  );
}

export { Input };
