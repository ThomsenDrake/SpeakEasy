"use client";

import * as React from "react";

import { cn } from "#utils/cn.ts";

import { FieldContext } from "../ds/form";

const Textarea = ({
  className,
  ref,
  ...props
}: React.ComponentProps<"textarea">) => {
  const fieldContext = React.useContext(FieldContext);

  return (
    <textarea
      className={cn(
        "border-default bg-default ring-offset-background placeholder:text-muted focus-visible:ring-default flex min-h-[80px] w-full rounded-md border px-3 py-2 text-sm focus-visible:ring-2 focus-visible:ring-offset-2 focus-visible:outline-hidden disabled:cursor-not-allowed disabled:opacity-50 sm:text-sm",
        className,
        {
          "border-destructive": fieldContext?.isInvalid,
        },
      )}
      ref={ref}
      {...props}
    />
  );
};
Textarea.displayName = "Textarea";

export { Textarea };
