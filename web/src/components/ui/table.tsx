import * as React from "react";

import { cn } from "cn";

// Hairline-ruled data table. `containerClassName` styles the scroll wrapper: give it a max-height so a sticky
// header row scrolls inside the table instead of the page. Columns that hold numbers are `numeric`
// (right-aligned, tabular numerals); columns that hold identifiers (SHAs, ids, timestamps) are `mono`.
function Table({
  className,
  containerClassName,
  ...props
}: React.ComponentProps<"table"> & { containerClassName?: string }) {
  return (
    <div
      data-slot="table-container"
      data-testid="table-container"
      className={cn("relative w-full overflow-x-auto", containerClassName)}
    >
      <table
        data-slot="table"
        data-testid="table"
        className={cn("w-full caption-bottom text-sm", className)}
        {...props}
      />
    </div>
  );
}

function TableHeader({ className, ...props }: React.ComponentProps<"thead">) {
  return <thead data-slot="table-header" data-testid="table-header" className={cn(className)} {...props} />;
}

function TableBody({ className, ...props }: React.ComponentProps<"tbody">) {
  return (
    <tbody
      data-slot="table-body"
      data-testid="table-body"
      className={cn("[&_tr:last-child]:border-0", className)}
      {...props}
    />
  );
}

function TableFooter({ className, ...props }: React.ComponentProps<"tfoot">) {
  return (
    <tfoot
      data-slot="table-footer"
      data-testid="table-footer"
      className={cn("border-border bg-muted-hover border-t font-medium [&>tr]:last:border-b-0", className)}
      {...props}
    />
  );
}

// Rows divide with `border`, hover with a quiet `muted` tint, and the selected row fills `muted`.
function TableRow({ className, ...props }: React.ComponentProps<"tr">) {
  return (
    <tr
      data-slot="table-row"
      data-testid="table-row"
      className={cn(
        "border-border hover:bg-muted-hover has-aria-expanded:bg-muted-hover data-[state=selected]:bg-muted border-b transition-colors",
        className,
      )}
      {...props}
    />
  );
}

function TableHead({ className, numeric = false, ...props }: React.ComponentProps<"th"> & { numeric?: boolean }) {
  return (
    <th
      data-slot="table-head"
      data-testid="table-head"
      // Local addition on top of Patina's file (docs/DECISIONS.md #102): every
      // `<TableHead>` in this codebase is a column header inside a `<thead>`
      // row — without an explicit `scope`, Chromium's a11y tree maps a bare
      // `<th>` to the generic "cell" role instead of "columnheader" (verified
      // empirically; HTML-AAM says it shouldn't need this, but Chromium's
      // implementation does), which silently breaks
      // `getByRole("columnheader")` lookups. `scope="col"` is overridable via
      // `props` for the rare row-header case.
      scope="col"
      className={cn(
        "text-foreground h-(--table-head) px-3 align-middle font-medium whitespace-nowrap [&:has([role=checkbox])]:pr-0",
        numeric ? "text-right tabular-nums" : "text-left",
        className,
      )}
      {...props}
    />
  );
}

function TableCell({
  className,
  numeric = false,
  mono = false,
  ...props
}: React.ComponentProps<"td"> & { numeric?: boolean; mono?: boolean }) {
  return (
    <td
      data-slot="table-cell"
      data-testid="table-cell"
      className={cn(
        "p-3 align-middle whitespace-nowrap [&:has([role=checkbox])]:pr-0",
        numeric && "text-right tabular-nums",
        mono && "font-mono text-[13px]/5",
        className,
      )}
      {...props}
    />
  );
}

function TableCaption({ className, ...props }: React.ComponentProps<"caption">) {
  return (
    <caption
      data-slot="table-caption"
      data-testid="table-caption"
      className={cn("text-muted-foreground mt-4 text-sm", className)}
      {...props}
    />
  );
}

export { Table, TableHeader, TableBody, TableFooter, TableHead, TableRow, TableCell, TableCaption };
