"use client";

import * as React from "react";
import type { TextareaAutosizeProps } from "react-textarea-autosize";
import TextareaAutosize from "react-textarea-autosize";

import { cn } from "../utils/cn";

export const AutosizeTextarea = ({
  ref,
  minRows,
  maxRows,
  className,
  ...props
}: TextareaAutosizeProps & { ref?: React.Ref<HTMLTextAreaElement> }) => (
  <TextareaAutosize
    className={cn(
      "border-default bg-default ring-offset-background placeholder:text-muted focus-visible:ring-default flex w-full rounded-md border px-3 py-2 text-base focus-visible:ring-2 focus-visible:ring-offset-2 focus-visible:outline-hidden disabled:cursor-not-allowed disabled:opacity-50 sm:text-sm",
      className,
    )}
    ref={ref}
    minRows={minRows}
    maxRows={maxRows}
    {...props}
  />
);
AutosizeTextarea.displayName = "AutosizeTextarea";
