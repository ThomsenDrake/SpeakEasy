import * as React from "react";

import { cn } from "#utils/cn.ts";

const Table = ({
  className,
  wrapperClassName,
  blockInnerScrolling = false,
  ref,
  wrapperRef,
  ...props
}: React.ComponentProps<"table"> & {
  wrapperClassName?: string;
  blockInnerScrolling?: boolean;
  wrapperRef?: React.Ref<HTMLDivElement>;
}) => (
  <div
    ref={wrapperRef}
    className={cn(
      "relative w-full overflow-x-auto",
      wrapperClassName,
      blockInnerScrolling && "overflow-y-hidden",
    )}
  >
    <table
      ref={ref}
      className={cn("w-full caption-bottom text-sm", className)}
      {...props}
    />
  </div>
);
Table.displayName = "Table";

const TableHeader = ({
  ref,
  className,
  ...props
}: React.ComponentProps<"thead">) => (
  <thead
    ref={ref}
    className={cn("bg-muted text-xs [&_tr]:border-b", className)}
    {...props}
  />
);
TableHeader.displayName = "TableHeader";

const TableBody = ({
  ref,
  className,
  ...props
}: React.ComponentProps<"tbody">) => (
  <tbody
    ref={ref}
    className={cn(
      "[&_tr:last-child]:border-0 [&_tr:only-child]:border-b",
      className,
    )}
    {...props}
  />
);
TableBody.displayName = "TableBody";

const TableFooter = ({
  ref,
  className,
  ...props
}: React.ComponentProps<"tfoot">) => (
  <tfoot
    ref={ref}
    className={cn(
      "bg-muted/50 border-t font-medium last:[&>tr]:border-b-0",
      className,
    )}
    {...props}
  />
);
TableFooter.displayName = "TableFooter";

const TableRow = ({ ref, className, ...props }: React.ComponentProps<"tr">) => (
  <tr
    ref={ref}
    className={cn(
      "hover:bg-state-ghost-hover data-[state=selected]:bg-badge-orange border-b transition-colors",
      className,
    )}
    {...props}
  />
);
TableRow.displayName = "TableRow";

const TableHead = ({
  ref,
  className,
  ...props
}: React.ComponentProps<"th">) => (
  <th
    ref={ref}
    className={cn("h-10 p-2 text-left align-middle font-medium", className)}
    {...props}
  />
);
TableHead.displayName = "TableHead";

const TableCell = ({
  ref,
  className,
  ...props
}: React.ComponentProps<"td">) => (
  <td ref={ref} className={cn("p-2 align-middle", className)} {...props} />
);
TableCell.displayName = "TableCell";

const TableCaption = ({
  ref,
  className,
  ...props
}: React.ComponentProps<"caption">) => (
  <caption
    ref={ref}
    className={cn("text-muted mt-4 text-sm", className)}
    {...props}
  />
);
TableCaption.displayName = "TableCaption";

export {
  Table,
  TableBody,
  TableCaption,
  TableCell,
  TableFooter,
  TableHead,
  TableHeader,
  TableRow,
};
