"use client";

import type { ReactNode } from "react";
import { useIsClient } from "usehooks-ts";

import { cn } from "#utils/cn.ts";

import { useCollapsibleContext } from "./collapsible-context";

interface CollapsibleContentProps {
  className?: string;
  children: ReactNode;
}

const CollapsibleContent = ({
  className,
  children,
}: CollapsibleContentProps) => {
  const { isOpen, contentId, triggerId } = useCollapsibleContext();
  const isClient = useIsClient();

  return (
    <div
      id={contentId}
      role="region"
      aria-labelledby={triggerId}
      inert={!isOpen ? true : undefined}
      className={cn(
        "grid overflow-hidden transition-[grid-template-rows] duration-300 ease-[cubic-bezier(0,0,0,1)]",
        isOpen ? "grid-rows-[1fr]" : "grid-rows-[0fr]",
        !isClient && "duration-0!",
      )}
    >
      <div className={cn("min-h-0 overflow-hidden", className)}>{children}</div>
    </div>
  );
};

CollapsibleContent.displayName = "CollapsibleContent";

export type { CollapsibleContentProps };
export { CollapsibleContent };
