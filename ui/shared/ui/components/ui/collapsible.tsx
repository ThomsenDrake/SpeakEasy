"use client";

import * as CollapsiblePrimitive from "@radix-ui/react-collapsible";
import React from "react";
import { useIsClient } from "usehooks-ts";

import { cn } from "#utils/cn.ts";

const Collapsible = CollapsiblePrimitive.Root;

const CollapsibleTrigger = CollapsiblePrimitive.CollapsibleTrigger;

const CollapsibleContent = ({
  className,
  ref,
  ...props
}: React.ComponentProps<typeof CollapsiblePrimitive.Content>) => {
  const isClient = useIsClient();

  return (
    <CollapsiblePrimitive.Content
      ref={ref}
      className={cn(
        "animated-collapsible-content",
        !isClient && "duration-0!",
        className,
      )}
      {...props}
    />
  );
};

export { Collapsible, CollapsibleContent, CollapsibleTrigger };
